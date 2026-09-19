import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, Divider, Input, InputNumber, Select, Switch, theme, App } from "antd";
import { CircleDot, ExternalLink, Github, RefreshCw, Tag } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettingsStore, useUIStore } from "@/stores";
import type { SettingsSection } from "@/stores/uiStore";
import { getAppVersion, PACKAGE_VERSION } from "@/lib/appVersion";
import {
  GITHUB_ISSUES_URL,
  GITHUB_RELEASES_URL,
  GITHUB_REPO_URL,
} from "@/lib/constants";
import { invoke, isAppError, isTauri } from "@/lib/invoke";
import { openExternalUrl } from "@/lib/openUrl";
import type { AppPaths, AppSettings, ProxyMode, ProxyProtocol } from "@/types/domain";
import { SettingsSidebar } from "@/components/settings/SettingsSidebar";

/**
 * 通知悬浮窗设置变了。
 *
 * 悬浮窗是独立 webview，拿不到这里的 store；跨窗口只能走 Tauri 事件。
 * 非 Tauri 环境（浏览器 dev / 测试）没有事件系统，静默跳过。
 */
function notifyFloatingSettingsChanged() {
  if (!isTauri()) return;
  void import("@tauri-apps/api/event")
    .then(({ emit }) => emit("floating-settings-changed"))
    .catch(() => undefined);
}
import { SettingsGroup } from "@/components/settings/SettingsGroup";
import { BackupCenter } from "@/components/settings/BackupCenter";
import { useUpdateCheckBusy, useUpdateChecker } from "@/hooks/useUpdateChecker";
import appIconUrl from "../../assets/brand/app-icon-1024.png?url";

const rowStyle: React.CSSProperties = { padding: "4px 0" };

/** 输入框原始文本 → 草稿数字：空 / 非法文本 → null。 */
function parseNumberDraft(raw: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

/** 提交时的规约：空 / 非法 → null，否则取整并夹进 [min, max]。 */
function clampNumberDraft(raw: number | null, min: number, max: number): number | null {
  if (raw == null || !Number.isFinite(raw)) return null;
  return Math.min(max, Math.max(min, Math.round(raw)));
}

interface NumberDraftInputProps {
  value: number | null;
  min: number;
  max: number;
  step?: number;
  size?: "small" | "middle" | "large";
  style?: React.CSSProperties;
  disabled?: boolean;
  addonAfter?: React.ReactNode;
  /** 提交（失焦 / Enter / 卸载）时回调，入参已取整并夹进 [min, max]。 */
  onCommit: (value: number) => void;
}

/**
 * 数字设置输入框：本地草稿 + 失焦 / Enter 提交。
 *
 * 为什么不像开关那样即时提交（AGENTS.md 的偏好是能即时就即时，这里是明确的例外）：
 * 1) 值直接取自服务端回显时，每击键一次 `onChange` → `save_settings`，输入 `18087` 就是 5 次写库；
 * 2) 回显值是 clamp 过的（min/max + 取整），会盖掉正在输入的内容——想输 `1440`，刚敲下 `1`
 *    就被回填成 `1`，光标也回到末尾。
 *
 * 所以连续输入（数字/文本）先在组件内走草稿，提交时机是**失焦、Enter 或组件卸载**
 * （卸载即切分区 / 退出设置页，提交是为了不把已经输入的内容悄悄丢掉）。
 * 编辑期间不接受外部回显：`editingRef` 为真时跳过 `value → draft` 同步。
 * 开关、下拉这类离散控件仍然即时提交。
 */
function NumberDraftInput({ value, min, max, onCommit, ...rest }: NumberDraftInputProps) {
  const [draft, setDraft] = useState<number | null>(value);
  // 是否处于"正在编辑"：聚焦或刚键入过即为真，期间不接受外部回显。
  const editingRef = useRef(false);
  // 最近一次键入的原始文本（未提交）。用原始文本而不是 rc 解析后的值：超出 min/max 的
  // 输入 rc 在键入期间不会回调 onChange，只认 onChange 会把这些输入整段丢掉。
  const typedRef = useRef<string | null>(null);
  // 提交发生在事件回调与卸载清理里，统一从 ref 取最新值，避免闭包捕获旧 state。
  const latestRef = useRef({ value, min, max, onCommit });
  useEffect(() => {
    latestRef.current = { value, min, max, onCommit };
  }, [value, min, max, onCommit]);

  // 外部值变化（别处保存后回读、切换页面回来）只在没有编辑时同步到草稿。
  useEffect(() => {
    if (!editingRef.current) setDraft(value);
  }, [value]);

  /** 落盘：与已保存值相同就只把显示对齐到 clamp 结果，不做无谓的写库。 */
  const emitIfChanged = useCallback((next: number | null) => {
    if (next == null) return;
    setDraft(next);
    const { value: current, onCommit: emit } = latestRef.current;
    if (next !== current) emit(next);
  }, []);

  /** 失焦 / Enter / 卸载时的提交：只提交真正被键入过的文本。 */
  const commit = useCallback(() => {
    const typed = typedRef.current;
    typedRef.current = null;
    editingRef.current = false;
    if (typed == null) return;
    const { min: lo, max: hi } = latestRef.current;
    emitIfChanged(clampNumberDraft(parseNumberDraft(typed), lo, hi));
  }, [emitIfChanged]);

  /** 步进按钮 / 上下方向键是离散操作，直接提交（与开关的即时提交一致）。 */
  const handleStep = useCallback(
    (next: number) => {
      typedRef.current = null;
      editingRef.current = false;
      const { min: lo, max: hi } = latestRef.current;
      emitIfChanged(clampNumberDraft(next, lo, hi));
    },
    [emitIfChanged],
  );

  const handleInput = useCallback((raw: string) => {
    typedRef.current = raw;
    editingRef.current = true;
    const parsed = parseNumberDraft(raw);
    // 空 / 非法文本不动草稿：DOM 已经显示用户输入，失焦时由 rc-input-number 自己还原；
    // 把草稿置空反而会在中文输入法组合期间把输入框清掉。
    if (parsed != null) setDraft(parsed);
  }, []);

  const handleChange = useCallback((next: number | string | null) => {
    // rc 在失焦 / 步进后会回传它自己钳过的值：只同步显示，不当作用户输入
    // （否则卸载时会重复提交一次）。
    if (typeof next === "number") setDraft(next);
  }, []);

  // 卸载时把未提交的输入落盘：以前是每击键都保存，不落盘会让"输入后按 Esc 退出/切页"丢改动。
  useEffect(
    () => () => {
      if (editingRef.current) commit();
    },
    [commit],
  );

  return (
    <InputNumber
      {...rest}
      min={min}
      max={max}
      value={draft}
      onChange={handleChange}
      onInput={handleInput}
      onStep={(next) => handleStep(Number(next))}
      onFocus={() => {
        editingRef.current = true;
      }}
      onBlur={commit}
      onPressEnter={commit}
    />
  );
}

function GeneralSection() {
  const { t, i18n } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);

  const patch = async (partial: Partial<AppSettings>) => {
    try {
      await saveSettings(partial);
      if (partial.language) await i18n.changeLanguage(partial.language);
      if (typeof partial.alwaysOnTop === "boolean") {
        await invoke("set_always_on_top", { enabled: partial.alwaysOnTop }).catch(() => null);
      }
    } catch (e) {
      if (isAppError(e) && e.code === "autostart_failed") {
        message.error(t("settings.autoStartFailed"));
        return;
      }
      message.error(isAppError(e) ? e.message : t("settings.saveFailed"));
    }
  };

  return (
    <div className="p-6 pb-12">
      <SettingsGroup title={t("settings.groupAppearance")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.language")}</span>
          <Select
            size="small"
            style={{ minWidth: 140 }}
            value={settings.language}
            onChange={(language) => void patch({ language })}
            options={[
              { value: "zh-CN", label: "简体中文" },
              { value: "en-US", label: "English" },
            ]}
          />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.theme")}</span>
          <Select
            size="small"
            style={{ minWidth: 140 }}
            value={settings.themeMode}
            onChange={(themeMode) => void patch({ themeMode })}
            options={[
              { value: "system", label: t("settings.themeSystem") },
              { value: "light", label: t("settings.themeLight") },
              { value: "dark", label: t("settings.themeDark") },
            ]}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("settings.groupWindow")}>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("settings.alwaysOnTop")}</span>
          <Switch checked={settings.alwaysOnTop} onChange={(alwaysOnTop) => void patch({ alwaysOnTop })} />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("settings.autoStart")}</span>
          <Switch checked={settings.autoStart} onChange={(autoStart) => void patch({ autoStart })} />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div>{t("settings.closeToTray")}</div>
            <div className="text-xs" style={{ color: token.colorTextSecondary }}>
              {t("settings.closeToTrayHint")}
            </div>
          </div>
          <Switch
            checked={settings.closeToTray}
            onChange={(closeToTray) => void patch({ closeToTray })}
          />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div>{t("settings.startInTray")}</div>
            <div className="text-xs" style={{ color: token.colorTextSecondary }}>
              {settings.closeToTray ? t("settings.startInTrayHint") : t("settings.startInTrayDisabled")}
            </div>
          </div>
          <Switch
            checked={settings.startInTray}
            disabled={!settings.closeToTray}
            onChange={(startInTray) => void patch({ startInTray })}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("settings.groupApply")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div>
            <div>{t("settings.forceExclusiveAuth")}</div>
            <div style={{ color: "var(--color-text-secondary)", fontSize: 12 }}>
              {t("settings.forceExclusiveAuthHint")}
            </div>
          </div>
          <Switch
            checked={settings.forceExclusiveClaudeAuthKey}
            onChange={(forceExclusiveClaudeAuthKey) => void patch({ forceExclusiveClaudeAuthKey })}
          />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.codexEnvMode")}</span>
          <Select
            size="small"
            style={{ minWidth: 180 }}
            value={settings.codexEnvInjectMode}
            onChange={(codexEnvInjectMode) => void patch({ codexEnvInjectMode })}
            options={[
              { value: "auto", label: t("settings.codexEnvAuto") },
              { value: "shell_rc", label: t("settings.codexEnvShell") },
              { value: "user_env", label: t("settings.codexEnvUser") },
              { value: "file_only", label: t("settings.codexEnvFile") },
            ]}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("settings.groupFloatingWindow")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div>{t("settings.floatingWindowEnabled")}</div>
            <div className="text-xs" style={{ color: token.colorTextSecondary }}>
              {t("settings.floatingWindowEnabledHint")}
            </div>
          </div>
          <Switch
            checked={settings.floatingWindow?.enabled ?? true}
            onChange={(enabled) => {
              void patch({
                floatingWindow: {
                  enabled,
                  autoRefreshMinutes: settings.floatingWindow?.autoRefreshMinutes ?? 5,
                  positionX: settings.floatingWindow?.positionX ?? 100,
                  positionY: settings.floatingWindow?.positionY ?? 100,
                  collapsed: settings.floatingWindow?.collapsed ?? false,
                },
              });
              if (enabled) {
                invoke("toggle_floating_window", {}).catch((e) => {
                  message.error(t("settings.floatingWindowOpenFailed"));
                  console.error(e);
                });
              }            }}
          />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.floatingWindowRefreshInterval")}</span>
          <NumberDraftInput
            size="small"
            min={1}
            max={60}
            step={1}
            style={{ width: 120 }}
            value={settings.floatingWindow?.autoRefreshMinutes ?? 5}
            onCommit={(autoRefreshMinutes) => {
              void patch({
                floatingWindow: {
                  enabled: settings.floatingWindow?.enabled ?? true,
                  autoRefreshMinutes,
                  positionX: settings.floatingWindow?.positionX ?? 100,
                  positionY: settings.floatingWindow?.positionY ?? 100,
                  collapsed: settings.floatingWindow?.collapsed ?? false,
                },
              });
              // 让已打开的悬浮窗立刻按新间隔刷新，不必重开。
              notifyFloatingSettingsChanged();
            }}
            addonAfter={t("settings.minutes")}
          />
        </div>
      </SettingsGroup>
    </div>
  );
}

function NetworkSection() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);
  const [host, setHost] = useState(settings.proxyHost ?? "");

  useEffect(() => {
    setHost(settings.proxyHost ?? "");
  }, [settings.proxyHost]);

  const patch = async (partial: Partial<AppSettings>) => {
    await saveSettings(partial);
  };

  const saveHost = () => {
    const next = host.trim();
    if (next === (settings.proxyHost ?? "")) return;
    if (!next) {
      void patch({ proxyHost: null });
      return;
    }
    void patch({ proxyHost: next });
  };

  return (
    <div className="p-6 pb-12">
      <SettingsGroup title={t("settings.groupProxy")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div>{t("settings.proxyMode")}</div>
            {settings.proxyMode === "system" && (
              <div className="text-xs" style={{ color: token.colorTextSecondary }}>
                {t("settings.proxySystemHint")}
              </div>
            )}
          </div>
          <Select
            size="small"
            style={{ minWidth: 160 }}
            value={settings.proxyMode}
            onChange={(proxyMode: ProxyMode) => void patch({ proxyMode })}
            options={[
              { value: "system", label: t("settings.proxySystem") },
              { value: "none", label: t("settings.proxyNone") },
              { value: "custom", label: t("settings.proxyCustom") },
            ]}
          />
        </div>
        {settings.proxyMode === "custom" && (
          <>
            <Divider style={{ margin: "8px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between gap-4">
              <span>{t("settings.proxyProtocol")}</span>
              <Select
                size="small"
                style={{ minWidth: 160 }}
                value={settings.proxyProtocol}
                onChange={(proxyProtocol: ProxyProtocol) => void patch({ proxyProtocol })}
                options={[
                  { value: "http", label: "HTTP" },
                  { value: "https", label: "HTTPS" },
                  { value: "socks5", label: "SOCKS5" },
                ]}
              />
            </div>
            <Divider style={{ margin: "8px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between gap-4">
              <span>{t("settings.proxyHost")}</span>
              <Input
                size="small"
                style={{ width: 220 }}
                value={host}
                allowClear
                placeholder="127.0.0.1"
                onChange={(e) => setHost(e.target.value)}
                onBlur={saveHost}
                onPressEnter={saveHost}
              />
            </div>
            <Divider style={{ margin: "8px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between gap-4">
              <span>{t("settings.proxyPort")}</span>
              <NumberDraftInput
                size="small"
                min={1}
                max={65535}
                style={{ width: 160 }}
                value={settings.proxyPort}
                onCommit={(proxyPort) => void patch({ proxyPort })}
              />
            </div>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.groupProbe")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div>{t("settings.probeTtl")}</div>
            <div className="text-xs" style={{ color: token.colorTextSecondary }}>
              {t("settings.probeTtlHint")}
            </div>
          </div>
          <NumberDraftInput
            min={1}
            max={1440}
            style={{ width: 140 }}
            value={settings.routeProbeTtlMinutes}
            addonAfter={t("settings.probeTtlUnit")}
            onCommit={(routeProbeTtlMinutes) => void patch({ routeProbeTtlMinutes })}
          />
        </div>
      </SettingsGroup>
    </div>
  );
}

function PathsSection() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);
  const [claude, setClaude] = useState(settings.claudeHomeOverride ?? "");
  const [codex, setCodex] = useState(settings.codexHomeOverride ?? "");
  const [pi, setPi] = useState(settings.piAgentDirOverride ?? "");
  const [prime, setPrime] = useState(settings.primeAgentDirOverride ?? "");

  useEffect(() => {
    setClaude(settings.claudeHomeOverride ?? "");
    setCodex(settings.codexHomeOverride ?? "");
    setPi(settings.piAgentDirOverride ?? "");
    setPrime(settings.primeAgentDirOverride ?? "");
  }, [
    settings.claudeHomeOverride,
    settings.codexHomeOverride,
    settings.piAgentDirOverride,
    settings.primeAgentDirOverride,
  ]);

  const onSave = async () => {
    await saveSettings({
      claudeHomeOverride: claude.trim() || null,
      codexHomeOverride: codex.trim() || null,
      piAgentDirOverride: pi.trim() || null,
      primeAgentDirOverride: prime.trim() || null,
    });
    message.success(t("settings.pathsSaved"));
  };

  return (
    <div className="p-6 pb-12">
      <SettingsGroup title={t("settings.paths")}>
        <div className="mb-3">
          <div className="mb-1 text-sm">{t("settings.claudeHome")}</div>
          <Input
            value={claude}
            onChange={(e) => setClaude(e.target.value)}
            placeholder={t("settings.pathPlaceholder")}
          />
        </div>
        <div className="mb-3">
          <div className="mb-1 text-sm">{t("settings.piAgentDir")}</div>
          <Input
            value={pi}
            onChange={(event) => setPi(event.target.value)}
            placeholder={t("settings.piAgentDirPlaceholder")}
          />
          <div className="mt-1 text-xs opacity-50">{t("settings.piAgentDirHint")}</div>
        </div>
        <div className="mb-3">
          <div className="mb-1 text-sm">{t("settings.primeAgentDir")}</div>
          <Input
            value={prime}
            onChange={(event) => setPrime(event.target.value)}
            placeholder={t("settings.primeAgentDirPlaceholder")}
          />
          <div className="mt-1 text-xs opacity-50">{t("settings.primeAgentDirHint")}</div>
        </div>
        <div className="mb-3">
          <div className="mb-1 text-sm">{t("settings.codexHome")}</div>
          <Input
            value={codex}
            onChange={(e) => setCodex(e.target.value)}
            placeholder={t("settings.pathPlaceholder")}
          />
        </div>
        <Button type="primary" onClick={() => void onSave()}>
          {t("settings.save")}
        </Button>
      </SettingsGroup>
    </div>
  );
}

function BackupSection() {
  return <BackupCenter />;
}

function displayUrl(url: string): string {
  return url.replace(/^https:\/\//, "");
}

function AboutLinkRow({
  icon,
  label,
  url,
}: {
  icon: React.ReactNode;
  label: string;
  url: string;
}) {
  const { token } = theme.useToken();
  const [hovered, setHovered] = useState(false);

  return (
    <button
      type="button"
      onClick={() => void openExternalUrl(url)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className="flex w-full items-center justify-between gap-3"
      style={{
        margin: "0 -8px",
        padding: "8px",
        border: "none",
        borderRadius: 8,
        background: hovered ? token.colorFillTertiary : "transparent",
        cursor: "pointer",
        color: token.colorText,
        textAlign: "left",
      }}
    >
      <span className="flex min-w-0 items-center gap-2.5">
        <span className="inline-flex shrink-0" style={{ color: token.colorTextSecondary }}>
          {icon}
        </span>
        <span className="min-w-0">
          <span className="block">{label}</span>
          <span className="block truncate text-xs" style={{ color: token.colorTextTertiary }}>
            {displayUrl(url)}
          </span>
        </span>
      </span>
      <ExternalLink size={14} className="shrink-0" style={{ color: token.colorTextQuaternary }} />
    </button>
  );
}

function AboutSection() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { checkForUpdate } = useUpdateChecker();
  const checking = useUpdateCheckBusy();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);
  const [paths, setPaths] = useState<AppPaths | null>(null);
  const [version, setVersion] = useState(PACKAGE_VERSION);

  useEffect(() => {
    void invoke<AppPaths>("get_app_paths")
      .then(setPaths)
      .catch(() => null);
    void getAppVersion()
      .then(setVersion)
      .catch(() => null);
  }, []);

  const handleCheckUpdate = () => {
    void checkForUpdate();
  };

  const patch = async (partial: Partial<AppSettings>) => {
    await saveSettings(partial);
  };

  return (
    <div className="p-6 pb-12">
      <SettingsGroup>
        <div className="flex flex-col items-center py-3 text-center">
          <img src={appIconUrl} alt={t("app.name")} width={88} height={88} draggable={false} />
          <div className="mt-3 text-lg font-semibold" style={{ color: token.colorText }}>
            {t("app.name")}
          </div>
          <div className="mt-1 text-sm" style={{ color: token.colorTextSecondary }}>
            {t("app.tagline")}
          </div>
          <div className="mt-2 text-xs" style={{ color: token.colorTextTertiary }}>
            {t("settings.version")} <span>{version}</span>
          </div>
        </div>
      </SettingsGroup>
      <SettingsGroup title={t("settings.groupUpdate")}>
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.checkUpdate")}</span>
          <Button
            icon={<RefreshCw size={16} />}
            loading={checking}
            disabled={checking}
            onClick={handleCheckUpdate}
          >
            {checking ? t("settings.checkingUpdate") : t("settings.checkUpdate")}
          </Button>
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.autoCheckUpdate")}</span>
          <Switch
            checked={settings.autoCheckUpdate}
            onChange={(autoCheckUpdate) => void patch({ autoCheckUpdate })}
          />
        </div>
        <Divider style={{ margin: "8px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between gap-4">
          <span>{t("settings.updateCheckInterval")}</span>
          <NumberDraftInput
            min={1}
            max={1440}
            style={{ width: 140 }}
            value={settings.updateCheckInterval}
            disabled={!settings.autoCheckUpdate}
            addonAfter={t("settings.minutes")}
            onCommit={(updateCheckInterval) => void patch({ updateCheckInterval })}
          />
        </div>
      </SettingsGroup>
      <SettingsGroup title={t("settings.linksTitle")}>
        <AboutLinkRow icon={<Github size={16} />} label={t("settings.githubRepo")} url={GITHUB_REPO_URL} />
        <Divider style={{ margin: "4px 0" }} />
        <AboutLinkRow icon={<CircleDot size={16} />} label={t("settings.githubIssues")} url={GITHUB_ISSUES_URL} />
        <Divider style={{ margin: "4px 0" }} />
        <AboutLinkRow icon={<Tag size={16} />} label={t("settings.githubReleases")} url={GITHUB_RELEASES_URL} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.securityTitle")}>
        <p className="m-0 text-sm" style={{ color: token.colorTextSecondary }}>
          {t("settings.securityBody")}
        </p>
      </SettingsGroup>
      {paths && (
        <SettingsGroup title={t("settings.plaintextPaths")}>
          <ul className="m-0 list-disc space-y-1 pl-5 font-mono text-xs">
            <li>{paths.appDir}</li>
            <li>{paths.codexEnvPath}</li>
            <li>~/.claude/settings.json</li>
            <li>~/.claude.json</li>
            <li>~/.codex/config.toml</li>
            <li>~/.pi/agent/auth.json</li>
            <li>~/.pi/agent/mcp.json</li>
            <li>~/.prime/agent/auth.json</li>
            <li>~/.prime/agent/settings.json</li>
          </ul>
          <Button className="mt-3" onClick={() => void invoke("open_path", { path: paths.appDir })}>
            {t("settings.openAppDir")}
          </Button>
        </SettingsGroup>
      )}
    </div>
  );
}

const SECTION_COMPONENTS: Record<SettingsSection, React.ComponentType> = {
  general: GeneralSection,
  network: NetworkSection,
  paths: PathsSection,
  backup: BackupSection,
  about: AboutSection,
};

export function SettingsPage() {
  const { token } = theme.useToken();
  const settingsTab = useUIStore((s) => s.settingsTab);
  const setPage = useUIStore((s) => s.setPage);
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const ContentComponent = SECTION_COMPONENTS[settingsTab];
  const contentScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void fetchSettings();
  }, [fetchSettings]);

  useLayoutEffect(() => {
    const el = contentScrollRef.current;
    if (!el) return;
    el.scrollTop = 0;
    el.scrollLeft = 0;
  }, [settingsTab]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setPage("sites");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPage]);

  return (
    <div className="flex h-full min-h-0">
      <div
        className="h-full w-56 shrink-0"
        style={{ borderRight: "1px solid var(--border-color)", backgroundColor: token.colorBgContainer }}
      >
        <SettingsSidebar />
      </div>
      <div
        ref={contentScrollRef}
        className="min-w-0 flex-1 overflow-y-auto"
        style={{ backgroundColor: token.colorBgElevated }}
      >
        <ContentComponent />
      </div>
    </div>
  );
}
