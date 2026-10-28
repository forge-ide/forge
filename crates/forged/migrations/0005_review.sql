-- Review: rounds of requested changes, comment threads and their comments, and the files a
-- human marked viewed in each round (interfaces-and-data.md, Data model). A thread is written
-- on a range of a file at one checkpoint; where those lines are later is worked out from git,
-- not stored; blob_id is the file's blob at that checkpoint, which they're worked out from.
-- Rows are never deleted while their session exists.
CREATE TABLE review_rounds (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  n INTEGER NOT NULL, checkpoint_id TEXT NOT NULL, created_at TEXT NOT NULL,
  PRIMARY KEY (session_id, n)
);
CREATE TABLE threads (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, path TEXT NOT NULL,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  checkpoint_id TEXT NOT NULL, blob_id TEXT NOT NULL,
  status TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX threads_by_session ON threads (session_id);
CREATE TABLE comments (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  author TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX comments_by_thread ON comments (thread_id);
CREATE TABLE review_viewed (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  round INTEGER NOT NULL, repo_name TEXT NOT NULL, path TEXT NOT NULL,
  PRIMARY KEY (session_id, round, repo_name, path)
);
