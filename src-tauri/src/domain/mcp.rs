use serde::{Deserialize, Serialize};
use serde_json::Value;
use super::TargetKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpKind { Stdio, Sse, Http }
impl Default for McpKind { fn default() -> Self { Self::Stdio } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
 pub id: String, pub name: String, pub kind: McpKind, pub enabled: bool,
 pub targets: Vec<TargetKind>, pub config: Value, pub env: Value, pub headers: Value,
 pub created_at: i64, pub updated_at: i64,
 #[serde(default)]
 pub current_version: Option<String>,
 #[serde(default)]
 pub latest_version: Option<String>,
 #[serde(default)]
 pub last_update_check_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInput { pub id: Option<String>, pub name: String, #[serde(default)] pub kind: McpKind, #[serde(default)] pub enabled: bool, #[serde(default)] pub targets: Vec<TargetKind>, #[serde(default)] pub config: Value, #[serde(default)] pub env: Value, #[serde(default)] pub headers: Value }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSummary {
 pub id: String, pub name: String, pub kind: McpKind, pub enabled: bool,
 pub targets: Vec<TargetKind>, pub created_at: i64, pub updated_at: i64,
 #[serde(default)]
 pub current_version: Option<String>,
 #[serde(default)]
 pub latest_version: Option<String>,
 #[serde(default)]
 pub last_update_check_at: Option<i64>,
 /// 派生字段（非 DB 列）：stdio 命令首段是绝对路径，跨机可能失效，界面据此预警。
 #[serde(default)]
 pub absolute_command: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpUpdateInfo {
 pub id: String,
 pub name: String,
 pub current_version: Option<String>,
 pub latest_version: Option<String>,
 pub update_available: bool,
 pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchUpdateResult {
 pub updated: Vec<String>,
 pub failed: Vec<(String, String)>,
 pub total: usize,
}
