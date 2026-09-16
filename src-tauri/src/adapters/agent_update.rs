use anyhow::{Context, Result};
use std::process::Output;
use std::time::Duration;

/// 版本查询类 npm 调用的超时：`npm list` / `npm view` 正常都在数秒内返回。
const NPM_QUERY_TIMEOUT: Duration = Duration::from_secs(20);

/// 全局安装的超时：慢网下 `npm install -g` 可以跑几分钟，但不能无限挂。
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Get NPM package name for the given agent kind
pub fn get_npm_package_name(kind: &str) -> Option<&'static str> {
    match kind {
        "claude_code" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@gitnexus/codex"),
        "pi" => Some("@picoding/pi"),
        "prime" => Some("@picoding/prime"),
        _ => None,
    }
}

/// 跑一次 npm，带超时。
///
/// 改前用的是 `std::process::Command::output()`（阻塞、无超时）：既占着 tokio 的工作线程，
/// npm 一旦挂起还会把整个检查永久拖住。这里换成 tokio 子进程 + `kill_on_drop`，
/// 超时后先 `taskkill /T` 结束整棵进程树（Windows 上 npm.cmd 是 cmd.exe 包着 node.exe，
/// 只 kill 直接子进程会留下 node 孤儿），再由 `kill_on_drop` 兜底。
async fn run_npm(args: &[&str], timeout: Duration) -> Result<Output> {
    let mut command = tokio::process::Command::new("npm");
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let label = args.join(" ");
    let child = command
        .spawn()
        .with_context(|| format!("Failed to execute npm {label}"))?;
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => output.with_context(|| format!("Failed to run npm {label}")),
        Err(_) => {
            terminate_process_tree(pid);
            anyhow::bail!("npm {label} timed out after {}s", timeout.as_secs())
        }
    }
}

/// 结束 npm 整棵进程树；非 Windows 上 `kill_on_drop` 已经够了。
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

/// Check current installed version and latest version from NPM
pub async fn check_npm_package_version(
    package_name: &str,
) -> Result<(Option<String>, Option<String>)> {
    // Check current installed version
    let current_version = get_current_installed_version(package_name).await;

    // Check latest version from NPM
    let latest_version = get_npm_latest_version(package_name).await?;

    Ok((current_version, Some(latest_version)))
}

/// Get currently installed version using `npm list -g --depth=0`
async fn get_current_installed_version(package_name: &str) -> Option<String> {
    let output = run_npm(
        &["list", "-g", "--depth=0", package_name, "--json"],
        NPM_QUERY_TIMEOUT,
    )
    .await
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    // Parse version from JSON structure
    json.get("dependencies")?
        .get(package_name)?
        .get("version")?
        .as_str()
        .map(|s| s.to_string())
}

/// Get latest version from NPM registry using `npm view`
///
/// 这里刻意保留 `npm view` 而不是直接请求 registry.npmjs.org：npm 会带上用户自己的
/// registry 配置（镜像、私有源、代理）与认证，硬编码官方 registry 会让配了镜像的用户
/// 直接查不到版本——那是行为回退。用子进程换来的是超时可控，不是行为改变。
async fn get_npm_latest_version(package_name: &str) -> Result<String> {
    let output = run_npm(&["view", package_name, "version"], NPM_QUERY_TIMEOUT)
        .await
        .context("Failed to execute npm view command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("npm view failed: {}", stderr);
    }

    let version = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in npm output")?
        .trim()
        .to_string();

    if version.is_empty() {
        anyhow::bail!("No version found for package {}", package_name);
    }

    Ok(version)
}

/// Update NPM package globally using `npm install -g`
pub async fn update_npm_package(package_name: &str) -> Result<String> {
    let output = run_npm(&["install", "-g", package_name], NPM_INSTALL_TIMEOUT)
        .await
        .context("Failed to execute npm install command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("npm install failed: {}", stderr);
    }

    // Get the newly installed version
    let new_version = get_npm_latest_version(package_name).await?;

    Ok(new_version)
}
