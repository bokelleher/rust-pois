// src/event_logging.rs
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::{error, info, instrument};

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
    pub matched_rule_id: Option<i64>,
    pub matched_rule_name: Option<String>,
    pub action: String,
    pub processing_time_ms: Option<i32>,
    pub response_status: i32,
    pub error_message: Option<String>,
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
        
        let event_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO esam_events (
                channel_name, acquisition_signal_id, utc_point, source_ip, user_agent,
                scte35_command, scte35_type_id, scte35_upid,
                matched_rule_id, matched_rule_name, action,
                request_size, processing_time_ms, response_status, error_message,
                raw_esam_request, raw_esam_response
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#
        )
        .bind(channel_name)
        .bind(facts.get("acquisitionSignalID").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(facts.get("utcPoint").and_then(|v| v.as_str()).unwrap_or(""))
        .bind(client_info.source_ip)
        .bind(client_info.user_agent)
        .bind(facts.get("scte35.command").and_then(|v| v.as_str()))
        .bind(facts.get("scte35.segmentation_type_id").and_then(|v| v.as_str()))
        .bind(facts.get("scte35.segmentation_upid").and_then(|v| v.as_str()))
        .bind(matched_rule_id)
        .bind(matched_rule_name)
        .bind(action)
        .bind(metrics.request_size)
        .bind(metrics.processing_time_ms)
        .bind(metrics.response_status)
        .bind(metrics.error_message.as_deref())
        .bind(raw_request)
        .bind(raw_response)
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

        Ok(event_id)
    }

    pub async fn get_recent_events(
        &self,
        limit: i64,
        offset: i64,
        filters: Option<EventFilters>,
    ) -> Result<Vec<EsamEventView>, sqlx::Error> {
        // Use separate queries for different filter combinations to avoid dynamic SQL
        match filters {
            Some(EventFilters { 
                channel_name: Some(channel), 
                action: Some(action), 
                since: Some(since) 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE channel_name = ? AND action = ? AND timestamp >= ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(channel)
                .bind(action)
                .bind(since)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: Some(channel), 
                action: None, 
                since: Some(since) 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE channel_name = ? AND timestamp >= ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(channel)
                .bind(since)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: Some(channel), 
                action: Some(action), 
                since: None 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE channel_name = ? AND action = ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(channel)
                .bind(action)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: Some(channel), 
                action: None, 
                since: None 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE channel_name = ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(channel)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: None, 
                action: Some(action), 
                since: Some(since) 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE action = ? AND timestamp >= ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(action)
                .bind(since)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: None, 
                action: Some(action), 
                since: None 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE action = ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(action)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            Some(EventFilters { 
                channel_name: None, 
                action: None, 
                since: Some(since) 
            }) => {
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view WHERE timestamp >= ? ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(since)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
            _ => {
                // No filters or all None
                sqlx::query_as::<_, EsamEventView>(
                    "SELECT * FROM esam_events_view ORDER BY timestamp DESC LIMIT ? OFFSET ?"
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.db)
                .await
            }
        }
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
}

impl ClientInfo {
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
}

#[derive(Debug, Serialize)]
pub struct EventStats {
    pub total_events: i64,
    pub last_24h_events: i64,
    pub action_counts: HashMap<String, i64>,
    pub avg_processing_time_ms: Option<f64>,
}