-- Checkpoints: snapshots of a session's worktrees after each turn, as commits on shadow refs
-- (interfaces-and-data.md, Data model). n counts up per session and is never reused.
-- discarded_by is NULL while the checkpoint is part of the session's history, and names the
-- checkpoint a rewind went back to once one discarded it; its refs stay in git.
CREATE TABLE checkpoints (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  n INTEGER NOT NULL, turn_id TEXT, created_at TEXT NOT NULL,
  discarded_by TEXT,
  UNIQUE (session_id, n)
);
CREATE TABLE checkpoint_refs (
  checkpoint_id TEXT NOT NULL REFERENCES checkpoints(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, commit_sha TEXT NOT NULL,
  PRIMARY KEY (checkpoint_id, repo_name)
);
