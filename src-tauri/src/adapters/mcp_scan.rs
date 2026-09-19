//! 读取各客户端**已有的** MCP 配置。
//!
//! 用户很可能在装本工具之前就手工配过 MCP（本机实测 Codex 就有 5 个）。这些条目对我们
//! 来说原本是不可见的：既不能跨设备同步，也不能复用到别的客户端。这个模块负责把它们的
//! 元数据读出来，供界面展示与纳管。
//!
//! 两条硬边界：
//! - **只读**：任何函数都不写文件。纳管后是否接管由 `apply_to_*` 决定，不在这里做。
//! - **密钥值不出后端**：`env`/`headers` 只返回键名（「需要 EXA_API_KEY」），值一律不返回、
//!   不记日志、不进错误信息。`command`/`args`/`url` 会返回——那是识别条目是什么的依据，
//!   且本来就来自用户自己可读的文件。

use crate::adapters::atomic::FileLock;
use crate::domain::McpKind;
use crate::error::{AppError, AppResult};
use crate::paths::{
    claude_mcp_json_path, resolve_codex_home, resolve_pi_agent_dir, resolve_prime_agent_dir,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// 托管条目的前缀；带它的条目属于我们，不提供给用户纳管。
pub const MANAGED_PREFIX: &str = "xiaobai_";

const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanTarget {
    ClaudeCode,
    Codex,
    Pi,
    Prime,
}

/// 扫描到的一条已有 MCP。**不含任何密钥值**。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedMcp {
    pub target: ScanTarget,
    /// 客户端配置里的原始键名（可能带 `xiaobai_` 前缀）。
    pub key: String,
    /// 归一化后的名称（去掉托管前缀），用于与数据库比对。
    pub name: String,
    /// 是否由本工具写入（带托管前缀）。
    pub managed: bool,
    pub kind: McpKind,
    /// 启动方式摘要，用于界面展示「将运行：npx -y xxx」。
    pub config: Value,
    /// 依赖的环境变量**键名**（不是值）。
    pub env_keys: Vec<String>,
    /// 依赖的请求头**键名**（不是值）。
    pub header_keys: Vec<String>,
    /// 该条目在库里的同一条目 id；非空表示已纳管过。
    #[serde(default)]
    pub imported_id: Option<String>,
}

/// 扫描过程中的一条告警：某个客户端的配置读不了（文件不合法等）。
/// 单个客户端失败不影响其它客户端——用户至少能看到能读的那些。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanWarning {
    pub target: ScanTarget,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanOutcome {
    pub entries: Vec<ScannedMcp>,
    pub warnings: Vec<ScanWarning>,
}

/// 去掉托管前缀。界面与数据库比对都用归一化后的名字。
pub fn normalize_scanned_name(key: &str) -> String {
    key.strip_prefix(MANAGED_PREFIX).unwrap_or(key).to_string()
}

fn read_text(path: &Path) -> AppResult<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(AppError::new(
            "invalid_config",
            format!("{} 过大，已跳过", path.display()),
        ));
    }
    Ok(Some(fs::read_to_string(path)?))
}

/// 把 Codex 的 `[mcp_servers.<name>]` 表转成 JSON 表示。
///
/// 统一形态是为了让「客户端里那条」与「库内记录」能用同一套规范化比对——
/// 纳管、接管判断、导入三处共用，避免各写一套提取逻辑而出现口径分叉。
/// `env` / `http_headers` 子表照原名保留。
pub(crate) fn codex_entry_to_json(entry: &dyn toml_edit::TableLike) -> Value {
    let mut map = serde_json::Map::new();
    for (field, value) in entry.iter() {
        if let Some(sub) = value.as_table_like() {
            let mut nested = serde_json::Map::new();
            for (key, item) in sub.iter() {
                if let Some(text) = item.as_str() {
                    nested.insert(key.to_string(), Value::String(text.to_string()));
                }
            }
            map.insert(field.to_string(), Value::Object(nested));
        } else if let Some(text) = value.as_str() {
            map.insert(field.to_string(), Value::String(text.to_string()));
        } else if let Some(array) = value.as_array() {
            let values: Vec<Value> = array
                .iter()
                .filter_map(|item| item.as_str().map(|s| Value::String(s.to_string())))
                .collect();
            map.insert(field.to_string(), Value::Array(values));
        }
    }
    Value::Object(map)
}

/// 库内记录在 Codex 里的等价 JSON 表示，供接管比对使用。
///
/// 与 `codex_entry_to_json` 对称：env 与 headers 分别落到 `env` / `http_headers`，
/// 其余 config 字段原样铺开。
pub(crate) fn codex_record_to_json(server: &crate::domain::McpServer) -> Value {
    let mut map = server.config.as_object().cloned().unwrap_or_default();
    if server.env.as_object().is_some_and(|env| !env.is_empty()) {
        map.insert("env".to_string(), server.env.clone());
    }
    if server
        .headers
        .as_object()
        .is_some_and(|headers| !headers.is_empty())
    {
        map.insert("http_headers".to_string(), server.headers.clone());
    }
    Value::Object(map)
}

/// 从条目对象里取**键名**列表（值会在调用方被丢弃）。
fn keys_of(object: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut names: Vec<String> = object.keys().cloned().collect();
    names.sort();
    names
}

fn kind_from_entry(entry: &Value) -> McpKind {
    match entry.get("type").and_then(|v| v.as_str()) {
        Some("sse") => McpKind::Sse,
        Some("http") | Some("streamable-http") => McpKind::Http,
        _ => {
            if entry.get("url").is_some() {
                McpKind::Http
            } else {
                McpKind::Stdio
            }
        }
    }
}

/// JSON 目标里 MCP 条目所在的那一层（键名）。
///
/// 三个 JSON 目标（Claude / Pi / Prime）都是顶层 `mcpServers`，Codex 走 TOML 不经过这里。
/// 退役目标曾把条目嵌在 `mcp.servers` 下，那层按目标特判的写法随它的枚举变体一起删除：
/// 删掉之后兜底路径对三个存活目标恰好仍是正确的那一层，所以**没有**静默语义变化
/// （原实现的兜底臂就是 `mcpServers`）。将来若有新目标换成别的层级，必须重新按
/// `ScanTarget` 分支取值，而不是往这里塞字符串特判。
const JSON_SERVERS_KEY: &str = "mcpServers";

fn json_servers_value(root: &Value) -> Option<&Value> {
    root.get(JSON_SERVERS_KEY)
}

/// JSON 形态（Claude / Pi / Prime）的 MCP 条目解析。
///
/// `config` 里剔除 env/headers 后返回：那两个字段单独转成键名列表，避免值流出去。
fn parse_json_servers(text: &str, target: ScanTarget, label: &str) -> AppResult<Vec<ScannedMcp>> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| AppError::new("invalid_config", format!("{label} 不是合法 JSON: {error}")))?;
    let Some(map) = json_servers_value(&root) else {
        return Ok(Vec::new());
    };
    let Some(map) = map.as_object() else {
        return Err(AppError::new(
            "invalid_config",
            format!("{label} 的 {JSON_SERVERS_KEY} 不是对象"),
        ));
    };

    let mut out = Vec::new();
    for (key, entry) in map {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let mut config = object.clone();
        // env / headers 只留键名，值在这里丢掉。
        let env_keys = config
            .remove("env")
            .and_then(|value| value.as_object().map(keys_of))
            .unwrap_or_default();
        let header_keys = config
            .remove("headers")
            .and_then(|value| value.as_object().map(keys_of))
            .unwrap_or_default();

        out.push(ScannedMcp {
            target,
            key: key.clone(),
            name: normalize_scanned_name(key),
            managed: key.starts_with(MANAGED_PREFIX),
            kind: kind_from_entry(entry),
            config: Value::Object(config),
            env_keys,
            header_keys,
            imported_id: None,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// Codex `config.toml` 的 `[mcp_servers.*]` 解析。
///
/// 必须处理 `[mcp_servers.<name>.env]` 这种嵌套子表——用户真实配置就是这么写的
/// （本机实测 `exa` / `tavily` / `ssh` 都是这个形状）。这里用 `toml_edit` 逐层遍历，
/// 不做强类型反序列化，避免因未知字段而读漏。
fn parse_codex_servers(text: &str) -> AppResult<Vec<ScannedMcp>> {
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error| AppError::new("invalid_config", format!("Codex config.toml 不合法: {error}")))?;

    let Some(table) = doc.get("mcp_servers").and_then(|item| item.as_table_like()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (key, item) in table.iter() {
        let Some(entry) = item.as_table_like() else {
            continue;
        };
        let mut config = serde_json::Map::new();
        let mut env_keys = Vec::new();

        for (field, value) in entry.iter() {
            match field {
                // 嵌套子表：只取键名。
                "env" => {
                    if let Some(sub) = value.as_table_like() {
                        env_keys = sub.iter().map(|(k, _)| k.to_string()).collect();
                        env_keys.sort();
                    }
                }
                // 这些是结构化字段，原样带出去供界面展示与指纹比对。
                _ => {
                    if let Some(text) = value.as_str() {
                        config.insert(field.to_string(), Value::String(text.to_string()));
                    } else if let Some(array) = value.as_array() {
                        let values: Vec<Value> = array
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                            .collect();
                        config.insert(field.to_string(), Value::Array(values));
                    }
                }
            }
        }

        let kind = match config.get("url").and_then(|v| v.as_str()) {
            Some(_) => McpKind::Http,
            None => McpKind::Stdio,
        };

        out.push(ScannedMcp {
            target: ScanTarget::Codex,
            key: key.to_string(),
            name: normalize_scanned_name(key),
            managed: key.starts_with(MANAGED_PREFIX),
            kind,
            config: Value::Object(config),
            env_keys,
            // Codex 用 http_headers 表达请求头。
            header_keys: {
                let mut names = Vec::new();
                if let Some(headers) = entry.get("http_headers").and_then(|v| v.as_table_like()) {
                    names = headers.iter().map(|(k, _)| k.to_string()).collect();
                    names.sort();
                }
                names
            },
            imported_id: None,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// 扫描单个目标。返回 `None` 表示该客户端没有配置文件（未安装/未配置），不算错误。
pub fn scan_target(
    target: ScanTarget,
    settings: &crate::domain::AppSettings,
) -> AppResult<Option<Vec<ScannedMcp>>> {
    let (text, label) = match target {
        ScanTarget::ClaudeCode => (
            read_text(&claude_mcp_json_path(settings.claude_home_override.as_deref())?)?,
            "~/.claude.json",
        ),
        ScanTarget::Codex => (
            read_text(&resolve_codex_home(settings.codex_home_override.as_deref())?.join("config.toml"))?,
            "Codex config.toml",
        ),
        ScanTarget::Pi => (
            read_text(&resolve_pi_agent_dir(settings.pi_agent_dir_override.as_deref())?.join("mcp.json"))?,
            "Pi mcp.json",
        ),
        ScanTarget::Prime => (
            read_text(&resolve_prime_agent_dir(settings.prime_agent_dir_override.as_deref())?.join("settings.json"))?,
            "Prime settings.json",
        ),
    };

    let Some(text) = text else {
        return Ok(None);
    };
    let entries = match target {
        ScanTarget::Codex => parse_codex_servers(&text)?,
        other => parse_json_servers(&text, other, label)?,
    };
    Ok(Some(entries))
}

/// 扫描全部目标。单个客户端失败只记告警，不影响其它客户端。
pub fn scan_all(settings: &crate::domain::AppSettings) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    for target in [
        ScanTarget::ClaudeCode,
        ScanTarget::Codex,
        ScanTarget::Pi,
        ScanTarget::Prime,
    ] {
        match scan_target(target, settings) {
            Ok(Some(entries)) => outcome.entries.extend(entries),
            Ok(None) => {}
            Err(error) => outcome.warnings.push(ScanWarning {
                target,
                message: error.to_string(),
            }),
        }
    }
    outcome
}

/// 读回某个已扫描条目的**完整**内容（含密钥值），仅供纳管使用。
///
/// 与扫描分开是刻意的：扫描面向界面（不返回值），这里的结果直接进加密存储，两者边界清晰。
pub fn load_entry_for_import(
    target: ScanTarget,
    key: &str,
    settings: &crate::domain::AppSettings,
) -> AppResult<crate::domain::McpServerInput> {
    let (path, label) = match target {
        ScanTarget::ClaudeCode => (
            claude_mcp_json_path(settings.claude_home_override.as_deref())?,
            "~/.claude.json",
        ),
        ScanTarget::Codex => (
            resolve_codex_home(settings.codex_home_override.as_deref())?.join("config.toml"),
            "Codex config.toml",
        ),
        ScanTarget::Pi => (
            resolve_pi_agent_dir(settings.pi_agent_dir_override.as_deref())?.join("mcp.json"),
            "Pi mcp.json",
        ),
        ScanTarget::Prime => (
            resolve_prime_agent_dir(settings.prime_agent_dir_override.as_deref())?.join("settings.json"),
            "Prime settings.json",
        ),
    };

    // 与写入路径共用锁，避免读到客户端正在改写的中间状态。
    let _lock = FileLock::acquire(&path)?;
    let text = read_text(&path)?
        .ok_or_else(|| AppError::new("not_found", format!("{label} 不存在")))?;

    match target {
        ScanTarget::Codex => {
            let doc: toml_edit::DocumentMut = text.parse().map_err(|error| {
                AppError::new("invalid_config", format!("Codex config.toml 不合法: {error}"))
            })?;
            let entry = doc
                .get("mcp_servers")
                .and_then(|item| item.as_table_like())
                .and_then(|table| table.get(key))
                .and_then(|item| item.as_table_like())
                .ok_or_else(|| AppError::new("not_found", format!("Codex 里没有 MCP 条目 {key}")))?;

            let mut config = serde_json::Map::new();
            let mut env = serde_json::Map::new();
            let mut headers = serde_json::Map::new();

            for (field, value) in entry.iter() {
                match field {
                    "env" => {
                        if let Some(sub) = value.as_table_like() {
                            for (k, v) in sub.iter() {
                                if let Some(text) = v.as_str() {
                                    env.insert(k.to_string(), Value::String(text.to_string()));
                                }
                            }
                        }
                    }
                    "http_headers" => {
                        if let Some(sub) = value.as_table_like() {
                            for (k, v) in sub.iter() {
                                if let Some(text) = v.as_str() {
                                    headers.insert(k.to_string(), Value::String(text.to_string()));
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(text) = value.as_str() {
                            config.insert(field.to_string(), Value::String(text.to_string()));
                        } else if let Some(array) = value.as_array() {
                            let values: Vec<Value> = array
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                                .collect();
                            config.insert(field.to_string(), Value::Array(values));
                        }
                    }
                }
            }

            let kind = match config.get("url") {
                Some(_) => McpKind::Http,
                None => McpKind::Stdio,
            };
            Ok(crate::domain::McpServerInput {
                id: None,
                name: normalize_scanned_name(key),
                kind,
                enabled: true,
                targets: Vec::new(),
                config: Value::Object(config),
                env: Value::Object(env),
                headers: Value::Object(headers),
            })
        }
        _ => {
            let root: Value = serde_json::from_str(&text).map_err(|error| {
                AppError::new("invalid_config", format!("{label} 不是合法 JSON: {error}"))
            })?;
            let entry = json_servers_value(&root)
                .and_then(|value| value.as_object())
                .and_then(|map| map.get(key))
                .ok_or_else(|| AppError::new("not_found", format!("{label} 里没有 MCP 条目 {key}")))?;
            let Some(object) = entry.as_object() else {
                return Err(AppError::new(
                    "invalid_config",
                    format!("{label} 的条目 {key} 不是对象"),
                ));
            };

            let mut config = object.clone();
            let env = config.remove("env").unwrap_or_else(|| Value::Object(Default::default()));
            let headers = config.remove("headers").unwrap_or_else(|| Value::Object(Default::default()));

            Ok(crate::domain::McpServerInput {
                id: None,
                name: normalize_scanned_name(key),
                kind: kind_from_entry(entry),
                enabled: true,
                targets: Vec::new(),
                config: Value::Object(config),
                env,
                headers,
            })
        }
    }
}

/// 条目的规范化指纹：用于判断客户端里那条未托管条目是否仍与库内记录一致。
///
/// 只做哈希、不保留明文；键顺序归一化，避免同一个条目因序列化顺序不同而误判为「已改动」。
pub fn entry_fingerprint(
    kind: McpKind,
    config: &Value,
    env: &Value,
    headers: &Value,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{kind:?}").as_bytes());
    for value in [config, env, headers] {
        hasher.update(canonical(value).as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

/// 递归排序对象键，产出稳定字符串。
///
/// `pub(crate)`：接管比对（`adapters::mcp`）也需要同一套规范化，共用一份避免两边口径分叉。
pub(crate) fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{key}={}", canonical(&map[key])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", inner.join(","))
        }
        Value::String(text) => format!("s:{text}"),
        Value::Number(number) => format!("n:{number}"),
        Value::Bool(flag) => format!("b:{flag}"),
        Value::Null => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CLAUDE_JSON: &str = r#"{
      "numStartups": 5,
      "mcpServers": {
        "user-fs": {
          "command": "npx",
          "args": ["-y", "fs-mcp"],
          "env": { "FS_ROOT": "PLACEHOLDER_PATH" }
        },
        "xiaobai_managed": { "command": "uvx", "args": ["managed"] }
      }
    }"#;

    // 与真实用户配置同形：顶层条目 + 独立的 .env 子表。
    const CODEX_TOML: &str = r#"model = "gpt-5"

[mcp_servers.first]
command = "npx"

[mcp_servers.first.env]
PROFILES_FILE = "PLACEHOLDER_PATH"

[mcp_servers.second]
command = "npx"
args = ["-y", "second-mcp"]

[mcp_servers.second.env]
SERVICE_ENDPOINT = "PLACEHOLDER_ENDPOINT"

[mcp_servers.third]
command = "npx"
args = ["-y", "third-mcp"]
"#;

    #[test]
    fn json_scan_lists_entries_and_marks_managed() {
        let entries = parse_json_servers(CLAUDE_JSON, ScanTarget::ClaudeCode, "test").unwrap();
        assert_eq!(entries.len(), 2);

        let user = entries.iter().find(|e| e.key == "user-fs").unwrap();
        assert!(!user.managed);
        assert_eq!(user.name, "user-fs");
        assert_eq!(user.kind, McpKind::Stdio);
        assert_eq!(user.config["command"], "npx");
        assert_eq!(user.env_keys, vec!["FS_ROOT"]);

        let managed = entries.iter().find(|e| e.key == "xiaobai_managed").unwrap();
        assert!(managed.managed, "managed entries are flagged so the UI can hide them");
        assert_eq!(managed.name, "managed", "prefix stripped for comparison");
    }

    #[test]
    fn scan_never_returns_secret_values() {
        // 关键安全断言：扫描结果序列化后不得出现 env 的值。
        let entries = parse_json_servers(CLAUDE_JSON, ScanTarget::ClaudeCode, "test").unwrap();
        let serialized = serde_json::to_string(&entries).unwrap();
        assert!(
            !serialized.contains("PLACEHOLDER_PATH"),
            "secret values must not leave the backend: {serialized}"
        );
        // 键名要保留，界面才能提示「需要填什么」。
        assert!(serialized.contains("FS_ROOT"));
    }

    #[test]
    fn codex_scan_handles_nested_env_subtables() {
        // 回归：真实配置里 env 是独立的 [mcp_servers.<name>.env] 子表。
        let entries = parse_codex_servers(CODEX_TOML).unwrap();
        assert_eq!(entries.len(), 3);

        let first = entries.iter().find(|e| e.key == "first").unwrap();
        assert_eq!(first.env_keys, vec!["PROFILES_FILE"]);
        assert_eq!(first.config["command"], "npx");

        let second = entries.iter().find(|e| e.key == "second").unwrap();
        assert_eq!(second.env_keys, vec!["SERVICE_ENDPOINT"]);
        assert_eq!(second.config["args"], json!(["-y", "second-mcp"]));

        let third = entries.iter().find(|e| e.key == "third").unwrap();
        assert!(third.env_keys.is_empty());

        // 嵌套子表的值同样不得外泄。
        let serialized = serde_json::to_string(&entries).unwrap();
        assert!(!serialized.contains("PLACEHOLDER_PATH"));
        assert!(!serialized.contains("PLACEHOLDER_ENDPOINT"));
        assert!(serialized.contains("PROFILES_FILE"));
    }

    #[test]
    fn codex_scan_detects_http_entries_and_headers() {
        let entries = parse_codex_servers(
            r#"[mcp_servers.remote]
url = "https://example.com/mcp"

[mcp_servers.remote.http_headers]
X_Trace = "PLACEHOLDER"
"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, McpKind::Http);
        assert_eq!(entries[0].config["url"], "https://example.com/mcp");
        assert_eq!(entries[0].header_keys, vec!["X_Trace"]);
        assert!(!serde_json::to_string(&entries).unwrap().contains("PLACEHOLDER"));
    }

    #[test]
    fn missing_or_absent_config_yields_no_entries() {
        assert!(parse_json_servers(r#"{"numStartups":1}"#, ScanTarget::Pi, "t")
            .unwrap()
            .is_empty());
        assert!(parse_codex_servers("model = \"gpt-5\"\n").unwrap().is_empty());
    }

    #[test]
    fn malformed_config_is_reported() {
        assert!(parse_json_servers("{ not json", ScanTarget::ClaudeCode, "test").is_err());
        assert!(parse_codex_servers("this is not = = toml").is_err());
        // mcpServers 形状不对要报错，而不是静默当作空。
        assert!(parse_json_servers(r#"{"mcpServers":"oops"}"#, ScanTarget::Pi, "t").is_err());
    }

    #[test]
    fn import_reads_full_content_including_secrets() {
        // 纳管与扫描相反：需要完整内容（含密钥）才能加密入库。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(
            &path,
            serde_json::to_string(&json!({
                "mcpServers": {
                    "user-fs": {
                        "command": "npx",
                        "args": ["-y", "fs-mcp"],
                        "env": { "FS_ROOT": "PLACEHOLDER_PATH" }
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut settings = crate::domain::AppSettings::default();
        settings.pi_agent_dir_override = Some(dir.path().to_string_lossy().to_string());

        let input = load_entry_for_import(ScanTarget::Pi, "user-fs", &settings).unwrap();
        assert_eq!(input.name, "user-fs");
        assert_eq!(input.config["command"], "npx");
        assert_eq!(input.env["FS_ROOT"], "PLACEHOLDER_PATH", "import needs the value");
        assert!(input.targets.is_empty(), "target selection stays with the user");
    }

    #[test]
    fn import_errors_when_entry_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();

        let mut settings = crate::domain::AppSettings::default();
        settings.pi_agent_dir_override = Some(dir.path().to_string_lossy().to_string());

        let error = load_entry_for_import(ScanTarget::Pi, "nope", &settings).unwrap_err();
        assert!(error.to_string().contains("nope"));
    }

    #[test]
    fn fingerprint_is_stable_across_key_order_but_sensitive_to_value() {
        let config = json!({"command": "npx", "args": ["-y", "x"]});
        let env = json!({"A": "1", "B": "2"});
        let reordered = json!({"B": "2", "A": "1"});

        let base = entry_fingerprint(McpKind::Stdio, &config, &env, &json!({}));
        let same = entry_fingerprint(McpKind::Stdio, &config, &reordered, &json!({}));
        assert_eq!(base, same, "key order must not change the fingerprint");

        let changed = entry_fingerprint(McpKind::Stdio, &config, &json!({"A": "1", "B": "3"}), &json!({}));
        assert_ne!(base, changed, "a changed value must change the fingerprint");

        let other_kind = entry_fingerprint(McpKind::Http, &config, &env, &json!({}));
        assert_ne!(base, other_kind);
    }

    #[test]
    fn fingerprint_does_not_leak_values() {
        let fingerprint = entry_fingerprint(
            McpKind::Stdio,
            &json!({"command": "npx"}),
            &json!({"TOKEN": "PLACEHOLDER_SECRET"}),
            &json!({}),
        );
        assert!(!fingerprint.contains("PLACEHOLDER_SECRET"));
        assert_eq!(fingerprint.len(), 64, "sha256 hex");
    }

    #[test]
    fn normalize_strips_only_the_managed_prefix() {        assert_eq!(normalize_scanned_name("xiaobai_demo"), "demo");
        assert_eq!(normalize_scanned_name("user-server"), "user-server");
        // 前缀之外的相似名字不能被误剥。
        assert_eq!(normalize_scanned_name("xiaobaiXdemo"), "xiaobaiXdemo");
    }

    /// 诊断：读本机真实客户端配置，确认扫描能认出用户已有的 MCP。
    /// 手动运行：`cargo test scan_real_user_config -- --ignored --nocapture`
    ///
    /// 不假设本机一定配过 MCP——空结果同样合法，所以这里不做断言，只打印。
    #[test]
    #[ignore = "读取真实用户目录，仅用于本机诊断"]
    fn scan_real_user_config() {
        let settings = crate::domain::AppSettings::default();
        let outcome = scan_all(&settings);
        println!("=== entries ({}) ===", outcome.entries.len());
        for entry in &outcome.entries {
            println!(
                "  [{:?}] key={} managed={} kind={:?} env={:?} headers={:?}",
                entry.target, entry.key, entry.managed, entry.kind, entry.env_keys, entry.header_keys
            );
        }
        println!("=== warnings ({}) ===", outcome.warnings.len());
        for warning in &outcome.warnings {
            println!("  [{:?}] {}", warning.target, warning.message);
        }
    }
}
