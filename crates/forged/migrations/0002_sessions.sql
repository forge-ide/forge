-- Sessions, their worktrees, and the event index (interfaces-and-data.md, Data model).
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title TEXT NOT NULL, agent_profile TEXT NOT NULL,
  state TEXT NOT NULL, mode TEXT NOT NULL, sandbox TEXT NOT NULL,
  branch TEXT NOT NULL, acp_session_id TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL, last_seq INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX sessions_by_project ON sessions (project_id, state);
CREATE TABLE worktrees (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, path TEXT NOT NULL,
  PRIMARY KEY (session_id, repo_name)
);
CREATE INDEX worktrees_by_path ON worktrees (path);
-- stream_seq is the daemon-wide cursor that event notifications carry and `afterSeq` resumes
-- from; seq counts within one session.
CREATE TABLE events_index (
  session_id TEXT NOT NULL, seq INTEGER NOT NULL, kind TEXT NOT NULL,
  at TEXT NOT NULL, tool TEXT, path TEXT, byte_offset INTEGER NOT NULL,
  stream_seq INTEGER NOT NULL UNIQUE,
  PRIMARY KEY (session_id, seq)
);
CREATE VIRTUAL TABLE events_fts USING fts5(session_id UNINDEXED, seq UNINDEXED, text);
