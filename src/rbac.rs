// src/rbac.rs
//! Groups + RBAC primitives and group/membership management (Phase 1).
//!
//! Identity is resolved from the DB per request (NOT baked into the JWT) so
//! membership changes and long-lived API tokens never go stale. `Eff` is the
//! effective identity used by scoping checks:
//!   - super_admin (users.role == "admin") bypasses all group checks.
//!   - member_of / admin_of are the group ids the caller belongs to / administers.
//!
//! This phase is additive: it manages groups + membership and exposes identity to
//! the frontend. It does NOT yet change channel/rule/template/project/event scoping
//! (later phases consume `Eff`).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Sqlite};

use sqlx::QueryBuilder;

use crate::jwt_auth::Claims;
use crate::AppState;

// ------------------------------- helpers ---------------------------------

fn resp<T: Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

// --------------------------- effective identity --------------------------

/// The caller's effective identity for group-scoped authorization.
pub struct Eff {
    pub uid: i64,
    pub super_admin: bool,
    pub member_of: Vec<i64>,
    pub admin_of: Vec<i64>,
}

impl Eff {
    /// May administer (write) within group `gid`.
    pub fn is_group_admin(&self, gid: i64) -> bool {
        self.super_admin || self.admin_of.contains(&gid)
    }
    /// May see (read) within group `gid`.
    pub fn is_member(&self, gid: i64) -> bool {
        self.super_admin || self.member_of.contains(&gid)
    }
}

// ------------------------- group-scoped predicates ------------------------
//
// The access model (super-admin bypasses everything):
//   READ  a resource  <=> owner OR is_global OR shared to a group I'm a MEMBER of
//   WRITE a resource  <=> owner OR group-ADMIN of a group it's shared to
// These helpers are generic over the three entities via their link tables:
//   ("channels","channel_groups","channel_id"), ("projects","project_groups",
//   "project_id"), ("templates","template_groups","template_id").
// `table`/`link_table`/`link_col` are internal string constants (never user
// input); all user values are bound.

/// Append the READ predicate to a QueryBuilder over `table` (e.g. a list query).
/// No-op for super-admins (they see everything). The base query must already be
/// inside a WHERE (we append `AND (...)`).
pub fn push_read_predicate<'a>(
    qb: &mut QueryBuilder<'a, Sqlite>,
    eff: &Eff,
    table: &str,
    link_table: &str,
    link_col: &str,
) {
    if eff.super_admin {
        return;
    }
    qb.push(" AND (")
        .push(table)
        .push(".owner_user_id = ")
        .push_bind(eff.uid)
        .push(" OR ")
        .push(table)
        .push(".is_global = 1");
    if !eff.member_of.is_empty() {
        qb.push(format!(
            " OR EXISTS(SELECT 1 FROM {link_table} lg WHERE lg.{link_col} = {table}.id AND lg.group_id IN ("
        ));
        let mut sep = qb.separated(", ");
        for g in &eff.member_of {
            sep.push_bind(*g);
        }
        qb.push("))");
    }
    qb.push(")");
}

async fn group_link_matches(
    db: &Pool<Sqlite>,
    link_table: &str,
    link_col: &str,
    id: i64,
    groups: &[i64],
) -> bool {
    if groups.is_empty() {
        return false;
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(format!(
        "SELECT COUNT(*) FROM {link_table} WHERE {link_col} = "
    ));
    qb.push_bind(id).push(" AND group_id IN (");
    let mut sep = qb.separated(", ");
    for g in groups {
        sep.push_bind(*g);
    }
    qb.push(")");
    let n: i64 = qb
        .build_query_scalar()
        .fetch_one(db)
        .await
        .unwrap_or(0);
    n > 0
}

/// True if the caller may READ resource `id` in `table`.
pub async fn can_read(
    db: &Pool<Sqlite>,
    eff: &Eff,
    table: &str,
    link_table: &str,
    link_col: &str,
    id: i64,
) -> bool {
    if eff.super_admin {
        return true;
    }
    let row: Option<(Option<i64>, i64)> = sqlx::query_as(&format!(
        "SELECT owner_user_id, is_global FROM {table} WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let Some((owner, is_global)) = row else {
        return false;
    };
    if is_global != 0 || owner == Some(eff.uid) {
        return true;
    }
    group_link_matches(db, link_table, link_col, id, &eff.member_of).await
}

/// True if the caller may WRITE (edit/delete/share) resource `id` in `table`.
pub async fn can_write(
    db: &Pool<Sqlite>,
    eff: &Eff,
    table: &str,
    link_table: &str,
    link_col: &str,
    id: i64,
) -> bool {
    if eff.super_admin {
        return true;
    }
    let owner: Option<(Option<i64>,)> = sqlx::query_as(&format!(
        "SELECT owner_user_id FROM {table} WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    match owner {
        None => return false, // not found
        Some((Some(o),)) if o == eff.uid => return true,
        _ => {}
    }
    group_link_matches(db, link_table, link_col, id, &eff.admin_of).await
}

/// Publish resource `id` to `group_ids` (idempotent).
pub async fn link_groups(
    db: &Pool<Sqlite>,
    link_table: &str,
    link_col: &str,
    id: i64,
    group_ids: &[i64],
) {
    for g in group_ids {
        let _ = sqlx::query(&format!(
            "INSERT OR IGNORE INTO {link_table}({link_col}, group_id) VALUES(?, ?)"
        ))
        .bind(id)
        .bind(*g)
        .execute(db)
        .await;
    }
}

/// The group ids a resource is currently published to (for responses/UI).
pub async fn resource_group_ids(
    db: &Pool<Sqlite>,
    link_table: &str,
    link_col: &str,
    id: i64,
) -> Vec<i64> {
    sqlx::query_as::<_, (i64,)>(&format!(
        "SELECT group_id FROM {link_table} WHERE {link_col} = ?"
    ))
    .bind(id)
    .fetch_all(db)
    .await
    .map(|v| v.into_iter().map(|(g,)| g).collect())
    .unwrap_or_default()
}

/// Map each resource id to its group ids, for a set of ids (one query). Used to
/// annotate list/detail responses so the UI can gate manage actions client-side.
pub async fn group_ids_map(
    db: &Pool<Sqlite>,
    link_table: &str,
    link_col: &str,
    ids: &[i64],
) -> std::collections::HashMap<i64, Vec<i64>> {
    let mut map: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    if ids.is_empty() {
        return map;
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(format!(
        "SELECT {link_col}, group_id FROM {link_table} WHERE {link_col} IN ("
    ));
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    qb.push(")");
    let rows: Vec<(i64, i64)> = qb.build_query_as().fetch_all(db).await.unwrap_or_default();
    for (rid, gid) in rows {
        map.entry(rid).or_default().push(gid);
    }
    map
}

/// Re-publish resource `id` to `desired`, scoped to what the caller may manage.
///
/// `manageable` defines the caller's reach:
///   - `None`        — full control (super-admin): the link set becomes exactly
///                     `desired`.
///   - `Some(scope)` — only links to groups IN `scope` are replaced (set to
///                     `desired`, which must already be a subset of `scope`);
///                     links to groups OUTSIDE `scope` are PRESERVED untouched.
///
/// The scoped form stops a group-admin from clobbering a resource's shares to
/// groups they can't even see (the picker only offers groups they belong to).
pub async fn set_groups_scoped(
    db: &Pool<Sqlite>,
    link_table: &str,
    link_col: &str,
    id: i64,
    desired: &[i64],
    manageable: Option<&[i64]>,
) {
    match manageable {
        None => {
            let _ = sqlx::query(&format!("DELETE FROM {link_table} WHERE {link_col} = ?"))
                .bind(id)
                .execute(db)
                .await;
        }
        Some(scope) if !scope.is_empty() => {
            let mut qb: QueryBuilder<Sqlite> =
                QueryBuilder::new(format!("DELETE FROM {link_table} WHERE {link_col} = "));
            qb.push_bind(id).push(" AND group_id IN (");
            let mut sep = qb.separated(", ");
            for g in scope {
                sep.push_bind(*g);
            }
            qb.push(")");
            let _ = qb.build().execute(db).await;
        }
        Some(_) => { /* empty scope: nothing in-scope to remove */ }
    }
    link_groups(db, link_table, link_col, id, desired).await;
}

/// Events are scoped by the channel (resolved from `channel_name`) the caller may
/// read. Returns None for super-admins (no restriction).
pub fn event_scope(eff: &Eff) -> Option<(i64, Vec<i64>)> {
    if eff.super_admin {
        None
    } else {
        Some((eff.uid, eff.member_of.clone()))
    }
}

/// Resolve the caller's effective identity from `group_members` (one query).
pub async fn effective(db: &Pool<Sqlite>, claims: &Claims) -> Eff {
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    let super_admin = claims.role == "admin";
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT group_id, role FROM group_members WHERE user_id = ?")
            .bind(uid)
            .fetch_all(db)
            .await
            .unwrap_or_default();
    let mut member_of = Vec::new();
    let mut admin_of = Vec::new();
    for (gid, role) in rows {
        member_of.push(gid);
        if role == "admin" {
            admin_of.push(gid);
        }
    }
    Eff { uid, super_admin, member_of, admin_of }
}

// -------------------------------- models ---------------------------------

#[derive(Serialize, sqlx::FromRow)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MemberRow {
    pub user_id: i64,
    pub username: String,
    pub role: String,
}

/// {id, name, role} brief used in /api/auth/me and login responses.
#[derive(Serialize, sqlx::FromRow)]
pub struct GroupBrief {
    pub id: i64,
    pub name: String,
    pub role: String,
}

#[derive(Deserialize)]
pub struct UpsertGroup {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateGroupMeta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct AddMember {
    pub user_id: i64,
    #[serde(default)]
    pub role: Option<String>, // 'admin' | 'member' (default 'member')
}

/// The groups a user belongs to (id, name, per-group role) for client gating.
pub async fn groups_brief(db: &Pool<Sqlite>, user_id: i64) -> Vec<GroupBrief> {
    sqlx::query_as::<_, GroupBrief>(
        "SELECT g.id, g.name, gm.role \
         FROM groups g JOIN group_members gm ON gm.group_id = g.id \
         WHERE gm.user_id = ? AND g.deleted_at IS NULL ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

// ------------------------------- handlers --------------------------------

/// GET /api/me/groups — the caller's effective identity for the UI.
pub async fn my_groups(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    let groups = groups_brief(&st.db, eff.uid).await;
    Json(json!({ "super_admin": eff.super_admin, "groups": groups })).into_response()
}

/// GET /api/groups — super-admin: all; otherwise the caller's groups.
pub async fn list_groups(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    let rows: Result<Vec<Group>, _> = if eff.super_admin {
        sqlx::query_as("SELECT * FROM groups WHERE deleted_at IS NULL ORDER BY name")
            .fetch_all(&st.db)
            .await
    } else {
        sqlx::query_as(
            "SELECT g.* FROM groups g JOIN group_members gm ON gm.group_id = g.id \
             WHERE gm.user_id = ? AND g.deleted_at IS NULL ORDER BY g.name",
        )
        .bind(eff.uid)
        .fetch_all(&st.db)
        .await
    };
    resp(rows)
}

/// POST /api/groups — super-admin only.
pub async fn create_group(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(p): Json<UpsertGroup>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.super_admin {
        return (StatusCode::FORBIDDEN, "Super-admin required").into_response();
    }
    let r = sqlx::query_as::<_, Group>(
        "INSERT INTO groups(name, description) VALUES(?, ?) RETURNING *",
    )
    .bind(p.name)
    .bind(p.description)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

/// GET /api/groups/{id} — super-admin or a member; returns the group + members.
pub async fn get_group(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.is_member(id) {
        return (StatusCode::FORBIDDEN, "Not a member of this group").into_response();
    }
    let group: Option<Group> =
        sqlx::query_as("SELECT * FROM groups WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    let Some(group) = group else {
        return (StatusCode::NOT_FOUND, "Group not found").into_response();
    };
    let members: Vec<MemberRow> = sqlx::query_as(
        "SELECT gm.user_id, u.username, gm.role \
         FROM group_members gm JOIN users u ON u.id = gm.user_id \
         WHERE gm.group_id = ? ORDER BY u.username",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    Json(json!({ "group": group, "members": members })).into_response()
}

/// PUT /api/groups/{id} — super-admin or a group-admin of this group.
pub async fn update_group(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpdateGroupMeta>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.is_group_admin(id) {
        return (StatusCode::FORBIDDEN, "Group-admin required").into_response();
    }
    let r = sqlx::query_as::<_, Group>(
        "UPDATE groups SET name = COALESCE(?, name), description = COALESCE(?, description), \
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id = ? AND deleted_at IS NULL RETURNING *",
    )
    .bind(p.name)
    .bind(p.description)
    .bind(id)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

/// DELETE /api/groups/{id} — super-admin only (soft delete).
pub async fn delete_group(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.super_admin {
        return (StatusCode::FORBIDDEN, "Super-admin required").into_response();
    }
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let r = sqlx::query("UPDATE groups SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(&now)
        .bind(id)
        .execute(&st.db)
        .await
        .map(|_| ());
    resp(r)
}

/// GET /api/groups/{id}/members — super-admin or a member.
pub async fn list_members(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.is_member(id) {
        return (StatusCode::FORBIDDEN, "Not a member of this group").into_response();
    }
    let rows = sqlx::query_as::<_, MemberRow>(
        "SELECT gm.user_id, u.username, gm.role \
         FROM group_members gm JOIN users u ON u.id = gm.user_id \
         WHERE gm.group_id = ? ORDER BY u.username",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await;
    resp(rows)
}

/// POST /api/groups/{id}/members — super-admin or a group-admin of this group.
/// Adds (or updates the role of) an existing user. Role is 'admin' | 'member'.
pub async fn add_member(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(p): Json<AddMember>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.is_group_admin(id) {
        return (StatusCode::FORBIDDEN, "Group-admin required").into_response();
    }
    let role = p.role.unwrap_or_else(|| "member".to_string());
    if role != "admin" && role != "member" {
        return (StatusCode::BAD_REQUEST, "role must be 'admin' or 'member'").into_response();
    }
    // Group must exist (and not be soft-deleted); target user must exist.
    let group_ok: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM groups WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    if group_ok.is_none() {
        return (StatusCode::NOT_FOUND, "Group not found").into_response();
    }
    let user_ok: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM users WHERE id = ?")
        .bind(p.user_id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    if user_ok.is_none() {
        return (StatusCode::NOT_FOUND, "User not found").into_response();
    }
    let r = sqlx::query(
        "INSERT INTO group_members(group_id, user_id, role) VALUES(?, ?, ?) \
         ON CONFLICT(group_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(id)
    .bind(p.user_id)
    .bind(&role)
    .execute(&st.db)
    .await
    .map(|_| ());
    resp(r)
}

/// DELETE /api/groups/{id}/members/{user_id} — super-admin or a group-admin.
pub async fn remove_member(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path((id, user_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let eff = effective(&st.db, &claims).await;
    if !eff.is_group_admin(id) {
        return (StatusCode::FORBIDDEN, "Group-admin required").into_response();
    }
    let r = sqlx::query("DELETE FROM group_members WHERE group_id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(&st.db)
        .await
        .map(|_| ());
    resp(r)
}

// --------------------------- resource sharing ----------------------------
//
// A single generic endpoint pair drives the "share to specific groups" picker
// for all three shareable entities. The `kind` path segment maps to the
// resource's (table, link_table, link_col); only this allowlist is accepted.

/// Resolve a share `kind` to its (table, link_table, link_col). None = unknown.
fn share_tables(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        "channel" => Some(("channels", "channel_groups", "channel_id")),
        "project" => Some(("projects", "project_groups", "project_id")),
        "template" => Some(("templates", "template_groups", "template_id")),
        _ => None,
    }
}

/// Current sharing of a resource, for the picker.
#[derive(Serialize)]
pub struct ShareState {
    pub kind: String,
    pub id: i64,
    /// All groups the resource is currently published to (super-set of what a
    /// non-super caller can manage — out-of-scope links are preserved on save).
    pub group_ids: Vec<i64>,
    pub is_global: bool,
    /// Whether the caller may actually change the sharing (gates the Save button).
    pub can_write: bool,
}

async fn build_share_state(
    db: &Pool<Sqlite>,
    eff: &Eff,
    kind: &str,
    table: &str,
    link_table: &str,
    link_col: &str,
    id: i64,
) -> ShareState {
    let group_ids = resource_group_ids(db, link_table, link_col, id).await;
    let is_global: i64 = sqlx::query_scalar(&format!(
        "SELECT is_global FROM {table} WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .unwrap_or(0);
    let can_write = can_write(db, eff, table, link_table, link_col, id).await;
    ShareState {
        kind: kind.to_string(),
        id,
        group_ids,
        is_global: is_global != 0,
        can_write,
    }
}

/// GET /api/share/{kind}/{id} — current sharing (group links + is_global).
/// Visible to anyone who may READ the resource; `can_write` tells the UI whether
/// the Save button should be live.
pub async fn get_share(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path((kind, id)): Path<(String, i64)>,
) -> impl IntoResponse {
    let Some((table, link_table, link_col)) = share_tables(&kind) else {
        return (StatusCode::BAD_REQUEST, "Unknown share kind").into_response();
    };
    let eff = effective(&st.db, &claims).await;
    if !can_read(&st.db, &eff, table, link_table, link_col, id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to view this resource").into_response();
    }
    let state = build_share_state(&st.db, &eff, &kind, table, link_table, link_col, id).await;
    Json(state).into_response()
}

#[derive(Deserialize)]
pub struct SetShare {
    /// The groups (within the caller's reach) the resource should be shared to.
    #[serde(default)]
    pub group_ids: Vec<i64>,
    /// Org-wide visibility (super-admin only; ignored otherwise).
    #[serde(default)]
    pub is_global: Option<bool>,
}

/// PUT /api/share/{kind}/{id} — set sharing (scoped merge). WRITE required.
pub async fn set_share(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path((kind, id)): Path<(String, i64)>,
    Json(p): Json<SetShare>,
) -> impl IntoResponse {
    let Some((table, link_table, link_col)) = share_tables(&kind) else {
        return (StatusCode::BAD_REQUEST, "Unknown share kind").into_response();
    };
    let eff = effective(&st.db, &claims).await;
    if !can_write(&st.db, &eff, table, link_table, link_col, id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to change sharing").into_response();
    }
    // Non-super callers may only target groups they belong to.
    if !eff.super_admin && !p.group_ids.iter().all(|g| eff.member_of.contains(g)) {
        return (StatusCode::FORBIDDEN, "Cannot share to a group you don't belong to")
            .into_response();
    }
    // Only super-admins toggle org-wide visibility.
    if let (Some(g), true) = (p.is_global, eff.super_admin) {
        let _ = sqlx::query(&format!(
            "UPDATE {table} SET is_global = ?, \
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(g as i64)
        .bind(id)
        .execute(&st.db)
        .await;
    }
    let manageable = if eff.super_admin {
        None
    } else {
        Some(eff.member_of.as_slice())
    };
    set_groups_scoped(&st.db, link_table, link_col, id, &p.group_ids, manageable).await;
    let state = build_share_state(&st.db, &eff, &kind, table, link_table, link_col, id).await;
    Json(state).into_response()
}
