use crate::adapters::atomic::{atomic_write, backup_file, FileLock};
use crate::domain::{McpKind, McpServer, TargetKind};
use crate::error::{AppError, AppResult};
use crate::paths::{
    claude_mcp_json_path, resolve_codex_home, resolve_pi_agent_dir, resolve_prime_agent_dir,
    set_secret_permissions,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// XiaoBaiSwitch 只管理这个前缀下的 MCP 条目：同前缀但不在当前清单里的条目会在应用时
/// 一并清理，这样重命名、禁用、删除或取消目标都不会在客户端里留下孤儿配置。
/// 与 Pi/Prime 的 `xiaobai_` provider 命名空间同一套约定。
const MANAGED_PREFIX: &str = "xiaobai_";

/// Result of applying MCP servers to one target.
#[derive(Debug, Clone)]
pub struct McpTargetResult {
    pub ok: bool,
    pub backup_paths: Vec<String>,
    pub message: String,
}

fn kind_name(kind: McpKind) -> &'static str {
    match kind {
        McpKind::Stdio => "stdio",
        McpKind::Sse => "sse",
        McpKind::Http => "http",
    }
}

fn managed_key(name: &str) -> String {
    format!("{MANAGED_PREFIX}{name}")
}

fn keep_keys(servers: &[McpServer]) -> Vec<String> {
    servers
        .iter()
        .filter(|server| server.enabled)
        .map(|server| managed_key(&server.name))
        .collect()
}

/// 客户端条目 = 用户填的 config 原样透传，再补上缺失的 `type` 与 env/headers。
/// 用户自己在 config 里写 `type` / `transport` 时以用户为准。
fn server_entry(server: &McpServer) -> Value {
    let mut entry = match server.config.as_object() {
        Some(map) => Value::Object(map.clone()),
        None => Value::Object(Map::new()),
    };
    let object = entry
        .as_object_mut()
        .expect("server_entry always builds an object");
    object
        .entry("type")
        .or_insert_with(|| Value::String(kind_name(server.kind).to_string()));
    if has_entries(&server.env) {
        object.insert("env".to_string(), server.env.clone());
    }
    if has_entries(&server.headers) {
        object.insert("headers".to_string(), server.headers.clone());
    }
    entry
}

fn has_entries(value: &Value) -> bool {
    value.as_object().is_some_and(|map| !map.is_empty())
}

/// 去掉类型标记后拆成 (config, env, headers)。
///
/// `type` / `transport` 只是「按字段推断出传输方式」的显式写法，不参与等价判断——
/// 否则用户手工写的条目（通常不带 type）永远匹配不上我们渲染出来的结果。
fn strip_entry(entry: &Value) -> (Value, Value, Value) {
    let mut config = entry.as_object().cloned().unwrap_or_default();
    config.remove("type");
    config.remove("transport");
    let env = config
        .remove("env")
        .unwrap_or_else(|| Value::Object(Map::new()));
    let headers = config
        .remove("headers")
        .unwrap_or_else(|| Value::Object(Map::new()));
    (Value::Object(config), env, headers)
}

/// 客户端里那条**未托管**条目，是否与库内记录等价。
///
/// 只在等价时才敢用托管条目替换它——否则同一个 MCP 会在客户端里存在两份、被加载两遍。
/// 不等价说明用户在我们纳管之后又改过那条配置：那种情况必须报错让用户决定，
/// 绝不能拿库里的旧版本覆盖用户的新改动。
///
/// 这里的口径**刻意比 [`crate::adapters::mcp_identity`] 更窄**，两者不是同一件事，不要合并：
/// 只忽略 `type` / `transport` 这类显式传输标记（见 [`strip_entry`]），其余字段逐一严格比对。
/// 因为这条判断的后果是**删掉用户文件里的一条配置**：`enabled: false`（用户在客户端里
/// 主动停用）与 `timeoutMs`（用户调过的启动超时）都是有意义的差异，一旦当成噪声剔除，
/// 就会把用户禁用的服务悄悄启用、把用户改过的超时改回库里的值。
/// 而 [`crate::adapters::mcp_identity::coarse_identity`] 只回答「是不是同一个服务器」，
/// 后果是不新建重复行，放宽不会丢数据，所以它才需要吞掉这些客户端私有字段。
fn untracked_matches_record(entry: &Value, server: &McpServer) -> bool {
    let (actual_config, actual_env, actual_headers) = strip_entry(entry);
    let (expected_config, expected_env, expected_headers) = strip_entry(&server_entry(server));

    crate::adapters::mcp_scan::canonical(&actual_config)
        == crate::adapters::mcp_scan::canonical(&expected_config)
        && crate::adapters::mcp_scan::canonical(&actual_env)
            == crate::adapters::mcp_scan::canonical(&expected_env)
        && crate::adapters::mcp_scan::canonical(&actual_headers)
            == crate::adapters::mcp_scan::canonical(&expected_headers)
}

/// 接管：库内已有记录、且目标里存在同名未托管条目时，判断能否安全替换。
///
/// 返回 `Ok(true)` 表示可以删除那条未托管条目（随后写入托管条目）；`Ok(false)`
/// 表示目标里没有同名条目，无需处理。
fn can_take_over(name: &str, existing: &Value, server: &McpServer) -> AppResult<bool> {
    if untracked_matches_record(existing, server) {
        return Ok(true);
    }
    Err(AppError::new(
        "mcp_untracked_conflict",
        format!(
            "目标配置里已有名为 '{name}' 的 MCP，内容与库内记录不一致。\
             为避免覆盖你在客户端里手工改过的配置，本次没有写入。\
             请给它改名，或先删除/对齐那条配置后重试。"
        ),
    ))
}

/// 合并到 JSON 配置的 `mcpServers`，保留所有非托管条目与文件里的其它未知字段。
pub fn merge_servers_into_json(root: &mut Value, servers: &[McpServer]) -> AppResult<()> {
    let object = root.as_object_mut().ok_or_else(|| {
        AppError::new("invalid_config", "MCP config root must be a JSON object")
    })?;
    let entry = object
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let map = entry.as_object_mut().ok_or_else(|| {
        AppError::new("invalid_config", "existing mcpServers must be a JSON object")
    })?;

    // 先接管：同名未托管条目若仍与库内记录等价，就先删掉，避免与托管条目并存被重复加载。
    for server in servers.iter().filter(|server| server.enabled) {
        if map.contains_key(&managed_key(&server.name)) {
            continue;
        }
        if let Some(existing) = map.get(&server.name).cloned() {
            if can_take_over(&server.name, &existing, server)? {
                map.remove(&server.name);
            }
        }
    }

    let keep = keep_keys(servers);
    map.retain(|key, _| !key.starts_with(MANAGED_PREFIX) || keep.contains(key));
    for server in servers.iter().filter(|server| server.enabled) {
        map.insert(managed_key(&server.name), server_entry(server));
    }
    Ok(())
}
fn read_json_config(
    path: &Path,
    backup_root: &Path,
    label: &str,
) -> AppResult<(Value, Vec<String>)> {
    if !path.exists() {
        return Ok((Value::Object(Map::new()), Vec::new()));
    }
    let backup = backup_file(path, backup_root)?;
    let text = fs::read_to_string(path)?;
    let root = serde_json::from_str(&text)
        .map_err(|error| AppError::new("invalid_config", format!("invalid {label}: {error}")))?;
    Ok((root, vec![backup.display().to_string()]))
}

fn write_json(path: &Path, root: &Value, secret: bool) -> AppResult<()> {
    let text = serde_json::to_string_pretty(root)? + "\n";
    atomic_write(path, text.as_bytes(), false)?;
    if secret {
        set_secret_permissions(path);
    }
    Ok(())
}

pub fn apply_to_claude(
    servers: &[McpServer],
    claude_home_override: Option<&str>,
    backup_root: &Path,
) -> AppResult<McpTargetResult> {
    let path = claude_mcp_json_path(claude_home_override)?;
    // Claude Code 自己也在写这个文件（会话、信任状态、缓存），读-改-写期间必须持锁。
    let _lock = FileLock::acquire(&path)?;
    let (mut root, backup_paths) = read_json_config(&path, backup_root, "~/.claude.json")?;
    merge_servers_into_json(&mut root, servers)?;
    write_json(&path, &root, true)?;
    Ok(McpTargetResult {
        ok: true,
        backup_paths,
        message: format!("Applied {} MCP servers to Claude Code", servers.len()),
    })
}

pub fn apply_to_codex(
    servers: &[McpServer],
    codex_home_override: Option<&str>,
    backup_root: &Path,
) -> AppResult<McpTargetResult> {
    let path = resolve_codex_home(codex_home_override)?.join("config.toml");
    let _lock = FileLock::acquire(&path)?;
    let mut backup_paths = Vec::new();
    let mut doc = if path.exists() {
        let backup = backup_file(&path, backup_root)?;
        backup_paths.push(backup.display().to_string());
        let text = fs::read_to_string(&path)?;
        text.parse::<toml_edit::DocumentMut>().map_err(|error| {
            AppError::new(
                "invalid_config",
                format!("invalid Codex config.toml: {error}"),
            )
        })?
    } else {
        toml_edit::DocumentMut::new()
    };

    // `mcp_servers` 存在但不是表（例如 `mcp_servers = "oops"` 或 `[[mcp_servers]]`）时，
    // toml_edit 的索引赋值会 panic，这里必须先判定形状并给出可读错误。
    if let Some(item) = doc.get("mcp_servers") {
        if !item.is_table() && !item.is_inline_table() {
            return Err(AppError::new(
                "invalid_config",
                "Codex config.toml has a non-table 'mcp_servers' key",
            ));
        }
    }
    // inline table 形态（`mcp_servers = { … }`）也要参与托管条目清理，
    // 否则它会被整段跳过、留下失效的 xiaobai_* 条目。统一收敛成标准表后只需处理一种形态。
    if doc
        .get("mcp_servers")
        .is_some_and(|item| item.is_inline_table())
    {
        let inline = doc["mcp_servers"]
            .as_inline_table()
            .cloned()
            .unwrap_or_default();
        let mut table = toml_edit::Table::new();
        for (key, value) in inline.iter() {
            table.insert(key, toml_edit::Item::Value(value.clone()));
        }
        doc["mcp_servers"] = toml_edit::Item::Table(table);
    }

    // 接管：同名未托管条目若仍与库内记录等价，先删掉再写托管条目——否则 Codex 会
    // 同时加载两份同一个 MCP。内容被改过则报错跳过该目标，绝不覆盖用户的新改动。
    let mut takeover: Vec<String> = Vec::new();
    if let Some(section) = doc.get("mcp_servers").and_then(|item| item.as_table_like()) {
        for server in servers.iter().filter(|server| server.enabled) {
            if section.contains_key(&managed_key(&server.name)) {
                continue;
            }
            let Some(existing) = section
                .get(&server.name)
                .and_then(|item| item.as_table_like())
            else {
                continue;
            };
            let actual = crate::adapters::mcp_scan::codex_entry_to_json(existing);
            let expected = crate::adapters::mcp_scan::codex_record_to_json(server);
            if crate::adapters::mcp_scan::canonical(&actual)
                != crate::adapters::mcp_scan::canonical(&expected)
            {
                return Err(AppError::new(
                    "mcp_untracked_conflict",
                    format!(
                        "Codex 里已有名为 '{}' 的 MCP，内容与库内记录不一致。\
                         为避免覆盖你在 config.toml 里手工改过的配置，本次没有写入。\
                         请给它改名，或先删除/对齐那条配置后重试。",
                        server.name
                    ),
                ));
            }
            takeover.push(server.name.clone());
        }
    }
    if !takeover.is_empty() {
        if let Some(section) = doc.get_mut("mcp_servers").and_then(|item| item.as_table_mut()) {
            for name in &takeover {
                section.remove(name);
            }
        }
    }

    let keep = keep_keys(servers);
    if let Some(section) = doc.get_mut("mcp_servers").and_then(|item| item.as_table_mut()) {
        section.retain(|key, _| !key.starts_with(MANAGED_PREFIX) || keep.iter().any(|k| k == key));
    }

    for server in servers.iter().filter(|server| server.enabled) {
        let key = managed_key(&server.name);
        let mut table = toml_edit::InlineTable::new();
        // Codex 从字段形状判断传输方式：有 command 走 stdio，有 url 走 streamable HTTP。
        if let Some(command) = server.config.get("command").and_then(|v| v.as_str()) {
            table.insert("command", command.into());
        }
        if let Some(args) = server.config.get("args").and_then(|v| v.as_array()) {
            let args: Vec<String> = args
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect();
            if !args.is_empty() {
                table.insert("args", toml_edit::Value::Array(toml_edit::Array::from_iter(args)));
            }
        }
        if let Some(url) = server.config.get("url").and_then(|v| v.as_str()) {
            table.insert("url", url.into());
        }
        if let Some(bearer) = server
            .config
            .get("bearer_token_env_var")
            .and_then(|v| v.as_str())
        {
            table.insert("bearer_token_env_var", bearer.into());
        }
        if let Some(cwd) = server.config.get("cwd").and_then(|v| v.as_str()) {
            table.insert("cwd", cwd.into());
        }
        for (field, source) in [
            ("http_headers", server.config.get("http_headers")),
            ("env_http_headers", server.config.get("env_http_headers")),
        ] {
            if let Some(map) = source.and_then(|value| value.as_object()) {
                if !map.is_empty() {
                    let mut headers = toml_edit::InlineTable::new();
                    for (name, value) in map {
                        if let Some(text) = value.as_str() {
                            headers.insert(name, text.into());
                        }
                    }
                    table.insert(field, toml_edit::Value::InlineTable(headers));
                }
            }
        }
        // 统一表单里的 headers 映射到 Codex 的 http_headers；用户自己写 http_headers 时以它为准。
        if !server.config.get("http_headers").is_some_and(|v| v.is_object())
            && has_entries(&server.headers)
        {
            let mut headers = toml_edit::InlineTable::new();
            if let Some(map) = server.headers.as_object() {
                for (name, value) in map {
                    if let Some(text) = value.as_str() {
                        headers.insert(name, text.into());
                    }
                }
            }
            table.insert("http_headers", toml_edit::Value::InlineTable(headers));
        }
        if has_entries(&server.env) {
            let mut env = toml_edit::InlineTable::new();
            if let Some(map) = server.env.as_object() {
                for (name, value) in map {
                    if let Some(text) = value.as_str() {
                        env.insert(name, text.into());
                    }
                }
            }
            table.insert("env", toml_edit::Value::InlineTable(env));
        }
        if !doc.contains_key("mcp_servers") {
            doc["mcp_servers"] = toml_edit::table();
        }
        doc["mcp_servers"][&key] = toml_edit::value(toml_edit::Value::InlineTable(table));
    }

    atomic_write(&path, doc.to_string().as_bytes(), false)?;

    Ok(McpTargetResult {
        ok: true,
        backup_paths,
        message: format!("Applied {} MCP servers to Codex", servers.len()),
    })
}

pub fn apply_to_pi(
    servers: &[McpServer],
    pi_agent_dir_override: Option<&str>,
    backup_root: &Path,
) -> AppResult<McpTargetResult> {
    let path = resolve_pi_agent_dir(pi_agent_dir_override)?.join("mcp.json");
    let _lock = FileLock::acquire(&path)?;
    let (mut root, backup_paths) = read_json_config(&path, backup_root, "Pi mcp.json")?;
    merge_servers_into_json(&mut root, servers)?;
    write_json(&path, &root, true)?;
    Ok(McpTargetResult {
        ok: true,
        backup_paths,
        message: format!("Applied {} MCP servers to Pi", servers.len()),
    })
}

pub fn apply_to_prime(
    servers: &[McpServer],
    prime_agent_dir_override: Option<&str>,
    backup_root: &Path,
) -> AppResult<McpTargetResult> {
    let path = resolve_prime_agent_dir(prime_agent_dir_override)?.join("settings.json");
    // 与 Prime 适配器共用 `settings.json.lock`，避免和它自己的写入互相覆盖。
    let _lock = FileLock::acquire(&path)?;
    let (mut root, backup_paths) = read_json_config(&path, backup_root, "Prime settings.json")?;
    merge_servers_into_json(&mut root, servers)?;
    write_json(&path, &root, true)?;
    Ok(McpTargetResult {
        ok: true,
        backup_paths,
        message: format!("Applied {} MCP servers to Prime", servers.len()),
    })
}

/// 某个目标「库内期望」与「客户端现状」的差异，用于驱动「需重新应用」提示。
/// 只读计算，不写盘、不备份、不加锁。
#[derive(Debug, Clone, Default)]
pub struct McpMergePlan {
    /// 需要写入或内容有变化的托管条目数。
    pub to_write: usize,
    /// 客户端里已失效、应用时会被清理的托管条目（孤儿）数。
    pub to_clean: usize,
    /// 存在同名未托管条目且内容与库内记录不一致、会导致应用报错跳过的服务名。
    pub conflicts: Vec<String>,
}

impl McpMergePlan {
    pub fn drift(&self) -> bool {
        self.to_write > 0 || self.to_clean > 0 || !self.conflicts.is_empty()
    }
}

/// 库内针对某目标、启用中的托管条目的期望内容（key = `xiaobai_<name>`）。
fn desired_entries(
    servers: &[McpServer],
    target: TargetKind,
    entry_of: impl Fn(&McpServer) -> String,
) -> BTreeMap<String, String> {
    servers
        .iter()
        .filter(|server| server.enabled && server.targets.contains(&target))
        .map(|server| (managed_key(&server.name), entry_of(server)))
        .collect()
}

/// 由「期望条目」「客户端现有托管条目」「冲突名单」算出差异计数。
fn plan_from(
    desired: &BTreeMap<String, String>,
    actual_managed: &BTreeMap<String, String>,
    conflicts: Vec<String>,
) -> McpMergePlan {
    let to_write = desired
        .iter()
        .filter(|(key, value)| actual_managed.get(*key).map(|v| v != *value).unwrap_or(true))
        .count();
    let to_clean = actual_managed
        .keys()
        .filter(|key| !desired.contains_key(*key))
        .count();
    McpMergePlan {
        to_write,
        to_clean,
        conflicts,
    }
}

fn canonical(value: &Value) -> String {
    crate::adapters::mcp_scan::canonical(value)
}

/// 读取一个 JSON 客户端配置（只读）：不存在回 `None`，形状非法回错误。
fn read_json_readonly(path: &Path, label: &str) -> AppResult<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|error| AppError::new("invalid_config", format!("invalid {label}: {error}")))?;
    Ok(Some(root))
}

/// JSON 目标（Claude / Pi / Prime）的漂移计划。
fn plan_json_target(
    path: &Path,
    label: &str,
    servers: &[McpServer],
    target: TargetKind,
) -> AppResult<McpMergePlan> {
    let root = read_json_readonly(path, label)?;
    let servers_map = root
        .as_ref()
        .and_then(|root| root.get("mcpServers"))
        .and_then(|value| value.as_object());
    if let Some(value) = root.as_ref().and_then(|root| root.get("mcpServers")) {
        if !value.is_object() {
            return Err(AppError::new(
                "invalid_config",
                format!("existing mcpServers must be a JSON object in {label}"),
            ));
        }
    }

    let mut actual_managed = BTreeMap::new();
    let mut conflicts = Vec::new();
    let desired = desired_entries(servers, target, |server| canonical(&server_entry(server)));

    if let Some(map) = servers_map {
        for (key, entry) in map {
            if key.starts_with(MANAGED_PREFIX) {
                actual_managed.insert(key.clone(), canonical(entry));
            }
        }
        for server in servers
            .iter()
            .filter(|server| server.enabled && server.targets.contains(&target))
        {
            if map.contains_key(&managed_key(&server.name)) {
                continue;
            }
            if let Some(existing) = map.get(&server.name) {
                if !untracked_matches_record(existing, server) {
                    conflicts.push(server.name.clone());
                }
            }
        }
    }

    Ok(plan_from(&desired, &actual_managed, conflicts))
}

pub fn plan_claude(
    servers: &[McpServer],
    claude_home_override: Option<&str>,
) -> AppResult<McpMergePlan> {
    let path = claude_mcp_json_path(claude_home_override)?;
    plan_json_target(&path, "~/.claude.json", servers, TargetKind::ClaudeCode)
}

pub fn plan_pi(servers: &[McpServer], pi_agent_dir_override: Option<&str>) -> AppResult<McpMergePlan> {
    let path = resolve_pi_agent_dir(pi_agent_dir_override)?.join("mcp.json");
    plan_json_target(&path, "Pi mcp.json", servers, TargetKind::Pi)
}

pub fn plan_codex(
    servers: &[McpServer],
    codex_home_override: Option<&str>,
) -> AppResult<McpMergePlan> {
    let path = resolve_codex_home(codex_home_override)?.join("config.toml");
    let doc = if path.exists() {
        let text = fs::read_to_string(&path)?;
        Some(text.parse::<toml_edit::DocumentMut>().map_err(|error| {
            AppError::new(
                "invalid_config",
                format!("invalid Codex config.toml: {error}"),
            )
        })?)
    } else {
        None
    };

    if let Some(doc) = &doc {
        if let Some(item) = doc.get("mcp_servers") {
            if !item.is_table() && !item.is_inline_table() {
                return Err(AppError::new(
                    "invalid_config",
                    "Codex config.toml has a non-table 'mcp_servers' key",
                ));
            }
        }
    }

    let desired = desired_entries(servers, TargetKind::Codex, |server| {
        canonical(&crate::adapters::mcp_scan::codex_record_to_json(server))
    });
    let mut actual_managed = BTreeMap::new();
    let mut conflicts = Vec::new();

    if let Some(section) = doc
        .as_ref()
        .and_then(|doc| doc.get("mcp_servers"))
        .and_then(|item| item.as_table_like())
    {
        for (key, item) in section.iter() {
            if key.starts_with(MANAGED_PREFIX) {
                if let Some(table) = item.as_table_like() {
                    let json = crate::adapters::mcp_scan::codex_entry_to_json(table);
                    actual_managed.insert(key.to_string(), canonical(&json));
                }
            }
        }
        for server in servers
            .iter()
            .filter(|server| server.enabled && server.targets.contains(&TargetKind::Codex))
        {
            if section.contains_key(&managed_key(&server.name)) {
                continue;
            }
            let Some(existing) = section.get(&server.name).and_then(|item| item.as_table_like())
            else {
                continue;
            };
            let actual = crate::adapters::mcp_scan::codex_entry_to_json(existing);
            let expected = crate::adapters::mcp_scan::codex_record_to_json(server);
            if canonical(&actual) != canonical(&expected) {
                conflicts.push(server.name.clone());
            }
        }
    }

    Ok(plan_from(&desired, &actual_managed, conflicts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server(name: &str, enabled: bool) -> McpServer {
        McpServer {
            id: format!("id-{name}"),
            name: name.to_string(),
            kind: McpKind::Stdio,
            enabled,
            targets: vec![],
            config: json!({"command": "mcp-demo", "args": ["--port", "3000"]}),
            env: json!({"API_KEY": "test"}),
            headers: json!({}),
            created_at: 0,
            updated_at: 0,
            current_version: None,
            latest_version: None,
            last_update_check_at: None,
        }
    }

    fn temp_backup_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        (dir, backup)
    }

    #[test]
    fn claude_merges_managed_servers_into_claude_json() {
        let (dir, backup) = temp_backup_root();
        let existing = json!({
            "numStartups": 12,
            "mcpServers": {"user-server": {"command": "user-cmd"}}
        });
        fs::write(
            dir.path().join(".claude.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        let result = apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.backup_paths.len(), 1);
        let text = fs::read_to_string(dir.path().join(".claude.json")).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["numStartups"], 12);
        assert_eq!(root["mcpServers"]["user-server"]["command"], "user-cmd");
        assert_eq!(root["mcpServers"]["xiaobai_demo"]["command"], "mcp-demo");
        assert_eq!(root["mcpServers"]["xiaobai_demo"]["type"], "stdio");
        assert_eq!(root["mcpServers"]["xiaobai_demo"]["env"]["API_KEY"], "test");
    }

    #[test]
    fn claude_does_not_touch_settings_json() {
        let (dir, backup) = temp_backup_root();
        apply_to_claude(&[server("demo", true)], Some(dir.path().to_str().unwrap()), &backup)
            .unwrap();
        assert!(dir.path().join(".claude.json").exists());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn managed_namespace_sweeps_stale_entries_only() {
        let (dir, backup) = temp_backup_root();
        let existing = json!({
            "mcpServers": {
                "xiaobai_removed": {"command": "old"},
                "user-server": {"command": "user-cmd"}
            }
        });
        fs::write(
            dir.path().join(".claude.json"),
            serde_json::to_string(&existing).unwrap(),
        )
        .unwrap();

        apply_to_claude(
            &[server("demo", true), server("disabled", false)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let text = fs::read_to_string(dir.path().join(".claude.json")).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert!(root["mcpServers"]["xiaobai_removed"].is_null());
        assert!(root["mcpServers"]["xiaobai_disabled"].is_null());
        assert_eq!(root["mcpServers"]["user-server"]["command"], "user-cmd");
        assert_eq!(root["mcpServers"]["xiaobai_demo"]["command"], "mcp-demo");
    }

    #[test]
    fn malformed_existing_config_is_reported_not_overwritten() {
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        fs::write(&path, r#"{"mcpServers":"not-an-object"}"#).unwrap();

        let error = apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap_err();

        assert!(error.to_string().contains("mcpServers"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("not-an-object"), "file must stay untouched");
    }

    #[test]
    fn pi_and_prime_use_their_own_files() {
        let (dir, backup) = temp_backup_root();
        let pi_dir = dir.path().join("pi");
        let prime_dir = dir.path().join("prime");

        apply_to_pi(&[server("demo", true)], Some(pi_dir.to_str().unwrap()), &backup).unwrap();
        apply_to_prime(&[server("demo", true)], Some(prime_dir.to_str().unwrap()), &backup).unwrap();

        let pi: Value =
            serde_json::from_str(&fs::read_to_string(pi_dir.join("mcp.json")).unwrap()).unwrap();
        let prime: Value = serde_json::from_str(
            &fs::read_to_string(prime_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(pi["mcpServers"]["xiaobai_demo"]["command"], "mcp-demo");
        assert_eq!(prime["mcpServers"]["xiaobai_demo"]["command"], "mcp-demo");
    }

    #[test]
    fn pi_and_prime_reject_malformed_shape_and_keep_their_files() {
        // agents.md 红线：既有配置形状不合法时报错并原样保留文件。Claude 侧由
        // `malformed_existing_config_is_reported_not_overwritten` 守着，Pi / Prime 两个
        // 落点由这条守着（原先挂在退役目标的同类用例上，随其一起消失后补在这里）。
        for dir_name in ["pi", "prime"] {
            let (dir, backup) = temp_backup_root();
            let target_dir = dir.path().join(dir_name);
            fs::create_dir_all(&target_dir).unwrap();
            let file_name = if dir_name == "pi" {
                "mcp.json"
            } else {
                "settings.json"
            };
            let path = target_dir.join(file_name);
            let original = r#"{"mcpServers":"not-an-object"}"#;
            fs::write(&path, original).unwrap();
            let override_path = Some(target_dir.to_str().unwrap());

            let result = if dir_name == "pi" {
                apply_to_pi(&[server("demo", true)], override_path, &backup)
            } else {
                apply_to_prime(&[server("demo", true)], override_path, &backup)
            };
            let error = result.expect_err("shape must be reported, not overwritten");
            assert!(error.to_string().contains("mcpServers"), "{error}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                original,
                "{dir_name} config must stay untouched"
            );
        }
    }

    #[test]
    fn codex_merges_toml_and_keeps_unrelated_tables_and_entries() {
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"model = "gpt-5"

[other]
key = "value"

[mcp_servers.xiaobai_removed]
command = "old"

[mcp_servers.user]
command = "user-cmd"
"#,
        )
        .unwrap();

        let result = apply_to_codex(
            &[server("demo", true), server("disabled", false)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.backup_paths.len(), 1);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("model = \"gpt-5\""));
        assert!(text.contains("[other]"));
        assert!(text.contains("[mcp_servers.user]"), "user entries preserved");
        assert!(text.contains("xiaobai_demo"));
        assert!(!text.contains("xiaobai_removed"));
        assert!(!text.contains("xiaobai_disabled"));
    }

    #[test]
    fn codex_keeps_real_world_user_entries_with_nested_env_tables() {
        // 回归：真实用户配置里，非托管条目常带 `[mcp_servers.<name>.env]` 嵌套子表
        // （本机实测就有这种形状：顶层条目 + 独立的 .env 子表）。清理逻辑只应动
        // xiaobai_ 前缀的键，用户条目连同其嵌套 env / args 必须原样保留。
        // 这里刻意用非凭据性质的键名——要验证的是结构保留，不是密钥处理。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        let user_config = r#"model = "gpt-5"

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

[mcp_servers.third.env]
LOG_LEVEL = "PLACEHOLDER_LEVEL"

[mcp_servers.fourth]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-sequential-thinking"]
"#;
        fs::write(&path, user_config).unwrap();

        apply_to_codex(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        // 每个用户条目及其嵌套子表都要还在。
        for expected in [
            "[mcp_servers.first]",
            "[mcp_servers.first.env]",
            "PROFILES_FILE",
            "[mcp_servers.second]",
            "[mcp_servers.second.env]",
            "SERVICE_ENDPOINT",
            "[mcp_servers.third]",
            "[mcp_servers.third.env]",
            "LOG_LEVEL",
            "[mcp_servers.fourth]",
            "@modelcontextprotocol/server-sequential-thinking",
        ] {
            assert!(text.contains(expected), "user config lost: {expected}\n{text}");
        }
        // 同时托管条目要写进去。
        assert!(text.contains("xiaobai_demo"), "{text}");
        assert!(text.contains("model = \"gpt-5\""), "unrelated keys preserved");
    }

    #[test]
    fn json_targets_keep_user_entries_with_nested_env() {
        // Pi / Prime / Claude 同样：用户条目里的 env 对象必须完整保留。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "user-entry": {
                        "command": "npx",
                        "args": ["-y", "user-mcp"],
                        "env": { "PROFILES_FILE": "PLACEHOLDER_PATH" }
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let root: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["user-entry"]["command"], "npx");
        assert_eq!(
            root["mcpServers"]["user-entry"]["env"]["PROFILES_FILE"],
            "PLACEHOLDER_PATH",
            "user's nested env must survive"
        );
        assert_eq!(root["mcpServers"]["xiaobai_demo"]["command"], "mcp-demo");
    }

    #[test]
    fn codex_maps_shared_headers_to_http_headers() {
        let (dir, backup) = temp_backup_root();
        let mut http = server("remote", true);
        http.kind = McpKind::Http;
        http.config = json!({"url": "https://example.com/mcp"});
        http.headers = json!({"Authorization": "Bearer token"});

        apply_to_codex(&[http], Some(dir.path().to_str().unwrap()), &backup).unwrap();

        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(
            text.contains("http_headers") && text.contains("Authorization"),
            "shared headers must reach Codex's http_headers field: {text}"
        );
    }

    #[test]
    fn codex_prefers_explicit_http_headers_over_shared_headers() {
        let (dir, backup) = temp_backup_root();
        let mut http = server("remote", true);
        http.kind = McpKind::Http;
        http.config = json!({
            "url": "https://example.com/mcp",
            "http_headers": {"X-Explicit": "yes"},
        });
        http.headers = json!({"X-Shared": "no"});

        apply_to_codex(&[http], Some(dir.path().to_str().unwrap()), &backup).unwrap();

        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("X-Explicit"));
        assert!(!text.contains("X-Shared"), "explicit config wins: {text}");
    }

    #[test]
    fn codex_sweeps_stale_entries_inside_an_inline_table() {
        // 回归：inline table 形态的 mcp_servers 也必须参与清理，
        // 不能被 as_table_mut() 的 None 整段跳过。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"mcp_servers = { xiaobai_removed = { command = "old" }, user = { command = "user-cmd" } }
"#,
        )
        .unwrap();

        let result = apply_to_codex(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        assert!(result.ok);
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("xiaobai_removed"), "stale entry must be swept: {text}");
        assert!(text.contains("user-cmd"), "user entries preserved: {text}");
        assert!(text.contains("xiaobai_demo"));
    }

    #[test]
    fn codex_creates_file_when_missing() {
        let (dir, backup) = temp_backup_root();
        let result =
            apply_to_codex(&[server("demo", true)], Some(dir.path().to_str().unwrap()), &backup)
                .unwrap();
        assert!(result.backup_paths.is_empty());
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("xiaobai_demo"));
    }

    #[test]
    fn codex_returns_error_instead_of_panicking_on_non_table_mcp_servers() {
        // 回归：`mcp_servers` 不是表时 toml_edit 的索引赋值会 panic；命令 panic
        // 会直接终止进程，所以这里必须是可读错误，且原文件保持原样。
        for content in ["mcp_servers = \"oops\"\n", "[[mcp_servers]]\ncommand = \"x\"\n"] {
            let (dir, backup) = temp_backup_root();
            let path = dir.path().join("config.toml");
            fs::write(&path, content).unwrap();

            let error = apply_to_codex(
                &[server("demo", true)],
                Some(dir.path().to_str().unwrap()),
                &backup,
            )
            .unwrap_err();

            assert!(error.to_string().contains("mcp_servers"));
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn codex_passes_through_cwd_and_http_headers() {
        let (dir, backup) = temp_backup_root();
        let mut http = server("remote", true);
        http.kind = McpKind::Http;
        http.config = json!({
            "url": "https://example.com/mcp",
            "cwd": "/tmp/work",
            "http_headers": {"X-Api-Key": "abc"},
            "bearer_token_env_var": "MY_TOKEN",
        });

        apply_to_codex(&[http], Some(dir.path().to_str().unwrap()), &backup).unwrap();

        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("cwd"));
        assert!(text.contains("http_headers"));
        assert!(text.contains("X-Api-Key"));
        assert!(text.contains("MY_TOKEN"));
    }

    #[test]
    fn writes_are_serialized_by_the_config_lock() {
        // 持锁期间再写同一个目标必须报 lock_busy，而不是并发读-改-写丢更新。
        let (dir, backup) = temp_backup_root();
        let codex_dir = dir.path().join("codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let _held = FileLock::acquire(&codex_dir.join("config.toml")).unwrap();

        let error = apply_to_codex(
            &[server("demo", true)],
            Some(codex_dir.to_str().unwrap()),
            &backup,
        )
        .unwrap_err();
        assert!(error.to_string().contains("using"));
    }

    #[test]
    fn lock_is_released_after_a_successful_apply() {
        let (dir, backup) = temp_backup_root();
        let pi_dir = dir.path().join("pi");
        apply_to_pi(&[server("demo", true)], Some(pi_dir.to_str().unwrap()), &backup).unwrap();
        assert!(!pi_dir.join("mcp.json.lock").exists());
    }

    #[test]
    fn renaming_a_server_removes_the_old_key_from_every_target() {
        // 端到端回归：改名后旧键必须从所有已应用的客户端里消失，
        // 用户自己的条目要全程保留。这是「托管命名空间」契约的核心行为。
        let (dir, backup) = temp_backup_root();
        let claude_dir = dir.path().join("claude");
        let codex_dir = dir.path().join("codex");
        let pi_dir = dir.path().join("pi");
        let prime_dir = dir.path().join("prime");

        let apply_all = |servers: &[McpServer]| {
            apply_to_claude(servers, Some(claude_dir.to_str().unwrap()), &backup).unwrap();
            apply_to_codex(servers, Some(codex_dir.to_str().unwrap()), &backup).unwrap();
            apply_to_pi(servers, Some(pi_dir.to_str().unwrap()), &backup).unwrap();
            apply_to_prime(servers, Some(prime_dir.to_str().unwrap()), &backup).unwrap();
        };

        // 预置用户自己的条目，它必须在所有操作后原样存在。
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join(".claude.json"),
            serde_json::to_string_pretty(&json!({
                "mcpServers": {"user-server": {"command": "user-cmd"}}
            }))
            .unwrap(),
        )
        .unwrap();

        apply_all(&[server("before-rename", true)]);
        for path in [
            claude_dir.join(".claude.json"),
            codex_dir.join("config.toml"),
            pi_dir.join("mcp.json"),
            prime_dir.join("settings.json"),
        ] {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                text.contains("xiaobai_before-rename"),
                "{path:?} should hold the old key"
            );
        }

        apply_all(&[server("after-rename", true)]);
        for path in [
            claude_dir.join(".claude.json"),
            codex_dir.join("config.toml"),
            pi_dir.join("mcp.json"),
            prime_dir.join("settings.json"),
        ] {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("before-rename"),
                "old key must be swept from {path:?}: {text}"
            );
            assert!(text.contains("xiaobai_after-rename"), "{path:?}");
        }

        let claude = fs::read_to_string(claude_dir.join(".claude.json")).unwrap();
        assert!(claude.contains("user-server"), "user entries survive rename");
        assert!(claude.contains("numStartups") || claude.contains("mcpServers"));
    }

    #[test]
    fn deleting_the_last_server_clears_managed_entries_but_keeps_user_config() {
        let (dir, backup) = temp_backup_root();
        let claude_dir = dir.path().join("claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join(".claude.json"),
            serde_json::to_string(&json!({
                "theme": "dark",
                "mcpServers": {"user-server": {"command": "user-cmd"}}
            }))
            .unwrap(),
        )
        .unwrap();

        apply_to_claude(
            &[server("demo", true)],
            Some(claude_dir.to_str().unwrap()),
            &backup,
        )
        .unwrap();
        // 删除 MCP 后应用空清单，托管条目应清空。
        apply_to_claude(&[], Some(claude_dir.to_str().unwrap()), &backup).unwrap();

        let root: Value = serde_json::from_str(
            &fs::read_to_string(claude_dir.join(".claude.json")).unwrap(),
        )
        .unwrap();
        assert!(
            root["mcpServers"]["xiaobai_demo"].is_null(),
            "managed entry must be gone"
        );
        assert_eq!(root["mcpServers"]["user-server"]["command"], "user-cmd");
        assert_eq!(root["theme"], "dark", "unrelated keys survive");
    }

    /// 与库内 `server("demo")` 等价的手工条目（没有 type，靠字段推断）。
    fn untracked_demo_json() -> Value {
        json!({
            "command": "mcp-demo",
            "args": ["--port", "3000"],
            "env": { "API_KEY": "test" }
        })
    }

    #[test]
    fn takeover_replaces_equivalent_untracked_entry() {
        // 用户手工配的 demo 被纳管后，应用时应当被托管条目**替换**而不是并存——
        // 并存会让客户端同时加载两份同一个 MCP。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": { "demo": untracked_demo_json() }
            }))
            .unwrap(),
        )
        .unwrap();

        apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let map = root["mcpServers"].as_object().unwrap();
        assert_eq!(map.len(), 1, "必须只剩一条，否则会被加载两遍: {map:?}");
        assert!(map.contains_key("xiaobai_demo"));
        assert!(!map.contains_key("demo"), "等价的未托管条目应被替换掉");
    }

    #[test]
    fn takeover_refuses_and_keeps_file_when_untracked_entry_differs() {
        // 用户在纳管之后又改过那条配置：绝不能拿库里的旧版本覆盖它。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        let original = json!({
            "mcpServers": {
                "demo": { "command": "user-edited", "env": { "API_KEY": "changed" } }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        let error = apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap_err();

        assert!(error.to_string().contains("demo"), "{error}");
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after, original, "拒绝时必须原样保留文件");
    }

    #[test]
    fn takeover_ignores_explicit_type_marker() {
        // 手工条目带上 `type` 只是显式说明，不该因此被判定为「已改动」。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        let mut entry = untracked_demo_json();
        entry["type"] = json!("stdio");
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({ "mcpServers": { "demo": entry } })).unwrap(),
        )
        .unwrap();

        apply_to_claude(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"].as_object().unwrap().len(), 1);
        assert!(root["mcpServers"]["xiaobai_demo"].is_object());
    }

    #[test]
    fn takeover_refuses_when_user_disabled_or_retuned_entry() {
        // 接管口径必须比身份口径窄：`enabled: false` 是用户在客户端里主动停用的，
        // `timeoutMs` 是用户调过的启动超时。若把它们当客户端噪声忽略，接管就会删掉用户那条、
        // 写入库内版本，等于悄悄把用户禁用的服务重新启用。这类差异必须报错并原样保留文件。
        for (label, extra) in [
            ("disabled", json!({ "enabled": false })),
            ("retuned", json!({ "timeoutMs": 30_000 })),
        ] {
            let (dir, backup) = temp_backup_root();
            let path = dir.path().join(".claude.json");
            let mut entry = untracked_demo_json();
            for (key, value) in extra.as_object().unwrap() {
                entry[key] = value.clone();
            }
            let original = json!({ "mcpServers": { "demo": entry } });
            fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

            let error = apply_to_claude(
                &[server("demo", true)],
                Some(dir.path().to_str().unwrap()),
                &backup,
            )
            .unwrap_err();

            assert!(error.to_string().contains("demo"), "{label}: {error}");
            let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(after, original, "{label}: 拒绝时必须原样保留文件");
        }
    }

    #[test]
    fn takeover_skips_disabled_servers() {
        // 禁用的记录不该去动客户端里的同名条目。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join(".claude.json");
        let original = json!({ "mcpServers": { "demo": untracked_demo_json() } });
        fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        apply_to_claude(
            &[server("demo", false)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after, original, "禁用时不应触碰用户条目");
    }

    #[test]
    fn codex_takeover_replaces_equivalent_untracked_entry() {
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"model = "gpt-5"

[mcp_servers.demo]
command = "mcp-demo"
args = ["--port", "3000"]

[mcp_servers.demo.env]
API_KEY = "placeholder"
"#,
        )
        .unwrap();

        // 库内记录的 env 必须与文件里那条一致，才构成「等价」。
        let mut record = server("demo", true);
        record.env = json!({ "API_KEY": "placeholder" });

        apply_to_codex(&[record], Some(dir.path().to_str().unwrap()), &backup).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("xiaobai_demo"), "{text}");
        assert!(
            !text.contains("[mcp_servers.demo]"),
            "等价的未托管条目应被替换: {text}"
        );
        assert!(text.contains("model = \"gpt-5\""), "无关配置保留");
    }

    #[test]
    fn codex_takeover_refuses_and_keeps_file_when_entry_differs() {
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        let original = "[mcp_servers.demo]\ncommand = \"user-edited\"\n";
        fs::write(&path, original).unwrap();

        let error = apply_to_codex(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap_err();

        assert!(error.to_string().contains("demo"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original, "拒绝时原样保留");
    }

    #[test]
    fn codex_writes_normally_when_no_untracked_entry_exists() {
        // 没有同名条目时不该误报冲突。
        let (dir, backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[mcp_servers.something-else]\ncommand = \"x\"\n").unwrap();

        apply_to_codex(
            &[server("demo", true)],
            Some(dir.path().to_str().unwrap()),
            &backup,
        )
        .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("xiaobai_demo"));
        assert!(text.contains("something-else"), "其它条目保留");
    }

    fn targeted(name: &str, targets: Vec<TargetKind>) -> McpServer {
        let mut s = server(name, true);
        s.targets = targets;
        s
    }

    #[test]
    fn plan_claude_clean_after_apply() {
        let (dir, backup) = temp_backup_root();
        let home = dir.path().to_str().unwrap();
        let servers = vec![targeted("demo", vec![TargetKind::ClaudeCode])];
        apply_to_claude(&servers, Some(home), &backup).unwrap();
        let plan = plan_claude(&servers, Some(home)).unwrap();
        assert!(!plan.drift(), "刚应用完不应有漂移: {plan:?}");
        assert_eq!(plan.to_write, 0);
        assert_eq!(plan.to_clean, 0);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn plan_claude_detects_pending_write() {
        let (dir, _backup) = temp_backup_root();
        let home = dir.path().to_str().unwrap();
        // 文件不存在：目标里有启用条目 → 待写入 1。
        let servers = vec![targeted("demo", vec![TargetKind::ClaudeCode])];
        let plan = plan_claude(&servers, Some(home)).unwrap();
        assert_eq!(plan.to_write, 1);
        assert_eq!(plan.to_clean, 0);
        assert!(plan.drift());
    }

    #[test]
    fn plan_claude_detects_orphan() {
        let (dir, _backup) = temp_backup_root();
        let existing = json!({
            "mcpServers": {"xiaobai_gone": {"command": "old", "type": "stdio"}}
        });
        fs::write(
            dir.path().join(".claude.json"),
            serde_json::to_string(&existing).unwrap(),
        )
        .unwrap();
        // 库里没有任何面向 Claude 的启用条目 → 该托管条目是孤儿，待清理 1。
        let plan = plan_claude(&[], Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(plan.to_write, 0);
        assert_eq!(plan.to_clean, 1);
        assert!(plan.drift());
    }

    #[test]
    fn plan_claude_flags_untracked_conflict() {
        let (dir, _backup) = temp_backup_root();
        let existing = json!({
            "mcpServers": {"demo": {"command": "user-edited"}}
        });
        fs::write(
            dir.path().join(".claude.json"),
            serde_json::to_string(&existing).unwrap(),
        )
        .unwrap();
        let servers = vec![targeted("demo", vec![TargetKind::ClaudeCode])];
        let plan = plan_claude(&servers, Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(plan.conflicts, vec!["demo".to_string()]);
        assert!(plan.drift());
    }

    #[test]
    fn plan_claude_malformed_is_reported() {
        let (dir, _backup) = temp_backup_root();
        fs::write(
            dir.path().join(".claude.json"),
            r#"{"mcpServers":"not-an-object"}"#,
        )
        .unwrap();
        let err = plan_claude(&[], Some(dir.path().to_str().unwrap())).unwrap_err();
        assert!(err.to_string().contains("mcpServers"));
    }

    #[test]
    fn plan_claude_ignores_disabled_and_other_targets() {
        let (dir, _backup) = temp_backup_root();
        let home = dir.path().to_str().unwrap();
        let mut disabled = targeted("disabled", vec![TargetKind::ClaudeCode]);
        disabled.enabled = false;
        let servers = vec![disabled, targeted("codex-only", vec![TargetKind::Codex])];
        let plan = plan_claude(&servers, Some(home)).unwrap();
        assert!(!plan.drift(), "禁用与非本目标条目都不计入: {plan:?}");
    }

    #[test]
    fn plan_pi_clean_after_apply() {
        let (dir, backup) = temp_backup_root();
        let pi_dir = dir.path().join("pi");
        let servers = vec![targeted("demo", vec![TargetKind::Pi])];
        apply_to_pi(&servers, Some(pi_dir.to_str().unwrap()), &backup).unwrap();
        let plan = plan_pi(&servers, Some(pi_dir.to_str().unwrap())).unwrap();
        assert!(!plan.drift(), "{plan:?}");
    }

    #[test]
    fn plan_codex_clean_after_apply() {
        let (dir, backup) = temp_backup_root();
        let home = dir.path().to_str().unwrap();
        let servers = vec![targeted("demo", vec![TargetKind::Codex])];
        apply_to_codex(&servers, Some(home), &backup).unwrap();
        let plan = plan_codex(&servers, Some(home)).unwrap();
        assert!(!plan.drift(), "Codex 刚应用完不应有漂移: {plan:?}");
    }

    #[test]
    fn plan_codex_detects_orphan_and_pending() {
        let (dir, _backup) = temp_backup_root();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[mcp_servers.xiaobai_gone]\ncommand = \"old\"\ntype = \"stdio\"\n",
        )
        .unwrap();
        let servers = vec![targeted("demo", vec![TargetKind::Codex])];
        let plan = plan_codex(&servers, Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(plan.to_write, 1, "demo 待写入");
        assert_eq!(plan.to_clean, 1, "xiaobai_gone 是孤儿");
        assert!(plan.drift());
    }

    #[test]
    fn plan_codex_rejects_non_table_mcp_servers() {
        let (dir, _backup) = temp_backup_root();
        fs::write(dir.path().join("config.toml"), "mcp_servers = 1\n").unwrap();
        let err = plan_codex(&[], Some(dir.path().to_str().unwrap())).unwrap_err();
        assert!(err.to_string().contains("mcp_servers"));
    }
}
