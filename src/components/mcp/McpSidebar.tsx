import { Badge, Menu, theme, Typography } from "antd";
import ClaudeCode from "@lobehub/icons/es/ClaudeCode";
import Codex from "@lobehub/icons/es/Codex";
import Pi from "@lobehub/icons/es/Pi";
import { useTranslation } from "react-i18next";
import { useMcpStore } from "@/stores";
import { useUIStore, type McpAgentTab } from "@/stores/uiStore";
import { MCP_TARGET_LABEL_KEYS } from "@/pages/mcp/targets";

/** MCP per-agent 侧栏目标（不含 Prime）。 */
export const MCP_AGENT_TABS: McpAgentTab[] = ["claude_code", "codex", "pi"];

const MENU_ICONS: Record<McpAgentTab, React.ReactNode> = {
  claude_code: <ClaudeCode size={16} />,
  codex: <Codex size={16} />,
  pi: <Pi size={16} />,
};

export function McpSidebar() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const mcpTab = useUIStore((s) => s.mcpTab);
  const setMcpTab = useUIStore((s) => s.setMcpTab);
  const servers = useMcpStore((s) => s.servers);
  const drift = useMcpStore((s) => s.drift);

  const items = MCP_AGENT_TABS.map((key) => {
    const enabledCount = servers.filter((s) => s.enabled && s.targets.includes(key)).length;
    const targetDrift = drift.find((d) => d.target === key);
    const hasDrift = targetDrift?.drift ?? false;
    const name = t(MCP_TARGET_LABEL_KEYS[key]);
    return {
      key,
      icon: MENU_ICONS[key],
      label: (
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <Badge
            dot={hasDrift}
            color={token.colorWarning}
            className="shrink-0"
            title={hasDrift ? t("mcp.driftSidebarHint") : undefined}
          />
          <span className="truncate">{name}</span>
          {enabledCount > 0 && (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {enabledCount}
            </Typography.Text>
          )}
        </span>
      ),
    };
  });

  return (
    <div
      className="flex h-full flex-col"
      style={{ backgroundColor: token.colorBgContainer, overflowY: "auto" }}
    >
      <div
        className="shrink-0"
        style={{
          borderBottom: `1px solid ${token.colorBorderSecondary}`,
          paddingLeft: 20,
          paddingRight: 16,
          paddingTop: 14,
          paddingBottom: 14,
        }}
      >
        <div style={{ fontSize: 14, fontWeight: 600, color: token.colorText }}>
          {t("mcp.title")}
        </div>
        <div style={{ fontSize: 12, color: token.colorTextSecondary, marginTop: 2 }}>
          {t("mcp.sidebarHint")}
        </div>
      </div>
      <div className="flex-1 pt-1" style={{ overflowY: "auto" }}>
        <Menu
          mode="inline"
          selectedKeys={[mcpTab]}
          items={items}
          style={{ borderInlineEnd: "none" }}
          styles={{ item: { height: 44, lineHeight: "44px" } }}
          onClick={({ key }) => setMcpTab(key as McpAgentTab)}
        />
      </div>
    </div>
  );
}
