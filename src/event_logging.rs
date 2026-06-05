// src/event_logging.rs
use crate::esam::decode_scte35_details;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::{debug, info, instrument};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EsamEvent {
    pub id: i64,
    pub timestamp: String,
    pub channel_name: String,
    pub acquisition_signal_id: String,
    pub utc_point: String,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    
    // SCTE-35 details
    pub scte35_command: Option<String>,
    pub scte35_type_id: Option<String>,
    pub scte35_upid: Option<String>,
    pub scte35_b64: Option<String>,

    // Rule matching
    pub matched_rule_id: Option<i64>,
    pub matched_rule_name: Option<String>,
    pub action: String,
    
    // Performance metrics
    pub request_size: Option<i32>,
    pub processing_time_ms: Option<i32>,
    pub response_status: i32,
    pub error_message: Option<String>,
    
    // Raw payloads (optional)
    pub raw_esam_request: Option<String>,
    pub raw_esam_response: Option<String>,

    // SESAME (SCTE 130-9) tier achieved (NULL = unauthenticated / Tier 0)
    pub sesame_tier: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EsamEventView {
    pub id: i64,
    pub timestamp: String,
    pub channel_name: String,
    pub acquisition_signal_id: String,
    pub utc_point: String,
    pub source_ip: Option<String>,
    pub scte35_command: Option<String>,
    pub scte35_type_id: Option<String>,
    pub scte35_upid: Option<String>,
    pub scte35_b64: Option<String>,
    pub matched_rule_id: Option<i64>,
    pub matched_rule_name: Option<String>,
    pub action: String,
    pub processing_time_ms: Option<i32>,
    pub response_status: i32,
    pub error_message: Option<String>,
    pub sesame_tier: Option<i32>,
    pub channel_timezone: Option<String>,
    pub rule_priority: Option<i64>,
}

#[derive(Clone)]
pub struct EventLogger {
    pub db: Pool<Sqlite>,
    pub store_raw_payloads: bool,
}

impl EventLogger {
    pub fn new(db: Pool<Sqlite>) -> Self {
        let store_raw_payloads = std::env::var("POIS_STORE_RAW_PAYLOADS")
            .map(|v| v == "true")
            .unwrap_or(false);
            
        Self {
            db,
            store_raw_payloads,
        }
    }

    #[instrument(skip(self, facts, request_body, response_body))]
    pub async fn log_esam_event(
        &self,
        channel_name: &str,
        facts: &serde_json::Value,
        matched_rule: Option<(&crate::models::Rule, &str)>,
        client_info: ClientInfo,
        metrics: ProcessingMetrics,
        request_body: Option<&str>,
        response_body: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let raw_request = if self.store_raw_payloads { request_body } else { None };
        let raw_response = if self.store_raw_payloads { response_body } else { None };
        
        let (matched_rule_id, matched_rule_name) = match matched_rule {
            Some((rule, _)) => (Some(rule.id), Some(rule.name.as_str())),
            None => (None, None),
        };
        
        let action = matched_rule.map(|(_, action)| action).unwrap_or("noop");
        // Decode SCTE-35 if present in BinaryData
        let (scte35_command, scte35_type_id, scte35_upid) = if let Some(binary_data) = facts
            .get("scte35_b64")
            
            .and_then(|v| v.as_str()) 
        {
            match decode_scte35_details(binary_data) {
                Ok(info) => {
                    debug!("Successfully decoded SCTE-35: command={:?}", info.command);
                    let upid_hex = info.segmentation_upid_with_type.as_ref().map(|(upid_type, data)| {
                        format!("0x{:02X}:{}", upid_type, data.iter().map(|b| format!("{:02X}", b)).collect::<String>())
                    });
                    (
                        info.command,
                        info.segmentation_type_id.map(|id| id.to_string()),
                        upid_hex
                    )
                },
                Err(e) => {
                    debug!("Failed to decode SCTE-35 in event logging: {}", e);
                    (None, None, None)
                }
            }
        } else {
            (None, None, None)
        };
        
        
        let event_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO esam_events (
                channel_name, acquisition_signal_id, utc_point, source_ip, user_agent,
                scte35_command, scte35_type_id, scte35_upid, scte35_b64,
                matched_rule_id, matched_rule_name, action,
                request_size, processing_time_ms, response_status, error_message,
                raw_esam_request, raw_esam_response, sesame_tier
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#
        )
        .bind(channel_name)
        .bind(facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(client_info.source_ip)
        .bind(client_info.user_agent)
        .bind(scte35_command.as_deref())
        .bind(scte35_type_id.as_deref())
        .bind(scte35_upid.as_deref())
        .bind(facts.get("scte35_b64").and_then(|v| v.as_str()))
        .bind(matched_rule_id)
        .bind(matched_rule_name)
        .bind(action)
        .bind(metrics.request_size)
        .bind(metrics.processing_time_ms)
        .bind(metrics.response_status)
        .bind(metrics.error_message.as_deref())
        .bind(raw_request)
        .bind(raw_response)
        .bind(client_info.sesame_tier)
        .fetch_one(&self.db)
        .await?;

        info!(
            event_id = event_id,
            channel = channel_name,
            action = action,
            rule_id = matched_rule_id,
            processing_ms = metrics.processing_time_ms,
            "ESAM event logged"
        );

        // ← NEW: Broadcast to WebSocket clients if registry provided

        Ok(event_id)
    }

    pub async fn get_recent_events(
        &self,
        limit: i64,
        offset: i64,
        filters: Option<EventFilters>,
    ) -> Result<Vec<EsamEventView>, sqlx::Error> {
        // Build the query dynamically; every user value is bound (no injection).
        let mut qb: sqlx::QueryBuilder<Sqlite> =
            sqlx::QueryBuilder::new("SELECT * FROM esam_events_view WHERE 1=1");

        if let Some(f) = filters {
            if let Some(channel) = f.channel_name {
                qb.push(" AND channel_name = ").push_bind(channel);
            }
            if let Some(action) = f.action {
                qb.push(" AND action = ").push_bind(action);
            }
            if let Some(since) = f.since {
                qb.push(" AND timestamp >= ").push_bind(since);
            }
            if let Some(search) = f.search {
                let term = search.trim();
                if !term.is_empty() {
                    let like = format!("%{}%", term);
                    // The UPID column stores hex ("0xTT:HHHH…"). Also match the
                    // ASCII→hex of the term so typing an ASCII UPID (e.g. ABCD1234)
                    // matches an ASCII-type UPID stored as its hex bytes.
                    let hex: String = term.bytes().map(|b| format!("{:02X}", b)).collect();
                    let hex_like = format!("%{}%", hex);
                    qb.push(" AND (acquisition_signal_id LIKE ")
                        .push_bind(like.clone())
                        .push(" OR source_ip LIKE ")
                        .push_bind(like.clone())
                        .push(" OR scte35_command LIKE ")
                        .push_bind(like.clone())
                        .push(" OR scte35_upid LIKE ")
                        .push_bind(like)
                        .push(" OR scte35_upid LIKE ")
                        .push_bind(hex_like)
                        .push(")");
                }
            }
            // RBAC: restrict to events whose channel the caller may read. Resolves
            // channel_name -> channels (names are globally unique). None = super.
            if let Some((uid, member_of)) = f.event_scope {
                qb.push(
                    " AND channel_name IN (SELECT name FROM channels WHERE deleted_at IS NULL \
                     AND (owner_user_id = ",
                )
                .push_bind(uid)
                .push(" OR is_global = 1");
                if !member_of.is_empty() {
                    qb.push(
                        " OR EXISTS(SELECT 1 FROM channel_groups cg WHERE cg.channel_id = channels.id AND cg.group_id IN (",
                    );
                    let mut sep = qb.separated(", ");
                    for g in &member_of {
                        sep.push_bind(*g);
                    }
                    qb.push("))");
                }
                qb.push("))");
            }
        }

        qb.push(" ORDER BY timestamp DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        qb.build_query_as::<EsamEventView>().fetch_all(&self.db).await
    }

    pub async fn get_event_stats(&self) -> Result<EventStats, sqlx::Error> {
        let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM esam_events")
            .fetch_one(&self.db)
            .await?;

        let last_24h_events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM esam_events WHERE timestamp >= datetime('now', '-1 day')"
        )
        .fetch_one(&self.db)
        .await?;

        let action_stats: Vec<(String, i64)> = sqlx::query_as(
            "SELECT action, COUNT(*) as count FROM esam_events 
             WHERE timestamp >= datetime('now', '-1 day') 
             GROUP BY action ORDER BY count DESC"
        )
        .fetch_all(&self.db)
        .await?;

        let avg_processing_time: Option<f64> = sqlx::query_scalar(
            "SELECT AVG(processing_time_ms) FROM esam_events 
             WHERE timestamp >= datetime('now', '-1 day') 
             AND processing_time_ms IS NOT NULL"
        )
        .fetch_one(&self.db)
        .await?;

        Ok(EventStats {
            total_events,
            last_24h_events,
            action_counts: action_stats.into_iter().collect(),
            avg_processing_time_ms: avg_processing_time,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    /// SESAME (SCTE 130-9) tier achieved on this request: Some(1|2|3) when
    /// authenticated, None for unauthenticated / Tier-0 passthrough.
    pub sesame_tier: Option<i32>,
}

impl ClientInfo {
    #[allow(dead_code)]
    pub fn from_headers_and_addr(headers: &HeaderMap, addr: Option<SocketAddr>) -> Self {
        let source_ip = headers
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .or_else(|| headers
                .get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
            )
            .or_else(|| addr.map(|a| a.ip().to_string()));

        let user_agent = headers
            .get("user-agent")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        Self {
            source_ip,
            user_agent,
            sesame_tier: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessingMetrics {
    pub request_size: Option<i32>,
    pub processing_time_ms: Option<i32>,
    pub response_status: i32,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EventFilters {
    pub channel_name: Option<String>,
    pub action: Option<String>,
    pub since: Option<String>,
    /// Free-text search across acquisition signal ID, source IP, SCTE-35 command,
    /// and UPID (hex form, plus an ASCII→hex match so an ASCII UPID also matches).
    pub search: Option<String>,
    /// RBAC scope: `Some((uid, member_of))` limits events to channels the caller
    /// may read; `None` = super-admin (no restriction).
    pub event_scope: Option<(i64, Vec<i64>)>,
}

#[derive(Debug, Serialize)]
pub struct EventStats {
    pub total_events: i64,
    pub last_24h_events: i64,
    pub action_counts: HashMap<String, i64>,
    pub avg_processing_time_ms: Option<f64>,
}