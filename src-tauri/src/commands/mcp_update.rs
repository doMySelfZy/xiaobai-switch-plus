//! MCP 版本检查与更新的 Tauri 命令。

use crate::adapters::mcp_update;
use crate::error::AppResult;
use crate::repo::{mcp, mcp_version};
use crate::state::AppState;
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 每个 MCP 一次 npm 检查（子进程 + registry HTTP）。改前是串行跑完所有服务器，
/// 条目一多就要等到天荒地老；这里改成有界并发 2 —— Windows 上每个 npm 都是
/// cmd.exe + node.exe，一次拉起全部条目会明显抢资源。
const MCP_CHECK_CONCURRENCY: usize = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct McpUpdateStatus {
    pub id: String,
    pub name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub last_check_at: Option<i64>,
}

/// 单个 MCP 的检查结果（尚未落库）。
struct McpCheck {
    index: usize,
    id: String,
    name: String,
    current_version: Option<String>,
    latest_version: Option<String>,
    has_update: bool,
    /// 成功查到最新版本时才有值，用于写库。
    checked: bool,
}

/// 检查所有已安装 MCP 的更新状态
#[tauri::command]
pub async fn check_mcp_updates(state: State<'_, AppState>) -> AppResult<Vec<McpUpdateStatus>> {
    let servers = state
        .db
        .with_conn(|conn| mcp::list_full(conn, &state.crypto))?;
    let now = Utc::now().timestamp();

    let checks = futures_util::stream::iter(servers.into_iter().enumerate().map(
        |(index, server)| async move {
            // 从 config 中提取包名
            let package_name = extract_package_name(&server.config);

            let Some(pkg_name) = package_name else {
                // 无法提取包名，使用数据库中的信息
                return McpCheck {
                    index,
                    id: server.id,
                    name: server.name,
                    current_version: server.current_version,
                    latest_version: server.latest_version,
                    has_update: false,
                    checked: false,
                };
            };

            match mcp_update::check_npm_package_version(&pkg_name).await {
                Ok(result) => McpCheck {
                    index,
                    id: server.id,
                    name: server.name,
                    current_version: result.current_version,
                    latest_version: result.latest_version,
                    has_update: result.has_update,
                    checked: true,
                },
                Err(e) => {
                    tracing::warn!("检查 MCP {} 版本失败: {}", server.name, e);
                    McpCheck {
                        index,
                        id: server.id,
                        name: server.name,
                        current_version: server.current_version,
                        latest_version: server.latest_version,
                        has_update: false,
                        checked: false,
                    }
                }
            }
        },
    ))
    .buffer_unordered(MCP_CHECK_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    // 并发完成顺序不定，落库与返回都按原顺序来。
    let mut checks = checks;
    checks.sort_by_key(|check| check.index);

    let mut results = Vec::with_capacity(checks.len());
    for check in checks {
        // 落库放在并发段之后：不跨 await 持锁。
        if check.checked {
            state.db.with_conn(|conn| {
                mcp_version::update_version_info(
                    conn,
                    &check.id,
                    check.current_version.clone(),
                    check.latest_version.clone(),
                    now,
                )
            })?;
        }
        results.push(McpUpdateStatus {
            id: check.id,
            name: check.name,
            current_version: check.current_version,
            latest_version: check.latest_version,
            has_update: check.has_update,
            last_check_at: Some(now),
        });
    }

    Ok(results)
}

/// 更新单个 MCP 到最新版本
#[tauri::command]
pub async fn update_mcp_server(state: State<'_, AppState>, id: String) -> AppResult<String> {
    // 获取服务器信息
    let server = state
        .db
        .with_conn(|conn| mcp::get(conn, &id, &state.crypto))?;

    // 提取包名
    let package_name = extract_package_name(&server.config)
        .ok_or_else(|| crate::error::AppError::new("mcp_update", "无法确定包名"))?;

    // 执行更新
    let updated_version = mcp_update::update_npm_package(&package_name).await?;

    // 更新数据库
    let now = Utc::now().timestamp();
    state.db.with_conn(|conn| {
        mcp_version::update_version_info(
            conn,
            &id,
            Some(updated_version.clone()),
            Some(updated_version.clone()),
            now,
        )
    })?;

    Ok(updated_version)
}

/// 批量更新所有有更新的 MCP
///
/// 与 agent 批量更新同理，安装保持串行：并发 `npm install -g` 会同时写同一个全局
/// prefix。超时与 kill 在适配器里。
#[tauri::command]
pub async fn batch_update_mcp_servers(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<Vec<(String, Result<String, String>)>> {
    let mut results = Vec::new();

    for id in ids {
        let result = match update_mcp_server(state.clone(), id.clone()).await {
            Ok(version) => Ok(version),
            Err(e) => Err(e.to_string()),
        };
        results.push((id, result));
    }

    Ok(results)
}

/// 从 MCP config 中提取 npm 包名
fn extract_package_name(config: &serde_json::Value) -> Option<String> {
    // 尝试从 command 字段提取
    if let Some(command) = config.get("command").and_then(|v| v.as_str()) {
        // npx @modelcontextprotocol/server-filesystem
        if command.starts_with("npx ") {
            let package = command.strip_prefix("npx ")?.trim();
            // 移除可能的参数
            let package = package.split_whitespace().next()?;
            return Some(package.to_string());
        }

        // 直接是包名的情况
        if command.contains("@modelcontextprotocol/") || command.contains("mcp-") {
            return Some(command.to_string());
        }
    }

    // 尝试从 url 字段提取 (对于 npx 远程包)
    if let Some(url) = config.get("url").and_then(|v| v.as_str()) {
        if url.contains("npm:") {
            let package = url.strip_prefix("npm:")?;
            return Some(package.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_package_name() {
        let config1 = json!({
            "command": "npx @modelcontextprotocol/server-filesystem"
        });
        assert_eq!(
            extract_package_name(&config1),
            Some("@modelcontextprotocol/server-filesystem".to_string())
        );

        let config2 = json!({
            "command": "npx @modelcontextprotocol/server-filesystem /path/to/dir"
        });
        assert_eq!(
            extract_package_name(&config2),
            Some("@modelcontextprotocol/server-filesystem".to_string())
        );
    }
}
