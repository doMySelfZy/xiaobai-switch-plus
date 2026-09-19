use crate::adapters::atomic::{atomic_write, backup_file};
use crate::adapters::{RestoreOfficialOutcome, RewriteOutcome};
use crate::crypto::{key_fingerprint, key_prefix};
use crate::domain::{
    provider_id_for_site, ApplyStatus, PiApplyOptions, SiteProtocol, SiteRow, TargetBinding,
    TargetKind, TouchedKeys,
};
use crate::error::{AppError, AppResult};
use crate::paths::{resolve_pi_agent_dir, set_secret_permissions};
use chrono::Utc;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

const BASELINE_MISSING: &str = "__xiaobai_missing__";
const BASELINE_PROVIDER: &str = "pi_baseline_default_provider";
const BASELINE_MODEL: &str = "pi_baseline_default_model";

#[derive(Debug)]
pub struct PiApplyOutcome {
    pub binding: TargetBinding,
    pub touched: TouchedKeys,
    pub backup_paths: Vec<String>,
    pub live_summary: HashMap<String, Option<String>>,
    pub message: String,
    pub provider_id: String,
}

pub fn models_path(override_path: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_pi_agent_dir(override_path)?.join("models.json"))
}

pub fn auth_path(override_path: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_pi_agent_dir(override_path)?.join("auth.json"))
}

pub fn settings_path(override_path: Option<&str>) -> AppResult<PathBuf> {
    Ok(resolve_pi_agent_dir(override_path)?.join("settings.json"))
}

fn parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: true,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

fn contains_block_comment(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
        } else if ch == '/' && chars.peek() == Some(&'*') {
            return true;
        }
    }
    false
}

fn parse_models(text: &str) -> AppResult<CstRootNode> {
    if contains_block_comment(text) {
        return Err(AppError::new(
            "invalid_config",
            "Pi models.json contains a block comment unsupported by Pi",
        ));
    }
    let root = CstRootNode::parse(text, &parse_options())
        .map_err(|e| AppError::new("invalid_config", format!("invalid Pi models.json: {e}")))?;
    if root.object_value().is_none() {
        return Err(AppError::new(
            "invalid_config",
            "Pi models.json root must be an object",
        ));
    }
    Ok(root)
}

fn read_models(path: &Path) -> AppResult<CstRootNode> {
    if path.exists() {
        parse_models(&fs::read_to_string(path)?)
    } else {
        parse_models("{\n  \"providers\": {}\n}\n")
    }
}

fn read_json_object(path: &Path, label: &str) -> AppResult<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?)
        .map_err(|e| AppError::new("invalid_config", format!("invalid Pi {label}: {e}")))?;
    if !value.is_object() {
        return Err(AppError::new(
            "invalid_config",
            format!("Pi {label} root must be an object"),
        ));
    }
    Ok(value)
}

fn protocol(site: &SiteRow) -> (&'static str, AppResult<String>) {
    let normalized = crate::url_normalize::normalize_base_url(&site.base_url);
    match site.protocol {
        SiteProtocol::OpenaiCompatible => (
            "openai-completions",
            normalized.map(|value| value.codex_base_url),
        ),
        SiteProtocol::Anthropic => (
            "anthropic-messages",
            normalized.map(|value| value.claude_base_url),
        ),
    }
}

fn model_values(
    options: &PiApplyOptions,
    selected: &str,
    protocol: SiteProtocol,
) -> Vec<CstInputValue> {
    let source = if options.write_all_models {
        options.catalog_models.clone()
    } else {
        Vec::new()
    };
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for (id, display_name) in source
        .into_iter()
        .chain(std::iter::once((selected.into(), selected.into())))
    {
        let id = id.trim().to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let display_name = display_name.trim();
        let mut fields = vec![
            ("id".into(), CstInputValue::String(id.clone())),
            (
                "input".into(),
                CstInputValue::Array(vec![
                    CstInputValue::String("text".into()),
                    CstInputValue::String("image".into()),
                ]),
            ),
        ];
        if !display_name.is_empty() && display_name != id {
            fields.push((
                "name".into(),
                CstInputValue::String(display_name.to_string()),
            ));
        }
        crate::adapters::thinking::push_model_thinking(
            &mut fields,
            &id,
            &options.thinking,
            protocol,
            TargetKind::Pi,
        );
        models.push(CstInputValue::Object(fields));
    }
    models
}

fn provider_value(
    name: &str,
    base_url: &str,
    api: &str,
    models: Vec<CstInputValue>,
) -> CstInputValue {
    CstInputValue::Object(vec![
        ("name".into(), CstInputValue::String(name.into())),
        ("baseUrl".into(), CstInputValue::String(base_url.into())),
        ("api".into(), CstInputValue::String(api.into())),
        ("models".into(), CstInputValue::Array(models)),
    ])
}

fn upsert_managed_provider(
    root: &CstRootNode,
    provider_id: &str,
    value: CstInputValue,
) -> AppResult<()> {
    let object = root
        .object_value()
        .ok_or_else(|| AppError::new("invalid_config", "Pi models.json root must be object"))?;
    let providers = object.object_value_or_create("providers").ok_or_else(|| {
        AppError::new("invalid_config", "Pi models.json providers must be object")
    })?;
    for prop in providers.properties() {
        let name = prop.name().and_then(|name| name.decoded_value().ok());
        if name
            .as_deref()
            .is_some_and(|name| name.starts_with("xiaobai_") && name != provider_id)
        {
            prop.remove();
        }
    }
    match providers.get(provider_id) {
        Some(prop) => prop.set_value(value),
        None => {
            providers.append(provider_id, value);
        }
    }
    Ok(())
}

fn remove_provider(root: &CstRootNode, provider_id: &str) -> AppResult<()> {
    let object = root
        .object_value()
        .ok_or_else(|| AppError::new("invalid_config", "Pi models.json root must be object"))?;
    if let Some(providers) = object.object_value("providers") {
        if let Some(prop) = providers.get(provider_id) {
            prop.remove();
        }
    }
    Ok(())
}

fn remove_all_managed_providers(root: &CstRootNode) -> AppResult<()> {
    let object = root
        .object_value()
        .ok_or_else(|| AppError::new("invalid_config", "Pi models.json root must be object"))?;
    if let Some(providers) = object.object_value("providers") {
        for prop in providers.properties() {
            let managed = prop
                .name()
                .and_then(|name| name.decoded_value().ok())
                .is_some_and(|name| name.starts_with("xiaobai_"));
            if managed {
                prop.remove();
            }
        }
    }
    Ok(())
}

pub fn encode_auth_literal(key: &str) -> String {
    let escaped = key.replace('$', "$$");
    if escaped.starts_with('!') {
        format!("${escaped}")
    } else {
        escaped
    }
}

pub fn decode_auth_literal(value: &str) -> String {
    let mut chars = value.chars().peekable();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            match chars.peek().copied() {
                Some('$') => {
                    chars.next();
                    output.push('$');
                }
                Some('!') => {
                    chars.next();
                    output.push('!');
                }
                _ => output.push('$'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn baseline_value(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| BASELINE_MISSING.into())
}

fn capture_baseline(settings: &Value, binding_before: Option<&TargetBinding>) -> (String, String) {
    if let Some(binding) = binding_before {
        if let (Some(provider), Some(model)) = (
            binding.expected_fields.get(BASELINE_PROVIDER),
            binding.expected_fields.get(BASELINE_MODEL),
        ) {
            return (provider.clone(), model.clone());
        }
    }
    (
        baseline_value(settings.get("defaultProvider")),
        baseline_value(settings.get("defaultModel")),
    )
}

fn restore_baseline(settings: &mut Value, binding: &TargetBinding) -> AppResult<()> {
    let object = settings
        .as_object_mut()
        .ok_or_else(|| AppError::new("invalid_config", "Pi settings.json root must be object"))?;
    let live = object.get("defaultProvider").and_then(Value::as_str);
    if live != binding.provider_id.as_deref() {
        return Ok(());
    }
    for (field, key) in [
        ("defaultProvider", BASELINE_PROVIDER),
        ("defaultModel", BASELINE_MODEL),
    ] {
        match binding.expected_fields.get(key).map(String::as_str) {
            Some(BASELINE_MISSING) | None => {
                object.remove(field);
            }
            Some(value) => {
                object.insert(field.into(), Value::String(value.into()));
            }
        }
    }
    crate::adapters::thinking::restore_settings_thinking(object, binding);
    Ok(())
}

struct PiFileLock {
    path: PathBuf,
}

impl PiFileLock {
    fn acquire(file: &Path) -> AppResult<Self> {
        let file_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::new("internal", "invalid Pi config path"))?;
        let path = file.with_file_name(format!("{file_name}.lock"));
        for attempt in 0..10 {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt < 9 {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(AppError::new(
            "lock_busy",
            format!("Pi is using {}", file.display()),
        ))
    }
}

impl Drop for PiFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

struct PiLocks {
    _auth: PiFileLock,
    _settings: PiFileLock,
}

impl PiLocks {
    fn acquire(auth: &Path, settings: &Path) -> AppResult<Self> {
        Ok(Self {
            _auth: PiFileLock::acquire(auth)?,
            _settings: PiFileLock::acquire(settings)?,
        })
    }
}

struct OriginalFile {
    path: PathBuf,
    content: Option<Vec<u8>>,
    secret: bool,
}

fn snapshot_files(paths: [(&Path, bool); 3]) -> AppResult<Vec<OriginalFile>> {
    paths
        .into_iter()
        .map(|(path, secret)| {
            Ok(OriginalFile {
                path: path.to_path_buf(),
                content: path.exists().then(|| fs::read(path)).transpose()?,
                secret,
            })
        })
        .collect()
}

fn rollback_files(files: &[OriginalFile]) -> AppResult<()> {
    for file in files {
        match &file.content {
            Some(content) => atomic_write(&file.path, content, file.secret)?,
            None if file.path.exists() => fs::remove_file(&file.path)?,
            None => {}
        }
    }
    Ok(())
}

fn backup_existing(
    files: &[OriginalFile],
    backup_root: &Path,
    touched: &mut TouchedKeys,
) -> AppResult<Vec<String>> {
    let mut backup_paths = Vec::new();
    for file in files {
        if file.content.is_some() {
            backup_paths.push(backup_file(&file.path, backup_root)?.display().to_string());
            touched.paths.push(file.path.display().to_string());
        } else {
            touched.created_paths.push(file.path.display().to_string());
        }
    }
    Ok(backup_paths)
}

fn write_three_files(
    auth_path: &Path,
    auth: &Value,
    models_path: &Path,
    models: &CstRootNode,
    settings_path: &Path,
    settings: &Value,
    originals: &[OriginalFile],
) -> AppResult<()> {
    let auth_text = serde_json::to_string_pretty(auth)? + "\n";
    let settings_text = serde_json::to_string_pretty(settings)? + "\n";
    let result = (|| {
        fail_test_write_at(1)?;
        atomic_write(auth_path, auth_text.as_bytes(), true)?;
        fail_test_write_at(2)?;
        atomic_write(models_path, models.to_string().as_bytes(), false)?;
        fail_test_write_at(3)?;
        atomic_write(settings_path, settings_text.as_bytes(), false)
    })();
    if let Err(error) = result {
        rollback_files(originals).map_err(|rollback| {
            AppError::new(
                "atomic_write_failed",
                format!("{error}; Pi rollback failed: {rollback}"),
            )
        })?;
        return Err(error);
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_test_write_at(_step: usize) -> AppResult<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static TEST_FAIL_WRITE_STEP: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn fail_test_write_at(step: usize) -> AppResult<()> {
    TEST_FAIL_WRITE_STEP.with(|target| {
        if target.get() == step {
            target.set(0);
            Err(AppError::new(
                "atomic_write_failed",
                format!("injected Pi write failure at step {step}"),
            ))
        } else {
            Ok(())
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub fn apply(
    site: &SiteRow,
    api_key: &str,
    model_id: &str,
    options: &PiApplyOptions,
    binding_before: Option<&TargetBinding>,
    override_path: Option<&str>,
    backup_root: &Path,
) -> AppResult<PiApplyOutcome> {
    let agent_dir = resolve_pi_agent_dir(override_path)?;
    let dir_created = !agent_dir.exists();
    fs::create_dir_all(&agent_dir)?;
    #[cfg(unix)]
    if dir_created {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&agent_dir, fs::Permissions::from_mode(0o700))?;
    }
    let models_path = agent_dir.join("models.json");
    let auth_path = agent_dir.join("auth.json");
    let settings_path = agent_dir.join("settings.json");
    let _locks = PiLocks::acquire(&auth_path, &settings_path)?;

    // Pi may have written auth/settings before the lock was acquired.
    let models = read_models(&models_path)?;
    let mut auth = read_json_object(&auth_path, "auth.json")?;
    let mut settings = read_json_object(&settings_path, "settings.json")?;
    let originals = snapshot_files([
        (&auth_path, true),
        (&models_path, false),
        (&settings_path, false),
    ])?;
    let mut touched = TouchedKeys::default();
    let backup_paths = backup_existing(&originals, backup_root, &mut touched)?;

    let mut thinking = options.thinking.clone();
    crate::adapters::thinking::fill_extended_maps(&mut thinking);
    crate::domain::validate_write(&thinking, TargetKind::Pi, site.protocol, model_id)?;
    let write_opts = PiApplyOptions {
        thinking: thinking.clone(),
        write_all_models: options.write_all_models,
        catalog_models: options.catalog_models.clone(),
    };

    let provider_id = provider_id_for_site(&site.id);
    let (api, base_url) = protocol(site);
    let base_url = base_url?;
    upsert_managed_provider(
        &models,
        &provider_id,
        provider_value(
            &site.name,
            &base_url,
            api,
            model_values(&write_opts, model_id, site.protocol),
        ),
    )?;

    let auth_object = auth
        .as_object_mut()
        .ok_or_else(|| AppError::new("invalid_config", "Pi auth.json root must be object"))?;
    auth_object.retain(|id, _| !id.starts_with("xiaobai_") || id == &provider_id);
    auth_object.insert(
        provider_id.clone(),
        json!({ "type": "api_key", "key": encode_auth_literal(api_key) }),
    );

    let (baseline_provider, baseline_model) = capture_baseline(&settings, binding_before);
    let settings_object = settings
        .as_object_mut()
        .ok_or_else(|| AppError::new("invalid_config", "Pi settings.json root must be object"))?;
    settings_object.insert("defaultProvider".into(), Value::String(provider_id.clone()));
    settings_object.insert("defaultModel".into(), Value::String(model_id.into()));
    let thinking_expected = crate::adapters::thinking::apply_settings_thinking(
        settings_object,
        &thinking,
        &provider_id,
        model_id,
        TargetKind::Pi,
        binding_before,
    );

    write_three_files(
        &auth_path,
        &auth,
        &models_path,
        &models,
        &settings_path,
        &settings,
        &originals,
    )?;
    set_secret_permissions(&auth_path);

    let mut expected = HashMap::from([
        ("base_url".into(), base_url),
        ("api".into(), api.into()),
        (BASELINE_PROVIDER.into(), baseline_provider),
        (BASELINE_MODEL.into(), baseline_model),
    ]);
    expected.insert(
        "write_all_models".into(),
        options.write_all_models.to_string(),
    );
    expected.extend(thinking_expected);
    let binding = TargetBinding {
        target: TargetKind::Pi,
        site_id: Some(site.id.clone()),
        site_name_snapshot: site.name.clone(),
        model_id: model_id.into(),
        provider_id: Some(provider_id.clone()),
        key_fingerprint: key_fingerprint(api_key),
        managed_paths: vec![
            models_path.display().to_string(),
            auth_path.display().to_string(),
            settings_path.display().to_string(),
        ],
        managed_env_keys: Vec::new(),
        expected_fields: expected,
        orphan: false,
        applied_at: Utc::now().timestamp_millis(),
        apply_record_id: Some(Uuid::new_v4().to_string()),
        api_key: Default::default(),
    };
    let verified = (|| {
        let summary = live_summary(override_path)?;
        let (status, reason) =
            detect_status(Some(&binding), Some(site), Some(api_key), override_path)?;
        if status != ApplyStatus::Applied {
            return Err(AppError::new(
                "invalid_config",
                format!("Pi post-write check failed: {}", reason.unwrap_or_default()),
            ));
        }
        let models = models_value(&models_path)?;
        let settings = read_json_object(&settings_path, "settings.json")?;
        let provider = provider(&models, &provider_id)
            .ok_or_else(|| AppError::new("invalid_config", "Pi provider missing after write"))?;
        crate::adapters::thinking::verify_written_thinking(
            provider,
            &settings,
            &thinking,
            site.protocol,
            TargetKind::Pi,
        )?;
        Ok(summary)
    })();
    let summary = if let Ok(summary) = verified {
        summary
    } else {
        let error = verified.unwrap_err();
        rollback_files(&originals)?;
        return Err(error);
    };
    Ok(PiApplyOutcome {
        binding,
        touched,
        backup_paths,
        live_summary: summary,
        message: "Pi models.json, auth.json and settings.json updated. Start a new Pi session."
            .into(),
        provider_id,
    })
}

fn remove_managed(
    binding: &TargetBinding,
    override_path: Option<&str>,
    backup_root: Option<&Path>,
) -> AppResult<Vec<String>> {
    let models_path = models_path(override_path)?;
    let auth_path = auth_path(override_path)?;
    let settings_path = settings_path(override_path)?;
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _locks = PiLocks::acquire(&auth_path, &settings_path)?;
    let models = read_models(&models_path)?;
    let mut auth = read_json_object(&auth_path, "auth.json")?;
    let mut settings = read_json_object(&settings_path, "settings.json")?;
    let originals = snapshot_files([
        (&auth_path, true),
        (&models_path, false),
        (&settings_path, false),
    ])?;
    let backup_paths = if let Some(root) = backup_root {
        backup_existing(&originals, root, &mut TouchedKeys::default())?
    } else {
        Vec::new()
    };
    if let Some(provider_id) = binding.provider_id.as_deref() {
        remove_provider(&models, provider_id)?;
        if let Some(object) = auth.as_object_mut() {
            object.remove(provider_id);
        }
    }
    restore_baseline(&mut settings, binding)?;
    write_three_files(
        &auth_path,
        &auth,
        &models_path,
        &models,
        &settings_path,
        &settings,
        &originals,
    )?;
    Ok(backup_paths)
}

pub fn surgical_revert(binding: &TargetBinding, override_path: Option<&str>) -> AppResult<()> {
    remove_managed(binding, override_path, None).map(|_| ())
}

pub fn restore_official(
    binding: Option<&TargetBinding>,
    override_path: Option<&str>,
    backup_root: &Path,
) -> AppResult<RestoreOfficialOutcome> {
    let backup_paths = match binding {
        Some(binding) => remove_managed(binding, override_path, Some(backup_root))?,
        None => Vec::new(),
    };
    Ok(RestoreOfficialOutcome {
        backup_paths,
        env_keys: Vec::new(),
    })
}

pub fn cleanup_orphans(override_path: Option<&str>) -> AppResult<()> {
    let models_path = models_path(override_path)?;
    let auth_path = auth_path(override_path)?;
    let settings_path = settings_path(override_path)?;
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _locks = PiLocks::acquire(&auth_path, &settings_path)?;
    let models = read_models(&models_path)?;
    let mut auth = read_json_object(&auth_path, "auth.json")?;
    let mut settings = read_json_object(&settings_path, "settings.json")?;
    let originals = snapshot_files([
        (&auth_path, true),
        (&models_path, false),
        (&settings_path, false),
    ])?;
    remove_all_managed_providers(&models)?;
    if let Some(object) = auth.as_object_mut() {
        object.retain(|provider_id, _| !provider_id.starts_with("xiaobai_"));
    }
    if let Some(object) = settings.as_object_mut() {
        if object
            .get("defaultProvider")
            .and_then(Value::as_str)
            .is_some_and(|provider_id| provider_id.starts_with("xiaobai_"))
        {
            object.remove("defaultProvider");
            object.remove("defaultModel");
        }
    }
    write_three_files(
        &auth_path,
        &auth,
        &models_path,
        &models,
        &settings_path,
        &settings,
        &originals,
    )
}

fn models_value(path: &Path) -> AppResult<Value> {
    read_models(path)?
        .object_value()
        .and_then(|object| object.to_serde_value())
        .ok_or_else(|| AppError::new("invalid_config", "Pi models.json root must be object"))
}

fn provider<'a>(models: &'a Value, provider_id: &str) -> Option<&'a Value> {
    models.get("providers")?.get(provider_id)
}

fn auth_key(auth: &Value, provider_id: &str) -> Option<String> {
    let entry = auth.get(provider_id)?;
    if entry.get("type").and_then(Value::as_str) != Some("api_key") {
        return None;
    }
    entry
        .get("key")
        .and_then(Value::as_str)
        .map(decode_auth_literal)
}

pub fn live_summary(override_path: Option<&str>) -> AppResult<HashMap<String, Option<String>>> {
    summary_from_paths(
        &models_path(override_path)?,
        &auth_path(override_path)?,
        &settings_path(override_path)?,
    )
}

fn summary_from_paths(
    models_path: &Path,
    auth_path: &Path,
    settings_path: &Path,
) -> AppResult<HashMap<String, Option<String>>> {
    let models = models_value(models_path)?;
    let auth = read_json_object(auth_path, "auth.json")?;
    let settings = read_json_object(settings_path, "settings.json")?;
    let provider_id = settings
        .get("defaultProvider")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with("xiaobai_"))
        .or_else(|| {
            models
                .get("providers")
                .and_then(Value::as_object)
                .and_then(|providers| providers.keys().find(|id| id.starts_with("xiaobai_")))
                .map(String::as_str)
        });
    let managed = provider_id.and_then(|id| provider(&models, id));
    let mut out = HashMap::from([
        (
            "defaultProvider".into(),
            settings
                .get("defaultProvider")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        (
            "defaultModel".into(),
            settings
                .get("defaultModel")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        (
            "baseUrl".into(),
            managed
                .and_then(|value| value.get("baseUrl"))
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        (
            "api".into(),
            managed
                .and_then(|value| value.get("api"))
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        (
            "apiKey".into(),
            provider_id
                .and_then(|id| auth_key(&auth, id))
                .map(|key| key_prefix(&key)),
        ),
        (
            "modelCount".into(),
            managed
                .and_then(|value| value.get("models"))
                .and_then(Value::as_array)
                .map(|models| models.len().to_string()),
        ),
    ]);
    for (key, value) in crate::adapters::thinking::thinking_live_fields(managed, &settings) {
        out.insert(key, value);
    }
    Ok(out)
}

pub fn backup_summary(dir: &Path) -> HashMap<String, Option<String>> {
    summary_from_paths(
        &dir.join("models.json"),
        &dir.join("auth.json"),
        &dir.join("settings.json"),
    )
    .unwrap_or_default()
}

pub fn detect_status(
    binding: Option<&TargetBinding>,
    site: Option<&SiteRow>,
    api_key_value: Option<&str>,
    override_path: Option<&str>,
) -> AppResult<(ApplyStatus, Option<String>)> {
    let models = models_value(&models_path(override_path)?)?;
    let providers = models.get("providers").and_then(Value::as_object);
    let orphan =
        providers.is_some_and(|providers| providers.keys().any(|id| id.starts_with("xiaobai_")));
    let Some(binding) = binding else {
        return Ok(if orphan {
            (ApplyStatus::Orphan, Some("untracked Pi provider".into()))
        } else {
            (ApplyStatus::NotApplied, None)
        });
    };
    if binding.orphan || site.is_none() {
        return Ok((ApplyStatus::Orphan, Some("bound site is missing".into())));
    }
    let provider_id = binding
        .provider_id
        .as_deref()
        .ok_or_else(|| AppError::new("invalid_config", "Pi binding provider id missing"))?;
    let Some(provider) = provider(&models, provider_id) else {
        return Ok((ApplyStatus::Stale, Some("Pi provider missing".into())));
    };
    let current_site = site.expect("site checked above");
    let (current_api, current_base_url) = protocol(current_site);
    let current_base_url = current_base_url?;
    if binding.expected_fields.get("api").map(String::as_str) != Some(current_api)
        || binding.expected_fields.get("base_url").map(String::as_str)
            != Some(current_base_url.as_str())
    {
        return Ok((
            ApplyStatus::Stale,
            Some("Pi site protocol or Base URL changed".into()),
        ));
    }
    for (field, expected) in [("baseUrl", "base_url"), ("api", "api")] {
        if provider.get(field).and_then(Value::as_str)
            != binding.expected_fields.get(expected).map(String::as_str)
        {
            return Ok((ApplyStatus::Stale, Some(format!("Pi {field} changed"))));
        }
    }
    let model_exists = provider
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models.iter().any(|model| {
                model.get("id").and_then(Value::as_str) == Some(binding.model_id.as_str())
            })
        });
    if !model_exists {
        return Ok((ApplyStatus::Stale, Some("Pi selected model missing".into())));
    }
    let auth = read_json_object(&auth_path(override_path)?, "auth.json")?;
    let live_key = auth_key(&auth, provider_id);
    let expected_fingerprint = api_key_value
        .map(key_fingerprint)
        .unwrap_or_else(|| binding.key_fingerprint.clone());
    if live_key.as_deref().map(key_fingerprint).as_deref() != Some(&expected_fingerprint) {
        return Ok((ApplyStatus::Stale, Some("Pi API key changed".into())));
    }
    Ok((ApplyStatus::Applied, None))
}

pub fn rewrite_base_url(
    binding: &TargetBinding,
    site: &SiteRow,
    override_path: Option<&str>,
    backup_root: &Path,
) -> AppResult<RewriteOutcome> {
    let path = models_path(override_path)?;
    let models = read_models(&path)?;
    let provider_id = binding
        .provider_id
        .as_deref()
        .ok_or_else(|| AppError::new("invalid_config", "Pi binding provider id missing"))?;
    let object = models
        .object_value()
        .ok_or_else(|| AppError::new("invalid_config", "Pi models.json root must be object"))?;
    let provider = object
        .object_value("providers")
        .and_then(|providers| providers.object_value(provider_id))
        .ok_or_else(|| AppError::new("invalid_config", "managed Pi provider missing"))?;
    let (_, base_url) = protocol(site);
    let base_url = base_url?;
    match provider.get("baseUrl") {
        Some(prop) => prop.set_value(base_url.clone().into()),
        None => {
            provider.append("baseUrl", base_url.clone().into());
        }
    }
    let backup = backup_file(&path, backup_root)?;
    atomic_write(&path, models.to_string().as_bytes(), false)?;
    let mut expected = binding.expected_fields.clone();
    expected.insert("base_url".into(), base_url);
    Ok(RewriteOutcome {
        backup_paths: vec![backup.display().to_string()],
        live_summary: live_summary(override_path)?,
        expected_fields: expected,
        message: "Pi provider route updated.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::SiteCapabilities;
    use crate::domain::{ClaudeAuthKeyStyle, ModelThinkingConfig, ThinkingWrite};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn site(protocol: SiteProtocol) -> SiteRow {
        SiteRow {
            id: "12345678-abcd-0000-0000-000000000000".into(),
            name: "Example".into(),
            base_url: "https://api.example.com".into(),
            base_urls: vec!["https://api.example.com".into()],
            api_key_encrypted: String::new(),
            key_prefix: "sk-***".into(),
            protocol,
            claude_auth_key_style: ClaudeAuthKeyStyle::AnthropicAuthToken,
            notes: None,
            enabled: true,
            sort_order: 0,
            selected_model_id: Some("model-a".into()),
            last_model_fetch_at: None,
            last_model_fetch_latency_ms: None,
            last_model_fetch_error: None,
            created_at: 0,
            updated_at: 0,
            capabilities: SiteCapabilities::default(),
            keys: Default::default(),
            newapi_access_token_encrypted: None,
            newapi_user_id: None,
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
        }
    }

    fn apply_fixture(
        dir: &Path,
        site: &SiteRow,
        key: &str,
        options: &PiApplyOptions,
        previous: Option<&TargetBinding>,
    ) -> PiApplyOutcome {
        let backup = dir.join("backup");
        fs::create_dir_all(&backup).unwrap();
        apply(
            site,
            key,
            "model-a",
            options,
            previous,
            dir.to_str(),
            &backup,
        )
        .unwrap()
    }

    fn assert_each_model_input_is_text_and_image(list: &[Value]) {
        assert!(!list.is_empty());
        for model in list {
            assert_eq!(model["input"], json!(["text", "image"]));
        }
    }

    #[test]
    fn maps_openai_and_anthropic_protocols() {
        let dir = tempdir().unwrap();
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        assert_eq!(outcome.binding.expected_fields["api"], "openai-completions");
        assert_eq!(
            outcome.binding.expected_fields["base_url"],
            "https://api.example.com/v1"
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        let list = models["providers"][&outcome.provider_id]["models"]
            .as_array()
            .unwrap();
        assert_eq!(
            list,
            &[json!({ "id": "model-a", "input": ["text", "image"] })]
        );
        let other = tempdir().unwrap();
        let outcome = apply_fixture(
            other.path(),
            &site(SiteProtocol::Anthropic),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        assert_eq!(outcome.binding.expected_fields["api"], "anthropic-messages");
        assert_eq!(
            outcome.binding.expected_fields["base_url"],
            "https://api.example.com"
        );
    }

    #[test]
    fn preserves_jsonc_comments_trailing_commas_and_other_providers() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("models.json"),
            "{\n  // keep me\n  \"providers\": {\n    \"other\": { \"baseUrl\": \"x\", },\n  },\n}\n",
        )
        .unwrap();
        apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        let text = fs::read_to_string(dir.path().join("models.json")).unwrap();
        assert!(text.contains("// keep me"));
        assert!(text.contains("\"other\""));
        assert!(text.contains("\"baseUrl\": \"x\","));
    }

    #[test]
    fn rejects_jsonc_features_that_pi_does_not_support() {
        assert!(parse_models("{/* block */\"providers\":{}}").is_err());
        assert!(parse_models("{'providers':{}}").is_err());
        assert!(parse_models("{providers:{}}").is_err());
    }

    #[test]
    fn keeps_oauth_and_unknown_settings_fields() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("auth.json"),
            r#"{"anthropic":{"type":"oauth","access":"token"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"theme":"dark","defaultProvider":"anthropic","defaultModel":"old"}"#,
        )
        .unwrap();
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::Anthropic),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        let auth = read_json_object(&dir.path().join("auth.json"), "auth.json").unwrap();
        assert_eq!(auth["anthropic"]["type"], "oauth");
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["theme"], "dark");
        surgical_revert(&outcome.binding, dir.path().to_str()).unwrap();
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultProvider"], "anthropic");
        assert_eq!(settings["defaultModel"], "old");
    }

    #[test]
    fn encodes_and_decodes_special_keys() {
        for key in ["sk-normal", "!command", "$ENV", "a$b", "!a$b"] {
            assert_eq!(decode_auth_literal(&encode_auth_literal(key)), key);
        }
        let dir = tempdir().unwrap();
        let site = site(SiteProtocol::OpenaiCompatible);
        let outcome = apply_fixture(dir.path(), &site, "!a$b", &PiApplyOptions::default(), None);
        let auth = read_json_object(&dir.path().join("auth.json"), "auth.json").unwrap();
        assert_eq!(auth[&outcome.provider_id]["key"], "$!a$$b");
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&site),
                Some("!a$b"),
                dir.path().to_str(),
            )
            .unwrap()
            .0,
            ApplyStatus::Applied
        );
    }

    #[test]
    fn write_all_models_deduplicates_and_always_keeps_selected() {
        let dir = tempdir().unwrap();
        let options = PiApplyOptions {
            write_all_models: true,
            catalog_models: vec![
                ("model-b".into(), "Model B".into()),
                ("model-b".into(), "Duplicate".into()),
            ],
            ..Default::default()
        };
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &options,
            None,
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        let list = models["providers"][&outcome.provider_id]["models"]
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|model| model["id"] == "model-a"));
        assert_eq!(list[0]["name"], "Model B");
        assert!(list[1].get("name").is_none());
        assert_each_model_input_is_text_and_image(list);
    }

    #[test]
    fn managed_models_always_declare_text_and_image_input() {
        let selected_dir = tempdir().unwrap();
        let selected = apply_fixture(
            selected_dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        let selected_models = models_value(&selected_dir.path().join("models.json")).unwrap();
        let selected_list = selected_models["providers"][&selected.provider_id]["models"]
            .as_array()
            .unwrap();
        assert_eq!(selected_list.len(), 1);
        assert_eq!(selected_list[0]["id"], "model-a");
        assert_each_model_input_is_text_and_image(selected_list);

        let catalog_dir = tempdir().unwrap();
        let options = PiApplyOptions {
            write_all_models: true,
            catalog_models: vec![
                ("vision-model".into(), "Vision".into()),
                ("text-model".into(), "Text".into()),
            ],
            ..Default::default()
        };
        let catalog = apply_fixture(
            catalog_dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &options,
            None,
        );
        let catalog_models = models_value(&catalog_dir.path().join("models.json")).unwrap();
        let catalog_list = catalog_models["providers"][&catalog.provider_id]["models"]
            .as_array()
            .unwrap();
        assert_eq!(catalog_list.len(), 3);
        assert_eq!(
            catalog_list
                .iter()
                .map(|model| model["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["vision-model", "text-model", "model-a"]
        );
        assert_each_model_input_is_text_and_image(catalog_list);
    }

    #[test]
    fn switch_is_single_active_and_carries_baseline() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultProvider":"anthropic","defaultModel":"old"}"#,
        )
        .unwrap();
        let first = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key-a",
            &PiApplyOptions::default(),
            None,
        );
        let mut second_site = site(SiteProtocol::Anthropic);
        second_site.id = "abcdef12-3456-0000-0000-000000000000".into();
        let second = apply_fixture(
            dir.path(),
            &second_site,
            "key-b",
            &PiApplyOptions::default(),
            Some(&first.binding),
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        let managed = models["providers"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|id| id.starts_with("xiaobai_"))
            .count();
        assert_eq!(managed, 1);
        surgical_revert(&second.binding, dir.path().to_str()).unwrap();
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultProvider"], "anthropic");
        assert_eq!(settings["defaultModel"], "old");
    }

    #[test]
    fn switching_again_keeps_the_first_apply_baseline() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultProvider":"original","defaultModel":"original-model"}"#,
        )
        .unwrap();
        let first = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key-a",
            &PiApplyOptions::default(),
            None,
        );
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultProvider":"temporary-user-choice","defaultModel":"temporary-model"}"#,
        )
        .unwrap();
        let mut second_site = site(SiteProtocol::Anthropic);
        second_site.id = "abcdef12-3456-0000-0000-000000000000".into();
        let second = apply_fixture(
            dir.path(),
            &second_site,
            "key-b",
            &PiApplyOptions::default(),
            Some(&first.binding),
        );
        assert_eq!(
            second.binding.expected_fields[BASELINE_PROVIDER],
            "original"
        );
        assert_eq!(
            second.binding.expected_fields[BASELINE_MODEL],
            "original-model"
        );
    }

    #[test]
    fn removal_does_not_override_a_later_user_default() {
        let dir = tempdir().unwrap();
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        let path = dir.path().join("settings.json");
        let mut settings = read_json_object(&path, "settings.json").unwrap();
        settings["defaultProvider"] = Value::String("user-provider".into());
        settings["defaultModel"] = Value::String("user-model".into());
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
        surgical_revert(&outcome.binding, dir.path().to_str()).unwrap();
        let settings = read_json_object(&path, "settings.json").unwrap();
        assert_eq!(settings["defaultProvider"], "user-provider");
        assert_eq!(settings["defaultModel"], "user-model");
    }

    #[test]
    fn changed_default_model_does_not_make_provider_stale() {
        let dir = tempdir().unwrap();
        let site = site(SiteProtocol::OpenaiCompatible);
        let outcome = apply_fixture(dir.path(), &site, "key", &PiApplyOptions::default(), None);
        let path = dir.path().join("settings.json");
        let mut settings = read_json_object(&path, "settings.json").unwrap();
        settings["defaultModel"] = Value::String("user-choice".into());
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&site),
                Some("key"),
                dir.path().to_str(),
            )
            .unwrap()
            .0,
            ApplyStatus::Applied
        );
    }

    #[test]
    fn changed_auth_key_makes_provider_stale() {
        let dir = tempdir().unwrap();
        let site = site(SiteProtocol::OpenaiCompatible);
        let outcome = apply_fixture(
            dir.path(),
            &site,
            "original-secret",
            &PiApplyOptions::default(),
            None,
        );
        let path = dir.path().join("auth.json");
        let mut auth = read_json_object(&path, "auth.json").unwrap();
        auth[&outcome.provider_id]["key"] = Value::String("changed-secret".into());
        fs::write(&path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&site),
                Some("original-secret"),
                dir.path().to_str(),
            )
            .unwrap()
            .0,
            ApplyStatus::Stale
        );
    }

    #[test]
    fn backup_summary_never_exposes_the_full_key() {
        let dir = tempdir().unwrap();
        apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "abcdefghijklmnop",
            &PiApplyOptions::default(),
            None,
        );
        let summary = backup_summary(dir.path());
        let key = summary.get("apiKey").and_then(Option::as_deref).unwrap();
        assert!(!key.contains("abcdefghijklmnop"));
        assert!(key.contains('…'));
    }

    #[test]
    fn route_rewrite_updates_only_the_managed_base_url() {
        let dir = tempdir().unwrap();
        let mut site = site(SiteProtocol::OpenaiCompatible);
        let outcome = apply_fixture(dir.path(), &site, "key", &PiApplyOptions::default(), None);
        site.base_url = "https://second.example.com/api".into();
        let backup = dir.path().join("route-backup");
        fs::create_dir_all(&backup).unwrap();
        let rewrite =
            rewrite_base_url(&outcome.binding, &site, dir.path().to_str(), &backup).unwrap();
        assert_eq!(
            rewrite.expected_fields["base_url"],
            "https://second.example.com/api/v1"
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        assert_eq!(
            models["providers"][&outcome.provider_id]["baseUrl"],
            "https://second.example.com/api/v1"
        );
        assert_eq!(rewrite.backup_paths.len(), 1);
    }

    #[test]
    fn rolls_back_all_files_when_the_last_write_fails() {
        let dir = tempdir().unwrap();
        let models_before = "{\n  // original\n  \"providers\": {}\n}\n";
        let auth_before = r#"{"other":{"type":"oauth","access":"token"}}"#;
        let settings_before = r#"{"defaultProvider":"other","defaultModel":"old"}"#;
        fs::write(dir.path().join("models.json"), models_before).unwrap();
        fs::write(dir.path().join("auth.json"), auth_before).unwrap();
        fs::write(dir.path().join("settings.json"), settings_before).unwrap();
        let backup = dir.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        TEST_FAIL_WRITE_STEP.with(|step| step.set(3));
        let error = apply(
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            "model-a",
            &PiApplyOptions::default(),
            None,
            dir.path().to_str(),
            &backup,
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected Pi write failure"));
        assert_eq!(
            fs::read_to_string(dir.path().join("models.json")).unwrap(),
            models_before
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("auth.json")).unwrap(),
            auth_before
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            settings_before
        );
    }

    #[test]
    fn detects_and_cleans_untracked_managed_provider() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("models.json"),
            r#"{"providers":{"xiaobai_orphan":{"baseUrl":"x","api":"openai-completions","models":[]},"other":{}}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("auth.json"),
            r#"{"xiaobai_orphan":{"type":"api_key","key":"secret"},"other":{"type":"oauth"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultProvider":"xiaobai_orphan","defaultModel":"old","theme":"dark"}"#,
        )
        .unwrap();
        assert_eq!(
            detect_status(None, None, None, dir.path().to_str())
                .unwrap()
                .0,
            ApplyStatus::Orphan
        );
        cleanup_orphans(dir.path().to_str()).unwrap();
        let models = models_value(&dir.path().join("models.json")).unwrap();
        assert!(models["providers"].get("xiaobai_orphan").is_none());
        assert!(models["providers"].get("other").is_some());
        let auth = read_json_object(&dir.path().join("auth.json"), "auth.json").unwrap();
        assert!(auth.get("xiaobai_orphan").is_none());
        assert!(auth.get("other").is_some());
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert!(settings.get("defaultProvider").is_none());
        assert_eq!(settings["theme"], "dark");
    }

    #[cfg(unix)]
    #[test]
    fn creates_secure_directory_and_auth_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir().unwrap();
        let agent = root.path().join("agent");
        let backup = root.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        apply(
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            "model-a",
            &PiApplyOptions::default(),
            None,
            agent.to_str(),
            &backup,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&agent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(agent.join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    fn thinking_enabled(level: &str) -> ThinkingWrite {
        let mut thinking = ThinkingWrite::default();
        thinking.default_level = Some(level.into());
        thinking.extended.max = Some("max".into());
        thinking.models.insert(
            "model-a".into(),
            ModelThinkingConfig {
                reasoning: true,
                thinking_level_map: BTreeMap::from([("max".into(), "max".into())]),
                ..Default::default()
            },
        );
        thinking
    }

    #[test]
    fn writes_reasoning_map_and_default_thinking_level() {
        let dir = tempdir().unwrap();
        let options = PiApplyOptions {
            thinking: thinking_enabled("medium"),
            ..Default::default()
        };
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &options,
            None,
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        let model = &models["providers"][&outcome.provider_id]["models"][0];
        assert_eq!(model["reasoning"], true);
        assert_eq!(model["thinkingLevelMap"]["max"], "max");
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultThinkingLevel"], "medium");
        assert_eq!(
            outcome.live_summary["defaultThinkingLevel"].as_deref(),
            Some("medium")
        );
        assert_eq!(
            outcome.live_summary["reasoningModelCount"].as_deref(),
            Some("1")
        );
    }

    #[test]
    fn off_keeps_reasoning_capability() {
        let dir = tempdir().unwrap();
        let options = PiApplyOptions {
            thinking: thinking_enabled("off"),
            ..Default::default()
        };
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &options,
            None,
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        assert_eq!(
            models["providers"][&outcome.provider_id]["models"][0]["reasoning"],
            true
        );
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultThinkingLevel"], "off");
    }

    #[test]
    fn unchanged_default_level_leaves_existing_setting() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultThinkingLevel":"high"}"#,
        )
        .unwrap();
        apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &PiApplyOptions::default(),
            None,
        );
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultThinkingLevel"], "high");
    }

    #[test]
    fn anthropic_writes_adaptive_compat() {
        let dir = tempdir().unwrap();
        let mut thinking = thinking_enabled("xhigh");
        thinking.extended.xhigh = Some("xhigh".into());
        thinking
            .models
            .get_mut("model-a")
            .unwrap()
            .force_adaptive_thinking = true;
        thinking
            .models
            .get_mut("model-a")
            .unwrap()
            .thinking_level_map
            .insert("xhigh".into(), "xhigh".into());
        let options = PiApplyOptions {
            thinking,
            ..Default::default()
        };
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::Anthropic),
            "key",
            &options,
            None,
        );
        let models = models_value(&dir.path().join("models.json")).unwrap();
        let model = &models["providers"][&outcome.provider_id]["models"][0];
        assert_eq!(model["compat"]["forceAdaptiveThinking"], true);
        assert_eq!(model["thinkingLevelMap"]["xhigh"], "xhigh");
    }

    #[test]
    fn revert_restores_thinking_baseline() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"defaultProvider":"anthropic","defaultModel":"old","defaultThinkingLevel":"low","modelThinkingLevels":{"xiaobai_placeholder/model-a":"high"}}"#,
        )
        .unwrap();
        let options = PiApplyOptions {
            thinking: thinking_enabled("medium"),
            ..Default::default()
        };
        let outcome = apply_fixture(
            dir.path(),
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            &options,
            None,
        );
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultThinkingLevel"], "medium");
        surgical_revert(&outcome.binding, dir.path().to_str()).unwrap();
        let settings =
            read_json_object(&dir.path().join("settings.json"), "settings.json").unwrap();
        assert_eq!(settings["defaultThinkingLevel"], "low");
        assert_eq!(settings["defaultProvider"], "anthropic");
    }

    #[test]
    fn user_changed_thinking_level_is_not_stale() {
        let dir = tempdir().unwrap();
        let options = PiApplyOptions {
            thinking: thinking_enabled("medium"),
            ..Default::default()
        };
        let site = site(SiteProtocol::OpenaiCompatible);
        let outcome = apply_fixture(dir.path(), &site, "key", &options, None);
        let path = dir.path().join("settings.json");
        let mut settings = read_json_object(&path, "settings.json").unwrap();
        settings["defaultThinkingLevel"] = Value::String("high".into());
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
        assert_eq!(
            detect_status(
                Some(&outcome.binding),
                Some(&site),
                Some("key"),
                dir.path().to_str(),
            )
            .unwrap()
            .0,
            ApplyStatus::Applied
        );
    }

    #[test]
    fn extended_without_mapping_fails() {
        let dir = tempdir().unwrap();
        let mut thinking = ThinkingWrite::default();
        thinking.default_level = Some("max".into());
        thinking.models.insert(
            "model-a".into(),
            ModelThinkingConfig {
                reasoning: true,
                ..Default::default()
            },
        );
        let backup = dir.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        let err = apply(
            &site(SiteProtocol::OpenaiCompatible),
            "key",
            "model-a",
            &PiApplyOptions {
                thinking,
                ..Default::default()
            },
            None,
            dir.path().to_str(),
            &backup,
        )
        .unwrap_err();
        assert!(err.to_string().contains("provider value"));
    }
}
