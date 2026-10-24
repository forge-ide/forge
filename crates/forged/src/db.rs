//! SQLite: connection setup, migrations, and the project store.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use forge_api::events::Remember;
use forge_api::model::{Approval, Mode, Project, Repo, Session, SessionState, Usage};
use forge_api::{ApprovalId, ProjectId, SessionId, Timestamp};
use forge_session::{ApprovalOutcome, ApprovalRecord, SessionStore, StoreError, WorktreeRecord};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Migrations in order. Never edit one that has shipped; add a new file instead.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_projects.sql"),
    include_str!("../migrations/0002_sessions.sql"),
    include_str!("../migrations/0003_approvals.sql"),
];

/// The daemon's database. Every call runs on the blocking pool.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let mut conn =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&mut conn).with_context(|| format!("migrating {}", path.display()))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The connection, for components that own their own tables (the event log).
    pub fn shared(&self) -> forge_events::SharedConn {
        self.conn.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    async fn call<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut conn)
        })
        .await?
    }

    /// Insert or update the project at `root`, keeping its ID if it was opened before.
    pub async fn upsert_project(
        &self,
        root: String,
        name: String,
        repos: Vec<Repo>,
    ) -> anyhow::Result<Project> {
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let existing: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE root = ?1", [&root], |r| r.get(0))
                .optional()?;
            let id = match existing {
                Some(id) => {
                    tx.execute("UPDATE projects SET name = ?1 WHERE id = ?2", params![name, id])?;
                    tx.execute("DELETE FROM repos WHERE project_id = ?1", [&id])?;
                    id.parse::<ProjectId>()?
                }
                None => {
                    let id = ProjectId::new();
                    tx.execute(
                        "INSERT INTO projects (id, name, root, created_at) VALUES (?1, ?2, ?3, ?4)",
                        params![id.as_str(), name, root, Timestamp::now().to_string()],
                    )?;
                    id
                }
            };
            for repo in &repos {
                tx.execute(
                    "INSERT INTO repos (project_id, name, path, base_branch) VALUES (?1, ?2, ?3, ?4)",
                    params![id.as_str(), repo.name, repo.path, repo.base_branch],
                )?;
            }
            tx.commit()?;
            Ok(Project { id, name, root, repos })
        })
        .await
    }

    pub async fn list_projects(&self) -> anyhow::Result<Vec<Project>> {
        self.call(|conn| {
            let rows: Vec<(String, String, String)> = conn
                .prepare("SELECT id, name, root FROM projects ORDER BY name, root")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            rows.into_iter()
                .map(|(id, name, root)| with_repos(conn, id, name, root))
                .collect()
        })
        .await
    }

    /// A project's root, without going through the async pool. Blocking.
    pub fn project_root(&self, id: &ProjectId) -> anyhow::Result<Option<std::path::PathBuf>> {
        let root: Option<String> = self
            .lock()
            .query_row(
                "SELECT root FROM projects WHERE id = ?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(root.map(std::path::PathBuf::from))
    }

    pub async fn get_project(&self, id: ProjectId) -> anyhow::Result<Option<Project>> {
        self.call(move |conn| {
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT name, root FROM projects WHERE id = ?1",
                    [id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            row.map(|(name, root)| with_repos(conn, id.to_string(), name, root))
                .transpose()
        })
        .await
    }
}

fn with_repos(
    conn: &Connection,
    id: String,
    name: String,
    root: String,
) -> anyhow::Result<Project> {
    let repos = conn
        .prepare("SELECT name, path, base_branch FROM repos WHERE project_id = ?1 ORDER BY rowid")?
        .query_map([&id], |r| {
            Ok(Repo {
                name: r.get(0)?,
                path: r.get(1)?,
                base_branch: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Project {
        id: id.parse()?,
        name,
        root,
        repos,
    })
}

const SESSION_COLUMNS: &str = "id, project_id, title, agent_profile, state, mode, sandbox, branch,
    input_tokens, output_tokens, cost_usd, created_at, updated_at";

impl SessionStore for Db {
    fn insert(&self, session: &Session, worktrees: &[WorktreeRecord]) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(store_err)?;
        tx.execute(
            "INSERT INTO sessions (id, project_id, title, agent_profile, state, mode, sandbox,
               branch, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session.id.as_str(),
                session.project_id.as_str(),
                session.title,
                session.agent_profile,
                wire(session.state),
                wire(session.mode),
                wire(session.sandbox),
                session.branch,
                session.created_at.to_string(),
                session.updated_at.to_string(),
            ],
        )
        .map_err(store_err)?;
        for w in worktrees {
            tx.execute(
                "INSERT INTO worktrees (session_id, repo_name, path) VALUES (?1, ?2, ?3)",
                params![session.id.as_str(), w.repo, w.path.to_string_lossy()],
            )
            .map_err(store_err)?;
        }
        tx.commit().map_err(store_err)
    }

    fn set_state(
        &self,
        id: &SessionId,
        state: SessionState,
        at: Timestamp,
    ) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE sessions SET state = ?1, updated_at = ?2 WHERE id = ?3",
                params![wire(state), at.to_string(), id.as_str()],
            )
            .map(drop)
            .map_err(store_err)
    }

    fn get(&self, id: &SessionId) -> Result<Option<Session>, StoreError> {
        self.lock()
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                [id.as_str()],
                session_row,
            )
            .optional()
            .map_err(store_err)
    }

    fn list(
        &self,
        project: &ProjectId,
        states: Option<&[SessionState]>,
    ) -> Result<Vec<Session>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions WHERE project_id = ?1
                 ORDER BY created_at DESC, id DESC"
            ))
            .map_err(store_err)?;
        let sessions = stmt
            .query_map([project.as_str()], session_row)
            .map_err(store_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_err)?;
        Ok(match states {
            Some(states) => sessions
                .into_iter()
                .filter(|s| states.contains(&s.state))
                .collect(),
            None => sessions,
        })
    }

    fn in_states(&self, states: &[SessionState]) -> Result<Vec<Session>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at"
            ))
            .map_err(store_err)?;
        let sessions = stmt
            .query_map([], session_row)
            .map_err(store_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_err)?;
        Ok(sessions
            .into_iter()
            .filter(|s| states.contains(&s.state))
            .collect())
    }

    fn worktrees(&self, id: &SessionId) -> Result<Vec<WorktreeRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT repo_name, path FROM worktrees WHERE session_id = ?1 ORDER BY rowid")
            .map_err(store_err)?;
        stmt.query_map([id.as_str()], |r| {
            Ok(WorktreeRecord {
                repo: r.get(0)?,
                path: std::path::PathBuf::from(r.get::<_, String>(1)?),
            })
        })
        .map_err(store_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_err)
    }

    fn live_owner(&self, path: &Path) -> Result<Option<SessionId>, StoreError> {
        let id: Option<String> = self
            .lock()
            .query_row(
                "SELECT s.id FROM worktrees w JOIN sessions s ON s.id = w.session_id
                 WHERE w.path = ?1 AND s.state NOT IN ('done', 'failed')
                 ORDER BY s.created_at DESC LIMIT 1",
                [path.to_string_lossy()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        id.map(|id| id.parse().map_err(|e| StoreError(format!("{e}"))))
            .transpose()
    }

    fn set_mode(&self, id: &SessionId, mode: Mode, at: Timestamp) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE sessions SET mode = ?1, updated_at = ?2 WHERE id = ?3",
                params![wire(mode), at.to_string(), id.as_str()],
            )
            .map(drop)
            .map_err(store_err)
    }

    fn insert_approval(&self, approval: &Approval) -> Result<(), StoreError> {
        let action = serde_json::to_string(&approval.action)
            .map_err(|e| StoreError(format!("encoding an action: {e}")))?;
        self.lock()
            .execute(
                "INSERT INTO approvals (id, session_id, action_json, reason, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    approval.id.as_str(),
                    approval.session_id.as_str(),
                    action,
                    approval.reason,
                    approval.created_at.to_string(),
                ],
            )
            .map(drop)
            .map_err(store_err)
    }

    fn approval(&self, id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        self.lock()
            .query_row(
                &format!(
                    "SELECT {APPROVAL_COLUMNS}, a.decision IS NOT NULL FROM approvals a
                     WHERE a.id = ?1"
                ),
                [id.as_str()],
                |r| {
                    Ok(ApprovalRecord {
                        approval: approval_row(r)?,
                        decided: r.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(store_err)
    }

    fn end_approval(
        &self,
        id: &ApprovalId,
        outcome: ApprovalOutcome,
        remember: Option<Remember>,
        at: Timestamp,
    ) -> Result<bool, StoreError> {
        let changed = self
            .lock()
            .execute(
                "UPDATE approvals SET decision = ?1, remember = ?2, decided_at = ?3
                 WHERE id = ?4 AND decision IS NULL",
                params![
                    outcome_name(outcome),
                    remember.map(wire),
                    at.to_string(),
                    id.as_str()
                ],
            )
            .map_err(store_err)?;
        Ok(changed == 1)
    }

    fn pending_approvals(&self, project: Option<&ProjectId>) -> Result<Vec<Approval>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {APPROVAL_COLUMNS} FROM approvals a JOIN sessions s ON s.id = a.session_id
                 WHERE a.decision IS NULL AND (?1 IS NULL OR s.project_id = ?1)
                 ORDER BY a.created_at, a.id"
            ))
            .map_err(store_err)?;
        stmt.query_map([project.map(ProjectId::as_str)], approval_row)
            .map_err(store_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_err)
    }

    fn cancel_pending_approvals(&self, at: Timestamp) -> Result<Vec<Approval>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(store_err)?;
        let pending = {
            let mut stmt = tx
                .prepare(&format!(
                    "SELECT {APPROVAL_COLUMNS} FROM approvals a WHERE a.decision IS NULL
                     ORDER BY a.created_at, a.id"
                ))
                .map_err(store_err)?;
            stmt.query_map([], approval_row)
                .map_err(store_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_err)?
        };
        tx.execute(
            "UPDATE approvals SET decision = ?1, decided_at = ?2 WHERE decision IS NULL",
            params![outcome_name(ApprovalOutcome::Cancelled), at.to_string()],
        )
        .map_err(store_err)?;
        tx.commit().map_err(store_err)?;
        Ok(pending)
    }
}

const APPROVAL_COLUMNS: &str = "a.id, a.session_id, a.action_json, a.reason, a.created_at";

fn approval_row(r: &Row<'_>) -> rusqlite::Result<Approval> {
    let text = |i: usize| r.get::<_, String>(i);
    let bad = |i: usize, e: String| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, e.into())
    };
    Ok(Approval {
        id: text(0)?
            .parse()
            .map_err(|e: forge_api::InvalidId| bad(0, e.to_string()))?,
        session_id: text(1)?
            .parse()
            .map_err(|e: forge_api::InvalidId| bad(1, e.to_string()))?,
        action: serde_json::from_str(&text(2)?).map_err(|e| bad(2, e.to_string()))?,
        reason: text(3)?,
        created_at: text(4)?.parse().map_err(|e| bad(4, format!("{e}")))?,
    })
}

fn outcome_name(outcome: ApprovalOutcome) -> &'static str {
    match outcome {
        ApprovalOutcome::Allow => "allow",
        ApprovalOutcome::Deny => "deny",
        ApprovalOutcome::Cancelled => "cancelled",
    }
}

fn session_row(r: &Row<'_>) -> rusqlite::Result<Session> {
    let text = |i: usize| r.get::<_, String>(i);
    let bad = |i: usize, e: String| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, e.into())
    };
    Ok(Session {
        id: text(0)?
            .parse()
            .map_err(|e: forge_api::InvalidId| bad(0, e.to_string()))?,
        project_id: text(1)?
            .parse()
            .map_err(|e: forge_api::InvalidId| bad(1, e.to_string()))?,
        title: text(2)?,
        agent_profile: text(3)?,
        state: from_wire(&text(4)?).map_err(|e| bad(4, e))?,
        mode: from_wire(&text(5)?).map_err(|e| bad(5, e))?,
        sandbox: from_wire(&text(6)?).map_err(|e| bad(6, e))?,
        branch: text(7)?,
        usage: Usage {
            input_tokens: r.get::<_, i64>(8)?.max(0) as u64,
            output_tokens: r.get::<_, i64>(9)?.max(0) as u64,
            cost_usd: r.get(10)?,
        },
        created_at: text(11)?.parse().map_err(|e| bad(11, format!("{e}")))?,
        updated_at: text(12)?.parse().map_err(|e| bad(12, format!("{e}")))?,
    })
}

/// An enum's wire spelling, as stored in text columns.
fn wire<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        other => unreachable!("expected a string enum, got {other:?}"),
    }
}

fn from_wire<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).map_err(|e| e.to_string())
}

fn store_err(e: rusqlite::Error) -> StoreError {
    StoreError(e.to_string())
}

/// Apply pending migrations, tracking progress in `PRAGMA user_version`.
fn migrate(conn: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let known = MIGRATIONS.len() as i64;
    if current > known {
        bail!(
            "database schema version {current} is newer than this daemon supports ({known}); upgrade Forge"
        );
    }
    for (version, sql) in (1..).zip(MIGRATIONS).skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {version}"))?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, branch: &str) -> Repo {
        Repo {
            name: name.into(),
            path: ".".into(),
            base_branch: branch.into(),
        }
    }

    #[tokio::test]
    async fn upsert_keeps_id_and_replaces_repos() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("forge.db")).unwrap();
        let first = db
            .upsert_project("/p".into(), "p".into(), vec![repo("p", "main")])
            .await
            .unwrap();
        let second = db
            .upsert_project("/p".into(), "q".into(), vec![repo("p", "trunk")])
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(db.list_projects().await.unwrap(), vec![second.clone()]);
        assert_eq!(
            db.get_project(second.id.clone()).await.unwrap(),
            Some(second)
        );
        assert_eq!(db.get_project(ProjectId::new()).await.unwrap(), None);
    }

    #[test]
    fn migrations_are_idempotent_and_refuse_newer_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        drop(Db::open(&path).unwrap());
        drop(Db::open(&path).unwrap());

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1)
            .unwrap();
        drop(conn);
        let err = Db::open(&path).err().unwrap();
        assert!(format!("{err:#}").contains("newer"), "{err:#}");
    }

    #[test]
    fn sessions_round_trip() {
        use forge_api::model::{Mode, SandboxTier};
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("forge.db")).unwrap();
        let project = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(db.upsert_project("/p".into(), "p".into(), vec![repo("p", "main")]))
            .unwrap();
        let session = Session {
            id: SessionId::new(),
            project_id: project.id.clone(),
            title: "t".into(),
            agent_profile: "fake".into(),
            state: SessionState::Provisioning,
            mode: Mode::AcceptEdits,
            sandbox: SandboxTier::Unsandboxed,
            branch: "feat/t".into(),
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
            },
        };
        let wt = WorktreeRecord {
            repo: "p".into(),
            path: "/p/.worktrees/feat/t".into(),
        };
        db.insert(&session, std::slice::from_ref(&wt)).unwrap();
        assert_eq!(db.get(&session.id).unwrap(), Some(session.clone()));
        assert_eq!(
            db.worktrees(&session.id).unwrap(),
            std::slice::from_ref(&wt)
        );
        assert_eq!(db.live_owner(&wt.path).unwrap(), Some(session.id.clone()));
        assert_eq!(
            db.in_states(&[SessionState::Provisioning]).unwrap().len(),
            1
        );

        let at = Timestamp::now();
        db.set_state(&session.id, SessionState::Done, at).unwrap();
        let done = db.get(&session.id).unwrap().unwrap();
        assert_eq!((done.state, done.updated_at), (SessionState::Done, at));
        assert_eq!(db.live_owner(&wt.path).unwrap(), None);
        assert_eq!(db.list(&project.id, None).unwrap().len(), 1);
        assert!(
            db.list(&project.id, Some(&[SessionState::Ready]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn approvals_end_once_and_list_while_pending() {
        use forge_api::model::{Action, Mode, SandboxTier};
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("forge.db")).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let project = rt
            .block_on(db.upsert_project("/p".into(), "p".into(), vec![repo("p", "main")]))
            .unwrap();
        let other = rt
            .block_on(db.upsert_project("/q".into(), "q".into(), vec![repo("q", "main")]))
            .unwrap();
        let session = Session {
            id: SessionId::new(),
            project_id: project.id.clone(),
            title: "t".into(),
            agent_profile: "fake".into(),
            state: SessionState::Working,
            mode: Mode::Ask,
            sandbox: SandboxTier::Unsandboxed,
            branch: "feat/t".into(),
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
            },
        };
        db.insert(&session, &[]).unwrap();
        db.set_mode(&session.id, Mode::FullAccess, Timestamp::now())
            .unwrap();
        assert_eq!(db.get(&session.id).unwrap().unwrap().mode, Mode::FullAccess);

        let approval = |argv: &str| Approval {
            id: ApprovalId::new(),
            session_id: session.id.clone(),
            action: Action::Exec {
                argv: vec![argv.into()],
                cwd: ".".into(),
            },
            reason: "why".into(),
            created_at: Timestamp::now(),
        };
        let (a, b) = (approval("a"), approval("b"));
        db.insert_approval(&a).unwrap();
        db.insert_approval(&b).unwrap();
        assert_eq!(db.pending_approvals(None).unwrap(), [a.clone(), b.clone()]);
        assert_eq!(db.pending_approvals(Some(&project.id)).unwrap().len(), 2);
        assert!(db.pending_approvals(Some(&other.id)).unwrap().is_empty());

        let at = Timestamp::now();
        assert!(
            db.end_approval(&a.id, ApprovalOutcome::Allow, Some(Remember::Session), at)
                .unwrap()
        );
        assert!(
            !db.end_approval(&a.id, ApprovalOutcome::Deny, None, at)
                .unwrap()
        );
        let record = db.approval(&a.id).unwrap().unwrap();
        assert_eq!((record.approval, record.decided), (a, true));
        assert_eq!(
            db.pending_approvals(None).unwrap(),
            std::slice::from_ref(&b)
        );
        assert_eq!(
            db.cancel_pending_approvals(at).unwrap(),
            std::slice::from_ref(&b)
        );
        assert!(db.pending_approvals(None).unwrap().is_empty());
        assert!(db.approval(&b.id).unwrap().unwrap().decided);
        assert_eq!(db.approval(&ApprovalId::new()).unwrap(), None);
    }

    #[test]
    fn uses_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        let db = Db::open(&path).unwrap();
        let conn = db.conn.lock().unwrap();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
}
