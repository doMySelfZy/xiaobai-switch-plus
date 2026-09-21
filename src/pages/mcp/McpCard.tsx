import { Button, Card, Space, Switch, Tag, Tooltip, Typography, theme } from "antd";
import { DeleteOutlined, EditOutlined, SyncOutlined, WarningOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import type { McpUpdateStatus } from "@/stores/mcpUpdateStore";
import type { McpServerSummary } from "@/types/mcp";
import type { TargetKind } from "@/types/domain";
import { MCP_TARGETS } from "./targets";

/** 每个客户端在某条 MCP 上的三态：已应用 / 未应用 / 同名冲突。 */
export type ClientState = "on" | "off" | "conflict";

interface McpCardProps {
  server: McpServerSummary;
  /** 每个客户端的当前状态（由页面根据 targets + 扫描冲突算好）。 */
  clientStates: Record<TargetKind, ClientState>;
  updateStatus?: McpUpdateStatus;
  updating: boolean;
  /** 该客户端开关是否正在写盘（防连点）。 */
  busyTarget: TargetKind | null;
  targetLabel: (target: TargetKind) => string;
  onToggleClient: (server: McpServerSummary, target: TargetKind, next: boolean) => void;
  onResolveConflict: (server: McpServerSummary, target: TargetKind) => void;
  onToggleEnabled: (server: McpServerSummary, enabled: boolean) => void;
  onEdit: (id: string) => void;
  onDelete: (server: McpServerSummary) => void;
  onUpdate: (id: string) => void;
}

/** 命令/地址预览：本地 stdio 拼 command+args，远程取 url（summary 无 config，退回类型说明）。 */
function describeSummary(server: McpServerSummary): string {
  // McpServerSummary 不带 config，命令细节在编辑时才拉全量；这里给类型级摘要。
  return server.kind === "stdio" ? "stdio · 本地命令" : server.kind.toUpperCase();
}

/**
 * 统一的 MCP 卡片（单列表方向）。
 *
 * 一张卡同时承载：名称 / 类型 / 总开关 / 编辑删除 / 更新提示 + 四客户端开关行 + 冲突横幅。
 * 客户端开关点击即时生效（改 targets → save），冲突态点击走解决弹窗而非直接切。
 */
export function McpCard({
  server,
  clientStates,
  updateStatus,
  updating,
  busyTarget,
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
  const conflictTarget = MCP_TARGETS.find((target) => clientStates[target] === "conflict");

  return (
    <Card
      size="small"
      data-testid="mcp-card"
      className={server.enabled ? undefined : "opacity-60"}
      title={
        <Space size={6} wrap>
          <Typography.Text strong>{server.name}</Typography.Text>
          <Tag>{server.kind}</Tag>
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

        {/* 四客户端开关：点亮=已应用到该客户端，冲突态标橙、点击走解决弹窗。 */}
        <div className="flex flex-wrap gap-2">
          {MCP_TARGETS.map((target) => {
            const state = clientStates[target];
            const label = targetLabel(target);
            const isConflict = state === "conflict";
            return (
              <div
                key={target}
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
                  minWidth: 116,
                }}
              >
                <Typography.Text style={{ fontSize: 13, flex: 1 }}>{label}</Typography.Text>
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
                    loading={busyTarget === target}
                    aria-label={t("mcp.clientToggle", { name: server.name, target: label })}
                    onChange={(checked) => onToggleClient(server, target, checked)}
                  />
                )}
              </div>
            );
          })}
        </div>

        {conflictTarget && (
          <div
            className="flex items-start gap-2 rounded-lg px-3 py-2"
            style={{
              background: token.colorWarningBg,
              border: `1px solid ${token.colorWarningBorder}`,
            }}
          >
            <WarningOutlined style={{ color: token.colorWarning, marginTop: 2 }} />
            <Typography.Text style={{ fontSize: 12, color: token.colorWarningText, flex: 1 }}>
              {t("mcp.conflictBanner", { target: targetLabel(conflictTarget), name: server.name })}
            </Typography.Text>
            <Button size="small" onClick={() => onResolveConflict(server, conflictTarget)}>
              {t("mcp.conflictCompare")}
            </Button>
          </div>
        )}
      </div>
    </Card>
  );
}
