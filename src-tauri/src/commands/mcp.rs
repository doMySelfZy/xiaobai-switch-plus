use crate::{
    adapters::mcp as mcp_adapters,
    adapters::mcp_identity,
    crypto::Crypto,
    domain::{AppSettings, McpServer, McpServerInput, McpServerSummary, TargetKind},
    error::{AppError, AppResult},
    paths::backups_dir,
    repo,
    state::AppState,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
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

    // 先收集所有 term 为 String，避免生命周期问题
    let terms: Vec<String> = registry::DISCOVERY_TERMS.iter().map(|s| s.to_string()).collect();
    
    let requests = terms.into_iter().map(|term| {
        let client = client.clone();
        async move {
            let url = registry::search_url(&term, None, Some(50));
            let response = client.get(&url).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let bytes = response.bytes().await.ok()?;
            registry::parse_search_response(&bytes).ok()
        }
    });

    // 并发拉取：限制最多 3 个并发请求，避免过载。个别类目失败不影响整体（ok() 折叠成 None）。
    use futures_util::stream::{self, StreamExt};
    let pages: Vec<_> = stream::iter(requests)
        .buffer_unordered(3)
        .collect()
        .await;
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
    let server = state.db.with_conn(|conn| {
        // 身份去重守卫（命令层）：粗身份撞上别的行就明确拒绝，绝不静默写入第二条。
        // repo::mcp::save 只比名字的重名校验保持原样，这里是叠加判定。
        let servers = repo::mcp::list_full(conn, &state.crypto)?;
        guard_identity_collision(&servers, &input)?;
        repo::mcp::save(conn, &state.crypto, input)
    })?;
    // 之前应用过的目标要重新同步一遍：改名或取消目标后，旧客户端里的托管条目才能清掉。
    // 同步失败不能反过来判定保存失败（数据已落库），但必须把目标级结果回给界面。
    let sweep = apply_to_targets(&state, &[])?;
    Ok(McpSaveResult { server, sweep })
}

/// 找出与 `input` 共享粗身份的**另一行**（按 `id` 编辑自身时排除自己）。
fn identity_conflict<'a>(
    servers: &'a [McpServer],
    input: &McpServerInput,
) -> Option<&'a McpServer> {
    let identity = mcp_identity::coarse_identity(input.kind, &input.config);
    servers.iter().find(|server| {
        Some(server.id.as_str()) != input.id.as_deref()
            && mcp_identity::coarse_identity(server.kind, &server.config) == identity
    })
}

/// 手工新增/编辑保存的去重守卫：命中他行粗身份 → `validation_failed` 并点名冲突条目。
/// 按 `id` 编辑自身（哪怕库里还留着存量重复）必须放行。
fn guard_identity_collision(servers: &[McpServer], input: &McpServerInput) -> AppResult<()> {
    if let Some(conflict) = identity_conflict(servers, input) {
        return Err(AppError::new(
            "validation_failed",
            format!(
                "这份配置与已有 MCP 条目「{}」指向同一个服务器。\
                 为避免产生重复条目，本次没有写入：请直接编辑那条已有条目，或先处理重复项。",
                conflict.name
            ),
        ));
    }
    Ok(())
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

/// 单个目标的漂移状态：库内现状与客户端已应用内容是否一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTargetDrift {
    pub target: TargetKind,
    /// 需写入 / 内容有变化的托管条目数。
    pub to_write: usize,
    /// 应用时会被清理的失效托管条目（孤儿）数。
    pub to_clean: usize,
    /// 存在同名未托管条目、内容不一致会导致应用报错跳过的服务名。
    pub conflicts: Vec<String>,
    /// `to_write>0 || to_clean>0 || 有冲突`。
    pub drift: bool,
    /// 该目标配置文件形状非法等无法比对时的错误信息（此时其余计数为 0）。
    pub error: Option<String>,
}

/// 逐目标（Claude / Codex / Pi，排除 Prime）计算漂移，用于「需重新应用」提示。
/// 只读：不写盘、不备份、不加锁。某个目标读失败只影响该目标的 `error` 字段。
#[tauri::command(async)]
pub fn mcp_drift_status(state: State<'_, AppState>) -> AppResult<Vec<McpTargetDrift>> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let servers = state.db.with_conn(|conn| repo::mcp::list_full(conn, &state.crypto))?;

    let plans: [(TargetKind, AppResult<mcp_adapters::McpMergePlan>); 3] = [
        (
            TargetKind::ClaudeCode,
            mcp_adapters::plan_claude(&servers, settings.claude_home_override.as_deref()),
        ),
        (
            TargetKind::Codex,
            mcp_adapters::plan_codex(&servers, settings.codex_home_override.as_deref()),
        ),
        (
            TargetKind::Pi,
            mcp_adapters::plan_pi(&servers, settings.pi_agent_dir_override.as_deref()),
        ),
    ];

    Ok(plans
        .into_iter()
        .map(|(target, plan)| match plan {
            Ok(plan) => McpTargetDrift {
                target,
                to_write: plan.to_write,
                to_clean: plan.to_clean,
                drift: plan.drift(),
                conflicts: plan.conflicts,
                error: None,
            },
            Err(error) => McpTargetDrift {
                target,
                to_write: 0,
                to_clean: 0,
                conflicts: Vec::new(),
                drift: false,
                error: Some(error.to_string()),
            },
        })
        .collect())
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

/// 找出「启用且共享粗身份」的重复组（最后防线判据）：每组返回成员名字，
/// 按首次出现顺序稳定排列；不足两条不算组。禁用的条目不参与——它们本来就不会被写盘。
fn identity_conflicts(servers: &[McpServer]) -> Vec<Vec<String>> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for server in servers.iter().filter(|server| server.enabled) {
        let identity = mcp_identity::coarse_identity(server.kind, &server.config);
        match groups.iter_mut().find(|(known, _)| *known == identity) {
            Some((_, names)) => names.push(server.name.clone()),
            None => groups.push((identity, vec![server.name.clone()])),
        }
    }
    groups
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(_, names)| names)
        .collect()
}

/// 把数据库现状写到目标客户端：纯磁盘工作，不碰数据库——便于用 tempdir 端到端测试。
///
/// 最后防线：某个目标上出现两条「启用且粗身份相同」的条目时，该目标**整体跳过**并报失败，
/// 两条都不写入，也不做该目标的清理（宁可留陈旧的托管条目，也不猜用户想保留哪一条）；
/// 其余目标照常应用。失败目标会被 `targets_to_record` 留在记录里，下一次应用自动重试。
fn apply_servers_to_targets(
    settings: &AppSettings,
    servers: &[McpServer],
    previously_applied: &[TargetKind],
    requested: &[TargetKind],
    backup_root: &Path,
) -> Vec<McpApplyTargetResult> {
    let union = merged_targets(requested, previously_applied);

    let mut results = Vec::new();
    for target in &union {
        let target_servers: Vec<McpServer> = servers
            .iter()
            .filter(|server| server.targets.contains(target))
            .cloned()
            .collect();

        let conflicts = identity_conflicts(&target_servers);
        if !conflicts.is_empty() {
            let groups: Vec<String> = conflicts
                .iter()
                .map(|names| format!("「{}」", names.join(" / ")))
                .collect();
            results.push(McpApplyTargetResult {
                target: *target,
                ok: false,
                backup_paths: Vec::new(),
                message: format!(
                    "检测到指向同一个服务器的重复 MCP 条目（{}），本次没有写入该目标：\
                     两条都写进客户端会让同一个 MCP 被加载两遍。请先在 MCP 面板合并或停用其中一条。",
                    groups.join("；")
                ),
            });
            continue;
        }

        let outcome = match target {
            TargetKind::ClaudeCode => mcp_adapters::apply_to_claude(
                &target_servers,
                settings.claude_home_override.as_deref(),
                backup_root,
            ),
            TargetKind::Codex => mcp_adapters::apply_to_codex(
                &target_servers,
                settings.codex_home_override.as_deref(),
                backup_root,
            ),
            TargetKind::Pi => mcp_adapters::apply_to_pi(
                &target_servers,
                settings.pi_agent_dir_override.as_deref(),
                backup_root,
            ),
            TargetKind::Prime => mcp_adapters::apply_to_prime(
                &target_servers,
                settings.prime_agent_dir_override.as_deref(),
                backup_root,
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
    results
}

/// 把数据库里的 MCP 现状写到目标客户端。
///
/// 实际写入的目标 = 本次请求的目标 ∪ 上次应用过的目标：这样用户把某个 MCP 从目标里移除后，
/// 那个客户端里的托管条目会被清掉，而不是留成孤儿。空目标列表用于「删除后清理」。
fn apply_to_targets(state: &AppState, requested: &[TargetKind]) -> AppResult<McpApplyResult> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let servers = state.db.with_conn(|conn| repo::mcp::list_full(conn, &state.crypto))?;
    let previously_applied = state.db.with_conn(repo::mcp::applied_targets)?;

    let backup_root = backups_dir()?.join("mcp");
    std::fs::create_dir_all(&backup_root)?;

    let results =
        apply_servers_to_targets(&settings, &servers, &previously_applied, requested, &backup_root);

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
    // 先按粗身份匹配（名字是每个客户端各自的局部属性，同一服务器可以叫两个名字），
    // 身份未命中再按名字兜底（大小写不敏感，与 save 的重名校验口径一致）——
    // 保住用户手工改过名、args 又有微调的既有条目不被显示成「未纳管」。
    let existing = state.db.with_conn(|conn| repo::mcp::list_full(conn, &state.crypto))?;
    mark_imported_entries(&mut outcome.entries, &existing);

    Ok(outcome)
}

/// ScanTarget 与 TargetKind 取值一一对应（前端同样复用展示名）。
fn scan_target_kind(target: crate::adapters::mcp_scan::ScanTarget) -> TargetKind {
    match target {
        crate::adapters::mcp_scan::ScanTarget::ClaudeCode => TargetKind::ClaudeCode,
        crate::adapters::mcp_scan::ScanTarget::Codex => TargetKind::Codex,
        crate::adapters::mcp_scan::ScanTarget::Pi => TargetKind::Pi,
        crate::adapters::mcp_scan::ScanTarget::Prime => TargetKind::Prime,
    }
}

/// 按粗身份聚合扫描条目所在的目标：纳管时据此把新行关联到所有已存在同款配置的客户端。
///
/// 只看 `kind + config`（`coarse_identity` 的口径），env/headers 的值不参与——同一份启动配置
/// 在不同客户端里各带各的密钥是常态，不该因此被判成不同服务。目标顺序按四端固定序去重，稳定可测。
fn scan_target_union(
    entries: &[crate::adapters::mcp_scan::ScannedMcp],
) -> std::collections::HashMap<String, Vec<TargetKind>> {
    let mut map: std::collections::HashMap<String, Vec<TargetKind>> =
        std::collections::HashMap::new();
    for entry in entries {
        let identity = mcp_identity::coarse_identity(entry.kind, &entry.config);
        let target = scan_target_kind(entry.target);
        let targets = map.entry(identity).or_default();
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    map
}

/// 给扫描结果标注库内同一条目的 id（见 `scan_existing_mcp` 的两级匹配规则）。
///
/// 另附两份 R3 预告（只读，不影响纳管/应用判定）：
/// - `name_conflict`：同名但粗身份不同——导了撞重名校验，应用了撞接管校验，界面直接跳过；
/// - `adoptable`：身份命中的记录启用且覆盖本目标——下次应用删未托管写托管，界面预告接管。
fn mark_imported_entries(
    entries: &mut [crate::adapters::mcp_scan::ScannedMcp],
    servers: &[McpServer],
) {
    for entry in entries.iter_mut() {
        let identity = mcp_identity::coarse_identity(entry.kind, &entry.config);
        let name = entry.name.to_lowercase();
        let by_identity = servers
            .iter()
            .find(|server| mcp_identity::coarse_identity(server.kind, &server.config) == identity);
        let by_name = servers
            .iter()
            .find(|server| server.name.to_lowercase() == name);
        entry.imported_id = by_identity
            .or(by_name)
            .map(|server| server.id.clone());
        entry.name_conflict = by_name.is_some() && by_identity.is_none();
        entry.adoptable = !entry.managed
            && !entry.name_conflict
            && by_identity.is_some_and(|server| {
                server.enabled && server.targets.contains(&scan_target_kind(entry.target))
            });
    }
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

/// 纳管时命中「库里已有同一粗身份的行」的结果项：只回定位符与已有行的 id/名字，
/// **不含 config/env/headers**——已纳管就是已纳管，不替用户改已有行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlreadyImportedMcp {
    pub target: crate::adapters::mcp_scan::ScanTarget,
    pub key: String,
    pub existing_id: String,
    pub existing_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportResult {
    pub imported: Vec<McpServerSummary>,
    pub failed: Vec<McpImportFailure>,
    /// 命中已有行粗身份、被跳过而未建行的条目（既有行为：`failed` 仍只表示真错误）。
    pub already_imported: Vec<AlreadyImportedMcp>,
    /// 纳管即接管：入库后立即对涉及的客户端写盘接管的结果（删手工条目、写 `xiaobai_`）。
    /// 本次没有成功入库/命中任何记录（无客户端需要接管）时为 `None`。
    pub apply: Option<McpApplyResult>,
}

/// 单条纳管的两种归宿。`AlreadyImported` 不建任何行、不碰已有行。
#[derive(Debug)]
enum ImportOutcome {
    Imported(McpServerSummary),
    AlreadyImported(AlreadyImportedMcp),
}

fn summarize_server(server: &McpServer) -> McpServerSummary {
    McpServerSummary {
        id: server.id.clone(),
        name: server.name.clone(),
        kind: server.kind,
        enabled: server.enabled,
        targets: server.targets.clone(),
        created_at: server.created_at,
        updated_at: server.updated_at,
        current_version: server.current_version.clone(),
        latest_version: server.latest_version.clone(),
        last_update_check_at: server.last_update_check_at,
        absolute_command: crate::adapters::mcp_identity::command_is_absolute(
            server.kind,
            &server.config,
        ),
    }
}

/// 纳管一条已定位的条目：粗身份命中已有行 → 只回指那条行，**建行与已有行都不动**
/// （不改名、不改 targets、不刷新 updated_at）——用户要的是别建重复行，不是替他做决定。
/// 未命中才走 `repo::mcp::save` 建行。
fn import_entry(
    conn: &Connection,
    crypto: &Crypto,
    target: crate::adapters::mcp_scan::ScanTarget,
    key: &str,
    input: McpServerInput,
) -> AppResult<ImportOutcome> {
    let identity = mcp_identity::coarse_identity(input.kind, &input.config);
    let existing = repo::mcp::list_full(conn, crypto)?;
    if let Some(hit) = existing
        .iter()
        .find(|server| mcp_identity::coarse_identity(server.kind, &server.config) == identity)
    {
        return Ok(ImportOutcome::AlreadyImported(AlreadyImportedMcp {
            target,
            key: key.to_string(),
            existing_id: hit.id.clone(),
            existing_name: hit.name.clone(),
        }));
    }
    let server = repo::mcp::save(conn, crypto, input)?;
    Ok(ImportOutcome::Imported(summarize_server(&server)))
}

/// 纳管：把扫描到的条目导入数据库，env/headers 走既有加密存储，随后**立即接管**。
///
/// 密钥值由后端按定位符直接读盘取得，**不经过前端**。纳管即接管：入库后立刻对涉及的
/// 客户端复用「应用」路径写盘——删掉等价的手工条目、写入 `xiaobai_<name>`，备份+原子替换+
/// 锁+指纹校验一并生效。同名条目在扫描后被用户改成不等价时，适配器报错跳过该目标、原样
/// 保留文件（结果记在返回值的 `apply` 里）。命中已有行粗身份的条目进 `already_imported`
/// （不建行、不动已有行），真错误才进 `failed`。
/// `(async)`：逐条读盘、加密入库再写客户端文件，属于纯磁盘工作。
#[tauri::command(async)]
pub fn import_scanned_mcp(
    state: State<'_, AppState>,
    locators: Vec<McpImportLocator>,
) -> AppResult<McpImportResult> {
    import_scanned_mcp_impl(&state, locators)
}

/// 纳管主体，取 `&AppState` 便于端到端测试（命令层只做 `State` → `&AppState` 转接）。
fn import_scanned_mcp_impl(
    state: &AppState,
    locators: Vec<McpImportLocator>,
) -> AppResult<McpImportResult> {
    use crate::adapters::mcp_scan;

    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let mut imported = Vec::new();
    let mut failed = Vec::new();
    let mut already_imported = Vec::new();

    // 纳管即关联到**当前存在同一份配置**的所有客户端：扫一遍四端，按粗身份聚合目标并集。
    let identity_targets = scan_target_union(&mcp_scan::scan_all(&settings).entries);

    for locator in locators {
        let outcome = mcp_scan::load_entry_for_import(locator.target, &locator.key, &settings)
            .and_then(|mut input| {
                let identity = mcp_identity::coarse_identity(input.kind, &input.config);
                if let Some(targets) = identity_targets.get(&identity) {
                    input.targets = targets.clone();
                }
                state.db.with_conn(|conn| {
                    import_entry(conn, &state.crypto, locator.target, &locator.key, input)
                })
            });

        match outcome {
            // 只回元数据：save 的返回值含 env/headers 明文，不该回传前端。
            Ok(ImportOutcome::Imported(summary)) => imported.push(summary),
            Ok(ImportOutcome::AlreadyImported(existing)) => already_imported.push(existing),
            Err(error) => failed.push(McpImportFailure {
                target: locator.target,
                key: locator.key,
                message: error.to_string(),
            }),
        }
    }

    // 纳管即接管：对本次入库或命中记录所覆盖的客户端并集立即写盘。
    // 复用应用路径（含接管删手工条目、备份、原子替换、锁、指纹校验），并把这些目标记进
    // applied_targets——删除该记录时才能把所有关联客户端里的 `xiaobai_` 一起清干净。
    let touched = touched_targets(state, &imported, &already_imported)?;
    let apply = if touched.is_empty() {
        None
    } else {
        Some(apply_to_targets(state, &touched)?)
    };

    Ok(McpImportResult {
        imported,
        failed,
        already_imported,
        apply,
    })
}

/// 本次纳管涉及的客户端并集：入库记录 + 命中的已有记录，各自 targets 去重合并。
///
/// 首次纳管多客户端 MCP 走 `imported`，其 targets 已是并集；重复纳管同款走 `already_imported`，
/// 用命中记录当前的 targets 兜底补接管尚未写盘的客户端。
fn touched_targets(
    state: &AppState,
    imported: &[McpServerSummary],
    already_imported: &[AlreadyImportedMcp],
) -> AppResult<Vec<TargetKind>> {
    // 命中已有行时要用它当前的 targets 补接管，故只在有 already_imported 时才回读库。
    let servers = if already_imported.is_empty() {
        Vec::new()
    } else {
        state.db.with_conn(|conn| repo::mcp::list_full(conn, &state.crypto))?
    };
    Ok(merge_touched_targets(imported, already_imported, &servers))
}

/// 纯逻辑：合并本次纳管涉及的客户端并集，去重。抽出来便于脱离数据库单测。
///
/// `imported` 的 targets 已是并集（新建行时按粗身份并集设定）；`already_imported` 用命中记录
/// 当前的 targets 兜底——补接管那些同款却尚未写盘的客户端。
fn merge_touched_targets(
    imported: &[McpServerSummary],
    already_imported: &[AlreadyImportedMcp],
    servers: &[McpServer],
) -> Vec<TargetKind> {
    let mut touched: Vec<TargetKind> = Vec::new();
    let mut push = |targets: &[TargetKind], touched: &mut Vec<TargetKind>| {
        for target in targets {
            if !touched.contains(target) {
                touched.push(*target);
            }
        }
    };

    for summary in imported {
        push(&summary.targets, &mut touched);
    }
    for existing in already_imported {
        if let Some(server) = servers.iter().find(|server| server.id == existing.existing_id) {
            push(&server.targets, &mut touched);
        }
    }
    touched
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

    // ------------------------------------------------------------------
    // 身份去重（M1）：纳管 / 保存 / 应用三处判定收紧的行为测试。
    // ------------------------------------------------------------------

    use crate::domain::McpKind;
    use rusqlite::params;
    use serde_json::{json, Value};
    use std::fs;

    fn db_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        conn
    }

    fn test_crypto() -> Crypto {
        Crypto::from_key([9_u8; 32])
    }

    fn input_fixture(name: &str, config: Value, targets: Vec<TargetKind>) -> McpServerInput {
        McpServerInput {
            id: None,
            name: name.to_string(),
            kind: McpKind::Stdio,
            enabled: true,
            targets,
            config,
            env: json!({}),
            headers: json!({}),
        }
    }

    fn server_fixture(
        name: &str,
        config: Value,
        targets: Vec<TargetKind>,
        enabled: bool,
    ) -> McpServer {
        McpServer {
            id: format!("id-{name}"),
            name: name.to_string(),
            kind: McpKind::Stdio,
            enabled,
            targets,
            config,
            env: json!({}),
            headers: json!({}),
            created_at: 0,
            updated_at: 0,
            current_version: None,
            latest_version: None,
            last_update_check_at: None,
        }
    }

    fn scanned_fixture(key: &str, config: Value) -> crate::adapters::mcp_scan::ScannedMcp {
        crate::adapters::mcp_scan::ScannedMcp {
            target: crate::adapters::mcp_scan::ScanTarget::ClaudeCode,
            key: key.to_string(),
            name: key.to_string(),
            managed: false,
            kind: McpKind::Stdio,
            config,
            env_keys: Vec::new(),
            header_keys: Vec::new(),
            imported_id: None,
            name_conflict: false,
            adoptable: false,
        }
    }

    /// 同一服务器在两个客户端里的实测形状（本机 sequential-thinking / sequentialthinking）。
    fn same_server_two_shapes() -> (Value, Value) {
        (
            json!({"command": "npx", "args": ["-y", "@modelcontextprotocol/server-sequential-thinking"]}),
            json!({
                "command": "D:\\Program Files\\nodejs\\npx.cmd",
                "args": ["-y", "@modelcontextprotocol/server-sequential-thinking"],
                "type": "stdio",
                "enabled": true,
                "timeoutMs": 30_000,
            }),
        )
    }

    #[test]
    fn scan_marks_imported_id_by_identity_first_then_name() {
        let (bare, absolute) = same_server_two_shapes();
        let alpha = server_fixture("sequential-thinking", bare, vec![], true);
        let ssh = server_fixture(
            "ssh",
            json!({"command": "npx", "args": ["-y", "ssh-mcp", "--host", "h"]}),
            vec![],
            true,
        );
        let servers = vec![alpha.clone(), ssh.clone()];

        let mut entries = vec![
            // 键名完全不同，但粗身份命中 → 标成已纳管（这正是旧版漏掉、产生重复行的场景）。
            scanned_fixture("sequentialthinking", absolute),
            // 身份不命中（用户纳管后又改过 args），键名大小写不敏感地命中 → 名字兜底。
            scanned_fixture(
                "SSH",
                json!({"command": "npx", "args": ["-y", "ssh-mcp", "--host", "changed"]}),
            ),
            // 两者都不命中 → 保持未纳管。
            scanned_fixture("brand-new", json!({"command": "uvx", "args": ["zzz"]})),
        ];

        mark_imported_entries(&mut entries, &servers);

        assert_eq!(entries[0].imported_id.as_deref(), Some(alpha.id.as_str()));
        assert_eq!(entries[1].imported_id.as_deref(), Some(ssh.id.as_str()));
        assert_eq!(entries[2].imported_id, None);
        // R3 预告：名字兜底但身份不同 = 同名冲突（导了撞重名、应用了撞接管）。
        assert!(!entries[0].name_conflict, "身份命中的不是冲突");
        assert!(entries[1].name_conflict, "同名不同身份必须标冲突");
        assert!(!entries[2].name_conflict);
    }

    #[test]
    fn scan_marks_adoptable_only_for_enabled_covering_records() {
        let (bare, _) = same_server_two_shapes();
        let mut entry = scanned_fixture("sequential-thinking", bare.clone());
        entry.target = crate::adapters::mcp_scan::ScanTarget::ClaudeCode;

        // 记录启用且覆盖本目标 → 可接管预告。
        let covering =
            server_fixture("sequential-thinking", bare.clone(), vec![TargetKind::ClaudeCode], true);
        let mut entries = vec![entry.clone()];
        mark_imported_entries(&mut entries, &[covering]);
        assert!(entries[0].imported_id.is_some());
        assert!(entries[0].adoptable, "启用且覆盖目标时预告接管");

        // 记录禁用 → 只显示已纳管，不预告接管（禁用行不会被写盘）。
        let disabled =
            server_fixture("sequential-thinking", bare.clone(), vec![TargetKind::ClaudeCode], false);
        let mut entries = vec![entry.clone()];
        mark_imported_entries(&mut entries, &[disabled]);
        assert!(entries[0].imported_id.is_some());
        assert!(!entries[0].adoptable, "禁用记录不触发接管");

        // 记录不覆盖本目标 → 不预告（应用不会碰这个客户端的文件）。
        let elsewhere =
            server_fixture("sequential-thinking", bare.clone(), vec![TargetKind::Codex], true);
        let mut entries = vec![entry.clone()];
        mark_imported_entries(&mut entries, &[elsewhere]);
        assert!(!entries[0].adoptable, "不覆盖本目标时不预告接管");

        // 托管条目是自己的写入，无需接管自己。
        let mut managed = entry.clone();
        managed.managed = true;
        managed.key = "xiaobai_sequential-thinking".to_string();
        let covering =
            server_fixture("sequential-thinking", bare, vec![TargetKind::ClaudeCode], true);
        let mut entries = vec![managed];
        mark_imported_entries(&mut entries, &[covering]);
        assert!(!entries[0].adoptable, "托管条目不标可接管");
    }

    #[test]
    fn scan_new_fields_default_for_old_payloads() {
        // R5：旧版本发出的扫描载荷没有新字段，反序列化必须成功并取默认值。
        let entry: crate::adapters::mcp_scan::ScannedMcp = serde_json::from_value(json!({
            "target": "pi",
            "key": "legacy",
            "name": "legacy",
            "managed": false,
            "kind": "stdio",
            "config": {},
            "envKeys": [],
            "headerKeys": [],
        }))
        .unwrap();
        assert_eq!(entry.imported_id, None);
        assert!(!entry.name_conflict);
        assert!(!entry.adoptable);
    }

    #[test]
    fn import_name_collision_fails_without_touching_existing() {
        // 同名不同身份硬导：粗身份未命中，落到 save 的重名校验 → validation_failed，
        // 已有行不动、行数不变。界面靠 name_conflict 提前跳过，这里是后端兜底。
        let conn = db_conn();
        let crypto = test_crypto();
        let existing = repo::mcp::save(
            &conn,
            &crypto,
            input_fixture("taken", json!({"command": "uvx", "args": ["a"]}), vec![]),
        )
        .unwrap();

        let clash = input_fixture("Taken", json!({"command": "npx", "args": ["-y", "b"]}), vec![]);
        let error = import_entry(
            &conn,
            &crypto,
            crate::adapters::mcp_scan::ScanTarget::Pi,
            "taken",
            clash,
        )
        .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let name: String = conn
            .query_row(
                "SELECT name FROM mcp_servers WHERE id = ?1",
                params![existing.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "taken");
    }

    #[test]
    fn import_hitting_coarse_identity_creates_no_row_and_leaves_existing_untouched() {
        let conn = db_conn();
        let crypto = test_crypto();
        let (bare, absolute) = same_server_two_shapes();
        let existing =
            repo::mcp::save(&conn, &crypto, input_fixture("existing", bare, vec![TargetKind::ClaudeCode]))
                .unwrap();

        type Row = (String, String, String, i64, String, Option<String>);
        let snapshot = |conn: &Connection| -> Row {
            conn.query_row(
                "SELECT id, name, targets_json, updated_at, config_json, secrets_encrypted \
                 FROM mcp_servers WHERE id = ?1",
                params![existing.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap()
        };
        let before = snapshot(&conn);

        // 从 Codex 侧再纳管一次同一服务器：不同键名、绝对路径 command、带私有字段、还带密钥值。
        let mut dup = input_fixture("existing-copy", absolute, vec![]);
        dup.env = json!({"TOKEN": "PLACEHOLDER_SECRET"});
        let outcome =
            import_entry(&conn, &crypto, crate::adapters::mcp_scan::ScanTarget::Codex, "existing-copy", dup)
                .unwrap();

        match outcome {
            ImportOutcome::AlreadyImported(already) => {
                assert_eq!(already.existing_id, existing.id);
                assert_eq!(already.existing_name, "existing");
                // 只回定位符与已有行的 id/名字：不携带任何 config/env/headers。
                let serialized = serde_json::to_value(&already).unwrap();
                assert_eq!(serialized["existingId"], existing.id.as_str());
                assert_eq!(serialized["existingName"], "existing");
                let text = serialized.to_string();
                assert!(!text.contains("PLACEHOLDER_SECRET"), "secrets must not leave the backend");
                assert!(!text.contains("sequential-thinking"), "config must not be returned");
                assert!(serialized.get("config").is_none());
                assert!(serialized.get("env").is_none());
            }
            ImportOutcome::Imported(_) => panic!("同一粗身份不得建行"),
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "命中粗身份时不得新建行");
        assert_eq!(snapshot(&conn), before, "已有行必须逐字节原样：name/targets/updated_at/config 都不动");
    }

    #[test]
    fn import_with_new_identity_creates_row() {
        let conn = db_conn();
        let crypto = test_crypto();
        repo::mcp::save(&conn, &crypto, input_fixture("other", json!({"command": "uvx", "args": ["a"]}), vec![]))
            .unwrap();

        let outcome = import_entry(
            &conn,
            &crypto,
            crate::adapters::mcp_scan::ScanTarget::Pi,
            "fresh",
            input_fixture("fresh", json!({"command": "npx", "args": ["-y", "fresh-mcp"]}), vec![]),
        )
        .unwrap();

        match outcome {
            ImportOutcome::Imported(summary) => {
                assert_eq!(summary.name, "fresh");
                assert!(summary.targets.is_empty(), "目标选择仍归用户");
            }
            ImportOutcome::AlreadyImported(_) => panic!("不同身份必须建行"),
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn scan_target_union_groups_same_identity_across_clients() {
        let bare = json!({"command": "npx", "args": ["-y", "@modelcontextprotocol/server-sequential-thinking"]});
        // 同一份配置出现在 Claude 与 Pi；另一份只在 Codex。
        let mut on_claude = scanned_fixture("seq", bare.clone());
        on_claude.target = crate::adapters::mcp_scan::ScanTarget::ClaudeCode;
        let mut on_pi = scanned_fixture("seq", bare.clone());
        on_pi.target = crate::adapters::mcp_scan::ScanTarget::Pi;
        let mut other = scanned_fixture("other", json!({"command": "uvx", "args": ["x"]}));
        other.target = crate::adapters::mcp_scan::ScanTarget::Codex;

        let map = scan_target_union(&[on_claude, on_pi, other.clone()]);

        let seq_identity = mcp_identity::coarse_identity(McpKind::Stdio, &bare);
        assert_eq!(
            map.get(&seq_identity),
            Some(&vec![TargetKind::ClaudeCode, TargetKind::Pi]),
            "同款配置的两端并集，按四端固定序去重"
        );
        let other_identity = mcp_identity::coarse_identity(McpKind::Stdio, &other.config);
        assert_eq!(map.get(&other_identity), Some(&vec![TargetKind::Codex]));
    }

    #[test]
    fn scan_target_union_dedupes_duplicate_target() {
        // 极端形状：同一目标里被扫出两条同款（理论上不该发生，但并集不得出现重复目标）。
        let config = json!({"command": "npx", "args": ["a"]});
        let a = scanned_fixture("dup-a", config.clone());
        let b = scanned_fixture("dup-b", config.clone());
        let map = scan_target_union(&[a, b]);
        let identity = mcp_identity::coarse_identity(McpKind::Stdio, &config);
        assert_eq!(map.get(&identity), Some(&vec![TargetKind::ClaudeCode]));
    }

    #[test]
    fn import_batch_of_two_same_identity_creates_exactly_one_row() {
        // 「全部纳管」一次勾了同一服务器的两个客户端条目：第二条必须看到第一条刚建的行。
        // 这就是 import_entry 每条都重读 list_full 的原因——把读表提到循环外会漏判、建出重复行。
        let conn = db_conn();
        let crypto = test_crypto();
        let (bare, absolute) = same_server_two_shapes();

        let first = import_entry(
            &conn,
            &crypto,
            crate::adapters::mcp_scan::ScanTarget::Codex,
            "sequential-thinking",
            input_fixture("sequential-thinking", bare, vec![]),
        )
        .unwrap();
        let created_id = match first {
            ImportOutcome::Imported(summary) => summary.id,
            ImportOutcome::AlreadyImported(_) => panic!("空库里第一条应当建行"),
        };

        let second = import_entry(
            &conn,
            &crypto,
            crate::adapters::mcp_scan::ScanTarget::ClaudeCode,
            "sequentialthinking",
            input_fixture("sequentialthinking", absolute, vec![]),
        )
        .unwrap();
        match second {
            ImportOutcome::AlreadyImported(already) => {
                assert_eq!(already.existing_id, created_id, "必须指回同批次刚建的那一行");
            }
            ImportOutcome::Imported(_) => panic!("同批次内也不得建出第二行"),
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn save_guard_blocks_other_rows_identity_and_allows_self_edit() {
        let (bare, absolute) = same_server_two_shapes();
        let alpha = server_fixture("alpha", bare, vec![], true);
        let beta = server_fixture("beta", json!({"command": "uvx", "args": ["y"]}), vec![], true);
        let servers = vec![alpha.clone(), beta.clone()];

        // 新增行命中他行粗身份 → validation_failed 且信息点名冲突条目。
        let new = input_fixture("gamma", absolute.clone(), vec![]);
        let error = guard_identity_collision(&servers, &new).unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert!(error.to_string().contains("alpha"), "{error}");

        // 按 id 编辑自身（库里还留着存量重复也要能改）→ 放行。
        let mut self_edit = input_fixture("gamma", absolute, vec![]);
        self_edit.id = Some(alpha.id.clone());
        assert!(guard_identity_collision(&servers, &self_edit).is_ok());

        // 全新身份 → 放行。
        let fresh = input_fixture("delta", json!({"command": "uvx", "args": ["z"]}), vec![]);
        assert!(guard_identity_collision(&servers, &fresh).is_ok());

        // 编辑成与「另一行」同身份 → 同样拒绝（不允许靠编辑制造重复）。
        let mut cross_edit = self_edit;
        cross_edit.id = Some(alpha.id.clone());
        cross_edit.config = beta.config.clone();
        let error = guard_identity_collision(&servers, &cross_edit).unwrap_err();
        assert!(error.to_string().contains("beta"), "{error}");
    }

    #[test]
    fn identity_conflicts_only_groups_enabled_duplicates() {
        let (bare, absolute) = same_server_two_shapes();
        let a = server_fixture("a", bare.clone(), vec![], true);
        let mut disabled_twin = server_fixture("b", absolute.clone(), vec![], false);
        assert_eq!(identity_conflicts(&[a.clone(), disabled_twin.clone()]), Vec::<Vec<String>>::new());

        disabled_twin.enabled = true;
        assert_eq!(
            identity_conflicts(&[a.clone(), disabled_twin.clone()]),
            vec![vec!["a".to_string(), "b".to_string()]]
        );

        // 不同身份不构成组。
        let other = server_fixture("c", json!({"command": "uvx", "args": ["x"]}), vec![], true);
        assert_eq!(identity_conflicts(&[a, other]), Vec::<Vec<String>>::new());
    }

    #[test]
    fn apply_refuses_duplicate_identity_target_and_keeps_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let claude_dir = dir.path().join("claude");
        let codex_dir = dir.path().join("codex");
        let backup = dir.path().join("backup");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::create_dir_all(&backup).unwrap();
        let claude_path = claude_dir.join(".claude.json");
        let original = r#"{"mcpServers":{"user-server":{"command":"user-cmd"}}}"#;
        fs::write(&claude_path, original).unwrap();

        let settings = AppSettings {
            claude_home_override: Some(claude_dir.to_string_lossy().to_string()),
            codex_home_override: Some(codex_dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        // 同一服务器的两行（本机 sequential-thinking / sequentialthinking 的实测形状）都指向
        // Claude；第三行身份独立、指向 Codex，用来验证「其余目标照常应用」。
        let (bare, absolute) = same_server_two_shapes();
        let a = server_fixture("sequential-thinking", bare, vec![TargetKind::ClaudeCode], true);
        let b = server_fixture("sequentialthinking", absolute, vec![TargetKind::ClaudeCode], true);
        let c = server_fixture(
            "tavily",
            json!({"command": "npx", "args": ["-y", "tavily-mcp"]}),
            vec![TargetKind::Codex],
            true,
        );

        let results = apply_servers_to_targets(
            &settings,
            &[a, b, c],
            &[],
            &[TargetKind::ClaudeCode, TargetKind::Codex],
            &backup,
        );

        let claude = results
            .iter()
            .find(|result| result.target == TargetKind::ClaudeCode)
            .expect("冲突目标必须逐条上报");
        assert!(!claude.ok, "同粗身份的两条不得写入");
        assert!(
            claude.message.contains("sequential-thinking")
                && claude.message.contains("sequentialthinking"),
            "失败信息必须点名冲突的两条: {}",
            claude.message
        );
        assert!(claude.backup_paths.is_empty());
        assert_eq!(
            fs::read_to_string(&claude_path).unwrap(),
            original,
            "跳过时不得写盘：既没有 xiaobai_sequential-thinking 也没有 xiaobai_sequentialthinking"
        );
        assert!(!original.contains("xiaobai_"));

        let codex = results
            .iter()
            .find(|result| result.target == TargetKind::Codex)
            .expect("其余目标必须继续应用");
        assert!(codex.ok, "其余目标不受影响: {}", codex.message);
        let text = fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(text.contains("xiaobai_tavily"), "{text}");
        assert!(!text.contains("xiaobai_sequential"), "冲突两条哪儿都不写: {text}");
    }

    #[test]
    fn apply_without_conflicts_writes_all_targets() {
        // 回归：没有身份冲突时行为与改动前一致（含「上次应用过的目标」清理并集语义）。
        let dir = tempfile::tempdir().unwrap();
        let pi_dir = dir.path().join("pi");
        let backup = dir.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        let settings = AppSettings {
            pi_agent_dir_override: Some(pi_dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        let a = server_fixture("solo", json!({"command": "npx", "args": ["-y", "solo"]}), vec![TargetKind::Pi], true);
        let results = apply_servers_to_targets(&settings, &[a], &[], &[TargetKind::Pi], &backup);
        assert_eq!(results.len(), 1);
        assert!(results[0].ok, "{}", results[0].message);
        let text = fs::read_to_string(pi_dir.join("mcp.json")).unwrap();
        assert!(text.contains("xiaobai_solo"), "{text}");
    }

    // ------------------------------------------------------------------
    // 纳管即接管：本次纳管涉及的客户端并集计算（决定接管写盘打到哪些目标）。
    // ------------------------------------------------------------------

    fn summary_fixture(id: &str, targets: Vec<TargetKind>) -> McpServerSummary {
        McpServerSummary {
            id: id.to_string(),
            name: id.to_string(),
            kind: McpKind::Stdio,
            enabled: true,
            targets,
            created_at: 0,
            updated_at: 0,
            current_version: None,
            latest_version: None,
            last_update_check_at: None,
            absolute_command: false,
        }
    }

    fn already_imported_fixture(existing_id: &str) -> AlreadyImportedMcp {
        AlreadyImportedMcp {
            target: crate::adapters::mcp_scan::ScanTarget::ClaudeCode,
            key: existing_id.to_string(),
            existing_id: existing_id.to_string(),
            existing_name: existing_id.to_string(),
        }
    }

    #[test]
    fn touched_targets_union_new_rows_and_dedupe() {
        // 首次纳管多客户端 MCP：imported 行的 targets 就是并集，跨行再去重。
        let touched = merge_touched_targets(
            &[
                summary_fixture("a", vec![TargetKind::ClaudeCode, TargetKind::Codex]),
                summary_fixture("b", vec![TargetKind::Codex, TargetKind::Pi]),
            ],
            &[],
            &[],
        );
        assert_eq!(touched.len(), 3, "并集去重: {touched:?}");
        assert!(touched.contains(&TargetKind::ClaudeCode));
        assert!(touched.contains(&TargetKind::Codex));
        assert!(touched.contains(&TargetKind::Pi));
    }

    #[test]
    fn touched_targets_use_existing_record_for_already_imported() {
        // 重复纳管同款：命中已有行，用它当前的 targets 补接管尚未写盘的客户端。
        let existing = server_fixture(
            "seq",
            json!({"command": "npx"}),
            vec![TargetKind::ClaudeCode, TargetKind::Prime],
            true,
        );
        let touched = merge_touched_targets(
            &[],
            &[already_imported_fixture(&existing.id)],
            &[existing.clone()],
        );
        assert_eq!(touched, vec![TargetKind::ClaudeCode, TargetKind::Prime]);
    }

    #[test]
    fn touched_targets_empty_when_nothing_imported() {
        // 全部 failed（无 imported / already_imported）→ 无目标需要接管，不触发写盘。
        assert!(merge_touched_targets(&[], &[], &[]).is_empty());
    }

    // ------------------------------------------------------------------
    // 纳管即接管：端到端（tempdir 客户端文件）。覆盖 import_scanned_mcp_impl 的接线本身
    // ——若删掉「入库后调 apply_to_targets」两行，这些断言就会失败。
    // ------------------------------------------------------------------

    use std::sync::atomic::{AtomicBool, AtomicI64};

    /// 串行化会改真实进程环境变量（XIAOBAI_SWITCH_DATA_DIR）的用例，避免并行互相踩。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_state(conn: Connection) -> AppState {
        AppState {
            db: crate::db::Db { conn: std::sync::Mutex::new(conn) },
            crypto: test_crypto(),
            close_to_tray: AtomicBool::new(true),
            start_in_tray: AtomicBool::new(false),
            is_quitting: AtomicBool::new(false),
            webdav_sync_handle: tokio::sync::Mutex::new(None),
            webdav_operation: tokio::sync::Mutex::new(()),
            next_webdav_sync_at: AtomicI64::new(0),
            local_proxy: tokio::sync::Mutex::new(None),
        }
    }

    fn claude_mcp_keys(path: &Path) -> Vec<String> {
        let text = fs::read_to_string(path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        value
            .get("mcpServers")
            .and_then(|servers| servers.as_object())
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn codex_mcp_keys(path: &Path) -> Vec<String> {
        let text = fs::read_to_string(path).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        doc.get("mcp_servers")
            .and_then(|item| item.as_table_like())
            .map(|table| table.iter().map(|(k, _)| k.to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn adopt_takes_over_all_matching_clients_and_delete_cleans_them() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        // backups_dir() 走 app_dir()，靠这个环境变量隔离到 tempdir。
        std::env::set_var("XIAOBAI_SWITCH_DATA_DIR", &data_dir);

        let claude_dir = dir.path().join("claude");
        let codex_dir = dir.path().join("codex");
        let pi_dir = dir.path().join("pi");
        let prime_dir = dir.path().join("prime");
        for d in [&claude_dir, &codex_dir, &pi_dir, &prime_dir] {
            fs::create_dir_all(d).unwrap();
        }
        // 同一个 MCP 手工配在 Claude 与 Codex（同粗身份：command + args 一致）。
        let claude_path = claude_dir.join(".claude.json");
        fs::write(
            &claude_path,
            r#"{"mcpServers":{"existing-fs":{"command":"npx","args":["-y","fs-mcp"]}}}"#,
        )
        .unwrap();
        let codex_path = codex_dir.join("config.toml");
        fs::write(
            &codex_path,
            "[mcp_servers.existing-fs]\ncommand = \"npx\"\nargs = [\"-y\", \"fs-mcp\"]\n",
        )
        .unwrap();

        let conn = db_conn();
        let settings = AppSettings {
            claude_home_override: Some(claude_dir.to_string_lossy().to_string()),
            codex_home_override: Some(codex_dir.to_string_lossy().to_string()),
            pi_agent_dir_override: Some(pi_dir.to_string_lossy().to_string()),
            prime_agent_dir_override: Some(prime_dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        repo::settings::save_settings(&conn, &settings).unwrap();
        let state = test_state(conn);

        // 从 Claude 纳管一次。
        let result = import_scanned_mcp_impl(
            &state,
            vec![McpImportLocator {
                target: crate::adapters::mcp_scan::ScanTarget::ClaudeCode,
                key: "existing-fs".to_string(),
            }],
        )
        .unwrap();

        // 单条记录、targets = 并集 [Claude, Codex]。
        assert_eq!(result.imported.len(), 1, "只建一条记录");
        let servers = state.db.with_conn(|c| repo::mcp::list(c, &state.crypto)).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "existing-fs");
        assert!(servers[0].targets.contains(&TargetKind::ClaudeCode));
        assert!(servers[0].targets.contains(&TargetKind::Codex));

        // 接管全绿，两端都写了盘。
        let apply = result.apply.expect("纳管应触发接管写盘");
        assert!(apply.results.iter().all(|r| r.ok), "接管应全成功: {apply:?}");

        // Claude：手工键消失，托管键出现。
        let keys = claude_mcp_keys(&claude_path);
        assert!(keys.contains(&"xiaobai_existing-fs".to_string()), "{keys:?}");
        assert!(!keys.contains(&"existing-fs".to_string()), "手工条目须被接管删除: {keys:?}");

        // Codex：同理（按解析后的 mcp_servers 键判定，避免 xiaobai_ 前缀的子串误伤）。
        let codex_keys = codex_mcp_keys(&codex_path);
        assert!(codex_keys.contains(&"xiaobai_existing-fs".to_string()), "{codex_keys:?}");
        assert!(
            !codex_keys.contains(&"existing-fs".to_string()),
            "Codex 手工条目须被接管删除: {codex_keys:?}"
        );

        // 删除该记录：两端托管条目一起清干净（复刻 delete_mcp_server 的两步）。
        let id = servers[0].id.clone();
        state.db.with_conn(|c| repo::mcp::delete(c, &id)).unwrap();
        apply_to_targets(&state, &[]).unwrap();

        assert!(
            claude_mcp_keys(&claude_path).is_empty(),
            "删除后 Claude 不应残留任何托管条目"
        );
        assert!(
            codex_mcp_keys(&codex_path).is_empty(),
            "删除后 Codex 不应残留托管条目"
        );

        std::env::remove_var("XIAOBAI_SWITCH_DATA_DIR");
    }
}
