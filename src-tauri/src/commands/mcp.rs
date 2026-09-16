use crate::{
    adapters::mcp as mcp_adapters,
    domain::{AppSettings, McpServer, McpServerInput, McpServerSummary, TargetKind},
    error::AppResult,
    paths::backups_dir,
    repo,
    state::AppState,
};
use serde::{Deserialize, Serialize};
use tauri::State;

#[tauri::command]
pub fn list_mcp_servers(state: State<'_, AppState>) -> AppResult<Vec<McpServerSummary>> {
    state.db.with_conn(|conn| repo::mcp::list(conn, &state.crypto))
}

/// 首次打开面板时给出「热门」候选：用一批常见类目词并发查询官方仓库再合并去重。
///
/// 官方仓库没有热度排序，只拉「最近更新」时按本地过滤后往往只剩几条，首屏太空。
/// 这里换成几个常见类目词并发查，条目仍完全来自仓库实时数据（不是写死的推荐清单）。
#[tauri::command]
pub async fn discover_mcp_registry(
    state: State<'_, AppState>,
    local_only: Option<bool>,
    min_results: Option<u32>,
) -> AppResult<crate::mcp_registry::RegistrySearchResult> {
    use crate::mcp_registry as registry;
    use std::time::Duration;

    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let client = crate::http_client::build_client(&settings, Duration::from_secs(20))?;
    let only_local = local_only.unwrap_or(false);
    let target = min_results.unwrap_or(20) as usize;

    let requests = registry::DISCOVERY_TERMS.iter().map(|term| {
        let client = client.clone();
        let url = registry::search_url(term, None, Some(50));
        async move {
            let response = client.get(&url).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let bytes = response.bytes().await.ok()?;
            registry::parse_search_response(&bytes).ok()
        }
    });

    // 并发拉取：串行会让首屏等待明显变长。个别类目失败不影响整体（ok() 折叠成 None）。
    let pages = futures_util::future::join_all(requests).await;
    let batches: Vec<Vec<registry::RegistryCandidate>> =
        pages.into_iter().flatten().map(|page| page.candidates).collect();
    if batches.is_empty() {
        return Err(crate::error::AppError::new(
            "mcp_registry_unreachable",
            "无法连接官方 MCP 仓库",
        ));
    }

    let merged = registry::merge_candidates(batches);
    let filtered = registry::filter_candidates(merged, only_local);
    let candidates = filtered.into_iter().take(target.max(1)).collect();
    Ok(crate::mcp_registry::RegistrySearchResult {
        candidates,
        next_cursor: None,
    })
}

/// 搜索官方 MCP Registry。
///
/// 只访问固定的官方域名（见 `mcp_registry::REGISTRY_BASE`），不接受调用方传入地址。
/// `local_only` 是界面「只看本地运行」开关：勾着时只返回 npx/uvx 这类跑在用户自己机器上的
/// 条目，避免把请求交给来源不明的第三方服务器。
/// `query` 允许为空——那表示「浏览最近更新」，用于页面首次打开时给出内容而不是空白。
/// `min_results` 是「至少凑够多少条」：仓库里远程条目占多数，只看本地时单页往往只剩两三条，
/// 这里会自动往后翻页补齐，避免用户一点搜索只看到寥寥几项。
#[tauri::command]
pub async fn search_mcp_registry(
    state: State<'_, AppState>,
    query: String,
    cursor: Option<String>,
    local_only: Option<bool>,
    limit: Option<u32>,
    min_results: Option<u32>,
) -> AppResult<crate::mcp_registry::RegistrySearchResult> {
    use crate::mcp_registry as registry;
    use std::time::Duration;

    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let client = crate::http_client::build_client(&settings, Duration::from_secs(20))?;
    let only_local = local_only.unwrap_or(false);
    // 补齐目标只在「按本地过滤」时才有意义：不过滤时一页就是完整结果集。
    let target = if only_local {
        min_results.unwrap_or(0).min(registry::MAX_FILL_RESULTS) as usize
    } else {
        0
    };

    let mut collected: Vec<registry::RegistryCandidate> = Vec::new();
    let mut next = cursor.clone();
    let mut pages = 0;

    loop {
        let url = registry::search_url(&query, next.as_deref(), limit);
        let response = client.get(&url).send().await.map_err(|error| {
            crate::error::AppError::new(
                "mcp_registry_unreachable",
                format!("无法连接官方 MCP 仓库: {error}"),
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(crate::error::AppError::new(
                "mcp_registry_http",
                format!("官方 MCP 仓库返回 {status}"),
            ));
        }
        let bytes = response.bytes().await.map_err(|error| {
            crate::error::AppError::new(
                "mcp_registry_unreachable",
                format!("读取官方 MCP 仓库响应失败: {error}"),
            )
        })?;

        let page = registry::parse_search_response(&bytes)?;
        next = page.next_cursor.clone();
        collected.extend(page.candidates);

        pages += 1;
        let enough = collected.len() >= target;
        if enough || next.is_none() || pages >= registry::MAX_FILL_PAGES {
            break;
        }
    }

    let filtered = registry::filter_candidates(collected, only_local);
    Ok(crate::mcp_registry::RegistrySearchResult {
        candidates: filtered,
        next_cursor: next,
    })
}

#[tauri::command]
pub fn get_mcp_server(state: State<'_, AppState>, id: String) -> AppResult<McpServer> {
    state.db
        .with_conn(|conn| repo::mcp::get(conn, &id, &state.crypto))
}

/// `(async)` 是必须的：普通 `#[tauri::command]` 的同步函数在 IPC 处理线程上执行，
/// 本命令会连带把托管条目写回各客户端配置文件（写盘 + 原子替换），
/// 放在 IPC 线程上会卡住窗口消息泵。`try_lock_*` 目标级锁语义不变。
#[tauri::command(async)]
pub fn save_mcp_server(
    state: State<'_, AppState>,
    input: McpServerInput,
) -> AppResult<McpSaveResult> {
    let server = state
        .db
        .with_conn(|conn| repo::mcp::save(conn, &state.crypto, input))?;
    // 之前应用过的目标要重新同步一遍：改名或取消目标后，旧客户端里的托管条目才能清掉。
    // 同步失败不能反过来判定保存失败（数据已落库），但必须把目标级结果回给界面。
    let sweep = apply_to_targets(&state, &[])?;
    Ok(McpSaveResult { server, sweep })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSaveResult {
    pub server: McpServer,
    pub sweep: McpApplyResult,
}

/// 见 `save_mcp_server`：删除后要清理各客户端里的托管条目。
#[tauri::command(async)]
pub fn delete_mcp_server(state: State<'_, AppState>, id: String) -> AppResult<McpApplyResult> {
    state.db.with_conn(|conn| repo::mcp::delete(conn, &id))?;
    apply_to_targets(&state, &[])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApplyTargetResult {
    pub target: TargetKind,
    pub ok: bool,
    pub backup_paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApplyResult {
    pub results: Vec<McpApplyTargetResult>,
    pub applied_at: i64,
}

/// 见 `save_mcp_server`：一次要把多个客户端配置文件重写并落盘。
#[tauri::command(async)]
pub fn apply_mcp_servers(
    state: State<'_, AppState>,
    targets: Vec<TargetKind>,
) -> AppResult<McpApplyResult> {
    apply_to_targets(&state, &targets)
}

/// 需要写盘的目标 = 本次请求 ∪ 上次应用过的目标。
///
/// 之所以要并上「上次应用过的目标」：用户把某个 MCP 从目标里移除、改名或禁用后，
/// 那个客户端里已经写入的托管条目必须被清掉，否则会留下孤儿配置。
/// 删除 MCP 时请求列表为空，此时完全依赖这个并集来清理。
fn merged_targets(requested: &[TargetKind], previously_applied: &[TargetKind]) -> Vec<TargetKind> {
    let mut union = requested.to_vec();
    for target in previously_applied {
        if !union.contains(target) {
            union.push(*target);
        }
    }
    union
}

/// 应用后要记下的目标集合 = 「清理后仍然持有托管条目的目标」∪「本次写失败的目标」。
///
/// - 仍然持有条目：该目标上还有启用的 MCP 指向它（`desired`），且本次写成功。
///   一个目标被清理干净后就不该继续留在记录里，否则以后每次保存都会重写它那份
///   客户端配置（`~/.claude.json` 这类文件是客户端自己在频繁改写的，不该被无谓触碰）。
/// - 写失败的目标：保留下来，下次仍然参与清理重试。
fn targets_to_record(
    results: &[McpApplyTargetResult],
    desired: &[TargetKind],
) -> Vec<TargetKind> {
    let mut recorded: Vec<TargetKind> = Vec::new();
    for result in results {
        let keep = !result.ok || desired.contains(&result.target);
        if keep && !recorded.contains(&result.target) {
            recorded.push(result.target);
        }
    }
    recorded
}

/// 当前有启用的 MCP 指向的目标集合：只有这些目标在清理后还需要保留托管条目。
fn targets_with_enabled_servers(servers: &[McpServer]) -> Vec<TargetKind> {
    let mut desired = Vec::new();
    for server in servers.iter().filter(|server| server.enabled) {
        for target in &server.targets {
            if !desired.contains(target) {
                desired.push(*target);
            }
        }
    }
    desired
}

/// 把数据库里的 MCP 现状写到目标客户端。
///
/// 实际写入的目标 = 本次请求的目标 ∪ 上次应用过的目标：这样用户把某个 MCP 从目标里移除后，
/// 那个客户端里的托管条目会被清掉，而不是留成孤儿。空目标列表用于「删除后清理」。
fn apply_to_targets(state: &AppState, requested: &[TargetKind]) -> AppResult<McpApplyResult> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let servers = state.db.with_conn(|conn| repo::mcp::list_full(conn, &state.crypto))?;
    let previously_applied = state.db.with_conn(repo::mcp::applied_targets)?;

    let union = merged_targets(requested, &previously_applied);

    let backup_root = backups_dir()?.join("mcp");
    std::fs::create_dir_all(&backup_root)?;

    let mut results = Vec::new();
    for target in &union {
        let target_servers: Vec<McpServer> = servers
            .iter()
            .filter(|server| server.targets.contains(target))
            .cloned()
            .collect();

        let outcome = match target {
            TargetKind::ClaudeCode => mcp_adapters::apply_to_claude(
                &target_servers,
                settings.claude_home_override.as_deref(),
                &backup_root,
            ),
            TargetKind::Codex => mcp_adapters::apply_to_codex(
                &target_servers,
                settings.codex_home_override.as_deref(),
                &backup_root,
            ),
            TargetKind::Pi => mcp_adapters::apply_to_pi(
                &target_servers,
                settings.pi_agent_dir_override.as_deref(),
                &backup_root,
            ),
            TargetKind::Prime => mcp_adapters::apply_to_prime(
                &target_servers,
                settings.prime_agent_dir_override.as_deref(),
                &backup_root,
            ),
            TargetKind::ZCode => mcp_adapters::apply_to_zcode(
                &target_servers,
                settings.zcode_home_override.as_deref(),
                &backup_root,
            ),
        };

        match outcome {
            Ok(result) => results.push(McpApplyTargetResult {
                target: *target,
                ok: result.ok,
                backup_paths: result.backup_paths,
                message: result.message,
            }),
            Err(error) => results.push(McpApplyTargetResult {
                target: *target,
                ok: false,
                backup_paths: Vec::new(),
                message: error.to_string(),
            }),
        }
    }

    let desired = targets_with_enabled_servers(&servers);
    let merged = targets_to_record(&results, &desired);
    state
        .db
        .with_conn(|conn| repo::mcp::record_applied_targets(conn, &merged))?;

    Ok(McpApplyResult {
        results,
        applied_at: chrono::Utc::now().timestamp_millis(),
    })
}

/// 供设置面板展示目标配置文件的实际落点。
#[tauri::command]
pub fn mcp_target_paths(state: State<'_, AppState>) -> AppResult<Vec<(TargetKind, String)>> {
    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    Ok(vec![
        (
            TargetKind::ClaudeCode,
            crate::paths::claude_mcp_json_path(settings.claude_home_override.as_deref())?
                .display()
                .to_string(),
        ),
        (
            TargetKind::Codex,
            crate::paths::resolve_codex_home(settings.codex_home_override.as_deref())?
                .join("config.toml")
                .display()
                .to_string(),
        ),
        (
            TargetKind::Pi,
            crate::paths::resolve_pi_agent_dir(settings.pi_agent_dir_override.as_deref())?
                .join("mcp.json")
                .display()
                .to_string(),
        ),
        (
            TargetKind::Prime,
            crate::paths::resolve_prime_agent_dir(settings.prime_agent_dir_override.as_deref())?
                .join("settings.json")
                .display()
                .to_string(),
        ),
        (
            TargetKind::ZCode,
            crate::paths::zcode_mcp_path(settings.zcode_home_override.as_deref())?
                .display()
                .to_string(),
        ),
    ])
}

/// 扫描四个客户端里**用户已有**的 MCP 配置。
///
/// 只读：不修改任何客户端文件。返回值不含任何密钥值，只含键名——界面据此展示
/// 「需要填什么」，密钥要到纳管时才由后端直接读盘入库。
/// `(async)`：要读四个客户端的大配置文件（`~/.claude.json` 可能很大），
/// 放在 IPC 线程上会卡住窗口消息泵。
#[tauri::command(async)]
pub fn scan_existing_mcp(
    state: State<'_, AppState>,
) -> AppResult<crate::adapters::mcp_scan::ScanOutcome> {
    use crate::adapters::mcp_scan;

    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let mut outcome = mcp_scan::scan_all(&settings);

    // 与库里已有记录比对，标出哪些已纳管过，避免界面重复提供「纳管」。
    // 名称比较不区分大小写，与 save 的重名校验口径一致。
    let existing = state.db.with_conn(|conn| repo::mcp::list(conn, &state.crypto))?;
    let by_name: std::collections::HashMap<String, String> = existing
        .into_iter()
        .map(|server| (server.name.to_lowercase(), server.id))
        .collect();
    for entry in &mut outcome.entries {
        entry.imported_id = by_name.get(&entry.name.to_lowercase()).cloned();
    }

    Ok(outcome)
}

/// 纳管定位符：前端只说明「哪个客户端的哪个键」，不承担传递配置内容的职责。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportLocator {
    pub target: crate::adapters::mcp_scan::ScanTarget,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportFailure {
    pub target: crate::adapters::mcp_scan::ScanTarget,
    pub key: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportResult {
    pub imported: Vec<McpServerSummary>,
    pub failed: Vec<McpImportFailure>,
}

/// 纳管：把扫描到的条目导入数据库，env/headers 走既有加密存储。
///
/// 密钥值由后端按定位符直接读盘取得，**不经过前端**。导入本身不修改来源客户端文件——
/// 接管（删除等价的手工条目）发生在之后的「应用」时，且有指纹校验兜底。
/// `(async)`：逐条读盘并做加密入库，属于纯磁盘工作。
#[tauri::command(async)]
pub fn import_scanned_mcp(
    state: State<'_, AppState>,
    locators: Vec<McpImportLocator>,
) -> AppResult<McpImportResult> {
    use crate::adapters::mcp_scan;

    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let mut imported = Vec::new();
    let mut failed = Vec::new();

    for locator in locators {
        let saved = mcp_scan::load_entry_for_import(locator.target, &locator.key, &settings)
            .and_then(|input| {
                state
                    .db
                    .with_conn(|conn| repo::mcp::save(conn, &state.crypto, input))
            });

        match saved {
            // 只回元数据：save 的返回值含 env/headers 明文，不该回传前端。
            Ok(server) => imported.push(McpServerSummary {
                id: server.id,
                name: server.name,
                kind: server.kind,
                enabled: server.enabled,
                targets: server.targets,
                created_at: server.created_at,
                updated_at: server.updated_at,
                current_version: server.current_version,
                latest_version: server.latest_version,
                last_update_check_at: server.last_update_check_at,
            }),
            Err(error) => failed.push(McpImportFailure {
                target: locator.target,
                key: locator.key,
                message: error.to_string(),
            }),
        }
    }

    Ok(McpImportResult { imported, failed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(target: TargetKind, ok: bool) -> McpApplyTargetResult {
        McpApplyTargetResult {
            target,
            ok,
            backup_paths: Vec::new(),
            message: String::new(),
        }
    }

    #[test]
    fn cleanup_covers_targets_that_were_applied_before() {
        // 用户把 MCP 从 Claude 改绑到 Codex 后，请求里只有 Codex，
        // 但 Claude 里已经写过的托管条目必须一起清掉。
        let union = merged_targets(&[TargetKind::Codex], &[TargetKind::ClaudeCode]);
        assert_eq!(union.len(), 2);
        assert!(union.contains(&TargetKind::Codex));
        assert!(union.contains(&TargetKind::ClaudeCode));
    }

    #[test]
    fn delete_with_no_requested_targets_still_cleans_every_previous_target() {
        let union = merged_targets(&[], &[TargetKind::Pi, TargetKind::Prime]);
        assert_eq!(union, vec![TargetKind::Pi, TargetKind::Prime]);
    }

    #[test]
    fn merged_targets_does_not_duplicate_requested_targets() {
        let union = merged_targets(
            &[TargetKind::ClaudeCode, TargetKind::Prime],
            &[TargetKind::Prime, TargetKind::Codex],
        );
        assert_eq!(union.len(), 3);
        assert!(union.contains(&TargetKind::ClaudeCode));
        assert!(union.contains(&TargetKind::Prime));
        assert!(union.contains(&TargetKind::Codex));
    }

    #[test]
    fn failed_targets_stay_eligible_for_the_next_cleanup() {
        // Claude 写成功且仍有启用的服务指向它，Codex 写失败：两个都保留在记录里，
        // 失败的那个下次仍会被重试，而不是被静默遗忘。
        let desired = vec![TargetKind::ClaudeCode, TargetKind::Codex];
        let recorded = targets_to_record(
            &[
                result(TargetKind::ClaudeCode, true),
                result(TargetKind::Codex, false),
            ],
            &desired,
        );
        assert_eq!(recorded.len(), 2);
        assert!(recorded.contains(&TargetKind::Codex));
    }

    #[test]
    fn successful_apply_records_the_new_target_set() {
        let recorded = targets_to_record(
            &[result(TargetKind::Codex, true)],
            &[TargetKind::Codex],
        );
        assert_eq!(recorded, vec![TargetKind::Codex]);
    }

    #[test]
    fn delete_cleanup_reports_every_swept_target() {
        // 删除后没有启用的服务指向任何目标，清理成功后记录应为空；
        // 但结果里仍要逐目标出现，界面才能说明哪些客户端被清理过。
        let previous = [TargetKind::Pi, TargetKind::Prime];
        let union = merged_targets(&[], &previous);
        let results: Vec<McpApplyTargetResult> = union
            .iter()
            .map(|target| result(*target, true))
            .collect();
        let recorded = targets_to_record(&results, &[]);
        assert!(recorded.is_empty(), "cleaned targets must not stay on the record");
        assert_eq!(results.len(), 2, "each swept target is reported");
    }

    #[test]
    fn a_target_that_stops_being_requested_is_dropped_from_the_record() {
        // Claude 曾经应用过，本次只写 Codex：Claude 仅用于清理，
        // 清理成功后不应继续留在记录里，否则每次保存都会重写它那份配置。
        let union = merged_targets(&[TargetKind::Codex], &[TargetKind::ClaudeCode]);
        let results = vec![
            result(TargetKind::ClaudeCode, true),
            result(TargetKind::Codex, true),
        ];
        let recorded = targets_to_record(&results, &[TargetKind::Codex]);
        assert_eq!(recorded, vec![TargetKind::Codex]);
    }

    #[test]
    fn enabled_servers_decide_which_targets_keep_managed_entries() {
        // 只有启用且有目标指向的服务才让目标留在记录里；
        // 禁用服务或空目标列表都不应阻止清理。
        let enabled = McpServer {
            id: "1".into(),
            name: "a".into(),
            kind: crate::domain::McpKind::Stdio,
            enabled: true,
            targets: vec![TargetKind::Pi],
            config: serde_json::json!({}),
            env: serde_json::json!({}),
            headers: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
            current_version: None,
            latest_version: None,
            last_update_check_at: None,
        };
        let mut disabled = enabled.clone();
        disabled.enabled = false;
        disabled.targets = vec![TargetKind::Prime];
        let mut no_targets = enabled.clone();
        no_targets.targets = vec![];

        assert_eq!(
            targets_with_enabled_servers(&[enabled, disabled, no_targets]),
            vec![TargetKind::Pi]
        );
        assert!(targets_with_enabled_servers(&[]).is_empty());
    }
}
