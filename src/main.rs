// src/main.rs
// Version: 3.10.2
// Last Modified: 2026-03-12
// Changes:
//   - Issue 1: Channel routing now uses acquisitionPointIdentity from XML body as fallback
//   - Issue 2: acquisitionPointIdentity echoed from inbound request (no longer hardcoded)
//   - Issue 3: UTCPoint now echoes inbound value (falls back to now+4s only if absent)
//   - Issue 4: ESAM responses now include <?xml?> declaration
//   - Issue 5: warn! logged when replace action has no scte35_b64 in rule params
//   - CRITICAL FIX: noop action now passes through original SCTE-35 BinaryData payload
//   - Fixed both matched-rule noop and fallback noop paths
//   - scte35_b64 from request facts is injected into params before build_notification
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
mod sesame_axum; // SESAME (SCTE 130-9) Axum adapter
mod template_library; // Template library + projects
mod rbac; // Groups + RBAC (identity resolution, group/membership management)

use axum::{
    body::{Body, Bytes},
    extract::{ConnectInfo, Extension, OriginalUri, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use crate::sesame_axum::SesameRuntime;
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
use crate::esam::{build_notification, esam_verb, extract_facts};
use crate::models::{
    Channel, DryRunRequest, DryRunResult, ReorderRules, Rule, UpsertChannel, UpsertRule,
};
use crate::rules::rule_matches;

#[derive(Clone)]
struct AppState {
    db: Pool<Sqlite>,
    #[allow(dead_code)]
    admin_token: String,
    event_logger: EventLogger,
    sesame: Arc<SesameRuntime>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --version / -V: print the build version and exit before any setup runs.
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    // Initialize SESAME (SCTE 130-9) runtime from environment. When no SESAME
    // env is set this is a transparent passthrough (legacy behavior).
    let sesame = Arc::new(SesameRuntime::from_env());
    if sesame.is_enabled() {
        info!(
            "SESAME enabled (default min tier {:?}, replay window {}s, response signing {})",
            sesame.default_min_tier,
            sesame.cfg.replay_window_secs,
            if sesame.response_key_id.is_some() { "on" } else { "off" },
        );
    } else {
        info!("SESAME inactive (no POIS_SESAME_* config); ESAM path unauthenticated");
    }

    let state = Arc::new(AppState {
        db,
        admin_token,
        event_logger,
        sesame,
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
        // Template library + projects
        .route("/api/projects", get(template_library::list_projects).post(template_library::create_project))
        .route("/api/projects/{id}", get(template_library::get_project).put(template_library::update_project).delete(template_library::delete_project))
        .route("/api/projects/{id}/apply", post(template_library::apply_project))
        .route("/api/templates", get(template_library::list_templates))
        .route("/api/templates/{id}", get(template_library::get_template).put(template_library::update_template).delete(template_library::delete_template))
        .route("/api/templates/from-rule/{rule_id}", post(template_library::save_rule_template))
        .route("/api/templates/from-channel/{channel_id}", post(template_library::save_channel_template))
        .route("/api/templates/{id}/apply", post(template_library::apply_template))
        // Groups + RBAC (Phase 1)
        .route("/api/me/groups", get(rbac::my_groups))
        .route("/api/groups", get(rbac::list_groups).post(rbac::create_group))
        .route("/api/groups/{id}", get(rbac::get_group).put(rbac::update_group).delete(rbac::delete_group))
        .route("/api/groups/{id}/members", get(rbac::list_members).post(rbac::add_member))
        .route("/api/groups/{id}/members/{user_id}", delete(rbac::remove_member))
        .with_state(state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            require_jwt_auth,
        ));

    // Main app - merge all routers (no with_state at this level!)
    let app = Router::new()
        .route("/esam/channel/{channel}", post(handle_esam_with_path))
        .route("/esam/channel={channel}", post(handle_esam_with_path))
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

#[allow(dead_code)]
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

#[derive(serde::Deserialize, Default)]
struct EsamQuery {
    channel: Option<String>,
}

async fn handle_esam(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<EsamQuery>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    handle_esam_impl(st, addr, uri, headers, body, q.channel).await
}

async fn handle_esam_with_path(
    State(st): State<Arc<AppState>>,
    Path(channel_name): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    handle_esam_impl(st, addr, uri, headers, body, Some(channel_name)).await
}

async fn handle_esam_impl(
    st: Arc<AppState>,
    addr: SocketAddr,
    uri: axum::http::Uri,
    headers: HeaderMap,
    raw_body: Bytes,
    path_channel: Option<String>,
) -> Response {
    let start = Instant::now();

    let mut client_info = ClientInfo {
        source_ip: Some(addr.ip().to_string()),
        user_agent: Some(headers
            .get("user-agent")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string()),
        sesame_tier: None,
    };

    // ---- SESAME (SCTE 130-9) inbound verification ----
    // Verify/authorize/decrypt BEFORE the ESAM XML is parsed. On failure,
    // short-circuit with the Appendix A.7 error and an audit log entry; never
    // hand an unverified body to the parser.
    let request_target = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| uri.path().to_string());

    let (body, sesame_ctx): (String, Option<sesame_axum::IncomingCtx>) = if st.sesame.is_enabled() {
        match sesame_axum::verify_incoming(
            &st.sesame,
            "POST",
            &request_target,
            path_channel.as_deref(),
            &headers,
            &raw_body,
        ) {
            Ok(incoming) => (
                String::from_utf8_lossy(&incoming.body).to_string(),
                incoming.ctx,
            ),
            Err(rej) => {
                let duration = start.elapsed();
                let _ = st
                    .event_logger
                    .log_esam_event(
                        path_channel.as_deref().unwrap_or("unknown"),
                        &serde_json::json!({"error": "sesame_rejected", "code": rej.error_code()}),
                        None,
                        client_info,
                        ProcessingMetrics {
                            request_size: Some(raw_body.len() as i32),
                            processing_time_ms: Some(duration.as_millis() as i32),
                            response_status: 0,
                            error_message: Some(format!("SESAME: {}", rej.error_code())),
                        },
                        None,
                        None,
                    )
                    .await;
                return rej.into_response();
            }
        }
    } else {
        (String::from_utf8_lossy(&raw_body).to_string(), None)
    };

    // Record the achieved SESAME tier (1/2/3, or None for unauthenticated) so it
    // is logged on every event below.
    client_info.sesame_tier = sesame_ctx
        .as_ref()
        .map(|c| c.achieved_tier.level() as i32);

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

    // Determine channel name: URL path takes priority, then acquisitionPointIdentity from XML body
    let channel_name = path_channel
        .or_else(|| {
            obj.get("acquisitionPointIdentity")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "default".into());

    let ch: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, timezone, sesame_min_tier FROM channels WHERE name=? AND enabled=1 AND deleted_at IS NULL",
    )
    .bind(&channel_name)
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten();

    let Some((channel_id, _tz, channel_min_tier)) = ch else {
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

    // ---- SESAME per-channel policy (§9.3), now that the channel is resolved ----
    // The global default tier was enforced during inbound verification; here we
    // additionally enforce any higher per-channel minimum, and confirm a Tier-2
    // scope matches the channel the request actually targets.
    if st.sesame.is_enabled() {
        let achieved = sesame_ctx
            .as_ref()
            .map(|c| c.achieved_tier)
            .unwrap_or(sesame_axum::Tier::Zero);
        let required = sesame_axum::Tier::from_u8(channel_min_tier.clamp(0, 3) as u8);
        if required.level() > achieved.level() {
            let key_id = sesame_ctx.as_ref().map(|c| c.key_id.clone());
            return sesame_axum::reject_insufficient_tier(key_id, required, achieved).into_response();
        }
        if let Some(ctx) = sesame_ctx.as_ref() {
            if let Some(scope_ch) = ctx.scope_channel.as_deref() {
                if scope_ch != channel_name {
                    return sesame_axum::reject_scope_mismatch(
                        Some(ctx.key_id.clone()),
                        scope_ch,
                        &channel_name,
                    )
                    .into_response();
                }
            }
        }
    }

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
        let rule_params: serde_json::Value = serde_json::from_str(&r.params_json).unwrap_or_default();
        let orig_b64 = facts.get("scte35_b64").and_then(|v| v.as_str());

        // Condition the outbound SCTE-35 for the friendly action (build / passthrough
        // / in-place edit of the incoming cue). The standard ESAM verb is derived in
        // build_notification; the authored params ride the <pois:Decision> element.
        let final_params = apply_action(&r.action, rule_params.clone(), orig_b64);

        if esam_verb(&r.action) == "replace" && final_params.get("scte35_b64").is_none() {
            tracing::warn!(
                "handle_esam: replace-class action '{}' on channel '{}' rule '{}' produced no scte35_b64 — BinaryData will be absent (no incoming cue to condition, or unparseable)",
                r.action, channel_name, r.name
            );
        }

        // Decision metadata = authored params minus the (possibly large) raw payload.
        let mut decision = rule_params.clone();
        if let Some(obj) = decision.as_object_mut() {
            obj.remove("scte35_b64");
        }

        let acq_id = facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or("");
        let utc_point = facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or("");
        let acq_point = facts.get("acquisitionPointIdentity").and_then(|v| v.as_str()).unwrap_or("");
        let resp_xml = build_notification(acq_id, utc_point, acq_point, &r.action, &final_params, Some(&decision));

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

        // Sign (and, if the request was Tier 3, encrypt) the outbound response —
        // the primary SESAME protection against a forged POIS decision.
        sesame_axum::build_esam_response(&st.sesame, sesame_ctx.as_ref(), acq_id, &resp_xml)
    } else {
        let duration = start.elapsed();
        
        let acq_id = facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or("");
        let utc_point = facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or("");
        let acq_point = facts.get("acquisitionPointIdentity").and_then(|v| v.as_str()).unwrap_or("");

        // Pass through original SCTE-35 payload on fallback noop
        let noop_params = match facts.get("scte35_b64").and_then(|v| v.as_str()) {
            Some(b64) => serde_json::json!({ "scte35_b64": b64 }),
            None => serde_json::json!({}),
        };
        let resp_xml = build_notification(acq_id, utc_point, acq_point, "noop", &noop_params, None);
        
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

        // Sign (and, if the request was Tier 3, encrypt) the outbound response —
        // the primary SESAME protection against a forged POIS decision.
        sesame_axum::build_esam_response(&st.sesame, sesame_ctx.as_ref(), acq_id, &resp_xml)
    }
}

// -------------------- Channels with ownership --------------------

async fn list_channels(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let mut qb: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT * FROM channels WHERE deleted_at IS NULL");
    rbac::push_read_predicate(&mut qb, &eff, "channels", "channel_groups", "channel_id");
    qb.push(" ORDER BY name");
    let channels = qb.build_query_as::<Channel>().fetch_all(&st.db).await;
    resp(channels)
}

async fn create_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    let enabled = p.enabled.unwrap_or(true) as i64;
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let is_global = (eff.super_admin && p.is_global.unwrap_or(false)) as i64;

    // Resolve the groups to publish the new channel to.
    let mut groups: Vec<i64> = p.group_ids.unwrap_or_default();
    if !eff.super_admin {
        if groups.is_empty() {
            match eff.member_of.len() {
                1 => groups = eff.member_of.clone(),
                0 => return (StatusCode::FORBIDDEN, "You belong to no group").into_response(),
                _ => return (StatusCode::BAD_REQUEST, "group_ids required (you belong to multiple groups)").into_response(),
            }
        }
        if !groups.iter().all(|g| eff.member_of.contains(g)) {
            return (StatusCode::FORBIDDEN, "Cannot publish to a group you don't belong to").into_response();
        }
    }

    let r = sqlx::query_as::<_, Channel>(
        "INSERT INTO channels(name,enabled,timezone,owner_user_id,is_global) VALUES(?,?,?,?,?) RETURNING *",
    )
    .bind(p.name)
    .bind(enabled)
    .bind(tz)
    .bind(eff.uid)
    .bind(is_global)
    .fetch_one(&st.db)
    .await;
    match r {
        Ok(ch) => {
            rbac::link_groups(&st.db, "channel_groups", "channel_id", ch.id, &groups).await;
            Json(ch).into_response()
        }
        Err(e) => err(e),
    }
}

async fn update_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
    Json(p): Json<UpsertChannel>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to modify this channel").into_response();
    }

    let enabled = p.enabled.map(|b| b as i64);
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    // Only super-admins can toggle org-wide visibility.
    let is_global = if eff.super_admin { p.is_global.map(|b| b as i64) } else { None };
    let r = sqlx::query_as::<_, Channel>(
        "UPDATE channels
         SET name=COALESCE(?,name), enabled=COALESCE(?,enabled), timezone=?,
             is_global=COALESCE(?,is_global),
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id=? AND deleted_at IS NULL
         RETURNING *",
    )
    .bind(Some(p.name))
    .bind(enabled)
    .bind(tz)
    .bind(is_global)
    .bind(id)
    .fetch_one(&st.db)
    .await;

    // Re-publish to the supplied groups (super-admin any; others only own groups).
    if let Some(gids) = p.group_ids {
        if eff.super_admin || gids.iter().all(|g| eff.member_of.contains(g)) {
            let _ = sqlx::query("DELETE FROM channel_groups WHERE channel_id = ?")
                .bind(id)
                .execute(&st.db)
                .await;
            rbac::link_groups(&st.db, "channel_groups", "channel_id", id, &gids).await;
        }
    }
    resp(r)
}

async fn delete_channel(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let eff = rbac::effective(&st.db, &claims).await;
    if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to delete this channel").into_response();
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

// -------------------- Rules with ownership --------------------

async fn list_rules(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Path(channel_id): Path<i64>,
) -> impl IntoResponse {
    // Rules inherit the parent channel's visibility.
    let eff = rbac::effective(&st.db, &claims).await;
    if !rbac::can_read(&st.db, &eff, "channels", "channel_groups", "channel_id", channel_id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to view this channel").into_response();
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
    // Managing rules requires write access to the parent channel.
    let eff = rbac::effective(&st.db, &claims).await;
    if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", channel_id).await {
        return (StatusCode::FORBIDDEN, "Not allowed to modify this channel").into_response();
    }
    let owner_id: i64 = eff.uid;

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
    let eff = rbac::effective(&st.db, &claims).await;
    match rule_parent_channel(&st.db, id).await {
        None => return (StatusCode::NOT_FOUND, "Rule not found").into_response(),
        Some(cid) => {
            if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", cid).await {
                return (StatusCode::FORBIDDEN, "Not allowed to modify this rule").into_response();
            }
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
    let eff = rbac::effective(&st.db, &claims).await;
    match rule_parent_channel(&st.db, id).await {
        None => return (StatusCode::NOT_FOUND, "Rule not found").into_response(),
        Some(cid) => {
            if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", cid).await {
                return (StatusCode::FORBIDDEN, "Not allowed to delete this rule").into_response();
            }
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

/// The parent channel id of a (non-deleted) rule, if it exists.
async fn rule_parent_channel(db: &Pool<Sqlite>, rule_id: i64) -> Option<i64> {
    sqlx::query_as::<_, (i64,)>("SELECT channel_id FROM rules WHERE id = ? AND deleted_at IS NULL")
        .bind(rule_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|(c,)| c)
}

async fn reorder_rules(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Json(p): Json<ReorderRules>,
) -> impl IntoResponse {
    // All reordered rules belong to one channel; gate on its write access.
    if let Some(&first) = p.ordered_ids.first() {
        let eff = rbac::effective(&st.db, &claims).await;
        match rule_parent_channel(&st.db, first).await {
            None => return (StatusCode::NOT_FOUND, "Rule not found").into_response(),
            Some(cid) => {
                if !rbac::can_write(&st.db, &eff, "channels", "channel_groups", "channel_id", cid).await {
                    return (StatusCode::FORBIDDEN, "Not allowed to reorder these rules").into_response();
                }
            }
        }
    }
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

/// Parse a hex byte like "0x34" / "34" into a u8.
fn parse_hex_u8(s: &str) -> Option<u8> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u8::from_str_radix(s, 16).ok()
}

/// Set `scte35_b64` on params from an in-place edit, falling back to the original
/// incoming payload when the edit couldn't be applied (e.g. no segmentation
/// descriptor / unparseable) so a conditioning action never drops the signal.
fn set_payload(p: &mut serde_json::Value, edited: Option<String>, orig_b64: Option<&str>) {
    match edited {
        Some(b) => p["scte35_b64"] = serde_json::json!(b),
        None => {
            if let Some(o) = orig_b64 {
                p["scte35_b64"] = serde_json::json!(o);
            }
        }
    }
}

/// Resolve a matched rule's friendly action into conditioned params carrying the
/// outbound `scte35_b64`. `build`/`replace` produce a fresh or rewritten cue;
/// blackout/regionalize/shorten/extend/fill edit the *incoming* cue in place;
/// noop/tracking-only/slate pass the original through; delete emits no payload.
fn apply_action(
    action: &str,
    params: serde_json::Value,
    orig_b64: Option<&str>,
) -> serde_json::Value {
    let mut p = maybe_build_scte35(params);
    match action.to_ascii_lowercase().as_str() {
        "noop" | "tracking-only" | "slate" => {
            if let Some(o) = orig_b64 {
                p["scte35_b64"] = serde_json::json!(o);
            }
        }
        "replace" => {
            // Back-compat: a replace rule may carry an in-place UPID rewrite.
            if let Some(rw) = p.get("rewrite_upid").cloned() {
                let nt = rw.get("upid_type").and_then(|v| v.as_str()).and_then(parse_hex_u8);
                let nv = rw.get("upid").and_then(|v| v.as_str()).unwrap_or("");
                let edited = orig_b64.and_then(|o| tools_api::rewrite_upid_b64(o, nt, nv));
                set_payload(&mut p, edited, orig_b64);
            }
            // else: built/literal payload already set by maybe_build_scte35 / params.
        }
        "regionalize" => {
            let nt = p.get("upid_type").and_then(|v| v.as_str()).and_then(parse_hex_u8);
            let nv = p.get("upid").and_then(|v| v.as_str()).unwrap_or("");
            let edited = orig_b64.and_then(|o| tools_api::rewrite_upid_b64(o, nt, nv));
            set_payload(&mut p, edited, orig_b64);
        }
        "blackout" => {
            let r = p.get("restrictions").cloned().unwrap_or_else(|| serde_json::json!({}));
            let web = r.get("web_delivery_allowed").and_then(|v| v.as_bool()).unwrap_or(false);
            let nrb = r.get("no_regional_blackout").and_then(|v| v.as_bool()).unwrap_or(false);
            let arc = r.get("archive_allowed").and_then(|v| v.as_bool()).unwrap_or(true);
            let dev = r.get("device_restrictions").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
            let edited = orig_b64.and_then(|o| tools_api::rewrite_delivery_flags_b64(o, web, nrb, arc, dev));
            set_payload(&mut p, edited, orig_b64);
        }
        "shorten" | "fill" => {
            // Absolute: SET the break to the given duration ("shorten/fill to Ns").
            let secs = p
                .get("to_duration_s")
                .or_else(|| p.get("duration_s"))
                .and_then(|v| v.as_f64());
            let edited = match (secs, orig_b64) {
                (Some(s), Some(o)) => tools_api::rewrite_break_duration_b64(o, (s * 90000.0) as u64),
                _ => None,
            };
            set_payload(&mut p, edited, orig_b64);
        }
        "extend" => {
            // Additive: ADD duration_s seconds to the incoming break ("extend by Ns").
            let secs = p
                .get("duration_s")
                .or_else(|| p.get("to_duration_s"))
                .and_then(|v| v.as_f64());
            let edited = match (secs, orig_b64) {
                (Some(s), Some(o)) => {
                    tools_api::adjust_break_duration_b64(o, (s * 90000.0) as i64)
                }
                _ => None,
            };
            set_payload(&mut p, edited, orig_b64);
        }
        "delete" => {}
        _ => {
            // Unknown action → safe pass-through (mirrors esam_verb's default).
            if let Some(o) = orig_b64 {
                p["scte35_b64"] = serde_json::json!(o);
            }
        }
    }
    p
}

fn maybe_build_scte35(mut params: serde_json::Value) -> serde_json::Value {
    if let Some(build) = params.get("build").cloned() {
        if let Some(cmd) = build.get("command").and_then(|v| v.as_str()) {
            // Optional segmentation descriptor params. When any are present we
            // route to the advanced builders so the replacement carries a custom
            // segmentation type id / UPID; otherwise the basic builders run.
            let seg_type = build
                .get("segmentation_type_id")
                .and_then(|v| v.as_str())
                .and_then(parse_hex_u8);
            let upid_type = build
                .get("upid_type")
                .and_then(|v| v.as_str())
                .and_then(parse_hex_u8);
            let upid_val = build.get("upid").and_then(|v| v.as_str());
            let advanced = seg_type.is_some() || upid_type.is_some() || upid_val.is_some();

            let out = match cmd {
                "time_signal_immediate" | "time_signal" => {
                    if advanced {
                        scte35::build_time_signal_advanced_b64(seg_type, upid_type, upid_val)
                    } else {
                        scte35::build_time_signal_immediate_b64()
                    }
                }
                "splice_insert_out" => {
                    let dur = build
                        .get("duration_s")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    if advanced {
                        scte35::build_splice_insert_out_advanced_b64(
                            dur, seg_type, upid_type, upid_val,
                        )
                    } else {
                        scte35::build_splice_insert_out_b64(dur)
                    }
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

    // RBAC: scope events to channels the caller may read (None = super-admin).
    let eff = rbac::effective(&st.db, &claims).await;
    let filters = EventFilters {
        channel_name: params.get("channel").cloned(),
        action: params.get("action").cloned(),
        since: params.get("since").cloned(),
        search: params
            .get("search")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        event_scope: rbac::event_scope(&eff),
    };

    match st.event_logger.get_recent_events(limit, offset, Some(filters)).await {
        Ok(events) => Json(events).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_event_stats(
    State(st): State<Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
) -> impl IntoResponse {
    // Stats are scoped to the channels the caller may read (None = super-admin).
    let eff = rbac::effective(&st.db, &claims).await;
    match st.event_logger.get_event_stats(rbac::event_scope(&eff)).await {
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
            // RBAC: must be able to read the event's channel (resolved by name).
            let eff = rbac::effective(&st.db, &claims).await;
            if !eff.super_admin {
                let chan: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM channels WHERE name = ? AND deleted_at IS NULL"
                )
                .bind(&ev.channel_name)
                .fetch_optional(&st.db)
                .await
                .ok()
                .flatten();
                let allowed = match chan {
                    Some((cid,)) => {
                        rbac::can_read(&st.db, &eff, "channels", "channel_groups", "channel_id", cid).await
                    }
                    None => false, // channel gone -> super-admin only
                };
                if !allowed {
                    return (StatusCode::FORBIDDEN, "Not allowed to view this event").into_response();
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
    // Hard-delete any soft-deleted 'default' channel so the unique constraint
    // doesn't block re-creation after an upgrade or accidental deletion
    sqlx::query("DELETE FROM channels WHERE name='default' AND deleted_at IS NOT NULL")
        .execute(db)
        .await?;

    // Check if a live 'default' channel exists
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM channels WHERE name='default' AND deleted_at IS NULL"
    )
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

        // Insert a default noop rule only if no rules exist for this channel
        let (rule_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM rules WHERE channel_id=? AND deleted_at IS NULL"
        )
        .bind(default_channel.id)
        .fetch_one(db)
        .await?;

        if rule_count == 0 {
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

        info!("Seeded default channel and noop rule");
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

#[cfg(test)]
mod build_params_tests {
    use super::*;
    use serde_json::json;

    fn b64(params: serde_json::Value) -> String {
        maybe_build_scte35(params)["scte35_b64"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn basic_builds_unchanged() {
        // No segmentation params -> basic builders, matching the demo rules.
        assert_eq!(
            b64(json!({"build":{"command":"splice_insert_out","duration_s":30}})),
            scte35::build_splice_insert_out_b64(30)
        );
        assert_eq!(
            b64(json!({"build":{"command":"time_signal_immediate"}})),
            scte35::build_time_signal_immediate_b64()
        );
    }

    #[test]
    fn seg_upid_params_route_to_advanced_builder() {
        // The router must parse the hex fields and forward them verbatim to the
        // advanced builder (and the result must differ from the basic build).
        let got = b64(json!({"build":{
            "command":"splice_insert_out","duration_s":60,
            "segmentation_type_id":"0x34","upid_type":"0x0C","upid":"ABCD1234"
        }}));
        assert_eq!(
            got,
            scte35::build_splice_insert_out_advanced_b64(60, Some(0x34), Some(0x0C), Some("ABCD1234"))
        );
        assert_ne!(got, scte35::build_splice_insert_out_b64(60));
    }

    #[test]
    fn time_signal_alias_and_partial_params() {
        // "time_signal" is accepted as an alias; a lone seg type still goes advanced.
        assert_eq!(
            b64(json!({"build":{"command":"time_signal","segmentation_type_id":"0x10"}})),
            scte35::build_time_signal_advanced_b64(Some(0x10), None, None)
        );
    }

    // ---- richer action set: verb mapping, dispatch, decision metadata ----

    #[test]
    fn esam_verb_maps_friendly_actions() {
        assert_eq!(esam_verb("blackout"), "replace");
        assert_eq!(esam_verb("shorten"), "replace");
        assert_eq!(esam_verb("regionalize"), "replace");
        assert_eq!(esam_verb("tracking-only"), "noop");
        assert_eq!(esam_verb("slate"), "noop");
        assert_eq!(esam_verb("delete"), "delete");
        assert_eq!(esam_verb("totally-unknown"), "noop"); // safe default
    }

    #[test]
    fn notification_uses_standard_verb_and_emits_decision() {
        let params = json!({ "scte35_b64": "AAAA" });
        let decision = json!({ "restrictions": { "no_regional_blackout": false } });
        let xml = build_notification("sig", "2026-06-02T00:00:00Z", "ap", "blackout", &params, Some(&decision));
        // Standard verb drives the wire; the friendly action only appears in <pois:Decision>.
        assert!(xml.contains(r#"<ResponseSignal action="replace""#), "{xml}");
        assert!(xml.contains(r#"<pois:Decision action="blackout">"#), "{xml}");
        assert!(xml.contains("no_regional_blackout"), "{xml}");
        assert!(xml.contains(r#"<sig:BinaryData signalType="SCTE35">AAAA</sig:BinaryData>"#));
    }

    #[test]
    fn apply_action_blackout_conditions_incoming_cue() {
        let orig = scte35::build_time_signal_advanced_b64(Some(0x34), Some(0x0C), Some("X"));
        let params = json!({ "restrictions": { "web_delivery_allowed": false, "no_regional_blackout": false, "archive_allowed": true, "device_restrictions": 0 } });
        let out = apply_action("blackout", params, Some(&orig));
        assert_ne!(out["scte35_b64"].as_str().unwrap(), orig, "payload conditioned");
    }

    #[test]
    fn apply_action_passthrough_actions_keep_original() {
        for action in ["tracking-only", "slate", "noop", "totally-unknown"] {
            let out = apply_action(action, json!({}), Some("ORIG"));
            assert_eq!(out["scte35_b64"], json!("ORIG"), "action {action}");
        }
    }

    #[test]
    fn apply_action_delete_emits_no_payload() {
        let out = apply_action("delete", json!({}), Some("ORIG"));
        assert!(out.get("scte35_b64").is_none());
    }
}