import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { App, Button, Checkbox, Input, Modal, Segmented, Tooltip, theme } from "antd";
import { Check, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Site, SiteModel } from "@/types/domain";
import { groupModelsByPrefix } from "@/lib/modelPrefix";
import { runModelProbes, type ProbeMode } from "@/lib/modelProbe";
import { formatLatency } from "@/lib/routeProbe";
import { ModelCountBadge } from "@/components/sites/ModelTag";

interface Props {
  open: boolean;
  site: Site | null;
  models: SiteModel[];
  onClose: () => void;
}

type RowStatus = "idle" | "running" | "ok" | "error";

interface RowResult {
  status: RowStatus;
  latencyMs?: number;
  error?: string;
}

/** 未开始探测的行共用一个常量，保证 memo 行引用的稳定。 */
const IDLE_RESULT: RowResult = { status: "idle" };

export function TestModelsModal({ open, site, models, onClose }: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();
  const [query, setQuery] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [mode, setMode] = useState<ProbeMode>("serial");
  const [running, setRunning] = useState(false);
  // 探测结果先原地累积在 ref 里，按帧批量 flush 到 state。一个结果一次 setState 会让
  // 整张列表每次都重渲染（整轮 O(N²)）；flush 时复制一次，未变化的行仍共享同一个
  // result 引用，配合 memo 行只重渲染真正变了的那几行。
  const resultsRef = useRef<Record<string, RowResult>>({});
  const finishedRef = useRef(0);
  const frameRef = useRef<number | null>(null);
  const [snapshot, setSnapshot] = useState<{ results: Record<string, RowResult>; finished: number }>(
    { results: {}, finished: 0 },
  );
  const abortRef = useRef<AbortController | null>(null);
  const modelKey = models.map((m) => m.modelId).join("\0");

  const flushNow = useCallback(() => {
    setSnapshot({ results: { ...resultsRef.current }, finished: finishedRef.current });
  }, []);

  const cancelScheduledFlush = useCallback(() => {
    if (frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
  }, []);

  const scheduleFlush = useCallback(() => {
    if (frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      flushNow();
    });
  }, [flushNow]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelectedIds(new Set(models.map((m) => m.modelId)));
    setMode("serial");
    setRunning(false);
    abortRef.current?.abort();
    abortRef.current = null;
    cancelScheduledFlush();
    resultsRef.current = {};
    finishedRef.current = 0;
    flushNow();
    // Reset against the model ids present when the dialog opens / site changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, site?.id, modelKey, cancelScheduledFlush, flushNow]);

  useEffect(() => {
    return () => {
      abortRef.current?.abort();
      cancelScheduledFlush();
    };
  }, [cancelScheduledFlush]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => {
      const name = (m.displayName || m.modelId).toLowerCase();
      return name.includes(q) || m.modelId.toLowerCase().includes(q);
    });
  }, [models, query]);

  const groups = useMemo(() => groupModelsByPrefix(filtered), [filtered]);
  const selectedCount = selectedIds.size;
  const totalCount = models.length;
  const allSelected = totalCount > 0 && selectedCount === totalCount;
  const someSelected = selectedCount > 0 && selectedCount < totalCount;
  const results = snapshot.results;
  const finishedCount = snapshot.finished;

  // 稳定引用：memo 行不会因为父组件重渲染而全部重渲染。
  const toggleOne = useCallback(
    (modelId: string) => {
      if (running) return;
      setSelectedIds((prev) => {
        const next = new Set(prev);
        if (next.has(modelId)) next.delete(modelId);
        else next.add(modelId);
        return next;
      });
    },
    [running],
  );

  const toggleGroup = (ids: string[], checked: boolean) => {
    if (running) return;
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) {
        if (checked) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  };

  const toggleAll = (checked: boolean) => {
    if (running) return;
    setSelectedIds(checked ? new Set(models.map((m) => m.modelId)) : new Set());
  };

  const handleClose = () => {
    abortRef.current?.abort();
    cancelScheduledFlush();
    onClose();
  };

  const handleTest = async () => {
    if (!site || selectedCount === 0 || running) return;
    const ids = models.map((m) => m.modelId).filter((id) => selectedIds.has(id));
    const controller = new AbortController();
    abortRef.current?.abort();
    abortRef.current = controller;
    setRunning(true);
    resultsRef.current = {};
    finishedRef.current = 0;
    for (const id of ids) resultsRef.current[id] = IDLE_RESULT;
    flushNow();

    let ok = 0;
    let fail = 0;
    try {
      await runModelProbes({
        siteId: site.id,
        modelIds: ids,
        mode,
        signal: controller.signal,
        onStart: (modelId) => {
          resultsRef.current[modelId] = { status: "running" };
          scheduleFlush();
        },
        onResult: (result) => {
          if (result.ok) ok += 1;
          else fail += 1;
          finishedRef.current += 1;
          resultsRef.current[result.modelId] = result.ok
            ? { status: "ok", latencyMs: result.latencyMs }
            : {
                status: "error",
                latencyMs: result.latencyMs,
                error: result.error || t("sites.testFail"),
              };
          scheduleFlush();
        },
      });
    } finally {
      // 收尾：撤掉未执行的帧回调并立刻同步一次，进度/结果不会停在半帧上。
      cancelScheduledFlush();
      flushNow();
      setRunning(false);
    }

    if (controller.signal.aborted) return;
    if (fail === 0) {
      message.success(t("sites.testAllOk", { count: ok }));
    } else {
      message.warning(t("sites.testPartial", { ok, fail }));
    }
  };

  return (
    <Modal
      open={open}
      title={t("sites.testModelsTitle")}
      onCancel={handleClose}
      width={720}
      destroyOnHidden
      centered
      mask={{ enabled: true }}
      footer={
        <div className="flex items-center justify-between gap-3">
          <div className="flex min-w-0 flex-wrap items-center gap-3">
            <Checkbox
              checked={allSelected}
              indeterminate={someSelected}
              disabled={running || totalCount === 0}
              onChange={(e) => toggleAll(e.target.checked)}
            >
              {t("sites.testSelectAll")}
            </Checkbox>
            <span className="text-xs" style={{ color: token.colorTextSecondary }}>
              {t("sites.testSelectedCount", { selected: selectedCount, total: totalCount })}
            </span>
            <Segmented
              size="small"
              disabled={running}
              value={mode}
              onChange={(value) => setMode(value as ProbeMode)}
              options={[
                { label: t("sites.testModeSerial"), value: "serial" },
                { label: t("sites.testModeParallel"), value: "parallel" },
              ]}
            />
            {running && (
              <span className="text-xs" style={{ color: token.colorTextSecondary }}>
                {t("sites.testProgress", { done: finishedCount, total: selectedCount })}
              </span>
            )}
          </div>
          <div className="flex shrink-0 gap-2">
            <Button onClick={handleClose}>{t("common.cancel")}</Button>
            <Button
              type="primary"
              loading={running}
              disabled={selectedCount === 0}
              onClick={() => void handleTest()}
            >
              {t("sites.testNow")}
            </Button>
          </div>
        </div>
      }
    >
      <div className="flex min-h-0 flex-col gap-3">
        <Input.Search
          allowClear
          placeholder={t("sites.modelSearch")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <div className="scroll-y min-h-[200px] max-h-[48vh]">
          {filtered.length === 0 ? (
            <div
              className="flex min-h-[200px] items-center justify-center text-sm"
              style={{ color: token.colorTextSecondary }}
            >
              {models.length === 0 ? t("sites.noModels") : t("sites.noModelsMatch")}
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              {groups.map((group) => {
                const groupIds = group.models.map((m) => m.modelId);
                const selectedInGroup = groupIds.filter((id) => selectedIds.has(id)).length;
                const groupChecked = groupIds.length > 0 && selectedInGroup === groupIds.length;
                const groupPartial = selectedInGroup > 0 && selectedInGroup < groupIds.length;
                return (
                  <div key={group.prefix} data-model-group={group.prefix}>
                    <div className="mb-1.5 flex items-center gap-1.5">
                      <Checkbox
                        checked={groupChecked}
                        indeterminate={groupPartial}
                        disabled={running}
                        aria-label={group.prefix}
                        onChange={(e) => toggleGroup(groupIds, e.target.checked)}
                      />
                      <span
                        className="text-xs font-medium"
                        style={{ color: token.colorTextTertiary }}
                      >
                        {group.prefix}
                      </span>
                      <ModelCountBadge>
                        {t("sites.modelCount", { count: group.models.length })}
                      </ModelCountBadge>
                    </div>
                    <div className="flex flex-col">
                      {group.models.map((model) => (
                        <ProbeRow
                          key={model.id || model.modelId}
                          modelId={model.modelId}
                          label={model.displayName || model.modelId}
                          checked={selectedIds.has(model.modelId)}
                          disabled={running}
                          result={results[model.modelId] ?? IDLE_RESULT}
                          onToggle={toggleOne}
                        />
                      ))}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}

interface ProbeRowProps {
  modelId: string;
  label: string;
  checked: boolean;
  disabled: boolean;
  result: RowResult;
  onToggle: (modelId: string) => void;
}

/**
 * 单行用 memo 包住：一次 flush 只让结果真的变了的那几行重渲染，而不是整张列表。
 * onToggle 由父组件 useCallback 提供稳定引用，否则 memo 会被新函数引用击穿。
 */
const ProbeRow = memo(function ProbeRow({
  modelId,
  label,
  checked,
  disabled,
  result,
  onToggle,
}: ProbeRowProps) {
  const { token } = theme.useToken();
  return (
    <div
      data-model-probe-row={modelId}
      data-probe-status={result.status}
      className="flex items-center gap-2 py-1"
    >
      <Checkbox
        checked={checked}
        disabled={disabled}
        aria-label={modelId}
        onChange={() => onToggle(modelId)}
      />
      <span className="min-w-0 flex-1 truncate" title={modelId} style={{ color: token.colorText }}>
        {label}
      </span>
      <ProbeStatus result={result} />
    </div>
  );
});

function ProbeStatus({ result }: { result: RowResult }) {
  const { token } = theme.useToken();
  const { t } = useTranslation();

  if (result.status === "idle") {
    return <div className="min-w-[160px]" />;
  }

  if (result.status === "running") {
    return (
      <div
        className="flex min-w-[160px] items-center justify-end gap-1.5"
        style={{ color: token.colorTextSecondary }}
      >
        <Loader2 size={14} className="animate-spin" aria-label={t("sites.testRunning")} />
      </div>
    );
  }

  const latency =
    result.latencyMs != null ? formatLatency(result.latencyMs) : null;

  if (result.status === "ok") {
    return (
      <div
        className="flex min-w-[160px] items-center justify-end gap-1.5"
        style={{ color: token.colorSuccess }}
      >
        <Check size={14} aria-label={t("sites.testOk")} />
        {latency && <span className="text-xs">{latency}</span>}
      </div>
    );
  }

  return (
    <div className="flex min-w-[160px] max-w-[280px] items-center justify-end gap-1.5">
      <X size={14} aria-label={t("sites.testFail")} style={{ color: token.colorError }} />
      {result.error && (
        <Tooltip title={result.error}>
          <span
            data-probe-error
            className="max-w-[240px] truncate text-xs"
            style={{ color: token.colorError }}
          >
            {result.error}
          </span>
        </Tooltip>
      )}
      {latency && (
        <span className="shrink-0 text-xs" style={{ color: token.colorTextSecondary }}>
          {latency}
        </span>
      )}
    </div>
  );
}
