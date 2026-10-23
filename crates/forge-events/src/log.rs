//! The event log: one JSONL file per session as the source of truth, indexed in SQLite.
//!
//! Appending an event redacts it, gives it the next per-session `seq` and the next daemon-wide
//! stream cursor, writes it to the session's JSONL file, indexes it, and publishes it to the
//! hub, all under one lock so subscribers see events in cursor order. Replay reads the index
//! for events after a cursor and loads each from its file.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use forge_api::events::{Event, EventKind, EventPayload, Topic};
use forge_api::model::Action;
use forge_api::{ProjectId, SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::hub::Hub;

/// The daemon's database connection, shared with the code that owns the rest of the schema.
pub type SharedConn = Arc<Mutex<Connection>>;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("session {0} has no open event log")]
    NotRegistered(SessionId),
    #[error("session {0} not found")]
    UnknownSession(SessionId),
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("event index: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("event log {path}: {message}")]
    Corrupt { path: PathBuf, message: String },
}

/// Rewrites event bodies before they are written or published, e.g. to replace secrets with
/// placeholders.
pub trait Redactor: Send + Sync {
    fn redact(&self, payload: &mut EventPayload);
}

/// A [`Redactor`] that changes nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRedaction;

impl Redactor for NoRedaction {
    fn redact(&self, _: &mut EventPayload) {}
}

/// Where a session's event log lives.
pub fn log_path(project_root: &Path, session: &SessionId) -> PathBuf {
    project_root
        .join(".forge/sessions")
        .join(session.as_str())
        .join("events.jsonl")
}

/// The topics an event is published on, most specific first.
pub fn topics(session: &SessionId, project: &ProjectId, kind: EventKind) -> Vec<Topic> {
    let board = matches!(kind, EventKind::SessionState | EventKind::Usage);
    let approvals = matches!(
        kind,
        EventKind::ApprovalRequested | EventKind::ApprovalDecided
    );
    let mut topics = vec![Topic::Session(session.clone())];
    if board || approvals {
        topics.push(Topic::Project(project.clone()));
    }
    if board {
        topics.push(Topic::Board);
    }
    if approvals {
        topics.push(Topic::Approvals);
    }
    topics
}

/// One line of a JSONL log: the event plus its stream cursor.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogRecord {
    stream_seq: u64,
    #[serde(flatten)]
    event: Event,
}

/// An event replayed for a subscription.
#[derive(Clone, Debug, PartialEq)]
pub struct Replayed {
    pub stream_seq: u64,
    pub topic: Topic,
    pub event: Event,
}

/// Filters for [`EventLog::query`].
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub kinds: Option<Vec<EventKind>>,
    /// Full-text match over messages, prompts, and output.
    pub text: Option<String>,
    /// Only events with a lower `seq`.
    pub before: Option<u64>,
    pub limit: u32,
}

struct OpenLog {
    project: ProjectId,
    path: PathBuf,
    file: File,
    next_seq: u64,
    offset: u64,
}

struct Inner {
    next_stream: u64,
    sessions: HashMap<SessionId, OpenLog>,
}

/// Every session's event log and the index over them.
pub struct EventLog {
    inner: Mutex<Inner>,
    conn: SharedConn,
    hub: Arc<Hub>,
    redactor: Box<dyn Redactor>,
}

impl EventLog {
    /// Open the log over an already-migrated database.
    pub fn open(
        conn: SharedConn,
        hub: Arc<Hub>,
        redactor: Box<dyn Redactor>,
    ) -> Result<Self, LogError> {
        let last: Option<u64> =
            lock(&conn).query_row("SELECT MAX(stream_seq) FROM events_index", [], |r| r.get(0))?;
        Ok(Self {
            inner: Mutex::new(Inner {
                next_stream: last.unwrap_or(0) + 1,
                sessions: HashMap::new(),
            }),
            conn,
            hub,
            redactor,
        })
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }

    /// The cursor of the newest event, or 0 if there are none.
    pub fn head(&self) -> u64 {
        lock(&self.inner).next_stream - 1
    }

    /// Open `session`'s log for appending, creating it if needed. Its project root comes from
    /// the database.
    pub fn register(&self, session: &SessionId) -> Result<(), LogError> {
        let mut inner = lock(&self.inner);
        if inner.sessions.contains_key(session) {
            return Ok(());
        }
        let (project, root) = session_home(&lock(&self.conn), session)?;
        let path = log_path(&root, session);
        let dir = path.parent().expect("log path has a parent");
        std::fs::create_dir_all(dir).map_err(io(format!("creating {}", dir.display())))?;
        ignore_sessions_dir(&root)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(io(format!("opening {}", path.display())))?;
        let offset = file
            .metadata()
            .map_err(io(format!("reading {}", path.display())))?
            .len();
        let last: Option<u64> = lock(&self.conn).query_row(
            "SELECT MAX(seq) FROM events_index WHERE session_id = ?1",
            [session.as_str()],
            |r| r.get(0),
        )?;
        inner.sessions.insert(
            session.clone(),
            OpenLog {
                project,
                path,
                file,
                next_seq: last.unwrap_or(0) + 1,
                offset,
            },
        );
        Ok(())
    }

    /// Close `session`'s log file. Appending reopens nothing; call [`register`](Self::register).
    pub fn unregister(&self, session: &SessionId) {
        lock(&self.inner).sessions.remove(session);
    }

    /// Append an event to a registered session's log and publish it. Blocking.
    pub fn append(
        &self,
        session: &SessionId,
        parent: Option<u64>,
        mut payload: EventPayload,
    ) -> Result<Event, LogError> {
        self.redactor.redact(&mut payload);
        let mut inner = lock(&self.inner);
        let stream_seq = inner.next_stream;
        let log = inner
            .sessions
            .get_mut(session)
            .ok_or_else(|| LogError::NotRegistered(session.clone()))?;
        let event = Event {
            seq: log.next_seq,
            session_id: session.clone(),
            at: Timestamp::now(),
            parent,
            payload,
        };
        let record = LogRecord {
            stream_seq,
            event: event.clone(),
        };
        let mut line = serde_json::to_string(&record).expect("events serialize");
        line.push('\n');
        log.file
            .write_all(line.as_bytes())
            .map_err(io(format!("writing {}", log.path.display())))?;
        if let Err(e) = index(&mut lock(&self.conn), &event, stream_seq, log.offset) {
            // Keep the file and the index in step: drop the line the index doesn't know about.
            let _ = log.file.set_len(log.offset);
            return Err(e);
        }
        log.offset += line.len() as u64;
        log.next_seq += 1;
        let topics = topics(session, &log.project, event.payload.kind());
        inner.next_stream += 1;
        self.hub.publish(&topics, stream_seq, &event);
        Ok(event)
    }

    /// Async [`append`](Self::append), run on the blocking pool.
    pub async fn emit(
        self: &Arc<Self>,
        session: &SessionId,
        parent: Option<u64>,
        payload: EventPayload,
    ) -> Result<Event, LogError> {
        let (log, session) = (self.clone(), session.clone());
        tokio::task::spawn_blocking(move || log.append(&session, parent, payload))
            .await
            .expect("event append panicked")
    }

    /// Every event after cursor `after` on any of `topics`, in cursor order. Blocking.
    pub fn replay(&self, topics: &HashSet<Topic>, after: u64) -> Result<Vec<Replayed>, LogError> {
        let rows: Vec<(u64, String, String, u64, String, String)> = {
            let conn = lock(&self.conn);
            let mut stmt = conn.prepare(
                "SELECT i.stream_seq, i.session_id, i.kind, i.byte_offset, s.project_id, p.root
                 FROM events_index i
                 JOIN sessions s ON s.id = i.session_id
                 JOIN projects p ON p.id = s.project_id
                 WHERE i.stream_seq > ?1 ORDER BY i.stream_seq",
            )?;
            stmt.query_map([after], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<Result<_, _>>()?
        };
        let mut readers = Readers::default();
        let mut out = Vec::new();
        for (stream_seq, session, kind, offset, project, root) in rows {
            let (Ok(session), Ok(project), Some(kind)) = (
                session.parse::<SessionId>(),
                project.parse::<ProjectId>(),
                parse_kind(&kind),
            ) else {
                continue;
            };
            let Some(topic) = self::topics(&session, &project, kind)
                .into_iter()
                .find(|t| topics.contains(t))
            else {
                continue;
            };
            let record = readers.read(&log_path(Path::new(&root), &session), offset)?;
            out.push(Replayed {
                stream_seq,
                topic,
                event: record.event,
            });
        }
        Ok(out)
    }

    /// Up to `limit` of `session`'s events matching `query`, oldest first: the newest matches
    /// below `before`. Blocking.
    pub fn query(&self, session: &SessionId, query: &Query) -> Result<Vec<Event>, LogError> {
        let (root, offsets) = {
            let conn = lock(&self.conn);
            let (_, root) = session_home(&conn, session)?;
            let mut sql = String::from("SELECT byte_offset FROM events_index WHERE session_id = ?");
            let mut args: Vec<rusqlite::types::Value> = vec![session.as_str().to_owned().into()];
            if let Some(kinds) = &query.kinds {
                sql.push_str(" AND kind IN (");
                sql.push_str(&vec!["?"; kinds.len()].join(", "));
                sql.push(')');
                args.extend(kinds.iter().map(|k| kind_name(*k).to_owned().into()));
                if kinds.is_empty() {
                    return Ok(Vec::new());
                }
            }
            if let Some(before) = query.before {
                sql.push_str(" AND seq < ?");
                args.push((before as i64).into());
            }
            if let Some(text) = &query.text {
                sql.push_str(
                    " AND seq IN (SELECT seq FROM events_fts WHERE events_fts MATCH ? AND session_id = ?)",
                );
                // A quoted phrase, so user text is never parsed as query syntax.
                args.push(format!("\"{}\"", text.replace('"', "\"\"")).into());
                args.push(session.as_str().to_owned().into());
            }
            sql.push_str(" ORDER BY seq DESC LIMIT ?");
            args.push(i64::from(query.limit).into());
            let offsets: Vec<u64> = conn
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(args), |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            (root, offsets)
        };
        let path = log_path(&root, session);
        let mut readers = Readers::default();
        let mut events = offsets
            .into_iter()
            .map(|offset| readers.read(&path, offset).map(|r| r.event))
            .collect::<Result<Vec<_>, _>>()?;
        events.reverse();
        Ok(events)
    }
}

/// The project and project root a session belongs to.
fn session_home(conn: &Connection, session: &SessionId) -> Result<(ProjectId, PathBuf), LogError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT s.project_id, p.root FROM sessions s JOIN projects p ON p.id = s.project_id
             WHERE s.id = ?1",
            [session.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (project, root) = row.ok_or_else(|| LogError::UnknownSession(session.clone()))?;
    let project = project
        .parse()
        .map_err(|_| LogError::UnknownSession(session.clone()))?;
    Ok((project, PathBuf::from(root)))
}

/// Keep session logs out of the project's git status without touching its own ignore files.
fn ignore_sessions_dir(root: &Path) -> Result<(), LogError> {
    let path = root.join(".forge/sessions/.gitignore");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(b"# Forge session logs are local to this machine.\n*\n")
            .map_err(io(format!("writing {}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(io(format!("creating {}", path.display()))(e)),
    }
}

fn index(
    conn: &mut Connection,
    event: &Event,
    stream_seq: u64,
    offset: u64,
) -> Result<(), LogError> {
    let (tool, path) = match &event.payload {
        EventPayload::ToolCall(b) => {
            let path = match &b.action {
                Action::Edit { path, .. } => Some(path.clone()),
                _ => None,
            };
            (Some(action_type(&b.action)), path)
        }
        EventPayload::Edit(b) => (None, Some(b.path.clone())),
        _ => (None, None),
    };
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO events_index (session_id, seq, kind, at, tool, path, byte_offset, stream_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event.session_id.as_str(),
            event.seq,
            kind_name(event.payload.kind()),
            event.at.to_string(),
            tool,
            path,
            offset,
            stream_seq,
        ],
    )?;
    if let Some(text) = searchable_text(&event.payload) {
        tx.execute(
            "INSERT INTO events_fts (session_id, seq, text) VALUES (?1, ?2, ?3)",
            params![event.session_id.as_str(), event.seq, text],
        )?;
    }
    tx.execute(
        "UPDATE sessions SET last_seq = ?1 WHERE id = ?2",
        params![event.seq, event.session_id.as_str()],
    )?;
    tx.commit()?;
    Ok(())
}

fn searchable_text(payload: &EventPayload) -> Option<&str> {
    match payload {
        EventPayload::Prompt(b) => Some(&b.text),
        EventPayload::AgentMessage(b) => Some(&b.delta),
        EventPayload::ToolCall(b) => Some(&b.title),
        EventPayload::ToolResult(b) => b.output.as_deref(),
        EventPayload::TerminalOutput(b) => Some(&b.chunk),
        EventPayload::SessionState(b) => b.reason.as_deref(),
        EventPayload::Error(b) => Some(&b.detail),
        _ => None,
    }
}

fn action_type(action: &Action) -> &'static str {
    match action {
        Action::Edit { .. } => "edit",
        Action::Exec { .. } => "exec",
        Action::Network { .. } => "network",
        Action::Mcp { .. } => "mcp",
        Action::Git { .. } => "git",
    }
}

fn kind_name(kind: EventKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(name)) => name,
        _ => unreachable!("event kinds serialize as strings"),
    }
}

fn parse_kind(name: &str) -> Option<EventKind> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

/// Reads records by byte offset, keeping each file open across reads.
#[derive(Default)]
struct Readers {
    open: HashMap<PathBuf, BufReader<File>>,
}

impl Readers {
    fn read(&mut self, path: &Path, offset: u64) -> Result<LogRecord, LogError> {
        let reader = match self.open.get_mut(path) {
            Some(reader) => reader,
            None => {
                let file = File::open(path).map_err(io(format!("opening {}", path.display())))?;
                self.open
                    .entry(path.to_path_buf())
                    .or_insert(BufReader::new(file))
            }
        };
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(io(format!("reading {}", path.display())))?;
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(io(format!("reading {}", path.display())))?;
        serde_json::from_str(&line).map_err(|e| LogError::Corrupt {
            path: path.to_path_buf(),
            message: format!("at byte {offset}: {e}"),
        })
    }
}

fn io(context: String) -> impl FnOnce(std::io::Error) -> LogError {
    move |source| LogError::Io { context, source }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the lock leaves the guarded state usable.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_api::events::{AgentMessageBody, PromptBody, SessionStateBody};
    use forge_api::model::SessionState;
    use forge_api::{SubscriptionId, TurnId};

    const MIGRATIONS: [&str; 2] = [
        include_str!("../../forged/migrations/0001_projects.sql"),
        include_str!("../../forged/migrations/0002_sessions.sql"),
    ];

    struct Fixture {
        dir: tempfile::TempDir,
        conn: SharedConn,
        project: ProjectId,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let conn = Connection::open(dir.path().join("forge.db")).unwrap();
            for sql in MIGRATIONS {
                conn.execute_batch(sql).unwrap();
            }
            let root = dir.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let project = ProjectId::new();
            conn.execute(
                "INSERT INTO projects (id, name, root, created_at) VALUES (?1, 'p', ?2, 'now')",
                params![project.as_str(), root.to_str().unwrap()],
            )
            .unwrap();
            Self {
                dir,
                conn: Arc::new(Mutex::new(conn)),
                project,
            }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("project")
        }

        fn session(&self, log: &EventLog) -> SessionId {
            let id = SessionId::new();
            lock(&self.conn)
                .execute(
                    "INSERT INTO sessions (id, project_id, title, agent_profile, state, mode, sandbox,
                       branch, created_at, updated_at)
                     VALUES (?1, ?2, 't', 'fake', 'ready', 'ask', 'unsandboxed', 'b', 'now', 'now')",
                    params![id.as_str(), self.project.as_str()],
                )
                .unwrap();
            log.register(&id).unwrap();
            id
        }

        fn log(&self) -> EventLog {
            self.log_with(Box::new(NoRedaction))
        }

        fn log_with(&self, redactor: Box<dyn Redactor>) -> EventLog {
            EventLog::open(self.conn.clone(), Arc::new(Hub::new()), redactor).unwrap()
        }
    }

    fn state(to: SessionState) -> EventPayload {
        EventPayload::SessionState(SessionStateBody {
            from: SessionState::Ready,
            to,
            reason: None,
        })
    }

    fn message(text: &str) -> EventPayload {
        EventPayload::AgentMessage(AgentMessageBody {
            turn_id: TurnId::new(),
            message_id: "m".into(),
            delta: text.into(),
        })
    }

    #[test]
    fn appends_number_events_per_session_and_across_the_stream() {
        let fx = Fixture::new();
        let log = fx.log();
        let (a, b) = (fx.session(&log), fx.session(&log));
        assert_eq!(log.head(), 0);
        let e1 = log.append(&a, None, message("one")).unwrap();
        let e2 = log.append(&b, None, message("two")).unwrap();
        let e3 = log.append(&a, Some(1), message("three")).unwrap();
        assert_eq!((e1.seq, e2.seq, e3.seq), (1, 1, 2));
        assert_eq!(e3.parent, Some(1));
        assert_eq!(log.head(), 3);

        let text = std::fs::read_to_string(log_path(&fx.root(), &a)).unwrap();
        let records: Vec<LogRecord> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].stream_seq, records[1].stream_seq), (1, 3));
        assert_eq!(records[1].event, e3);
        // Each line is also a plain event.
        let plain: Event = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(plain, e1);

        let gitignore = fx.root().join(".forge/sessions/.gitignore");
        assert_eq!(
            std::fs::read_to_string(gitignore).unwrap().lines().last(),
            Some("*")
        );
        let last_seq: u64 = lock(&fx.conn)
            .query_row(
                "SELECT last_seq FROM sessions WHERE id = ?1",
                [a.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last_seq, 2);

        assert!(matches!(
            log.append(&SessionId::new(), None, message("x")),
            Err(LogError::NotRegistered(_))
        ));
    }

    #[test]
    fn numbering_continues_after_reopening() {
        let fx = Fixture::new();
        let log = fx.log();
        let a = fx.session(&log);
        log.append(&a, None, message("one")).unwrap();
        log.append(&a, None, message("two")).unwrap();
        drop(log);

        let log = fx.log();
        assert_eq!(log.head(), 2);
        log.register(&a).unwrap();
        let e = log.append(&a, None, message("three")).unwrap();
        assert_eq!(e.seq, 3);
        assert_eq!(log.head(), 3);
        let all = log
            .query(
                &a,
                &Query {
                    limit: 10,
                    ..Query::default()
                },
            )
            .unwrap();
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn replay_filters_by_topic_and_cursor_and_names_the_first_match() {
        let fx = Fixture::new();
        let log = fx.log();
        let (a, b) = (fx.session(&log), fx.session(&log));
        log.append(&a, None, state(SessionState::Working)).unwrap(); // 1
        log.append(&a, None, message("hi")).unwrap(); // 2
        log.append(&b, None, state(SessionState::Working)).unwrap(); // 3
        log.append(&b, None, message("yo")).unwrap(); // 4

        let set = |t: &[Topic]| t.iter().cloned().collect::<HashSet<_>>();
        let cursors = |r: Vec<Replayed>| r.iter().map(|r| r.stream_seq).collect::<Vec<_>>();

        let board = log.replay(&set(&[Topic::Board]), 0).unwrap();
        assert_eq!(cursors(board.clone()), [1, 3]);
        assert!(board.iter().all(|r| r.topic == Topic::Board));

        let mixed = log
            .replay(&set(&[Topic::Board, Topic::Session(a.clone())]), 0)
            .unwrap();
        assert_eq!(cursors(mixed.clone()), [1, 2, 3]);
        assert_eq!(mixed[0].topic, Topic::Session(a.clone()));
        assert_eq!(mixed[2].topic, Topic::Board);
        assert!(
            matches!(&mixed[1].event.payload, EventPayload::AgentMessage(b) if b.delta == "hi")
        );

        let project = log
            .replay(&set(&[Topic::Project(fx.project.clone())]), 1)
            .unwrap();
        assert_eq!(cursors(project), [3]);
        assert!(log.replay(&set(&[Topic::Approvals]), 0).unwrap().is_empty());
        assert!(
            log.replay(&set(&[Topic::Session(b)]), 4)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn query_filters_pages_and_searches() {
        let fx = Fixture::new();
        let log = fx.log();
        let a = fx.session(&log);
        log.append(&a, None, state(SessionState::Working)).unwrap();
        log.append(
            &a,
            None,
            EventPayload::Prompt(PromptBody {
                turn_id: TurnId::new(),
                text: "add retry backoff".into(),
            }),
        )
        .unwrap();
        for i in 0..5 {
            log.append(&a, None, message(&format!("chunk {i} \"quoted\"")))
                .unwrap();
        }
        let q = |query: Query| {
            log.query(&a, &query)
                .unwrap()
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            q(Query {
                limit: 3,
                ..Query::default()
            }),
            [5, 6, 7]
        );
        assert_eq!(
            q(Query {
                limit: 2,
                before: Some(5),
                ..Query::default()
            }),
            [3, 4]
        );
        assert_eq!(
            q(Query {
                limit: 10,
                kinds: Some(vec![EventKind::SessionState, EventKind::Prompt]),
                ..Query::default()
            }),
            [1, 2]
        );
        assert_eq!(
            q(Query {
                limit: 10,
                kinds: Some(vec![]),
                ..Query::default()
            }),
            Vec::<u64>::new()
        );
        assert_eq!(
            q(Query {
                limit: 10,
                text: Some("retry".into()),
                ..Query::default()
            }),
            [2]
        );
        assert_eq!(
            q(Query {
                limit: 10,
                text: Some("chunk 3".into()),
                ..Query::default()
            }),
            [6]
        );
        // Query syntax in user text is matched literally, not parsed.
        assert_eq!(
            q(Query {
                limit: 10,
                text: Some("\"quoted\" OR".into()),
                ..Query::default()
            }),
            Vec::<u64>::new()
        );
        assert!(matches!(
            log.query(
                &SessionId::new(),
                &Query {
                    limit: 1,
                    ..Query::default()
                }
            ),
            Err(LogError::UnknownSession(_))
        ));
    }

    struct Mask;

    impl Redactor for Mask {
        fn redact(&self, payload: &mut EventPayload) {
            if let EventPayload::AgentMessage(b) = payload {
                b.delta = b.delta.replace("hunter2", "[secret]");
            }
        }
    }

    #[test]
    fn redaction_runs_before_writing_and_publishing() {
        let fx = Fixture::new();
        let log = fx.log_with(Box::new(Mask));
        let a = fx.session(&log);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        log.hub().subscribe(
            [Topic::Session(a.clone())],
            Box::new(move |n| {
                sink.lock().unwrap().push(n);
                true
            }),
        );
        let event = log.append(&a, None, message("pw is hunter2")).unwrap();
        let EventPayload::AgentMessage(body) = &event.payload else {
            panic!()
        };
        assert_eq!(body.delta, "pw is [secret]");
        let file = std::fs::read_to_string(log_path(&fx.root(), &a)).unwrap();
        assert!(!file.contains("hunter2"));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].event, event);
        assert_eq!(seen[0].seq, 1);
        let _: &SubscriptionId = &seen[0].subscription_id;
    }
}
