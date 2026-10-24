-- Approvals: actions waiting on a human, and how each ended (interfaces-and-data.md, Data
-- model). decision is NULL while pending, then 'allow', 'deny', or 'cancelled'.
CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  action_json TEXT NOT NULL, reason TEXT NOT NULL,
  decision TEXT, remember TEXT, decided_at TEXT, created_at TEXT NOT NULL
);
