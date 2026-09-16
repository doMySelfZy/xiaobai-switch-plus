//! MCP 版本检查与更新逻辑。

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// npm registry 查询的超时。改前 `reqwest::Client::new()` 没有任何超时：
/// registry 一旦不响应，这次检查会永久挂起（UI 上表现为一直转圈）。
const NPM_REGISTRY_TIMEOUT: Duration = Duration::from_secs(15);
const NPM_REGISTRY_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// `npm list` / `npm install` 的超时；超时后子进程会被结束，不留孤儿。
const NPM_QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// registry 客户端：显式设置超时，并带上应用自己的 user agent。
/// 走 `reqwest` 的默认代理解析（env 代理），与改前行为一致。
fn registry_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(NPM_REGISTRY_TIMEOUT)
        .connect_timeout(NPM_REGISTRY_CONNECT_TIMEOUT)
        .user_agent(crate::http_client::default_user_agent())
        .build()
        .map_err(|e| AppError::new("mcp_update", format!("创建 HTTP 客户端失败: {}", e)))
}

/// 跑一次 npm，带超时；超时后用 `taskkill /T` 收掉整棵进程树（Windows 上
/// npm.cmd 是 cmd.exe 包着 node.exe）。
async fn run_npm(args: &[&str], timeout: Duration) -> AppResult<std::process::Output> {
    let mut command = tokio::process::Command::new("npm");
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let label = args.join(" ");
    let child = command
        .spawn()
        .map_err(|e| AppError::new("mcp_update", format!("无法执行 npm {label}: {e}")))?;
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => output
            .map_err(|e| AppError::new("mcp_update", format!("执行 npm {label} 失败: {e}"))),
        Err(_) => {
            terminate_process_tree(pid);
            Err(AppError::new(
                "mcp_update",
                format!("npm {label} 超时（{}s）", timeout.as_secs()),
            ))
        }
    }
}

fn terminate_process_tree(pid: Option<u32>) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let Some(pid) = pid else {
            return;
        };
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub has_update: bool,
}

/// 检查 npm 包的当前版本和最新版本
pub async fn check_npm_package_version(package_name: &str) -> AppResult<UpdateCheckResult> {
    // 1. 获取本地已安装版本
    let current_version = get_installed_npm_version(package_name).await?;

    // 2. 从 npm registry 获取最新版本
    let latest_version = get_latest_npm_version(package_name).await?;

    // 3. 比较版本
    let has_update = match (&current_version, &latest_version) {
        (Some(current), Some(latest)) => current != latest,
        _ => false,
    };

    Ok(UpdateCheckResult {
        current_version,
        latest_version,
        has_update,
    })
}

/// 获取本地已安装的 npm 包版本
async fn get_installed_npm_version(package_name: &str) -> AppResult<Option<String>> {
    // 使用 npm list 命令检查本地版本
    let output = run_npm(
        &["list", package_name, "--global", "--json", "--depth=0"],
        NPM_QUERY_TIMEOUT,
    )
    .await?;

    if !output.status.success() {
        // 可能是包未安装
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let list_result: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| AppError::new("mcp_update", format!("解析 npm list 输出失败: {}", e)))?;

    // 从 JSON 中提取版本号
    let version = list_result
        .get("dependencies")
        .and_then(|deps| deps.get(package_name))
        .and_then(|pkg| pkg.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(version)
}

/// 从 npm registry 获取包的最新版本
async fn get_latest_npm_version(package_name: &str) -> AppResult<Option<String>> {
    let registry_url = format!("https://registry.npmjs.org/{}", package_name);

    let response = registry_client()?
        .get(&registry_url)
        .send()
        .await
        .map_err(|e| AppError::new("mcp_update", format!("请求 npm registry 失败: {}", e)))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let package_info: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::new("mcp_update", format!("解析 npm registry 响应失败: {}", e)))?;

    // 获取 dist-tags.latest
    let latest_version = package_info
        .get("dist-tags")
        .and_then(|tags| tags.get("latest"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(latest_version)
}

/// 更新 npm 包到最新版本
pub async fn update_npm_package(package_name: &str) -> AppResult<String> {
    let output = run_npm(&["install", "-g", package_name], NPM_INSTALL_TIMEOUT).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::new("mcp_update", format!("npm 更新失败: {}", stderr)));
    }

    // 获取更新后的版本
    let updated_version = get_installed_npm_version(package_name)
        .await?
        .unwrap_or_else(|| "unknown".to_string());

    Ok(updated_version)
}

/// 检查 npx 远程包的版本（对于 npx 执行的包）
pub async fn check_npx_package_version(package_spec: &str) -> AppResult<UpdateCheckResult> {
    // npx 包通常格式是 package@version 或只是 package
    let package_name = package_spec
        .split('@')
        .next()
        .unwrap_or(package_spec);

    // 对于 npx 包，我们只能检查 registry 的最新版本
    // 无法获取"当前版本"，因为 npx 每次都会下载
    let latest_version = get_latest_npm_version(package_name).await?;

    Ok(UpdateCheckResult {
        current_version: None,
        latest_version,
        has_update: false, // npx 包无法比较
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // 需要网络连接
    async fn test_check_npm_version() {
        let result = check_npm_package_version("@modelcontextprotocol/server-filesystem")
            .await
            .unwrap();
        println!("Current: {:?}", result.current_version);
        println!("Latest: {:?}", result.latest_version);
        println!("Has update: {}", result.has_update);
    }
}
