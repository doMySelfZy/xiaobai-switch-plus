import { useCallback, useEffect, useState } from "react";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { restrictToVerticalAxis } from "@dnd-kit/modifiers";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { App, Button, Checkbox, Empty, Skeleton, Switch, Tooltip, theme } from "antd";
import { Plus, Trash2, Pencil, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useApplyStore, useSiteStore, useUIStore } from "@/stores";
import { reorderList } from "@/lib/reorder";
import { SiteFormModal, type SiteFormInitialValues } from "@/components/sites/SiteFormModal";
import { ManualModelModal } from "@/components/sites/ManualModelModal";
import { GoApplyButton } from "@/components/sites/GoApplyButton";
import { ModelPicker } from "@/components/sites/ModelPicker";
import { SiteAvatar } from "@/components/sites/SiteAvatar";
import { SiteListItem } from "@/components/sites/SiteListItem";
import { SiteDetailSkeleton } from "@/components/sites/SiteDetailSkeleton";
import { EmptyOnboarding } from "@/components/sites/EmptyOnboarding";
import { SiteRouteSwitcher } from "@/components/sites/SiteRouteSwitcher";
import { SiteApiKeySwitcher } from "@/components/sites/SiteApiKeySwitcher";
import { SiteQuotaRow } from "@/components/sites/SiteQuotaRow";
import type { Site } from "@/types/domain";
import { isAppError } from "@/lib/invoke";
import { SITE_QUOTA_AUTO_REFRESH_MS, quotaCacheKey } from "@/lib/quotaProbe";
import { useDeferredReady } from "@/hooks/useDeferredReady";
import { usePageVisible } from "@/hooks/usePageVisible";
import { targetKindLabelKey, targetsAppliedForSite } from "@/components/apply/TargetStatusCard";

function protocolLabelKey(protocol: Site["protocol"]): string {
  return protocol === "anthropic" ? "sites.protocolAnthropic" : "sites.protocolOpenai";
}

export function SitesPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { modal, message } = App.useApp();
  const sites = useSiteStore((s) => s.sites);
  const modelsBySite = useSiteStore((s) => s.modelsBySite);
  const loading = useSiteStore((s) => s.loading);
  const hydrated = useSiteStore((s) => s.hydrated);
  // per-site：某站点在拉模型时，只让它的刷新按钮转圈（全局布尔会让别的站点也转）。
  const fetchingModelsBySite = useSiteStore((s) => s.fetchingModelsBySite);
  const loadSites = useSiteStore((s) => s.loadSites);
  const listModels = useSiteStore((s) => s.listModels);
  const fetchModels = useSiteStore((s) => s.fetchModels);
  const probeQuota = useSiteStore((s) => s.probeQuota);
  const quotaBySite = useSiteStore((s) => s.quotaBySite);
  const quotaAttemptBySite = useSiteStore((s) => s.quotaAttemptBySite);
  const quotaCacheKeyBySite = useSiteStore((s) => s.quotaCacheKeyBySite);
  const quotaAttemptCacheKeyBySite = useSiteStore((s) => s.quotaAttemptCacheKeyBySite);
  const quotaLoadingBySite = useSiteStore((s) => s.quotaLoadingBySite);
  const setSelectedModel = useSiteStore((s) => s.setSelectedModel);
  const deleteSite = useSiteStore((s) => s.deleteSite);
  const reorderSites = useSiteStore((s) => s.reorderSites);
  const updateSite = useSiteStore((s) => s.updateSite);
  const loadStatus = useApplyStore((s) => s.loadStatus);
  const revert = useApplyStore((s) => s.revert);
  const selectedSiteId = useUIStore((s) => s.selectedSiteId);
  const setSelectedSiteId = useUIStore((s) => s.setSelectedSiteId);
  const setPage = useUIStore((s) => s.setPage);
  const setApplyTab = useUIStore((s) => s.setApplyTab);
  const setApplyPrefillSiteId = useUIStore((s) => s.setApplyPrefillSiteId);

  // KeepAlivePages 让本页常驻：只有"当前页正是站点页且窗口可见"才算真的在看。
  const pageVisible = usePageVisible("sites");

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Site | null>(null);
  const [forceAdvancedOpen, setForceAdvancedOpen] = useState(false);
  const [formInitial, setFormInitial] = useState<SiteFormInitialValues | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const pendingSiteForm = useUIStore((s) => s.pendingSiteForm);
  const setPendingSiteForm = useUIStore((s) => s.setPendingSiteForm);

  useEffect(() => {
    // Soft when already hydrated so revisiting sites page doesn't flash loading.
    void loadSites({ soft: useSiteStore.getState().hydrated });
  }, [loadSites]);

  // 列表额度摘要预取 + 自动刷新：probeQuota 自带 5 分钟 TTL + in-flight 去重，
  // 就是节流层，不要再包一层缓存。失败静默（错误态只在右侧详情展示）。依赖用
  // 站点 id 串，避免对象引用变化导致重复触发。
  // 轮询只在「本页真正可见」时跑：KeepAlive 让页面常驻，只判 document.visibilityState
  // 会在用户看别的页面时继续发请求。后台刷新一律不 force——TTL 该挡住就挡住；
  // 只有手动刷新按钮（handleRefreshQuota）才 force。pageVisible 变真时补一次非强制刷新，
  // 既覆盖"切回本页"，也覆盖"窗口重新可见"。
  // 优化：间隔从 2 分钟降至 30 秒，且只轮询当前选中的站点以降低网络噪音。
  useEffect(() => {
    if (!pageVisible || !selectedSiteId) return;
    const selected = useSiteStore.getState().sites.find((s) => s.id === selectedSiteId);
    if (!selected?.enabled) return;

    const refresh = () => {
      const current = useSiteStore.getState().sites.find((s) => s.id === selectedSiteId);
      if (current?.enabled) {
        void probeQuota(selectedSiteId).catch(() => undefined);
      }
    };

    refresh();
    const timer = window.setInterval(refresh, SITE_QUOTA_AUTO_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [selectedSiteId, probeQuota, pageVisible]);

  useEffect(() => {
    if (!pendingSiteForm) return;
    setEditing(null);
    setFormInitial({
      name: pendingSiteForm.name,
      baseUrls: pendingSiteForm.baseUrls,
      apiKey: pendingSiteForm.apiKey,
      protocol: pendingSiteForm.protocol,
      notes: pendingSiteForm.notes,
      capabilities: pendingSiteForm.hasCapabilityParams ? pendingSiteForm.capabilities : undefined,
    });
    setFormOpen(true);
    setPendingSiteForm(null);
  }, [pendingSiteForm, setPendingSiteForm]);

  const openCreateForm = () => {
    setEditing(null);
    setFormInitial(null);
    setFormOpen(true);
    setForceAdvancedOpen(false);
  };

  const selected = sites.find((s) => s.id === selectedSiteId) ?? sites[0] ?? null;

  useEffect(() => {
    if (selected && selected.id !== selectedSiteId) {
      setSelectedSiteId(selected.id);
    }
  }, [selected, selectedSiteId, setSelectedSiteId]);

  useEffect(() => {
    setManualOpen(false);
  }, [selected?.id]);

  // Defer heavy detail mount one frame after site switch so sidebar highlight paints first.
  const detailReady = useDeferredReady(selected?.id ?? null);

  useEffect(() => {
    if (!selectedSiteId) return;
    void listModels(selectedSiteId).then((models) => {
      const site = useSiteStore.getState().sites.find((s) => s.id === selectedSiteId);
      if (site && !site.selectedModelId && models[0]) {
        void setSelectedModel(selectedSiteId, models[0].modelId);
      }
    });
  }, [selectedSiteId, listModels, setSelectedModel]);

  // 详情额度探测与窗口 focus 探测同样只在「本页真正可见」时发请求：页面常驻，
  // 用户在别处时不该替这个页面打探测。回到本页时 pageVisible 变真、effect 重跑，
  // 自然补一次（非 force，走 TTL）。
  useEffect(() => {
    if (!pageVisible || !selected?.id || !selected.hasKey) return;
    void probeQuota(selected.id).catch(() => {
      message.error(t("sites.quotaRefreshFailed"));
    });
  }, [
    pageVisible,
    selected?.id,
    selected?.baseUrl,
    selected?.quotaRevision,
    selected?.hasKey,
    probeQuota,
    message,
    t,
  ]);

  useEffect(() => {
    if (!pageVisible || !selected?.id || !selected.hasKey) return;
    const refreshOnFocus = () => {
      // 回到窗口只补一次非强制刷新，不绕过 5 分钟 TTL。
      void probeQuota(selected.id).catch(() => {
        message.error(t("sites.quotaRefreshFailed"));
      });
    };
    window.addEventListener("focus", refreshOnFocus);
    return () => window.removeEventListener("focus", refreshOnFocus);
  }, [
    pageVisible,
    selected?.id,
    selected?.baseUrl,
    selected?.quotaRevision,
    selected?.hasKey,
    probeQuota,
    message,
    t,
  ]);

  const handleFetchModels = useCallback(
    async (site: Site) => {
      try {
        const result = await fetchModels(site.id);
        if (!site.selectedModelId && result.models[0]) {
          await setSelectedModel(site.id, result.models[0].modelId);
        }
        message.success(t("sites.fetchModelsSuccess", { count: result.models.length }));
      } catch (e) {
        message.error(
          isAppError(e)
            ? e.code === "unauthorized"
              ? t("sites.modelListUnauthorized")
              : t(`errors.${e.code}`)
            : t("sites.modelListFetchRetry"),
        );
      }
    },
    [fetchModels, setSelectedModel, message, t],
  );

  const handleRefreshQuota = useCallback(async () => {
    if (!selected) return;
    try {
      const result = await probeQuota(selected.id, { force: true });
      if (result.status !== "available") {
        message.warning(t("sites.quotaRefreshFailed"));
      }
    } catch (e) {
      message.warning(isAppError(e) ? e.message : t("sites.quotaRefreshFailed"));
    }
  }, [selected, probeQuota, message, t]);

  // 全局刷新：批量刷新所有站点的模型列表（带重试）
  const [refreshingAll, setRefreshingAll] = useState(false);

  const handleRefreshAll = useCallback(async () => {
    if (refreshingAll) return;
    
    setRefreshingAll(true);
    
    // 只刷新已启用的站点
    const enabledSites = sites.filter(s => s.enabled);
    
    // 带重试的刷新函数
    const fetchWithRetry = async (site: Site, maxRetries = 2): Promise<boolean> => {
      for (let attempt = 0; attempt <= maxRetries; attempt++) {
        try {
          await fetchModels(site.id);
          return true;
        } catch (e) {
          if (attempt === maxRetries) {
            return false;
          }
          // 等待后重试
          await new Promise(resolve => setTimeout(resolve, 1000));
        }
      }
      return false;
    };

    // 并行刷新所有站点
    const results = await Promise.all(
      enabledSites.map(site => fetchWithRetry(site))
    );

    setRefreshingAll(false);

    // 统计结果
    const successCount = results.filter(r => r).length;
    const failureCount = results.filter(r => !r).length;

    if (failureCount === 0) {
      message.success(t("sites.refreshAllSuccess", { count: successCount }));
    } else {
      message.warning(
        t("sites.refreshAllPartial", { 
          success: successCount, 
          total: enabledSites.length 
        })
      );
    }
  }, [refreshingAll, sites, fetchModels, message, t]);

  const handleSiteSaved = useCallback(
    (site: Site, isCreate: boolean) => {
      setSelectedSiteId(site.id);
      if (isCreate) {
        void handleFetchModels(site);
      }
    },
    [setSelectedSiteId, handleFetchModels],
  );

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const handleSiteDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const ids = useSiteStore.getState().sites.map((s) => s.id);
      const next = reorderList(ids, ids.indexOf(String(active.id)), ids.indexOf(String(over.id)));
      if (next === ids) return;
      void reorderSites(next).catch((e) => {
        message.error(isAppError(e) ? e.message : t("sites.reorderFailed"));
      });
    },
    [reorderSites, message, t],
  );

  const disableSite = async (site: Site, clear: boolean) => {
    try {
      if (clear) {
        const targets = targetsAppliedForSite(useApplyStore.getState().statuses, site.id);
        for (const kind of targets) {
          await revert(kind);
        }
      }
      await updateSite(site.id, { enabled: false });
      message.success(clear ? t("sites.disableClearedSuccess") : t("sites.disableSuccess"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
    }
  };

  /**
   * 启用 / 停用站点。
   *
   * 停用原先要先 `loadStatus({ force: true })` 才更新 UI——那会串行 spawn 4 个 CLI 进程
   * 探测版本，开关看起来"点了没反应"。现在先乐观落库（updateSite 直接把新状态写回
   * store），状态刷新扔后台且**不 force**（非 force 命中后端 60s CLI 探测缓存）。
   * 确认弹窗用已有状态判断，不需要新探测：用户点「取消」就把乐观更新回滚，
   * 语义与旧版一致——取消 = 不停用。
   */
  const handleEnabledChange = async (site: Site, enabled: boolean) => {
    try {
      await updateSite(site.id, { enabled });
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
      return;
    }
    if (enabled) return;

    // 「取消」= 不停用：把乐观更新回滚回去。
    const restoreEnabled = async () => {
      try {
        await updateSite(site.id, { enabled: true });
      } catch (e) {
        message.error(isAppError(e) ? e.message : String(e));
      }
    };

    // 后台补状态，不 force：只重读绑定表 + 命中后端 60s CLI 探测缓存。
    const refreshStatuses = loadStatus({ background: true });
    if (useApplyStore.getState().statusHydrated) {
      void refreshStatuses.catch(() => undefined);
    } else {
      // 冷启动时状态还没读过：必须等这一次（非强制）刷新，
      // 否则会把"已应用"误判成"没应用"而跳过确认弹窗。
      try {
        await refreshStatuses;
      } catch (e) {
        // 状态读不出来就无法判断是否正被使用：回滚乐观更新，保持旧语义（站点仍启用）。
        await restoreEnabled();
        message.error(isAppError(e) ? e.message : String(e));
        return;
      }
    }

    const targets = targetsAppliedForSite(useApplyStore.getState().statuses, site.id);
    if (targets.length === 0) {
      await disableSite(site, false);
      return;
    }

    const targetLabels = targets
      .map((kind) => t(targetKindLabelKey(kind)))
      .join(t("common.listSep"));

    const dlg = modal.confirm({
      centered: true,
      title: t("sites.disableAppliedTitle"),
      content: t("sites.disableAppliedHint", { targets: targetLabels }),
      footer: (
        <div className="flex justify-end gap-2">
          <Button
            onClick={() => {
              dlg.destroy();
              void restoreEnabled();
            }}
          >
            {t("common.cancel")}
          </Button>
          <Button
            onClick={() => {
              dlg.destroy();
              void disableSite(site, false);
            }}
          >
            {t("sites.disableSkip")}
          </Button>
          <Button
            type="primary"
            danger
            onClick={() => {
              dlg.destroy();
              void disableSite(site, true);
            }}
          >
            {t("sites.disableClear")}
          </Button>
        </div>
      ),
    });
  };

  const handleDelete = (site: Site) => {
    let cleanup = false;
    modal.confirm({
      title: t("sites.deleteConfirm", { name: site.name }),
      centered: true,
      content: (
        <div className="mt-2">
          <Checkbox
            onChange={(e) => {
              cleanup = e.target.checked;
            }}
          >
            <div>
              <div>{t("sites.cleanupTargets")}</div>
              <div className="text-xs opacity-60" style={{ whiteSpace: "normal" }}>
                {t("sites.cleanupTargetsHint")}
              </div>
            </div>
          </Checkbox>
        </div>
      ),
      okText: t("common.confirm"),
      cancelText: t("common.cancel"),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await deleteSite(site.id, cleanup);
          if (selectedSiteId === site.id) setSelectedSiteId(null);
          message.success(t("sites.deleteSuccess"));
        } catch (e) {
          message.error(isAppError(e) ? e.message : String(e));
        }
      },
    });
  };

  // First load with no cache: lightweight skeleton list instead of blank freeze.
  if (loading && !hydrated) {
    return (
      <div className="flex h-full min-h-0">
        <div
          className="flex w-72 shrink-0 flex-col border-r p-3"
          style={{ borderColor: token.colorBorderSecondary }}
        >
          <Skeleton active paragraph={{ rows: 6 }} title={{ width: "50%" }} />
        </div>
        <div className="min-w-0 flex-1 p-4">
          <SiteDetailSkeleton />
        </div>
      </div>
    );
  }

  if (!loading && sites.length === 0) {
    return (
      <>
        <EmptyOnboarding onAdd={openCreateForm} />
        <SiteFormModal
          open={formOpen}
          site={editing}
          initialValues={editing ? null : formInitial}
          forceAdvancedOpen={forceAdvancedOpen}
          onClose={() => {
            setFormOpen(false);
            setFormInitial(null);
            setForceAdvancedOpen(false);
          }}
          onSaved={handleSiteSaved}
        />
      </>
    );
  }

  const models = selected ? (modelsBySite[selected.id] ?? []) : [];
  const modelsCached = selected
    ? Object.prototype.hasOwnProperty.call(modelsBySite, selected.id)
    : false;
  // Sidebar paints first; detail waits one frame + model list cache for the selected site.
  const showDetailSkeleton = Boolean(selected) && (!detailReady || !modelsCached);

  return (
    <div className="flex h-full min-h-0">
      <div
        className="flex w-72 shrink-0 flex-col border-r"
        style={{ borderColor: token.colorBorderSecondary }}
      >
        <div className="flex items-center justify-between p-3">
          <span className="font-medium">{t("sites.title")}</span>
          <div className="flex items-center gap-2">
            <Tooltip title={t("sites.refreshAll")}>
              <Button
                type="text"
                size="small"
                loading={refreshingAll}
                icon={<RefreshCw size={14} />}
                onClick={() => void handleRefreshAll()}
                aria-label={t("sites.refreshAll")}
              />
            </Tooltip>
            <Button
              color="default"
              size="small"
              icon={<Plus size={14} />}
              onClick={openCreateForm}
            >
              {t("sites.add")}
            </Button>
          </div>
        </div>
        <div className="scroll-y flex flex-1 flex-col gap-2 px-2 pb-2">
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={handleSiteDragEnd}
          >
            <SortableContext items={sites.map((s) => s.id)} strategy={verticalListSortingStrategy}>
              {sites.map((site) => (
                <SiteListItem
                  key={site.id}
                  site={site}
                  active={selected?.id === site.id}
                  onSelect={() => setSelectedSiteId(site.id)}
                  onEdit={() => {
                    setEditing(site);
                    setForceAdvancedOpen(false);
                    setFormOpen(true);
                  }}
                  onDelete={() => handleDelete(site)}
                  onToggleEnabled={(next) => void handleEnabledChange(site, next)}
                />
              ))}
            </SortableContext>
          </DndContext>
        </div>
      </div>

      <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden p-4">
        {selected ? (
          showDetailSkeleton ? (
            <SiteDetailSkeleton />
          ) : (
            <div className="flex h-full min-h-0 flex-1 flex-col">
              <div className="mb-4 flex shrink-0 items-start justify-between gap-3">
                <div className="flex min-w-0 flex-1 items-center gap-2.5">
                  <SiteAvatar
                    siteId={selected.id}
                    name={selected.name}
                    baseUrl={selected.baseUrl}
                    size={32}
                  />
                  <span className="min-w-0 truncate text-base font-medium">{selected.name}</span>
                  <div className="shrink-0">
                    <GoApplyButton
                      disabled={!selected.selectedModelId || !selected.enabled}
                      onApply={() => {
                        setSelectedSiteId(selected.id);
                        setApplyPrefillSiteId(selected.id);
                        setApplyTab("claude_code");
                        setPage("apply");
                      }}
                    />
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <Switch
                    size="small"
                    checked={selected.enabled}
                    checkedChildren={t("sites.enabled")}
                    unCheckedChildren={t("sites.disabled")}
                    onChange={(v) => void handleEnabledChange(selected, v)}
                  />
                  <Tooltip title={t("sites.fetchModels")}>
                    <Button
                      type="text"
                      size="small"
                      loading={Boolean(selected && fetchingModelsBySite[selected.id])}
                      icon={<RefreshCw size={14} />}
                      onClick={() => void handleFetchModels(selected)}
                      aria-label={t("sites.fetchModels")}
                    />
                  </Tooltip>
                  <Tooltip title={t("sites.edit")}>
                    <Button
                      type="text"
                      size="small"
                      icon={<Pencil size={14} />}
                      onClick={() => {
                        setEditing(selected);
                        setFormOpen(true);
                      }}
                      aria-label={t("sites.edit")}
                    />
                  </Tooltip>
                  <Tooltip title={t("sites.delete")}>
                    <Button
                      type="text"
                      size="small"
                      danger
                      icon={<Trash2 size={14} />}
                      onClick={() => handleDelete(selected)}
                      aria-label={t("sites.delete")}
                    />
                  </Tooltip>
                </div>
              </div>

              <div className="mb-4 shrink-0 space-y-2 text-sm">
                <div className="flex gap-2">
                  <span className="w-28 shrink-0 opacity-50">{t("sites.baseUrl")}</span>
                  <div className="min-w-0 flex-1">
                    <SiteRouteSwitcher site={selected} />
                  </div>
                </div>
                <div className="flex gap-2">
                  <span className="w-28 shrink-0 opacity-50">{t("sites.apiKey")}</span>
                  <div className="min-w-0 flex-1">
                    <SiteApiKeySwitcher site={selected} />
                  </div>
                </div>
                <SiteQuotaRow
                  quota={
                    quotaCacheKeyBySite[selected.id] === quotaCacheKey(selected)
                      ? (quotaBySite[selected.id] ?? null)
                      : null
                  }
                  attempt={
                    quotaAttemptCacheKeyBySite[selected.id] === quotaCacheKey(selected)
                      ? (quotaAttemptBySite[selected.id] ?? null)
                      : null
                  }
                  loading={Boolean(quotaLoadingBySite[selected.id])}
                  refreshing={
                    Boolean(quotaLoadingBySite[selected.id]) &&
                    quotaBySite[selected.id]?.status === "available"
                  }
                  onRefresh={() => void handleRefreshQuota()}
                  showConfigureHint={!selected.newapiConfigured}
                  onConfigureQuota={() => {
                    setEditing(selected);
                    setFormInitial(null);
                    setFormOpen(true);
                    setForceAdvancedOpen(true);
                  }}
                />
                <div className="flex gap-2">
                  <span className="w-28 shrink-0 opacity-50">{t("sites.protocol")}</span>
                  <span>{t(protocolLabelKey(selected.protocol))}</span>
                </div>
                {selected.notes && (
                  <div className="flex gap-2">
                    <span className="w-28 shrink-0 opacity-50">{t("sites.notes")}</span>
                    <span className="min-w-0 break-words">{selected.notes}</span>
                  </div>
                )}
              </div>

              <ModelPicker
                site={selected}
                models={models}
                onAddManual={() => setManualOpen(true)}
                onFetch={() => handleFetchModels(selected)}
              />
            </div>
          )
        ) : (
          <Empty description={t("sites.emptyTitle")} />
        )}
      </div>

      <SiteFormModal
        open={formOpen}
        site={editing}
        initialValues={editing ? null : formInitial}
        forceAdvancedOpen={forceAdvancedOpen}
        onClose={() => {
          setFormOpen(false);
          setForceAdvancedOpen(false);
          setFormInitial(null);
        }}
        onSaved={handleSiteSaved}
      />
      <ManualModelModal open={manualOpen} site={selected} onClose={() => setManualOpen(false)} />
    </div>
  );
}
