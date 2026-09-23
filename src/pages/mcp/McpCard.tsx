import { Button, Card, Space, Switch, Tag, Tooltip, Typography, theme } from "antd";
import { DeleteOutlined, EditOutlined, SyncOutlined, WarningOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import type { McpUpdateStatus } from "@/stores/mcpUpdateStore";
import type { McpServerSummary } from "@/types/mcp";
import type { TargetKind } from "@/types/domain";

/** 每个客户端在某条 MCP 上的三态：已应用 / 未应用 / 同名冲突。 */
export type ClientState = "on" | "off" | "conflict";

interface McpCardProps {
  server: McpServerSummary;
  /** 本卡片所属的 agent（per-agent 视图，一次只展示一个目标）。 */
  target: TargetKind;
  /** 该服务器在本目标上的状态。 */
  state: ClientState;
  updateStatus?: McpUpdateStatus;
  updating: boolean;
  /** 本目标开关是否正在写盘（防连点）。 */
  busy: boolean;
  targetLabel: (target: TargetKind) => string;
  onToggleClient: (server: McpServerSummary, target: TargetKind, next: boolean) => void;
  onResolveConflict: (server: McpServerSummary, target: TargetKind) => void;
  onToggleEnabled: (server: McpServerSummary, enabled: boolean) => void;
  onEdit: (id: string) => void;
  onDelete: (server: McpServerSummary) => void;
  onUpdate: (id: string) => void;
}

/** 命令/地址预览：summary 无 config，退回类型说明。 */
function describeSummary(server: McpServerSummary): string {
  return server.kind === "stdio" ? "stdio · 本地命令" : server.kind.toUpperCase();
}

/**
 * 单 agent 的 MCP 卡片。
 *
 * 一张卡承载：名称 / 类型 / 绝对路径预警 / 主启用开关 / 编辑删除 / 更新提示
 * + 「应用到本 agent」的单个开关 + 冲突横幅。
 */
export function McpCard({
  server,
  target,
  state,
  updateStatus,
  updating,
  busy,
  targetLabel,
  onToggleClient,
  onResolveConflict,
  onToggleEnabled,
  onEdit,
  onDelete,
  onUpdate,
}: McpCardProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const label = targetLabel(target);
  const isConflict = state === "conflict";

  return (
    <Card
      size="small"
      data-testid="mcp-card"
      className={server.enabled ? undefined : "opacity-60"}
      title={
        <Space size={6} wrap>
          <Typography.Text strong>{server.name}</Typography.Text>
          <Tag>{server.kind}</Tag>
          {server.absoluteCommand && (
            <Tooltip title={t("mcp.absolutePathHint")}>
              <Tag color="warning" icon={<WarningOutlined />} style={{ fontSize: 11 }}>
                {t("mcp.absolutePathTag")}
              </Tag>
            </Tooltip>
          )}
          {updateStatus?.hasUpdate && (
            <Tag color="orange" style={{ fontSize: 11 }}>
              {t("mcp.hasUpdate")}
            </Tag>
          )}
        </Space>
      }
      extra={
        <Space size={0} align="center">
          <Tooltip title={t("mcp.enabledToggle", { name: server.name })}>
            <Switch
              size="small"
              checked={server.enabled}
              aria-label={t("mcp.enabledToggle", { name: server.name })}
              onChange={(checked) => onToggleEnabled(server, checked)}
            />
          </Tooltip>
          <Typography.Text type="secondary" style={{ fontSize: 12, marginLeft: 6 }}>
            {server.enabled ? t("mcp.enabled") : t("mcp.disabled")}
          </Typography.Text>
          {updateStatus?.hasUpdate && (
            <Tooltip title={t("mcp.update")}>
              <Button
                type="text"
                size="small"
                aria-label={t("mcp.update")}
                icon={<SyncOutlined spin={updating} />}
                loading={updating}
                onClick={() => onUpdate(server.id)}
              />
            </Tooltip>
          )}
          <Tooltip title={t("common.edit")}>
            <Button
              type="text"
              size="small"
              aria-label={t("common.edit")}
              icon={<EditOutlined />}
              onClick={() => onEdit(server.id)}
            />
          </Tooltip>
          <Tooltip title={t("common.delete")}>
            <Button
              type="text"
              size="small"
              danger
              aria-label={t("common.delete")}
              icon={<DeleteOutlined />}
              onClick={() => onDelete(server)}
            />
          </Tooltip>
        </Space>
      }
    >
      <div className="flex flex-col gap-2">
        <Typography.Text type="secondary" style={{ fontSize: 12 }} code>
          {describeSummary(server)}
        </Typography.Text>

        {/* 单目标开关：点亮=已应用到本 agent，冲突态标橙、点击走解决弹窗。 */}
        <div
          className="flex items-center gap-2 rounded-lg border px-2.5 py-1.5"
          style={{
            borderColor: isConflict
              ? token.colorWarningBorder
              : state === "on"
                ? token.colorPrimaryBorder
                : token.colorBorderSecondary,
            background: isConflict
              ? token.colorWarningBg
              : state === "on"
                ? token.colorPrimaryBg
                : undefined,
            alignSelf: "flex-start",
            minWidth: 160,
          }}
        >
          <Typography.Text style={{ fontSize: 13, flex: 1 }}>
            {t("mcp.applyTo", { target: label })}
          </Typography.Text>
          {isConflict ? (
            <Tooltip title={t("mcp.conflictResolve")}>
              <Button
                type="text"
                size="small"
                aria-label={t("mcp.conflictOn", { target: label })}
                icon={<WarningOutlined style={{ color: token.colorWarning }} />}
                onClick={() => onResolveConflict(server, target)}
              />
            </Tooltip>
          ) : (
            <Switch
              size="small"
              checked={state === "on"}
              loading={busy}
              aria-label={t("mcp.clientToggle", { name: server.name, target: label })}
              onChange={(checked) => onToggleClient(server, target, checked)}
            />
          )}
        </div>

        {isConflict && (
          <div
            className="flex items-start gap-2 rounded-lg px-3 py-2"
            style={{
              background: token.colorWarningBg,
              border: `1px solid ${token.colorWarningBorder}`,
            }}
          >
            <WarningOutlined style={{ color: token.colorWarning, marginTop: 2 }} />
            <Typography.Text style={{ fontSize: 12, color: token.colorWarningText, flex: 1 }}>
              {t("mcp.conflictBanner", { target: label, name: server.name })}
            </Typography.Text>
            <Button size="small" onClick={() => onResolveConflict(server, target)}>
              {t("mcp.conflictCompare")}
            </Button>
          </div>
        )}
      </div>
    </Card>
  );
}
