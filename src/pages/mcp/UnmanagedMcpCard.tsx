import { Button, Card, Space, Tag, Typography, theme } from "antd";
import { InfoCircleOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import type { ScannedMcp } from "@/types/mcp";
import type { TargetKind } from "@/types/domain";

interface UnmanagedMcpCardProps {
  entry: ScannedMcp;
  importing: boolean;
  targetLabel: (target: TargetKind) => string;
  onImport: (entry: ScannedMcp) => void;
}

/** 扫描条目的启动摘要：本地是命令行，远程是地址。 */
function describeScanned(entry: ScannedMcp): string {
  const command = entry.config.command;
  if (typeof command === "string") {
    const args = Array.isArray(entry.config.args)
      ? entry.config.args.filter((item): item is string => typeof item === "string")
      : [];
    return [command, ...args].join(" ");
  }
  const url = entry.config.url;
  return typeof url === "string" ? url : "—";
}

/** ScanTarget 与 TargetKind 取值一致，展示名可复用。 */
function scanTargetLabel(
  entry: ScannedMcp,
  targetLabel: (target: TargetKind) => string,
): string {
  return targetLabel(entry.target as unknown as TargetKind);
}

/**
 * 未纳管卡片（R3）：用户直接在客户端手配、本工具还不认识的「野生」MCP。
 *
 * 虚线边框区分于托管卡片；只列键名（密钥值永不离开后端）；一键纳管收编进统一清单。
 */
export function UnmanagedMcpCard({
  entry,
  importing,
  targetLabel,
  onImport,
}: UnmanagedMcpCardProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const keys = [...entry.envKeys, ...entry.headerKeys];

  return (
    <Card
      size="small"
      data-testid="mcp-unmanaged-card"
      style={{ borderStyle: "dashed" }}
      title={
        <Space size={6} wrap>
          <Typography.Text strong>{entry.name}</Typography.Text>
          <Tag>{entry.kind}</Tag>
          <Tag color="default">
            {t("mcp.unmanagedIn", { target: scanTargetLabel(entry, targetLabel) })}
          </Tag>
        </Space>
      }
      extra={
        <Button
          type="primary"
          size="small"
          loading={importing}
          onClick={() => onImport(entry)}
        >
          {t("mcp.adopt")}
        </Button>
      }
    >
      <div className="flex flex-col gap-1">
        <Typography.Text code style={{ fontSize: 12 }}>
          {describeScanned(entry)}
        </Typography.Text>
        {keys.length > 0 && (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.existingKeys", { keys: keys.join(", ") })}
          </Typography.Text>
        )}
        <Typography.Text
          type="secondary"
          style={{ fontSize: 12, display: "flex", gap: 6, alignItems: "flex-start" }}
        >
          <InfoCircleOutlined style={{ color: token.colorTextTertiary, marginTop: 3 }} />
          {t("mcp.unmanagedHint")}
        </Typography.Text>
      </div>
    </Card>
  );
}
