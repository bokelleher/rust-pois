-- migrations/0009_templates_and_projects.sql
-- Template library + projects.
--
-- A "project" is a persistent, shareable container that bundles multiple
-- channels (each with their rules). A "template" is a reusable entity captured
-- from a live row:
--   kind='rule'    -> body_json is a RuleBackup        (applied into a channel)
--   kind='channel' -> body_json is a ChannelFullBackup (channel + its rules)
-- A template may belong to a project (project_id) or be an unfiled library item.
-- Ownership/soft-delete mirror the multi-tenancy pattern from migration 0004;
-- is_shared exposes a row to all users (apply-only for non-owners).

CREATE TABLE IF NOT EXISTS projects (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  name          TEXT NOT NULL,
  description   TEXT,
  is_shared     INTEGER NOT NULL DEFAULT 0,
  owner_user_id INTEGER REFERENCES users(id),
  deleted_at    TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS templates (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  name          TEXT NOT NULL,
  kind          TEXT NOT NULL,                 -- 'rule' | 'channel'
  description   TEXT,
  project_id    INTEGER REFERENCES projects(id) ON DELETE SET NULL,
  body_json     TEXT NOT NULL,                 -- RuleBackup | ChannelFullBackup
  is_shared     INTEGER NOT NULL DEFAULT 0,
  owner_user_id INTEGER REFERENCES users(id),
  deleted_at    TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_templates_owner   ON templates(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_templates_deleted ON templates(deleted_at);
CREATE INDEX IF NOT EXISTS idx_templates_project ON templates(project_id);
CREATE INDEX IF NOT EXISTS idx_projects_owner    ON projects(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_projects_deleted  ON projects(deleted_at);
