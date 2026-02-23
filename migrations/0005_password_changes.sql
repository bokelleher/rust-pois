-- migrations/0005_password_changes.sql
-- Add password change logging

CREATE TABLE IF NOT EXISTS password_changes (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  username          TEXT NOT NULL,
  timestamp         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ip_address        TEXT,
  user_agent        TEXT,
  success           INTEGER NOT NULL DEFAULT 1,
  failure_reason    TEXT
);

CREATE INDEX IF NOT EXISTS idx_password_changes_user ON password_changes(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_password_changes_timestamp ON password_changes(timestamp DESC);
