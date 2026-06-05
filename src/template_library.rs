// src/template_library.rs
//! Template library + projects.
//!
//! - A **template** is a reusable entity captured from a live row:
//!     kind='rule'    -> body_json is a `backup::RuleBackup`        (applied into a channel)
//!     kind='channel' -> body_json is a `backup::ChannelFullBackup` (channel + its rules)
//! - A **project** is a persistent, shareable container bundling channel templates.
//!
//! Ownership/soft-delete/sharing mirror the channel & rule handlers in `main.rs`:
//! a non-admin sees a row when they own it, it is shared, or (for templates) its
//! project is shared. Only the owner or an admin may edit/delete/share; any user
//! may **apply** what they can see. Apply always creates NEW rows owned by the
//! applying user and never mutates the source.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use sqlx::{Pool, QueryBuilder, Sqlite};

use crate::backup::{ChannelBackup, ChannelFullBackup, RuleBackup};
use crate::jwt_auth::Claims;
use crate::models::{
    ApplyTemplate, Channel, Project, Rule, SaveTemplate, Template, UpdateProjectMeta,
    UpdateTemplateMeta, UpsertProject,
};
use crate::rbac;
use crate::AppState;

// ----------------------------- small helpers -----------------------------

/// Groups a newly created project/template should be published to:
///   - filed in a project  -> inherit that project's groups,
///   - else super-admin     -> none (unlinked; share via is_global later),
///   - else (member/g-admin) -> the saver's own groups.
async fn default_groups(
    db: &Pool<Sqlite>,
    eff: &rbac::Eff,
    inherit_project: Option<i64>,
) -> Vec<i64> {
    if let Some(pid) = inherit_project {
        return rbac::resource_group_ids(db, "project_groups", "project_id", pid).await;
    }
    if eff.super_admin {
        Vec::new()
    } else {
        eff.member_of.clone()
    }
}

fn resp<T: serde::Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

/// Generate a channel name that does not collide with the UNIQUE `channels.name`
/// constraint (which ignores `deleted_at`). Returns `base`, else `base (copy)`,
/// `base (copy 2)`, ...
async fn unique_channel_name(db: &Pool<Sqlite>, base: &str) -> String {
    for attempt in 0..10_000 {
        let candidate = match attempt {
            0 => base.to_string(),
            1 => format!("{base} (copy)"),
            n => format!("{base} (copy {n})"),
        };
        let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM channels WHERE name = ?")
            .bind(&candidate)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
        if exists.is_none() {
            return candidate;
        }
    }
    // Pathological fallback (10k same-named channels); keep it deterministic.
    format!("{base} (copy 10000)")
}

/// Ensure `eff` may write to project `pid` (group-aware). `Some(rejection)` if not.
async fn reject_if_project_unwritable(
    db: &Pool<Sqlite>,
    eff: &rbac::Eff,
    pid: i64,
) -> Option<Response> {
    if rbac::can_write(db, eff, "projects", "project_groups", "project_id", pid).await {
        None
    } else {
        Some((StatusCode::FORBIDDEN, "Not allowed to modify this project").into_response())
    }
}

// ------------------------------- projects --------------------------------

pub async fn list_projects(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("SELECT * FROM projects WHERE deleted_at IS NULL");
    rbac::push_read_predicate(&mut qb, &eff, "projects", "project_groups", "project_id");
    qb.push(" ORDER BY name");
    let rows = qb.build_query_as::<Project>().fetch_all(&st.db).await;
    resp(rows)
}

pub async fn create_project(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(p): Json<UpsertProject>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let is_global = (eff.super_admin && p.is_global.unwrap_or(false)) as i64;
    let r = sqlx::query_as::<_, Project>(
        "INSERT INTO projects(name,description,owner_user_id,is_global) VALUES(?,?,?,?) RETURNING *",
    )
    .bind(p.name)
    .bind(p.description)
    .bind(eff.uid)
    .bind(is_global)
    .fetch_one(&st.db)
    .await;
    match r {
        Ok(proj) => {
            let groups = match p.group_ids {
                Some(g) if eff.super_admin || g.iter().all(|x| eff.member_of.contains(x)) => g,
                Some(_) => return (StatusCode::FORBIDDEN, "Cannot publish to a group you don't belong to").into_response(),
                None => default_groups(&st.db, &eff, None).await,
            };
            rbac::link_groups(&st.db, "project_groups", "project_id", proj.id, &groups).await;
            Json(proj).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

pub async fn get_project(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let project: Option<Project> =
        sqlx::query_as("SELECT * FROM projects WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();

    let Some(project) = project else {
        return (StatusCode::NOT_FOUND, "Project not found").into_response();
    };
    if !rbac::can_read(&st.db, &eff, "projects", "project_groups", "project_id", id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to view this project").into_response();
    }

    let members: Result<Vec<Template>, _> = sqlx::query_as(
        "SELECT * FROM templates WHERE project_id = ? AND deleted_at IS NULL ORDER BY kind, name",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await;

    match members {
        Ok(templates) => Json(json!({ "project": project, "templates": templates })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

pub async fn update_project(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpdateProjectMeta>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if let Some(rej) = reject_if_project_unwritable(&st.db, &eff, id).await {
        return rej;
    }
    let is_global = if eff.super_admin { p.is_global.map(|b| b as i64) } else { None };
    let r = sqlx::query_as::<_, Project>(
        "UPDATE projects SET \
           name=COALESCE(?,name), \
           description=COALESCE(?,description), \
           is_shared=COALESCE(?,is_shared), \
           is_global=COALESCE(?,is_global), \
           updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id=? AND deleted_at IS NULL RETURNING *",
    )
    .bind(p.name)
    .bind(p.description)
    .bind(p.is_shared.map(|b| b as i64))
    .bind(is_global)
    .bind(id)
    .fetch_one(&st.db)
    .await;
    if let Some(gids) = p.group_ids {
        if eff.super_admin || gids.iter().all(|g| eff.member_of.contains(g)) {
            let _ = sqlx::query("DELETE FROM project_groups WHERE project_id = ?")
                .bind(id)
                .execute(&st.db)
                .await;
            rbac::link_groups(&st.db, "project_groups", "project_id", id, &gids).await;
        }
    }
    resp(r)
}

pub async fn delete_project(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if let Some(rej) = reject_if_project_unwritable(&st.db, &eff, id).await {
        return rej;
    }
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    // Soft-delete the project and unfile its templates so they survive as
    // standalone library items.
    let r1 = sqlx::query("UPDATE projects SET deleted_at=? WHERE id=? AND deleted_at IS NULL")
        .bind(&now)
        .bind(id)
        .execute(&st.db)
        .await;
    if let Err(e) = r1 {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    let r2 = sqlx::query("UPDATE templates SET project_id=NULL WHERE project_id=?")
        .bind(id)
        .execute(&st.db)
        .await
        .map(|_| ());
    resp(r2)
}

pub async fn apply_project(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Visibility check on the project (read).
    let eff = rbac::effective(&st.db, &claims).await;
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM projects WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    if exists.is_none() {
        return (StatusCode::NOT_FOUND, "Project not found").into_response();
    }
    if !rbac::can_read(&st.db, &eff, "projects", "project_groups", "project_id", id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to apply this project").into_response();
    }

    let members: Vec<Template> = match sqlx::query_as(
        "SELECT * FROM templates \
         WHERE project_id = ? AND kind = 'channel' AND deleted_at IS NULL ORDER BY name",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let owner_id = eff.uid;
    // New channels land in the applier's own group(s) (a station applying a
    // national project gets the channels in their station). Super: unlinked.
    let target_groups: Vec<i64> = if eff.super_admin { Vec::new() } else { eff.member_of.clone() };
    let mut created = Vec::new();
    let mut errors = Vec::new();

    for m in members {
        match serde_json::from_str::<ChannelFullBackup>(&m.body_json) {
            Ok(cfb) => match instantiate_channel_full(&st.db, &cfb, owner_id, None).await {
                Ok((cid, cname, n_rules)) => {
                    rbac::link_groups(&st.db, "channel_groups", "channel_id", cid, &target_groups).await;
                    created.push(json!({
                        "template_id": m.id,
                        "channel_id": cid,
                        "channel_name": cname,
                        "rules_created": n_rules,
                    }));
                }
                Err(e) => errors.push(format!("template {}: {}", m.id, e)),
            },
            Err(e) => errors.push(format!("template {} has invalid body: {}", m.id, e)),
        }
    }

    Json(json!({
        "project_id": id,
        "channels_created": created.len(),
        "created": created,
        "errors": errors,
    }))
    .into_response()
}

// ------------------------------- templates -------------------------------

#[derive(Deserialize)]
pub struct TemplateQuery {
    #[serde(default)]
    pub project_id: Option<i64>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub unfiled: Option<bool>,
    /// Only featured/default templates (the curated gallery subset).
    #[serde(default)]
    pub default: Option<bool>,
}

pub async fn list_templates(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<TemplateQuery>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("SELECT * FROM templates WHERE deleted_at IS NULL");
    rbac::push_read_predicate(&mut qb, &eff, "templates", "template_groups", "template_id");
    if let Some(pid) = q.project_id {
        qb.push(" AND project_id = ").push_bind(pid);
    }
    if q.unfiled == Some(true) {
        qb.push(" AND project_id IS NULL");
    }
    if let Some(kind) = q.kind {
        qb.push(" AND kind = ").push_bind(kind);
    }
    if q.default == Some(true) {
        qb.push(" AND is_default = 1");
    }
    // Featured (default) templates first, then alphabetical within kind.
    qb.push(" ORDER BY is_default DESC, kind, name");

    let rows = qb.build_query_as::<Template>().fetch_all(&st.db).await;
    resp(rows)
}

pub async fn get_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    match load_visible_template(&st.db, &eff, id).await {
        Ok(t) => Json(t).into_response(),
        Err(rej) => rej,
    }
}

/// Load a template if the caller may read it (group-aware).
async fn load_visible_template(
    db: &Pool<Sqlite>,
    eff: &rbac::Eff,
    id: i64,
) -> Result<Template, Response> {
    let t: Option<Template> =
        sqlx::query_as("SELECT * FROM templates WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let Some(t) = t else {
        return Err((StatusCode::NOT_FOUND, "Template not found").into_response());
    };
    if rbac::can_read(db, eff, "templates", "template_groups", "template_id", id).await {
        Ok(t)
    } else {
        Err((StatusCode::FORBIDDEN, "Not allowed to view this template").into_response())
    }
}

/// Write check for edit/delete (group-aware). `Some(rejection)` if not allowed.
async fn reject_if_template_unwritable(
    db: &Pool<Sqlite>,
    eff: &rbac::Eff,
    id: i64,
) -> Option<Response> {
    if rbac::can_write(db, eff, "templates", "template_groups", "template_id", id).await {
        None
    } else {
        Some((StatusCode::FORBIDDEN, "Not allowed to modify this template").into_response())
    }
}

pub async fn save_rule_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(rule_id): Path<i64>,
    Json(p): Json<SaveTemplate>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    // Load the source rule; the caller must be able to READ its parent channel.
    let rule: Option<Rule> =
        sqlx::query_as("SELECT * FROM rules WHERE id = ? AND deleted_at IS NULL")
            .bind(rule_id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    let Some(rule) = rule else {
        return (StatusCode::NOT_FOUND, "Rule not found").into_response();
    };
    if !rbac::can_read(&st.db, &eff, "channels", "channel_groups", "channel_id", rule.channel_id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to template this rule").into_response();
    }
    if let Some(pid) = p.project_id {
        if let Some(rej) = reject_if_project_unwritable(&st.db, &eff, pid).await {
            return rej;
        }
    }

    let match_json: JsonValue = serde_json::from_str(&rule.match_json).unwrap_or(JsonValue::Null);
    let params_json: JsonValue = serde_json::from_str(&rule.params_json).unwrap_or(JsonValue::Null);
    let body = RuleBackup {
        name: rule.name.clone(),
        match_json,
        action: rule.action.clone(),
        params_json,
        priority: rule.priority,
        enabled: rule.enabled != 0,
    };
    let body_json = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let name = p.name.unwrap_or(rule.name);

    insert_template(
        &st.db, &eff, &name, "rule", p.description, p.project_id, &body_json, p.is_shared,
        p.is_default, p.group_ids,
    )
    .await
}

pub async fn save_channel_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(channel_id): Path<i64>,
    Json(p): Json<SaveTemplate>,
) -> impl IntoResponse {
    let channel: Option<Channel> =
        sqlx::query_as("SELECT * FROM channels WHERE id = ? AND deleted_at IS NULL")
            .bind(channel_id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    let Some(channel) = channel else {
        return (StatusCode::NOT_FOUND, "Channel not found").into_response();
    };
    let eff = rbac::effective(&st.db, &claims).await;
    if !rbac::can_read(&st.db, &eff, "channels", "channel_groups", "channel_id", channel_id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to template this channel").into_response();
    }
    if let Some(pid) = p.project_id {
        if let Some(rej) = reject_if_project_unwritable(&st.db, &eff, pid).await {
            return rej;
        }
    }

    let rules: Vec<Rule> = match sqlx::query_as(
        "SELECT * FROM rules WHERE channel_id = ? AND deleted_at IS NULL ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let rule_backups: Vec<RuleBackup> = rules
        .into_iter()
        .filter_map(|r| {
            Some(RuleBackup {
                name: r.name,
                match_json: serde_json::from_str(&r.match_json).ok()?,
                action: r.action,
                params_json: serde_json::from_str(&r.params_json).ok()?,
                priority: r.priority,
                enabled: r.enabled != 0,
            })
        })
        .collect();

    let body = ChannelFullBackup {
        channel: ChannelBackup {
            name: channel.name.clone(),
            enabled: channel.enabled != 0,
            timezone: channel.timezone.clone(),
        },
        rules: rule_backups,
        backup_metadata: Default::default(),
    };
    let body_json = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let name = p.name.unwrap_or(channel.name);

    insert_template(
        &st.db, &eff, &name, "channel", p.description, p.project_id, &body_json, p.is_shared,
        p.is_default, p.group_ids,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_template(
    db: &Pool<Sqlite>,
    eff: &rbac::Eff,
    name: &str,
    kind: &str,
    description: Option<String>,
    project_id: Option<i64>,
    body_json: &str,
    is_shared: Option<bool>,
    is_default: Option<bool>,
    group_ids: Option<Vec<i64>>,
) -> Response {
    let want_default = is_default.unwrap_or(false);
    // A super-admin's "share"/"default" template is org-wide (is_global); members
    // publish to their groups instead (links below).
    let want_global = eff.super_admin && (is_shared.unwrap_or(false) || want_default);
    let r = sqlx::query_as::<_, Template>(
        "INSERT INTO templates(name,kind,description,project_id,body_json,is_shared,is_default,is_global,owner_user_id) \
         VALUES(?,?,?,?,?,?,?,?,?) RETURNING *",
    )
    .bind(name)
    .bind(kind)
    .bind(description)
    .bind(project_id)
    .bind(body_json)
    .bind(want_global as i64) // keep legacy is_shared in sync with is_global
    .bind(want_default as i64)
    .bind(want_global as i64)
    .bind(eff.uid)
    .fetch_one(db)
    .await;
    match r {
        Ok(t) => {
            // Publish to groups: explicit list (validated) else the project's /
            // saver's own groups. Super-admin globals stay unlinked.
            let groups = match group_ids {
                Some(g) if eff.super_admin || g.iter().all(|x| eff.member_of.contains(x)) => g,
                Some(_) => return (StatusCode::FORBIDDEN, "Cannot publish to a group you don't belong to").into_response(),
                None => default_groups(db, eff, project_id).await,
            };
            rbac::link_groups(db, "template_groups", "template_id", t.id, &groups).await;
            Json(t).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

pub async fn update_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpdateTemplateMeta>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if let Some(rej) = reject_if_template_unwritable(&st.db, &eff, id).await {
        return rej;
    }
    // If moving into a project, the caller must be able to write that project.
    if let Some(Some(pid)) = p.project_id {
        if let Some(rej) = reject_if_project_unwritable(&st.db, &eff, pid).await {
            return rej;
        }
    }

    // Fetch current row to compute final values (project_id can be set to NULL).
    let cur: Option<Template> =
        sqlx::query_as("SELECT * FROM templates WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    let Some(cur) = cur else {
        return (StatusCode::NOT_FOUND, "Template not found").into_response();
    };

    let name = p.name.unwrap_or(cur.name);
    let description = p.description.or(cur.description);
    let project_id = match p.project_id {
        Some(v) => v, // explicit set (may be None to unfile)
        None => cur.project_id,
    };
    let is_default = p.is_default.map(|b| b as i64).unwrap_or(cur.is_default);
    // Only super-admins toggle org-wide visibility; a super default stays global.
    let mut is_global = if eff.super_admin {
        p.is_global.map(|b| b as i64).unwrap_or(cur.is_global)
    } else {
        cur.is_global
    };
    if is_default == 1 && eff.super_admin {
        is_global = 1;
    }

    let r = sqlx::query_as::<_, Template>(
        "UPDATE templates SET name=?, description=?, project_id=?, is_global=?, is_default=?, \
           updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id=? AND deleted_at IS NULL RETURNING *",
    )
    .bind(name)
    .bind(description)
    .bind(project_id)
    .bind(is_global)
    .bind(is_default)
    .bind(id)
    .fetch_one(&st.db)
    .await;
    // Re-publish to the supplied groups (super any; others only own groups).
    if let Some(gids) = p.group_ids {
        if eff.super_admin || gids.iter().all(|g| eff.member_of.contains(g)) {
            let _ = sqlx::query("DELETE FROM template_groups WHERE template_id = ?")
                .bind(id)
                .execute(&st.db)
                .await;
            rbac::link_groups(&st.db, "template_groups", "template_id", id, &gids).await;
        }
    }
    resp(r)
}

pub async fn delete_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if let Some(rej) = reject_if_template_unwritable(&st.db, &eff, id).await {
        return rej;
    }
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let r = sqlx::query("UPDATE templates SET deleted_at=? WHERE id=? AND deleted_at IS NULL")
        .bind(&now)
        .bind(id)
        .execute(&st.db)
        .await
        .map(|_| ());
    resp(r)
}

pub async fn apply_template(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(p): Json<ApplyTemplate>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let t = match load_visible_template(&st.db, &eff, id).await {
        Ok(t) => t,
        Err(rej) => return rej,
    };
    let owner_id = eff.uid;

    match t.kind.as_str() {
        "rule" => {
            let Some(channel_id) = p.target_channel_id else {
                return (
                    StatusCode::BAD_REQUEST,
                    "target_channel_id is required to apply a rule template",
                )
                    .into_response();
            };
            // Caller must be able to WRITE the target channel.
            if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", channel_id).await {
                return (StatusCode::FORBIDDEN, "Not allowed to add a rule to this channel").into_response();
            }

            let rb: RuleBackup = match serde_json::from_str(&t.body_json) {
                Ok(v) => v,
                Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            };

            let maxp: Option<(i64,)> = sqlx::query_as(
                "SELECT MAX(priority) FROM rules WHERE channel_id=? AND deleted_at IS NULL",
            )
            .bind(channel_id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
            let nextp = maxp.map(|t| t.0 + 10).unwrap_or(0);
            let name = p.name.unwrap_or(rb.name);

            let r = sqlx::query_as::<_, Rule>(
                "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json,owner_user_id) \
                 VALUES(?,?,?,?,?,?,?,?) RETURNING *",
            )
            .bind(channel_id)
            .bind(name)
            .bind(nextp)
            .bind(rb.enabled as i64)
            .bind(rb.match_json.to_string())
            .bind(rb.action)
            .bind(rb.params_json.to_string())
            .bind(owner_id)
            .fetch_one(&st.db)
            .await;
            resp(r)
        }
        "channel" => {
            let cfb: ChannelFullBackup = match serde_json::from_str(&t.body_json) {
                Ok(v) => v,
                Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            };
            match instantiate_channel_full(&st.db, &cfb, owner_id, p.name).await {
                Ok((cid, cname, n_rules)) => {
                    // New channel lands in the applier's own group(s); super: unlinked.
                    let groups: Vec<i64> = if eff.super_admin { Vec::new() } else { eff.member_of.clone() };
                    rbac::link_groups(&st.db, "channel_groups", "channel_id", cid, &groups).await;
                    Json(json!({
                        "channel_id": cid,
                        "channel_name": cname,
                        "rules_created": n_rules,
                    }))
                    .into_response()
                }
                Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
            }
        }
        other => (
            StatusCode::BAD_REQUEST,
            format!("Unknown template kind '{other}'"),
        )
            .into_response(),
    }
}

/// Recreate a channel + its rules from a `ChannelFullBackup`, owned by `owner_id`.
/// The channel name is made unique (UNIQUE constraint on channels.name).
/// Returns (channel_id, final_name, rules_created).
async fn instantiate_channel_full(
    db: &Pool<Sqlite>,
    cfb: &ChannelFullBackup,
    owner_id: i64,
    name_override: Option<String>,
) -> Result<(i64, String, usize), String> {
    let base = name_override.unwrap_or_else(|| cfb.channel.name.clone());
    let name = unique_channel_name(db, &base).await;

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO channels(name,enabled,timezone,owner_user_id) VALUES(?,?,?,?) RETURNING id",
    )
    .bind(&name)
    .bind(cfb.channel.enabled as i64)
    .bind(&cfb.channel.timezone)
    .bind(owner_id)
    .fetch_one(db)
    .await
    .map_err(|e| e.to_string())?;
    let channel_id = row.0;

    let mut n_rules = 0usize;
    for rb in &cfb.rules {
        sqlx::query(
            "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json,owner_user_id) \
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(channel_id)
        .bind(&rb.name)
        .bind(rb.priority)
        .bind(rb.enabled as i64)
        .bind(rb.match_json.to_string())
        .bind(&rb.action)
        .bind(rb.params_json.to_string())
        .bind(owner_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
        n_rules += 1;
    }

    Ok((channel_id, name, n_rules))
}
