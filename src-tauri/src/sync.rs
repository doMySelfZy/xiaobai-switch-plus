use crate::app_backup;
use crate::domain::SyncOutcome;
use crate::error::{AppError, AppResult};
use crate::repo;
use crate::state::AppState;
use crate::webdav::WebDavClient;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// WebDAV 同步版本指针文件名。属于跨机内部协议标识：更名前后必须保持一致，
/// 否则新旧版本机器互相读不到版本指针，会误判并相互覆盖。
pub const SYNC_MANIFEST_FILE_NAME: &str = "xiaobai-switch-sync.json";
pub const SYNC_FORMAT_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

const META_LAST_SYNCED_DATABASE_SHA256: &str = "last_synced_database_sha256";
const META_LAST_SYNCED_MASTER_KEY_SHA256: &str = "last_synced_master_key_sha256";
const META_LAST_PUBLISHED_REVISION: &str = "last_published_revision";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFingerprint {
    pub database_sha256: String,
    pub master_key_sha256: String,
}

impl DataFingerprint {
    fn from_parts(database_sha256: String, master_key_sha256: String) -> Self {
        Self {
            database_sha256,
            master_key_sha256,
        }
    }
}

/// 参与内容指纹的业务表（引擎记账表与 WebDAV 配置不参与）。
const FINGERPRINT_TABLES: [&str; 9] = [
    "settings",
    "sites",
    "site_api_keys",
    "site_models",
    "site_thinking_presets",
    "target_bindings",
    "apply_records",
    "mcp_servers",
    "agent_rules",
];

/// 逻辑指纹的算法版本，与 `FINGERPRINT_TABLES` 同源维护：表清单任何增删都必须同步递增。
/// 两端算法不同会对同一份数据算出不同指纹，判定结果恒定相反，进而反复互相覆盖
/// （0.1.3 的 8 表与 0.1.4+ 的 9 表真实发生过这种循环）。
pub const FINGERPRINT_ALGORITHM_VERSION: u32 = 1;

/// 未修复版本（manifest 里没有 `fingerprintAlgorithm` 字段）写出时**实际**使用的算法版本：
/// 0.1.4/0.1.5 的 9 表实现。缺字段的 manifest 必须固定按它处理，**不能**回退到"当前版本"——
/// 否则下次表清单变更、版本递增之后，旧对端的 manifest 会被当成本机算法照常比较指纹，
/// 跨算法互相覆盖的缺陷就会原样复现。
const LEGACY_FINGERPRINT_ALGORITHM_VERSION: u32 = 1;

/// 远端声明的算法版本（`None` = 未修复版本写出）是否与本机算法一致。
fn algorithm_matches(declared: Option<u32>, current: u32) -> bool {
    declared.unwrap_or(LEGACY_FINGERPRINT_ALGORITHM_VERSION) == current
}

/// 逻辑内容指纹：按表遍历全部业务行做稳定哈希。
/// 不使用数据库文件字节 hash——SQLite 文件头部（change counter 等）每次
/// VACUUM 都会变化，会让"内容没变指纹却变了"，导致同步误判反复应用。
pub fn compute_logical_fingerprint(
    conn: &Connection,
    master_key: &[u8],
) -> AppResult<DataFingerprint> {
    use rusqlite::types::Value;
    let mut hasher = Sha256::new();
    for table in FINGERPRINT_TABLES {
        hasher.update(table.as_bytes());
        hasher.update([0]);
        let mut statement = conn.prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))?;
        let column_count = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            for index in 0..column_count {
                let value: Value = row.get(index)?;
                match value {
                    Value::Null => hasher.update([0_u8]),
                    Value::Integer(number) => {
                        hasher.update([1_u8]);
                        hasher.update(number.to_le_bytes());
                    }
                    Value::Real(number) => {
                        hasher.update([2_u8]);
                        hasher.update(number.to_le_bytes());
                    }
                    Value::Text(ref text) => {
                        hasher.update([3_u8]);
                        hasher.update((text.len() as u64).to_le_bytes());
                        hasher.update(text.as_bytes());
                    }
                    Value::Blob(ref bytes) => {
                        hasher.update([4_u8]);
                        hasher.update((bytes.len() as u64).to_le_bytes());
                        hasher.update(bytes);
                    }
                }
            }
            hasher.update([255_u8]);
        }
        hasher.update([254_u8]);
    }
    Ok(DataFingerprint::from_parts(
        hex::encode(hasher.finalize()),
        hex::encode(Sha256::digest(master_key)),
    ))
}

/// 对一个数据库文件计算逻辑指纹（校验下载 bundle 的内容时使用）。
pub fn fingerprint_database_file(database_path: &std::path::Path, master_key: &[u8]) -> AppResult<DataFingerprint> {
    let conn = Connection::open(database_path)
        .map_err(|e| AppError::new("sync_manifest_invalid", format!("cannot open extracted database: {e}")))?;
    compute_logical_fingerprint(&conn, master_key)
}

/// 远端"版本指针"：谁在何时上传了哪个数据包。
/// 判断新旧唯一依据是数据指纹；revision 只用于展示与遥测。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncManifest {
    pub format_version: u32,
    pub revision: u64,
    pub device_name: String,
    pub updated_at: i64,
    pub bundle_file_name: String,
    pub database_sha256: String,
    pub master_key_sha256: String,
    pub app_version: String,
    /// 写出该 manifest 的机器的指纹算法版本。
    /// 缺省 = 未修复版本写出（0.1.4/0.1.5 的 9 表实现），按当前算法处理；
    /// 可忽略字段，保证未修复版本仍能读取新 manifest（可回滚）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint_algorithm: Option<u32>,
}

impl SyncManifest {
    fn fingerprint(&self) -> DataFingerprint {
        DataFingerprint {
            database_sha256: self.database_sha256.clone(),
            master_key_sha256: self.master_key_sha256.clone(),
        }
    }

    /// 远端声明的算法版本；缺省 = 未修复版本写出，按 [`LEGACY_FINGERPRINT_ALGORITHM_VERSION`] 处理。
    fn declared_algorithm(&self) -> u32 {
        self.fingerprint_algorithm
            .unwrap_or(LEGACY_FINGERPRINT_ALGORITHM_VERSION)
    }

    fn algorithm_matches_current(&self) -> bool {
        algorithm_matches(self.fingerprint_algorithm, FINGERPRINT_ALGORITHM_VERSION)
    }

    fn validate(&self) -> AppResult<()> {
        if self.format_version != SYNC_FORMAT_VERSION {
            return Err(AppError::new(
                "sync_manifest_invalid",
                format!("unsupported sync manifest format: {}", self.format_version),
            ));
        }
        // 未知的算法版本不是有效性错误（是否兼容由决策处理），只有 0 这种非法值才拒绝。
        if self.fingerprint_algorithm == Some(0) {
            return Err(AppError::new(
                "sync_manifest_invalid",
                "sync manifest fingerprint algorithm version must be >= 1",
            ));
        }
        if self.revision == 0 || self.device_name.trim().is_empty() {
            return Err(AppError::new(
                "sync_manifest_invalid",
                "sync manifest revision and device are required",
            ));
        }
        crate::webdav::validate_backup_file_name(&self.bundle_file_name)?;
        for hash in [&self.database_sha256, &self.master_key_sha256] {
            if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(AppError::new(
                    "sync_manifest_invalid",
                    "sync manifest checksums must be sha256 hex digests",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Upload,
    Download,
    InSync,
    /// 远端指纹算法与本机不同：禁止上传与下载两种替换动作，否则会互相覆盖。
    Incompatible,
}

/// 纯决策：比较本地指纹、远端指纹与上次同步指纹。
/// 冲突（两侧都变过）在 Phase A 策略下以远端为准，本地先快照留底。
fn decide_action(
    local: &DataFingerprint,
    remote: Option<&SyncManifest>,
    last_synced: Option<&DataFingerprint>,
) -> (SyncAction, bool) {
    let Some(remote) = remote else {
        return (SyncAction::Upload, false);
    };
    // 算法检查必须先于指纹比较：算法不同的两端对同一份数据算出不同指纹，
    // 继续比较只会得到恒定相反的结论。
    if !remote.algorithm_matches_current() {
        return (SyncAction::Incompatible, false);
    }
    let remote_fp = remote.fingerprint();
    if remote_fp == *local {
        return (SyncAction::InSync, false);
    }
    match last_synced {
        // 首次接触：以云端为准（本地未同步的改动已被快照兜底）。
        None => (SyncAction::Download, false),
        Some(last) if remote_fp == *last => (SyncAction::Upload, false),
        Some(last) if *last == *local => (SyncAction::Download, false),
        Some(_) => (SyncAction::Download, true),
    }
}

pub async fn run_sync(
    app: &AppHandle,
    state: &AppState,
    reason: &str,
) -> AppResult<SyncOutcome> {
    let _guard = state.webdav_operation.lock().await;
    state
        .db
        .with_conn(|conn| repo::webdav::record_sync_status(conn, "running", None, false))?;

    let outcome = run_sync_inner(app, state, reason).await;

    match &outcome {
        Ok(outcome) => {
            let status = if outcome.conflict || outcome.warning.is_some() {
                "warning"
            } else {
                "success"
            };
            let note = outcome.warning.clone().or_else(|| {
                outcome.conflict.then(|| {
                    "sync conflict: both sides changed since the last sync; applied the remote data after taking a local snapshot".to_string()
                })
            });
            state.db.with_conn(|conn| {
                repo::webdav::record_sync_status(conn, status, note.as_deref(), true)
            })?;
        }
        Err(error) => {
            let message = error.to_string();
            state.db.with_conn(|conn| {
                repo::webdav::record_sync_status(conn, "failed", Some(&message), false)
            })?;
        }
    }
    outcome
}

async fn run_sync_inner(
    app: &AppHandle,
    state: &AppState,
    reason: &str,
) -> AppResult<SyncOutcome> {
    let stored = state
        .db
        .with_conn(repo::webdav::get_config)?
        .ok_or_else(|| AppError::new("webdav_not_configured", "WebDAV is not configured"))?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let client = WebDavClient::new(
        crate::commands::webdav::runtime_config(&stored, &state.crypto)?,
        &settings,
    )?;

    let app_dir = crate::paths::app_dir()?;
    let key_bytes = std::fs::read(&crate::paths::master_key_path()?)
        .map_err(|e| AppError::new("sync_failed", format!("cannot read master.key: {e}")))?;
    if key_bytes.len() != 32 {
        return Err(AppError::new("sync_failed", "master.key must contain exactly 32 bytes"));
    }
    let local_fp = state
        .db
        .with_conn(|conn| compute_logical_fingerprint(conn, &key_bytes))?;

    let remote = client.download_sync_manifest().await?;
    let last_synced = load_last_synced(state)?;
    let (action, conflict) = decide_action(&local_fp, remote.as_ref(), last_synced.as_ref());

    match action {
        SyncAction::InSync => {
            state.db.with_conn(|conn| save_last_synced(conn, &local_fp))?;
            Ok(SyncOutcome {
                action: "in_sync".into(),
                revision: remote.map(|manifest| manifest.revision).unwrap_or(0),
                bundle_file_name: None,
                conflict: false,
                pending_restart: false,
                warning: None,
            })
        }
        // 算法不兼容：既不读也不写远端数据，等对端升级后再同步。
        SyncAction::Incompatible => {
            let remote = remote.expect("incompatible decision requires a remote manifest");
            Err(AppError::new(
                "sync_algorithm_mismatch",
                format!(
                    "remote fingerprint algorithm {} does not match local {}; upgrade the other device",
                    remote.declared_algorithm(),
                    FINGERPRINT_ALGORITHM_VERSION
                ),
            ))
        }
        SyncAction::Upload => {
            // 全库复制（VACUUM + zip）只在上传时做：决策前打包会让"已同步/下载"白付一次开销。
            let temp_dir = tempfile::Builder::new()
                .prefix(".sync-")
                .tempdir_in(&app_dir)?;
            // 锁内只做需要连接的 `VACUUM INTO`；sha256 与 zip 打包只需要那个独立快照文件，
            // 放到锁外做——否则整段时间都占着全局数据库锁，所有 UI 读命令排队。
            let snapshot = state
                .db
                .with_conn(|conn| app_backup::snapshot_database(conn, temp_dir.path()))?;
            let local_bundle = app_backup::pack_snapshot(
                &snapshot,
                &crate::paths::master_key_path()?,
                temp_dir.path(),
                reason,
            )?;
            let device_name = app_backup::parse_device_from_filename(&local_bundle.file_name);
            let next_revision = remote.as_ref().map(|m| m.revision + 1).unwrap_or(1);
            client
                .upload_file(&local_bundle.file_name, &local_bundle.path)
                .await?;
            let manifest = SyncManifest {
                format_version: SYNC_FORMAT_VERSION,
                revision: next_revision,
                device_name: device_name.clone(),
                updated_at: chrono::Utc::now().timestamp_millis(),
                bundle_file_name: local_bundle.file_name.clone(),
                database_sha256: local_fp.database_sha256.clone(),
                master_key_sha256: local_fp.master_key_sha256.clone(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                fingerprint_algorithm: Some(FINGERPRINT_ALGORITHM_VERSION),
            };
            client.upload_sync_manifest(&manifest).await?;
            // 按保留数清理本设备在云端的旧数据包，防止无限堆积。
            let warning = client
                .cleanup_device_backups(&device_name, stored.max_remote_backups)
                .await
                .err()
                .map(|error| {
                    format!("synced, but old remote bundles could not be pruned: {error}")
                });
            state.db.with_conn(|conn| save_last_synced(conn, &local_fp))?;
            state.db.with_conn(|conn| {
                repo::sync_meta::set_meta(
                    conn,
                    META_LAST_PUBLISHED_REVISION,
                    &next_revision.to_string(),
                )
            })?;
            Ok(SyncOutcome {
                action: "upload".into(),
                revision: next_revision,
                bundle_file_name: Some(local_bundle.file_name),
                conflict: false,
                pending_restart: false,
                warning,
            })
        }
        SyncAction::Download => {
            let remote = remote.expect("download decision requires a remote manifest");
            let temp_dir = tempfile::Builder::new()
                .prefix(".sync-")
                .tempdir_in(&app_dir)?;
            let archive = temp_dir.path().join(&remote.bundle_file_name);
            client
                .download_file(&remote.bundle_file_name, &archive)
                .await?;
            let extract_dir = tempfile::Builder::new()
                .prefix(".sync-apply-")
                .tempdir_in(&app_dir)?;
            let validated =
                app_backup::validate_and_extract_bundle(&archive, extract_dir.path())?;
            // 内容校验：解包后的数据库逻辑指纹必须与远端版本指针一致
            // （zip 内部的文件级 sha256 校验已在 validate_and_extract_bundle 完成）。
            let extracted_key = std::fs::read(&validated.master_key_path)?;
            let extracted_fp =
                fingerprint_database_file(&validated.database_path, &extracted_key)?;
            if extracted_fp != remote.fingerprint() {
                return Err(AppError::new(
                    "sync_manifest_invalid",
                    "downloaded bundle does not match the remote sync manifest",
                ));
            }
            // 应用前强制本地快照：任何远端数据替换都可回滚。
            // （只有 `VACUUM INTO` 占库锁，hash/打包在锁外做）
            app_backup::create_local_backup(
                &state.db,
                "pre_sync_apply",
                settings.max_backup_copies,
            )?;
            // 记账延后：换库要等下次启动，此刻提交 last_synced 会留下"远端即共同祖先"的假账，
            // 一旦重启前退出或恢复失败，下一轮就会判定"只有本地变了"并用旧数据覆盖云端。
            crate::pending_restore::queue_pending_restore(
                &archive,
                &app_dir,
                Some(remote.fingerprint()),
            )?;
            Ok(SyncOutcome {
                action: "download".into(),
                revision: remote.revision,
                bundle_file_name: Some(remote.bundle_file_name),
                conflict,
                pending_restart: true,
                warning: None,
            })
        }
    }
}

fn load_last_synced(state: &AppState) -> AppResult<Option<DataFingerprint>> {
    state.db.with_conn(load_last_synced_conn)
}

fn load_last_synced_conn(conn: &Connection) -> AppResult<Option<DataFingerprint>> {
    let database = repo::sync_meta::get_meta(conn, META_LAST_SYNCED_DATABASE_SHA256)?;
    let master_key = repo::sync_meta::get_meta(conn, META_LAST_SYNCED_MASTER_KEY_SHA256)?;
    Ok(match (database, master_key) {
        (Some(database_sha256), Some(master_key_sha256)) => Some(DataFingerprint {
            database_sha256,
            master_key_sha256,
        }),
        _ => None,
    })
}

/// 提交同步记账。接收 Connection 而非 AppState，供启动流程在 `Db::open` 之后
/// 提交"下载已真正落地"的目标指纹复用。
pub(crate) fn save_last_synced(conn: &Connection, fingerprint: &DataFingerprint) -> AppResult<()> {
    repo::sync_meta::set_meta(
        conn,
        META_LAST_SYNCED_DATABASE_SHA256,
        &fingerprint.database_sha256,
    )?;
    repo::sync_meta::set_meta(
        conn,
        META_LAST_SYNCED_MASTER_KEY_SHA256,
        &fingerprint.master_key_sha256,
    )
}

// ---------------------------------------------------------------------------
// Phase B：变更驱动触发
// ---------------------------------------------------------------------------

static SYNC_DIRTY: AtomicBool = AtomicBool::new(false);
static SYNC_POLL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 引擎自身的记账表，写入它们不算"数据变更"。
const NOISY_SYNC_TABLES: [&str; 2] = ["sync_meta", "webdav_sync_state"];

pub(crate) fn is_noisy_sync_table(table: &str) -> bool {
    NOISY_SYNC_TABLES.contains(&table)
}

/// SQLite update_hook 回调：任何业务数据写入都会标记脏。
pub fn note_db_write(table: &str) {
    if !is_noisy_sync_table(table) {
        SYNC_DIRTY.store(true, Ordering::Relaxed);
    }
}

/// 请求做一次完整同步决策（启动、窗口聚焦时调用）。
pub fn request_sync_poll() {
    SYNC_POLL_REQUESTED.store(true, Ordering::Relaxed);
}

/// 给应用数据库挂上变更钩子（Db::open 时调用一次）。
pub fn install_db_hook(conn: &rusqlite::Connection) {
    // 用具名函数而非闭包：闭包在这里会触发 HRTB 生命周期推断失败。
    conn.update_hook(Some(sync_update_hook));
}

fn sync_update_hook(_action: rusqlite::hooks::Action, _db: &str, table: &str, _rowid: i64) {
    note_db_write(table);
}

/// 同步守护任务：5 秒一轮，处理三类触发——
/// 1) 启动后首轮决策（换机打开即拉取）；2) 数据变更（防抖后上传）；
/// 3) 窗口聚焦（切回窗口时决策一次）。定时器只作为兜底留在调度器里。
pub fn spawn_sync_daemon(app: AppHandle) {
    request_sync_poll();
    tauri::async_runtime::spawn(async move {
        let mut first_poll_done = false;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let dirty = SYNC_DIRTY.swap(false, Ordering::Relaxed);
            let poll = SYNC_POLL_REQUESTED.swap(false, Ordering::Relaxed);
            if !dirty && !poll {
                continue;
            }
            // 防抖：等连续写突发平息后再打包。
            tokio::time::sleep(Duration::from_secs(3)).await;
            let state = app.state::<AppState>();
            let enabled = state.db.with_conn(|conn| {
                Ok(repo::webdav::get_config(conn)?.is_some_and(|config| config.auto_sync_enabled))
            });
            let enabled = match enabled {
                Ok(enabled) => enabled,
                Err(error) => {
                    tracing::warn!(error = %error, "sync daemon config check failed");
                    continue;
                }
            };
            if !enabled {
                continue;
            }
            let reason = if dirty && !poll {
                "auto"
            } else if !first_poll_done {
                first_poll_done = true;
                "startup"
            } else {
                "poll"
            };
            match run_sync(&app, &state, reason).await {
                Ok(outcome) => {
                    if outcome.pending_restart {
                        drop(state);
                        crate::commands::webdav::relaunch_after_restore(app.clone());
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, reason, "background sync failed");
                }
            }
        }
    });
}

/// 把远端 bundle 复制到 staging 并解析出它的指纹（供引擎与测试复用）。
pub fn parse_remote_manifest_bytes(bytes: &[u8]) -> AppResult<SyncManifest> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(AppError::new(
            "sync_manifest_invalid",
            "sync manifest is too large",
        ));
    }
    let manifest: SyncManifest = serde_json::from_slice(bytes)
        .map_err(|error| AppError::new("sync_manifest_invalid", format!("invalid sync manifest: {error}")))?;
    manifest.validate()?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(db: &str, key: &str) -> DataFingerprint {
        DataFingerprint {
            database_sha256: sha64(db),
            master_key_sha256: sha64(key),
        }
    }

    fn sha64(seed: &str) -> String {
        format!("{:0>64}", seed)
    }

    fn manifest(db: &str, key: &str, revision: u64) -> SyncManifest {
        SyncManifest {
            format_version: SYNC_FORMAT_VERSION,
            revision,
            device_name: "work-pc".into(),
            updated_at: 1_000,
            bundle_file_name: "xiaobai-switch-backup-20260912_000000.work-pc.abcdef01.zip".into(),
            database_sha256: sha64(db),
            master_key_sha256: sha64(key),
            app_version: "0.0.0".into(),
            fingerprint_algorithm: Some(FINGERPRINT_ALGORITHM_VERSION),
        }
    }

    /// 未修复版本（0.1.4/0.1.5）写出的 manifest：不带指纹算法字段。
    fn legacy_manifest(db: &str, key: &str, revision: u64) -> SyncManifest {
        SyncManifest {
            fingerprint_algorithm: None,
            ..manifest(db, key, revision)
        }
    }

    #[test]
    fn uploads_when_remote_is_empty() {
        let local = fingerprint("a", "f");
        let (action, conflict) = decide_action(&local, None, None);
        assert_eq!(action, SyncAction::Upload);
        assert!(!conflict);
    }

    #[test]
    fn in_sync_when_fingerprints_match() {
        let local = fingerprint("a", "f");
        let remote = manifest("a", "f", 3);
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&local));
        assert_eq!(action, SyncAction::InSync);
        assert!(!conflict);
    }

    #[test]
    fn downloads_when_only_remote_changed() {
        let local = fingerprint("a", "f");
        let remote = manifest("b", "f", 4);
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&local));
        assert_eq!(action, SyncAction::Download);
        assert!(!conflict);
    }

    #[test]
    fn uploads_when_only_local_changed() {
        let local = fingerprint("b", "f");
        let remote = manifest("a", "f", 4);
        let synced = fingerprint("a", "f");
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&synced));
        assert_eq!(action, SyncAction::Upload);
        assert!(!conflict);
    }

    #[test]
    fn flags_conflict_when_both_sides_changed() {
        let local = fingerprint("c", "f");
        let remote = manifest("b", "f", 4);
        let synced = fingerprint("a", "f");
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&synced));
        assert_eq!(action, SyncAction::Download);
        assert!(conflict);
    }

    #[test]
    fn downloads_on_first_contact_without_conflict() {
        let local = fingerprint("a", "f");
        let remote = manifest("b", "f", 7);
        let (action, conflict) = decide_action(&local, Some(&remote), None);
        assert_eq!(action, SyncAction::Download);
        assert!(!conflict);
    }

    #[test]
    fn flags_incompatible_when_fingerprint_algorithm_differs() {
        // 算法不同时禁止任何替换动作：先做算法检查，指纹相同也要拦下。
        let local = fingerprint("a", "f");
        let mut remote = manifest("a", "f", 3);
        remote.fingerprint_algorithm = Some(FINGERPRINT_ALGORITHM_VERSION + 1);
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&local));
        assert_eq!(action, SyncAction::Incompatible);
        assert!(!conflict);

        // 只有远端变过也一样：跨算法比较没有意义。
        let remote = {
            let mut remote = manifest("b", "f", 4);
            remote.fingerprint_algorithm = Some(FINGERPRINT_ALGORITHM_VERSION + 1);
            remote
        };
        let (action, conflict) = decide_action(&local, Some(&remote), Some(&local));
        assert_eq!(action, SyncAction::Incompatible);
        assert!(!conflict);
    }

    #[test]
    fn legacy_manifest_without_algorithm_is_compared_as_current() {
        // 缺字段 = 未修复版本写出（实为 9 表实现），按当前算法照常比较，
        // 否则升级后会出现"必须上传才能补字段、但缺字段又禁止上传"的死锁。
        let local = fingerprint("a", "f");
        let legacy = legacy_manifest("a", "f", 3);
        let (action, conflict) = decide_action(&local, Some(&legacy), Some(&local));
        assert_eq!(action, SyncAction::InSync);
        assert!(!conflict);

        let legacy = legacy_manifest("b", "f", 4);
        let (action, conflict) = decide_action(&local, Some(&legacy), Some(&local));
        assert_eq!(action, SyncAction::Download);
        assert!(!conflict);
    }

    #[test]
    fn missing_algorithm_field_stays_pinned_to_the_legacy_version() {
        // 缺字段必须固定等价于「旧版 9 表算法」，而不是「等价于当前算法」：
        // 否则表清单下一次变更（版本递增）后，旧对端的 manifest 会被当成本机算法照常比较，
        // 跨算法互相覆盖的缺陷原样复现。这条断言把该语义钉死。
        assert!(algorithm_matches(None, LEGACY_FINGERPRINT_ALGORITHM_VERSION));
        assert!(!algorithm_matches(
            None,
            LEGACY_FINGERPRINT_ALGORITHM_VERSION + 1
        ));
        assert_eq!(
            LEGACY_FINGERPRINT_ALGORITHM_VERSION,
            1,
            "旧版（0.1.4/0.1.5）manifest 无版本字段，对应 9 表实现 = 算法 1"
        );
        // 有字段时严格按声明值比较。
        assert!(algorithm_matches(Some(2), 2));
        assert!(!algorithm_matches(Some(2), 3));
    }

    #[test]
    fn fingerprint_algorithm_version_is_pinned_to_the_table_list() {
        // 算法版本 → 表清单快照。表清单决定指纹，改动它而不递增版本号会让两端互相覆盖，
        // 这条断言就是护栏：递增 FINGERPRINT_ALGORITHM_VERSION 时必须在此补上新快照。
        const VERSION_1_TABLES: [&str; 9] = [
            "settings",
            "sites",
            "site_api_keys",
            "site_models",
            "site_thinking_presets",
            "target_bindings",
            "apply_records",
            "mcp_servers",
            "agent_rules",
        ];
        assert_eq!(FINGERPRINT_ALGORITHM_VERSION, 1);
        assert_eq!(FINGERPRINT_TABLES.len(), 9);
        assert_eq!(FINGERPRINT_TABLES, VERSION_1_TABLES);
    }

    #[test]
    fn manifest_round_trips_and_validates() {
        let remote = manifest("a", "f", 2);
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("fingerprintAlgorithm"));
        assert_eq!(parse_remote_manifest_bytes(&bytes).unwrap(), remote);

        // 未修复版本写出的 manifest（无新字段）继续可读，且原样忽略该字段的缺失。
        let legacy = legacy_manifest("a", "f", 2);
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("fingerprintAlgorithm"));
        assert_eq!(parse_remote_manifest_bytes(&bytes).unwrap(), legacy);

        // 反向（未修复版本读新 manifest）依赖"无 deny_unknown_fields"：未知字段必须被忽略。
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["fingerprintAlgorithm"] = serde_json::json!(1);
        value["futureFieldFromNewerVersion"] = serde_json::json!("ignored");
        assert!(parse_remote_manifest_bytes(&serde_json::to_vec(&value).unwrap()).is_ok());
    }

    #[test]
    fn rejects_tampered_manifests() {
        let mut remote = manifest("a", "f", 2);
        remote.format_version = 99;
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_err());

        let remote = manifest("a", "f", 0);
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_err());

        let mut remote = manifest("short", "k", 2);
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_err());

        let mut remote = manifest("a", "f", 2);
        remote.bundle_file_name = "../escape.zip".into();
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_err());

        // 算法版本 0 是非法值；未知版本不是有效性错误（由决策判定不兼容）。
        let mut remote = manifest("a", "f", 2);
        remote.fingerprint_algorithm = Some(0);
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_err());

        let mut remote = manifest("a", "f", 2);
        remote.fingerprint_algorithm = Some(FINGERPRINT_ALGORITHM_VERSION + 1);
        let bytes = serde_json::to_vec(&remote).unwrap();
        assert!(parse_remote_manifest_bytes(&bytes).is_ok());
    }

    #[test]
    fn last_synced_round_trips_the_given_fingerprint() {
        // 下载路径的记账必须原样提交远端 manifest 声明的指纹：
        // 恢复后的库若被 schema 迁移改写，本地重算值与远端值不同，误记会导致反复下载。
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        assert_eq!(load_last_synced_conn(&conn).unwrap(), None);

        let remote = fingerprint("remote-declared", "key");
        save_last_synced(&conn, &remote).unwrap();
        assert_eq!(load_last_synced_conn(&conn).unwrap(), Some(remote.clone()));

        let local_recomputed = fingerprint("local-recomputed", "key");
        save_last_synced(&conn, &local_recomputed).unwrap();
        assert_eq!(load_last_synced_conn(&conn).unwrap(), Some(local_recomputed));
    }

    /// 打包时机的结构护栏：完整引擎路径需要 AppHandle 与真实 WebDAV 服务，无法在单测里跑通，
    /// 这里直接对 `run_sync_inner` 源码分段断言——整库复制只允许出现在 Upload 分支，
    /// 且 Download 必须保留应用前的本地快照。
    ///
    /// 快照拆成"锁内 `VACUUM INTO` + 锁外 hash/zip"后，Upload 分支不再出现
    /// `create_backup_in`（它仍保留给只需要一键完成的调用方），因此护栏改为锁定
    /// `snapshot_database` 出现在 `with_conn` 内、`pack_snapshot` 出现在其后。
    #[test]
    fn whole_database_bundle_is_built_only_for_uploads() {
        fn segment<'a>(body: &'a str, from: &str, to: Option<&str>) -> &'a str {
            let start = body.find(from).unwrap();
            match to {
                Some(to) => {
                    let end = body.find(to).unwrap();
                    assert!(start < end, "{from} must precede {to}");
                    &body[start..end]
                }
                None => &body[start..],
            }
        }

        let source = include_str!("sync.rs");
        let body_start = source.find("async fn run_sync_inner").unwrap();
        let body_end = source.find("fn load_last_synced").unwrap();
        let body = &source[body_start..body_end];

        // 决策前不做任何打包/快照：InSync 与 Incompatible 由此天然零副作用。
        let before_decision = segment(body, "async fn run_sync_inner", Some("match action {"));
        assert!(!before_decision.contains("create_backup_in"));
        assert!(!before_decision.contains("create_local_backup"));
        assert!(!before_decision.contains("snapshot_database"));

        let upload = segment(body, "SyncAction::Upload =>", Some("SyncAction::Download =>"));
        // 全库复制只允许一次，且 `VACUUM INTO` 必须在库锁内、打包必须在库锁外。
        assert_eq!(upload.matches("snapshot_database").count(), 1);
        assert_eq!(upload.matches("pack_snapshot").count(), 1);
        assert!(
            upload.contains("with_conn(|conn| app_backup::snapshot_database(conn,"),
            "只有 VACUUM INTO 需要数据库连接，它必须留在 with_conn 内"
        );
        let snapshot_at = upload.find("snapshot_database").unwrap();
        let pack_at = upload.find("pack_snapshot").unwrap();
        assert!(
            snapshot_at < pack_at,
            "先取快照再打包：打包读的是已经完整的快照文件"
        );
        // `create_backup_in` 会把 hash/zip 一起塞回库锁里，上传路径不得使用。
        assert!(!upload.contains("create_backup_in"));
        assert!(!upload.contains("create_local_backup"));

        let download = segment(body, "SyncAction::Download =>", None);
        assert!(!download.contains("create_backup_in"), "下载分支不得打包本机数据");
        assert!(!download.contains("snapshot_database"));
        assert_eq!(download.matches("create_local_backup").count(), 1);
        assert!(download.contains("pre_sync_apply"));
        assert!(download.contains("queue_pending_restore"));
        // 记账必须延后到恢复真正落地，排队时不得提交。
        assert!(!download.contains("save_last_synced"));

        let in_sync = segment(body, "SyncAction::InSync =>", Some("SyncAction::Incompatible =>"));
        assert!(!in_sync.contains("create_backup_in"));
        assert!(!in_sync.contains("snapshot_database"));
        let incompatible = segment(body, "SyncAction::Incompatible =>", Some("SyncAction::Upload =>"));
        assert!(!incompatible.contains("create_backup_in"));
        assert!(!incompatible.contains("create_local_backup"));
        assert!(!incompatible.contains("snapshot_database"));
        assert!(!incompatible.contains("save_last_synced"));
    }

    #[test]
    fn fingerprint_equality_uses_both_hashes() {
        assert_eq!(fingerprint("a", "f"), fingerprint("a", "f"));
        assert_ne!(fingerprint("a", "f"), fingerprint("a", "f2"));
    }

    #[test]
    fn logical_fingerprint_is_stable_across_vacuum_and_detects_changes() {
        let source = tempfile::tempdir().unwrap();
        let db_path = source.path().join("data.db");
        let conn = Connection::open(&db_path).unwrap();
        crate::db::apply_schema(&conn).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (id, json) VALUES (1, '{\"a\":1}')",
            [],
        )
        .unwrap();
        let key = [7_u8; 32];
        let fingerprint = compute_logical_fingerprint(&conn, &key).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();

        // VACUUM 出字节布局不同的副本，逻辑指纹必须保持一致。
        let copy_path = source.path().join("copy.db");
        let copy_arg = copy_path.to_string_lossy().to_string();
        conn.execute("VACUUM INTO ?", [&copy_arg]).unwrap();
        let copy_fp = fingerprint_database_file(&copy_path, &key).unwrap();
        assert_eq!(fingerprint, copy_fp);

        // 逻辑内容变化必须改变指纹。
        conn.execute(
            "INSERT OR REPLACE INTO settings (id, json) VALUES (1, '{\"a\":2}')",
            [],
        )
        .unwrap();
        let changed = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_ne!(fingerprint, changed);
    }

    #[test]
    fn fingerprint_tracks_mcp_server_changes() {
        // 回归：MCP 表不参与指纹时，新增/修改 MCP 不会被判定为数据变更，
        // 跨设备同步就永远不会发布这份配置。
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let key = [5_u8; 32];
        let before = compute_logical_fingerprint(&conn, &key).unwrap();

        conn.execute(
            "INSERT INTO mcp_servers
               (id, name, kind, enabled, targets_json, config_json, secrets_encrypted, created_at, updated_at)
             VALUES ('m1', 'demo', 'stdio', 1, '[\"claude_code\"]', '{\"command\":\"x\"}', NULL, 1, 1)",
            [],
        )
        .unwrap();
        let added = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_ne!(before, added, "adding an MCP server must change the fingerprint");

        conn.execute(
            "UPDATE mcp_servers SET enabled = 0, updated_at = 2 WHERE id = 'm1'",
            [],
        )
        .unwrap();
        let updated = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_ne!(added, updated, "updating an MCP server must change the fingerprint");

        conn.execute("DELETE FROM mcp_servers WHERE id = 'm1'", [])
            .unwrap();
        let removed = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_eq!(before, removed, "deleting back to the original state restores the fingerprint");
    }

    #[test]
    fn fingerprint_tracks_agent_rules_changes() {
        // 回归：全局约束表不参与指纹时，改动约束不会被判定为数据变更，
        // 跨设备同步就永远不会把这套约束发布出去。
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let key = [9_u8; 32];
        let before = compute_logical_fingerprint(&conn, &key).unwrap();

        conn.execute(
            "INSERT INTO agent_rules (id, body, targets_json, updated_at)
             VALUES (1, '# 约束', '[\"claude_code\"]', 1)",
            [],
        )
        .unwrap();
        let added = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_ne!(before, added, "saving agent rules must change the fingerprint");

        conn.execute(
            "UPDATE agent_rules SET body = '# 约束 v2', updated_at = 2 WHERE id = 1",
            [],
        )
        .unwrap();
        let updated = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_ne!(added, updated, "updating agent rules must change the fingerprint");

        conn.execute("DELETE FROM agent_rules WHERE id = 1", [])
            .unwrap();
        let removed = compute_logical_fingerprint(&conn, &key).unwrap();
        assert_eq!(before, removed, "deleting back to the original state restores the fingerprint");
    }

    #[test]
    fn only_business_tables_mark_dirty() {
        assert!(is_noisy_sync_table("sync_meta"));
        assert!(is_noisy_sync_table("webdav_sync_state"));
        assert!(!is_noisy_sync_table("sites"));
        assert!(!is_noisy_sync_table("settings"));
    }

    #[test]
    fn db_writes_mark_dirty_except_sync_tables() {
        SYNC_DIRTY.store(false, Ordering::Relaxed);
        SYNC_POLL_REQUESTED.store(false, Ordering::Relaxed);

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        install_db_hook(&conn);

        // 引擎记账表写入：不标记脏。
        repo::sync_meta::set_meta(&conn, "probe", "1").unwrap();
        assert!(!SYNC_DIRTY.load(Ordering::Relaxed));

        // 业务数据写入：标记脏。
        conn.execute(
            "INSERT OR REPLACE INTO settings (id, json) VALUES (1, '{}')",
            [],
        )
        .unwrap();
        assert!(SYNC_DIRTY.swap(false, Ordering::Relaxed));
        assert!(SYNC_POLL_REQUESTED.swap(false, Ordering::Relaxed) == false);
    }
}
