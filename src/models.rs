use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub enabled: i64,
    pub timezone: String,
    pub owner_user_id: Option<i64>,  // NEW: ownership tracking
    pub deleted_at: Option<String>,   // NEW: soft delete
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
    pub owner_user_id: Option<i64>,  // NEW: ownership tracking
    pub deleted_at: Option<String>,   // NEW: soft delete
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

// === TEMPLATE LIBRARY + PROJECTS ===

/// A project: a persistent, shareable container bundling channel templates
/// (each channel + its rules). Members are rows in `templates`.
#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub is_shared: i64,
    pub owner_user_id: Option<i64>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A reusable template captured from a live entity.
/// `kind` is 'rule' (body_json = RuleBackup) or 'channel' (body_json = ChannelFullBackup).
#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub project_id: Option<i64>,
    pub body_json: String,
    pub is_shared: i64,
    pub is_default: i64,
    pub owner_user_id: Option<i64>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct UpsertProject {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateProjectMeta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_shared: Option<bool>,
}

/// Body for "save as template" / "add to project" (from-rule and from-channel).
#[derive(Deserialize)]
pub struct SaveTemplate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub project_id: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_shared: Option<bool>,
    /// Mark as a default/featured starter shown in the rule gallery. When the
    /// saver is an admin this is saved global (visible to all users).
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateTemplateMeta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    // Use Option<Option<i64>> so an explicit null can unfile (move to library).
    #[serde(default, deserialize_with = "double_option")]
    pub project_id: Option<Option<i64>>,
    #[serde(default)]
    pub is_shared: Option<bool>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct ApplyTemplate {
    #[serde(default)]
    pub target_channel_id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Deserialize helper distinguishing "field absent" (None) from "field is null"
/// (Some(None)) so PATCH-style updates can clear project_id.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::deserialize(de)?))
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

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct ExportedChannel {
    pub name: String,
    pub enabled: bool,
    pub timezone: String,
    pub rules: Vec<ExportedRule>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct RulesBackup {
    pub version: u32,

    #[serde(default)]
    pub exported_at: Option<String>,
    pub channels: Vec<ExportedChannel>,
}
