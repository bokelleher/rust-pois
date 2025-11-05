use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub enabled: i64,
    pub timezone: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct UpsertChannel {
    pub name: String,
    pub enabled: Option<bool>,
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct Rule {
    pub id: i64,
    pub channel_id: i64,
    pub name: String,
    pub priority: i64,
    pub enabled: i64,
    pub match_json: String,
    pub action: String,
    pub params_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct UpsertRule {
    pub name: String,
    pub priority: i64, // pass -1 to append at end
    pub enabled: Option<bool>,

    #[serde(default)]
    pub match_json: serde_json::Value,
    pub action: String,

    #[serde(default)]
    pub params_json: serde_json::Value,
}

#[derive(Deserialize)]
pub struct ReorderRules {
    pub ordered_ids: Vec<i64>, // first -> 0, then 10, 20, ...
}

#[derive(Deserialize)]
pub struct DryRunRequest {
    pub channel: String,
    pub esam_xml: String,
}

#[derive(Serialize)]
pub struct DryRunResult {
    pub matched_rule_id: Option<i64>,
    pub action: String,
    pub note: String,
}

// === BACKUP/EXPORT MODELS ===

#[derive(Serialize, Deserialize)]
pub struct ExportedRule {
    pub name: String,
    pub priority: i64,
    pub enabled: bool,

    #[serde(default)]
    pub match_json: Value,
    pub action: String,

    #[serde(default)]
    pub params_json: Value,
}

#[derive(Serialize, Deserialize)]
pub struct ExportedChannel {
    pub name: String,
    pub enabled: bool,
    pub timezone: String,
    pub rules: Vec<ExportedRule>,
}

#[derive(Serialize, Deserialize)]
pub struct RulesBackup {
    pub version: u32,

    #[serde(default)]
    pub exported_at: Option<String>,
    pub channels: Vec<ExportedChannel>,
}
