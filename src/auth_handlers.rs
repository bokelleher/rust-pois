// src/auth_handlers.rs
//
// HTTP handlers for JWT authentication endpoints

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;

use crate::jwt_auth::{AuthService, Claims, PasswordService};

// AppState that includes auth
pub struct AuthState {
    pub db: Pool<Sqlite>,
    pub auth_service: AuthService,
}

/// Request/Response types

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub enabled: bool,
    pub email: Option<String>,
    pub created_at: String,
    pub last_login: Option<String>,
    /// True while the user must set their own password before using the app.
    pub must_change_password: bool,
}

impl From<crate::jwt_auth::User> for UserResponse {
    fn from(u: crate::jwt_auth::User) -> Self {
        UserResponse {
            id: u.id,
            username: u.username,
            role: u.role,
            enabled: u.enabled,
            email: u.email,
            created_at: u.created_at,
            last_login: u.last_login,
            must_change_password: u.must_change_password,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
    pub email: Option<String>,
    /// RBAC: group to add the new user to (required for a group-admin who
    /// administers more than one group; ignored field for super-admins unless set).
    #[serde(default)]
    pub group_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub password: Option<String>,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub token: String,
    pub token_info: ApiTokenResponse,
}

#[derive(Debug, Serialize)]
pub struct ApiTokenResponse {
    pub id: i64,
    pub name: String,
    pub user_id: i64,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
    pub revoked: bool,
}

/// Extract token from Authorization header
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Extract claims from request headers
async fn extract_claims(
    auth_service: &AuthService,
    headers: &HeaderMap,
) -> Result<Claims, (StatusCode, String)> {
    let token = extract_bearer_token(headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))?;

    auth_service
        .validate_token(&token)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))
}

/// Require admin role
fn require_admin(claims: &Claims) -> Result<(), (StatusCode, &'static str)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin access required"));
    }
    Ok(())
}

/// User ids that are members of any group the caller administers.
async fn users_in_admin_groups(
    db: &sqlx::Pool<sqlx::Sqlite>,
    admin_of: &[i64],
) -> Vec<i64> {
    if admin_of.is_empty() {
        return Vec::new();
    }
    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> =
        sqlx::QueryBuilder::new("SELECT DISTINCT user_id FROM group_members WHERE group_id IN (");
    let mut sep = qb.separated(", ");
    for g in admin_of {
        sep.push_bind(*g);
    }
    qb.push(")");
    qb.build_query_scalar().fetch_all(db).await.unwrap_or_default()
}

// ==================== Public Endpoints ====================

/// POST /auth/login
pub async fn login(
    State(auth_state): State<Arc<AuthState>>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    match auth_state.auth_service.authenticate(&req.username, &req.password).await {
        Ok((user, token)) => {
            let uid = user.id;
            let response = LoginResponse {
                token,
                user: UserResponse::from(user),
            };
            // Attach the caller's group memberships onto the user object so the
            // frontend can gate on them (rbac groups, Phase 1).
            let groups = crate::rbac::groups_brief(&auth_state.db, uid).await;
            let mut v = serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(u) = v.get_mut("user").and_then(|u| u.as_object_mut()) {
                u.insert("groups".to_string(), serde_json::json!(groups));
            }
            (StatusCode::OK, Json(v)).into_response()
        }
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /auth/me
pub async fn get_current_user(
    State(auth_state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Invalid user ID" }))).into_response(),
    };

    match sqlx::query_as::<_, crate::jwt_auth::User>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&auth_state.db)
        .await
    {
        Ok(Some(user)) => {
            let uid = user.id;
            let response = UserResponse::from(user);
            // Attach group memberships for client-side gating (rbac Phase 1).
            let groups = crate::rbac::groups_brief(&auth_state.db, uid).await;
            let mut v = serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(o) = v.as_object_mut() {
                o.insert("groups".to_string(), serde_json::json!(groups));
            }
            (StatusCode::OK, Json(v)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ==================== User Management (Admin Only) ====================

/// GET /users
pub async fn list_users(
    State(auth_state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    // Super-admins see all users; group-admins see only members of their groups.
    let eff = crate::rbac::effective(&auth_state.db, &claims).await;
    if !eff.super_admin && eff.admin_of.is_empty() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin access required" }))).into_response();
    }
    let scope: Option<Vec<i64>> = if eff.super_admin {
        None
    } else {
        Some(users_in_admin_groups(&auth_state.db, &eff.admin_of).await)
    };

    match auth_state.auth_service.list_users().await {
        Ok(users) => {
            let response: Vec<UserResponse> = users
                .into_iter()
                .filter(|u| scope.as_ref().map(|ids| ids.contains(&u.id)).unwrap_or(true))
                .map(UserResponse::from).collect();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /users
pub async fn create_user(
    State(auth_state): State<Arc<AuthState>>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let eff = crate::rbac::effective(&auth_state.db, &claims).await;
    if !eff.super_admin && eff.admin_of.is_empty() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin access required" }))).into_response();
    }
    // Resolve the group the new user joins (group-admins must administer it).
    let target_group: Option<i64> = if eff.super_admin {
        req.group_id
    } else {
        match req.group_id {
            Some(g) if eff.admin_of.contains(&g) => Some(g),
            Some(_) => return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "You don't administer that group" }))).into_response(),
            None if eff.admin_of.len() == 1 => Some(eff.admin_of[0]),
            None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "group_id required (you administer multiple groups)" }))).into_response(),
        }
    };
    // Group-admins may only create regular users (never super-admins).
    let role = if eff.super_admin { req.role.clone() } else { "user".to_string() };

    match auth_state.auth_service.create_user(&req.username, &req.password, &role, req.email.as_deref()).await {
        Ok(user) => {
            if let Some(g) = target_group {
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO group_members(group_id, user_id, role) VALUES(?, ?, 'member')",
                )
                .bind(g)
                .bind(user.id)
                .execute(&auth_state.db)
                .await;
            }
            let response = UserResponse::from(user);
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /users/:id
pub async fn get_user(
    State(auth_state): State<Arc<AuthState>>,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let eff = crate::rbac::effective(&auth_state.db, &claims).await;
    if !eff.super_admin {
        let visible = users_in_admin_groups(&auth_state.db, &eff.admin_of).await;
        if !visible.contains(&user_id) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Not allowed" }))).into_response();
        }
    }

    match sqlx::query_as::<_, crate::jwt_auth::User>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&auth_state.db)
        .await
    {
        Ok(Some(user)) => {
            let response = UserResponse::from(user);
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// PUT /users/:id
pub async fn update_user(
    State(auth_state): State<Arc<AuthState>>,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
    Json(req): Json<UpdateUserRequest>,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    // Super-admins manage anyone; group-admins only members of their groups, and
    // can never grant super-admin.
    let eff = crate::rbac::effective(&auth_state.db, &claims).await;
    if !eff.super_admin {
        let visible = users_in_admin_groups(&auth_state.db, &eff.admin_of).await;
        if eff.admin_of.is_empty() || !visible.contains(&user_id) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Not allowed" }))).into_response();
        }
        if req.role.as_deref() == Some("admin") {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Cannot grant super-admin" }))).into_response();
        }
    }

    // Protect admin user (ID 1) from being demoted or disabled
    if user_id == 1 {
        if let Some(role) = &req.role {
            if role != "admin" {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": "Cannot change admin user role" })),
                ).into_response();
            }
        }
        if let Some(enabled) = req.enabled {
            if !enabled {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": "Cannot disable admin user" })),
                ).into_response();
            }
        }
    }

    // Build update query
    let mut updates = Vec::new();
    let mut values: Vec<String> = Vec::new();

    if let Some(password) = req.password {
        match PasswordService::hash_password(&password) {
            Ok(hash) => {
                updates.push("password_hash = ?");
                values.push(hash);
                // An admin-set password is a temp credential: force the user to
                // choose their own on next login.
                updates.push("must_change_password = 1");
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Password hashing failed: {}", e) })),
                )
                    .into_response();
            }
        }
    }

    if let Some(role) = req.role {
        updates.push("role = ?");
        values.push(role);
    }

    if let Some(enabled) = req.enabled {
        updates.push("enabled = ?");
        values.push(if enabled { "1".to_string() } else { "0".to_string() });
    }

    if let Some(email) = req.email {
        updates.push("email = ?");
        values.push(email);
    }

    if updates.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "No fields to update" })),
        )
            .into_response();
    }

    updates.push("updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')");

    let query_str = format!(
        "UPDATE users SET {} WHERE id = ? RETURNING *",
        updates.join(", ")
    );

    let mut query = sqlx::query_as::<_, crate::jwt_auth::User>(&query_str);
    for val in values {
        query = query.bind(val);
    }
    query = query.bind(user_id);

    match query.fetch_one(&auth_state.db).await {
        Ok(user) => {
            let response = UserResponse::from(user);
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /users/:id
pub async fn delete_user(
    State(auth_state): State<Arc<AuthState>>,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    if let Err((status, msg)) = require_admin(&claims) {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    // Protect admin user (ID 1) from deletion
    if user_id == 1 {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Cannot delete admin user" })),
        ).into_response();
    }

    match sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(&auth_state.db)
        .await
    {
        Ok(_) => (StatusCode::NO_CONTENT, ()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ==================== API Token Management ====================

/// GET /tokens - List my tokens
pub async fn list_my_tokens(
    State(auth_state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Invalid user ID" }))).into_response(),
    };

    match auth_state.auth_service.list_user_tokens(user_id).await {
        Ok(tokens) => {
            let response: Vec<ApiTokenResponse> = tokens.into_iter().map(|t| ApiTokenResponse {
                id: t.id,
                name: t.name,
                user_id: t.user_id,
                expires_at: t.expires_at,
                created_at: t.created_at,
                last_used: t.last_used,
                revoked: t.revoked,
            }).collect();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /tokens - Create new API token
pub async fn create_api_token(
    State(auth_state): State<Arc<AuthState>>,
    headers: HeaderMap,
    Json(req): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Invalid user ID" }))).into_response(),
    };

    match auth_state.auth_service.create_api_token(&req.name, user_id, req.expires_in_days).await {
        Ok((token_record, token)) => {
            let response = CreateTokenResponse {
                token,
                token_info: ApiTokenResponse {
                    id: token_record.id,
                    name: token_record.name,
                    user_id: token_record.user_id,
                    expires_at: token_record.expires_at,
                    created_at: token_record.created_at,
                    last_used: token_record.last_used,
                    revoked: token_record.revoked,
                },
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /tokens/:id - Revoke API token
pub async fn revoke_api_token(
    State(auth_state): State<Arc<AuthState>>,
    Path(token_id): Path<i64>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let claims = match extract_claims(&auth_state.auth_service, &headers).await {
        Ok(c) => c,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Invalid user ID" }))).into_response(),
    };

    // Verify token belongs to user
    let token_user: Option<(i64,)> = match sqlx::query_as("SELECT user_id FROM api_tokens WHERE id = ?")
        .bind(token_id)
        .fetch_optional(&auth_state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match token_user {
        Some((owner_id,)) if owner_id == user_id || claims.role == "admin" => {
            match auth_state.auth_service.revoke_token(token_id).await {
                Ok(()) => (StatusCode::NO_CONTENT, ()).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        Some(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Not authorized to revoke this token" })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Token not found" })),
        )
            .into_response(),
    }
}