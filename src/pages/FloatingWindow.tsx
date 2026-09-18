import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@/lib/invoke";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { invoke } from "@/lib/invoke";
import { useSiteStore } from "@/stores";
import type { AppSettings, SiteQuotaSummary } from "@/types/domain";
import {
  ReloadOutlined,
  CloseOutlined,
  DollarOutlined,
  DownOutlined,
  WarningOutlined,
} from "@ant-design/icons";
import { App, Button, Empty, Spin, Tooltip, Typography, theme } from "antd";

const { Text } = Typography;

/**
 * 给颜色套一个透明度上限。
 *
 * 注意是「上限」而不是「覆盖」：antd 在深色主题下很多 token 本身就是 rgba
 * （如 colorFillTertiary 约等于 rgba(255,255,255,0.08)），那一层低透明度正是
 * 「很淡的填充」这个设计意图。直接替换成 0.6 会让本该几乎看不见的填充变成
 * 一块明显的白 —— 卡片就会亮得刺眼。
 */
export function withAlpha(color: string, alpha: number): string {
  const c = color.trim();
  if (c.startsWith("#")) {
    const body = c.slice(1);
    const full =
      body.length === 3
        ? body
            .split("")
            .map((ch) => ch + ch)
            .join("")
        : body;
    const num = Number.parseInt(full.slice(0, 6), 16);
    if (Number.isNaN(num)) return c;
    return `rgba(${(num >> 16) & 255}, ${(num >> 8) & 255}, ${num & 255}, ${alpha})`;
  }
  const match = c.match(/^rgba?\(([^)]+)\)$/);
  if (match) {
    const parts = match[1].split(",").map((s) => s.trim());
    const original = parts.length > 3 ? Number(parts[3]) : 1;
    const final = Number.isNaN(original) ? alpha : Math.min(original, alpha);
    return `rgba(${parts[0]}, ${parts[1]}, ${parts[2]}, ${final})`;
  }
  return c;
}

/** 余额低于这个值（美元）就提示，避免用户用到一半才发现没钱了。 */
const LOW_BALANCE_USD = 5;

/** 收起态是一个小球，展开态是完整列表——对齐主流流量悬浮窗的交互。 */
const EXPANDED_WIDTH = 280;
const EXPANDED_HEIGHT = 400;
const ORB_SIZE = 56;

/** 位移超过这个像素数就算拖动、不算点击。 */
const DRAG_THRESHOLD = 4;

/**
 * 停止移动多久之后把窗口位置落盘。
 *
 * 拖动期间不能写库（每帧一次会打满数据库）；而系统拖动并没有一个可靠的「结束」
 * 回调——Windows 上 tao 是在 WM_EXITSIZEMOVE 里补发一个 WM_LBUTTONUP
 * （tao-0.35.3/src/platform_impl/windows/event_loop.rs:1038），别的平台不一定有。
 * 所以以「窗口不再移动」为准：最后一次 move 事件之后再等这么久才写一次。
 */
const POSITION_SAVE_DEBOUNCE_MS = 400;

/** 余额低（但不为 0）时用警示色；无限额或未知不提示。 */
function isLowBalance(quota: SiteQuotaSummary["quota"]): boolean {
  return (
    !!quota &&
    !quota.unlimited &&
    quota.remainingUsd !== null &&
    quota.remainingUsd !== undefined &&
    quota.remainingUsd > 0 &&
    quota.remainingUsd < LOW_BALANCE_USD
  );
}

function quotaColor(
  quota: SiteQuotaSummary["quota"],
  token: {
    colorSuccess: string;
    colorWarning: string;
    colorPrimary: string;
    colorTextTertiary: string;
  },
): string {
  if (!quota) return token.colorTextTertiary;
  if (quota.unlimited) return token.colorSuccess;
  if (isLowBalance(quota)) return token.colorWarning;
  if ((quota.remainingUsd ?? 0) > 0) return token.colorPrimary;
  return token.colorTextTertiary;
}

function formatQuota(
  quota: SiteQuotaSummary["quota"],
  t: (key: string) => string,
): string {
  if (!quota) return t("settings.floatingWindowUnavailable");
  if (quota.unlimited) return t("settings.floatingWindowUnlimited");
  if (quota.remainingUsd !== null && quota.remainingUsd !== undefined) {
    return `$${quota.remainingUsd.toFixed(2)}`;
  }
  if (
    quota.totalUsd !== null &&
    quota.totalUsd !== undefined &&
    quota.usedUsd !== null &&
    quota.usedUsd !== undefined
  ) {
    return `$${(quota.totalUsd - quota.usedUsd).toFixed(2)}`;
  }
  return t("settings.floatingWindowUnavailable");
}

export const FloatingWindow: React.FC = () => {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  /**
   * 窗口句柄要固定一份。
   *
   * `getCurrentWindow()` 每次调用都返回一个新的包装对象；在渲染期直接调用会让
   * `applyCollapsed` / 拖动监听这些以它为依赖的 effect 每次渲染都重挂，
   * 拖动过程中重挂监听会丢事件。窗口本身不会变，用 `useMemo(…, [])` 固定。
   */
  const appWindow = useMemo(() => getCurrentWindow(), []);
  const { message } = App.useApp();
  const refreshAllSites = useSiteStore((state) => state.refreshAllSites);
  const [sites, setSites] = useState<SiteQuotaSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [lastUpdate, setLastUpdate] = useState<Date | null>(null);
  const [collapsed, setCollapsed] = useState(false);

  /**
   * 拖动状态。
   *
   * 位置移动本身已经交给系统（`startDragging`）——以前在 `mousemove` 里
   * `setPosition` 是「JS→Rust→SetWindowPos」的每帧往返，而且目标位置由
   * 「按下原点 + 窗口相对位移」推导：窗口一移动，下一个事件的 `clientX` 就变小，
   * 形成反馈环，窗口永远追不上光标；光标移出窗口后 `mousemove` 也不再触发，
   * 拖动直接冻结。这里只保留两件必须由 JS 判断的事：越过阈值（算不算拖动）
   * 与拖完抑制点击。
   */
  const drag = useRef<{ mouseX: number; mouseY: number; moved: boolean } | null>(null);
  /**
   * 「刚刚拖动过」标志。
   *
   * 不能靠 `drag.current.moved` 在 click 里判断——mouseup 已经把它清空了，
   * 拖动之后 click 仍会触发，于是拖完窗口就自己收起/展开了。拖动一开始就立起来
   * （系统拖动期间可能收不到 mouseup，等 mouseup 再立就晚了），由
   * `suppressClickAfterDrag` 在 mouseup 时收尾。
   */
  const draggedRecently = useRef(false);
  /** 拖动结束后抑制 click 的兜底定时器，避免标志永久卡住。 */
  const dragClickTimer = useRef<number | null>(null);
  /** 已经落盘的位置：同一次拖动只写一次，也避免「没动也写库」。 */
  const lastSavedPosition = useRef<{ x: number; y: number } | null>(null);
  const positionSaveTimer = useRef<number | null>(null);

  /** 拖动结束：抑制紧随其后的 click。click 与 mouseup 在同一个任务里派发，0ms 足够。 */
  const suppressClickAfterDrag = useCallback(() => {
    draggedRecently.current = true;
    if (dragClickTimer.current !== null) window.clearTimeout(dragClickTimer.current);
    dragClickTimer.current = window.setTimeout(() => {
      draggedRecently.current = false;
      dragClickTimer.current = null;
    }, 0);
  }, []);

  /** 把窗口当前的真实位置落盘（位置变了才写）。 */
  const persistPosition = useCallback(
    async (x: number, y: number) => {
      if (lastSavedPosition.current?.x === x && lastSavedPosition.current?.y === y) return;
      lastSavedPosition.current = { x, y };
      try {
        await invoke("save_floating_window_position", { x, y });
      } catch (error) {
        console.error("Failed to save position:", error);
      }
    },
    [],
  );

  /** 拖动/移动停止后再落盘：拖动过程中不写库。 */
  const schedulePositionSave = useCallback(
    (x: number, y: number) => {
      if (positionSaveTimer.current !== null) window.clearTimeout(positionSaveTimer.current);
      positionSaveTimer.current = window.setTimeout(() => {
        positionSaveTimer.current = null;
        void persistPosition(x, y);
      }, POSITION_SAVE_DEBOUNCE_MS);
    },
    [persistPosition],
  );

  /** 读取统一刷新任务写入的余额缓存。 */
  const loadQuotaCache = useCallback(async () => {
    try {
      const data = await invoke<SiteQuotaSummary[]>("get_all_sites_quota");
      setSites(data);
      setLastUpdate(new Date());
    } catch (error) {
      console.error("Failed to load quota cache:", error);
      message.error(t("settings.floatingWindowFetchFailed"));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  /** 主窗口持有统一刷新任务，悬浮窗只请求它并读取后端缓存。 */
  const requestUnifiedRefresh = useCallback(async () => {
    setLoading(true);
    try {
      if (isTauri()) {
        const { emit } = await import("@tauri-apps/api/event");
        await emit("sites-refresh-requested");
      } else {
        await refreshAllSites();
        await loadQuotaCache();
      }
    } catch (error) {
      console.error("Failed to request unified refresh:", error);
      message.error(t("settings.floatingWindowFetchFailed"));
      setLoading(false);
    }
  }, [loadQuotaCache, message, refreshAllSites, t]);

  /** 展开/收起时同步窗口尺寸：收起是圆球，展开是面板。 */
  const applyCollapsed = useCallback(
    async (next: boolean) => {
      try {
        await appWindow.setSize(
          next
            ? new LogicalSize(ORB_SIZE, ORB_SIZE)
            : new LogicalSize(EXPANDED_WIDTH, EXPANDED_HEIGHT),
        );
      } catch (error) {
        console.error("Failed to resize floating window:", error);
      }
    },
    [appWindow],
  );

  // 首屏：先读缓存立刻出内容，再后台刷新一次。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [cached, settings] = await Promise.all([
          invoke<SiteQuotaSummary[]>("get_all_sites_quota"),
          invoke<AppSettings>("get_settings"),
        ]);
        if (cancelled) return;
        setSites(cached);
        if (cached.length > 0) setLastUpdate(new Date());
        const isCollapsed = settings.floatingWindow?.collapsed ?? false;
        setCollapsed(isCollapsed);
        await applyCollapsed(isCollapsed);
        setLoading(false);
      } catch (error) {
        console.error("Failed to load floating window state:", error);
        if (!cancelled) await loadQuotaCache();
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 统一刷新完成后读取后端余额缓存；定时器由主窗口 AppInner 唯一持有。
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen("sites-refresh-finished", () => {
      void loadQuotaCache();
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((error) => console.error("Failed to listen for site refresh:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadQuotaCache]);

  const persistCollapsed = useCallback(
    async (next: boolean) => {
      setCollapsed(next);
      await applyCollapsed(next);
      try {
        await invoke("set_floating_window_collapsed", { collapsed: next });
      } catch (error) {
        console.error("Failed to save collapsed state:", error);
      }
    },
    [applyCollapsed],
  );

  /** 小球与标题栏共用的拖动：按下记录起点，移动超过阈值就交给系统拖动。 */
  const beginDrag = useCallback((event: React.MouseEvent) => {
    if (event.button !== 0) return;
    if (event.target instanceof HTMLElement && event.target.closest("button")) return;
    drag.current = {
      mouseX: event.clientX,
      mouseY: event.clientY,
      moved: false,
    };
  }, []);

  // 拖动：只在越过阈值那一刻交给系统，之后不再自己算位置。
  useEffect(() => {
    const onMove = (event: MouseEvent) => {
      const origin = drag.current;
      if (!origin || origin.moved) return;
      // 这里只判断「算不算拖动」，不做坐标换算：位置交给系统，鼠标事件只用 CSS 像素。
      const dx = event.clientX - origin.mouseX;
      const dy = event.clientY - origin.mouseY;
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
      origin.moved = true;
      // 先立起抑制标志：拖完紧跟的 click 不能触发展开/收起。
      draggedRecently.current = true;
      // 交给系统拖动（wry/tao 走的是窗口的非客户区拖动），跟手、能拖出屏幕外，
      // 也不再需要每次 mousemove 一次 IPC。
      void appWindow.startDragging().catch((error) => {
        console.error("Failed to start window dragging:", error);
        draggedRecently.current = false;
      });
    };

    const onUp = () => {
      const origin = drag.current;
      if (!origin) return;
      drag.current = null;
      if (!origin.moved) return;
      // 拖动结束：抑制紧随其后的 click，并把最终位置落盘。
      suppressClickAfterDrag();
      // mouseup 读到的是权威位置，撤掉 move 事件的防抖落盘，避免它稍后用略旧的坐标覆盖。
      if (positionSaveTimer.current !== null) {
        window.clearTimeout(positionSaveTimer.current);
        positionSaveTimer.current = null;
      }
      void appWindow
        .outerPosition()
        .then((pos) => persistPosition(pos.x, pos.y))
        .catch((error) => {
          // 位置读不到就交给 move 事件的防抖落盘兜底，不打断交互。
          console.error("Failed to read window position after drag:", error);
        });
    };

    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
  }, [appWindow, persistPosition, suppressClickAfterDrag]);

  /**
   * 位置落盘的兜底通道。
   *
   * 系统拖动期间窗口每次移动都会发 `tauri://move`，用「最后一次移动 + 防抖」
   * 当作拖动结束——不依赖 mouseup（见 POSITION_SAVE_DEBOUNCE_MS 的说明）。
   * Windows 上 mouseup 通常会先到并即时落盘，这里随后再跑一次会被位置去重挡掉。
   */
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void appWindow
      .onMoved(({ payload }) => {
        schedulePositionSave(payload.x, payload.y);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((error) => console.error("Failed to listen for window moves:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [appWindow, schedulePositionSave]);

  // 组件卸载时清掉未落盘的定时器，避免卸载后还写库。
  useEffect(
    () => () => {
      if (positionSaveTimer.current !== null) window.clearTimeout(positionSaveTimer.current);
      if (dragClickTimer.current !== null) window.clearTimeout(dragClickTimer.current);
    },
    [],
  );

  /** 点击（而非拖动）时切换展开/收起。 */
  const handleOrbClick = () => {
    if (draggedRecently.current) {
      // 拖动后的那次 click：吞掉并复位，防止标志卡住让小球再也点不动。
      draggedRecently.current = false;
      return;
    }
    void persistCollapsed(!collapsed);
  };

  const handleClose = () => {
    // 「关闭」= 关掉这个功能；只 hide 的话下次启动它又冒出来，用户会以为关不掉。
    void invoke("set_floating_window_enabled", { enabled: false }).catch((error) => {
      console.error("Failed to close floating window:", error);
    });
  };

  const lowCount = useMemo(
    () => sites.filter((site) => site.enabled && isLowBalance(site.quota)).length,
    [sites],
  );

  /** 收起态的小球：一眼看出是否有站点余额偏低。 */
  const orb = (
    <div
      className="flex h-full w-full items-center justify-center"
      style={{
        background: withAlpha(
          lowCount > 0 ? token.colorWarning : token.colorPrimary,
          0.9,
        ),
        borderRadius: "50%",
        border: `1px solid ${withAlpha(token.colorBorderSecondary, 0.6)}`,
        cursor: "pointer",
        boxShadow: "0 4px 16px rgba(0, 0, 0, 0.28)",
      }}
      onMouseDown={beginDrag}
      onClick={handleOrbClick}
      role="button"
      tabIndex={0}
      aria-label={
        collapsed
          ? t("settings.floatingWindowExpand")
          : t("settings.floatingWindowCollapse")
      }
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") void persistCollapsed(!collapsed);
      }}
    >
      <DollarOutlined style={{ color: "#fff", fontSize: 20 }} />
    </div>
  );

  return (
    <div className="h-screen w-full" style={{ padding: collapsed ? 0 : 0 }}>
      {collapsed ? (
        orb
      ) : (
        <div
          data-testid="floating-panel"
          className="flex h-full w-full flex-col"
          style={{
            background: withAlpha(token.colorBgElevated, 0.72),
            borderRadius: 14,
            overflow: "hidden",
            border: `1px solid ${withAlpha(token.colorBorderSecondary, 0.7)}`,
            color: token.colorText,
          }}
        >
          {/* 标题栏：整条都是拖动区（按下拖动、松手不位移则视为点击收起） */}
          <div
            data-testid="floating-header"
            className="flex items-center justify-between px-3 py-2.5 border-b"
            style={{
              borderColor: withAlpha(token.colorBorderSecondary, 0.6),
              cursor: "move",
            }}
            onMouseDown={beginDrag}
            onClick={handleOrbClick}
            title={t("settings.floatingWindowCollapse")}
          >
            <div className="flex items-center gap-2">
              <DollarOutlined style={{ color: token.colorPrimary, fontSize: 16 }} />
              <Text strong style={{ color: token.colorText, fontSize: 13 }}>
                {t("settings.floatingWindowTitle")}
              </Text>
              {lowCount > 0 && (
                <Tooltip title={t("settings.floatingWindowLowBalance")}>
                  <WarningOutlined style={{ color: token.colorWarning, fontSize: 13 }} />
                </Tooltip>
              )}
            </div>
            <div className="flex items-center gap-1">
              <Tooltip title={t("common.refresh")}>
                <Button
                  type="text"
                  size="small"
                  aria-label={t("common.refresh")}
                  icon={<ReloadOutlined />}
                  onClick={(event) => {
                    event.stopPropagation();
                    void requestUnifiedRefresh();
                  }}
                  loading={loading}
                  style={{ color: token.colorTextTertiary }}
                />
              </Tooltip>
              <Tooltip title={t("settings.floatingWindowCollapse")}>
                <Button
                  type="text"
                  size="small"
                  aria-label={t("settings.floatingWindowCollapse")}
                  icon={<DownOutlined />}
                  onClick={(event) => {
                    event.stopPropagation();
                    void persistCollapsed(true);
                  }}
                  style={{ color: token.colorTextTertiary }}
                />
              </Tooltip>
              <Tooltip title={t("common.close")}>
                <Button
                  type="text"
                  size="small"
                  aria-label={t("common.close")}
                  icon={<CloseOutlined />}
                  onClick={(event) => {
                    event.stopPropagation();
                    handleClose();
                  }}
                  style={{ color: token.colorTextTertiary }}
                />
              </Tooltip>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto px-4 py-3">
            {loading && sites.length === 0 ? (
              <div className="flex items-center justify-center h-full">
                <Spin />
              </div>
            ) : sites.length === 0 ? (
              <Empty
                description={t("settings.floatingWindowNoSites")}
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                style={{ marginTop: 60 }}
              />
            ) : (
              <div className="space-y-1.5">
                {sites.map((site) => (
                  <div
                    key={site.siteId}
                    className="flex items-center justify-between rounded-lg px-3 py-2"
                    style={{
                      background: withAlpha(token.colorFillTertiary, 0.6),
                      border: `1px solid ${withAlpha(token.colorBorderSecondary, 0.5)}`,
                      opacity: site.enabled ? 1 : 0.45,
                    }}
                  >
                    <Text
                      style={{
                        color: token.colorText,
                        fontSize: 13,
                        maxWidth: 150,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                      title={site.siteName}
                    >
                      {site.siteName}
                    </Text>
                    <Text strong style={{ color: quotaColor(site.quota, token), fontSize: 13 }}>
                      {formatQuota(site.quota, t)}
                    </Text>
                    {/* 刷新失败时说明原因：否则用户只看到「不可用」，无从判断原因。 */}
                    {site.quota?.error && (
                      <Tooltip title={site.quota.error}>
                        <WarningOutlined
                          style={{ color: token.colorWarning, fontSize: 12, marginLeft: 4 }}
                        />
                      </Tooltip>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>

          {lastUpdate && (
            <div
              className="px-4 py-2 border-t text-center"
              style={{ borderColor: withAlpha(token.colorBorderSecondary, 0.6) }}
            >
              <Text style={{ color: token.colorTextTertiary, fontSize: 11 }}>
                {t("settings.floatingWindowLastUpdate")} {lastUpdate.toLocaleTimeString()}
              </Text>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
