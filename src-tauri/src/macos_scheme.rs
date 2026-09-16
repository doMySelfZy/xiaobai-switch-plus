use crate::error::{AppError, AppResult};
use crate::paths::{app_dir, ensure_app_dirs, home_dir};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const MAX_PENDING_LEN: usize = 16 * 1024;

pub fn pending_deep_link_path() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("pending-deeplink.url"))
}

/// 当前深链 scheme；同时接受改名前后用过的 legacy scheme，避免旧链接失效。
pub fn accepted_deep_link_schemes() -> [&'static str; 3] {
    ["xiaobaiswitchplus", "anyswitch", "xiaobaiswitch"]
}

pub fn parse_pending_deep_link_file(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > MAX_PENDING_LEN {
        return None;
    }
    let has_accepted_scheme = accepted_deep_link_schemes()
        .iter()
        .any(|scheme| url.starts_with(&format!("{scheme}:")));
    if !has_accepted_scheme {
        return None;
    }
    Some(url.to_string())
}

pub fn take_pending_deep_link() -> AppResult<Option<String>> {
    let path = pending_deep_link_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let _ = fs::remove_file(&path);
    Ok(parse_pending_deep_link_file(&raw))
}

/// 前端是否需要定时轮询待处理深链。
///
/// 写 `pending-deeplink.url` 的只有 macOS 分支（`install_dev_url_handler` 注册的
/// AppleScript 处理器：`tauri dev` 下 Launch Services 看不到 CFBundleURLTypes）。
/// Windows / Linux 的运行时深链由 deep-link 插件的 `onOpenUrl` 事件送达，启动时还有
/// `getCurrent()`，轮询一个永远不会被写入的文件纯属浪费（每 400ms 一次 IPC）。
pub fn requires_polling() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(target_os = "macos")]
pub fn install_dev_url_handler() -> AppResult<PathBuf> {
    ensure_app_dirs()?;
    let apps = home_dir()?.join("Applications");
    fs::create_dir_all(&apps)?;
    let dest = apps.join("XiaoBaiSwitch Plus Dev.app");

    let script = r#"on open location theURL
	set dest to (POSIX path of (path to home folder)) & ".xiaobai-switch/pending-deeplink.url"
	do shell script "mkdir -p \"$HOME/.xiaobai-switch\" && umask 077 && printf '%s' " & quoted form of theURL & " > " & quoted form of dest & ".tmp && mv " & quoted form of dest & ".tmp " & quoted form of dest
end open location
"#;

    let tmp = std::env::temp_dir().join("XiaoBaiSwitch-Plus-Dev-url-handler.applescript");
    fs::write(&tmp, script)?;

    if dest.exists() {
        let _ = fs::remove_dir_all(&dest);
    }

    let status = Command::new("osacompile")
        .arg("-o")
        .arg(&dest)
        .arg(&tmp)
        .status()
        .map_err(|e| AppError::new("internal", format!("osacompile failed: {e}")))?;
    if !status.success() {
        return Err(AppError::new(
            "internal",
            "osacompile failed to build URL handler app",
        ));
    }

    let info = dest.join("Contents/Info.plist");
    patch_handler_info_plist(&info)?;

    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-f").arg(&dest).status();

    tracing::info!("registered xiaobaiswitchplus:// via {}", dest.display());
    Ok(dest)
}

#[cfg(target_os = "macos")]
fn patch_handler_info_plist(info: &PathBuf) -> AppResult<()> {
    let buddy = "/usr/libexec/PlistBuddy";
    let run = |cmd: &str, required: bool| -> AppResult<()> {
        let status = Command::new(buddy)
            .args(["-c", cmd])
            .arg(info)
            .status()
            .map_err(|e| AppError::new("internal", format!("PlistBuddy failed: {e}")))?;
        if required && !status.success() {
            return Err(AppError::new(
                "internal",
                format!("PlistBuddy command failed: {cmd}"),
            ));
        }
        Ok(())
    };

    let _ = run("Delete :CFBundleURLTypes", false);
    run("Add :CFBundleURLTypes array", true)?;
    run("Add :CFBundleURLTypes:0 dict", true)?;
    run(
        "Add :CFBundleURLTypes:0:CFBundleTypeRole string Editor",
        true,
    )?;
    run(
        "Add :CFBundleURLTypes:0:CFBundleURLName string com.github.licoy.xiaobai-switch.plus.url-handler",
        true,
    )?;
    run("Add :CFBundleURLTypes:0:CFBundleURLSchemes array", true)?;
    // 旧 scheme 也要注册：已经分享出去的 anyswitch:// / xiaobaiswitch:// 链接
    // 必须在系统层就能拉起本应用，否则「兼容旧链接」只在解析层成立。
    for (index, scheme) in accepted_deep_link_schemes().iter().enumerate() {
        run(
            &format!("Add :CFBundleURLTypes:0:CFBundleURLSchemes:{index} string {scheme}"),
            true,
        )?;
    }
    let _ = run(
        "Set :CFBundleIdentifier com.github.licoy.xiaobai-switch.plus.url-handler",
        false,
    );
    let _ = run(
        "Add :CFBundleIdentifier string com.github.licoy.xiaobai-switch.plus.url-handler",
        false,
    );
    let _ = run("Set :CFBundleName XiaoBaiSwitch Plus Dev", false);
    let _ = run("Add :CFBundleName string XiaoBaiSwitch Plus Dev", false);
    let _ = run("Set :CFBundleDisplayName XiaoBaiSwitch Plus Dev", false);
    let _ = run("Add :CFBundleDisplayName string XiaoBaiSwitch Plus Dev", false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pending_accepts_scheme_only() {
        assert_eq!(
            parse_pending_deep_link_file(
                "  xiaobaiswitchplus://sites?name=A&baseurls=https://a.example.com  \n"
            )
            .as_deref(),
            Some("xiaobaiswitchplus://sites?name=A&baseurls=https://a.example.com")
        );
    }

    #[test]
    fn parse_pending_accepts_legacy_schemes() {
        assert_eq!(
            parse_pending_deep_link_file("anyswitch://sites?name=A").as_deref(),
            Some("anyswitch://sites?name=A")
        );
        assert_eq!(
            parse_pending_deep_link_file("xiaobaiswitch://sites?name=A").as_deref(),
            Some("xiaobaiswitch://sites?name=A")
        );
    }

    #[test]
    fn parse_pending_rejects_other_schemes() {
        assert_eq!(parse_pending_deep_link_file("https://example.com"), None);
        assert_eq!(parse_pending_deep_link_file(""), None);
        assert_eq!(parse_pending_deep_link_file("aqbot://providers"), None);
    }

    #[test]
    fn polling_is_only_required_where_the_pending_file_gets_written() {
        // 写这个文件的路径只有 macOS 的 URL handler。谁把这里改成「所有平台都要轮询」，
        // 就等于把 Windows 上每 400ms 一次的空 IPC 加回来。
        assert_eq!(requires_polling(), cfg!(target_os = "macos"));
    }
}
