// src/jwt_auth.rs
//
// Core JWT authentication module for POIS
// Provides:
// - JWT token generation and validation
// - Password hashing with Argon2
// - User authentication
// - API token management

use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

// JWT Claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,        // user_id or token_id
    pub username: String,   // for session tokens
    pub role: String,       // "admin" or "user"
    pub token_type: String, // "session" or "api"
    pub exp: i64,           // expiration timestamp
    pub iat: i64,           // issued at timestamp
}

// User model matching the database schema
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub enabled: bool,
    pub email: Option<String>,
    pub created_at: String,
    pub last_login: Option<String>,
}

// API Token model matching the database schema
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub token_hash: String,
    pub user_id: i64,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
    pub revoked: bool,
}

/// JWT Service for token generation and validation
pub struct JwtService {
    secret: String,
}

impl JwtService {
    pub fn new(secret: String) -> Self {
        Self { secret }
    }

    /// Generate a session token (24 hours)
    pub fn generate_session_token(&self, user_id: i64, username: &str, role: &str) -> Result<String> {
        let now = Utc::now();
        let exp = now + Duration::hours(24);

        let claims = Claims {
            sub: user_id.to_string(),
            username: username.to_string(),
            role: role.to_string(),
            token_type: "session".to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        self.encode_token(&claims)
    }

    /// Generate an API token (custom expiration)
    pub fn generate_api_token(
        &self,
        _token_id: i64,
        user_id: i64,
        username: &str,
        role: &str,
        expires_in_days: Option<i64>,
    ) -> Result<String> {
        let now = Utc::now();
        let exp = if let Some(days) = expires_in_days {
            now + Duration::days(days)
        } else {
            now + Duration::days(365) // Default 1 year
        };

        let claims = Claims {
            sub: user_id.to_string(),
            username: username.to_string(),
            role: role.to_string(),
            token_type: "api".to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        self.encode_token(&claims)
    }

    /// Validate and decode a JWT token
    pub fn validate_token(&self, token: &str) -> Result<Claims> {
        self.decode_token(token)
    }

    /// Simple JWT encoding using HMAC-SHA256
    fn encode_token(&self, claims: &Claims) -> Result<String> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        
        let header = serde_json::json!({
            "alg": "HS256",
            "typ": "JWT"
        });

        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_string(&header)?);
        let claims_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_string(claims)?);
        let signature_input = format!("{}.{}", header_b64, claims_b64);
        
        // HMAC-SHA256 signature
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .map_err(|e| anyhow!("HMAC error: {}", e))?;
        mac.update(signature_input.as_bytes());
        let signature = mac.finalize();
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature.into_bytes().as_slice());

        Ok(format!("{}.{}.{}", header_b64, claims_b64, signature_b64))
    }

    /// Simple JWT decoding
    fn decode_token(&self, token: &str) -> Result<Claims> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(anyhow!("Invalid JWT format"));
        }

        let claims_b64 = parts[1];
        let signature_b64 = parts[2];
        let signature_input = format!("{}.{}", parts[0], parts[1]);

        // Verify signature
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .map_err(|e| anyhow!("HMAC error: {}", e))?;
        mac.update(signature_input.as_bytes());
        let expected_signature = mac.finalize();
        let expected_signature_b64 = URL_SAFE_NO_PAD.encode(expected_signature.into_bytes().as_slice());

        if signature_b64 != expected_signature_b64 {
            return Err(anyhow!("Invalid signature"));
        }

        // Decode claims
        let claims_json = URL_SAFE_NO_PAD.decode(claims_b64)
            .map_err(|e| anyhow!("Base64 decode error: {}", e))?;
        let claims: Claims = serde_json::from_slice(&claims_json)?;

        // Check expiration
        let now = Utc::now().timestamp();
        if claims.exp < now {
            return Err(anyhow!("Token expired"));
        }

        Ok(claims)
    }
}

/// Password Service using Argon2
pub struct PasswordService;

impl PasswordService {
    /// Hash a password using Argon2
    pub fn hash_password(password: &str) -> Result<String> {
        use argon2::{
            password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
            Argon2,
        };

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow!("Password hashing error: {}", e))?
            .to_string();

        Ok(password_hash)
    }

    /// Verify a password against a hash
    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        use argon2::{
            password_hash::{PasswordHash, PasswordVerifier},
            Argon2,
        };

        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| anyhow!("Invalid hash format: {}", e))?;

        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

/// User authentication service
pub struct AuthService {
    db: Pool<Sqlite>,
    jwt_service: JwtService,
}

impl AuthService {
    pub fn new(db: Pool<Sqlite>, jwt_secret: String) -> Self {
        Self {
            db,
            jwt_service: JwtService::new(jwt_secret),
        }
    }

    /// Authenticate user with username/password
    pub async fn authenticate(&self, username: &str, password: &str) -> Result<(User, String)> {
        // Fetch user
        let user: User = sqlx::query_as("SELECT * FROM users WHERE username = ? AND enabled = 1")
            .bind(username)
            .fetch_optional(&self.db)
            .await?
            .ok_or_else(|| anyhow!("Invalid credentials"))?;

        // Verify password
        if !PasswordService::verify_password(password, &user.password_hash)? {
            return Err(anyhow!("Invalid credentials"));
        }

        // Update last_login
        sqlx::query("UPDATE users SET last_login = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?")
            .bind(user.id)
            .execute(&self.db)
            .await?;

        // Generate session token
        let token = self.jwt_service.generate_session_token(user.id, &user.username, &user.role)?;

        Ok((user, token))
    }

    /// Create a new user
    pub async fn create_user(&self, username: &str, password: &str, role: &str, email: Option<&str>) -> Result<User> {
        let password_hash = PasswordService::hash_password(password)?;

        let user: User = sqlx::query_as(
            "INSERT INTO users (username, password_hash, role, email, enabled, created_at) 
             VALUES (?, ?, ?, ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ','now')) 
             RETURNING *"
        )
        .bind(username)
        .bind(password_hash)
        .bind(role)
        .bind(email)
        .fetch_one(&self.db)
        .await?;

        Ok(user)
    }

    /// Create an API token
    pub async fn create_api_token(
        &self,
        name: &str,
        user_id: i64,
        expires_in_days: Option<i64>,
    ) -> Result<(ApiToken, String)> {
        // Get user info for token claims
        let user: User = sqlx::query_as("SELECT * FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&self.db)
            .await?;

        // Create token record
        let expires_at = if let Some(days) = expires_in_days {
            let exp_date = Utc::now() + Duration::days(days);
            Some(exp_date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        } else {
            None
        };

        let token_record: ApiToken = sqlx::query_as(
            "INSERT INTO api_tokens (name, token_hash, user_id, expires_at, created_at, revoked) 
             VALUES (?, '', ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'), 0) 
             RETURNING *"
        )
        .bind(name)
        .bind(user_id)
        .bind(&expires_at)
        .fetch_one(&self.db)
        .await?;

        // Generate JWT token
        let token = self.jwt_service.generate_api_token(
            token_record.id,
            user.id,
            &user.username,
            &user.role,
            expires_in_days,
        )?;

        // Store hash of the token (for security)
        use sha2::{Digest, Sha256};
        let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));

        sqlx::query("UPDATE api_tokens SET token_hash = ? WHERE id = ?")
            .bind(&token_hash)
            .bind(token_record.id)
            .execute(&self.db)
            .await?;

        Ok((token_record, token))
    }

    /// Validate a token (session or API)
    pub async fn validate_token(&self, token: &str) -> Result<Claims> {
        let claims = self.jwt_service.validate_token(token)?;

        // For API tokens, check if revoked
        if claims.token_type == "api" {
            let token_id: i64 = claims.sub.parse()?;
            let is_revoked: bool = sqlx::query_scalar("SELECT revoked FROM api_tokens WHERE id = ?")
                .bind(token_id)
                .fetch_one(&self.db)
                .await?;

            if is_revoked {
                return Err(anyhow!("Token revoked"));
            }

            // Update last_used
            sqlx::query("UPDATE api_tokens SET last_used = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?")
                .bind(token_id)
                .execute(&self.db)
                .await?;
        }

        Ok(claims)
    }

    /// Revoke an API token
    pub async fn revoke_token(&self, token_id: i64) -> Result<()> {
        sqlx::query("UPDATE api_tokens SET revoked = 1 WHERE id = ?")
            .bind(token_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// List all users
    pub async fn list_users(&self) -> Result<Vec<User>> {
        let users = sqlx::query_as("SELECT * FROM users ORDER BY username")
            .fetch_all(&self.db)
            .await?;
        Ok(users)
    }

    /// List API tokens for a user
    pub async fn list_user_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>> {
        let tokens = sqlx::query_as("SELECT * FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC")
            .bind(user_id)
            .fetch_all(&self.db)
            .await?;
        Ok(tokens)
    }
}