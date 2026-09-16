import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert, App, Button, Space, Switch, Table, Tag, Typography, theme } from "antd";
import { InfoCircleOutlined, ReloadOutlined, VerticalAlignBottomOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "@/components/settings/SettingsGroup";
import { useProxyStore, useUIStore } from "@/stores";
import type { LocalProxyRequestLogEntry } from "@/types/proxy";
import type { TargetKind } from "@/types/domain";

const TARGETS: TargetKind[] = ["claude_code", "codex", "pi", "prime", "zcode"];

const TARGET_LABEL_KEYS: Record<TargetKind, string> = {
  claude_code: "proxy.targetClaudeCode",
  codex: "proxy.targetCodex",
  pi: "proxy.targetPi",
  prime: "proxy.targetPrime",
  zcode: "proxy.targetZCode",
};

/** invoke 抛出的错误对象形状（与 McpPage 一致）。 */
function errorText(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

function formatUptime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rest = s % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${rest}s`;
  return `${rest}s`;
}

/** 只展示口令的首尾，避免截图/录屏泄露完整凭据。 */
function maskToken(token: string): string {
  if (token.length <= 8) return "••••";
  return `${token.slice(0, 4)}…${token.slice(-4)}`;
}

/** 接管地址预览同样要遮口令：它和展示用的 token 是同一份本机凭据。 */
function maskBaseUrl(url: string, token: string): string {
  if (!token) return url;
  return url.replace(token, maskToken(token));
}

interface PortInputProps {
  port: number;
  label: string;
  hint: string;
  onCommit: (raw: string) => Promise<void>;
}

/**
 * 端口输入自己持有草稿 state：击键只重渲染这一个输入框，
 * 不会带着整页（含下方的请求日志表）重渲染。
 */
function PortInput({ port, label, hint, onCommit }: PortInputProps) {
  const { token } = theme.useToken();
  // null = 不在编辑中，显示服务端那份；失焦提交后清空草稿回落到服务端值。
  const [draft, setDraft] = useState<string | null>(null);

  const commit = async () => {
    if (draft === null) return;
    await onCommit(draft);
    setDraft(null);
  };

  return (
    <div className="flex items-center gap-2">
      <span className="text-sm">{label}</span>
      <input
        className="w-24 rounded px-2 py-1 text-sm"
        style={{
          border: `1px solid ${token.colorBorder}`,
          background: "transparent",
          color: token.colorText,
        }}
        type="number"
        min={1024}
        max={65535}
        value={draft ?? String(port)}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => void commit()}
      />
      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
        {hint}
      </Typography.Text>
    </div>
  );
}

export function ProxyPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message, modal } = App.useApp();
  // 按字段订阅：日志轮询每 4 秒写入 requests，不该顺带重渲染标题/接管面板。
  const status = useProxyStore((s) => s.status);
  const requests = useProxyStore((s) => s.requests);
  const loadStatus = useProxyStore((s) => s.loadStatus);
  const loadRequests = useProxyStore((s) => s.loadRequests);
  const start = useProxyStore((s) => s.start);
  const stop = useProxyStore((s) => s.stop);
  const setTakeover = useProxyStore((s) => s.setTakeover);
  const setPort = useProxyStore((s) => s.setPort);
  const clearRequests = useProxyStore((s) => s.clearRequests);
  const activePage = useUIStore((s) => s.activePage);

  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void loadStatus();
    void loadRequests();
  }, [loadStatus, loadRequests]);

  // 只在本页可见时轮询：KeepAlive 会让页面常驻，不判断当前页就会一直发请求。
  useEffect(() => {
    if (activePage !== "proxy") return;
    const tick = () => {
      if (document.visibilityState !== "visible") return;
      void loadStatus();
      void loadRequests();
    };
    const timer = window.setInterval(tick, 4000);
    document.addEventListener("visibilitychange", tick);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", tick);
    };
  }, [activePage, loadStatus, loadRequests]);

  const running = status?.running ?? false;

  const toggleRunning = useCallback(
    async (next: boolean) => {
      setBusy(true);
      try {
        if (next) {
          await start();
        } else {
          const engaged = (status?.targets ?? []).some((item) => item.takeover);
          await stop();
          if (engaged) {
            // 停止会顺带把接管目标改回直连，否则客户端会指着已关闭的端口。
            message.info(t("proxy.stopRestored"));
          }
        }
      } catch (error) {
        message.error(errorText(error));
      } finally {
        setBusy(false);
        void loadStatus();
      }
    },
    [start, stop, message, loadStatus, status?.targets, t],
  );

  const toggleTakeover = useCallback(
    async (target: TargetKind, enabled: boolean) => {
      try {
        await setTakeover(target, enabled);
        message.success(
          enabled
            ? t("proxy.takeoverEnabled", { target: t(TARGET_LABEL_KEYS[target]) })
            : t("proxy.takeoverDisabled", { target: t(TARGET_LABEL_KEYS[target]) }),
        );
      } catch (error) {
        message.error(errorText(error));
      }
    },
    [setTakeover, message, t],
  );

  const applyPort = useCallback(
    async (raw: string) => {
      const parsed = Number(raw);
      if (raw.trim() === "" || !Number.isFinite(parsed)) return;
      const next = Math.min(65535, Math.max(1024, Math.round(parsed)));
      if (next === status?.port) return;
      try {
        await setPort(next);
        message.success(t("proxy.portSaved"));
        await loadStatus();
      } catch (error) {
        message.error(errorText(error));
      }
    },
    [status?.port, setPort, message, t, loadStatus],
  );

  const confirmDisableTakeoverAll = useCallback(() => {
    const active = (status?.targets ?? []).filter((item) => item.takeover);
    if (active.length === 0) return;
    modal.confirm({
      centered: true,
      title: t("proxy.disableAllTitle"),
      content: t("proxy.disableAllBody"),
      okText: t("proxy.disableAllOk"),
      cancelText: t("common.cancel"),
      onOk: async () => {
        for (const item of active) {
          await setTakeover(item.target, false);
        }
        void loadStatus();
      },
    });
  }, [status?.targets, modal, t, setTakeover, loadStatus]);

  // 列定义只在语言变化时重建：轮询刷新日志不再让 Table 重新生成整套 columns。
  const columns = useMemo(
    () => [
    {
      title: t("proxy.colTime"),
      dataIndex: "at",
      width: 170,
      render: (value: number) => new Date(value).toLocaleString(),
    },
    {
      title: t("proxy.colTarget"),
      dataIndex: "target",
      width: 120,
      render: (value: string) => <Tag>{value}</Tag>,
    },
    { title: t("proxy.colMethod"), dataIndex: "method", width: 80 },
    {
      title: t("proxy.colPath"),
      dataIndex: "path",
      ellipsis: true,
      render: (value: string) => <code style={{ fontSize: 12 }}>{value}</code>,
    },
    {
      title: t("proxy.colStatus"),
      dataIndex: "status",
      width: 100,
      render: (value: number) => (
        <Tag color={value < 400 ? "green" : "red"}>{value}</Tag>
      ),
    },
    {
      title: t("proxy.colDuration"),
      dataIndex: "durationMs",
      width: 100,
      render: (value: number) => `${value} ms`,
    },
    {
      title: t("proxy.colError"),
      dataIndex: "error",
      ellipsis: true,
      render: (value: string | null) =>
        value ? <Typography.Text type="danger">{value}</Typography.Text> : null,
    },
    ],
    [t],
  );

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-6">
      <div className="flex items-center justify-between gap-4">
        <div>
          <Typography.Title level={4} style={{ margin: 0 }}>
            {t("proxy.title")}
          </Typography.Title>
          <Typography.Text type="secondary">{t("proxy.subtitle")}</Typography.Text>
        </div>
        <Space>
          <Button icon={<ReloadOutlined />} onClick={() => void toggleRunning(!running)}>
            {running ? t("proxy.stop") : t("proxy.start")}
          </Button>
        </Space>
      </div>

      <SettingsGroup
        title={t("proxy.groupService")}
        extra={
          <Switch
            checked={running}
            loading={busy}
            onChange={(next) => void toggleRunning(next)}
            checkedChildren={t("proxy.running")}
            unCheckedChildren={t("proxy.stopped")}
          />
        }
      >
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-x-8 gap-y-2 text-sm">
            <span>
              {t("proxy.address")}:{" "}
              <code>
                {status?.address ?? `127.0.0.1:${status?.port ?? 18087}`}
              </code>
            </span>
            <span>
              {t("proxy.pathToken")}:{" "}
              <code title={t("proxy.pathTokenHint")}>
                {status?.pathToken ? maskToken(status.pathToken) : "—"}
              </code>
            </span>
            <span>
              {t("proxy.uptime")}: {running ? formatUptime(status?.uptimeSeconds ?? 0) : "—"}
            </span>
            <span>
              {t("proxy.counters")}:{" "}
              {t("proxy.counterValue", {
                total: status?.totalRequests ?? 0,
                success: status?.successRequests ?? 0,
                failed: status?.failedRequests ?? 0,
                active: status?.activeConnections ?? 0,
              })}
            </span>
          </div>

          <PortInput
            port={status?.port ?? 18087}
            label={t("proxy.port")}
            hint={t("proxy.portHint")}
            onCommit={applyPort}
          />

          {status?.lastError && (
            <Alert type="error" showIcon message={t("proxy.lastError")} description={status.lastError} />
          )}
          {!running && (
            <Alert type="info" showIcon message={t("proxy.stoppedHint")} />
          )}
        </div>
      </SettingsGroup>

      <SettingsGroup
        title={t("proxy.groupTakeover")}
        extra={
          <Button size="small" danger onClick={confirmDisableTakeoverAll}>
            {t("proxy.disableAll")}
          </Button>
        }
      >
        <div className="flex flex-col gap-3">
          <Alert
            type="warning"
            showIcon
            message={t("proxy.takeoverHint")}
            description={t("proxy.takeoverHintDetail")}
          />
          {(status?.targets ?? TARGETS.map((target) => ({
            target,
            takeover: false,
            siteId: null,
            siteName: null,
            clientBaseUrl: null,
          }))).map((item) => (
            <div
              key={item.target}
              className="flex items-center justify-between gap-4"
              style={{ borderBottom: `1px solid ${token.colorBorderSecondary}`, paddingBottom: 8 }}
            >
              <div className="min-w-0">
                <div className="text-sm">{t(TARGET_LABEL_KEYS[item.target])}</div>
                <div className="text-xs" style={{ color: token.colorTextSecondary }}>
                  {item.siteName
                    ? t("proxy.boundSite", { site: item.siteName })
                    : t("proxy.noBoundSite")}
                  {item.takeover && item.clientBaseUrl
                    ? ` · ${maskBaseUrl(item.clientBaseUrl, status?.pathToken ?? "")}`
                    : ""}
                </div>
              </div>
              <Switch
                checked={item.takeover}
                disabled={!running && !item.takeover}
                onChange={(next) => void toggleTakeover(item.target, next)}
              />
            </div>
          ))}
          {!running && (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              <InfoCircleOutlined /> {t("proxy.takeoverNeedsRunning")}
            </Typography.Text>
          )}
        </div>
      </SettingsGroup>

      <SettingsGroup
        title={t("proxy.groupLog")}
        extra={
          <Space>
            <Button size="small" icon={<ReloadOutlined />} onClick={() => void loadRequests()}>
              {t("proxy.refresh")}
            </Button>
            <Button size="small" danger icon={<VerticalAlignBottomOutlined />} onClick={() => void clearRequests()}>
              {t("proxy.clear")}
            </Button>
          </Space>
        }
      >
        <div className="flex flex-col gap-2">
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("proxy.logPrivacy")}
          </Typography.Text>
          <Table<LocalProxyRequestLogEntry>
            size="small"
            rowKey="id"
            pagination={false}
            scroll={{ y: 260 }}
            dataSource={requests}
            columns={columns}
            locale={{ emptyText: t("proxy.noRequests") }}
          />
        </div>
      </SettingsGroup>
    </div>
  );
}
