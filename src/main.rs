// src/main.rs
// Version: 3.2.2
// Last Modified: 2024-11-28
// Changes: 
//   - CRITICAL: Fixed Axum 0.7 route syntax (:id → {id}, :channel → {channel})
//   - CRITICAL: Fixed multi-tenancy security - non-admin users now properly filtered
//   - Fixed event logging API calls (log_event → log_esam_event)
//   - Fixed build_notification calls (3 params → 4 params: acq_id, utc_point, action, params)
//   - Fixed ClientInfo structure (ip → source_ip, user_agent → Option<String>)
//   - Fixed ProcessingMetrics (removed matched_rule_id/name fields)
//   - Builder consolidation - removed /api/tools/scte35/build-advanced endpoint
//   - Unified builder functionality now in /api/tools/scte35/build
//   - Added tools_api module for SCTE-35 tools
//   - Code cleanup: removed unused imports (EsamEvent, error, HashMap)

mod models;
mod rules;
mod esam;
mod scte35; // SCTE-35 builder module
mod event_logging; // Events Logging
mod backup; // Backup/restore module
mod jwt_auth; // JWT authentication
mod auth_handlers; // Auth API endpoints
mod tools_api; // SCTE-35 Tools API

use axum::{
    body::Body,
    extract::{ConnectInfo, Extension, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use base64::Engine;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{net::SocketAddr, sync::Arc, time::Instant};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

// Import event logging types
use crate::event_logging::{
    ClientInfo, EventFilters, EventLogger, EsamEventView, ProcessingMetrics,
};

// bring model types into scope
use crate::esam::{build_notification, extract_facts};
use crate::models::{
    Channel, DryRunRequest, DryRunResult, ReorderRules, Rule, UpsertChannel, UpsertRule,
};
use crate::rules::rule_matches;

#[derive(Clone)]
struct AppState {
    db: Pool<Sqlite>,
    admin_token: String,
    event_logger: EventLogger,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("pois_esam_server=info".parse()?),
        )
        .init();

    // --- Config from env ---
    let db_url = std::env::var("POIS_DB").unwrap_or_else(|_| "sqlite://pois.db".to_string());
    let admin_token =
        std::env::var("POIS_ADMIN_TOKEN").unwrap_or_else(|_| "dev-token".to_string());
    let jwt_secret = std::env::var("POIS_JWT_SECRET").unwrap_or_else(|_| {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
        let secret = base64::engine::general_purpose::STANDARD.encode(&bytes);
        eprintln!("⚠️  POIS_JWT_SECRET not set. Generated random secret (set in ENV for persistence!):");
        eprintln!("   POIS_JWT_SECRET={}", secret);
        secret
    });
    let port: u16 = std::env::var("POIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    // --- DB & migrations ---
    let db = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;
    sqlx::migrate!().run(&db).await?;

    // Seed admin user on first install if env vars are set
    if let (Ok(seed_user), Ok(seed_pass)) = (
        std::env::var("POIS_SEED_ADMIN_USER"),
        std::env::var("POIS_SEED_ADMIN_PASS"),
    ) {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&db)
            .await?;
        if count.0 == 0 {
            match crate::jwt_auth::PasswordService::hash_password(&seed_pass) {
                Ok(hash) => {
                    sqlx::query(
                        "INSERT INTO users(username, password_hash, role, enabled) VALUES(?, ?, 'admin', 1)"
                    )
                    .bind(&seed_user)
                    .bind(&hash)
                    .execute(&db)
                    .await?;
                    info!("Seeded admin user '{}'", seed_user);
                }
                Err(e) => {
                    tracing::error!("Failed to hash seed password: {}", e);
                }
            }
        }
    }

    // Seed default channel + rule if DB is empty
    seed_default_channel_and_rule(&db).await?;

    // Initialize event logger
    let event_logger = EventLogger::new(db.clone());

    // Initialize JWT auth service
    let auth_service = jwt_auth::AuthService::new(db.clone(), jwt_secret.clone());

    // Create AuthState for auth endpoints
    let auth_state = Arc::new(auth_handlers::AuthState {
        db: db.clone(),
        auth_service,
    });

    let state = Arc::new(AppState {
        db,
        admin_token,
        event_logger,
    });

    // --- App / routes ---
    
    // Create auth router with AuthState (public endpoints)
    let auth_public = Router::new()
        .route("/api/auth/login", post(auth_handlers::login))
        .route("/api/auth/me", get(auth_handlers::get_current_user))
        .with_state(auth_state.clone());

    // Create JWT-protected user/token management routes
    let auth_protected = Router::new()
        .route("/api/users", get(auth_handlers::list_users).post(auth_handlers::create_user))
        .route("/api/users/{id}", get(auth_handlers::get_user).put(auth_handlers::update_user).delete(auth_handlers::delete_user))
        .route("/api/tokens", get(auth_handlers::list_my_tokens).post(auth_handlers::create_api_token))
        .route("/api/tokens/{id}", delete(auth_handlers::revoke_api_token))
        .with_state(auth_state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            require_jwt_auth,
        ));

    // Create JWT-protected POIS API routes (changed from Bearer to JWT)
    let pois_api = Router::new()
        .route("/api/channels", get(list_channels).post(create_channel))
        .route("/api/channels/{id}", put(update_channel).delete(delete_channel))
        .route("/api/channels/{id}/rules", get(list_rules).post(create_rule))
        .route("/api/rules/{id}", put(update_rule).delete(delete_rule))
        .route("/api/rules/reorder", post(reorder_rules))
        .route("/api/dryrun", post(dryrun))
        .route("/api/tools/scte35/build", post(tools_api::build_scte35))
        .route("/api/tools/scte35/decode", post(tools_api::decode_scte35))
        .route("/api/tools/scte35/validate", post(tools_api::validate_scte35))
        .route("/api/tools/scte35/test-send", post(tools_api::test_send))
        .route("/api/events", get(list_events))
        .route("/api/events/stats", get(get_event_stats))
        .route("/api/events/{id}", get(get_event_detail))
        .route("/api/backup/export/channel/{id}", post(backup::export_channel_only))
        .with_state(state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            require_jwt_auth,
        ));

    // Main app - merge all routers (no with_state at this level!)
    let app = Router::new()
        .route("/esam/channel/{channel}", post(handle_esam_with_path))
        .route("/healthz", get(|| async { "ok" }))
        .route("/esam", post(handle_esam))
        .with_state(state.clone())
        .nest_service("/static", ServeDir::new("static"))
        .route(
            "/",
            get(|| async { axum::response::Redirect::temporary("/static/login.html") }),
        )
        .route(
            "/tools.html",
            get(|| async { axum::response::Redirect::temporary("/static/tools.html") }),
        )
        .route(
            "/events.html",
            get(|| async { axum::response::Redirect::temporary("/static/events.html") }),
        )
        .route(
            "/login.html",
            get(|| async { axum::response::Redirect::temporary("/static/login.html") }),
        )
        .route(
            "/users.html",
            get(|| async { axum::response::Redirect::temporary("/static/users.html") }),
        )
        .route(
            "/tokens.html",
            get(|| async { axum::response::Redirect::temporary("/static/tokens.html") }),
        )
        // Merge all the sub-routers
        .merge(auth_public)
        .merge(auth_protected)
        .merge(pois_api)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // --- Bind (HTTP by default, HTTPS if cert/key provided) ---
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let tls_cert = std::env::var("POIS_TLS_CERT");
    let tls_key = std::env::var("POIS_TLS_KEY");

    if let (Ok(cert_path), Ok(key_path)) = (tls_cert, tls_key) {
        use axum_server::tls_rustls::RustlsConfig;
        let config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        info!("POIS listening with TLS on https://{addr}  (UI: /login.html | Events: /events.html)");
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        info!("POIS listening on http://{addr}  (UI: /login.html | Events: /events.html)");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>()
        ).await?;
    }

    Ok(())
}

// ---------------- JWT authentication middleware ----------------

async fn require_jwt_auth(
    State(auth_state): State<Arc<auth_handlers::AuthState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers().clone();
    
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing authorization token").into_response(),
    };

    match auth_state.auth_service.validate_token(token).await {
        Ok(claims) => {
            // Store claims in request extensions for handlers to use
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(_) => (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response(),
    }
}

// ---------------- Bearer token middleware (legacy admin token) ----------------

async fn require_bearer(
    State(st): axum::extract::State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // Extract Bearer token from Authorization header
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing/invalid token").into_response();
    };

    if token == st.admin_token {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "invalid token").into_response()
    }
}

// ---------------------- ESAM handler ----------------------

async fn handle_esam(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    handle_esam_impl(st, addr, headers, body, None).await
}

async fn handle_esam_with_path(
    State(st): State<Arc<AppState>>,
    Path(channel_name): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    handle_esam_impl(st, addr, headers, body, Some(channel_name)).await
}

async fn handle_esam_impl(
    st: Arc<AppState>,
    addr: SocketAddr,
    headers: HeaderMap,
    body: String,
    path_channel: Option<String>,
) -> impl IntoResponse {
    let start = Instant::now();

    let client_info = ClientInfo {
        source_ip: Some(addr.ip().to_string()),
        user_agent: Some(headers
            .get("user-agent")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string()),
    };

    let facts = match extract_facts(&body) {
        Ok(v) => v,
        Err(e) => {
            let duration = start.elapsed();
            let _ = st
                .event_logger
                .log_esam_event(
                    "unknown",
                    &serde_json::json!({"error": "parse_error"}),
                    None,
                    client_info,
                    ProcessingMetrics {
                        request_size: Some(body.len() as i32),
                        processing_time_ms: Some(duration.as_millis() as i32),
                        response_status: 400,
                        error_message: Some(format!("Parse error: {e}")),
                    },
                    Some(&body),
                    None,
                )
                .await;
            return (StatusCode::BAD_REQUEST, format!("parse error: {e}")).into_response();
        }
    };

    let obj = facts.as_object().cloned().unwrap_or_default();

    // Determine channel name
    let channel_name = path_channel
        .or_else(|| {
            obj.get("ChannelName")
                .or_else(|| obj.get("channelName"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "default".into());

    let ch: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, timezone FROM channels WHERE name=? AND enabled=1 AND deleted_at IS NULL",
    )
    .bind(&channel_name)
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten();

    let Some((channel_id, tz)) = ch else {
        let duration = start.elapsed();
        let _ = st
            .event_logger
            .log_esam_event(
                &channel_name,
                &facts,
                None,
                client_info,
                ProcessingMetrics {
                    request_size: Some(body.len() as i32),
                    processing_time_ms: Some(duration.as_millis() as i32),
                    response_status: 404,
                    error_message: Some("Channel not found or disabled".to_string()),
                },
                Some(&body),
                None,
            )
            .await;
        return (
            StatusCode::NOT_FOUND,
            "channel not found or disabled".to_string(),
        )
            .into_response();
    };

    let rules = match sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id=? AND enabled=1 AND deleted_at IS NULL ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let duration = start.elapsed();
            let _ = st
                .event_logger
                .log_esam_event(
                    &channel_name,
                    &facts,
                    None,
                    client_info,
                    ProcessingMetrics {
                        request_size: Some(body.len() as i32),
                        processing_time_ms: Some(duration.as_millis() as i32),
                        response_status: 500,
                        error_message: Some(format!("DB error: {e}")),
                    },
                    Some(&body),
                    None,
                )
                .await;
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let mut matched_rule: Option<Rule> = None;
    for r in rules {
        let m: serde_json::Value =
            serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
        if rule_matches(&m, &obj) {
            matched_rule = Some(r);
            break;
        }
    }

    if let Some(r) = matched_rule {
        let params: serde_json::Value = serde_json::from_str(&r.params_json).unwrap_or_default();
        let final_params = maybe_build_scte35(params);
        
        let acq_id = facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or("");
        let utc_point = facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or("");
        let resp_xml = build_notification(acq_id, utc_point, &r.action, &final_params);

        let duration = start.elapsed();
        let _ = st
            .event_logger
            .log_esam_event(
                &channel_name,
                &facts,
                Some((&r, &r.action)),
                client_info,
                ProcessingMetrics {
                    request_size: Some(body.len() as i32),
                    processing_time_ms: Some(duration.as_millis() as i32),
                    response_status: 200,
                    error_message: None,
                },
                Some(&body),
                Some(&resp_xml),
            )
            .await;

        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/xml")],
            resp_xml,
        )
            .into_response()
    } else {
        let duration = start.elapsed();
        
        let acq_id = facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or("");
        let utc_point = facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or("");
        let resp_xml = build_notification(acq_id, utc_point, "noop", &serde_json::json!({}));
        
        let _ = st
            .event_logger
            .log_esam_event(
                &channel_name,
                &facts,
                None,
                client_info,
                ProcessingMetrics {
                    request_size: Some(body.len() as i32),
                    processing_time_ms: Some(duration.as_millis() as i32),
                    response_status: 200,
                    error_message: None,
                },
                Some(&body),
                Some(&resp_xml),
            )
            .await;

        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/xml")],
            resp_xml,
        )
            .into_response()
    }
}

// -------------------- Channels with ownership --------------------

async fn list_channels(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
) -> impl IntoResponse {
    let query = if claims.role == "admin" {
        // Admins see all channels (including NULL-owned)
        "SELECT * FROM channels WHERE deleted_at IS NULL ORDER BY name"
    } else {
        // Users see only their own channels
        "SELECT * FROM channels WHERE deleted_at IS NULL AND owner_user_id = ? ORDER BY name"
    };

    let channels: Result<Vec<Channel>, _> = if claims.role == "admin" {
        sqlx::query_as(query).fetch_all(&st.db).await
    } else {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        sqlx::query_as(query).bind(user_id).fetch_all(&st.db).await
    };

    resp(channels)
}

async fn create_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    let enabled = p.enabled.unwrap_or(true) as i64;
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let owner_id: i64 = claims.sub.parse().unwrap_or(0);

    let r = sqlx::query_as::<_, Channel>(
        "INSERT INTO channels(name,enabled,timezone,owner_user_id) VALUES(?,?,?,?) RETURNING *",
    )
    .bind(p.name)
    .bind(enabled)
    .bind(tz)
    .bind(owner_id)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

async fn update_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    // Check ownership
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM channels WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot modify system channel").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Channel not found").into_response();
            }
            _ => {}
        }
    }

    let enabled = p.enabled.map(|b| b as i64);
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let r = sqlx::query_as::<_, Channel>(
        "UPDATE channels 
         SET name=COALESCE(?,name), enabled=COALESCE(?,enabled), timezone=?, 
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') 
         WHERE id=? AND deleted_at IS NULL 
         RETURNING *",
    )
    .bind(Some(p.name))
    .bind(enabled)
    .bind(tz)
    .bind(id)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

async fn delete_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Check ownership
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM channels WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot delete system channel").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Channel not found").into_response();
            }
            _ => {}
        }
    }

    // Soft delete
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let r = sqlx::query(
        "UPDATE channels SET deleted_at=?, enabled=0 WHERE id=? AND deleted_at IS NULL"
    )
    .bind(&now)
    .bind(id)
    .execute(&st.db)
    .await
    .map(|_| ());
    resp(r)
}

// -------------------- Rules with ownership --------------------

async fn list_rules(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(channel_id): Path<i64>,
) -> impl IntoResponse {
    // Check channel ownership first
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM channels WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(channel_id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot access system channel").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Channel not found").into_response();
            }
            _ => {}
        }
    }

    let rows = sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules 
         WHERE channel_id=? AND deleted_at IS NULL 
         ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await;
    resp(rows)
}

async fn create_rule(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(channel_id): Path<i64>,
    Json(mut p): Json<UpsertRule>,
) -> impl IntoResponse {
    // Check channel ownership
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM channels WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(channel_id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot modify system channel").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Channel not found").into_response();
            }
            _ => {}
        }
    }

    let owner_id: i64 = claims.sub.parse().unwrap_or(0);

    // space priorities by 10; append if negative
    let maxp: Option<(i64,)> = sqlx::query_as(
        "SELECT MAX(priority) FROM rules WHERE channel_id=? AND deleted_at IS NULL"
    )
    .bind(channel_id)
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten();
    let nextp = maxp.map(|t| t.0 + 10).unwrap_or(0);
    if p.priority < 0 {
        p.priority = nextp;
    }

    let r = sqlx::query_as::<_, Rule>(
        "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json,owner_user_id) 
         VALUES(?,?,?,?,?,?,?,?) RETURNING *"
    )
    .bind(channel_id)
    .bind(p.name)
    .bind(p.priority)
    .bind(p.enabled.unwrap_or(true) as i64)
    .bind(p.match_json.to_string())
    .bind(p.action)
    .bind(p.params_json.to_string())
    .bind(owner_id)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

async fn update_rule(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertRule>,
) -> impl IntoResponse {
    // Check ownership
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM rules WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your rule").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot modify system rule").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Rule not found").into_response();
            }
            _ => {}
        }
    }

    let r = sqlx::query_as::<_, Rule>(
        "UPDATE rules 
         SET name=?, priority=?, enabled=?, match_json=?, action=?, params_json=?, 
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') 
         WHERE id=? AND deleted_at IS NULL 
         RETURNING *",
    )
    .bind(p.name)
    .bind(p.priority)
    .bind(p.enabled.unwrap_or(true) as i64)
    .bind(p.match_json.to_string())
    .bind(p.action)
    .bind(p.params_json.to_string())
    .bind(id)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

async fn delete_rule(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Check ownership
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT owner_user_id FROM rules WHERE id = ? AND deleted_at IS NULL"
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();

        match owner {
            Some((Some(owner_id),)) if owner_id != user_id => {
                return (StatusCode::FORBIDDEN, "Not your rule").into_response();
            }
            Some((None,)) => {
                return (StatusCode::FORBIDDEN, "Cannot delete system rule").into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "Rule not found").into_response();
            }
            _ => {}
        }
    }

    // Soft delete
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let r = sqlx::query(
        "UPDATE rules SET deleted_at=?, enabled=0 WHERE id=? AND deleted_at IS NULL"
    )
    .bind(&now)
    .bind(id)
    .execute(&st.db)
    .await
    .map(|_| ());
    resp(r)
}

async fn reorder_rules(
    State(st): State<Arc<AppState>>,
    Json(p): Json<ReorderRules>,
) -> impl IntoResponse {
    let mut tx = match st.db.begin().await {
        Ok(t) => t,
        Err(e) => return err(e),
    };
    let mut prio = 0i64;
    for id in p.ordered_ids {
        if let Err(e) = sqlx::query(
            "UPDATE rules SET priority=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
        )
        .bind(prio)
        .bind(id)
        .execute(&mut *tx)
        .await
        {
            let _ = tx.rollback().await;
            return err(e);
        }
        prio += 10;
    }
    if let Err(e) = tx.commit().await {
        return err(e);
    }
    (StatusCode::NO_CONTENT, ()).into_response()
}

async fn dryrun(
    State(st): State<Arc<AppState>>,
    Json(p): Json<DryRunRequest>,
) -> impl IntoResponse {
    let facts = match extract_facts(&p.esam_xml) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("parse error: {e}"),
            )
                .into_response()
        }
    };
    let ch: Option<(i64,)> =
        match sqlx::query_as("SELECT id FROM channels WHERE name=? AND enabled=1 AND deleted_at IS NULL")
            .bind(&p.channel)
            .fetch_optional(&st.db)
            .await
        {
            Ok(v) => v,
            Err(e) => return err(e),
        };
    let Some((channel_id,)) = ch else {
        return (
            StatusCode::NOT_FOUND,
            "channel not found or disabled",
        )
            .into_response();
    };

    let rules =
        match sqlx::query_as::<_, Rule>(
            "SELECT * FROM rules WHERE channel_id=? AND enabled=1 AND deleted_at IS NULL ORDER BY priority",
        )
        .bind(channel_id)
        .fetch_all(&st.db)
        .await
        {
            Ok(v) => v,
            Err(e) => return err(e),
        };

    let map = facts.as_object().cloned().unwrap_or_default();
    for r in rules {
        let m: serde_json::Value =
            serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
        if rule_matches(&m, &map) {
            return Json(DryRunResult {
                matched_rule_id: Some(r.id),
                action: r.action.clone(),
                note: "first matching rule".into(),
            })
            .into_response();
        }
    }
    Json(DryRunResult {
        matched_rule_id: None,
        action: "noop".into(),
        note: "no rules matched".into(),
    })
    .into_response()
}

// ---------- Builder endpoint & helper ----------

fn maybe_build_scte35(mut params: serde_json::Value) -> serde_json::Value {
    if let Some(build) = params.get("build").cloned() {
        if let Some(cmd) = build.get("command").and_then(|v| v.as_str()) {
            let out = match cmd {
                "time_signal_immediate" => scte35::build_time_signal_immediate_b64(),
                "splice_insert_out" => {
                    let dur = build
                        .get("duration_s")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    scte35::build_splice_insert_out_b64(dur)
                }
                _ => String::new(),
            };
            if !out.is_empty() {
                params["scte35_b64"] = serde_json::Value::String(out);
            }
        }
    }
    params
}

// ---------------------- Event logging endpoints ----------------------

async fn list_events(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let limit: i64 = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(100);
    let offset: i64 = params.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
    
    // For non-admins, filter events by owned channels
    let channel_filter = if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owned_channels: Vec<(String,)> = match sqlx::query_as(
            "SELECT name FROM channels WHERE owner_user_id = ? AND deleted_at IS NULL"
        )
        .bind(user_id)
        .fetch_all(&st.db)
        .await
        {
            Ok(channels) => channels,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        
        if owned_channels.is_empty() {
            return Json(Vec::<EsamEventView>::new()).into_response();
        }
        
        params.get("channel").cloned().or_else(|| Some(owned_channels[0].0.clone()))
    } else {
        params.get("channel").cloned()
    };

    let filters = EventFilters {
        channel_name: channel_filter,
        action: params.get("action").cloned(),
        since: params.get("since").cloned(),
    };

    match st.event_logger.get_recent_events(limit, offset, Some(filters)).await {
        Ok(events) => {
            if claims.role != "admin" {
                let user_id: i64 = claims.sub.parse().unwrap_or(0);
                let owned_channel_names: Vec<String> = match sqlx::query_as(
                    "SELECT name FROM channels WHERE owner_user_id = ? AND deleted_at IS NULL"
                )
                .bind(user_id)
                .fetch_all(&st.db)
                .await
                {
                    Ok(channels) => channels.into_iter().map(|(name,)| name).collect(),
                    Err(_) => vec![],
                };
                
                let filtered_events: Vec<EsamEventView> = events
                    .into_iter()
                    .filter(|e| owned_channel_names.contains(&e.channel_name))
                    .collect();
                
                Json(filtered_events).into_response()
            } else {
                Json(events).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_event_stats(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
) -> impl IntoResponse {
    if claims.role != "admin" {
        // For non-admins, we'd need custom queries to filter by owned channels
        // For now, return empty stats (this should be enhanced later)
        return Json(serde_json::json!({
            "total_events": 0,
            "last_24h_events": 0,
            "action_counts": {},
            "avg_processing_time_ms": null
        })).into_response();
    }
    
    // Admins get full stats
    match st.event_logger.get_event_stats().await {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_event_detail(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let event: Result<Option<EsamEventView>, _> = sqlx::query_as(
        "SELECT * FROM esam_events_view WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await;
    
    match event {
        Ok(Some(ev)) => {
            // For non-admins, check if they own the channel
            if claims.role != "admin" {
                let user_id: i64 = claims.sub.parse().unwrap_or(0);
                let owned: Option<(i64,)> = sqlx::query_as(
                    "SELECT COUNT(*) FROM channels WHERE name = ? AND owner_user_id = ? AND deleted_at IS NULL"
                )
                .bind(&ev.channel_name)
                .bind(user_id)
                .fetch_optional(&st.db)
                .await
                .ok()
                .flatten();
                
                if owned.map(|(c,)| c).unwrap_or(0) == 0 {
                    return (StatusCode::FORBIDDEN, "Not your channel").into_response();
                }
            }
            Json(ev).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Event not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ------------------------ DB seeding helper ------------------------

async fn seed_default_channel_and_rule(db: &Pool<Sqlite>) -> anyhow::Result<()> {
    // Check if any channels exist
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels")
        .fetch_one(db)
        .await?;

    if count == 0 {
        // Insert a default channel (owner_user_id = NULL for system channel)
        let default_channel: Channel = sqlx::query_as(
            "INSERT INTO channels(name,enabled,timezone,owner_user_id) VALUES(?,?,?,NULL) RETURNING *",
        )
        .bind("default")
        .bind(1_i64)
        .bind("UTC")
        .fetch_one(db)
        .await?;

        // Insert a default noop rule for that channel (owner_user_id = NULL for system rule)
        sqlx::query(
            "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json,owner_user_id) \
             VALUES(?,?,?,?,?,?,?,NULL)",
        )
        .bind(default_channel.id)
        .bind("Default noop")
        .bind(0_i64)
        .bind(1_i64)
        .bind("{}") // empty match_json
        .bind("noop")
        .bind("{}") // empty params_json
        .execute(db)
        .await?;
    }

    Ok(())
}

// ----------------------------- helpers -----------------------------

fn resp<T: serde::Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}
fn err<E: std::fmt::Display>(e: E) -> Response {
    (StatusCode::BAD_REQUEST, e.to_string()).into_response()
}