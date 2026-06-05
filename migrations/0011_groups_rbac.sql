-- migrations/0011_groups_rbac.sql
-- Groups + RBAC (generic multi-tenant grouping). See groups-rbac-plan.md.
--
-- Phase 1 is ADDITIVE: it creates the group tables, the resource<->group publish
-- links, an is_global escape hatch, and backfills a "Default" group so existing
-- behavior is preserved. Scoping queries are NOT changed in this phase.
--
-- Model: users belong to one or more groups (group_members, with a per-membership
-- role admin|member). Resources (channels/projects/templates) publish to one or
-- more groups via the *_groups join tables. is_global = visible to all groups
-- (national content / system defaults). users.role stays 'admin' (super-admin) |
-- 'user'; group-admin is group_members.role='admin', NOT a user role.

CREATE TABLE IF NOT EXISTS groups (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL UNIQUE,
  description TEXT,
  deleted_at  TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS group_members (
  group_id   INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  user_id    INTEGER NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role       TEXT NOT NULL DEFAULT 'member' CHECK(role IN ('admin','member')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (group_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_group_members_user ON group_members(user_id);

-- Resource <-> group publish links (many-to-many).
CREATE TABLE IF NOT EXISTS channel_groups (
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  group_id   INTEGER NOT NULL REFERENCES groups(id)   ON DELETE CASCADE,
  PRIMARY KEY (channel_id, group_id)
);
CREATE TABLE IF NOT EXISTS project_groups (
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  group_id   INTEGER NOT NULL REFERENCES groups(id)   ON DELETE CASCADE,
  PRIMARY KEY (project_id, group_id)
);
CREATE TABLE IF NOT EXISTS template_groups (
  template_id INTEGER NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
  group_id    INTEGER NOT NULL REFERENCES groups(id)    ON DELETE CASCADE,
  PRIMARY KEY (template_id, group_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_groups_group  ON channel_groups(group_id);
CREATE INDEX IF NOT EXISTS idx_project_groups_group  ON project_groups(group_id);
CREATE INDEX IF NOT EXISTS idx_template_groups_group ON template_groups(group_id);

-- Org-wide escape hatch (national channels, global default templates).
ALTER TABLE channels  ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;
ALTER TABLE projects  ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;
ALTER TABLE templates ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;

-- ---- Backfill so existing behavior is preserved ----

-- A Default group every current user belongs to.
INSERT INTO groups(name, description) VALUES ('Default', 'Auto-created on the groups migration; holds pre-existing users and resources.');

-- All existing users become members of Default; existing super-admins become
-- group-admins of Default too (they bypass group checks anyway).
INSERT INTO group_members(group_id, user_id, role)
SELECT (SELECT id FROM groups WHERE name='Default'), u.id,
       CASE WHEN u.role='admin' THEN 'admin' ELSE 'member' END
FROM users u;

-- Link existing OWNED (non-system) resources to Default so their owners still see
-- them. NULL-owned ("system") rows stay unlinked => super-admin-only, as today.
INSERT INTO channel_groups(channel_id, group_id)
SELECT c.id, (SELECT id FROM groups WHERE name='Default')
FROM channels c WHERE c.owner_user_id IS NOT NULL AND c.deleted_at IS NULL;

INSERT INTO project_groups(project_id, group_id)
SELECT p.id, (SELECT id FROM groups WHERE name='Default')
FROM projects p WHERE p.owner_user_id IS NOT NULL AND p.deleted_at IS NULL;

INSERT INTO template_groups(template_id, group_id)
SELECT t.id, (SELECT id FROM groups WHERE name='Default')
FROM templates t WHERE t.owner_user_id IS NOT NULL AND t.deleted_at IS NULL;

-- Pre-existing "shared to everyone" templates/projects become org-wide (is_global).
UPDATE templates SET is_global = 1 WHERE is_shared = 1;
UPDATE projects  SET is_global = 1 WHERE is_shared = 1;
