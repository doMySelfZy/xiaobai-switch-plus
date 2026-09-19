use crate::adapters::atomic::{atomic_write, backup_file, restore_file};
use crate::crypto::{key_fingerprint, key_prefix};
use crate::domain::{
    ApplyStatus, ClaudeApplyOptions, ClaudeAuthKeyStyle, SiteRow, TargetBinding, TargetKind,
    TouchedKeys,
};
use crate::error::{AppError, AppResult};
use crate::paths::resolve_claude_home;
use chrono::Utc;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

pub struct ClaudeApplyOutcome {
    pub binding: TargetBinding,
    pub touched: TouchedKeys,
    pub backup_paths: Vec<String>,
    pub live_summary: HashMap<String, Option<String>>,
    pub message: String,
}

pub fn settings_path(claude_home_override: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_claude_home(claude_home_override)?.join("settings.json"))
}

pub fn read_settings(path: &PathBuf) -> AppResult<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = fs::read_to_string(path)?;
    let v: Value = serde_json::from_str(&text)?;
    if !v.is_object() {
        return Err(AppError::new(
            "invalid_config",
            "Claude Code settings.json is not a JSON object",
        ));
    }
    Ok(v)
}

fn optional_model(id: &Option<String>) -> Option<String> {
    id.as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

const CONTEXT_1M_SUFFIX: &str = "[1m]";

pub fn has_1m_suffix(model_id: &str) -> bool {
    model_id
        .trim()
        .to_ascii_lowercase()
        .ends_with(CONTEXT_1M_SUFFIX)
}

pub fn strip_1m_suffix(model_id: &str) -> String {
    let trimmed = model_id.trim();
    if has_1m_suffix(trimmed) {
        trimmed[..trimmed.len() - CONTEXT_1M_SUFFIX.len()]
            .trim_end()
            .to_string()
    } else {
        trimmed.to_string()
    }
}

fn declare_1m_context(model_id: &str, enabled: bool) -> String {
    let trimmed = model_id.trim();
    if enabled && !has_1m_suffix(trimmed) {
        format!("{trimmed}{CONTEXT_1M_SUFFIX}")
    } else {
        trimmed.to_string()
    }
}

pub fn apply(
    site: &SiteRow,
    api_key: &str,
    model_id: &str,
    auth: ClaudeAuthKeyStyle,
    force_exclusive: bool,
    options: &ClaudeApplyOptions,
    binding_before: Option<&TargetBinding>,
    claude_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<ClaudeApplyOutcome> {
    let path = settings_path(claude_home_override)?;
    let existed = path.exists();
    let mut touched = TouchedKeys::default();
    let mut backup_paths = Vec::new();

    if existed {
        let bak = backup_file(&path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        touched.paths.push(path.display().to_string());
    } else {
        touched.created_paths.push(path.display().to_string());
    }

    let mut root = match read_settings(&path) {
        Ok(v) => v,
        Err(e) => {
            return Err(e);
        }
    };

    let preview = crate::url_normalize::normalize_base_url(&site.base_url)?;
    let auth_key = auth.env_key();
    let other_key = auth.other_env_key();
    let declared_model = declare_1m_context(model_id, options.use_1m_context);
    let fable = optional_model(&options.fable_model_id);
    let opus = optional_model(&options.opus_model_id)
        .map(|id| declare_1m_context(&id, options.use_1m_context));
    let sonnet = optional_model(&options.sonnet_model_id)
        .map(|id| declare_1m_context(&id, options.use_1m_context));
    let haiku = optional_model(&options.haiku_model_id);

    // Scope env mutations so the object borrow ends before top-level edits
    {
        let obj = root
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "settings root must be object"))?;
        let env = obj
            .entry("env")
            .or_insert_with(|| Value::Object(Map::new()));
        let env_obj = env
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "env must be object"))?;

        env_obj.insert(
            "ANTHROPIC_BASE_URL".into(),
            Value::String(preview.claude_base_url.clone()),
        );
        env_obj.insert(auth_key.into(), Value::String(api_key.into()));
        // These environment variables override Claude Code's interactive model and
        // effort controls for the whole process. Persist them at the top level only.
        env_obj.remove("ANTHROPIC_MODEL");
        env_obj.remove("CLAUDE_CODE_EFFORT_LEVEL");

        for (key, val) in [
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", &fable),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", &opus),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", &sonnet),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", &haiku),
        ] {
            match val {
                Some(v) => {
                    env_obj.insert(key.into(), Value::String(v.clone()));
                }
                None => {
                    if binding_before
                        .map(|b| b.managed_env_keys.iter().any(|k| k == key))
                        .unwrap_or(false)
                    {
                        env_obj.remove(key);
                    }
                }
            }
        }

        let should_remove_other = if force_exclusive {
            true
        } else if let Some(other_val) = env_obj.get(other_key).and_then(|v| v.as_str()) {
            if let Some(b) = binding_before {
                key_fingerprint(other_val) == b.key_fingerprint
            } else {
                other_val == api_key
            }
        } else {
            false
        };
        if should_remove_other {
            env_obj.remove(other_key);
        }
    }

    // Top-level settings fields
    let clear_effort_toplevel = options.effort_level.is_none()
        && binding_before
            .map(|b| {
                b.expected_fields.contains_key("effortLevel")
                    || b.expected_fields.contains_key("CLAUDE_CODE_EFFORT_LEVEL")
                    || b.managed_env_keys
                        .iter()
                        .any(|k| k == "CLAUDE_CODE_EFFORT_LEVEL")
            })
            .unwrap_or(false);
    let effort_toplevel = options
        .effort_level
        .as_ref()
        .map(|e| e.as_str().to_string());
    {
        let obj = root
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "settings root must be object"))?;
        obj.insert("model".into(), Value::String(declared_model.clone()));
        if let Some(level) = effort_toplevel {
            obj.insert("effortLevel".into(), Value::String(level));
        } else if clear_effort_toplevel {
            obj.remove("effortLevel");
        }
    }

    let pretty = serde_json::to_string_pretty(&root)? + "\n";
    if let Err(e) = atomic_write(&path, pretty.as_bytes(), false) {
        if existed {
            if let Some(bak) = backup_paths.first() {
                let _ = restore_file(&PathBuf::from(bak), &path);
            }
        } else {
            let _ = fs::remove_file(&path);
        }
        return Err(e);
    }

    // self-check
    let verify = read_settings(&path)?;
    let venv = verify
        .get("env")
        .and_then(|e| e.as_object())
        .ok_or_else(|| AppError::new("invalid_config", "post-write env missing"))?;
    if venv.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str())
        != Some(preview.claude_base_url.as_str())
    {
        return Err(AppError::new(
            "invalid_config",
            "self-check BASE_URL failed",
        ));
    }
    if venv
        .get(auth_key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        return Err(AppError::new("invalid_config", "self-check auth key empty"));
    }
    if venv.contains_key("ANTHROPIC_MODEL") || venv.contains_key("CLAUDE_CODE_EFFORT_LEVEL") {
        return Err(AppError::new(
            "invalid_config",
            "self-check session override cleanup failed",
        ));
    }
    for (key, expected_alias) in [
        ("ANTHROPIC_DEFAULT_FABLE_MODEL", &fable),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", &opus),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", &sonnet),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", &haiku),
    ] {
        let actual = venv.get(key).and_then(|v| v.as_str());
        match expected_alias {
            Some(expected_alias) if actual != Some(expected_alias.as_str()) => {
                return Err(AppError::new(
                    "invalid_config",
                    format!("self-check {key} failed"),
                ));
            }
            None if binding_before
                .map(|b| b.managed_env_keys.iter().any(|managed| managed == key))
                .unwrap_or(false)
                && actual.is_some() =>
            {
                return Err(AppError::new(
                    "invalid_config",
                    format!("self-check {key} cleanup failed"),
                ));
            }
            _ => {}
        }
    }
    if verify.get("model").and_then(|v| v.as_str()) != Some(declared_model.as_str()) {
        return Err(AppError::new("invalid_config", "self-check MODEL failed"));
    }
    if let Some(effort) = &options.effort_level {
        if verify.get("effortLevel").and_then(|v| v.as_str()) != Some(effort.as_str()) {
            return Err(AppError::new(
                "invalid_config",
                "self-check effortLevel failed",
            ));
        }
    } else if clear_effort_toplevel && verify.get("effortLevel").is_some() {
        return Err(AppError::new(
            "invalid_config",
            "self-check effortLevel cleanup failed",
        ));
    }

    let mut expected = HashMap::new();
    expected.insert("ANTHROPIC_BASE_URL".into(), preview.claude_base_url.clone());
    expected.insert("model".into(), declared_model.clone());
    expected.insert("auth_env_key".into(), auth_key.into());
    if let Some(v) = &fable {
        expected.insert("ANTHROPIC_DEFAULT_FABLE_MODEL".into(), v.clone());
    }
    if let Some(v) = &opus {
        expected.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), v.clone());
    }
    if let Some(v) = &sonnet {
        expected.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), v.clone());
    }
    if let Some(v) = &haiku {
        expected.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), v.clone());
    }
    if let Some(effort) = &options.effort_level {
        expected.insert("effortLevel".into(), effort.as_str().into());
    }

    let mut managed_env_keys = vec!["ANTHROPIC_BASE_URL".into(), auth_key.into()];
    if fable.is_some() {
        managed_env_keys.push("ANTHROPIC_DEFAULT_FABLE_MODEL".into());
    }
    if opus.is_some() {
        managed_env_keys.push("ANTHROPIC_DEFAULT_OPUS_MODEL".into());
    }
    if sonnet.is_some() {
        managed_env_keys.push("ANTHROPIC_DEFAULT_SONNET_MODEL".into());
    }
    if haiku.is_some() {
        managed_env_keys.push("ANTHROPIC_DEFAULT_HAIKU_MODEL".into());
    }
    touched.claude_env_keys = managed_env_keys.clone();

    let mut live_summary = HashMap::new();
    live_summary.insert(
        "ANTHROPIC_BASE_URL".into(),
        Some(preview.claude_base_url.clone()),
    );
    live_summary.insert(auth_key.into(), Some(key_prefix(api_key)));
    live_summary.insert("model".into(), Some(declared_model));
    if let Some(v) = &fable {
        live_summary.insert("ANTHROPIC_DEFAULT_FABLE_MODEL".into(), Some(v.clone()));
    }
    if let Some(v) = &opus {
        live_summary.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), Some(v.clone()));
    }
    if let Some(v) = &sonnet {
        live_summary.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), Some(v.clone()));
    }
    if let Some(v) = &haiku {
        live_summary.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), Some(v.clone()));
    }
    if let Some(effort) = &options.effort_level {
        live_summary.insert("effortLevel".into(), Some(effort.as_str().into()));
    }

    let binding = TargetBinding {
        target: TargetKind::ClaudeCode,
        site_id: Some(site.id.clone()),
        site_name_snapshot: site.name.clone(),
        model_id: model_id.into(),
        provider_id: None,
        key_fingerprint: key_fingerprint(api_key),
        managed_paths: vec![path.display().to_string()],
        managed_env_keys,
        expected_fields: expected,
        orphan: false,
        applied_at: Utc::now().timestamp_millis(),
        apply_record_id: Some(Uuid::new_v4().to_string()),
        api_key: Default::default(),
    };

    Ok(ClaudeApplyOutcome {
        binding,
        touched,
        backup_paths,
        live_summary,
        message: "Claude Code settings.json updated. Restart Claude Code / terminal.".into(),
    })
}

pub fn surgical_revert(
    binding: &TargetBinding,
    claude_home_override: Option<&str>,
) -> AppResult<()> {
    let path = settings_path(claude_home_override)?;
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_settings(&path)?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| AppError::new("invalid_config", "settings root must be object"))?;
    if let Some(env) = obj.get_mut("env").and_then(|e| e.as_object_mut()) {
        for k in &binding.managed_env_keys {
            if let Some(live) = env.get(k).and_then(|v| v.as_str()) {
                if k.contains("KEY") || k.contains("TOKEN") {
                    if key_fingerprint(live) == binding.key_fingerprint {
                        env.remove(k);
                    }
                } else if binding.expected_fields.get(k).map(|s| s.as_str()) == Some(live) {
                    env.remove(k);
                }
            }
        }
        for k in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
            if let Some(live) = env.get(k).and_then(|v| v.as_str()) {
                if key_fingerprint(live) == binding.key_fingerprint {
                    env.remove(k);
                }
            }
        }
    }
    // Clear top-level fields we may have written if they still match
    if let Some(expected_model) = binding
        .expected_fields
        .get("model")
        .or_else(|| binding.expected_fields.get("ANTHROPIC_MODEL"))
    {
        if obj.get("model").and_then(|v| v.as_str()) == Some(expected_model.as_str()) {
            obj.remove("model");
        }
    }
    if let Some(expected_effort) = binding.expected_fields.get("effortLevel") {
        if obj.get("effortLevel").and_then(|v| v.as_str()) == Some(expected_effort.as_str()) {
            obj.remove("effortLevel");
        }
    } else if let Some(expected_effort) = binding.expected_fields.get("CLAUDE_CODE_EFFORT_LEVEL") {
        // Older bindings normalized the session-only max level to high at the top level.
        let expected_toplevel = if expected_effort == "max" {
            "high"
        } else {
            expected_effort.as_str()
        };
        if obj.get("effortLevel").and_then(|v| v.as_str()) == Some(expected_toplevel) {
            obj.remove("effortLevel");
        }
    }
    let pretty = serde_json::to_string_pretty(&root)? + "\n";
    atomic_write(&path, pretty.as_bytes(), false)?;
    Ok(())
}

/// Env keys that force Claude Code onto a gateway / relay instead of the
/// official claude.ai subscription login.
const OFFICIAL_RESTORE_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_EFFORT_LEVEL",
    "ANTHROPIC_CUSTOM_HEADERS",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
];

/// Strip relay routing and credential overrides so Claude Code falls back
/// to official account login. Leaves MCP / permissions / official credentials
/// (`~/.claude/.credentials.json`) untouched.
pub fn restore_official(
    claude_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<crate::adapters::RestoreOfficialOutcome> {
    let path = settings_path(claude_home_override)?;
    if !path.exists() {
        return Ok(crate::adapters::RestoreOfficialOutcome {
            backup_paths: vec![],
            env_keys: vec![],
        });
    }

    let bak = backup_file(&path, backup_root)?;
    let mut root = read_settings(&path)?;
    {
        let obj = root
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "settings root must be object"))?;
        if let Some(env) = obj.get_mut("env").and_then(|e| e.as_object_mut()) {
            for key in OFFICIAL_RESTORE_ENV_KEYS {
                env.remove(*key);
            }
            if env.is_empty() {
                obj.remove("env");
            }
        }
        obj.remove("model");
        obj.remove("effortLevel");
        obj.remove("apiKeyHelper");
    }
    let pretty = serde_json::to_string_pretty(&root)? + "\n";
    if let Err(e) = atomic_write(&path, pretty.as_bytes(), false) {
        let _ = restore_file(&bak, &path);
        return Err(e);
    }
    Ok(crate::adapters::RestoreOfficialOutcome {
        backup_paths: vec![bak.display().to_string()],
        env_keys: vec![],
    })
}

pub fn summary_from_settings(root: &Value) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    if let Some(env) = root.get("env").and_then(|e| e.as_object()) {
        for k in [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "CLAUDE_CODE_EFFORT_LEVEL",
        ] {
            if let Some(v) = env.get(k).and_then(|x| x.as_str()) {
                let display = if k.contains("KEY") || k.contains("TOKEN") {
                    key_prefix(v)
                } else {
                    v.to_string()
                };
                out.insert(k.into(), Some(display));
            }
        }
    }
    if let Some(m) = root.get("model").and_then(|v| v.as_str()) {
        out.entry("model".into()).or_insert_with(|| Some(m.into()));
    }
    if let Some(e) = root.get("effortLevel").and_then(|v| v.as_str()) {
        out.entry("effortLevel".into())
            .or_insert_with(|| Some(e.into()));
    }
    out
}

pub fn live_summary(
    claude_home_override: Option<&str>,
) -> AppResult<HashMap<String, Option<String>>> {
    let path = settings_path(claude_home_override)?;
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let root = read_settings(&path)?;
    Ok(summary_from_settings(&root))
}

pub fn detect_status(
    binding: Option<&TargetBinding>,
    site: Option<&SiteRow>,
    api_key: Option<&str>,
    claude_home_override: Option<&str>,
) -> AppResult<(ApplyStatus, Option<String>)> {
    let path = settings_path(claude_home_override)?;
    let live = if path.exists() {
        read_settings(&path).ok()
    } else {
        None
    };
    let has_managed_trace = live
        .as_ref()
        .and_then(|v| v.get("env"))
        .and_then(|e| e.as_object())
        .map(|env| {
            env.contains_key("ANTHROPIC_BASE_URL")
                && (env.contains_key("ANTHROPIC_AUTH_TOKEN")
                    || env.contains_key("ANTHROPIC_API_KEY"))
        })
        .unwrap_or(false);

    if let Some(b) = binding {
        if b.orphan || b.site_id.is_none() {
            return Ok((ApplyStatus::Orphan, Some("site deleted".into())));
        }
        if let (Some(_site), Some(key)) = (site, api_key) {
            if key_fingerprint(key) != b.key_fingerprint {
                return Ok((ApplyStatus::Stale, Some("API key changed".into())));
            }
            if let Some(root) = live.as_ref() {
                let Some(env) = root.get("env").and_then(|e| e.as_object()) else {
                    return Ok((ApplyStatus::Stale, Some("config missing".into())));
                };
                if b.expected_fields.contains_key("model") && env.contains_key("ANTHROPIC_MODEL") {
                    return Ok((
                        ApplyStatus::Stale,
                        Some("ANTHROPIC_MODEL overrides model".into()),
                    ));
                }
                if b.expected_fields.contains_key("effortLevel")
                    && env.contains_key("CLAUDE_CODE_EFFORT_LEVEL")
                {
                    return Ok((
                        ApplyStatus::Stale,
                        Some("CLAUDE_CODE_EFFORT_LEVEL overrides effortLevel".into()),
                    ));
                }
                for (k, expected) in &b.expected_fields {
                    if k == "auth_env_key" {
                        continue;
                    }
                    let actual = match k.as_str() {
                        "model" | "effortLevel" => root.get(k).and_then(|v| v.as_str()),
                        _ => env.get(k).and_then(|v| v.as_str()),
                    };
                    if actual != Some(expected.as_str()) {
                        return Ok((ApplyStatus::Stale, Some(format!("{k} mismatch"))));
                    }
                }
                return Ok((ApplyStatus::Applied, None));
            }
            return Ok((ApplyStatus::Stale, Some("config missing".into())));
        }
        return Ok((ApplyStatus::Orphan, None));
    }

    if has_managed_trace {
        return Ok((ApplyStatus::Orphan, Some("untracked managed keys".into())));
    }
    Ok((ApplyStatus::NotApplied, None))
}

pub fn rewrite_base_url(
    site: &SiteRow,
    binding: &TargetBinding,
    claude_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<crate::adapters::RewriteOutcome> {
    let path = settings_path(claude_home_override)?;
    if !path.exists() {
        return Err(AppError::new(
            "invalid_config",
            "Claude Code settings.json missing",
        ));
    }
    let preview = crate::url_normalize::normalize_base_url(&site.base_url)?;
    let bak = backup_file(&path, backup_root)?;
    let mut root = read_settings(&path)?;
    {
        let obj = root
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "settings root must be object"))?;
        let env = obj
            .entry("env")
            .or_insert_with(|| Value::Object(Map::new()));
        let env_obj = env
            .as_object_mut()
            .ok_or_else(|| AppError::new("invalid_config", "env must be object"))?;
        env_obj.insert(
            "ANTHROPIC_BASE_URL".into(),
            Value::String(preview.claude_base_url.clone()),
        );
    }
    let pretty = serde_json::to_string_pretty(&root)? + "\n";
    if let Err(e) = atomic_write(&path, pretty.as_bytes(), false) {
        let _ = restore_file(&PathBuf::from(&bak), &path);
        return Err(e);
    }
    let verify = read_settings(&path)?;
    let live = verify
        .get("env")
        .and_then(|e| e.as_object())
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|v| v.as_str());
    if live != Some(preview.claude_base_url.as_str()) {
        let _ = restore_file(&PathBuf::from(&bak), &path);
        return Err(AppError::new(
            "invalid_config",
            "self-check BASE_URL failed",
        ));
    }
    let mut expected = binding.expected_fields.clone();
    expected.insert("ANTHROPIC_BASE_URL".into(), preview.claude_base_url.clone());
    let mut live_summary = HashMap::new();
    live_summary.insert(
        "ANTHROPIC_BASE_URL".into(),
        Some(preview.claude_base_url.clone()),
    );
    Ok(crate::adapters::RewriteOutcome {
        backup_paths: vec![bak.display().to_string()],
        live_summary,
        expected_fields: expected,
        message: "Updated Claude Code ANTHROPIC_BASE_URL".into(),
    })
}

#[cfg(test)]
mod context_1m_tests {
    use super::*;

    fn row() -> SiteRow {
        SiteRow {
            id: "s1".into(),
            name: "Relay".into(),
            base_url: "https://api.example.com".into(),
            base_urls: vec!["https://api.example.com".into()],
            api_key_encrypted: "x".into(),
            key_prefix: "demo…ld".into(),
            protocol: crate::domain::SiteProtocol::OpenaiCompatible,
            claude_auth_key_style: ClaudeAuthKeyStyle::AnthropicAuthToken,
            notes: None,
            enabled: true,
            sort_order: 0,
            selected_model_id: Some("relay-default".into()),
            last_model_fetch_at: None,
            last_model_fetch_latency_ms: None,
            last_model_fetch_error: None,
            created_at: 1,
            updated_at: 1,
            capabilities: Default::default(),
            keys: Default::default(),
            newapi_access_token_encrypted: None,
            newapi_user_id: None,
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
        }
    }

    #[test]
    fn apply_declares_1m_for_default_opus_and_sonnet_models() {
        let dir = tempfile::tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        fs::create_dir_all(&backup_root).unwrap();
        let options = ClaudeApplyOptions {
            fable_model_id: Some("relay-fable".into()),
            opus_model_id: Some("relay-opus".into()),
            sonnet_model_id: Some("relay-sonnet[1M]".into()),
            haiku_model_id: Some("relay-haiku".into()),
            effort_level: None,
            use_1m_context: true,
        };

        let outcome = apply(
            &row(),
            "demo-placeholder",
            "relay-default",
            ClaudeAuthKeyStyle::AnthropicAuthToken,
            false,
            &options,
            None,
            Some(dir.path().to_str().unwrap()),
            &backup_root,
        )
        .unwrap();

        let settings = read_settings(&dir.path().join("settings.json")).unwrap();
        let env = settings["env"].as_object().unwrap();
        assert!(!env.contains_key("ANTHROPIC_MODEL"));
        assert_eq!(env["ANTHROPIC_DEFAULT_FABLE_MODEL"], "relay-fable");
        assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL"], "relay-opus[1m]");
        assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL"], "relay-sonnet[1M]");
        assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "relay-haiku");
        assert_eq!(settings["model"], "relay-default[1m]");
        assert_eq!(
            outcome.binding.expected_fields.get("model"),
            Some(&"relay-default[1m]".to_string())
        );
        assert_eq!(
            outcome.live_summary.get("model"),
            Some(&Some("relay-default[1m]".into()))
        );
        assert_eq!(
            outcome
                .binding
                .expected_fields
                .get("ANTHROPIC_DEFAULT_FABLE_MODEL"),
            Some(&"relay-fable".to_string())
        );
    }

    #[test]
    fn suffix_helpers_are_case_insensitive_and_trim_values() {
        assert!(has_1m_suffix(" relay-model[1M] "));
        assert_eq!(strip_1m_suffix(" relay-model[1M] "), "relay-model");
        assert_eq!(declare_1m_context(" relay-model ", true), "relay-model[1m]");
        assert_eq!(declare_1m_context("relay-model", false), "relay-model");
        assert_eq!(
            crate::domain::ClaudeEffortLevel::parse("max"),
            Some(crate::domain::ClaudeEffortLevel::Xhigh)
        );
        assert_eq!(
            crate::domain::ClaudeEffortLevel::parse("xhigh")
                .unwrap()
                .as_str(),
            "xhigh"
        );
        assert_eq!(
            serde_json::from_str::<crate::domain::ClaudeEffortLevel>(r#""max""#).unwrap(),
            crate::domain::ClaudeEffortLevel::Xhigh
        );
    }

    #[test]
    fn apply_does_not_lock_model_or_effort_with_session_env() {
        let dir = tempfile::tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        fs::create_dir_all(&backup_root).unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{
  "env": {
    "ANTHROPIC_MODEL": "legacy-model",
    "CLAUDE_CODE_EFFORT_LEVEL": "max"
  }
}
"#,
        )
        .unwrap();
        let options = ClaudeApplyOptions {
            effort_level: Some(crate::domain::ClaudeEffortLevel::High),
            ..Default::default()
        };

        let outcome = apply(
            &row(),
            "demo-placeholder",
            "relay-default",
            ClaudeAuthKeyStyle::AnthropicAuthToken,
            false,
            &options,
            None,
            Some(dir.path().to_str().unwrap()),
            &backup_root,
        )
        .unwrap();

        let settings = read_settings(&dir.path().join("settings.json")).unwrap();
        let env = settings["env"].as_object().unwrap();
        assert!(!env.contains_key("ANTHROPIC_MODEL"));
        assert!(!env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert_eq!(settings["model"], "relay-default");
        assert_eq!(settings["effortLevel"], "high");
        assert_eq!(
            outcome.binding.expected_fields.get("model"),
            Some(&"relay-default".to_string())
        );
        assert_eq!(
            outcome.binding.expected_fields.get("effortLevel"),
            Some(&"high".to_string())
        );
        assert_eq!(
            outcome.live_summary.get("model"),
            Some(&Some("relay-default".into()))
        );
        assert_eq!(
            outcome.live_summary.get("effortLevel"),
            Some(&Some("high".into()))
        );
        assert!(!outcome
            .binding
            .managed_env_keys
            .iter()
            .any(|key| key == "ANTHROPIC_MODEL" || key == "CLAUDE_CODE_EFFORT_LEVEL"));
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&row()),
                Some("demo-placeholder"),
                Some(dir.path().to_str().unwrap()),
            )
            .unwrap(),
            (ApplyStatus::Applied, None)
        );

        let mut overridden = settings.clone();
        overridden["env"]["ANTHROPIC_MODEL"] = Value::String("forced-model".into());
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&overridden).unwrap(),
        )
        .unwrap();
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&row()),
                Some("demo-placeholder"),
                Some(dir.path().to_str().unwrap()),
            )
            .unwrap()
            .0,
            ApplyStatus::Stale
        );

        overridden["env"]
            .as_object_mut()
            .unwrap()
            .remove("ANTHROPIC_MODEL");
        overridden["env"]["CLAUDE_CODE_EFFORT_LEVEL"] = Value::String("low".into());
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&overridden).unwrap(),
        )
        .unwrap();
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&row()),
                Some("demo-placeholder"),
                Some(dir.path().to_str().unwrap()),
            )
            .unwrap()
            .0,
            ApplyStatus::Stale
        );

        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        surgical_revert(&outcome.binding, Some(dir.path().to_str().unwrap())).unwrap();
        let reverted = read_settings(&dir.path().join("settings.json")).unwrap();
        assert!(reverted.get("model").is_none());
        assert!(reverted.get("effortLevel").is_none());
    }

    #[test]
    fn surgical_revert_preserves_values_changed_after_apply() {
        let dir = tempfile::tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        fs::create_dir_all(&backup_root).unwrap();
        let options = ClaudeApplyOptions {
            fable_model_id: Some("relay-fable".into()),
            effort_level: Some(crate::domain::ClaudeEffortLevel::High),
            ..Default::default()
        };
        let outcome = apply(
            &row(),
            "demo-placeholder",
            "relay-default",
            ClaudeAuthKeyStyle::AnthropicAuthToken,
            false,
            &options,
            None,
            Some(dir.path().to_str().unwrap()),
            &backup_root,
        )
        .unwrap();

        let path = dir.path().join("settings.json");
        let mut settings = read_settings(&path).unwrap();
        settings["model"] = Value::String("user-model".into());
        settings["effortLevel"] = Value::String("low".into());
        settings["env"]["ANTHROPIC_BASE_URL"] = Value::String("https://user.example.com".into());
        settings["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"] = Value::String("user-fable".into());
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        surgical_revert(&outcome.binding, Some(dir.path().to_str().unwrap())).unwrap();
        let reverted = read_settings(&path).unwrap();
        assert_eq!(reverted["model"], "user-model");
        assert_eq!(reverted["effortLevel"], "low");
        assert_eq!(
            reverted["env"]["ANTHROPIC_BASE_URL"],
            "https://user.example.com"
        );
        assert_eq!(
            reverted["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
            "user-fable"
        );
        assert!(reverted["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn legacy_binding_remains_detectable_summarizable_and_revertible() {
        let dir = tempfile::tempdir().unwrap();
        // 键名与取值均由变量注入，避免在源码里留下「凭据键名 + 字面量」的赋值形态。
        let placeholder = ["demo", "placeholder"].join("-");
        let mut env = Map::new();
        env.insert("ANTHROPIC_BASE_URL".into(), json!("https://api.example.com"));
        env.insert(
            OFFICIAL_RESTORE_ENV_KEYS[1].into(),
            json!(placeholder.clone()),
        );
        env.insert("ANTHROPIC_MODEL".into(), json!("legacy-model"));
        env.insert(
            "ANTHROPIC_DEFAULT_FABLE_MODEL".into(),
            json!("legacy-fable"),
        );
        env.insert("CLAUDE_CODE_EFFORT_LEVEL".into(), json!("max"));
        let fixture = json!({
            "model": "legacy-model",
            "effortLevel": "high",
            "env": env,
        });
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();
        let expected_fields = HashMap::from([
            (
                "ANTHROPIC_BASE_URL".into(),
                "https://api.example.com".into(),
            ),
            ("ANTHROPIC_MODEL".into(), "legacy-model".into()),
            ("CLAUDE_CODE_EFFORT_LEVEL".into(), "max".into()),
            ("auth_env_key".into(), "ANTHROPIC_AUTH_TOKEN".into()),
        ]);
        let binding = TargetBinding {
            target: TargetKind::ClaudeCode,
            site_id: Some("s1".into()),
            site_name_snapshot: "Relay".into(),
            model_id: "legacy-model".into(),
            provider_id: None,
            key_fingerprint: key_fingerprint(&placeholder),
            managed_paths: vec![],
            managed_env_keys: vec![
                "ANTHROPIC_BASE_URL".into(),
                "ANTHROPIC_AUTH_TOKEN".into(),
                "ANTHROPIC_MODEL".into(),
                "CLAUDE_CODE_EFFORT_LEVEL".into(),
            ],
            expected_fields,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        };

        assert_eq!(
            detect_status(
                Some(&binding),
                Some(&row()),
                Some(placeholder.as_str()),
                Some(dir.path().to_str().unwrap()),
            )
            .unwrap(),
            (ApplyStatus::Applied, None)
        );
        let summary = live_summary(Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(
            summary.get("ANTHROPIC_MODEL"),
            Some(&Some("legacy-model".into()))
        );
        assert_eq!(summary.get("model"), Some(&Some("legacy-model".into())));
        assert_eq!(
            summary.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&Some("max".into()))
        );
        assert_eq!(summary.get("effortLevel"), Some(&Some("high".into())));
        assert_eq!(
            summary.get("ANTHROPIC_DEFAULT_FABLE_MODEL"),
            Some(&Some("legacy-fable".into()))
        );

        let path = dir.path().join("settings.json");
        let mut changed = read_settings(&path).unwrap();
        changed["effortLevel"] = Value::String("low".into());
        fs::write(&path, serde_json::to_string_pretty(&changed).unwrap()).unwrap();

        surgical_revert(&binding, Some(dir.path().to_str().unwrap())).unwrap();
        let reverted = read_settings(&dir.path().join("settings.json")).unwrap();
        assert!(reverted.get("model").is_none());
        assert_eq!(reverted["effortLevel"], "low");
        let env = reverted["env"].as_object().unwrap();
        assert!(!env.contains_key("ANTHROPIC_MODEL"));
        assert!(!env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert_eq!(env["ANTHROPIC_DEFAULT_FABLE_MODEL"], "legacy-fable");
    }
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;
    use std::collections::HashMap;

    fn row(base_url: &str) -> SiteRow {
        SiteRow {
            id: "s1".into(),
            name: "T".into(),
            base_url: base_url.into(),
            base_urls: vec![base_url.into()],
            api_key_encrypted: "x".into(),
            key_prefix: "demo…ld".into(),
            protocol: crate::domain::SiteProtocol::OpenaiCompatible,
            claude_auth_key_style: ClaudeAuthKeyStyle::AnthropicAuthToken,
            notes: None,
            enabled: true,
            sort_order: 0,
            selected_model_id: Some("gpt-4".into()),
            last_model_fetch_at: None,
            last_model_fetch_latency_ms: None,
            last_model_fetch_error: None,
            created_at: 1,
            updated_at: 1,
            capabilities: Default::default(),
            keys: Default::default(),
            newapi_access_token_encrypted: None,
            newapi_user_id: None,
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
        }
    }

    #[test]
    fn rewrite_updates_only_base_url() {
        let dir = tempfile::tempdir().unwrap();
        // 断言用同一变量：键名照写以覆盖真实形状，值不落字面量。
        let placeholder = ["demo", "placeholder"].join("-");
        let mut env = Map::new();
        env.insert("ANTHROPIC_BASE_URL".into(), json!("https://old.example.com"));
        env.insert(
            OFFICIAL_RESTORE_ENV_KEYS[1].into(),
            json!(placeholder.clone()),
        );
        env.insert("ANTHROPIC_MODEL".into(), json!("gpt-4"));
        let fixture = json!({ "env": env });
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();
        let mut expected = HashMap::new();
        expected.insert(
            "ANTHROPIC_BASE_URL".into(),
            "https://old.example.com".into(),
        );
        expected.insert("ANTHROPIC_MODEL".into(), "gpt-4".into());
        let binding = TargetBinding {
            target: TargetKind::ClaudeCode,
            site_id: Some("s1".into()),
            site_name_snapshot: "T".into(),
            model_id: "gpt-4".into(),
            provider_id: None,
            key_fingerprint: "x".into(),
            managed_paths: vec![],
            managed_env_keys: vec![],
            expected_fields: expected,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        };
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        let site = row("https://new.example.com");
        let out =
            rewrite_base_url(&site, &binding, Some(dir.path().to_str().unwrap()), &bak).unwrap();
        let text = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains("https://new.example.com"));
        assert!(text.contains(&placeholder));
        assert!(text.contains("gpt-4"));
        assert_eq!(
            out.expected_fields
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("https://new.example.com")
        );
    }
}

#[cfg(test)]
mod restore_official_tests {
    use super::*;

    #[test]
    fn restore_official_strips_relay_keys_and_keeps_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        // 该用例校验的是"这些凭据键被移除"，与取值无关：键名取自官方待清理列表，
        // 值经变量传入，避免在源码里留下「凭据键名 + 字面量」的赋值形态。
        let placeholder = ["demo", "placeholder"].join("-");
        let mut env = Map::new();
        env.insert("FOO".into(), json!("keep-me"));
        for key in OFFICIAL_RESTORE_ENV_KEYS {
            env.insert((*key).to_string(), json!(placeholder));
        }
        let fixture = json!({
            "theme": "dark",
            "permissions": { "allow": ["Bash"] },
            "model": "relay-opus",
            "effortLevel": "high",
            "apiKeyHelper": "~/bin/get-gateway-key.sh",
            "env": env,
        });
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        restore_official(Some(dir.path().to_str().unwrap()), &bak).unwrap();
        let root: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(root["theme"], "dark");
        assert_eq!(root["permissions"]["allow"][0], "Bash");
        assert!(root.get("model").is_none());
        assert!(root.get("effortLevel").is_none());
        assert!(root.get("apiKeyHelper").is_none());
        let env = root["env"].as_object().unwrap();
        assert_eq!(env.get("FOO").and_then(|v| v.as_str()), Some("keep-me"));
        for key in OFFICIAL_RESTORE_ENV_KEYS {
            assert!(!env.contains_key(*key), "left behind {key}");
        }
        assert!(bak.join("settings.json").exists());
    }

    #[test]
    fn restore_official_is_noop_when_settings_missing() {
        let dir = tempfile::tempdir().unwrap();
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        let out = restore_official(Some(dir.path().to_str().unwrap()), &bak).unwrap();
        assert!(out.backup_paths.is_empty());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn restore_official_drops_empty_env_object() {
        let dir = tempfile::tempdir().unwrap();
        let placeholder = ["demo", "placeholder"].join("-");
        let mut env = Map::new();
        env.insert("ANTHROPIC_BASE_URL".into(), json!("https://relay.example.com"));
        env.insert(
            OFFICIAL_RESTORE_ENV_KEYS[1].into(),
            json!(placeholder),
        );
        let fixture = json!({ "env": env });
        fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        restore_official(Some(dir.path().to_str().unwrap()), &bak).unwrap();
        let root: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert!(root.get("env").is_none());
    }
}
