import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Button, Dropdown, Switch, Tooltip, theme } from "antd";
import type { MenuProps } from "antd";
import { Ellipsis, GripVertical, Pencil, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ReactNode } from "react";
import type { Site } from "@/types/domain";
import { StatusDot } from "@/components/StatusDot";
import { SiteAvatar } from "@/components/sites/SiteAvatar";
import { useSiteStore } from "@/stores";
import {
  formatQuotaAmountLocalized,
  formatQuotaUpdatedText,
  isBalanceQuotaSummary,
  quotaRemainingPercent,
  quotaRemainingTone,
  quotaWindowLabelKey,
  quotaWindowShortLabelKey,
  windowRemainingPercent,
} from "@/lib/quotaProbe";

interface Props {
  site: Site;
  active: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onDelete: () => void;
  /** 直接在列表里启用/禁用，不必先点进站点详情。 */
  onToggleEnabled: (enabled: boolean) => void;
}

export function SiteListItem({
  site,
  active,
  onSelect,
  onEdit,
  onDelete,
  onToggleEnabled,
}: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  // 摘要只基于最近一次成功探测（quotaBySite）；失败/加载态交给右侧详情展示。
  const quota = useSiteStore((s) => s.quotaBySite[site.id]);
  const modelsBySite = useSiteStore((s) => s.modelsBySite);
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: site.id,
  });

  // 判断站点可用性：已启用 + 有模型 = 可用
  const hasModels = modelsBySite[site.id]?.length > 0;
  const siteStatus = !site.enabled 
    ? "disabled" 
    : hasModels 
      ? "available" 
      : "unavailable";
  
  const statusTitle = !site.enabled
    ? t("sites.disabled")
    : hasModels
      ? t("sites.available")
      : t("sites.unavailable");

  const menu: MenuProps = {
    items: [
      {
        key: "edit",
        icon: <Pencil size={14} />,
        label: t("sites.edit"),
      },
      {
        key: "delete",
        icon: <Trash2 size={14} />,
        label: t("sites.delete"),
        danger: true,
      },
    ],
    onClick: ({ key, domEvent }) => {
      domEvent.stopPropagation();
      onSelect();
      if (key === "edit") onEdit();
      else if (key === "delete") onDelete();
    },
  };

  // 列表额度摘要：余额型显示剩余金额，窗口型显示每个窗口的剩余百分比
  // （5 小时 / 周 / 月全列，窄容器用短标签）。两类都是「剩余」口径，颜色阈值
  // 走同一个 quotaRemainingTone。不支持的站点安静不显示，避免一列表灰字。
  let quotaSummary: ReactNode = null;
  if (quota?.status === "available") {
    const windows = quota.windows ?? [];
    if (windows.length > 0) {
      const updatedText = formatQuotaUpdatedText(quota.fetchedAt, t);
      const parts = windows.map((window) => {
        const usable =
          window.usagePercent != null && Number.isFinite(window.usagePercent);
        const remainingPercent = usable
          ? windowRemainingPercent(window.usagePercent as number)
          : null;
        const tone =
          remainingPercent == null ? "neutral" : quotaRemainingTone(remainingPercent);
        const color =
          tone === "danger"
            ? token.colorError
            : tone === "warn"
              ? token.colorWarning
              : token.colorTextTertiary;
        const shortKey = quotaWindowShortLabelKey(window.kind);
        const fullKey = quotaWindowLabelKey(window.kind);
        return {
          kind: window.kind,
          shortLabel: shortKey ? t(shortKey) : window.kind,
          fullLabel: fullKey ? t(fullKey) : window.kind,
          remainingPercent,
          color,
        };
      });
      if (parts.some((part) => part.remainingPercent != null)) {
        quotaSummary = (
          <Tooltip
            title={
              <div className="text-xs">
                {parts.map((part) => (
                  <div key={part.kind}>
                    {`${part.fullLabel} ${
                      part.remainingPercent == null
                        ? "—"
                        : t("sites.quotaRemaining", {
                            amount: `${Math.round(part.remainingPercent)}%`,
                          })
                    }`}
                  </div>
                ))}
                <div>{updatedText}</div>
              </div>
            }
          >
            <span
              className="flex min-w-0 items-center gap-2 text-xs tabular-nums"
              data-testid="site-quota-summary"
            >
              {parts.map((part) => (
                <span
                  key={part.kind}
                  className="min-w-0 truncate"
                  style={{ color: part.color }}
                  data-testid={`site-quota-window-summary-${part.kind}`}
                >
                  <span className="opacity-50">{part.shortLabel}</span>{" "}
                  {part.remainingPercent == null
                    ? "—"
                    : `${Math.round(part.remainingPercent)}%`}
                </span>
              ))}
            </span>
          </Tooltip>
        );
      }
    } else if (isBalanceQuotaSummary(quota)) {
      const amount = formatQuotaAmountLocalized(quota.remainingUsd, quota.unit, t);
      const updatedText = formatQuotaUpdatedText(quota.fetchedAt, t);
      // 与详情面板同一条规则：有进度条就摆出已用，让分母（剩余 + 已用）可还原。
      const usedText =
        quota.usedUsd != null && quotaRemainingPercent(quota) != null
          ? formatQuotaAmountLocalized(quota.usedUsd, quota.unit, t)
          : null;
      quotaSummary = (
        <Tooltip
          title={
            <div className="text-xs">
              <div>{t("sites.quotaRemaining", { amount })}</div>
              {usedText && <div>{t("sites.quotaUsed", { amount: usedText })}</div>}
              <div>{updatedText}</div>
            </div>
          }
        >
          <span
            className="block truncate text-xs tabular-nums"
            style={{ color: token.colorTextTertiary }}
            data-testid="site-quota-summary"
          >
            {t("sites.quotaRemaining", { amount })}
          </span>
        </Tooltip>
      );
    }
  }

  return (
    <div
      ref={setNodeRef}
      data-testid="site-list-item"
      style={{
        transform: CSS.Translate.toString(transform),
        transition,
        opacity: isDragging ? 0.4 : 1,
        zIndex: isDragging ? 1 : undefined,
      }}
    >
      <Dropdown
        trigger={["contextMenu"]}
        destroyOnHidden
        menu={menu}
        onOpenChange={(open) => {
          if (open) onSelect();
        }}
      >
        <div
          className="site-list-item group flex w-full cursor-pointer items-center rounded-lg pr-0.5 transition-colors"
          data-active={active ? "true" : "false"}
          style={{
            background: active ? token.colorPrimaryBg : undefined,
            color: token.colorText,
            ["--site-item-bg" as string]: token.colorFillQuaternary,
            ["--site-item-hover" as string]: token.colorFillTertiary,
          }}
        >
          <button
            type="button"
            className="site-drag-handle inline-flex h-8 w-6 shrink-0 cursor-grab touch-none items-center justify-center active:cursor-grabbing"
            style={{ color: token.colorTextQuaternary }}
            aria-label={t("sites.dragHandle")}
            title={t("sites.dragHandle")}
            data-testid="site-drag-handle"
            {...attributes}
            {...listeners}
            onClick={(e) => {
              e.stopPropagation();
              onSelect();
            }}
          >
            <GripVertical size={14} className="block" />
          </button>
          <button
            type="button"
            onClick={onSelect}
            className="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 px-1.5 py-2 text-left"
          >
            <SiteAvatar siteId={site.id} name={site.name} baseUrl={site.baseUrl} size={28} />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-1.5">
                <StatusDot
                  className="shrink-0"
                  status={siteStatus}
                  title={statusTitle}
                />
                <div className="truncate text-sm font-medium">{site.name}</div>
              </div>
              {/* 第二行固定高度：站点没有额度摘要时也占位，列表行高保持整齐 */}
              <div className="min-h-5 min-w-0">{quotaSummary}</div>
            </div>
          </button>
          {/* 列表里直接开关：禁用后不再探测额度、不参与模型获取、也不出现在悬浮窗，
              所以不必先点进详情页才能关掉某个不想用的站点。 */}
          <Tooltip title={site.enabled ? t("sites.disabledHint") : t("sites.enabledHint")}>
            <Switch
              size="small"
              className="site-enable-switch shrink-0"
              checked={site.enabled}
              onChange={(next, event) => {
                event.stopPropagation();
                onToggleEnabled(next);
              }}
              onClick={(_, event) => event.stopPropagation()}
              aria-label={t("sites.enabled")}
            />
          </Tooltip>
          <Dropdown trigger={["click"]} destroyOnHidden menu={menu} placement="bottomRight">
            <Button
              type="text"
              size="small"
              className="site-more-btn mr-0.5 shrink-0"
              icon={<Ellipsis size={16} />}
              aria-label={t("sites.moreActions")}
              onClick={(e) => {
                e.stopPropagation();
                onSelect();
              }}
            />
          </Dropdown>
        </div>
      </Dropdown>
    </div>
  );
}
