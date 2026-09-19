use crate::adapters::atomic::{atomic_write, backup_file, restore_file};
use crate::crypto::{key_fingerprint, key_prefix};
use crate::domain::{
    env_key_for_site, provider_id_for_site, ApplyStatus, CodexApplyOptions, SiteRow, TargetBinding,
    TargetKind, TouchedKeys,
};
use crate::env_inject::codex_env_file::{
    list_defined_keys, read_env_file, remove_env_key, upsert_env_key, write_env_file,
};
use crate::error::{AppError, AppResult};
use crate::paths::{codex_env_path, resolve_codex_home, set_secret_permissions};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use toml_edit::{value, DocumentMut};
use uuid::Uuid;

pub struct CodexApplyOutcome {
    pub binding: TargetBinding,
    pub touched: TouchedKeys,
    pub backup_paths: Vec<String>,
    pub live_summary: HashMap<String, Option<String>>,
    pub message: String,
    pub env_key: String,
    pub provider_id: String,
}

pub fn config_path(codex_home_override: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_codex_home(codex_home_override)?.join("config.toml"))
}

/// Existing catalog files that write-all-models will replace (pointer and/or dest).
pub fn catalogs_to_backup(
    doc: &DocumentMut,
    our_catalog: &PathBuf,
    codex_home: &std::path::Path,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(raw) = doc.get("model_catalog_json").and_then(|i| i.as_str()) {
        if let Some(path) = expand_catalog_path(raw, codex_home) {
            if path.exists() && !out.iter().any(|p| p == &path) {
                out.push(path);
            }
        }
    }
    if our_catalog.exists() && !out.iter().any(|p| p == our_catalog) {
        out.push(our_catalog.clone());
    }
    out
}

fn expand_catalog_path(raw: &str, codex_home: &std::path::Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "~" {
        return crate::paths::home_dir().ok();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return crate::paths::home_dir().ok().map(|h| h.join(rest));
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(codex_home.join(path))
    }
}

pub fn model_catalog_path(codex_home_override: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_codex_home(codex_home_override)?.join("xiaobai-model-catalog.json"))
}

const REMOTE_COMPACTION_PROVIDER_NAME: &str = "OpenAI";
const WEB_SEARCH_DISABLED: &str = "disabled";
const WEB_SEARCH_ENABLED: &str = "cached";

fn flag_str(on: bool) -> &'static str {
    if on {
        "true"
    } else {
        "false"
    }
}

fn apply_provider_display_name(
    table: &mut toml_edit::Table,
    site_name: &str,
    remote_compaction: bool,
) {
    let name = if remote_compaction {
        REMOTE_COMPACTION_PROVIDER_NAME
    } else {
        site_name
    };
    table["name"] = value(name);
}

fn apply_web_search(doc: &mut DocumentMut, enabled: bool) {
    doc["web_search"] = value(if enabled {
        WEB_SEARCH_ENABLED
    } else {
        WEB_SEARCH_DISABLED
    });
}

fn apply_image_understanding(doc: &mut DocumentMut, enabled: bool) {
    doc["tools"]["view_image"] = value(enabled);
}

fn apply_image_generation(doc: &mut DocumentMut, enabled: bool) {
    doc["features"]["image_generation"] = value(enabled);
}

fn revert_managed_capability_fields(doc: &mut DocumentMut, expected: &HashMap<String, String>) {
    if let Some(want) = expected.get("web_search") {
        if doc.get("web_search").and_then(|i| i.as_str()) == Some(want.as_str()) {
            doc.as_table_mut().remove("web_search");
        }
    }
    if let Some(want) = expected.get("tools_view_image") {
        let live = doc
            .get("tools")
            .and_then(|i| i.get("view_image"))
            .and_then(|i| i.as_bool())
            .map(flag_str);
        if live == Some(want.as_str()) {
            if let Some(tools) = doc.get_mut("tools").and_then(|i| i.as_table_like_mut()) {
                tools.remove("view_image");
            }
        }
    }
    if let Some(want) = expected.get("features_image_generation") {
        let live = doc
            .get("features")
            .and_then(|i| i.get("image_generation"))
            .and_then(|i| i.as_bool())
            .map(flag_str);
        if live == Some(want.as_str()) {
            if let Some(features) = doc.get_mut("features").and_then(|i| i.as_table_like_mut()) {
                features.remove("image_generation");
            }
        }
    }
}

fn build_model_catalog(
    models: &[(String, String)],
    site_name: &str,
    image_understanding: bool,
) -> Value {
    let modalities = if image_understanding {
        json!(["text", "image"])
    } else {
        json!(["text"])
    };
    let items: Vec<Value> = models
        .iter()
        .enumerate()
        .map(|(i, (id, display))| {
            json!({
                "slug": id,
                "display_name": display,
                "description": format!("From XiaoBaiSwitch Plus · {site_name}"),
                "context_window": 128000,
                "max_context_window": 128000,
                "visibility": "list",
                "supported_in_api": true,
                "input_modalities": modalities,
                "priority": i + 1,
                "default_reasoning_level": "medium",
                "supported_reasoning_levels": [
                    { "effort": "low", "description": "Fast responses with lighter reasoning" },
                    { "effort": "medium", "description": "Balances speed and reasoning depth" },
                    { "effort": "high", "description": "Greater reasoning depth for complex problems" },
                    { "effort": "xhigh", "description": "Extra high reasoning depth" }
                ]
            })
        })
        .collect();
    json!({ "models": items })
}

pub fn apply(
    site: &SiteRow,
    api_key: &str,
    model_id: &str,
    options: &CodexApplyOptions,
    codex_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<CodexApplyOutcome> {
    let cfg_path = config_path(codex_home_override)?;
    let env_path = codex_env_path()?;
    let catalog_path = model_catalog_path(codex_home_override)?;
    let provider_id = provider_id_for_site(&site.id);
    let env_key = env_key_for_site(&site.id);
    let preview = crate::url_normalize::normalize_base_url(&site.base_url)?;

    let mut touched = TouchedKeys::default();
    let mut backup_paths = Vec::new();

    // config.toml
    let cfg_existed = cfg_path.exists();
    if cfg_existed {
        let bak = backup_file(&cfg_path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        touched.paths.push(cfg_path.display().to_string());
    } else {
        touched.created_paths.push(cfg_path.display().to_string());
    }

    let mut doc = if cfg_existed {
        let text = fs::read_to_string(&cfg_path)?;
        text.parse::<DocumentMut>()
            .map_err(|e| AppError::new("invalid_config", format!("invalid config.toml: {e}")))?
    } else {
        DocumentMut::new()
    };

    doc["model"] = value(model_id);
    doc["model_provider"] = value(&provider_id);

    if let Some(effort) = &options.reasoning_effort {
        doc["model_reasoning_effort"] = value(effort.as_str());
    } else {
        // leave existing value unless we previously managed it — keep simple: only set when provided
    }

    {
        let providers = doc["model_providers"]
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| AppError::new("invalid_config", "model_providers must be table"))?;
        let table = providers
            .entry(&provider_id)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| AppError::new("invalid_config", "provider table"))?;
        apply_provider_display_name(table, &site.name, options.remote_compaction);
        table["base_url"] = value(&preview.codex_base_url);
        table["env_key"] = value(&env_key);
        table["wire_api"] = value("responses");
    }

    apply_web_search(&mut doc, options.web_search);
    apply_image_understanding(&mut doc, options.image_understanding);
    apply_image_generation(&mut doc, options.image_generation);

    // Optional model catalog for in-app model switching
    if options.write_all_models {
        let catalog_models = if options.catalog_models.is_empty() {
            vec![(model_id.to_string(), model_id.to_string())]
        } else {
            options.catalog_models.clone()
        };
        let catalog = build_model_catalog(&catalog_models, &site.name, options.image_understanding);
        let catalog_text = serde_json::to_string_pretty(&catalog)? + "\n";
        let codex_home = resolve_codex_home(codex_home_override)?;
        for existing in catalogs_to_backup(&doc, &catalog_path, &codex_home) {
            let bak = backup_file(&existing, backup_root)?;
            backup_paths.push(bak.display().to_string());
            let src = existing.display().to_string();
            if !touched.paths.contains(&src) {
                touched.paths.push(src);
            }
        }
        if !catalog_path.exists() {
            touched
                .created_paths
                .push(catalog_path.display().to_string());
        }
        if let Err(e) = atomic_write(&catalog_path, catalog_text.as_bytes(), false) {
            if cfg_existed {
                if let Some(bak) = backup_paths.first() {
                    let _ = restore_file(&PathBuf::from(bak), &cfg_path);
                }
            }
            return Err(e);
        }
        doc["model_catalog_json"] = value(catalog_path.display().to_string());
    }

    let cfg_text = doc.to_string();
    if let Err(e) = atomic_write(&cfg_path, cfg_text.as_bytes(), false) {
        if cfg_existed {
            if let Some(bak) = backup_paths.first() {
                let _ = restore_file(&PathBuf::from(bak), &cfg_path);
            }
        } else {
            let _ = fs::remove_file(&cfg_path);
        }
        return Err(e);
    }

    // codex.env
    let env_existed = env_path.exists();
    if env_existed {
        let bak = backup_file(&env_path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        if !touched.paths.contains(&env_path.display().to_string()) {
            touched.paths.push(env_path.display().to_string());
        }
    } else {
        touched.created_paths.push(env_path.display().to_string());
    }

    let mut lines = if env_existed {
        read_env_file(&env_path)?
    } else {
        vec![
            "# Managed by XiaoBaiSwitch Plus — do not commit".into(),
            String::new(),
        ]
    };
    upsert_env_key(&mut lines, &env_key, api_key);
    if let Err(e) = write_env_file(&env_path, &lines) {
        if cfg_existed {
            if let Some(bak) = backup_paths.first() {
                let _ = restore_file(&PathBuf::from(bak), &cfg_path);
            }
        }
        return Err(e);
    }
    set_secret_permissions(&env_path);
    touched.env_keys.push(env_key.clone());

    let mut expected = HashMap::new();
    expected.insert("model".into(), model_id.into());
    expected.insert("model_provider".into(), provider_id.clone());
    expected.insert("base_url".into(), preview.codex_base_url.clone());
    expected.insert("env_key".into(), env_key.clone());
    expected.insert("wire_api".into(), "responses".into());
    if let Some(effort) = &options.reasoning_effort {
        expected.insert("model_reasoning_effort".into(), effort.as_str().into());
    }
    let provider_display_name = if options.remote_compaction {
        REMOTE_COMPACTION_PROVIDER_NAME
    } else {
        site.name.as_str()
    };
    expected.insert("provider_display_name".into(), provider_display_name.into());
    expected.insert(
        "remote_compaction".into(),
        if options.remote_compaction {
            "on".into()
        } else {
            "off".into()
        },
    );
    expected.insert(
        "web_search".into(),
        if options.web_search {
            WEB_SEARCH_ENABLED.into()
        } else {
            WEB_SEARCH_DISABLED.into()
        },
    );
    expected.insert(
        "tools_view_image".into(),
        flag_str(options.image_understanding).into(),
    );
    expected.insert(
        "features_image_generation".into(),
        flag_str(options.image_generation).into(),
    );
    expected.insert(
        "capability_source".into(),
        options.capability_source.as_str().into(),
    );
    if options.write_all_models {
        expected.insert(
            "model_catalog_json".into(),
            catalog_path.display().to_string(),
        );
    }

    let mut live_summary = HashMap::new();
    live_summary.insert("model".into(), Some(model_id.into()));
    live_summary.insert("model_provider".into(), Some(provider_id.clone()));
    live_summary.insert("base_url".into(), Some(preview.codex_base_url));
    live_summary.insert("env_key".into(), Some(env_key.clone()));
    live_summary.insert(env_key.clone(), Some(key_prefix(api_key)));
    if let Some(effort) = &options.reasoning_effort {
        live_summary.insert(
            "model_reasoning_effort".into(),
            Some(effort.as_str().into()),
        );
    }
    live_summary.insert(
        "provider_display_name".into(),
        Some(provider_display_name.into()),
    );
    live_summary.insert(
        "remote_compaction".into(),
        Some(if options.remote_compaction {
            "on".into()
        } else {
            "off".into()
        }),
    );
    live_summary.insert(
        "web_search".into(),
        Some(if options.web_search {
            WEB_SEARCH_ENABLED.into()
        } else {
            WEB_SEARCH_DISABLED.into()
        }),
    );
    live_summary.insert(
        "tools_view_image".into(),
        Some(flag_str(options.image_understanding).into()),
    );
    live_summary.insert(
        "features_image_generation".into(),
        Some(flag_str(options.image_generation).into()),
    );
    live_summary.insert(
        "capability_source".into(),
        Some(options.capability_source.as_str().into()),
    );
    if options.write_all_models {
        live_summary.insert(
            "model_catalog_json".into(),
            Some(catalog_path.display().to_string()),
        );
        live_summary.insert(
            "catalog_models".into(),
            Some(options.catalog_models.len().max(1).to_string()),
        );
    }

    let mut managed_paths = vec![
        cfg_path.display().to_string(),
        env_path.display().to_string(),
    ];
    if options.write_all_models {
        managed_paths.push(catalog_path.display().to_string());
    }

    let binding = TargetBinding {
        target: TargetKind::Codex,
        site_id: Some(site.id.clone()),
        site_name_snapshot: site.name.clone(),
        model_id: model_id.into(),
        provider_id: Some(provider_id.clone()),
        key_fingerprint: key_fingerprint(api_key),
        managed_paths,
        managed_env_keys: vec![env_key.clone()],
        expected_fields: expected,
        orphan: false,
        applied_at: Utc::now().timestamp_millis(),
        apply_record_id: Some(Uuid::new_v4().to_string()),
        api_key: Default::default(),
    };

    let mut message =
        "Codex config.toml + codex.env updated. Restart Codex / open a new terminal.".to_string();
    if options.write_all_models {
        message.push_str(" Model catalog written for in-CLI model switching.");
    }

    Ok(CodexApplyOutcome {
        binding,
        touched,
        backup_paths,
        live_summary,
        message,
        env_key,
        provider_id,
    })
}

pub fn surgical_revert(
    binding: &TargetBinding,
    codex_home_override: Option<&str>,
) -> AppResult<()> {
    let cfg_path = config_path(codex_home_override)?;
    let catalog_path = model_catalog_path(codex_home_override)?;

    if cfg_path.exists() {
        if let Some(provider_id) = &binding.provider_id {
            let text = fs::read_to_string(&cfg_path)?;
            if let Ok(mut doc) = text.parse::<DocumentMut>() {
                if let Some(providers) = doc["model_providers"].as_table_mut() {
                    providers.remove(provider_id);
                }
                if doc.get("model_provider").and_then(|i| i.as_str()) == Some(provider_id.as_str())
                {
                    doc.as_table_mut().remove("model_provider");
                    doc.as_table_mut().remove("model");
                    if binding
                        .expected_fields
                        .contains_key("model_reasoning_effort")
                    {
                        doc.as_table_mut().remove("model_reasoning_effort");
                    }
                }
                if let Some(expected_catalog) = binding.expected_fields.get("model_catalog_json") {
                    if doc.get("model_catalog_json").and_then(|i| i.as_str())
                        == Some(expected_catalog.as_str())
                    {
                        doc.as_table_mut().remove("model_catalog_json");
                    }
                }
                revert_managed_capability_fields(&mut doc, &binding.expected_fields);
                atomic_write(&cfg_path, doc.to_string().as_bytes(), false)?;
            }
        }
    }

    // Remove catalog if we own it
    if let Some(expected_catalog) = binding.expected_fields.get("model_catalog_json") {
        if catalog_path.display().to_string() == *expected_catalog && catalog_path.exists() {
            let _ = fs::remove_file(&catalog_path);
        }
    }

    let env_path = codex_env_path()?;
    if env_path.exists() {
        let mut lines = read_env_file(&env_path)?;
        for k in &binding.managed_env_keys {
            remove_env_key(&mut lines, k);
        }
        write_env_file(&env_path, &lines)?;
    }
    Ok(())
}

fn is_managed_provider(id: &str, binding: Option<&TargetBinding>) -> bool {
    id.starts_with("xiaobai_") || binding.and_then(|b| b.provider_id.as_deref()) == Some(id)
}

fn prune_empty_table(doc: &mut DocumentMut, key: &str) {
    let empty = doc
        .get(key)
        .and_then(|i| i.as_table())
        .map(|t| t.is_empty())
        .unwrap_or(false);
    if empty {
        doc.as_table_mut().remove(key);
    }
}

fn collect_provider_env_keys(doc: &DocumentMut, binding: Option<&TargetBinding>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(providers) = doc.get("model_providers").and_then(|v| v.as_table()) {
        for (id, item) in providers.iter() {
            if !is_managed_provider(id, binding) {
                continue;
            }
            if let Some(env_key) = item.get("env_key").and_then(|i| i.as_str()) {
                if !env_key.is_empty() && !keys.iter().any(|k| k == env_key) {
                    keys.push(env_key.to_string());
                }
            }
        }
    }
    if let Some(b) = binding {
        for key in &b.managed_env_keys {
            if !keys.iter().any(|k| k == key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

/// Strip custom relay providers and injected keys so Codex uses the built-in
/// `openai` provider (ChatGPT login / `auth.json`). Leaves MCP, sandbox,
/// and official credentials untouched.
pub fn restore_official(
    binding: Option<&TargetBinding>,
    codex_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<crate::adapters::RestoreOfficialOutcome> {
    restore_official_at(
        &config_path(codex_home_override)?,
        &codex_env_path()?,
        &model_catalog_path(codex_home_override)?,
        binding,
        backup_root,
    )
}

fn restore_official_at(
    cfg_path: &std::path::Path,
    env_path: &std::path::Path,
    catalog_path: &std::path::Path,
    binding: Option<&TargetBinding>,
    backup_root: &PathBuf,
) -> AppResult<crate::adapters::RestoreOfficialOutcome> {
    let mut backup_paths = Vec::new();
    let mut env_keys = Vec::new();

    if cfg_path.exists() {
        let bak = backup_file(cfg_path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        let text = fs::read_to_string(cfg_path)?;
        let mut doc = text
            .parse::<DocumentMut>()
            .map_err(|e| AppError::new("invalid_config", format!("invalid config.toml: {e}")))?;

        env_keys.extend(collect_provider_env_keys(&doc, binding));
        strip_official_config(&mut doc, binding);

        if let Err(e) = atomic_write(cfg_path, doc.to_string().as_bytes(), false) {
            let _ = restore_file(&bak, cfg_path);
            return Err(e);
        }
    }

    if catalog_path.exists() {
        let bak = backup_file(catalog_path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        let _ = fs::remove_file(catalog_path);
    }

    if env_path.exists() {
        let bak = backup_file(env_path, backup_root)?;
        backup_paths.push(bak.display().to_string());
        let mut lines = read_env_file(env_path)?;
        for key in list_defined_keys(&lines) {
            if key.starts_with("XIAOBAI_") && !env_keys.iter().any(|k| k == &key) {
                env_keys.push(key);
            }
        }
        for key in &env_keys {
            remove_env_key(&mut lines, key);
        }
        if let Err(e) = write_env_file(env_path, &lines) {
            let _ = restore_file(&bak, env_path);
            return Err(e);
        }
    }

    Ok(crate::adapters::RestoreOfficialOutcome {
        backup_paths,
        env_keys,
    })
}

fn strip_official_config(doc: &mut DocumentMut, binding: Option<&TargetBinding>) {
    let mut removed_relay = binding.is_some();

    let live_provider = doc
        .get("model_provider")
        .and_then(|i| i.as_str())
        .map(|s| s.to_string());
    let mut provider_ids = Vec::new();
    if let Some(providers) = doc.get("model_providers").and_then(|v| v.as_table()) {
        for (id, _) in providers.iter() {
            if is_managed_provider(id, binding) {
                provider_ids.push(id.to_string());
            }
        }
    }
    if !provider_ids.is_empty() {
        removed_relay = true;
    }
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|v| v.as_table_mut())
    {
        for id in &provider_ids {
            providers.remove(id);
        }
    }
    prune_empty_table(doc, "model_providers");

    let provider_was_relay = live_provider
        .as_deref()
        .map(|p| is_managed_provider(p, binding))
        .unwrap_or(false);
    if provider_was_relay {
        removed_relay = true;
        doc.as_table_mut().remove("model_provider");
        doc.as_table_mut().remove("model");
        if binding
            .map(|b| b.expected_fields.contains_key("model_reasoning_effort"))
            .unwrap_or(true)
        {
            doc.as_table_mut().remove("model_reasoning_effort");
        }
    }

    if doc.get("openai_base_url").is_some() {
        removed_relay = true;
        doc.as_table_mut().remove("openai_base_url");
    }

    let expected_catalog = binding.and_then(|b| b.expected_fields.get("model_catalog_json"));
    let live_catalog = doc
        .get("model_catalog_json")
        .and_then(|i| i.as_str())
        .map(|s| s.to_string());
    let catalog_is_ours = live_catalog
        .as_deref()
        .map(|p| {
            p.ends_with("xiaobai-model-catalog.json")
                || expected_catalog.map(|e| e.as_str()) == Some(p)
        })
        .unwrap_or(false);
    if catalog_is_ours {
        doc.as_table_mut().remove("model_catalog_json");
    }

    if removed_relay {
        doc.as_table_mut().remove("web_search");
        if let Some(tools) = doc.get_mut("tools").and_then(|i| i.as_table_like_mut()) {
            tools.remove("view_image");
        }
        prune_empty_table(doc, "tools");
        if let Some(features) = doc.get_mut("features").and_then(|i| i.as_table_like_mut()) {
            features.remove("image_generation");
        }
        prune_empty_table(doc, "features");
    }
}

pub fn summary_from_config(doc: &DocumentMut) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    if let Some(m) = doc.get("model").and_then(|i| i.as_str()) {
        out.insert("model".into(), Some(m.into()));
    }
    if let Some(p) = doc.get("model_provider").and_then(|i| i.as_str()) {
        out.insert("model_provider".into(), Some(p.into()));
    }
    if let Some(e) = doc.get("model_reasoning_effort").and_then(|i| i.as_str()) {
        out.insert("model_reasoning_effort".into(), Some(e.into()));
    }
    if let Some(c) = doc.get("model_catalog_json").and_then(|i| i.as_str()) {
        out.insert("model_catalog_json".into(), Some(c.into()));
    }
    if let Some(provider_id) = doc.get("model_provider").and_then(|i| i.as_str()) {
        if let Some(table) = doc
            .get("model_providers")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get(provider_id))
            .and_then(|v| v.as_table())
        {
            if let Some(url) = table.get("base_url").and_then(|i| i.as_str()) {
                out.insert("base_url".into(), Some(url.into()));
            }
            if let Some(env_key) = table.get("env_key").and_then(|i| i.as_str()) {
                out.insert("env_key".into(), Some(env_key.into()));
            }
            if let Some(name) = table.get("name").and_then(|i| i.as_str()) {
                out.insert("provider_display_name".into(), Some(name.into()));
                out.insert(
                    "remote_compaction".into(),
                    Some(
                        if name == REMOTE_COMPACTION_PROVIDER_NAME {
                            "on"
                        } else {
                            "off"
                        }
                        .into(),
                    ),
                );
            }
        }
    }
    if let Some(search) = doc.get("web_search").and_then(|i| i.as_str()) {
        out.insert("web_search".into(), Some(search.into()));
    }
    if let Some(view_image) = doc
        .get("tools")
        .and_then(|v| v.get("view_image"))
        .and_then(|i| i.as_bool())
    {
        out.insert("tools_view_image".into(), Some(flag_str(view_image).into()));
    }
    if let Some(image_gen) = doc
        .get("features")
        .and_then(|v| v.get("image_generation"))
        .and_then(|i| i.as_bool())
    {
        out.insert(
            "features_image_generation".into(),
            Some(flag_str(image_gen).into()),
        );
    }
    out
}

pub fn live_summary(
    codex_home_override: Option<&str>,
) -> AppResult<HashMap<String, Option<String>>> {
    let cfg_path = config_path(codex_home_override)?;
    if !cfg_path.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(&cfg_path)?;
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return Ok(HashMap::new());
    };
    Ok(summary_from_config(&doc))
}

pub fn detect_status(
    binding: Option<&TargetBinding>,
    site: Option<&SiteRow>,
    api_key: Option<&str>,
    codex_home_override: Option<&str>,
) -> AppResult<(ApplyStatus, Option<String>)> {
    let cfg_path = config_path(codex_home_override)?;
    let has_trace = if cfg_path.exists() {
        let text = fs::read_to_string(&cfg_path).unwrap_or_default();
        text.contains("xiaobai_")
    } else {
        false
    };

    if let Some(b) = binding {
        if b.orphan || b.site_id.is_none() {
            return Ok((ApplyStatus::Orphan, Some("site deleted".into())));
        }
        if let Some(key) = api_key {
            if key_fingerprint(key) != b.key_fingerprint {
                return Ok((ApplyStatus::Stale, Some("API key changed".into())));
            }
        }
        if let Some(site) = site {
            let expected_provider = provider_id_for_site(&site.id);
            if b.provider_id.as_deref() != Some(expected_provider.as_str()) {
                return Ok((ApplyStatus::Stale, Some("provider changed".into())));
            }
        }
        if cfg_path.exists() {
            let text = fs::read_to_string(&cfg_path)?;
            if let Ok(doc) = text.parse::<DocumentMut>() {
                let live_p = doc.get("model_provider").and_then(|i| i.as_str());
                if live_p != b.provider_id.as_deref() {
                    return Ok((ApplyStatus::Stale, Some("model_provider mismatch".into())));
                }
                return Ok((ApplyStatus::Applied, None));
            }
        }
        return Ok((ApplyStatus::Stale, Some("config missing".into())));
    }

    if has_trace {
        return Ok((
            ApplyStatus::Orphan,
            Some("untracked xiaobai provider".into()),
        ));
    }
    Ok((ApplyStatus::NotApplied, None))
}

pub fn rewrite_base_url(
    site: &SiteRow,
    binding: &TargetBinding,
    codex_home_override: Option<&str>,
    backup_root: &PathBuf,
) -> AppResult<crate::adapters::RewriteOutcome> {
    let cfg_path = config_path(codex_home_override)?;
    if !cfg_path.exists() {
        return Err(AppError::new("invalid_config", "Codex config.toml missing"));
    }
    let provider_id = binding
        .provider_id
        .clone()
        .unwrap_or_else(|| provider_id_for_site(&site.id));
    let preview = crate::url_normalize::normalize_base_url(&site.base_url)?;
    let bak = backup_file(&cfg_path, backup_root)?;
    let text = fs::read_to_string(&cfg_path)?;
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::new("invalid_config", e.to_string()))?;
    {
        let providers = doc["model_providers"]
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| AppError::new("invalid_config", "model_providers must be table"))?;
        let table = providers
            .entry(&provider_id)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| AppError::new("invalid_config", "provider table"))?;
        table["base_url"] = value(&preview.codex_base_url);
    }
    if let Err(e) = atomic_write(&cfg_path, doc.to_string().as_bytes(), false) {
        let _ = restore_file(&PathBuf::from(&bak), &cfg_path);
        return Err(e);
    }
    let mut expected = binding.expected_fields.clone();
    expected.insert("base_url".into(), preview.codex_base_url.clone());
    let mut live_summary = HashMap::new();
    live_summary.insert("base_url".into(), Some(preview.codex_base_url.clone()));
    Ok(crate::adapters::RewriteOutcome {
        backup_paths: vec![bak.display().to_string()],
        live_summary,
        expected_fields: expected,
        message: "Updated Codex provider base_url".into(),
    })
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;

    #[test]
    fn rewrite_updates_only_provider_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let provider = provider_id_for_site("s1");
        fs::write(
            dir.path().join("config.toml"),
            format!(
                r#"model = "gpt-4"
model_provider = "{provider}"

[model_providers.{provider}]
name = "T"
base_url = "https://old.example.com/v1"
env_key = "XIAOBAI_SITE_S1_API_KEY"
wire_api = "responses"
"#
            ),
        )
        .unwrap();
        let mut expected = HashMap::new();
        expected.insert("base_url".into(), "https://old.example.com/v1".into());
        expected.insert("model".into(), "gpt-4".into());
        let binding = TargetBinding {
            target: TargetKind::Codex,
            site_id: Some("s1".into()),
            site_name_snapshot: "T".into(),
            model_id: "gpt-4".into(),
            provider_id: Some(provider.clone()),
            key_fingerprint: "x".into(),
            managed_paths: vec![],
            managed_env_keys: vec![],
            expected_fields: expected,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        };
        let site = SiteRow {
            id: "s1".into(),
            name: "T".into(),
            base_url: "https://new.example.com".into(),
            base_urls: vec!["https://new.example.com".into()],
            api_key_encrypted: "x".into(),
            key_prefix: "sk-xx".into(),
            protocol: crate::domain::SiteProtocol::OpenaiCompatible,
            claude_auth_key_style: crate::domain::ClaudeAuthKeyStyle::AnthropicAuthToken,
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
        };
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        let out =
            rewrite_base_url(&site, &binding, Some(dir.path().to_str().unwrap()), &bak).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("https://new.example.com/v1"));
        assert!(text.contains("gpt-4"));
        assert!(text.contains("XIAOBAI_SITE_S1_API_KEY"));
        assert_eq!(
            out.expected_fields.get("base_url").map(String::as_str),
            Some("https://new.example.com/v1")
        );
    }

    #[test]
    fn rewrite_preserves_capability_fields() {
        let dir = tempfile::tempdir().unwrap();
        let provider = provider_id_for_site("s1");
        fs::write(
            dir.path().join("config.toml"),
            format!(
                r#"model = "gpt-4"
model_provider = "{provider}"
web_search = "disabled"

[model_providers.{provider}]
name = "OpenAI"
base_url = "https://old.example.com/v1"
env_key = "XIAOBAI_SITE_S1_API_KEY"
wire_api = "responses"

[tools]
view_image = true

[features]
image_generation = false
"#
            ),
        )
        .unwrap();
        let mut expected = HashMap::new();
        expected.insert("base_url".into(), "https://old.example.com/v1".into());
        let binding = TargetBinding {
            target: TargetKind::Codex,
            site_id: Some("s1".into()),
            site_name_snapshot: "T".into(),
            model_id: "gpt-4".into(),
            provider_id: Some(provider.clone()),
            key_fingerprint: "x".into(),
            managed_paths: vec![],
            managed_env_keys: vec![],
            expected_fields: expected,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        };
        let site = SiteRow {
            id: "s1".into(),
            name: "T".into(),
            base_url: "https://new.example.com".into(),
            base_urls: vec!["https://new.example.com".into()],
            api_key_encrypted: "x".into(),
            key_prefix: "sk-xx".into(),
            protocol: crate::domain::SiteProtocol::OpenaiCompatible,
            claude_auth_key_style: crate::domain::ClaudeAuthKeyStyle::AnthropicAuthToken,
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
        };
        let bak = dir.path().join("bak");
        fs::create_dir_all(&bak).unwrap();
        rewrite_base_url(&site, &binding, Some(dir.path().to_str().unwrap()), &bak).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("https://new.example.com/v1"));
        assert!(text.contains("name = \"OpenAI\""));
        assert!(text.contains("web_search = \"disabled\""));
        assert!(text.contains("view_image = true"));
        assert!(text.contains("image_generation = false"));
    }
}

#[cfg(test)]
mod catalog_backup_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn collects_original_catalog_and_ours() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let original = home.join("models_catalog.json");
        fs::write(&original, r#"{"models":[{"slug":"old"}]}"#).unwrap();
        let ours = home.join("xiaobai-model-catalog.json");
        fs::write(&ours, r#"{"models":[]}"#).unwrap();
        let mut doc = DocumentMut::new();
        doc["model_catalog_json"] = value(original.display().to_string());
        let paths = catalogs_to_backup(&doc, &ours, home);
        assert!(paths.contains(&original));
        assert!(paths.contains(&ours));
    }

    #[test]
    fn resolves_relative_catalog_under_codex_home() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let original = home.join("custom-models.json");
        fs::write(&original, "{}").unwrap();
        let ours = home.join("xiaobai-model-catalog.json");
        let mut doc = DocumentMut::new();
        doc["model_catalog_json"] = value("custom-models.json");
        let paths = catalogs_to_backup(&doc, &ours, home);
        assert_eq!(paths, vec![original]);
    }

    #[test]
    fn backups_original_catalog_into_dir() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let original = home.join("models_catalog.json");
        fs::write(&original, r#"{"models":[{"slug":"kept"}]}"#).unwrap();
        let ours = home.join("xiaobai-model-catalog.json");
        let mut doc = DocumentMut::new();
        doc["model_catalog_json"] = value(original.display().to_string());
        let backup_root = home.join("bak");
        fs::create_dir_all(&backup_root).unwrap();
        for path in catalogs_to_backup(&doc, &ours, home) {
            backup_file(&path, &backup_root).unwrap();
        }
        let copied = backup_root.join("models_catalog.json");
        assert!(copied.exists());
        assert!(fs::read_to_string(copied).unwrap().contains("kept"));
    }
}

#[cfg(test)]
mod capability_write_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remote_compaction_writes_openai_display_name() {
        let mut table = toml_edit::Table::new();
        apply_provider_display_name(&mut table, "Relay", true);
        assert_eq!(table["name"].as_str(), Some("OpenAI"));
    }

    #[test]
    fn remote_compaction_off_uses_site_name() {
        let mut table = toml_edit::Table::new();
        apply_provider_display_name(&mut table, "Relay", false);
        assert_eq!(table["name"].as_str(), Some("Relay"));
    }

    #[test]
    fn web_search_writes_disabled_and_cached() {
        let mut doc = DocumentMut::new();
        apply_web_search(&mut doc, false);
        assert_eq!(doc["web_search"].as_str(), Some("disabled"));
        apply_web_search(&mut doc, true);
        assert_eq!(doc["web_search"].as_str(), Some("cached"));
    }

    #[test]
    fn image_understanding_sets_view_image() {
        let mut doc = DocumentMut::new();
        apply_image_understanding(&mut doc, true);
        assert_eq!(doc["tools"]["view_image"].as_bool(), Some(true));
        apply_image_understanding(&mut doc, false);
        assert_eq!(doc["tools"]["view_image"].as_bool(), Some(false));
    }

    #[test]
    fn image_generation_preserves_other_features() {
        let mut doc = DocumentMut::new();
        doc["features"]["fast_mode"] = value(true);
        apply_image_generation(&mut doc, false);
        assert_eq!(doc["features"]["image_generation"].as_bool(), Some(false));
        assert_eq!(doc["features"]["fast_mode"].as_bool(), Some(true));
        apply_image_generation(&mut doc, true);
        assert_eq!(doc["features"]["image_generation"].as_bool(), Some(true));
        assert_eq!(doc["features"]["fast_mode"].as_bool(), Some(true));
    }

    #[test]
    fn catalog_modalities_follow_understanding() {
        let off = build_model_catalog(&[("m".into(), "M".into())], "S", false);
        assert_eq!(off["models"][0]["input_modalities"], json!(["text"]));
        let on = build_model_catalog(&[("m".into(), "M".into())], "S", true);
        assert_eq!(off["models"][0]["input_modalities"], json!(["text"]));
        assert_eq!(
            on["models"][0]["input_modalities"],
            json!(["text", "image"])
        );
    }

    #[test]
    fn summary_reads_capability_fields() {
        let provider = provider_id_for_site("s1");
        let mut doc = DocumentMut::new();
        doc["model_provider"] = value(&provider);
        let providers = doc["model_providers"]
            .or_insert(toml_edit::table())
            .as_table_mut()
            .unwrap();
        let table = providers
            .entry(&provider)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .unwrap();
        apply_provider_display_name(table, "Relay", true);
        apply_web_search(&mut doc, false);
        apply_image_understanding(&mut doc, true);
        apply_image_generation(&mut doc, false);
        let sum = summary_from_config(&doc);
        assert_eq!(
            sum.get("remote_compaction").and_then(|v| v.as_deref()),
            Some("on")
        );
        assert_eq!(
            sum.get("provider_display_name").and_then(|v| v.as_deref()),
            Some("OpenAI")
        );
        assert_eq!(
            sum.get("web_search").and_then(|v| v.as_deref()),
            Some("disabled")
        );
        assert_eq!(
            sum.get("tools_view_image").and_then(|v| v.as_deref()),
            Some("true")
        );
        assert_eq!(
            sum.get("features_image_generation")
                .and_then(|v| v.as_deref()),
            Some("false")
        );
    }

    #[test]
    fn revert_removes_only_owned_capability_keys() {
        let mut doc = DocumentMut::new();
        apply_web_search(&mut doc, false);
        apply_image_understanding(&mut doc, true);
        apply_image_generation(&mut doc, false);
        doc["features"]["fast_mode"] = value(true);
        doc["web_search"] = value("live");
        let mut expected = HashMap::new();
        expected.insert("web_search".into(), "disabled".into());
        expected.insert("tools_view_image".into(), "true".into());
        expected.insert("features_image_generation".into(), "false".into());
        revert_managed_capability_fields(&mut doc, &expected);
        assert_eq!(doc["web_search"].as_str(), Some("live"));
        assert!(doc.get("tools").and_then(|t| t.get("view_image")).is_none());
        assert!(doc
            .get("features")
            .and_then(|t| t.get("image_generation"))
            .is_none());
        assert_eq!(doc["features"]["fast_mode"].as_bool(), Some(true));
    }
}

#[cfg(test)]
mod restore_official_tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_binding(provider: &str, env_key: &str) -> TargetBinding {
        let mut expected = HashMap::new();
        expected.insert("model".into(), "relay-model".into());
        expected.insert("model_provider".into(), provider.into());
        expected.insert("model_reasoning_effort".into(), "high".into());
        expected.insert(
            "model_catalog_json".into(),
            "/tmp/xiaobai-model-catalog.json".into(),
        );
        expected.insert("web_search".into(), "disabled".into());
        TargetBinding {
            target: TargetKind::Codex,
            site_id: Some("s1".into()),
            site_name_snapshot: "Relay".into(),
            model_id: "relay-model".into(),
            provider_id: Some(provider.into()),
            key_fingerprint: "x".into(),
            managed_paths: vec![],
            managed_env_keys: vec![env_key.into()],
            expected_fields: expected,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        }
    }

    #[test]
    fn restore_official_removes_relay_and_keeps_user_settings() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let provider = provider_id_for_site("s1");
        let env_key = env_key_for_site("s1");
        fs::write(
            home.join("config.toml"),
            format!(
                r#"model = "relay-model"
model_provider = "{provider}"
model_reasoning_effort = "high"
openai_base_url = "https://relay.example.com/v1"
web_search = "disabled"
model_catalog_json = "{catalog}"

[model_providers.{provider}]
name = "Relay"
base_url = "https://relay.example.com/v1"
env_key = "{env_key}"
wire_api = "responses"

[mcp_servers.foo]
command = "uvx"

[tools]
view_image = true

[features]
image_generation = false
fast_mode = true
"#,
                catalog = home.join("xiaobai-model-catalog.json").display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        fs::write(home.join("xiaobai-model-catalog.json"), r#"{"models":[]}"#).unwrap();
        let env_path = home.join("codex.env");
        fs::write(
            &env_path,
            format!("# header\nexport {env_key}=\"sk-relay\"\nexport KEEP_ME=\"1\"\n"),
        )
        .unwrap();
        let bak = home.join("bak");
        fs::create_dir_all(&bak).unwrap();
        let binding = sample_binding(&provider, &env_key);
        let out = restore_official_at(
            &home.join("config.toml"),
            &env_path,
            &home.join("xiaobai-model-catalog.json"),
            Some(&binding),
            &bak,
        )
        .unwrap();
        let text = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!text.contains("relay-model"));
        assert!(!text.contains(&provider));
        assert!(!text.contains("openai_base_url"));
        assert!(!text.contains("model_catalog_json"));
        assert!(!text.contains("web_search"));
        assert!(!text.contains("view_image"));
        assert!(!text.contains("image_generation"));
        assert!(text.contains("command = \"uvx\""));
        assert!(text.contains("fast_mode = true"));
        assert!(!home.join("xiaobai-model-catalog.json").exists());
        let env_text = fs::read_to_string(&env_path).unwrap();
        assert!(!env_text.contains(&env_key));
        assert!(env_text.contains("KEEP_ME"));
        assert!(out.env_keys.contains(&env_key));
        assert!(!out.backup_paths.is_empty());
    }

    #[test]
    fn restore_official_without_binding_still_strips_xiaobai_providers() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let provider = provider_id_for_site("s1");
        fs::write(
            home.join("config.toml"),
            format!(
                r#"model = "relay-model"
model_provider = "{provider}"

[model_providers.{provider}]
name = "Relay"
base_url = "https://relay.example.com/v1"
env_key = "XIAOBAI_SITE_S1_API_KEY"
wire_api = "responses"
"#
            ),
        )
        .unwrap();
        let bak = home.join("bak");
        fs::create_dir_all(&bak).unwrap();
        restore_official_at(
            &home.join("config.toml"),
            &home.join("missing.env"),
            &home.join("missing-catalog.json"),
            None,
            &bak,
        )
        .unwrap();
        let text = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!text.contains(&provider));
        assert!(!text.contains("model_provider"));
        assert!(!text.contains("relay-model"));
    }

    #[test]
    fn restore_official_leaves_true_official_config_alone() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        fs::write(
            home.join("config.toml"),
            r#"model = "gpt-5.4"
model_provider = "openai"
web_search = "cached"

[mcp_servers.foo]
command = "uvx"
"#,
        )
        .unwrap();
        let bak = home.join("bak");
        fs::create_dir_all(&bak).unwrap();
        restore_official_at(
            &home.join("config.toml"),
            &home.join("missing.env"),
            &home.join("missing-catalog.json"),
            None,
            &bak,
        )
        .unwrap();
        let text = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("gpt-5.4"));
        assert!(text.contains("model_provider = \"openai\""));
        assert!(text.contains("web_search = \"cached\""));
        assert!(text.contains("command = \"uvx\""));
    }
}
