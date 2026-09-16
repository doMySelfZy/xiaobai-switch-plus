use crate::adapters::claude_code::{has_1m_suffix, strip_1m_suffix};
use crate::capabilities::{
    capability_on, CODEX_COMPACT, CODEX_IMAGEGEN, CODEX_SEARCH, CODEX_VISION,
};
use crate::domain::{
    ApplyStatus, ApplyTargetResult, BindingApiKeySnapshot, CapabilitySource, ClaudeApplyOptions,
    ClaudeAuthKeyStyle, ClaudeEffortLevel, CodexApplyOptions, CodexReasoningEffort,
    ModelFetchOutcome, PiApplyOptions, PrimeApplyOptions, SiteModelDto, SiteRow,
    SwitchSiteApiKeyResult, TargetBinding, TargetKind,
};
use crate::error::AppResult;
use crate::lock::{try_lock_site, try_lock_target};
use crate::paths::backups_dir;
use crate::redact;
use crate::repo;
use crate::state::AppState;
use chrono::Utc;
use std::collections::HashMap;
use std::fs;

pub async fn switch_site_api_key(
    state: &AppState,
    site_id: &str,
    api_key_id: &str,
    sync_targets: bool,
) -> AppResult<SwitchSiteApiKeyResult> {
    let (site, captured_key_id, api_key, settings, cached_models) = {
        let _site_lock = try_lock_site(site_id)?;
        state.db.with_conn(|c| {
            repo::site_api_key::get_for_site(c, site_id, api_key_id)?;
            let activated = repo::site_api_key::activate(c, site_id, api_key_id)?;
            let site = repo::site::get_site(c, site_id)?;
            let key = repo::site_api_key::decrypt(&state.crypto, &activated)?;
            let settings = repo::settings::get_settings(c)?;
            let models = repo::site::list_models_for_key(c, &activated.id)?;
            Ok((site, activated.id, key, settings, models))
        })?
    };

    let fetch = match crate::models_fetch::fetch_models(&site, &api_key, &settings).await {
        Ok(mut result) => {
            for model in &mut result.models {
                model.api_key_id = captured_key_id.clone();
                model.site_id = site_id.to_string();
            }
            let models = state.db.with_conn(|c| {
                let still_active = repo::site_api_key::get_active(c, site_id)?;
                if still_active.id != captured_key_id {
                    return Ok(repo::site::list_models_for_key(c, &still_active.id)?);
                }
                repo::site::replace_models(c, site_id, &captured_key_id, &result.models)?;
                repo::site::update_fetch_meta_for_key(
                    c,
                    &captured_key_id,
                    result.latency_ms as i64,
                    None,
                )?;
                repo::site::list_models_for_key(c, &captured_key_id)
            })?;
            (
                true,
                ModelFetchOutcome {
                    ok: true,
                    api_key_id: captured_key_id.clone(),
                    latency_ms: result.latency_ms,
                    endpoint: Some(result.endpoint),
                    fetched_at: Some(result.fetched_at),
                    error: None,
                },
                models,
            )
        }
        Err(e) => {
            let msg = redact::api_key(&e.to_string());
            let _ = state.db.with_conn(|c| {
                repo::site::update_fetch_meta_for_key(c, &captured_key_id, 0, Some(&msg))
            });
            (
                false,
                ModelFetchOutcome {
                    ok: false,
                    api_key_id: captured_key_id.clone(),
                    latency_ms: 0,
                    endpoint: None,
                    fetched_at: None,
                    error: Some(msg),
                },
                cached_models,
            )
        }
    };

    let (fetch_ok, fetch_outcome, models) = fetch;
    let results = if sync_targets && fetch_ok {
        let _site_lock = try_lock_site(site_id)?;
        let site = state.db.with_conn(|c| {
            let active = repo::site_api_key::get_active(c, site_id)?;
            if active.id != captured_key_id {
                return Err(crate::error::AppError::new(
                    "validation_failed",
                    "api key is not the site's current key",
                ));
            }
            repo::site::get_site(c, site_id)
        })?;
        sync_applied_keys(state, &site, &models)?
    } else {
        Vec::new()
    };

    let site = state.db.with_conn(|c| repo::site::get_site(c, site_id))?;
    Ok(SwitchSiteApiKeyResult {
        site: site.to_dto(),
        models,
        fetch: fetch_outcome,
        results,
    })
}

fn model_ids(models: &[SiteModelDto]) -> std::collections::HashSet<String> {
    models.iter().map(|m| m.model_id.clone()).collect()
}

fn supported(ids: &std::collections::HashSet<String>, model_id: &str) -> bool {
    let trimmed = model_id.trim();
    ids.contains(trimmed) || ids.contains(&strip_1m_suffix(trimmed))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeApplyValues {
    model_id: String,
    auth: Option<ClaudeAuthKeyStyle>,
    fable_model_id: Option<String>,
    opus_model_id: Option<String>,
    sonnet_model_id: Option<String>,
    haiku_model_id: Option<String>,
    effort_level: Option<ClaudeEffortLevel>,
    use_1m_context: bool,
}

fn live_value(summary: Option<&HashMap<String, Option<String>>>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        summary
            .and_then(|values| values.get(*key))
            .and_then(|value| value.as_ref())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn binding_value(binding: &TargetBinding, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        binding
            .expected_fields
            .get(*key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn resolve_claude_values(
    binding: &TargetBinding,
    live: Option<&HashMap<String, Option<String>>>,
) -> ClaudeApplyValues {
    // Legacy env keys win in live settings because Claude Code gives them runtime precedence.
    let raw_model = live_value(live, &["ANTHROPIC_MODEL", "model"])
        .or_else(|| binding_value(binding, &["model", "ANTHROPIC_MODEL"]))
        .unwrap_or_else(|| binding.model_id.clone());
    let fable = live_value(live, &["ANTHROPIC_DEFAULT_FABLE_MODEL"])
        .or_else(|| binding_value(binding, &["ANTHROPIC_DEFAULT_FABLE_MODEL"]));
    let opus = live_value(live, &["ANTHROPIC_DEFAULT_OPUS_MODEL"])
        .or_else(|| binding_value(binding, &["ANTHROPIC_DEFAULT_OPUS_MODEL"]));
    let sonnet = live_value(live, &["ANTHROPIC_DEFAULT_SONNET_MODEL"])
        .or_else(|| binding_value(binding, &["ANTHROPIC_DEFAULT_SONNET_MODEL"]));
    let haiku = live_value(live, &["ANTHROPIC_DEFAULT_HAIKU_MODEL"])
        .or_else(|| binding_value(binding, &["ANTHROPIC_DEFAULT_HAIKU_MODEL"]));
    let effort = live_value(live, &["CLAUDE_CODE_EFFORT_LEVEL", "effortLevel"])
        .or_else(|| binding_value(binding, &["effortLevel", "CLAUDE_CODE_EFFORT_LEVEL"]))
        .and_then(|value| ClaudeEffortLevel::parse(&value));
    let auth = if live_value(live, &["ANTHROPIC_AUTH_TOKEN"]).is_some() {
        Some(ClaudeAuthKeyStyle::AnthropicAuthToken)
    } else if live_value(live, &["ANTHROPIC_API_KEY"]).is_some() {
        Some(ClaudeAuthKeyStyle::AnthropicApiKey)
    } else {
        match binding_value(binding, &["auth_env_key"]).as_deref() {
            Some("ANTHROPIC_AUTH_TOKEN") => Some(ClaudeAuthKeyStyle::AnthropicAuthToken),
            Some("ANTHROPIC_API_KEY") => Some(ClaudeAuthKeyStyle::AnthropicApiKey),
            _ => None,
        }
    };
    let use_1m_context = [
        &raw_model,
        opus.as_deref().unwrap_or(""),
        sonnet.as_deref().unwrap_or(""),
    ]
    .into_iter()
    .any(|model| has_1m_suffix(model));

    ClaudeApplyValues {
        model_id: strip_1m_suffix(&raw_model),
        auth,
        fable_model_id: fable.map(|model| strip_1m_suffix(&model)),
        opus_model_id: opus.map(|model| strip_1m_suffix(&model)),
        sonnet_model_id: sonnet.map(|model| strip_1m_suffix(&model)),
        haiku_model_id: haiku.map(|model| strip_1m_suffix(&model)),
        effort_level: effort,
        use_1m_context,
    }
}

fn skip_result(target: TargetKind, message: &str) -> ApplyTargetResult {
    ApplyTargetResult {
        target,
        ok: false,
        status: ApplyStatus::Stale,
        backup_paths: vec![],
        message: message.into(),
        live_summary: None,
        touched_keys: None,
    }
}

pub fn sync_applied_keys(
    state: &AppState,
    site: &SiteRow,
    models: &[SiteModelDto],
) -> AppResult<Vec<ApplyTargetResult>> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let bindings = state
        .db
        .with_conn(|c| repo::binding::list_bindings_for_site(c, &site.id))?;
    let api_key = state.crypto.decrypt(&site.api_key_encrypted)?;
    let snapshot = site.api_key_snapshot();
    let pi_thinking = state
        .db
        .with_conn(|c| repo::thinking::get_write(c, &site.id, TargetKind::Pi))?;
    let prime_thinking = state
        .db
        .with_conn(|c| repo::thinking::get_write(c, &site.id, TargetKind::Prime))?;
    let ids = model_ids(models);
    let applied_at = Utc::now().timestamp_millis();
    let mut results = Vec::new();

    for binding in bindings {
        if binding.orphan {
            continue;
        }
        let target = binding.target;
        let claude_values = if target == TargetKind::ClaudeCode {
            match crate::adapters::claude_code::live_summary(
                settings.claude_home_override.as_deref(),
            ) {
                Ok(live) => Some(resolve_claude_values(&binding, Some(&live))),
                Err(error) => {
                    results.push(ApplyTargetResult {
                        target,
                        ok: false,
                        status: ApplyStatus::Failed,
                        backup_paths: vec![],
                        message: error.to_string(),
                        live_summary: None,
                        touched_keys: None,
                    });
                    continue;
                }
            }
        } else {
            None
        };
        if let Some(skipped) = skip_if_incompatible(target, &binding, &ids, claude_values.as_ref())
        {
            results.push(skipped);
            continue;
        }
        let _lock = try_lock_target(target.as_str())?;
        let backup_model_id = claude_values
            .as_ref()
            .map(|values| values.model_id.clone())
            .unwrap_or_else(|| binding.model_id.clone());
        let backup_root = backups_dir()?
            .join(target.as_str())
            .join(format!("{}", applied_at));
        fs::create_dir_all(&backup_root)?;

        // 接管开启时把可写地址换成代理地址；数据库与后续状态检测仍用真实上游。
        let effective_site = crate::local_proxy::routing::effective_site(site, target, &settings);

        let rewrite = match target {
            TargetKind::ClaudeCode => apply_claude(
                &effective_site,
                &api_key,
                &binding,
                claude_values
                    .as_ref()
                    .expect("Claude values resolved above"),
                &settings,
                &backup_root,
            ),
            TargetKind::Codex => apply_codex(
                &effective_site,
                &api_key,
                &binding,
                models,
                &settings,
                &backup_root,
            ),
            TargetKind::Pi => apply_pi(
                &effective_site,
                &api_key,
                &binding,
                models,
                &settings,
                &backup_root,
                &pi_thinking,
            ),
            TargetKind::Prime => apply_prime(
                &effective_site,
                &api_key,
                &binding,
                models,
                &settings,
                &backup_root,
                &prime_thinking,
            ),
            TargetKind::ZCode => apply_zcode(
                &effective_site,
                &api_key,
                &binding,
                models,
                &settings,
                &backup_root,
            ),
        };

        match rewrite {
            Ok((mut next, backup_paths, message, live_summary, touched)) => {
                next.api_key = snapshot.clone();
                next.applied_at = applied_at;
                state
                    .db
                    .with_conn(|c| repo::binding::upsert_binding(c, &next))?;
                results.push(ApplyTargetResult {
                    target,
                    ok: true,
                    status: ApplyStatus::Applied,
                    backup_paths,
                    message,
                    live_summary: Some(live_summary),
                    touched_keys: Some(touched),
                });
            }
            Err(e) => {
                results.push(ApplyTargetResult {
                    target,
                    ok: false,
                    status: ApplyStatus::Failed,
                    backup_paths: vec![],
                    message: e.to_string(),
                    live_summary: None,
                    touched_keys: None,
                });
            }
        }

        crate::commands::apply::finalize_backup_dir(
            &backup_root,
            target,
            &site.name,
            &backup_model_id,
            None,
            applied_at,
            settings.max_backup_copies,
            &mut std::collections::HashSet::new(),
        );
    }

    Ok(results)
}

fn skip_if_incompatible(
    target: TargetKind,
    binding: &TargetBinding,
    ids: &std::collections::HashSet<String>,
    claude_values: Option<&ClaudeApplyValues>,
) -> Option<ApplyTargetResult> {
    let model_id = claude_values
        .map(|values| values.model_id.as_str())
        .unwrap_or(binding.model_id.as_str());
    if !supported(ids, model_id) {
        return Some(skip_result(
            target,
            "current model is not available on the new API key",
        ));
    }
    if target == TargetKind::ClaudeCode {
        let fallback;
        let aliases = if let Some(values) = claude_values {
            [
                values.fable_model_id.as_deref(),
                values.opus_model_id.as_deref(),
                values.sonnet_model_id.as_deref(),
                values.haiku_model_id.as_deref(),
            ]
        } else {
            fallback = resolve_claude_values(binding, None);
            [
                fallback.fable_model_id.as_deref(),
                fallback.opus_model_id.as_deref(),
                fallback.sonnet_model_id.as_deref(),
                fallback.haiku_model_id.as_deref(),
            ]
        };
        for model in aliases.into_iter().flatten() {
            if !supported(ids, model) {
                return Some(skip_result(
                    target,
                    "Claude model alias is not available on the new API key",
                ));
            }
        }
    }
    None
}

fn apply_claude(
    site: &SiteRow,
    api_key: &str,
    binding: &TargetBinding,
    values: &ClaudeApplyValues,
    settings: &crate::domain::AppSettings,
    backup_root: &std::path::PathBuf,
) -> AppResult<(
    TargetBinding,
    Vec<String>,
    String,
    std::collections::HashMap<String, Option<String>>,
    Vec<String>,
)> {
    let auth = values
        .auth
        .clone()
        .unwrap_or_else(|| site.claude_auth_key_style.clone());
    let options = ClaudeApplyOptions {
        fable_model_id: values.fable_model_id.clone(),
        opus_model_id: values.opus_model_id.clone(),
        sonnet_model_id: values.sonnet_model_id.clone(),
        haiku_model_id: values.haiku_model_id.clone(),
        effort_level: values.effort_level.clone(),
        use_1m_context: values.use_1m_context,
    };
    let outcome = crate::adapters::claude_code::apply(
        site,
        api_key,
        &values.model_id,
        auth,
        settings.force_exclusive_claude_auth_key,
        &options,
        Some(binding),
        settings.claude_home_override.as_deref(),
        backup_root,
    )?;
    Ok((
        outcome.binding,
        outcome.backup_paths,
        outcome.message,
        outcome.live_summary,
        outcome.touched.paths,
    ))
}

fn apply_codex(
    site: &SiteRow,
    api_key: &str,
    binding: &TargetBinding,
    models: &[SiteModelDto],
    settings: &crate::domain::AppSettings,
    backup_root: &std::path::PathBuf,
) -> AppResult<(
    TargetBinding,
    Vec<String>,
    String,
    std::collections::HashMap<String, Option<String>>,
    Vec<String>,
)> {
    let write_all = binding.expected_fields.contains_key("model_catalog_json");
    let catalog_models = if write_all {
        models
            .iter()
            .map(|m| (m.model_id.clone(), m.display_name.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let options = CodexApplyOptions {
        write_all_models: write_all,
        reasoning_effort: binding
            .expected_fields
            .get("model_reasoning_effort")
            .and_then(|s| CodexReasoningEffort::parse(s)),
        catalog_models,
        remote_compaction: binding
            .expected_fields
            .get("remote_compaction")
            .map(|s| s == "enabled")
            .unwrap_or_else(|| capability_on(&site.capabilities, CODEX_COMPACT)),
        image_understanding: binding
            .expected_fields
            .get("image_understanding")
            .map(|s| s == "enabled")
            .unwrap_or_else(|| capability_on(&site.capabilities, CODEX_VISION)),
        image_generation: binding
            .expected_fields
            .get("image_generation")
            .map(|s| s == "enabled")
            .unwrap_or_else(|| capability_on(&site.capabilities, CODEX_IMAGEGEN)),
        web_search: binding
            .expected_fields
            .get("web_search")
            .map(|s| s == "enabled")
            .unwrap_or_else(|| capability_on(&site.capabilities, CODEX_SEARCH)),
        capability_source: if binding.expected_fields.contains_key("web_search") {
            CapabilitySource::Custom
        } else {
            CapabilitySource::Site
        },
    };
    let outcome = crate::adapters::codex::apply(
        site,
        api_key,
        &binding.model_id,
        &options,
        settings.codex_home_override.as_deref(),
        backup_root,
    )?;
    let inject_msg = crate::env_inject::inject_codex_env(settings, &outcome.env_key, api_key)
        .unwrap_or_else(|e| e.to_string());
    Ok((
        outcome.binding,
        outcome.backup_paths,
        format!("{} {}", outcome.message, inject_msg),
        outcome.live_summary,
        outcome.touched.paths,
    ))
}

fn apply_pi(
    site: &SiteRow,
    api_key: &str,
    binding: &TargetBinding,
    models: &[SiteModelDto],
    settings: &crate::domain::AppSettings,
    backup_root: &std::path::Path,
    thinking: &crate::domain::ThinkingWrite,
) -> AppResult<(
    TargetBinding,
    Vec<String>,
    String,
    std::collections::HashMap<String, Option<String>>,
    Vec<String>,
)> {
    let write_all = binding
        .expected_fields
        .get("write_all_models")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let catalog_models = if write_all {
        models
            .iter()
            .map(|m| (m.model_id.clone(), m.display_name.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let options = PiApplyOptions {
        write_all_models: write_all,
        catalog_models,
        thinking: thinking.clone(),
    };
    let outcome = crate::adapters::pi::apply(
        site,
        api_key,
        &binding.model_id,
        &options,
        Some(binding),
        settings.pi_agent_dir_override.as_deref(),
        backup_root,
    )?;
    Ok((
        outcome.binding,
        outcome.backup_paths,
        outcome.message,
        outcome.live_summary,
        outcome.touched.paths,
    ))
}

fn apply_prime(
    site: &SiteRow,
    api_key: &str,
    binding: &TargetBinding,
    models: &[SiteModelDto],
    settings: &crate::domain::AppSettings,
    backup_root: &std::path::Path,
    thinking: &crate::domain::ThinkingWrite,
) -> AppResult<(
    TargetBinding,
    Vec<String>,
    String,
    std::collections::HashMap<String, Option<String>>,
    Vec<String>,
)> {
    let write_all = binding
        .expected_fields
        .get("write_all_models")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let catalog_models = if write_all {
        models
            .iter()
            .map(|m| (m.model_id.clone(), m.display_name.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let options = PrimeApplyOptions {
        write_all_models: write_all,
        catalog_models,
        thinking: thinking.clone(),
    };
    let outcome = crate::adapters::prime::apply(
        site,
        api_key,
        &binding.model_id,
        &options,
        Some(binding),
        settings.prime_agent_dir_override.as_deref(),
        backup_root,
    )?;
    Ok((
        outcome.binding,
        outcome.backup_paths,
        outcome.message,
        outcome.live_summary,
        outcome.touched.paths,
    ))
}

/// ZCode 的 key 就写在自己的两份配置文件里，没有独立的 auth 文件——重新应用一遍即可换 key。
fn apply_zcode(
    site: &SiteRow,
    api_key: &str,
    binding: &TargetBinding,
    models: &[SiteModelDto],
    settings: &crate::domain::AppSettings,
    backup_root: &std::path::Path,
) -> AppResult<(
    TargetBinding,
    Vec<String>,
    String,
    std::collections::HashMap<String, Option<String>>,
    Vec<String>,
)> {
    let write_all = binding
        .expected_fields
        .get("write_all_models")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let catalog: Vec<String> = if write_all {
        models.iter().map(|m| m.model_id.clone()).collect()
    } else {
        Vec::new()
    };
    let model_ids = crate::commands::apply::zcode_model_ids(&binding.model_id, catalog);
    let outcome = crate::adapters::zcode::apply(
        site,
        api_key,
        &model_ids,
        settings.zcode_home_override.as_deref(),
        backup_root,
    )?;
    let mut next = binding.clone();
    next.applied_at = Utc::now().timestamp_millis();
    Ok((
        next,
        outcome.backup_paths,
        outcome.message,
        crate::adapters::zcode::live_summary(settings.zcode_home_override.as_deref())?,
        Vec::new(),
    ))
}

pub fn stamp_binding(binding: &mut TargetBinding, snapshot: &BindingApiKeySnapshot) {
    binding.api_key = snapshot.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(expected_fields: HashMap<String, String>) -> TargetBinding {
        TargetBinding {
            target: TargetKind::ClaudeCode,
            site_id: Some("site-1".into()),
            site_name_snapshot: "Relay".into(),
            model_id: "raw-binding-model".into(),
            provider_id: None,
            key_fingerprint: "fingerprint".into(),
            managed_paths: vec![],
            managed_env_keys: vec![],
            expected_fields,
            orphan: false,
            applied_at: 1,
            apply_record_id: None,
            api_key: Default::default(),
        }
    }

    #[test]
    fn claude_values_prefer_live_runtime_values() {
        let binding = binding(HashMap::from([
            ("model".into(), "binding-model".into()),
            ("effortLevel".into(), "low".into()),
            ("auth_env_key".into(), "ANTHROPIC_AUTH_TOKEN".into()),
        ]));
        let live = HashMap::from([
            ("model".into(), Some("top-level-model".into())),
            ("ANTHROPIC_MODEL".into(), Some("live-model[1M]".into())),
            (
                "ANTHROPIC_DEFAULT_FABLE_MODEL".into(),
                Some("live-fable[1m]".into()),
            ),
            (
                "ANTHROPIC_DEFAULT_OPUS_MODEL".into(),
                Some("live-opus[1m]".into()),
            ),
            (
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                Some("live-sonnet".into()),
            ),
            (
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".into(),
                Some("live-haiku[1m]".into()),
            ),
            ("ANTHROPIC_API_KEY".into(), Some("sk-live".into())),
            ("effortLevel".into(), Some("low".into())),
            ("CLAUDE_CODE_EFFORT_LEVEL".into(), Some("high".into())),
        ]);

        let values = resolve_claude_values(&binding, Some(&live));

        assert_eq!(values.model_id, "live-model");
        assert_eq!(values.auth, Some(ClaudeAuthKeyStyle::AnthropicApiKey));
        assert_eq!(values.fable_model_id.as_deref(), Some("live-fable"));
        assert_eq!(values.opus_model_id.as_deref(), Some("live-opus"));
        assert_eq!(values.sonnet_model_id.as_deref(), Some("live-sonnet"));
        assert_eq!(values.haiku_model_id.as_deref(), Some("live-haiku"));
        assert_eq!(values.effort_level, Some(ClaudeEffortLevel::High));
        assert!(values.use_1m_context);
    }

    #[test]
    fn claude_values_fall_back_to_new_and_legacy_binding_fields() {
        let binding = binding(HashMap::from([
            ("model".into(), "binding-model[1m]".into()),
            (
                "ANTHROPIC_DEFAULT_FABLE_MODEL".into(),
                "binding-fable[1m]".into(),
            ),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), "binding-opus".into()),
            (
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                "binding-sonnet".into(),
            ),
            (
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".into(),
                "binding-haiku[1m]".into(),
            ),
            ("CLAUDE_CODE_EFFORT_LEVEL".into(), "max".into()),
            ("auth_env_key".into(), "ANTHROPIC_AUTH_TOKEN".into()),
        ]));

        let values = resolve_claude_values(&binding, None);

        assert_eq!(values.model_id, "binding-model");
        assert_eq!(values.auth, Some(ClaudeAuthKeyStyle::AnthropicAuthToken));
        assert_eq!(values.fable_model_id.as_deref(), Some("binding-fable"));
        assert_eq!(values.opus_model_id.as_deref(), Some("binding-opus"));
        assert_eq!(values.sonnet_model_id.as_deref(), Some("binding-sonnet"));
        assert_eq!(values.haiku_model_id.as_deref(), Some("binding-haiku"));
        assert_eq!(
            values.effort_level.as_ref().map(ClaudeEffortLevel::as_str),
            Some("xhigh")
        );
        assert!(values.use_1m_context);
    }

    #[test]
    fn compatibility_check_uses_resolved_live_models() {
        let binding = binding(HashMap::from([
            ("model".into(), "stale-binding-model".into()),
            (
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                "stale-binding-sonnet".into(),
            ),
        ]));
        let live = HashMap::from([
            ("model".into(), Some("live-model".into())),
            (
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                Some("live-sonnet".into()),
            ),
        ]);
        let values = resolve_claude_values(&binding, Some(&live));
        let ids = std::collections::HashSet::from(["live-model".into(), "live-sonnet".into()]);

        assert!(
            skip_if_incompatible(TargetKind::ClaudeCode, &binding, &ids, Some(&values)).is_none()
        );

        let ids = std::collections::HashSet::from(["live-model".into()]);
        assert!(
            skip_if_incompatible(TargetKind::ClaudeCode, &binding, &ids, Some(&values)).is_some()
        );
    }
}
