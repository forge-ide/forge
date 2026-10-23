-- Projects and their repos (interfaces-and-data.md, Data model).
CREATE TABLE projects (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL
);
CREATE TABLE repos (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL, path TEXT NOT NULL, base_branch TEXT NOT NULL,
  PRIMARY KEY (project_id, name)
);
