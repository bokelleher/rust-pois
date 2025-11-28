// src/password_change.rs
// Password change functionality with security logging
// Version: 3.1.3
// Last Modified: 2025-11-25
// Changes: Fixed JSON error responses, proper router placement

use anyhow::{anyhow, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use tracing::info;

use crate::auth_handlers::AuthState;
use crate::jwt_auth::{Claims, PasswordService};

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct ChangePasswordResponse {
    pub success: bool,
    pub message: String,
}

/// POST /api/auth/change-password
/// Changes user password with current password verification
pub async fn change_password_handler(
    State(auth_state): State<Arc<AuthState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    // Validate password strength
    if req.new_password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Password must be at least 8 characters" })),
        ).into_response();
    }

    // Parse user_id from claims
    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Invalid user ID in token" })),
            ).into_response();
        }
    };

    // Change password
    match change_password_internal(
        &auth_state.db,
        user_id,
        &claims.username,
        &req.current_password,
        &req.new_password,
    ).await {
        Ok(()) => {
            info!("Password changed successfully for user_id={}, username={}", user_id, claims.username);
            (
                StatusCode::OK,
                Json(ChangePasswordResponse {
                    success: true,
                    message: "Password changed successfully".to_string(),
                }),
            ).into_response()
        }
        Err(e) => {
            info!("Password change failed for user_id={}, username={}: {}", user_id, claims.username, e);
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": format!("Password change failed: {}", e) })),
            ).into_response()
        }
    }
}

async fn change_password_internal(
    db: &Pool<Sqlite>,
    user_id: i64,
    _username: &str,
    current_password: &str,
    new_password: &str,
) -> Result<()> {
    // Start transaction
    let mut tx = db.begin().await?;

    // Fetch user
    let user: (i64, String, String) = sqlx::query_as(
        "SELECT id, username, password_hash FROM users WHERE id = ? AND enabled = 1"
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("User not found or disabled"))?;

    let (uid, db_username, current_hash) = user;

    // Verify current password
    if !PasswordService::verify_password(current_password, &current_hash)? {
        // Log failed attempt
        let _ = sqlx::query(
            "INSERT INTO password_changes (user_id, username, ip_address, user_agent, success, failure_reason)
             VALUES (?, ?, ?, ?, 0, ?)"
        )
        .bind(uid)
        .bind(&db_username)
        .bind("unknown")
        .bind("unknown")
        .bind("Invalid current password")
        .execute(&mut *tx)
        .await;

        tx.commit().await?;
        return Err(anyhow!("Current password is incorrect"));
    }

    // Hash new password
    let new_hash = PasswordService::hash_password(new_password)?;

    // Update password
    sqlx::query(
        "UPDATE users 
         SET password_hash = ?, 
             password_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?"
    )
    .bind(&new_hash)
    .bind(uid)
    .execute(&mut *tx)
    .await?;

    // Log successful password change
    sqlx::query(
        "INSERT INTO password_changes (user_id, username, ip_address, user_agent, success)
         VALUES (?, ?, ?, ?, 1)"
    )
    .bind(uid)
    .bind(&db_username)
    .bind("unknown")
    .bind("unknown")
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}