mod models;
mod rules;
mod esam;
mod scte35; // NEW: builder module

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    body::Body,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put}, // include delete
    Json, Router,
};
use sqlx::{sqlite::{SqlitePoolOptions, SqliteConnectOptions}, Pool, Sqlite};
use std::str::FromStr;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

// bring model types into scope
use crate::models::{
    Channel, DryRunRequest, DryRunResult, ReorderRules, Rule, UpsertChannel, UpsertRule,
};
use crate::rules::rule_matches;
use crate::esam::{build_notification, extract_facts};

#[derive(Clone)]
struct AppState {
    db: Pool<Sqlite>,
    admin_token: String,
}



fn ensure_sqlite_parent(db_url: &str) -> std::io::Result<()> {
    if !db_url.starts_with("sqlite:") { return Ok(()); }
    if db_url.contains(":memory:") { return Ok(()); }

    let path_str = if let Some(rest) = db_url.strip_prefix("sqlite://") {
        rest
    } else if let Some(rest) = db_url.strip_prefix("sqlite:") {
        rest
    } else { return Ok(()); };

    if path_str.is_empty() { return Ok(()); }
    let p = std::path::Path::new(path_str);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("pois_esam_server=info".parse()?),
        )
        .init();

    let db_url = std::env::var("POIS_DB").unwrap_or_else(|_| "sqlite://pois.db".to_string());
    let admin_token = std::env::var("POIS_ADMIN_TOKEN").unwrap_or_else(|_| "dev-token".to_string());

    ensure_sqlite_parent(&db_url).ok();

    let conn_opts = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true);
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(conn_opts)
        .await?;
    sqlx::migrate!().run(&db).await?;

    let state = Arc::new(AppState { db, admin_token });

        let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/esam", post(handle_esam))
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(|| async { axum::response::Redirect::temporary("/static/login.html") }))
        .route("/tools.html", get(|| async { axum::response::Redirect::temporary("/static/tools.html") }))
        .nest("/api", {
            let api = api_router().with_state(state.clone())
                .route_layer(axum::middleware::from_fn_with_state(state.clone(), require_bearer));
            api
        })
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // ↓↓↓ replace the old hard-coded 8080 block with this ↓↓↓
    let port: u16 = std::env::var("POIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    info!("POIS listening on http://{addr}  (UI: /)");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/channels", get(list_channels).post(create_channel))
        .route("/channels/{id}", put(update_channel).delete(delete_channel))
        .route("/channels/{id}/rules", get(list_rules).post(create_rule))
        .route("/rules/{id}", put(update_rule).delete(delete_rule))
        .route("/rules/reorder", post(reorder_rules))
        .route("/dryrun", post(dryrun))
        // NEW: builder endpoint
        .route("/tools/scte35/build", post(build_scte35))
}

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

#[derive(serde::Deserialize)]
struct EsamQuery { channel: Option<String> }

async fn handle_esam(
    State(st): State<Arc<AppState>>,
    Query(q): Query<EsamQuery>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // Extract facts from ESAM request
    let facts = match extract_facts(&body) {
        Ok(v) => v,
        Err(e) => {
            error!("ESAM parse error: {e}");
            return (StatusCode::BAD_REQUEST, "invalid ESAM payload").into_response();
        }
    };
    let acq_id = facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or("no-id");
    let utc    = facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or("1970-01-01T00:00:00Z");

    // Resolve channel via query/header, fallback to "default"
    let channel_name = q.channel
        .or_else(|| headers.get("X-POIS-Channel").and_then(|v| v.to_str().ok()).map(|s| s.to_string()))
        .unwrap_or_else(|| "default".to_string());

    // Load rules
    let ch: Option<(i64,)> = match sqlx::query_as("SELECT id FROM channels WHERE name=? AND enabled=1")
        .bind(&channel_name)
        .fetch_optional(&st.db).await {
        Ok(v) => v,
        Err(e) => {
            error!("DB error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    let (action, mut params) = if let Some((channel_id,)) = ch {
        let rules = sqlx::query_as::<_, Rule>(
            "SELECT * FROM rules WHERE channel_id=? AND enabled=1 ORDER BY priority"
        ).bind(channel_id).fetch_all(&st.db).await.unwrap_or_default();

        let map = facts.as_object().cloned().unwrap_or_default();
        let mut chosen: Option<(String, serde_json::Value)> = None;

        for r in rules {
            let m: serde_json::Value = serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
            if rule_matches(&m, &map) {
                let p: serde_json::Value = serde_json::from_str(&r.params_json).unwrap_or(serde_json::json!({}));
                chosen = Some((r.action.clone(), p));
                break;
            }
        }
        chosen.unwrap_or(("noop".into(), serde_json::json!({})))
    } else {
        ("noop".into(), serde_json::json!({}))
    };

    // Auto-build SCTE-35 if params carry a "build" object
    params = maybe_build_scte35(params);

    // Build ESAM response XML
    let xml = build_notification(acq_id, utc, &action, &params);
    let mut out_headers = HeaderMap::new();
    out_headers.insert(axum::http::header::CONTENT_TYPE, "application/xml".parse().unwrap());
    (StatusCode::OK, out_headers, xml).into_response()
}

// ----------------------------- API handlers -----------------------------

async fn list_channels(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    resp(sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY name").fetch_all(&st.db).await)
}

async fn create_channel(State(st): State<Arc<AppState>>, Json(p): Json<UpsertChannel>) -> impl IntoResponse {
    let enabled = p.enabled.unwrap_or(true) as i64;
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let r = sqlx::query_as::<_, Channel>("INSERT INTO channels(name,enabled,timezone) VALUES(?,?,?) RETURNING *")
        .bind(p.name).bind(enabled).bind(tz).fetch_one(&st.db).await;
    resp(r)
}

async fn update_channel(State(st): State<Arc<AppState>>, Path(id): Path<i64>, Json(p): Json<UpsertChannel>) -> impl IntoResponse {
    let enabled = p.enabled.map(|b| b as i64);
    let tz = p.timezone.unwrap_or_else(|| "UTC".into());
    let r = sqlx::query_as::<_, Channel>(
        "UPDATE channels SET name=COALESCE(?,name), enabled=COALESCE(?,enabled), timezone=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? RETURNING *")
        .bind(Some(p.name)).bind(enabled).bind(tz).bind(id).fetch_one(&st.db).await;
    resp(r)
}

async fn delete_channel(State(st): State<Arc<AppState>>, Path(id): Path<i64>) -> impl IntoResponse {
    let r = sqlx::query("DELETE FROM channels WHERE id=?").bind(id).execute(&st.db).await.map(|_| ());
    resp(r)
}

async fn list_rules(State(st): State<Arc<AppState>>, Path(channel_id): Path<i64>) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, Rule>("SELECT * FROM rules WHERE channel_id=? ORDER BY priority")
        .bind(channel_id).fetch_all(&st.db).await;
    resp(rows)
}

async fn create_rule(State(st): State<Arc<AppState>>, Path(channel_id): Path<i64>, Json(mut p): Json<UpsertRule>) -> impl IntoResponse {
    // space priorities by 10; append if negative
    let maxp: Option<(i64,)> = sqlx::query_as("SELECT MAX(priority) FROM rules WHERE channel_id=?")
        .bind(channel_id).fetch_optional(&st.db).await.ok().flatten();
    let nextp = maxp.map(|t| t.0 + 10).unwrap_or(0);
    if p.priority < 0 { p.priority = nextp; }
    let r = sqlx::query_as::<_, Rule>("INSERT INTO rules(channel_id,name,priority,enabled,match_json,action,params_json) VALUES(?,?,?,?,?,?,?) RETURNING *")
        .bind(channel_id)
        .bind(p.name)
        .bind(p.priority)
        .bind(p.enabled.unwrap_or(true) as i64)
        .bind(p.match_json.to_string())
        .bind(p.action)
        .bind(p.params_json.to_string())
        .fetch_one(&st.db).await;
    resp(r)
}

async fn update_rule(State(st): State<Arc<AppState>>, Path(id): Path<i64>, Json(p): Json<UpsertRule>) -> impl IntoResponse {
    let r = sqlx::query_as::<_, Rule>(
        "UPDATE rules SET name=?, priority=?, enabled=?, match_json=?, action=?, params_json=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? RETURNING *")
        .bind(p.name)
        .bind(p.priority)
        .bind(p.enabled.unwrap_or(true) as i64)
        .bind(p.match_json.to_string())
        .bind(p.action)
        .bind(p.params_json.to_string())
        .bind(id)
        .fetch_one(&st.db).await;
    resp(r)
}

async fn delete_rule(State(st): State<Arc<AppState>>, Path(id): Path<i64>) -> impl IntoResponse {
    let r = sqlx::query("DELETE FROM rules WHERE id=?").bind(id).execute(&st.db).await.map(|_| ());
    resp(r)
}

async fn reorder_rules(State(st): State<Arc<AppState>>, Json(p): Json<ReorderRules>) -> impl IntoResponse {
    let mut tx = match st.db.begin().await {
        Ok(t) => t,
        Err(e) => return err(e),
    };
    let mut prio = 0i64;
    for id in p.ordered_ids {
        if let Err(e) = sqlx::query("UPDATE rules SET priority=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
            .bind(prio).bind(id).execute(&mut *tx).await {
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

async fn dryrun(State(st): State<Arc<AppState>>, Json(p): Json<DryRunRequest>) -> impl IntoResponse {
    let facts = match extract_facts(&p.esam_xml) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("parse error: {e}")).into_response(),
    };
    let ch: Option<(i64,)> = match sqlx::query_as("SELECT id FROM channels WHERE name=? AND enabled=1")
        .bind(&p.channel).fetch_optional(&st.db).await {
        Ok(v) => v, Err(e) => return err(e),
    };
    let Some((channel_id,)) = ch else {
        return (StatusCode::NOT_FOUND, "channel not found or disabled").into_response();
    };

    let rules = match sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id=? AND enabled=1 ORDER BY priority"
    ).bind(channel_id).fetch_all(&st.db).await {
        Ok(v) => v, Err(e) => return err(e),
    };

    let map = facts.as_object().cloned().unwrap_or_default();
    for r in rules {
        let m: serde_json::Value = serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
        if rule_matches(&m, &map) {
            // reflect any builder usage in params for clarity (not strictly required for dryrun)
            let params: serde_json::Value = serde_json::from_str(&r.params_json).unwrap_or(serde_json::json!({}));
            let _params = maybe_build_scte35(params);
            return Json(DryRunResult { matched_rule_id: Some(r.id), action: r.action.clone(), note: "first matching rule".into() }).into_response();
        }
    }
    Json(DryRunResult { matched_rule_id: None, action: "noop".into(), note: "no rules matched".into() }).into_response()
}

// ---------- Builder endpoint & helper ----------

#[derive(serde::Deserialize)]
struct BuildReq { command: String, duration_s: Option<u32> }

#[derive(serde::Serialize)]
struct BuildResp { scte35_b64: String }

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
                    let dur = build.get("duration_s").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
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