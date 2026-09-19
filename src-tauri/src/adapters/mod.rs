pub mod agent_rules;
pub mod agent_update;
pub mod atomic;
pub mod claude_code;
pub mod codex;
pub mod mcp;
pub mod mcp_identity;
pub mod mcp_scan;
pub mod mcp_update;
pub mod mcp_version;
pub mod pi;
pub mod prime;
pub mod thinking;

use std::collections::HashMap;

pub struct RewriteOutcome {
    pub backup_paths: Vec<String>,
    pub live_summary: HashMap<String, Option<String>>,
    pub expected_fields: HashMap<String, String>,
    pub message: String,
}

pub struct RestoreOfficialOutcome {
    #[allow(dead_code)]
    pub backup_paths: Vec<String>,
    pub env_keys: Vec<String>,
}
