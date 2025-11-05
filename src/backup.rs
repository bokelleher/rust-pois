// src/backup.rs
//! Backup and Restore system for POIS channels and rules
//! 
//! Matches the actual POIS schema with:
//! - i64 IDs and enabled flags (SQLite integers)
//! - match_json/action/params_json fields (not condition/action)
//! - No description fields
//! - Arc<AppState> for handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::Arc;

use crate::models::{Channel, Rule};
use crate::AppState;

// ===== Backup/Restore Models =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RuleBackup {
    pub name: String,
    pub match_json: JsonValue,
    pub action: String,
    pub params_json: JsonValue,
    #[serde(default)]
    pub priority: i64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChannelBackup {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelFullBackup {
    pub channel: ChannelBackup,
    #[serde(default)]
    pub rules: Vec<RuleBackup>,
    #[serde(default)]
    pub backup_metadata: BackupMetadata,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BackupMetadata {
    pub version: String,
    pub created_at: String,
    pub backup_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_count: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    pub version: String,
    pub created_at: String,
    pub backup_type: String,
    #[serde(default)]
    pub full_channels: Vec<ChannelFullBackup>,
    #[serde(default)]
    pub channels: Vec<ChannelBackup>,
    #[serde(default)]
    pub rules: Vec<RuleBackup>,
    #[serde(default)]
    pub metadata: serde_json::Map<String, JsonValue>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RestoreOptions {
    #[serde(default = "default_true")]
    pub skip_existing: bool,
    #[serde(default)]
    pub update_existing: bool,
    #[serde(default = "default_true")]
    pub new_ids: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_names: Option<String>,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            skip_existing: true,
            update_existing: false,
            new_ids: true,
            prefix_names: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub success: bool,
    #[serde(default)]
    pub channels_created: u32,
    #[serde(default)]
    pub channels_updated: u32,
    #[serde(default)]
    pub channels_skipped: u32,
    #[serde(default)]
    pub rules_created: u32,
    #[serde(default)]
    pub rules_updated: u32,
    #[serde(default)]
    pub rules_skipped: u32,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Default for RestoreResult {
    fn default() -> Self {
        Self {
            success: true,
            channels_created: 0,
            channels_updated: 0,
            channels_skipped: 0,
            rules_created: 0,
            rules_updated: 0,
            rules_skipped: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ImportChannelRequest {
    #[serde(flatten)]
    pub channel: ChannelBackup,
    #[serde(default)]
    pub options: RestoreOptions,
}

#[derive(Debug, Deserialize)]
pub struct ImportChannelFullRequest {
    #[serde(flatten)]
    pub backup: ChannelFullBackup,
    #[serde(default)]
    pub options: RestoreOptions,
}

#[derive(Debug, Deserialize)]
pub struct ImportRuleRequest {
    #[serde(flatten)]
    pub rule: RuleBackup,
    #[serde(default)]
    pub options: RestoreOptions,
}

#[derive(Debug, Deserialize)]
pub struct ImportRulesRequest {
    pub rules: Vec<RuleBackup>,
    #[serde(default)]
    pub options: RestoreOptions,
}

#[derive(Debug, Deserialize)]
pub struct ImportFileRequest {
    #[serde(flatten)]
    pub backup: BackupFile,
    #[serde(default)]
    pub options: RestoreOptions,
}

// Helper functions
fn default_true() -> bool {
    true
}

fn default_timezone() -> String {
    "UTC".to_string()
}

// ===== Export Handlers =====

/// Export channel metadata only (no rules)
pub async fn export_channel_only(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
) -> Result<Json<ChannelBackup>, (StatusCode, String)> {
    let channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Channel not found".to_string()))?;

    Ok(Json(ChannelBackup {
        name: channel.name,
        enabled: channel.enabled != 0,
        timezone: channel.timezone,
    }))
}

/// Export full channel with all rules
pub async fn export_channel_full(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
) -> Result<Json<ChannelFullBackup>, (StatusCode, String)> {
    let channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Channel not found".to_string()))?;

    let rules = sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id = ? ORDER BY priority DESC, created_at ASC",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let rule_backups: Vec<RuleBackup> = rules
        .into_iter()
        .filter_map(|r| {
            let match_json: JsonValue = serde_json::from_str(&r.match_json).ok()?;
            let params_json: JsonValue = serde_json::from_str(&r.params_json).ok()?;
            Some(RuleBackup {
                name: r.name,
                match_json,
                action: r.action,
                params_json,
                priority: r.priority,
                enabled: r.enabled != 0,
            })
        })
        .collect();

    Ok(Json(ChannelFullBackup {
        channel: ChannelBackup {
            name: channel.name,
            enabled: channel.enabled != 0,
            timezone: channel.timezone,
        },
        rules: rule_backups.clone(),
        backup_metadata: BackupMetadata {
            version: "1.0".to_string(),
            created_at: Utc::now().to_rfc3339(),
            backup_type: "full".to_string(),
            channel_id: Some(channel_id),
            rule_count: Some(rule_backups.len()),
        },
    }))
}

/// Export a single rule
pub async fn export_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<i64>,
) -> Result<Json<RuleBackup>, (StatusCode, String)> {
    let rule = sqlx::query_as::<_, Rule>("SELECT * FROM rules WHERE id = ?")
        .bind(rule_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Rule not found".to_string()))?;

    let match_json: JsonValue = serde_json::from_str(&rule.match_json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid match_json: {}", e)))?;
    let params_json: JsonValue = serde_json::from_str(&rule.params_json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid params_json: {}", e)))?;

    Ok(Json(RuleBackup {
        name: rule.name,
        match_json,
        action: rule.action,
        params_json,
        priority: rule.priority,
        enabled: rule.enabled != 0,
    }))
}

/// Export multiple rules
pub async fn export_rules(
    State(state): State<Arc<AppState>>,
    Json(rule_ids): Json<Vec<i64>>,
) -> Result<Json<Vec<RuleBackup>>, (StatusCode, String)> {
    let mut rule_backups = Vec::new();
    let mut not_found = Vec::new();

    for rule_id in rule_ids {
        match sqlx::query_as::<_, Rule>("SELECT * FROM rules WHERE id = ?")
            .bind(rule_id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(rule)) => {
                if let (Ok(match_json), Ok(params_json)) = (
                    serde_json::from_str(&rule.match_json),
                    serde_json::from_str(&rule.params_json),
                ) {
                    rule_backups.push(RuleBackup {
                        name: rule.name,
                        match_json,
                        action: rule.action,
                        params_json,
                        priority: rule.priority,
                        enabled: rule.enabled != 0,
                    });
                }
            }
            Ok(None) => not_found.push(rule_id),
            Err(e) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
        }
    }

    if !not_found.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Rules not found: {:?}", not_found),
        ));
    }

    Ok(Json(rule_backups))
}

/// Export all channels with all rules (full system backup)
pub async fn export_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<BackupFile>, (StatusCode, String)> {
    let channels = sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut full_backups = Vec::new();
    let mut total_rules = 0;

    for channel in channels {
        let rules = sqlx::query_as::<_, Rule>(
            "SELECT * FROM rules WHERE channel_id = ? ORDER BY priority DESC, created_at ASC",
        )
        .bind(channel.id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let rule_backups: Vec<RuleBackup> = rules
            .into_iter()
            .filter_map(|r| {
                let match_json: JsonValue = serde_json::from_str(&r.match_json).ok()?;
                let params_json: JsonValue = serde_json::from_str(&r.params_json).ok()?;
                Some(RuleBackup {
                    name: r.name,
                    match_json,
                    action: r.action,
                    params_json,
                    priority: r.priority,
                    enabled: r.enabled != 0,
                })
            })
            .collect();

        total_rules += rule_backups.len();

        full_backups.push(ChannelFullBackup {
            channel: ChannelBackup {
                name: channel.name,
                enabled: channel.enabled != 0,
                timezone: channel.timezone,
            },
            rules: rule_backups,
            backup_metadata: BackupMetadata::default(),
        });
    }

    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "channel_count".to_string(),
        JsonValue::Number(full_backups.len().into()),
    );
    metadata.insert(
        "total_rules".to_string(),
        JsonValue::Number(total_rules.into()),
    );

    Ok(Json(BackupFile {
        version: "1.0".to_string(),
        created_at: Utc::now().to_rfc3339(),
        backup_type: "full".to_string(),
        full_channels: full_backups,
        channels: Vec::new(),
        rules: Vec::new(),
        metadata,
    }))
}

// ===== Import Handlers =====

/// Import a channel (metadata only)
pub async fn import_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportChannelRequest>,
) -> Result<Json<RestoreResult>, (StatusCode, String)> {
    let mut result = RestoreResult::default();
    let mut channel_name = req.channel.name.clone();

    // Apply prefix if specified
    if let Some(ref prefix) = req.options.prefix_names {
        channel_name = format!("{}{}", prefix, channel_name);
    }

    // Check if channel exists
    let existing = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE name = ?")
        .bind(&channel_name)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(existing_channel) = existing {
        if req.options.skip_existing {
            result.channels_skipped = 1;
            result
                .warnings
                .push(format!("Channel '{}' already exists, skipped", channel_name));
            return Ok(Json(result));
        } else if req.options.update_existing {
            sqlx::query(
                "UPDATE channels SET enabled = ?, timezone = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
            )
            .bind(req.channel.enabled as i64)
            .bind(&req.channel.timezone)
            .bind(existing_channel.id)
            .execute(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            result.channels_updated = 1;
            return Ok(Json(result));
        } else {
            result.success = false;
            result
                .errors
                .push(format!("Channel '{}' already exists", channel_name));
            return Ok(Json(result));
        }
    }

    // Create new channel
    sqlx::query(
        "INSERT INTO channels (name, enabled, timezone) VALUES (?, ?, ?)",
    )
    .bind(&channel_name)
    .bind(req.channel.enabled as i64)
    .bind(&req.channel.timezone)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    result.channels_created = 1;
    Ok(Json(result))
}

/// Import full channel with rules
pub async fn import_channel_full(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportChannelFullRequest>,
) -> Result<Json<RestoreResult>, (StatusCode, String)> {
    let mut result = RestoreResult::default();

    // First import the channel
    let channel_req = ImportChannelRequest {
        channel: req.backup.channel.clone(),
        options: req.options.clone(),
    };

    let channel_result = import_channel(State(state.clone()), Json(channel_req))
        .await?
        .0;

    result.channels_created = channel_result.channels_created;
    result.channels_updated = channel_result.channels_updated;
    result.channels_skipped = channel_result.channels_skipped;
    result.errors.extend(channel_result.errors);
    result.warnings.extend(channel_result.warnings);

    if !channel_result.success {
        result.success = false;
        return Ok(Json(result));
    }

    // Get channel name with prefix
    let mut channel_name = req.backup.channel.name.clone();
    if let Some(ref prefix) = req.options.prefix_names {
        channel_name = format!("{}{}", prefix, channel_name);
    }

    // Get the channel
    let channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE name = ?")
        .bind(&channel_name)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to retrieve created channel".to_string(),
        ))?;

    // Import rules
    for rule_backup in req.backup.rules {
        let mut rule_name = rule_backup.name.clone();
        if let Some(ref prefix) = req.options.prefix_names {
            rule_name = format!("{}{}", prefix, rule_name);
        }

        match sqlx::query(
            "INSERT INTO rules (channel_id, name, match_json, action, params_json, priority, enabled) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(channel.id)
        .bind(&rule_name)
        .bind(rule_backup.match_json.to_string())
        .bind(&rule_backup.action)
        .bind(rule_backup.params_json.to_string())
        .bind(rule_backup.priority)
        .bind(rule_backup.enabled as i64)
        .execute(&state.db)
        .await
        {
            Ok(_) => result.rules_created += 1,
            Err(e) => result
                .warnings
                .push(format!("Failed to import rule '{}': {}", rule_name, e)),
        }
    }

    Ok(Json(result))
}

/// Import a single rule to a specific channel
pub async fn import_rule_to_channel(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
    Json(req): Json<ImportRuleRequest>,
) -> Result<Json<RestoreResult>, (StatusCode, String)> {
    let mut result = RestoreResult::default();

    // Verify channel exists
    let _channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Channel not found".to_string()))?;

    // Apply prefix if specified
    let mut rule_name = req.rule.name.clone();
    if let Some(ref prefix) = req.options.prefix_names {
        rule_name = format!("{}{}", prefix, rule_name);
    }

    // Check if rule exists in this channel
    let existing = sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id = ? AND name = ?",
    )
    .bind(channel_id)
    .bind(&rule_name)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(existing_rule) = existing {
        if req.options.skip_existing {
            result.rules_skipped = 1;
            result.warnings.push(format!(
                "Rule '{}' already exists in channel, skipped",
                rule_name
            ));
            return Ok(Json(result));
        } else if req.options.update_existing {
            sqlx::query(
                "UPDATE rules SET match_json = ?, action = ?, params_json = ?, priority = ?, enabled = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
            )
            .bind(req.rule.match_json.to_string())
            .bind(&req.rule.action)
            .bind(req.rule.params_json.to_string())
            .bind(req.rule.priority)
            .bind(req.rule.enabled as i64)
            .bind(existing_rule.id)
            .execute(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            result.rules_updated = 1;
            return Ok(Json(result));
        } else {
            result.success = false;
            result
                .errors
                .push(format!("Rule '{}' already exists in channel", rule_name));
            return Ok(Json(result));
        }
    }

    // Create new rule
    sqlx::query(
        "INSERT INTO rules (channel_id, name, match_json, action, params_json, priority, enabled) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(channel_id)
    .bind(&rule_name)
    .bind(req.rule.match_json.to_string())
    .bind(&req.rule.action)
    .bind(req.rule.params_json.to_string())
    .bind(req.rule.priority)
    .bind(req.rule.enabled as i64)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    result.rules_created = 1;
    Ok(Json(result))
}

/// Import multiple rules to a channel
pub async fn import_rules_to_channel(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<i64>,
    Json(req): Json<ImportRulesRequest>,
) -> Result<Json<RestoreResult>, (StatusCode, String)> {
    let mut result = RestoreResult::default();

    for rule_backup in req.rules {
        let rule_req = ImportRuleRequest {
            rule: rule_backup,
            options: req.options.clone(),
        };

        match import_rule_to_channel(
            State(state.clone()),
            Path(channel_id),
            Json(rule_req),
        )
        .await
        {
            Ok(Json(rule_result)) => {
                result.rules_created += rule_result.rules_created;
                result.rules_updated += rule_result.rules_updated;
                result.rules_skipped += rule_result.rules_skipped;
                result.warnings.extend(rule_result.warnings);
                if !rule_result.success {
                    result.errors.extend(rule_result.errors);
                }
            }
            Err((_, err)) => {
                result.warnings.push(format!("Failed to import rule: {}", err));
            }
        }
    }

    Ok(Json(result))
}

/// Import a complete backup file
pub async fn import_backup_file(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportFileRequest>,
) -> Result<Json<RestoreResult>, (StatusCode, String)> {
    let mut result = RestoreResult::default();

    // Import full channel backups
    for full_backup in req.backup.full_channels {
        let full_req = ImportChannelFullRequest {
            backup: full_backup,
            options: req.options.clone(),
        };

        match import_channel_full(State(state.clone()), Json(full_req)).await {
            Ok(Json(fb_result)) => {
                result.channels_created += fb_result.channels_created;
                result.channels_updated += fb_result.channels_updated;
                result.channels_skipped += fb_result.channels_skipped;
                result.rules_created += fb_result.rules_created;
                result.rules_updated += fb_result.rules_updated;
                result.rules_skipped += fb_result.rules_skipped;
                result.errors.extend(fb_result.errors);
                result.warnings.extend(fb_result.warnings);
            }
            Err((_, err)) => {
                result
                    .warnings
                    .push(format!("Failed to import channel: {}", err));
            }
        }
    }

    // Import standalone channels
    for channel_backup in req.backup.channels {
        let channel_req = ImportChannelRequest {
            channel: channel_backup,
            options: req.options.clone(),
        };

        match import_channel(State(state.clone()), Json(channel_req)).await {
            Ok(Json(ch_result)) => {
                result.channels_created += ch_result.channels_created;
                result.channels_updated += ch_result.channels_updated;
                result.channels_skipped += ch_result.channels_skipped;
                result.errors.extend(ch_result.errors);
                result.warnings.extend(ch_result.warnings);
            }
            Err((_, err)) => {
                result
                    .warnings
                    .push(format!("Failed to import channel: {}", err));
            }
        }
    }

    Ok(Json(result))
}
