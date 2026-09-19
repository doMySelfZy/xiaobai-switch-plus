use crate::adapters::{claude_code, codex, pi, prime};
use crate::crypto::key_prefix;
use crate::domain::{
    clamp_max_backup_copies, AppSettings, BackupFileInfo, BackupInfo, BackupPreview, TargetKind,
};
use crate::error::{AppError, AppResult};
use crate::paths::{backups_dir, codex_env_path};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

pub const META_FILE: &str = "xiaobai-backup.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackupMeta {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub apply_record_id: Option<String>,
    #[serde(default)]
    pub files: Vec<BackupMetaFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackupMetaFile {
    pub name: String,
    #[serde(default)]
    pub original_path: Option<String>,
}

pub fn write_meta(dir: &Path, meta: &BackupMeta) -> AppResult<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(meta)? + "\n";
    fs::write(dir.join(META_FILE), text)?;
    Ok(())
}

pub fn read_meta(dir: &Path) -> Option<BackupMeta> {
    let path = dir.join(META_FILE);
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn payload_files(dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == META_FILE || name == crate::adapters::atomic::BACKUP_ORIGINS_FILE {
                None
            } else {
                Some(name)
            }
        })
        .collect();
    files.sort();
    files
}

pub fn prune_target_backups(target: TargetKind, max: u32) -> AppResult<usize> {
    prune_target_backups_in(&backups_dir()?, target, max)
}

pub fn prune_all(max: u32) -> AppResult<usize> {
    // 刻意不再遍历退役目标的 backups/zcode/：v0.1.5 起它就没进过 `list_backups` 的默认目标
    // 清单（见 `commands/apply.rs`），用户在 UI 里既看不见也无法恢复，继续自动删一个用户无法
    // 感知的目录反而更危险。该目录需要人工清理，见 release note。
    let mut n = 0;
    n += prune_target_backups(TargetKind::ClaudeCode, max)?;
    n += prune_target_backups(TargetKind::Codex, max)?;
    n += prune_target_backups(TargetKind::Pi, max)?;
    n += prune_target_backups(TargetKind::Prime, max)?;
    Ok(n)
}

pub fn prune_target_backups_in(root: &Path, target: TargetKind, max: u32) -> AppResult<usize> {
    let max = clamp_max_backup_copies(max) as usize;
    let dir = root.join(target.as_str());
    if !dir.exists() {
        return Ok(0);
    }
    let mut stamps: Vec<(i64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if payload_files(&path).is_empty() {
            let _ = fs::remove_dir_all(&path);
            continue;
        }
        if let Ok(ts) = name.parse::<i64>() {
            stamps.push((ts, path));
        }
    }
    stamps.sort_by(|a, b| b.0.cmp(&a.0));
    let mut removed = 0;
    for (_, path) in stamps.into_iter().skip(max) {
        fs::remove_dir_all(path)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn list_backups_in(
    root: &Path,
    targets: &[TargetKind],
    mut lookup: impl FnMut(&str) -> (Option<String>, Option<String>, Option<String>),
) -> AppResult<Vec<BackupInfo>> {
    let mut out = Vec::new();
    for target in targets {
        let dir = root.join(target.as_str());
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let files = payload_files(&path);
            if files.is_empty() {
                continue;
            }
            let created_at = name.parse::<i64>().unwrap_or(0);
            let dir_s = path.display().to_string();
            let meta = read_meta(&path);
            let (record_id, site, model) = if let Some(m) = meta {
                (m.apply_record_id, m.site_name, m.model_id)
            } else {
                lookup(&dir_s)
            };
            out.push(BackupInfo {
                id: format!("{}-{name}", target.as_str()),
                target: target.as_str().into(),
                dir: dir_s,
                created_at,
                files,
                apply_record_id: record_id,
                site_name_snapshot: site,
                model_id: model,
            });
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

pub fn resolve_backup_in(root: &Path, id: &str) -> AppResult<(TargetKind, PathBuf)> {
    let (target, stamp) = parse_backup_id(id)?;
    let path = root.join(target.as_str()).join(&stamp);
    ensure_under(root, &path)?;
    if !path.is_dir() {
        return Err(AppError::new("not_found", "backup not found"));
    }
    Ok((target, path))
}

pub fn parse_backup_id(id: &str) -> AppResult<(TargetKind, String)> {
    let (target, stamp) = if let Some(rest) = id.strip_prefix("claude_code-") {
        (TargetKind::ClaudeCode, rest)
    } else if let Some(rest) = id.strip_prefix("codex-") {
        (TargetKind::Codex, rest)
    } else if let Some(rest) = id.strip_prefix("pi-") {
        (TargetKind::Pi, rest)
    } else if let Some(rest) = id.strip_prefix("prime-") {
        (TargetKind::Prime, rest)
    } else {
        // 退役目标的 `zcode-<stamp>` 备份 id 落到 Err：`parse_backup_id` 要返回 `TargetKind`，
        // 而 `mapped_dest` 也已无对应臂 —— 两者同进退，只留其一会让恢复报
        // "no restorable files" 而不是明确的「未知备份 id」。
        return Err(AppError::new("validation_failed", "invalid backup id"));
    };
    if stamp.is_empty() || !stamp.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::new("validation_failed", "invalid backup id"));
    }
    Ok((target, stamp.to_string()))
}

fn ensure_under(root: &Path, path: &Path) -> AppResult<()> {
    let root_c = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let candidate = if path.exists() {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    if !candidate.starts_with(&root_c) && !path.starts_with(root) {
        return Err(AppError::new(
            "validation_failed",
            "backup path outside backups dir",
        ));
    }
    Ok(())
}

pub fn delete_backup_in(root: &Path, id: &str) -> AppResult<()> {
    let (_, path) = resolve_backup_in(root, id)?;
    fs::remove_dir_all(path)?;
    Ok(())
}

pub fn mapped_dest(
    target: TargetKind,
    file_name: &str,
    settings: &AppSettings,
) -> AppResult<Option<PathBuf>> {
    Ok(match (target, file_name) {
        (TargetKind::ClaudeCode, "settings.json") => Some(claude_code::settings_path(
            settings.claude_home_override.as_deref(),
        )?),
        (TargetKind::Codex, "config.toml") => {
            Some(codex::config_path(settings.codex_home_override.as_deref())?)
        }
        (TargetKind::Codex, "xiaobai-model-catalog.json") => Some(codex::model_catalog_path(
            settings.codex_home_override.as_deref(),
        )?),
        (TargetKind::Codex, "codex.env") => Some(codex_env_path()?),
        (TargetKind::Pi, "models.json") => {
            Some(pi::models_path(settings.pi_agent_dir_override.as_deref())?)
        }
        (TargetKind::Pi, "auth.json") => {
            Some(pi::auth_path(settings.pi_agent_dir_override.as_deref())?)
        }
        (TargetKind::Pi, "settings.json") => Some(pi::settings_path(
            settings.pi_agent_dir_override.as_deref(),
        )?),
        (TargetKind::Prime, "models.json") => Some(prime::models_path(
            settings.prime_agent_dir_override.as_deref(),
        )?),
        (TargetKind::Prime, "auth.json") => Some(prime::auth_path(
            settings.prime_agent_dir_override.as_deref(),
        )?),
        (TargetKind::Prime, "settings.json") => Some(prime::settings_path(
            settings.prime_agent_dir_override.as_deref(),
        )?),
        _ => None,
    })
}

fn dest_is_allowed(dest: &Path, settings: &AppSettings) -> bool {
    let mut roots = Vec::new();
    if let Ok(p) = crate::paths::resolve_codex_home(settings.codex_home_override.as_deref()) {
        roots.push(p);
    }
    if let Ok(p) = crate::paths::resolve_claude_home(settings.claude_home_override.as_deref()) {
        roots.push(p);
    }
    if let Ok(p) = crate::paths::resolve_pi_agent_dir(settings.pi_agent_dir_override.as_deref()) {
        roots.push(p);
    }
    if let Ok(p) =
        crate::paths::resolve_prime_agent_dir(settings.prime_agent_dir_override.as_deref())
    {
        roots.push(p);
    }
    if let Ok(p) = crate::paths::app_dir() {
        roots.push(p);
    }
    roots.iter().any(|root| dest.starts_with(root))
}

pub fn restore_backup_in(
    root: &Path,
    id: &str,
    settings: &AppSettings,
    dest_override: Option<&HashMap<String, PathBuf>>,
) -> AppResult<Vec<PathBuf>> {
    let (target, dir) = resolve_backup_in(root, id)?;
    let files = payload_files(&dir);
    if files.is_empty() {
        return Err(AppError::new("not_found", "backup has no files"));
    }
    let origins = crate::adapters::atomic::read_origins(&dir);
    let meta = read_meta(&dir);
    let meta_origins: HashMap<String, PathBuf> = meta
        .map(|m| {
            m.files
                .into_iter()
                .filter_map(|f| f.original_path.map(|p| (f.name, PathBuf::from(p))))
                .collect()
        })
        .unwrap_or_default();
    let mut restored = Vec::new();
    for name in files {
        let dest = if let Some(map) = dest_override {
            map.get(&name).cloned()
        } else if let Some(mapped) = mapped_dest(target, &name, settings)? {
            Some(mapped)
        } else {
            origins
                .get(&name)
                .map(PathBuf::from)
                .or_else(|| meta_origins.get(&name).cloned())
                .filter(|p| dest_is_allowed(p, settings))
        };
        let Some(dest) = dest else {
            continue;
        };
        crate::adapters::atomic::restore_file(&dir.join(&name), &dest)?;
        if name == "codex.env"
            || ((target == TargetKind::Pi || target == TargetKind::Prime) && name == "auth.json")
        {
            crate::paths::set_secret_permissions(&dest);
        }
        restored.push(dest);
    }
    if restored.is_empty() {
        return Err(AppError::new(
            "backup_failed",
            "no restorable files in this backup",
        ));
    }
    Ok(restored)
}

pub fn preview_backup_in(root: &Path, id: &str) -> AppResult<BackupPreview> {
    let (target, dir) = resolve_backup_in(root, id)?;
    let files = payload_files(&dir);
    Ok(BackupPreview {
        id: id.to_string(),
        summary: summary_from_backup_dir(&dir, target),
        files: files
            .into_iter()
            .map(|name| BackupFileInfo {
                path: dir.join(&name).display().to_string(),
                name,
            })
            .collect(),
    })
}

fn summary_from_backup_dir(dir: &Path, target: TargetKind) -> HashMap<String, Option<String>> {
    if target == TargetKind::Pi {
        return pi::backup_summary(dir);
    }
    if target == TargetKind::Prime {
        return prime::backup_summary(dir);
    }
    let mut out = HashMap::new();
    let settings_json = dir.join("settings.json");
    if settings_json.exists() {
        if let Ok(text) = fs::read_to_string(&settings_json) {
            if let Ok(v) = serde_json::from_str(&text) {
                out.extend(claude_code::summary_from_settings(&v));
            }
        }
    }
    let config_toml = dir.join("config.toml");
    if config_toml.exists() {
        if let Ok(text) = fs::read_to_string(&config_toml) {
            if let Ok(doc) = text.parse::<DocumentMut>() {
                out.extend(codex::summary_from_config(&doc));
            }
        }
    }
    let env_path = dir.join("codex.env");
    if env_path.exists() {
        out.extend(summary_from_env_file(&env_path));
    }
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") || name == META_FILE {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
                        let key = if name == "xiaobai-model-catalog.json" {
                            "catalog_models".into()
                        } else {
                            format!("catalog_models:{name}")
                        };
                        out.insert(key, Some(models.len().to_string()));
                    }
                }
            }
        }
    }
    out
}

fn summary_from_env_file(path: &Path) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((key, raw)) = body.split_once('=') else {
            continue;
        };
        let value = raw.trim().trim_matches('"').replace("\\\"", "\"");
        if key.contains("KEY") || key.contains("TOKEN") || key.contains("SECRET") {
            out.insert(key.to_string(), Some(key_prefix(&value)));
        } else {
            out.insert(key.to_string(), Some(value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::default_max_backup_copies;
    use tempfile::tempdir;

    fn stamp_dir(root: &Path, target: TargetKind, ts: i64, files: &[(&str, &str)]) -> PathBuf {
        let dir = root.join(target.as_str()).join(ts.to_string());
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            fs::write(dir.join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn parse_id_rejects_traversal() {
        assert!(parse_backup_id("codex-../etc").is_err());
        assert!(parse_backup_id("claude_code-abc").is_err());
        assert!(parse_backup_id("claude_code-1710000000000").is_ok());
        assert_eq!(
            parse_backup_id("pi-1710000000000").unwrap().0,
            TargetKind::Pi
        );
        assert_eq!(
            parse_backup_id("prime-1710000000000").unwrap().0,
            TargetKind::Prime
        );
        // 退役目标的备份 id 必须被明确拒绝（而不是映射到某个存活目标后被部分还原）。
        assert!(parse_backup_id("zcode-1710000000000").is_err());
    }

    #[test]
    fn prune_keeps_newest() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        stamp_dir(
            root,
            TargetKind::ClaudeCode,
            100,
            &[("settings.json", "{}")],
        );
        stamp_dir(
            root,
            TargetKind::ClaudeCode,
            200,
            &[("settings.json", "{}")],
        );
        stamp_dir(
            root,
            TargetKind::ClaudeCode,
            300,
            &[("settings.json", "{}")],
        );
        let removed = prune_target_backups_in(root, TargetKind::ClaudeCode, 2).unwrap();
        assert_eq!(removed, 1);
        let listed =
            list_backups_in(root, &[TargetKind::ClaudeCode], |_| (None, None, None)).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].created_at, 300);
        assert_eq!(listed[1].created_at, 200);
    }

    #[test]
    fn prune_drops_empty_dirs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let empty = root.join("claude_code").join("1");
        fs::create_dir_all(&empty).unwrap();
        stamp_dir(root, TargetKind::ClaudeCode, 2, &[("settings.json", "{}")]);
        prune_target_backups_in(root, TargetKind::ClaudeCode, 30).unwrap();
        assert!(!empty.exists());
    }

    #[test]
    fn preview_redacts_secrets() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let fixture_token = format!("test-{}-abcdefghijklmnop", "key");
        let fixture_json = format!(
            r#"{{"env":{{"ANTHROPIC_AUTH_TOKEN":"{}","ANTHROPIC_MODEL":"gpt-5.6"}}}}"#,
            fixture_token
        );
        stamp_dir(
            root,
            TargetKind::ClaudeCode,
            9,
            &[("settings.json", &fixture_json)],
        );
        let preview = preview_backup_in(root, "claude_code-9").unwrap();
        assert_eq!(
            preview
                .summary
                .get("ANTHROPIC_MODEL")
                .cloned()
                .flatten()
                .as_deref(),
            Some("gpt-5.6")
        );
        let token = preview
            .summary
            .get("ANTHROPIC_AUTH_TOKEN")
            .cloned()
            .flatten()
            .unwrap();
        assert!(!token.contains("abcdefghijklmnop"));
        assert!(token.contains('…'));
    }

    #[test]
    fn restore_overwrites_dest() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        stamp_dir(
            root,
            TargetKind::Codex,
            42,
            &[("config.toml", "model = \"old\"\n")],
        );
        let dest = tmp.path().join("live-config.toml");
        fs::write(&dest, "model = \"current\"\n").unwrap();
        let mut map = HashMap::new();
        map.insert("config.toml".into(), dest.clone());
        restore_backup_in(root, "codex-42", &AppSettings::default(), Some(&map)).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "model = \"old\"\n");
    }

    #[test]
    fn pi_backup_maps_all_three_files_and_secures_auth() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("backups");
        stamp_dir(
            &root,
            TargetKind::Pi,
            43,
            &[
                ("models.json", "{\"providers\":{}}"),
                ("auth.json", "{}"),
                ("settings.json", "{}"),
            ],
        );
        let agent = tmp.path().join("agent");
        let mut settings = AppSettings::default();
        settings.pi_agent_dir_override = Some(agent.display().to_string());
        let restored = restore_backup_in(&root, "pi-43", &settings, None).unwrap();
        assert_eq!(restored.len(), 3);
        assert!(agent.join("models.json").exists());
        assert!(agent.join("auth.json").exists());
        assert!(agent.join("settings.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(agent.join("auth.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn prime_backup_maps_all_three_files_and_secures_auth() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("backups");
        stamp_dir(
            &root,
            TargetKind::Prime,
            44,
            &[
                ("models.json", "{\"providers\":{}}"),
                ("auth.json", "{}"),
                ("settings.json", "{}"),
            ],
        );
        let agent = tmp.path().join("prime-agent");
        let mut settings = AppSettings::default();
        settings.prime_agent_dir_override = Some(agent.display().to_string());
        let restored = restore_backup_in(&root, "prime-44", &settings, None).unwrap();
        assert_eq!(restored.len(), 3);
        assert!(agent.join("models.json").exists());
        assert!(agent.join("auth.json").exists());
        assert!(agent.join("settings.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(agent.join("auth.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn delete_removes_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = stamp_dir(root, TargetKind::Codex, 7, &[("config.toml", "x")]);
        delete_backup_in(root, "codex-7").unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn default_retention_is_30() {
        assert_eq!(default_max_backup_copies(), 30);
        assert_eq!(clamp_max_backup_copies(0), 1);
        assert_eq!(clamp_max_backup_copies(999), 200);
    }

    #[test]
    fn old_settings_json_gets_default_retention() {
        let json = r##"{
            "language":"zh-CN",
            "themeMode":"system",
            "primaryColor":"#1677ff",
            "autoStart":false,
            "alwaysOnTop":false,
            "claudeHomeOverride":null,
            "codexHomeOverride":null,
            "codexEnvInjectMode":"auto",
            "forceExclusiveClaudeAuthKey":false,
            "autoCheckUpdate":true
        }"##;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.max_backup_copies, 30);
        assert_eq!(s.update_check_interval, 60);
        assert!(s.auto_check_update);
    }
}
