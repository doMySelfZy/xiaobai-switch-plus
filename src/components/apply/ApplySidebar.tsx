import { Badge, Menu, theme } from "antd";
import ClaudeCode from "@lobehub/icons/es/ClaudeCode";
import Codex from "@lobehub/icons/es/Codex";
import Pi from "@lobehub/icons/es/Pi";
import { Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import { StatusDot } from "@/components/StatusDot";
import { useApplyStore } from "@/stores";
import { useAgentUpdateStore } from "@/stores/agentUpdateStore";
import { useUIStore, type ApplyTargetTab } from "@/stores/uiStore";
import { isConfiguredStatus, targetKindLabelKey } from "./TargetStatusCard";

const TAB_KEYS: ApplyTargetTab[] = ["claude_code", "codex", "pi", "prime"];

const MENU_ICONS: Record<ApplyTargetTab, React.ReactNode> = {
  claude_code: <ClaudeCode size={16} />,
  codex: <Codex size={16} />,
  pi: <Pi size={16} />,
  prime: <Sparkles size={16} data-icon="prime" />,
};

export function ApplySidebar() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const applyTab = useUIStore((s) => s.applyTab);
  const setApplyTab = useUIStore((s) => s.setApplyTab);
  const statuses = useApplyStore((s) => s.statuses);
  const updateCount = useAgentUpdateStore((s) => s.updateCount());

  const items = TAB_KEYS.map((key) => {
    const configured = isConfiguredStatus(statuses.find((s) => s.kind === key)?.status);
    const name = t(targetKindLabelKey(key));
    return {
      key,
      icon: MENU_ICONS[key],
      label: (
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <StatusDot
            className="shrink-0"
            active={configured}
            title={configured ? t("apply.status_applied") : t("apply.status_not_applied")}
          />
          <span className="truncate">{name}</span>
        </span>
      ),
    };
  });

  return (
    <div className="flex h-full flex-col" style={{ backgroundColor: token.colorBgContainer, overflowY: "auto" }}>
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
        <div className="flex items-center gap-2">
          <div style={{ fontSize: 14, fontWeight: 600, color: token.colorText }}>{t("apply.title")}</div>
          {updateCount > 0 && (
            <Badge
              count={updateCount}
              style={{ backgroundColor: token.colorWarning }}
              title={t("apply.agentUpdatesAvailable", { count: updateCount })}
            />
          )}
        </div>
        <div style={{ fontSize: 12, color: token.colorTextSecondary, marginTop: 2 }}>
          {t("apply.sidebarHint")}
        </div>
      </div>
      <div className="flex-1 pt-1" style={{ overflowY: "auto" }}>
        <Menu
          mode="inline"
          selectedKeys={[applyTab]}
          items={items}
          style={{ borderInlineEnd: "none" }}
          styles={{ item: { height: 44, lineHeight: "44px" } }}
          onClick={({ key }) => setApplyTab(key as ApplyTargetTab)}
        />
      </div>
    </div>
  );
}
