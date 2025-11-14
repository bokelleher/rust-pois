// src/main.rs
// Version: 3.0.4
// Last Modified: 2025-01-14
// Changes: Added server-side HTML templating with shared header support

mod models;
mod rules;
mod esam;
mod scte35;
mod event_logging;
mod backup;
mod jwt_auth;
mod auth_handlers;
mod templates;

use axum::{
    body::Body,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use base64::Engine;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::templates::TemplateEngine;

use crate::event_logging::{
    ClientInfo, EsamEvent, EsamEventView, EventFilters, EventLogger, ProcessingMetrics,
};

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
    template_engine: Arc<TemplateEngine>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("pois_esam_server=info".parse()?),
        )
        .init();

    info!("Starting POIS v3.0.4 - Multi-tenancy enabled");

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

    let db = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;
    sqlx::migrate!().run(&db).await?;

    seed_default_channel_and_rule(&db).await?;

    let event_logger = EventLogger::new(db.clone());
    let auth_service = jwt_auth::AuthService::new(db.clone(), jwt_secret.clone());
    let template_engine = Arc::new(TemplateEngine::new("static"));

    let auth_state = Arc::new(auth_handlers::AuthState {
        db: db.clone(),
        auth_service,
    });

    let state = Arc::new(AppState {
        db,
        admin_token,
        event_logger,
        template_engine,
    });

    let auth_public = Router::new()
        .route("/api/auth/login", post(auth_handlers::login))
        .route("/api/auth/me", get(auth_handlers::get_current_user))
        .with_state(auth_state.clone());

    let auth_protected = Router::new()
        .route("/api/users", get(auth_handlers::list_users).post(auth_handlers::create_user))
        .route("/api/users/:id", get(auth_handlers::get_user).put(auth_handlers::update_user).delete(auth_handlers::delete_user))
        .route("/api/tokens", get(auth_handlers::list_my_tokens).post(auth_handlers::create_api_token))
        .route("/api/tokens/:id", delete(auth_handlers::revoke_api_token))
        .with_state(auth_state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            require_jwt_auth,
        ));

    let pois_api = Router::new()
        .route("/api/channels", get(list_channels).post(create_channel))
        .route("/api/channels/:id", put(update_channel).delete(delete_channel))
        .route("/api/channels/:id/rules", get(list_rules).post(create_rule))
        .route("/api/rules/:id", put(update_rule).delete(delete_rule))
        .route("/api/rules/reorder", post(reorder_rules))
        .route("/api/dryrun", post(dryrun))
        .route("/api/tools/scte35/build", post(build_scte35))
        .route("/api/events", get(list_events))
        .route("/api/events/stats", get(get_event_stats))
        .route("/api/events/:id", get(get_event_detail))
        .route("/api/backup/export/channel/:id", post(backup::export_channel_only))
        .with_state(state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            require_jwt_auth,
        ));

    let app = Router::new()
        .route("/esam/channel/:channel", post(handle_esam_with_path))
        .route("/healthz", get(|| async { "ok" }))
        .route("/esam", post(handle_esam))
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(|| async { axum::response::Redirect::temporary("/events") }))
        .route("/admin", get(|| async { axum::response::Redirect::temporary("/static/admin.html") }))
        .route("/admin.html", get(|| async { axum::response::Redirect::temporary("/static/admin.html") }))
        .route("/events", get(serve_events))
        .route("/events.html", get(serve_events))
        .route("/tools", get(serve_tools))
        .route("/tools.html", get(serve_tools))
        .route("/docs", get(serve_docs))
        .route("/docs.html", get(serve_docs))
        .route("/users", get(serve_users))
        .route("/users.html", get(serve_users))
        .route("/tokens", get(serve_tokens))
        .route("/tokens.html", get(serve_tokens))
        .route("/login", get(serve_login))
        .route("/login.html", get(serve_login))
        .merge(auth_public)
        .merge(auth_protected)
        .merge(pois_api)
        .with_state(state.clone())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let tls_cert = std::env::var("POIS_TLS_CERT");
    let tls_key = std::env::var("POIS_TLS_KEY");

    if let (Ok(cert_path), Ok(key_path)) = (tls_cert, tls_key) {
        use axum_server::tls_rustls::RustlsConfig;
        let config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        info!("POIS listening with TLS on https://{addr}");
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service())
            .await?;
    } else {
        info!("POIS listening on http://{addr}");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app.into_make_service()).await?;
    }

    Ok(())
}

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
        None => {
            info!("JWT auth failed: Missing authorization token");
            return (StatusCode::UNAUTHORIZED, "Missing authorization token").into_response();
        }
    };

    match auth_state.auth_service.validate_token(token).await {
        Ok(claims) => {
            info!("JWT auth success: user_id={}, username={}, role={}", 
                  claims.sub, claims.username, claims.role);
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(e) => {
            info!("JWT auth failed: {}", e);
            (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response()
        }
    }
}

async fn require_bearer(
    State(st): axum::extract::State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
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

// Template rendering handlers
async fn serve_events() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/events.html")
}

async fn serve_tools() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/tools.html")
}

async fn serve_docs() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/docs.html")
}

async fn serve_users() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/admin.html")
}

async fn serve_tokens() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/admin.html")
}

async fn serve_login() -> impl IntoResponse {
    axum::response::Redirect::temporary("/static/admin.html")
}

async fn render_template(st: &AppState, template: &str, title: &str) -> Response {
    let mut vars = HashMap::new();
    vars.insert("page_title".to_string(), title.to_string());
    
    match st.template_engine.render(template, Some(&vars)) {
        Ok(html) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                "text/html; charset=utf-8".parse().unwrap(),
            );
            (StatusCode::OK, headers, html).into_response()
        }
        Err(e) => {
            info!("Template error for {}: {}", template, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template error: {}", e),
            )
                .into_response()
        }
    }
}

async fn handle_esam(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    handle_esam_impl(st, headers, body, None).await
}

async fn handle_esam_with_path(
    State(st): State<Arc<AppState>>,
    Path(channel_name): Path<String>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    handle_esam_impl(st, headers, body, Some(channel_name)).await
}

async fn handle_esam_impl(
    st: Arc<AppState>,
    headers: HeaderMap,
    body: String,
    path_channel: Option<String>,
) -> impl IntoResponse {
    let start = Instant::now();
    let client_info = ClientInfo::from_headers_and_addr(&headers, None);

    let facts = match extract_facts(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("parse error: {e}")).into_response(),
    };

    let obj = facts.as_object().cloned().unwrap_or_default();

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

    let Some((channel_id, _tz)) = ch else {
        return (StatusCode::NOT_FOUND, "channel not found or disabled".to_string()).into_response();
    };

    let rules = match sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id=? AND enabled=1 AND deleted_at IS NULL ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
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
        
        let acq_id = obj.get("acquisitionSignalID")
            .or_else(|| obj.get("AcquisitionSignalID"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let utc = obj.get("utcPoint")
            .or_else(|| obj.get("UTCPoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        let resp_xml = build_notification(acq_id, utc, &r.action, &final_params);

        let duration = start.elapsed();
        let _ = st
            .event_logger
            .log_esam_event(
                &channel_name,
                &facts,
                Some((&r, r.action.as_str())),
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
        
        let acq_id = obj.get("acquisitionSignalID")
            .or_else(|| obj.get("AcquisitionSignalID"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let utc = obj.get("utcPoint")
            .or_else(|| obj.get("UTCPoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        let resp_xml = build_notification(acq_id, utc, "noop", &serde_json::json!({}));
        
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

async fn list_channels(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
) -> impl IntoResponse {
    info!("list_channels called by user_id={}, role={}", claims.sub, claims.role);
    
    let query = if claims.role == "admin" {
        "SELECT * FROM channels WHERE deleted_at IS NULL ORDER BY name"
    } else {
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

    info!("Creating channel '{}' - User: {} (sub={}), Role: {}, Parsed owner_id: {}", 
          p.name, claims.username, claims.sub, claims.role, owner_id);

    let r = sqlx::query_as::<_, Channel>(
        "INSERT INTO channels(name,enabled,timezone,owner_user_id) VALUES(?,?,?,?) RETURNING *",
    )
    .bind(&p.name)
    .bind(enabled)
    .bind(tz)
    .bind(owner_id)
    .fetch_one(&st.db)
    .await;
    
    match &r {
        Ok(ch) => info!("Channel '{}' created: id={}, owner_user_id={:?}", p.name, ch.id, ch.owner_user_id),
        Err(e) => info!("Channel '{}' creation failed: {}", p.name, e),
    }
    
    resp(r)
}

async fn update_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
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
                info!("update_channel denied: user {} tried to modify channel {} owned by {}", user_id, id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                info!("update_channel denied: user {} tried to modify system channel {}", user_id, id);
                return (StatusCode::FORBIDDEN, "Cannot modify system channel").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Channel not found").into_response(),
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
                info!("delete_channel denied: user {} tried to delete channel {} owned by {}", user_id, id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                info!("delete_channel denied: user {} tried to delete system channel {}", user_id, id);
                return (StatusCode::FORBIDDEN, "Cannot delete system channel").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Channel not found").into_response(),
            _ => {}
        }
    }

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

async fn list_rules(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(channel_id): Path<i64>,
) -> impl IntoResponse {
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
                info!("list_rules denied: user {} tried to access channel {} owned by {}", user_id, channel_id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                info!("list_rules denied: user {} tried to access system channel {}", user_id, channel_id);
                return (StatusCode::FORBIDDEN, "Cannot access system channel").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Channel not found").into_response(),
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
                info!("create_rule denied: user {} tried to modify channel {} owned by {}", user_id, channel_id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your channel").into_response();
            }
            Some((None,)) => {
                info!("create_rule denied: user {} tried to modify system channel {}", user_id, channel_id);
                return (StatusCode::FORBIDDEN, "Cannot modify system channel").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Channel not found").into_response(),
            _ => {}
        }
    }

    let owner_id: i64 = claims.sub.parse().unwrap_or(0);

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
                info!("update_rule denied: user {} tried to modify rule {} owned by {}", user_id, id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your rule").into_response();
            }
            Some((None,)) => {
                info!("update_rule denied: user {} tried to modify system rule {}", user_id, id);
                return (StatusCode::FORBIDDEN, "Cannot modify system rule").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Rule not found").into_response(),
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
                info!("delete_rule denied: user {} tried to delete rule {} owned by {}", user_id, id, owner_id);
                return (StatusCode::FORBIDDEN, "Not your rule").into_response();
            }
            Some((None,)) => {
                info!("delete_rule denied: user {} tried to delete system rule {}", user_id, id);
                return (StatusCode::FORBIDDEN, "Cannot delete system rule").into_response();
            }
            None => return (StatusCode::NOT_FOUND, "Rule not found").into_response(),
            _ => {}
        }
    }

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
        Err(e) => return (StatusCode::BAD_REQUEST, format!("parse error: {e}")).into_response(),
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
        return (StatusCode::NOT_FOUND, "channel not found or disabled").into_response();
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

#[derive(serde::Deserialize)]
struct BuildReq {
    command: String,
    duration_s: Option<u32>,
}

#[derive(serde::Serialize)]
struct BuildResp {
    scte35_b64: String,
}

async fn build_scte35(Json(req): Json<BuildReq>) -> impl IntoResponse {
    let b64 = match req.command.as_str() {
        "time_signal_immediate" => scte35::build_time_signal_immediate_b64(),
        "splice_insert_out" => scte35::build_splice_insert_out_b64(req.duration_s.unwrap_or(0)),
        _ => return (StatusCode::BAD_REQUEST, "unknown command").into_response(),
    };
    Json(BuildResp { scte35_b64: b64 }).into_response()
}

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

async fn list_events(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(100).min(1000);
    let offset = params.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);

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
    // For non-admins, filter stats by their owned channels
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        
        // Get list of channel names owned by this user
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
            // User has no channels, return zero stats
            return Json(serde_json::json!({
                "total_events": 0,
                "last_24h_events": 0,
                "action_counts": {},
                "avg_processing_time_ms": null
            })).into_response();
        }
        
        let channel_names: Vec<String> = owned_channels.into_iter().map(|(name,)| name).collect();
        let placeholders = channel_names.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        
        // Count total events for user's channels
        let total_query = format!("SELECT COUNT(*) FROM esam_events WHERE channel_name IN ({})", placeholders);
        let mut total_q = sqlx::query_scalar::<_, i64>(&total_query);
        for name in &channel_names {
            total_q = total_q.bind(name);
        }
        let total_events: i64 = match total_q.fetch_one(&st.db).await {
            Ok(count) => count,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        
        // Count last 24h events
        let last_24h_query = format!(
            "SELECT COUNT(*) FROM esam_events WHERE channel_name IN ({}) AND timestamp >= datetime('now', '-1 day')",
            placeholders
        );
        let mut last_24h_q = sqlx::query_scalar::<_, i64>(&last_24h_query);
        for name in &channel_names {
            last_24h_q = last_24h_q.bind(name);
        }
        let last_24h_events: i64 = match last_24h_q.fetch_one(&st.db).await {
            Ok(count) => count,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        
        // Get action counts
        let action_query = format!(
            "SELECT action, COUNT(*) as count FROM esam_events 
             WHERE channel_name IN ({}) AND timestamp >= datetime('now', '-1 day') 
             GROUP BY action ORDER BY count DESC",
            placeholders
        );
        let mut action_q = sqlx::query_as::<_, (String, i64)>(&action_query);
        for name in &channel_names {
            action_q = action_q.bind(name);
        }
        let action_stats: Vec<(String, i64)> = match action_q.fetch_all(&st.db).await {
            Ok(stats) => stats,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        
        // Get average processing time
        let avg_query = format!(
            "SELECT AVG(processing_time_ms) FROM esam_events 
             WHERE channel_name IN ({}) AND timestamp >= datetime('now', '-1 day') 
             AND processing_time_ms IS NOT NULL",
            placeholders
        );
        let mut avg_q = sqlx::query_scalar::<_, Option<f64>>(&avg_query);
        for name in &channel_names {
            avg_q = avg_q.bind(name);
        }
        let avg_processing_time: Option<f64> = match avg_q.fetch_one(&st.db).await {
            Ok(avg) => avg,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        
        Json(serde_json::json!({
            "total_events": total_events,
            "last_24h_events": last_24h_events,
            "action_counts": action_stats.into_iter().collect::<std::collections::HashMap<_, _>>(),
            "avg_processing_time_ms": avg_processing_time
        })).into_response()
    } else {
        // Admin sees all stats
        match st.event_logger.get_event_stats().await {
            Ok(stats) => Json(stats).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}

async fn get_event_detail(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match sqlx::query_as::<_, EsamEvent>("SELECT * FROM esam_events WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await
    {
        Ok(Some(event)) => {
            if claims.role != "admin" {
                let user_id: i64 = claims.sub.parse().unwrap_or(0);
                let is_owner: Option<(i64,)> = sqlx::query_as(
                    "SELECT 1 FROM channels WHERE name = ? AND owner_user_id = ? AND deleted_at IS NULL"
                )
                .bind(&event.channel_name)
                .bind(user_id)
                .fetch_optional(&st.db)
                .await
                .ok()
                .flatten();
                
                if is_owner.is_none() {
                    return (StatusCode::FORBIDDEN, "Not your event").into_response();
                }
            }
            
            Json(event).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "event not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn seed_default_channel_and_rule(db: &Pool<Sqlite>) -> anyhow::Result<()> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels")
        .fetch_one(db)
        .await?;

    if count == 0 {
        let default_channel: Channel = sqlx::query_as(
            "INSERT INTO channels(name,enabled,timezone,owner_user_id) VALUES(?,?,?,NULL) RETURNING *",
        )
        .bind("default")
        .bind(1_i64)
        .bind("UTC")
        .fetch_one(db)
        .await?;

        sqlx::query(
            "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json,owner_user_id) \
             VALUES(?,?,?,?,?,?,?,NULL)",
        )
        .bind(default_channel.id)
        .bind("Default noop")
        .bind(0_i64)
        .bind(1_i64)
        .bind("{}")
        .bind("noop")
        .bind("{}")
        .execute(db)
        .await?;
    }

    Ok(())
}

fn resp<T: serde::Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

fn err<E: std::fmt::Display>(e: E) -> Response {
    (StatusCode::BAD_REQUEST, e.to_string()).into_response()
}