import { Button, Card, Empty, Space, Switch, Tag, Tooltip, Typography } from "antd";
import { DeleteOutlined, EditOutlined, SyncOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import type { McpUpdateStatus } from "@/stores/mcpUpdateStore";
import type { McpServerSummary } from "@/types/mcp";
import type { TargetKind } from "@/types/domain";
import { MCP_TARGETS } from "./targets";

interface SavedMcpListProps {
  /** 已按工具栏条件过滤好的可见列表。 */
  servers: McpServerSummary[];
  /** 过滤前是否有存量（决定空态文案：真空 vs 条件过严）。 */
  hasAnyServer: boolean;
  loading: boolean;
  updateStatuses: McpUpdateStatus[];
  updating: Record<string, boolean>;
  targetLabel: (target: TargetKind) => string;
  onEdit: (id: string) => void;
  onDelete: (record: McpServerSummary) => void;
  onUpdate: (id: string) => void;
  onToggleEnabled: (record: McpServerSummary, enabled: boolean) => void;
}

/**
 * 已保存 MCP 卡片列表（R1）。
 *
 * 逐卡同时展示：名称 / 类型 / 更新角标 / 目标 chips / 未覆盖缺口 / 启用开关。
 * 启用开关直接落库（走 save + 后端 sweep 清理），禁用即从写盘清单摘除。
 */
export function SavedMcpList({
  servers,
  hasAnyServer,
  loading,
  updateStatuses,
  updating,
  targetLabel,
  onEdit,
  onDelete,
  onUpdate,
  onToggleEnabled,
}: SavedMcpListProps) {
  const { t } = useTranslation();

  if (servers.length === 0) {
    return (
      <Card loading={loading}>
        <Empty description={hasAnyServer ? t("mcp.mineNoResults") : t("mcp.emptyTitle")} />
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {servers.map((server) => {
        const status = updateStatuses.find((item) => item.id === server.id);
        const isUpdating = updating[server.id] || false;
        const uncovered = MCP_TARGETS.filter((target) => !server.targets.includes(target));
        return (
          <Card
            key={server.id}
            size="small"
            data-testid="mcp-card"
            title={
              <Space size={6} wrap>
                <Typography.Text strong>{server.name}</Typography.Text>
                <Tag>{server.kind}</Tag>
                {status?.hasUpdate && (
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
                {status?.hasUpdate && (
                  <Tooltip title={t("mcp.update")}>
                    <Button
                      type="text"
                      size="small"
                      aria-label={t("mcp.update")}
                      icon={<SyncOutlined spin={isUpdating} />}
                      loading={isUpdating}
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
            <div className="flex flex-col gap-1">
              {server.targets.length === 0 ? (
                <Typography.Text type="secondary">—</Typography.Text>
              ) : (
                <Space size={4} wrap>
                  {server.targets.map((target) => (
                    <Tag key={target} color="blue">
                      {targetLabel(target)}
                    </Tag>
                  ))}
                </Space>
              )}
              {uncovered.length > 0 && (
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {t("mcp.notAppliedTo", {
                    targets: uncovered.map((target) => targetLabel(target)).join("、"),
                  })}
                </Typography.Text>
              )}
            </div>
          </Card>
        );
      })}
    </div>
  );
}
