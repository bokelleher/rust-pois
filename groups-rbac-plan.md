# Plan: Groups + RBAC (generic multi-tenant grouping)

_Status: approved 2026-06-04. Implemented in phases (each its own release). Phase 1 = data model + identity primitive + groups/membership CRUD (additive, no change to existing scoping)._

## Context

POIS is single-level multi-tenant today: every channel/rule/template/project has an
`owner_user_id`, and a user (role `'admin'` or `'user'`) sees only what they own;
admins see everything (full inventory in the exploration above). The target customer
is **Gray Media — ~113 stations + 3 national channels**. They need users segregated by
**station group**, but the RBAC must stay **generic** (groups are arbitrary, not
hard-coded to stations).

This plan adds a **groups** layer: users belong to one or more groups; resources are
**published to one or more groups** (many-to-many); a per-membership **group-admin**
role lets a user manage users/channels/rules/projects within their group(s); and a
single global **super-admin** retains org-wide control.

**Confirmed decisions:**
- **Per-resource multi-group sharing** (M2M): a channel/project/template can be visible
  to N groups, plus an `is_global` escape hatch for org-wide/national content.
- **Role lives on membership**: `group_members.role ∈ ('admin','member')`. A user can be
  admin of one group and a member of another. Global `users.role='admin'` = super-admin.
- **Channel names stay globally UNIQUE** → ESAM ingest routing (name → channel → groups)
  is unchanged and unambiguous.
- **Group admins can create user accounts** (temp password), manage membership/roles, and
  manage resources — only within their own group(s).

## Core model

**Identity:** keep the JWT minimal (`sub`, `username`, global `role`, `token_type`) — do
NOT bake group membership into tokens (avoids staleness; API tokens are long-lived). Resolve
**effective identity from the DB per request**.

**Access rules (the whole model in four lines):**
- **super-admin** (`claims.role == "admin"`): bypasses all group checks (unchanged).
- **read** a resource ⇔ super-admin OR owner OR `is_global` OR resource is shared to a group
  the user is a **member** of.
- **write** a resource ⇔ super-admin OR owner OR user is a **group-admin** of a group the
  resource is shared to.
- **events / rules** inherit their channel's groups (events via `channel_name → channel`,
  rules via `channel_id`).

## Data model — migration `0011_groups_rbac.sql`

```sql
CREATE TABLE groups (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  description TEXT,
  deleted_at TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE group_members (
  group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  user_id  INTEGER NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role     TEXT NOT NULL DEFAULT 'member' CHECK(role IN ('admin','member')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (group_id, user_id)
);
CREATE INDEX idx_group_members_user ON group_members(user_id);

-- Resource ↔ group publish links (M2M). Same shape for each entity.
CREATE TABLE channel_groups  (channel_id  INTEGER NOT NULL REFERENCES channels(id)  ON DELETE CASCADE,
                              group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                              PRIMARY KEY (channel_id, group_id));
CREATE TABLE project_groups  (project_id  INTEGER NOT NULL REFERENCES projects(id)  ON DELETE CASCADE,
                              group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                              PRIMARY KEY (project_id, group_id));
CREATE TABLE template_groups (template_id INTEGER NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
                              group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                              PRIMARY KEY (template_id, group_id));
CREATE INDEX idx_channel_groups_group  ON channel_groups(group_id);
CREATE INDEX idx_project_groups_group  ON project_groups(group_id);
CREATE INDEX idx_template_groups_group ON template_groups(group_id);

-- Org-wide escape hatch (national content / system defaults visible to all).
ALTER TABLE channels  ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;
ALTER TABLE projects  ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;
ALTER TABLE templates ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0;
```

Notes:
- `users.role` is unchanged (`'admin'`=super, `'user'`=normal). Group-admin is NOT a user
  role; it's `group_members.role='admin'`.
- Keep `owner_user_id` on every entity for **attribution** (creator) even though visibility
  is now group-driven.
- `templates.is_shared` / `projects.is_shared` are **reinterpreted**: existing "shared to
  everyone" rows are migrated to `is_global=1` (see Migration). New sharing is via the
  `*_groups` links. `templates.is_default` (gallery featuring) stays as-is.
- esam_events get NO new column — scoped via `channel_name → channels → channel_groups`
  (names are globally unique, so this resolves to exactly one channel). See "What you're
  missing" #3 for the historical-events edge.

## Effective-identity resolution (the reusable primitive)

New `src/rbac.rs`:
```rust
pub struct Eff { pub uid: i64, pub super_admin: bool,
                pub member_of: Vec<i64>, pub admin_of: Vec<i64> }
pub async fn effective(db: &Pool<Sqlite>, claims: &Claims) -> Eff
// super_admin = claims.role=="admin"; otherwise one query:
//   SELECT group_id, role FROM group_members WHERE user_id = ?
```
Plus SQL-fragment helpers that every list/visibility query reuses (so the M2M predicate is
written once):
- `visible_channel_ids(eff)` / a pushable `EXISTS` predicate for read.
- `writable_channel(eff, channel_id)` boolean for write.
- analogous for projects/templates.

Use **`EXISTS (SELECT 1 FROM channel_groups cg WHERE cg.channel_id=… AND cg.group_id IN (…))`**
predicates (not JOIN+DISTINCT) to avoid row multiplication. Bind group-id lists via the
existing `sqlx::QueryBuilder` pattern already used in `list_templates` (template_library.rs)
and `get_recent_events` (event_logging.rs).

This replaces the ~29 `claims.role == "admin"` call sites (inventory above) with
`eff.super_admin` + group predicates. The current owner-only checks become "owner OR
group-admin-of-a-shared-group".

## Per-entity changes (reuse existing handler shapes)

- **Channels** (`src/main.rs` list/create/update/delete_channel, ~668–813): LIST → read
  predicate; CREATE → set `owner_user_id` (unchanged) **and** insert `channel_groups` rows
  for the target group(s) from the request (default to the user's sole group; super-admin
  may pass any; require explicit group if the user is in >1); UPDATE/DELETE → write predicate.
- **Rules** (`src/main.rs` ~815–1016): all ops gate on the **parent channel's** read/write
  predicate instead of rule owner. (Rules need no `*_groups` table — they inherit.)
- **Events** (`src/main.rs` list_events/get_event_detail ~1262–1390 + `event_logging.rs`
  get_recent_events): replace the "owned channel names" lookup with a visible-channels
  **subquery** scoped by `eff`. For non-super users add
  `AND channel_name IN (SELECT c.name FROM channels c WHERE <read predicate>)`.
- **Projects / Templates** (`src/template_library.rs`): extend `list_projects`,
  `list_templates` (QueryBuilder), `load_visible_template`, `reject_if_project_unwritable`,
  `reject_if_template_unwritable`, `get_project`, save/update/apply to the group predicates.
  Sharing UI sets `project_groups`/`template_groups`. **Apply** (instantiate) keeps "new rows
  owned by applier" and links the new channel/project to the **applier's group(s)**
  (or the source's groups when super-admin). `is_default` gallery = featured templates the
  user can read (their group defaults + global defaults).

## New API surface

- `src/rbac.rs` handlers (super-admin for groups; group-admin for membership within own groups):
  - `GET/POST /api/groups`, `GET/PUT/DELETE /api/groups/{id}` (super-admin; group-admin can
    rename/read own).
  - `GET /api/groups/{id}/members`, `POST /api/groups/{id}/members` (add existing user or set
    role), `DELETE /api/groups/{id}/members/{user_id}` (group-admin of that group or super).
  - `GET /api/me/groups` — the caller's `Eff` (member_of/admin_of + group names) for the UI.
- Extend **`GET /api/auth/me`** and the **login response** (`auth_handlers.rs`
  get_current_user / login) to include `groups: [{id,name,role}]` so the frontend can gate.
- **User management** (`auth_handlers.rs`): relax the `require_admin` gate to
  `require_super_or_group_admin` — a group-admin may `create_user` (temp password, returned
  once; reuse the Argon2 `PasswordService` + `password_changed_at` for force-change), and may
  `update_user`/`list_users` **only for users who share one of their admin groups**; creating
  the user auto-adds a `group_members` row in the creating group-admin's group. Super-admin
  unrestricted. Keep the user_id=1 protections; group-admins can never set `users.role='admin'`
  (no super-admin creation) nor manage users outside their groups.

## Frontend

- **New `static/groups.html`** (clone the channels.html two-pane Preact pattern): super-admin
  CRUD groups + assign members/roles; group-admin manages members of their own group(s) and
  can create accounts. Nav link in `header.js` gated on `super_admin || admin_of.length>0`.
- **Group context for creation/sharing:** a small group picker reused on channels.html and
  projects.html — when creating a channel/project/template, choose target group(s); when
  sharing, multi-select groups (plus an `is_global` toggle for super-admin).
- **Identity in the client:** `login.html`/`header.js` store `groups` from `/api/auth/me`;
  replace `ownerOf()` in projects.html and the implicit gating in channels.html with
  group-aware checks (can-write = super OR owner OR group-admin of a shared group). Backend is
  the source of truth; the frontend only hides affordances.
- **users.html:** show only users in the caller's admin groups (super sees all); role selector
  limited to non-super for group-admins; add a group-membership editor.
- **events.html:** no change beyond backend scoping (it already just renders what it's given);
  optionally add a Group filter once groups exist.

## Migration / backfill (in `0011`)
- Create a `Default` group; add **every existing user** as `member` (the existing super-admin
  also as `admin` of Default, though they bypass).
- Link every existing **owned** channel/project/template to `Default`. Leave NULL-owned
  ("system") rows unlinked → still super-admin-only (preserves today's behavior).
- **Behavior note to flag at deploy:** before, a normal user saw only their *own* resources;
  after, everyone in `Default` sees Default's resources. For the current tiny prod (admin +
  test users) this is fine and Gray will build real groups. If strict per-user isolation must
  be preserved, the alternative is one singleton group per existing owner — call this out and
  let Bo choose at execution time.

## What you're missing (gaps surfaced — review these)
1. **Resource-creation group selection.** A user in >1 group must say which group a new
   channel/project belongs to. Contract: request carries `group_ids`; default to the sole
   membership; error if ambiguous; super-admin may target any group or `is_global`.
2. **Group-admin user creation has no email/invite path.** Plan returns a **temp password**
   to the group-admin to relay, with force-change-on-first-login (reuse `password_changed_at`).
   A self-serve reset flow is out of scope unless wanted.
3. **Historical events when a channel is deleted/renamed.** Events store `channel_name`; if the
   channel row is later removed, those events stop resolving to a group (→ super-admin-only).
   Optional hardening: snapshot a single `owner_group_id` onto `esam_events` at log time. Left
   out of v1 to keep ingest simple; flag for decision.
4. **API-token + super-admin staleness.** Group membership resolves live (good). But the JWT's
   global `role` is a snapshot — demoting a super-admin doesn't take effect until token expiry.
   Also disabled/removed users: ensure the token path re-checks `users.enabled` (it checks
   revoked api_tokens today; add an enabled check).
5. **`is_global` vs the 113-row alternative.** National channels use `is_global=1` (one flag)
   rather than 113 `channel_groups` rows. Confirm `is_global` is acceptable (it's the pragmatic
   "share to everyone"); pure-M2M-only would require linking to every group.
6. **Group deletion semantics.** Deleting a group cascades its membership + resource links
   (ON DELETE CASCADE). A resource shared *only* to that group becomes owner/super-only. Decide
   whether to block deletion of non-empty groups.
7. **Scale.** ~116 groups, many channels/events. Indexes added above; use EXISTS predicates;
   the events subquery avoids a giant IN-list. Validate query plans on a seeded 113-group DB.
8. **Channel-name global uniqueness is load-bearing** for ingest attribution — document it as a
   constraint Gray must follow (callsign-qualified names). Two groups cannot reuse a name.
9. **Rollout is multi-release**, not one deploy (see Phasing). Each phase is independently
   shippable and reversible.

## Phasing (each phase = its own release, mirroring the project's flow)
1. **Model + identity:** migration `0011`, `src/rbac.rs` (Eff + predicates), groups/membership
   CRUD, `/api/me/groups`, extend `/api/auth/me`. No change to existing scoping yet (additive).
2. **Channels + rules + events** group-aware scoping & creation/sharing.
3. **Projects + templates** group-aware; reconcile `is_shared`→`is_global`/links.
4. **Group-admin role enforcement + user management** (create accounts, scoped users.html API).
5. **Frontend:** groups.html, group pickers, identity in client, nav gating, users.html.
6. **Backfill verification + docs + Gray seed script** (113 stations + 3 national as `is_global`).

## Verification
- **Unit/integration (Rust):** `effective()` resolution; read/write predicate truth tables for
  super / group-admin / member / non-member; rule inheritance; events subquery scoping.
- **API matrix (curl/python, seed admin via `POIS_SEED_ADMIN_USER/_PASS`):** create groups G1/G2;
  users u1(admin G1), u2(member G1), u3(member G2); a channel in G1, a national channel
  `is_global`. Assert: u2 sees G1 + national, not G2; u1 can edit G1 + manage G1 users + create
  a G1 account; u2 cannot edit; events filtered to visible channels; u1 cannot touch G2 or make a
  super-admin. Reuse the local-server harness (`setsid … & pidfile`, kill stale by PID — see
  prior gotchas).
- **Frontend:** jsdom render of groups.html + group pickers; nav gating by `Eff`; ownerOf→group.
- **Scale smoke:** seed 113 groups + channels, time `list_channels`/`list_events` for a national
  vs single-station user; confirm index usage.
- Per phase: `cargo test` green, deploy to `/opt/pois`, migration level check, rollback bak.
