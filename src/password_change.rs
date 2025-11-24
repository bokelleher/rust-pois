// src/password_change.rs
// Password change functionality with security logging

use anyhow::{anyhow, Result};
use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::jwt_auth::{Claims, PasswordService};
use crate::AppState;

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

/// Handler for POST /api/auth/change-password
/// Note: Claims come from Extension, added by auth middleware
pub async fn change_password_handler(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,  // Claims injected by auth middleware
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, (StatusCode, String)> {
    // Validate password strength
    if req.new_password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters".to_string(),
        ));
    }

    // Parse user_id from claims (sub is a string in JWT)
    let user_id = claims.sub.parse::<i64>()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Invalid user ID in token".to_string()))?;

    // Get client info for logging
    let ip_address = addr.ip().to_string();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    match change_password_internal(
        &state.db,
        user_id,
        &req.current_password,
        &req.new_password,
        &ip_address,
        &user_agent,
    )
    .await
    {
        Ok(()) => Ok(Json(ChangePasswordResponse {
            success: true,
            message: "Password changed successfully".to_string(),
        })),
        Err(e) => Err((
            StatusCode::UNAUTHORIZED,
            format!("Password change failed: {}", e),
        )),
    }
}

async fn change_password_internal(
    db: &Pool<Sqlite>,
    user_id: i64,
    current_password: &str,
    new_password: &str,
    ip_address: &str,
    user_agent: &str,
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
    .ok_or_else(|| anyhow!("User not found"))?;

    let (uid, username, current_hash) = user;

    // Verify current password
    if !PasswordService::verify_password(current_password, &current_hash)? {
        // Log failed attempt
        sqlx::query(
            "INSERT INTO password_changes (user_id, username, ip_address, user_agent, success, failure_reason)
             VALUES (?, ?, ?, ?, 0, ?)"
        )
        .bind(uid)
        .bind(&username)
        .bind(ip_address)
        .bind(user_agent)
        .bind("Invalid current password")
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        return Err(anyhow!("Current password is incorrect"));
    }

    // Hash new password
    let new_hash = PasswordService::hash_password(new_password)?;

    // Update password in users table
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
    .bind(&username)
    .bind(ip_address)
    .bind(user_agent)
    .execute(&mut *tx)
    .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(())
}

/// Helper function to get password change history for a user (admin only)
pub async fn get_password_history(
    db: &Pool<Sqlite>,
    user_id: i64,
    limit: i64,
) -> Result<Vec<PasswordChangeLog>> {
    let logs: Vec<PasswordChangeLog> = sqlx::query_as(
        "SELECT id, user_id, username, timestamp, ip_address, user_agent, success, failure_reason
         FROM password_changes 
         WHERE user_id = ? 
         ORDER BY timestamp DESC 
         LIMIT ?"
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(logs)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PasswordChangeLog {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub timestamp: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub success: i32,
    pub failure_reason: Option<String>,
}