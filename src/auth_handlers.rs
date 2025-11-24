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
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
    pub email: Option<String>,
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

// ==================== Public Endpoints ====================

/// POST /auth/login
pub async fn login(
    State(auth_state): State<Arc<AuthState>>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    match auth_state.auth_service.authenticate(&req.username, &req.password).await {
        Ok((user, token)) => {
            let response = LoginResponse {
                token,
                user: UserResponse {
                    id: user.id,
                    username: user.username,
                    role: user.role,
                    enabled: user.enabled,
                    email: user.email,
                    created_at: user.created_at,
                    last_login: user.last_login,
                },
            };
            (StatusCode::OK, Json(response)).into_response()
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
            let response = UserResponse {
                id: user.id,
                username: user.username,
                role: user.role,
                enabled: user.enabled,
                email: user.email,
                created_at: user.created_at,
                last_login: user.last_login,
            };
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

    if let Err((status, msg)) = require_admin(&claims) {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    match auth_state.auth_service.list_users().await {
        Ok(users) => {
            let response: Vec<UserResponse> = users.into_iter().map(|u| UserResponse {
                id: u.id,
                username: u.username,
                role: u.role,
                enabled: u.enabled,
                email: u.email,
                created_at: u.created_at,
                last_login: u.last_login,
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

    if let Err((status, msg)) = require_admin(&claims) {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    match auth_state.auth_service.create_user(&req.username, &req.password, &req.role, req.email.as_deref()).await {
        Ok(user) => {
            let response = UserResponse {
                id: user.id,
                username: user.username,
                role: user.role,
                enabled: user.enabled,
                email: user.email,
                created_at: user.created_at,
                last_login: user.last_login,
            };
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

    if let Err((status, msg)) = require_admin(&claims) {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    match sqlx::query_as::<_, crate::jwt_auth::User>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&auth_state.db)
        .await
    {
        Ok(Some(user)) => {
            let response = UserResponse {
                id: user.id,
                username: user.username,
                role: user.role,
                enabled: user.enabled,
                email: user.email,
                created_at: user.created_at,
                last_login: user.last_login,
            };
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

    if let Err((status, msg)) = require_admin(&claims) {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
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
            let response = UserResponse {
                id: user.id,
                username: user.username,
                role: user.role,
                enabled: user.enabled,
                email: user.email,
                created_at: user.created_at,
                last_login: user.last_login,
            };
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