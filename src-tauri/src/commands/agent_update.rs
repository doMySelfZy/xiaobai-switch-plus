use crate::adapters::agent_update::{
    check_npm_package_version, get_npm_package_name, update_npm_package,
};
use crate::domain::AgentUpdateStatus;
use crate::error::{AppError, AppResult};
use crate::repo::agent_update as repo_agent_update;
use crate::state::AppState;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 检查版本是「子进程 + 网络」：改前 4 个 agent 串行跑，最坏要十几秒才回。
/// 这里改成有界并发。并发度只给 2：Windows 上每个 npm 都是一层 cmd.exe + node.exe，
/// 一次拉起 4 个（更不用说 4 × 2 = 8 个）会明显抢资源。
const AGENT_CHECK_CONCURRENCY: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUpdateResult {
    pub successes: Vec<String>,
    pub failures: Vec<(String, String)>,
}

struct AgentCheck {
    kind: &'static str,
    name: &'static str,
    current_version: Option<String>,
    latest_version: Option<String>,
}

/// 单个 agent 的版本检查：本地已装版本 + registry 上的最新版本。
/// 复用 `check_npm_package_version`，不另写一套请求逻辑。
async fn check_agent(kind: &'static str, name: &'static str, package_name: &'static str) -> Option<AgentCheck> {
    match check_npm_package_version(package_name).await {
        Ok((current_version, latest_version)) => Some(AgentCheck {
            kind,
            name,
            current_version,
            latest_version,
        }),
        Err(e) => {
            eprintln!("Failed to check {} updates: {}", kind, e);
            None
        }
    }
}

fn status_of(check: AgentCheck) -> AgentUpdateStatus {
    let has_update = match (&check.current_version, &check.latest_version) {
        (Some(current), Some(latest)) => {
            // Compare versions using semver
            match (
                semver::Version::parse(current),
                semver::Version::parse(latest),
            ) {
                (Ok(current_ver), Ok(latest_ver)) => latest_ver > current_ver,
                _ => false,
            }
        }
        _ => false,
    };

    AgentUpdateStatus {
        kind: check.kind.to_string(),
        name: check.name.to_string(),
        current_version: check.current_version,
        latest_version: check.latest_version,
        has_update,
        last_check_at: Some(chrono::Utc::now().timestamp()),
    }
}

/// Check updates for all installed agents
#[tauri::command]
pub async fn check_agent_updates(state: State<'_, AppState>) -> AppResult<Vec<AgentUpdateStatus>> {
    // 用显式循环构造 future 列表，不用 `filter_map` 闭包：`#[tauri::command]` 展开后
    // 对闭包的泛型生命周期要求（FnOnce for any lifetime）会让「闭包返回 async block」
    // 直接编译失败。
    const AGENT_KINDS: [(&str, &str); 4] = [
        ("claude_code", "Claude Code"),
        ("codex", "Codex"),
        ("pi", "Pi"),
        ("prime", "Prime"),
    ];

    let mut pending = Vec::with_capacity(AGENT_KINDS.len());
    for (index, (kind, name)) in AGENT_KINDS.into_iter().enumerate() {
        let Some(package_name) = get_npm_package_name(kind) else {
            continue;
        };
        pending.push(async move { (index, check_agent(kind, name, package_name).await) });
    }

    let checks = futures_util::stream::iter(pending)
        .buffer_unordered(AGENT_CHECK_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    // 并发完成的顺序是不定的，落库/返回按固定顺序，界面与测试才稳定。
    let mut checked: Vec<(usize, AgentUpdateStatus)> = checks
        .into_iter()
        .filter_map(|(index, check)| check.map(|check| (index, status_of(check))))
        .collect();
    checked.sort_by_key(|(index, _)| *index);

    // DB 写入放在并发段之后：不跨 await 持锁，也不和其它 npm 调用抢锁。
    let mut statuses = Vec::with_capacity(checked.len());
    for (_, status) in checked {
        let _ = state
            .db
            .with_conn(|conn| repo_agent_update::upsert_agent_update_status(conn, &status));
        statuses.push(status);
    }

    Ok(statuses)
}

/// Update a specific agent
#[tauri::command]
pub async fn update_agent(kind: String, state: State<'_, AppState>) -> AppResult<String> {
    let package_name = get_npm_package_name(&kind)
        .ok_or_else(|| AppError::new("unknown_agent", format!("Unknown agent kind: {}", kind)))?;

    let new_version = update_npm_package(package_name)
        .await
        .map_err(|e| AppError::new("update_failed", e.to_string()))?;

    // Update database status
    let _ = state.db.with_conn(|conn| {
        if let Ok(Some(mut status)) = repo_agent_update::get_agent_update_status(conn, &kind) {
            status.current_version = Some(new_version.clone());
            status.has_update = false;
            status.last_check_at = Some(chrono::Utc::now().timestamp());
            let _ = repo_agent_update::upsert_agent_update_status(conn, &status);
        }
        Ok(())
    });

    Ok(new_version)
}

/// Batch update multiple agents
///
/// 安装刻意保持串行：并发跑多个 `npm install -g` 会同时写同一个全局 prefix，
/// 装出来的 bin 链接可能互相踩。这里是用户点的批量操作、有 loading 状态，
/// 慢一点可以接受（超时与 kill 在适配器里）。
#[tauri::command]
pub async fn batch_update_agents(
    kinds: Vec<String>,
    state: State<'_, AppState>,
) -> AppResult<BatchUpdateResult> {
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    for kind in kinds {
        match update_agent(kind.clone(), state.clone()).await {
            Ok(_) => successes.push(kind),
            Err(e) => failures.push((kind, e.to_string())),
        }
    }

    Ok(BatchUpdateResult {
        successes,
        failures,
    })
}

/// Get all agent update statuses from database
#[tauri::command]
pub async fn get_all_agent_update_statuses(
    state: State<'_, AppState>,
) -> AppResult<Vec<AgentUpdateStatus>> {
    state.db.with_conn(|conn| {
        repo_agent_update::get_all_agent_update_statuses(conn)
    })
}

/// Get single agent update status
#[tauri::command]
pub async fn get_agent_update_status(
    kind: String,
    state: State<'_, AppState>,
) -> AppResult<Option<AgentUpdateStatus>> {
    state.db.with_conn(|conn| {
        repo_agent_update::get_agent_update_status(conn, &kind)
    })
}

/// Clear/delete agent update status
#[tauri::command]
pub async fn clear_agent_update_status(
    kind: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.db.with_conn(|conn| {
        repo_agent_update::delete_agent_update_status(conn, &kind)
    })
}
