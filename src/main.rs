// src/main.rs
mod models;
mod rules;
mod esam;
mod scte35; // SCTE-35 builder module
mod event_logging; // Events Logging

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

// Import event logging types
use crate::event_logging::{
    ClientInfo, EsamEvent, EventFilters, EventLogger, ProcessingMetrics, EsamEventView,
};

// bring model types into scope
use crate::esam::{build_notification, extract_facts};
use crate::models::{
    Channel, DryRunRequest, DryRunResult, ExportedChannel, ExportedRule, ReorderRules, Rule,
    RulesBackup, UpsertChannel, UpsertRule,
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

    // Seed default channel + rule if DB is empty
    seed_default_channel_and_rule(&db).await?;

    // Initialize event logger
    let event_logger = EventLogger::new(db.clone());

    let state = Arc::new(AppState {
        db,
        admin_token,
        event_logger,
    });

    // --- App / routes ---
    let app = Router::new()
        .route("/esam/channel/:channel", post(handle_esam_with_path)) // NEW: path-param variant
        .route("/healthz", get(|| async { "ok" }))
        .route("/esam", post(handle_esam))
        .nest_service("/static", ServeDir::new("static"))
        .route(
            "/",
            get(|| async { axum::response::Redirect::temporary("/static/admin.html") }),
        )
        .route(
            "/tools.html",
            get(|| async { axum::response::Redirect::temporary("/static/tools.html") }),
        )
        .route(
            "/events.html",
            get(|| async { axum::response::Redirect::temporary("/static/events.html") }),
        ) // NEW
        .nest(
            "/api",
            {
                let api = api_router().with_state(state.clone()).route_layer(
                    axum::middleware::from_fn_with_state(state.clone(), require_bearer),
                );
                api
            },
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // --- Bind (HTTP by default, HTTPS if cert/key provided) ---
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let tls_cert = std::env::var("POIS_TLS_CERT");
    let tls_key = std::env::var("POIS_TLS_KEY");

    if let (Ok(cert_path), Ok(key_path)) = (tls_cert, tls_key) {
        use axum_server::tls_rustls::RustlsConfig;
        let config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        info!("POIS listening with TLS on https://{addr}  (UI: / | Events: /events.html)");
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        info!("POIS listening on http://{addr}  (UI: / | Events: /events.html)");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    }

    Ok(())
}

fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/channels", get(list_channels).post(create_channel))
        .route("/channels/:id", put(update_channel).delete(delete_channel))
        .route("/channels/:id/rules", get(list_rules).post(create_rule))
        .route("/rules/:id", put(update_rule).delete(delete_rule))
        .route("/rules/reorder", post(reorder_rules))
        .route("/backup", get(export_backup).post(import_backup))
        .route("/dryrun", post(dryrun))
        // SCTE-35 builder
        .route("/tools/scte35/build", post(build_scte35))
        // NEW: Event monitoring endpoints
        .route("/events", get(list_events))
        .route("/events/stats", get(get_event_stats))
        .route("/events/:id", get(get_event_detail))
}

// ---------------- middleware ----------------

async fn require_bearer(
    State(st): axum::extract::State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ok = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|s| s == format!("Bearer {}", st.admin_token))
        .unwrap_or(false);

    if !ok {
        return (StatusCode::UNAUTHORIZED, "missing/invalid token").into_response();
    }
    next.run(req).await
}

// ---------------- ESAM endpoint ----------------

#[derive(serde::Deserialize)]
struct EsamQuery {
    channel: Option<String>,
}

// NEW: support /esam/channel/:channel
#[derive(serde::Deserialize)]
struct EsamPath {
    channel: String,
}

// Updated handle_esam with event logging AND noop fix
async fn handle_esam(
    State(st): State<Arc<AppState>>,
    Query(q): Query<EsamQuery>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: String,
) -> impl IntoResponse {
    let start_time = Instant::now();
    let request_size = body.len() as i32;

    // Extract client information for logging
    let client_info = ClientInfo::from_headers_and_addr(&headers, Some(addr));

    // Extract facts from ESAM request
    let facts = match extract_facts(&body) {
        Ok(v) => v,
        Err(e) => {
            error!("ESAM parse error: {e}");

            // Log failed parsing attempt
            let channel_name = q
                .channel
                .or_else(|| {
                    headers
                        .get("X-POIS-Channel")
                        .and_then(|v| v.to_str().ok().map(String::from))
                })
                .unwrap_or_else(|| "unknown".to_string());

            let metrics = ProcessingMetrics {
                request_size: Some(request_size),
                processing_time_ms: Some(start_time.elapsed().as_millis() as i32),
                response_status: 400,
                error_message: Some(format!("ESAM parsing failed: {}", e)),
            };

            // Create minimal facts for logging
            let error_facts = serde_json::json!({
                "acquisitionSignalID": "parse-error",
                "utcPoint": "1970-01-01T00:00:00Z",
            });

            if let Err(log_err) = st
                .event_logger
                .log_esam_event(
                    &channel_name,
                    &error_facts,
                    None,
                    client_info,
                    metrics,
                    Some(&body),
                    None,
                )
                .await
            {
                error!("Failed to log ESAM event: {}", log_err);
            }

            return (StatusCode::BAD_REQUEST, "invalid ESAM payload").into_response();
        }
    };

    let acq_id = facts
        .get("acquisitionSignalID")
        .and_then(|v| v.as_str())
        .unwrap_or("no-id");
    let utc = facts
        .get("utcPoint")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    // CRITICAL FIX: Preserve original SCTE-35 payload for noop actions
    let original_scte35_b64 = facts
        .get("scte35_b64")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Resolve channel via query/header, fallback to "default"
    let channel_name = q
        .channel
        .or_else(|| {
            headers
                .get("X-POIS-Channel")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "default".to_string());

    // Load channel and rules
    let ch: Option<(i64,)> =
        match sqlx::query_as("SELECT id FROM channels WHERE name=? AND enabled=1")
            .bind(&channel_name)
            .fetch_optional(&st.db)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                error!("DB error loading channel: {e}");

                let metrics = ProcessingMetrics {
                    request_size: Some(request_size),
                    processing_time_ms: Some(start_time.elapsed().as_millis() as i32),
                    response_status: 500,
                    error_message: Some("Database error".to_string()),
                };

                if let Err(log_err) = st
                    .event_logger
                    .log_esam_event(
                        &channel_name,
                        &facts,
                        None,
                        client_info,
                        metrics,
                        Some(&body),
                        None,
                    )
                    .await
                {
                    error!("Failed to log ESAM event: {}", log_err);
                }

                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        };

    let (action, mut params, matched_rule) = if let Some((channel_id,)) = ch {
        let rules =
            match sqlx::query_as::<_, Rule>(
                "SELECT * FROM rules WHERE channel_id=? AND enabled=1 ORDER BY priority",
            )
            .bind(channel_id)
            .fetch_all(&st.db)
            .await
            {
                Ok(rules) => rules,
                Err(e) => {
                    error!("DB error loading rules: {e}");

                    let metrics = ProcessingMetrics {
                        request_size: Some(request_size),
                        processing_time_ms: Some(start_time.elapsed().as_millis() as i32),
                        response_status: 500,
                        error_message: Some("Database error loading rules".to_string()),
                    };

                    if let Err(log_err) = st
                        .event_logger
                        .log_esam_event(
                            &channel_name,
                            &facts,
                            None,
                            client_info,
                            metrics,
                            Some(&body),
                            None,
                        )
                        .await
                    {
                        error!("Failed to log ESAM event: {}", log_err);
                    }

                    return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
                }
            };

        let map = facts.as_object().cloned().unwrap_or_default();
        let mut chosen: Option<(String, serde_json::Value, Rule)> = None;

        for r in rules {
            let m: serde_json::Value =
                serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
            if rule_matches(&m, &map) {
                let p: serde_json::Value =
                    serde_json::from_str(&r.params_json).unwrap_or(serde_json::json!({}));
                chosen = Some((r.action.clone(), p, r));
                break;
            }
        }

        match chosen {
            Some((action, params, rule)) => (action, params, Some(rule)),
            None => ("noop".into(), serde_json::json!({}), None),
        }
    } else {
        ("noop".into(), serde_json::json!({}), None)
    };

    // CRITICAL FIX: For noop action, inject original SCTE-35 payload into params
    if action.eq_ignore_ascii_case("noop") {
        if let Some(ref original_b64) = original_scte35_b64 {
            params["scte35_b64"] = serde_json::Value::String(original_b64.clone());
        }
    }

    // Auto-build SCTE-35 if params carry a "build" object
    params = maybe_build_scte35(params);

    // Build ESAM response XML
    let xml_response = build_notification(acq_id, utc, &action, &params);

    // Calculate final metrics
    let processing_time_ms = start_time.elapsed().as_millis() as i32;
    let response_status = 200;

    let metrics = ProcessingMetrics {
        request_size: Some(request_size),
        processing_time_ms: Some(processing_time_ms),
        response_status,
        error_message: None,
    };

    // Log the successful event
    let matched_rule_info = matched_rule.as_ref().map(|rule| (rule, action.as_str()));
    if let Err(e) = st
        .event_logger
        .log_esam_event(
            &channel_name,
            &facts,
            matched_rule_info,
            client_info,
            metrics,
            Some(&body),
            Some(&xml_response),
        )
        .await
    {
        error!("Failed to log ESAM event: {}", e);
        // Continue processing - logging failure shouldn't break the main flow
    }

    // Return response
    let mut out_headers = HeaderMap::new();
    out_headers.insert(
        axum::http::header::CONTENT_TYPE,
        "application/xml".parse().unwrap(),
    );
    (StatusCode::OK, out_headers, xml_response).into_response()
}

// NEW: wrapper that forwards to the existing handler, filling EsamQuery from the path
async fn handle_esam_with_path(
    State(st): State<Arc<AppState>>,
    Path(p): Path<EsamPath>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: String,
) -> impl IntoResponse {
    // Reuse the exact logic in handle_esam by constructing the same inputs
    let q = EsamQuery {
        channel: Some(p.channel),
    };
    handle_esam(State(st), Query(q), headers, ConnectInfo(addr), body).await
}

// Event logging API handlers
async fn list_events(
    State(st): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
        .min(1000); // Cap at 1000

    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let filters = EventFilters {
        channel_name: params.get("channel").cloned(),
        action: params.get("action").cloned(),
        since: params.get("since").cloned(),
    };

    match st
        .event_logger
        .get_recent_events(limit, offset, Some(filters))
        .await
    {
        Ok(events) => Json(events).into_response(),
        Err(e) => {
            error!("Failed to fetch events: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to fetch events").into_response()
        }
    }
}

async fn get_event_stats(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.event_logger.get_event_stats().await {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => {
            error!("Failed to fetch stats: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch stats",
            )
                .into_response()
        }
    }
}

async fn get_event_detail(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match sqlx::query_as::<_, EsamEvent>("SELECT * FROM esam_events WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await
    {
        Ok(Some(event)) => Json(event).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "Event not found").into_response(),
        Err(e) => {
            error!("Failed to fetch event {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            )
                .into_response()
        }
    }
}

// ----------------------------- API handlers -----------------------------

async fn list_channels(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    resp(
        sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY name")
            .fetch_all(&st.db)
            .await,
    )
}

async fn create_channel(
    State(st): State<Arc<AppState>>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    let enabled = p.enabled.unwrap_or(true) as i64;
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let r = sqlx::query_as::<_, Channel>(
        "INSERT INTO channels(name,enabled,timezone) VALUES(?,?,?) RETURNING *",
    )
    .bind(p.name)
    .bind(enabled)
    .bind(tz)
    .fetch_one(&st.db)
    .await;
    resp(r)
}

async fn update_channel(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    let enabled = p.enabled.map(|b| b as i64);
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let r = sqlx::query_as::<_, Channel>(
        "UPDATE channels SET name=COALESCE(?,name), enabled=COALESCE(?,enabled), timezone=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? RETURNING *",
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
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let r = sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(id)
        .execute(&st.db)
        .await
        .map(|_| ());
    resp(r)
}

async fn list_rules(
    State(st): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id=? ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await;
    resp(rows)
}

async fn create_rule(
    State(st): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
    Json(mut p): Json<UpsertRule>,
) -> impl IntoResponse {
    // space priorities by 10; append if negative
    let maxp: Option<(i64,)> = sqlx::query_as("SELECT MAX(priority) FROM rules WHERE channel_id=?")
        .bind(channel_id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    let nextp = maxp.map(|t| t.0 + 10).unwrap_or(0);
    if p.priority < 0 {
        p.priority = nextp;
    }
    let r = sqlx::query_as::<_, Rule>("INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json) VALUES(?,?,?,?,?,?,?) RETURNING *")
        .bind(channel_id)
        .bind(p.name)
        .bind(p.priority)
        .bind(p.enabled.unwrap_or(true) as i64)
        .bind(p.match_json.to_string())
        .bind(p.action)
        .bind(p.params_json.to_string())
        .fetch_one(&st.db)
        .await;
    resp(r)
}

async fn update_rule(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertRule>,
) -> impl IntoResponse {
    let r = sqlx::query_as::<_, Rule>(
        "UPDATE rules SET name=?, priority=?, enabled=?, match_json=?, action=?, params_json=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? RETURNING *",
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
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let r = sqlx::query("DELETE FROM rules WHERE id=?")
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

async fn export_backup(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let channels = match sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY name")
        .fetch_all(&st.db)
        .await
    {
        Ok(v) => v,
        Err(e) => return err(e),
    };

    let mut out: Vec<ExportedChannel> = Vec::with_capacity(channels.len());
    for ch in channels {
        let rules = match sqlx::query_as::<_, Rule>(
            "SELECT * FROM rules WHERE channel_id=? ORDER BY priority",
        )
        .bind(ch.id)
        .fetch_all(&st.db)
        .await
        {
            Ok(v) => v,
            Err(e) => return err(e),
        };

        let exported_rules = rules
            .into_iter()
            .map(|r| ExportedRule {
                name: r.name,
                priority: r.priority,
                enabled: r.enabled != 0,
                match_json: serde_json::from_str(&r.match_json)
                    .unwrap_or(serde_json::json!({})),
                action: r.action,
                params_json: serde_json::from_str(&r.params_json)
                    .unwrap_or(serde_json::json!({})),
            })
            .collect();

        out.push(ExportedChannel {
            name: ch.name,
            enabled: ch.enabled != 0,
            timezone: ch.timezone,
            rules: exported_rules,
        });
    }

    let bundle = RulesBackup {
        version: 1,
        exported_at: Some(Utc::now().to_rfc3339()),
        channels: out,
    };

    Json(bundle).into_response()
}

async fn import_backup(
    State(st): State<Arc<AppState>>,
    Json(bundle): Json<RulesBackup>,
) -> impl IntoResponse {
    if bundle.version != 1 {
        return (StatusCode::BAD_REQUEST, "unsupported backup format").into_response();
    }

    let mut tx = match st.db.begin().await {
        Ok(t) => t,
        Err(e) => return err(e),
    };

    let mut channel_count = 0usize;
    let mut rule_count = 0usize;

    for channel in bundle.channels {
        let enabled = if channel.enabled { 1 } else { 0 };

        let existing = sqlx::query_as::<_, (i64,)>("SELECT id FROM channels WHERE name=?")
            .bind(&channel.name)
            .fetch_optional(&mut *tx)
            .await;

        let channel_id = match existing {
            Ok(Some((id,))) => {
                if let Err(e) = sqlx::query(
                    "UPDATE channels SET enabled=?, timezone=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
                )
                .bind(enabled)
                .bind(&channel.timezone)
                .bind(id)
                .execute(&mut *tx)
                .await
                {
                    let _ = tx.rollback().await;
                    return err(e);
                }
                id
            }
            Ok(None) => {
                match sqlx::query_as::<_, (i64,)>(
                    "INSERT INTO channels(name,enabled,timezone) VALUES(?,?,?) RETURNING id",
                )
                .bind(&channel.name)
                .bind(enabled)
                .bind(&channel.timezone)
                .fetch_one(&mut *tx)
                .await
                {
                    Ok((id,)) => id,
                    Err(e) => {
                        let _ = tx.rollback().await;
                        return err(e);
                    }
                }
            }
            Err(e) => {
                let _ = tx.rollback().await;
                return err(e);
            }
        };

        if let Err(e) = sqlx::query("DELETE FROM rules WHERE channel_id=?")
            .bind(channel_id)
            .execute(&mut *tx)
            .await
        {
            let _ = tx.rollback().await;
            return err(e);
        }

        for rule in channel.rules {
            let match_json = match serde_json::to_string(&rule.match_json) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.rollback().await;
                    return err(e);
                }
            };
            let params_json = match serde_json::to_string(&rule.params_json) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.rollback().await;
                    return err(e);
                }
            };

            let rule_enabled = if rule.enabled { 1 } else { 0 };
            if let Err(e) = sqlx::query(
                "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json) VALUES(?,?,?,?,?,?,?)",
            )
            .bind(channel_id)
            .bind(&rule.name)
            .bind(rule.priority)
            .bind(rule_enabled)
            .bind(match_json)
            .bind(&rule.action)
            .bind(params_json)
            .execute(&mut *tx)
            .await
            {
                let _ = tx.rollback().await;
                return err(e);
            }

            rule_count += 1;
        }

        channel_count += 1;
    }

    if let Err(e) = tx.commit().await {
        return err(e);
    }

    Json(serde_json::json!({
        "channels": channel_count,
        "rules": rule_count,
    }))
    .into_response()
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
        match sqlx::query_as("SELECT id FROM channels WHERE name=? AND enabled=1")
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
            "SELECT * FROM rules WHERE channel_id=? AND enabled=1 ORDER BY priority",
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
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "unknown command",
            )
                .into_response()
        }
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

// ------------------------ DB seeding helper ------------------------

async fn seed_default_channel_and_rule(db: &Pool<Sqlite>) -> anyhow::Result<()> {
    // Check if any channels exist
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels")
        .fetch_one(db)
        .await?;

    if count == 0 {
        // Insert a default channel
        let default_channel: Channel = sqlx::query_as(
            "INSERT INTO channels(name,enabled,timezone) VALUES(?,?,?) RETURNING *",
        )
        .bind("default")
        .bind(1_i64)
        .bind("UTC")
        .fetch_one(db)
        .await?;

        // Insert a default noop rule for that channel
        sqlx::query(
            "INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json) \
             VALUES(?,?,?,?,?,?,?)",
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
