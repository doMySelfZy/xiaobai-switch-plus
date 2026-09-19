import { useState, type ReactNode } from "react";
import { App, Button, Collapse, Descriptions, Popconfirm, Tag, theme } from "antd";
import { Eraser, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SiteAvatar } from "@/components/sites/SiteAvatar";
import { isAppError } from "@/lib/invoke";
import { revealInExplorer } from "@/lib/revealInExplorer";
import { useSiteStore } from "@/stores";
import { useAgentUpdateStore } from "@/stores/agentUpdateStore";
import type { ApplyStatus, CliToolInfo, TargetKind, TargetLiveStatus } from "@/types/domain";

const STATUS_COLOR: Record<string, string> = {
  applied: "success",
  stale: "warning",
  orphan: "orange",
  not_applied: "default",
  failed: "error",
};

interface TargetStatusCardProps {
  status: TargetLiveStatus | undefined;
  tool: CliToolInfo | undefined;
  onRefresh: () => void | Promise<void>;
  onRevert: () => void | Promise<void>;
  onCleanupOrphan: () => void | Promise<void>;
  onUpdate?: () => void | Promise<void>;
}

/** Prefer a numeric version token from `claude --version` / `codex --version` output. */
export function cliVersionLabel(version: string | null | undefined): string | null {
  if (!version) return null;
  const trimmed = version.trim();
  if (!trimmed) return null;
  const match = trimmed.match(/\d+\.\d+(?:\.\d+)?(?:[-+][A-Za-z0-9.]+)?/);
  return match?.[0] ?? trimmed;
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  const { token } = theme.useToken();
  return (
    <div className="min-w-0">
      <div className="text-xs leading-5" style={{ color: token.colorTextTertiary }}>
        {label}
      </div>
      <div className="mt-0.5 break-all text-sm leading-5" style={{ color: token.colorText }}>
        {children}
      </div>
    </div>
  );
}

export function TargetStatusCard({
  status,
  tool,
  onRefresh,
  onRevert,
  onCleanupOrphan,
  onUpdate,
}: TargetStatusCardProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();
  const sites = useSiteStore((s) => s.sites);
  const [refreshing, setRefreshing] = useState(false);
  const [reverting, setReverting] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [updating, setUpdating] = useState(false);

  const updateStatus = useAgentUpdateStore((s) => s.getUpdateStatus(status?.kind ?? ""));
  const isUpdating = useAgentUpdateStore((s) => s.isUpdating(status?.kind ?? ""));

  if (!status) {
    return (
      <div className="text-sm" style={{ color: token.colorTextSecondary }}>
        {t("common.loading")}
      </div>
    );
  }

  const kindLabel = t(targetKindLabelKey(status.kind));
  const installed = status.installed;
  const version = status.version ?? tool?.version ?? null;
  const versionLabel = cliVersionLabel(version);
  const appliedSite = status.appliedSiteId
    ? sites.find((s) => s.id === status.appliedSiteId)
    : undefined;
  const siteName = appliedSite?.name ?? status.appliedSiteName;
  const modelId = status.appliedModelId;
  const canRevert = status.status !== "not_applied" && !status.orphan;

  const summaryEntries = Object.entries(status.liveSummary).filter(
    ([, v]) => v != null && String(v).length > 0,
  );

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await onRefresh();
      message.success(t("apply.refreshSuccess"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : t("apply.refreshFailed"));
    } finally {
      setRefreshing(false);
    }
  };

  const handleRevert = async () => {
    setReverting(true);
    try {
      await onRevert();
      message.success(t("apply.revertSuccess"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
      throw e;
    } finally {
      setReverting(false);
    }
  };

  const handleCleanup = async () => {
    setCleaning(true);
    try {
      await onCleanupOrphan();
      message.success(t("apply.cleanupSuccess"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
      throw e;
    } finally {
      setCleaning(false);
    }
  };

  const handleOpenPath = async () => {
    try {
      await revealInExplorer(status.configPath);
    } catch (e) {
      message.error(isAppError(e) ? e.message : t("apply.openPathFailed"));
    }
  };

  const handleUpdate = async () => {
    if (!onUpdate) return;
    setUpdating(true);
    try {
      await onUpdate();
      message.success(t("apply.agentUpdateSuccess"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : t("apply.agentUpdateFailed"));
    } finally {
      setUpdating(false);
    }
  };

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="font-medium">{kindLabel}</span>
          <Tag color={installed ? "green" : "red"} title={version ?? undefined}>
            {installed ? (versionLabel ?? t("apply.installed")) : t("apply.notInstalled")}
          </Tag>
          {updateStatus?.hasUpdate && (
            <Tag color="orange" title={`${updateStatus.currentVersion} → ${updateStatus.latestVersion}`}>
              {t("apply.agentHasUpdate")}
            </Tag>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Tag color={STATUS_COLOR[status.status] ?? "default"}>
            {t(`apply.status_${status.status}`)}
          </Tag>
          {updateStatus?.hasUpdate && onUpdate && (
            <Button
              size="small"
              type="primary"
              loading={updating || isUpdating}
              onClick={() => void handleUpdate()}
            >
              {t("apply.agentUpdate")}
            </Button>
          )}
          {canRevert && (
            <Popconfirm
              title={t("apply.revertConfirm")}
              description={t("apply.revertConfirmHint")}
              okText={t("common.confirm")}
              cancelText={t("common.cancel")}
              okButtonProps={{ danger: true, loading: reverting }}
              onConfirm={handleRevert}
            >
              <Button size="small" danger icon={<Eraser size={14} />} loading={reverting}>
                {t("apply.revert")}
              </Button>
            </Popconfirm>
          )}
          {status.orphan && (
            <Popconfirm
              title={t("apply.cleanupConfirm")}
              description={t("apply.cleanupConfirmHint")}
              okText={t("common.confirm")}
              cancelText={t("common.cancel")}
              okButtonProps={{ danger: true, loading: cleaning }}
              onConfirm={handleCleanup}
            >
              <Button size="small" danger loading={cleaning}>
                {t("apply.cleanupOrphan")}
              </Button>
            </Popconfirm>
          )}
          <Button
            size="small"
            icon={<RefreshCw size={14} />}
            loading={refreshing}
            onClick={() => void handleRefresh()}
          >
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      <div className="flex flex-col" style={{ gap: 14 }}>
        <Fact label={t("apply.configPath")}>
          <button
            type="button"
            className="apply-config-path"
            title={t("apply.openInExplorer")}
            onClick={() => void handleOpenPath()}
          >
            {status.configPath}
          </button>
        </Fact>

        {(siteName || modelId) && (
          <Fact label={t("apply.siteModel")}>
            <span className="inline-flex min-w-0 items-center gap-2">
              <SiteAvatar
                siteId={appliedSite?.id ?? status.appliedSiteId ?? siteName ?? "site"}
                name={siteName ?? modelId ?? ""}
                baseUrl={appliedSite?.baseUrl ?? ""}
                size={18}
              />
              <span className="min-w-0 break-all">
                {siteName}
                {modelId ? (
                  <span style={{ color: token.colorTextTertiary }}>（{modelId}）</span>
                ) : null}
              </span>
            </span>
          </Fact>
        )}

        {status.providerId && <Fact label={t("apply.provider")}>{status.providerId}</Fact>}
      </div>

      {status.staleReason && (
        <div className="mt-3 text-sm" style={{ color: token.colorWarning }}>
          {status.staleReason}
        </div>
      )}

      {summaryEntries.length > 0 && (
        <Collapse
          size="small"
          className="mt-6"
          style={{ marginTop: 24 }}
          defaultActiveKey={[]}
          items={[
            {
              key: "summary",
              label: t("apply.liveSummary"),
              children: (
                <Descriptions
                  size="small"
                  column={1}
                  items={summaryEntries.map(([k, v]) => ({
                    key: k,
                    label: k,
                    children: v ?? "—",
                  }))}
                />
              ),
            },
          ]}
        />
      )}
    </div>
  );
}

export function targetKindLabelKey(
  kind: TargetKind,
):
  | "apply.targetClaude"
  | "apply.targetCodex"
  | "apply.targetPi"
  | "apply.targetPrime" {
  if (kind === "claude_code") return "apply.targetClaude";
  if (kind === "codex") return "apply.targetCodex";
  if (kind === "pi") return "apply.targetPi";
  if (kind === "prime") return "apply.targetPrime";
  // 穷尽性检查：四个目标都在上面返回了。将来加新目标却忘了加分支，这里编译失败。
  // 真在运行时拿到本版本不认识的名字（旧对端推来的退役目标残留）时原样回显裸 token，
  // 也不能冒充某个存活目标 —— 那会让用户把别的目标的数据看成这个目标的。
  const unreachable: never = kind;
  return unreachable;
}

export function statusFor(
  statuses: TargetLiveStatus[],
  kind: TargetKind,
): TargetLiveStatus | undefined {
  return statuses.find((s) => s.kind === kind);
}

/** True when the target still has written config (applied / stale / leftover). */
export function isConfiguredStatus(status: ApplyStatus | undefined): boolean {
  return status === "applied" || status === "stale" || status === "orphan";
}

export function targetsAppliedForSite(
  statuses: TargetLiveStatus[],
  siteId: string,
): TargetKind[] {
  return statuses.filter((s) => s.appliedSiteId === siteId).map((s) => s.kind);
}

export function toolFor(tools: CliToolInfo[], kind: TargetKind): CliToolInfo | undefined {
  return tools.find((t) => t.kind === kind);
}
