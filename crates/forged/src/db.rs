//! SQLite: connection setup, migrations, and the project store.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use forge_api::model::{Project, Repo};
use forge_api::{ProjectId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};

/// Migrations in order. Never edit one that has shipped; add a new file instead.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_projects.sql")];

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
