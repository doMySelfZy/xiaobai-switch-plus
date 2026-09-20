import { useState } from "react";
import { Button, Card, Checkbox, Typography } from "antd";
import { CloudUploadOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import type { McpApplyResult } from "@/types/mcp";
import type { TargetKind } from "@/types/domain";
import { MCP_TARGETS } from "./targets";

interface ApplyPanelProps {
  /** 启用服务的数量（写盘对象）。 */
  enabledCount: number;
  /** 当前启用服务指向的目标：面板勾选的默认值。 */
  activeTargets: TargetKind[];
  /** 四目标原生写盘位置（后端 mcp_target_paths）。 */
  targetPaths: [TargetKind, string][];
  targetLabel: (target: TargetKind) => string;
  applying: boolean;
  /** 最近一次应用结果：逐目标 inline 展示（modal 结果保留不变）。 */
  lastResult: McpApplyResult | null;
  onApply: (targets: TargetKind[]) => void;
}

/**
 * 应用目标独立面板（R2，常驻页内）。
 *
 * 勾选即预览「写入 / 清理」范围；改名、禁用、删除后的清理只动 `xiaobai_` 前缀，
 * 用户自建条目保留——该语义由后端保证，这里只负责说清楚。
 */
export function ApplyPanel({
  enabledCount,
  activeTargets,
  targetPaths,
  targetLabel,
  applying,
  lastResult,
  onApply,
}: ApplyPanelProps) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<TargetKind[]>(activeTargets);
  // 启用服务集合变化（加载完成 / 增删改后回读）时，把勾选同步回真实指向。
  const activeKey = activeTargets.join(",");
  const [syncedKey, setSyncedKey] = useState(activeKey);
  if (syncedKey !== activeKey) {
    setSyncedKey(activeKey);
    setSelected(activeTargets);
  }

  const ordered = MCP_TARGETS.filter((target) => selected.includes(target));
  const pathOf = (target: TargetKind) =>
    targetPaths.find(([item]) => item === target)?.[1] ?? "";

  const toggle = (target: TargetKind, checked: boolean) => {
    setSelected((current) =>
      checked ? [...current, target] : current.filter((item) => item !== target),
    );
  };

  return (
    <Card
      size="small"
      title={t("mcp.applyToTargets")}
      extra={
        <Button
          type="primary"
          size="small"
          icon={<CloudUploadOutlined />}
          loading={applying}
          disabled={ordered.length === 0}
          onClick={() => onApply(ordered)}
        >
          {t("mcp.applyToTargets")}
        </Button>
      }
    >
      <div className="flex flex-col gap-2">
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          {t("mcp.applyPanelDesc")}
        </Typography.Text>
        {MCP_TARGETS.map((target) => (
          <div key={target} className="flex items-start gap-2">
            <Checkbox
              checked={selected.includes(target)}
              onChange={(event) => toggle(target, event.target.checked)}
            >
              <span>{targetLabel(target)}</span>
            </Checkbox>
            {pathOf(target) && (
              <Typography.Text type="secondary" style={{ fontSize: 12 }} code>
                {pathOf(target)}
              </Typography.Text>
            )}
          </div>
        ))}
        {enabledCount === 0 ? (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.applyPreviewEmpty")}
          </Typography.Text>
        ) : (
          <div className="flex flex-col gap-1">
            <Typography.Text style={{ fontSize: 12 }}>
              {t("mcp.applyPreview", {
                count: enabledCount,
                targets:
                  ordered.length > 0
                    ? ordered.map((target) => targetLabel(target)).join("、")
                    : "—",
              })}
            </Typography.Text>
            {ordered.length < MCP_TARGETS.length && (
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {t("mcp.applyCleanupNote")}
              </Typography.Text>
            )}
          </div>
        )}
        {(lastResult?.results.length ?? 0) > 0 && (
          <div className="flex flex-col gap-1">
            {(lastResult?.results ?? []).map((item) => (
              <Typography.Text
                key={item.target}
                type={item.ok ? "success" : "danger"}
                style={{ fontSize: 12 }}
              >
                {item.ok
                  ? t("mcp.targetSuccess", { target: targetLabel(item.target) })
                  : t("mcp.targetFailed", {
                      target: targetLabel(item.target),
                      message: item.message,
                    })}
              </Typography.Text>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}
