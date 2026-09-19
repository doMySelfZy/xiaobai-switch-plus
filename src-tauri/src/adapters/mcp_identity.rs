//! MCP 条目的**身份口径**：判断「这两条配置是不是同一个服务器」的唯一算法。
//!
//! 背景（`.trellis/tasks/09-19-mcp-identity-dedup/research/diagnosis.md`）：名字是每个客户端
//! 各自的局部属性（Claude 侧手工条目叫 `sequentialthinking`，Codex 侧叫 `sequential-thinking`），
//! 只按名字判重会让同一个服务器被纳管成多行，最终以两条 `xiaobai_*` 写进同一个客户端。
//! 身份一律**读取时计算，不落库**：`mcp_servers` 在 `sync.rs` 的 `FINGERPRINT_TABLES` 里且
//! 指纹走 `SELECT *`，新增列等于改指纹算法。
//!
//! 两层身份：
//! - [`coarse_identity`]：「是不是同一个服务器」。剔除客户端私有字段（[`CLIENT_PRIVATE_FIELDS`]）、
//!   归一化 `command`（[`normalize_command`]），其余（`args`、含 query 的 `url`）走
//!   `mcp_scan::canonical` 稳定序列化。`url` 的 query 保留：`exa` 的 `?tools=...` 是功能选择，
//!   不是噪声。
//! - [`equivalence`]：「能不能自动合并」= 粗身份 + env/headers 的**值**。env 值不同但粗身份相同
//!   的一对（本机 `ssh` / `ssh-mcp-server` 就是这个形状）必须能在界面上被区分出来，绝不允许
//!   静默合并。
//!
//! 与接管比对的分工（**不要合并成一套口径**）：`adapters::mcp::untracked_matches_record`
//! 判断「能不能删掉用户客户端里那条手工条目」，后果是丢数据，所以它只忽略 `type`/`transport`，
//! 严格比对 `enabled`/`timeoutMs` 与 command 写法。本模块判断「是不是同一个服务器」，
//! 后果只是不新建重复行，放宽不丢数据，才需要吞掉这些客户端私有字段。
//!
//! 安全红线：两个函数都只回 sha256 hex，**任何 env/headers 明文都不可能出现在返回值里**；
//! 计算过程中也不打日志。

use crate::adapters::mcp_scan::canonical;
use crate::domain::McpKind;
use serde_json::Value;

/// 客户端私有字段：比对时剔除，存储原样保留（绝不改写用户已入库的 `config`）。
///
/// - `type` / `transport`：Claude 与部分客户端的显式传输标记，语义已由 `kind` 承载；
/// - `enabled` / `timeoutMs`：Claude Code 自己写回条目时附加的界面/启动字段，
///   Codex 完全不认。纳管时随 `config` 原样透传进库，比对时必须忽略，否则
///   「Claude 形状」与「Codex 形状」的同一条服务器会被判成两个身份。
pub const CLIENT_PRIVATE_FIELDS: [&str; 4] = ["type", "transport", "enabled", "timeoutMs"];

/// 归一化启动命令：取 basename（同时认 `\` 与 `/`）、剥掉一个尾随的
/// `.cmd`/`.exe`/`.bat`/`.com`、转小写。
///
/// `npx`、`npx.cmd`、`Npx.CMD`、`D:\Program Files\nodejs\npx.cmd` 必须收敛到同一个值：
/// 同一台机器上 Node 全局包在 PATH 里叫 `npx`，在 Windows 的 npm 垫片目录里叫 `npx.cmd`，
/// 那是实现细节，不是身份差异。剥后缀只剥一层，避免 `foo.cmd.exe` 被连环剥成 `foo`。
pub fn normalize_command(command: &str) -> String {
    let base = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_lowercase();
    for ext in ["cmd", "exe", "bat", "com"] {
        if let Some(head) = base.strip_suffix(&format!(".{ext}")) {
            if !head.is_empty() {
                return head.to_string();
            }
            break;
        }
    }
    base
}

/// 比对用的 config 形态：剔除私有字段 + 归一化 command，其余原样。
fn comparable_config(config: &Value) -> Value {
    let mut map = config.as_object().cloned().unwrap_or_default();
    for field in CLIENT_PRIVATE_FIELDS {
        map.remove(field);
    }
    if let Some(Value::String(command)) = map.get("command") {
        let normalized = normalize_command(command);
        map.insert("command".to_string(), Value::String(normalized));
    }
    Value::Object(map)
}

/// 粗身份：同一服务器在不同客户端里的不同键名/不同形状要收敛到同一个值。
///
/// 输入 = `kind` + [`comparable_config`] 的 `canonical` 稳定序列化（键序无关）。
pub fn coarse_identity(kind: McpKind, config: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{kind:?}").as_bytes());
    hasher.update(canonical(&comparable_config(config)).as_bytes());
    hex::encode(hasher.finalize())
}

/// 等价：粗身份 + env/headers 的**值**。用于「能不能自动合并」的判定。
///
/// M1 只有测试用到它：合并动作属 M2（`detect_mcp_duplicates` 用它区分「等价组可一键合并」与
/// 「非等价组必须用户点选」）。与 [`coarse_identity`] 同批落地，避免两套算法分头演化。
#[allow(dead_code, reason = "M2 的合并判定使用；身份原语与消费方同任务落地")]
pub fn equivalence(kind: McpKind, config: &Value, env: &Value, headers: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(coarse_identity(kind, config).as_bytes());
    hasher.update([0]);
    hasher.update(canonical(env).as_bytes());
    hasher.update([0]);
    hasher.update(canonical(headers).as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SEQUENTIAL_ARGS: [&str; 2] = ["-y", "@modelcontextprotocol/server-sequential-thinking"];

    fn codex_shaped() -> Value {
        json!({ "command": "npx", "args": SEQUENTIAL_ARGS.to_vec() })
    }

    fn claude_shaped() -> Value {
        json!({
            "command": "npx",
            "args": SEQUENTIAL_ARGS.to_vec(),
            "type": "stdio",
            "enabled": true,
            "timeoutMs": 30_000,
        })
    }

    #[test]
    fn normalize_command_collapses_paths_and_extensions() {
        assert_eq!(normalize_command("npx"), "npx");
        assert_eq!(normalize_command("npx.cmd"), "npx");
        assert_eq!(normalize_command("Npx.CMD"), "npx");
        assert_eq!(normalize_command(r"D:\Program Files\nodejs\npx.cmd"), "npx");
        assert_eq!(normalize_command("D:/Program Files/nodejs/npx.exe"), "npx");
        assert_eq!(normalize_command("npx.bat"), "npx");
        assert_eq!(normalize_command("uvx"), "uvx");
        assert_eq!(normalize_command("node.exe"), "node");
        // 退化的整名（就叫 ".cmd"）不能被剥成空串。
        assert_eq!(normalize_command(".cmd"), ".cmd");
        assert_eq!(normalize_command(""), "");
        // 只剥一层：`.cmd.exe` 结尾只吃掉 `.exe`。
        assert_eq!(normalize_command("npx.cmd.exe"), "npx.cmd");
    }

    #[test]
    fn coarse_identity_collapses_absolute_and_bare_command() {
        // 本机实测形状：sequential-thinking(command=npx) vs sequentialthinking(command=D:\...\npx.cmd)
        let bare = json!({ "command": "npx", "args": SEQUENTIAL_ARGS.to_vec() });
        let absolute = json!({
            "command": r"D:\Program Files\nodejs\npx.cmd",
            "args": SEQUENTIAL_ARGS.to_vec(),
        });
        assert_eq!(
            coarse_identity(McpKind::Stdio, &bare),
            coarse_identity(McpKind::Stdio, &absolute),
            "同一服务器的两种 command 写法必须收敛到同一个粗身份"
        );
    }

    #[test]
    fn claude_shaped_and_codex_shaped_are_equivalent() {
        // 客户端私有字段只在比对时剔除：Claude 形状与 Codex 形状必须粗身份相同且等价成立。
        assert_eq!(
            coarse_identity(McpKind::Stdio, &codex_shaped()),
            coarse_identity(McpKind::Stdio, &claude_shaped())
        );
        assert_eq!(
            equivalence(McpKind::Stdio, &codex_shaped(), &json!({}), &json!({})),
            equivalence(McpKind::Stdio, &claude_shaped(), &json!({}), &json!({}))
        );
        // transport 与 type 是同类标记，也要被剔除。
        let with_transport = json!({
            "command": "npx",
            "args": SEQUENTIAL_ARGS.to_vec(),
            "transport": "streamable_http",
        });
        assert_eq!(
            coarse_identity(McpKind::Stdio, &codex_shaped()),
            coarse_identity(McpKind::Stdio, &with_transport)
        );
    }

    #[test]
    fn coarse_identity_is_sensitive_to_args_and_url_query() {
        let base = coarse_identity(McpKind::Stdio, &codex_shaped());
        let other_args = json!({ "command": "npx", "args": ["-y", "different-mcp"] });
        assert_ne!(base, coarse_identity(McpKind::Stdio, &other_args));

        // url 的 query 是功能选择（exa 的 ?tools=...），不同 query 就是不同身份。
        let url_a = json!({ "url": "https://mcp.exa.ai/mcp?tools=web_search_exa" });
        let url_b = json!({ "url": "https://mcp.exa.ai/mcp" });
        assert_ne!(
            coarse_identity(McpKind::Http, &url_a),
            coarse_identity(McpKind::Http, &url_b)
        );

        // kind 也参与身份：同 config 换传输类型不是同一个服务器。
        assert_ne!(base, coarse_identity(McpKind::Http, &codex_shaped()));
    }

    #[test]
    fn env_values_split_equivalence_but_not_coarse_identity() {
        // 本机 ssh / ssh-mcp-server 那对的形状：配置主体相同、env 值不同 →
        // 粗身份相同（同一个服务器）但等价不成立（不能自动合并）。
        let config = json!({
            "command": "npx",
            "args": ["-y", "@fangjunjie/ssh-mcp-server", "--host", "nas.example.com"],
        });
        let env_a = json!({ "SSH_PASSWORD": "PLACEHOLDER_A" });
        let env_b = json!({ "SSH_PASSWORD": "PLACEHOLDER_B" });

        assert_eq!(
            coarse_identity(McpKind::Stdio, &config),
            coarse_identity(McpKind::Stdio, &config)
        );
        assert_ne!(
            equivalence(McpKind::Stdio, &config, &env_a, &json!({})),
            equivalence(McpKind::Stdio, &config, &env_b, &json!({})),
            "env 值不同必须让 equivalence 分开"
        );
        // headers 值同理。
        assert_ne!(
            equivalence(McpKind::Stdio, &config, &env_a, &json!({"X-Token": "1"})),
            equivalence(McpKind::Stdio, &config, &env_a, &json!({"X-Token": "2"}))
        );
    }

    #[test]
    fn identity_never_leaks_secret_values() {
        // 与 mcp_scan 的 scan_never_returns_secret_values 同一红线：任何明文都不许出现在
        // 会跨边界（前端 / 日志 / 错误信息）的串里。
        let secret = "PLACEHOLDER_SUPER_SECRET";
        let config = json!({ "command": "npx", "args": [secret] });
        let coarse = coarse_identity(McpKind::Stdio, &config);
        let equivalent = equivalence(
            McpKind::Stdio,
            &codex_shaped(),
            &json!({ "TOKEN": secret }),
            &json!({ "Authorization": format!("Bearer {secret}") }),
        );
        // args 参与粗身份（这是识别条目所必需的），但只以哈希形式出现。
        assert!(!coarse.contains(secret), "coarse identity leaked plaintext: {coarse}");
        assert!(!equivalent.contains(secret), "equivalence leaked plaintext: {equivalent}");
        assert_eq!(coarse.len(), 64, "sha256 hex");
        assert_eq!(equivalent.len(), 64, "sha256 hex");
        assert!(coarse.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn key_order_never_changes_identity() {
        let env_a = json!({ "A": "1", "B": "2" });
        let env_b = json!({ "B": "2", "A": "1" });
        let headers_a = json!({ "X-A": "1", "X-B": "2" });
        let headers_b = json!({ "X-B": "2", "X-A": "1" });
        assert_eq!(
            equivalence(McpKind::Stdio, &codex_shaped(), &env_a, &headers_a),
            equivalence(McpKind::Stdio, &codex_shaped(), &env_b, &headers_b)
        );

        // config 的键序同样无关（canonical 递归排序）。
        let config_a = json!({ "command": "npx", "args": ["-y", "x"], "cwd": "/tmp" });
        let config_b = json!({ "cwd": "/tmp", "args": ["-y", "x"], "command": "npx" });
        assert_eq!(
            coarse_identity(McpKind::Stdio, &config_a),
            coarse_identity(McpKind::Stdio, &config_b)
        );
    }

    #[test]
    fn private_fields_list_is_pinned() {
        // 常量清单是跨机协议级知识（纳管/接管/合并共用一套剔除口径），钉死防手滑。
        assert_eq!(
            CLIENT_PRIVATE_FIELDS,
            ["type", "transport", "enabled", "timeoutMs"]
        );
    }
}
