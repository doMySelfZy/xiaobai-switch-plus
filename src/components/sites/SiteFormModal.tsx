import { useEffect, useRef, useState } from "react";
import { App, Button, Collapse, Divider, Form, Input, Modal, Select, Typography, theme } from "antd";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { NewApiAccessProbe, ProtocolDetectionResult, Site, SiteCapabilities, SiteProtocol } from "@/types/domain";
import { invoke, isAppError } from "@/lib/invoke";
import { useSiteStore } from "@/stores";
import { UrlWritePreviewIcon } from "./UrlWritePreview";
import { ApiKeyListInput, loadSiteKeyDrafts, normalizeApiKeyDrafts } from "./ApiKeyListInput";
import { BaseUrlListInput } from "./BaseUrlListInput";
import { invalidateSiteIconCache } from "@/lib/siteIcon";
import { siteApiKeys } from "@/lib/siteApiKey";
import { normalizeBaseUrls, siteBaseUrls } from "@/lib/urlNormalize";
import { formatQuotaAmountLocalized } from "@/lib/quotaProbe";
import {
  anyCodexCapabilityOn,
  capabilitiesFromCodexFlags,
  codexFlagsFromCapabilities,
  EMPTY_CODEX_FLAGS,
  mergeCodexCapabilities,
  type CodexCapabilityFlags,
} from "@/lib/siteCapabilities";
import { CodexCapabilitySwitchList } from "@/components/apply/CodexCapabilitySwitchList";
import { ProxyHeaderEditor, parseProxyHeadersJson } from "./ProxyHeaderEditor";
import type { ProxyHeader } from "@/types/proxy";

function toActiveKeys(keys: string | string[]): string[] {
  return Array.isArray(keys) ? keys.map(String) : [String(keys)];
}

const { Text } = Typography;

function shouldOpenAdvanced(protocol?: SiteProtocol | null, notes?: string | null) {
  return protocol === "anthropic" || Boolean(notes?.trim());
}

/**
 * 连接测试失败的语义分类。后端可能用错误码（`unauthorized` / `network` / `timeout`…）
 * 或聚合错误码表达；分类只影响文案，不改行为。
 */
type ProtocolTestFailureKind = "unauthorized" | "unreachable" | "endpoint" | "unknown";

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: string } | null)?.message;
  return message ?? String(error);
}

/**
 * 「认证被拒」和「连不上 / 超时」要分开：前者让用户换 Key，后者让用户查地址与网络。
 * 只有前者才值得去换鉴权方式。后端若把多次尝试聚合成一个错误码，就退回错误文案里
 * 的 HTTP 401/403 判断（与 ModelPicker 对 HTTP 状态的处理同一口径）。
 */
function protocolTestFailureKind(error: unknown): ProtocolTestFailureKind {
  const code = isAppError(error) ? error.code : "";
  if (code === "unauthorized") return "unauthorized";
  if (code === "network" || code === "timeout" || code === "ssl") return "unreachable";
  if (code === "not_found" || code === "invalid_response") return "endpoint";
  const text = errorText(error);
  if (/\b40[13]\b/.test(text) || /unauthorized/i.test(text)) return "unauthorized";
  if (/timeout|timed out/i.test(text)) return "unreachable";
  return "unknown";
}

const PROTOCOL_TEST_FAILURE_KEYS: Record<ProtocolTestFailureKind, string> = {
  unauthorized: "sites.protocolTestUnauthorized",
  unreachable: "sites.protocolTestUnreachable",
  endpoint: "sites.protocolTestEndpointUnrecognized",
  unknown: "sites.protocolTestFailedDetail",
};

/**
 * 检测期间的分步提示。只让这一小块每秒重渲染，表单其余部分不受计时器影响。
 * 文案保持与后端语义一致但不耦合实现：并行化后不再是「第 n/5 种」，只说在试什么。
 */
function ProtocolTestProgress() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    const timer = window.setInterval(() => setSeconds((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <span
      role="status"
      className="inline-flex flex-wrap items-center gap-x-2 text-xs"
      style={{ color: token.colorTextSecondary }}
    >
      <span>{seconds >= 5 ? t("sites.protocolTestingSlow") : t("sites.protocolTestingHint")}</span>
      <span className="tabular-nums">
        {t("sites.protocolTestingElapsed", { seconds })}
      </span>
    </span>
  );
}

export interface SiteFormInitialValues {
  name?: string;
  baseUrls?: string[];
  apiKey?: string | null;
  protocol?: SiteProtocol;
  notes?: string | null;
  capabilities?: SiteCapabilities;
}

interface Props {
  open: boolean;
  site?: Site | null;
  initialValues?: SiteFormInitialValues | null;
  /** 打开时强制展开高级配置（如从额度提示跳入）。 */
  forceAdvancedOpen?: boolean;
  onClose: () => void;
  onSaved?: (site: Site, isCreate: boolean) => void | Promise<void>;
}

export function SiteFormModal({ open, site, initialValues, forceAdvancedOpen, onClose, onSaved }: Props) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const getSiteApiKey = useSiteStore((s) => s.getSiteApiKey);
  const createSite = useSiteStore((s) => s.createSite);
  const updateSite = useSiteStore((s) => s.updateSite);
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const [keyLoading, setKeyLoading] = useState(false);
  const [codexFlags, setCodexFlags] = useState<CodexCapabilityFlags>(EMPTY_CODEX_FLAGS);
  const [advancedOpen, setAdvancedOpen] = useState<string[]>([]);
  const [capOpen, setCapOpen] = useState<string[]>([]);
  const [newapiTokenLoadFailed, setNewapiTokenLoadFailed] = useState(false);
  const [newapiTesting, setNewapiTesting] = useState(false);
  const [proxyHeadersJson, setProxyHeadersJson] = useState("");
  const [proxyHeadersError, setProxyHeadersError] = useState<string | null>(null);
  const [protocolTesting, setProtocolTesting] = useState(false);
  const [protocolTestResult, setProtocolTestResult] = useState<{
    ok: boolean;
    protocol?: SiteProtocol;
    modelCount?: number;
    /** 失败分类（连不上 / 认证被拒 / 接口不可识别）。 */
    failureKind?: ProtocolTestFailureKind;
    error?: string;
    /** 用户按了「取消等待」。 */
    cancelled?: boolean;
  } | null>(null);
  /** 只用于丢弃结果；后端命令没有取消通道，见 handleTestProtocol 的注释。 */
  const protocolTestAbortRef = useRef<AbortController | null>(null);
  const protocolTestRunRef = useRef(0);
  const [newapiTestResult, setNewapiTestResult] = useState<{
    ok: boolean;
    amount?: string;
    detail?: string;
  } | null>(null);
  const watchedUrls = Form.useWatch("baseUrls", form) as string[] | undefined;
  const previewUrl = watchedUrls?.find((u) => String(u ?? "").trim()) ?? "";

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const caps = site?.capabilities ?? initialValues?.capabilities ?? {};
    const flags = codexFlagsFromCapabilities(caps);
    const protocol = site?.protocol ?? initialValues?.protocol ?? "openai_compatible";
    const notes = site ? (site.notes ?? "") : (initialValues?.notes ?? "");
    setCodexFlags(flags);
    setAdvancedOpen(shouldOpenAdvanced(protocol, notes) ? ["advanced"] : []);
    setCapOpen(anyCodexCapabilityOn(caps) ? ["codex"] : []);
    if (site) {
      const currentKeys = siteApiKeys(site);
      const summaries = [
        ...currentKeys.filter((key) => key.isActive),
        ...currentKeys.filter((key) => !key.isActive),
      ];
      setKeyLoading(summaries.length > 0 || site.hasKey);
      setNewapiTokenLoadFailed(false);
      setNewapiTestResult(null);
      setAdvancedOpen(
        shouldOpenAdvanced(protocol, notes) ||
          site.newapiConfigured ||
          (site.proxyHeaderCount ?? 0) > 0 ||
          forceAdvancedOpen
          ? ["advanced"]
          : [],
      );
      setProxyHeadersError(null);
      // 请求头密文按需解密：只在编辑时取一次，失败不阻塞表单其余部分。
      void invoke<ProxyHeader[]>("get_site_proxy_headers", { siteId: site.id })
        .then((headers) => {
          if (!cancelled) {
            setProxyHeadersJson(headers.length ? JSON.stringify(headers, null, 2) : "");
          }
        })
        .catch(() => {
          if (!cancelled) setProxyHeadersJson("");
        });
      form.setFieldsValue({
        name: site.name,
        baseUrls: siteBaseUrls(site),
        protocol: site.protocol,
        notes: site.notes ?? "",
        newapiAccessToken: "",
        newapiUserId: site.newapiUserId ?? "",
        apiKeys: summaries.length
          ? summaries.map((key) => ({ id: key.id, label: key.label, apiKey: "" }))
          : [{ label: "", apiKey: "" }],
      });
      void loadSiteKeyDrafts(site, getSiteApiKey)
        .then((apiKeys) => {
          if (!cancelled) form.setFieldValue("apiKeys", apiKeys);
        })
        .catch((error: unknown) => {
          if (cancelled) return;
          message.error(isAppError(error) ? error.message : t("sites.apiKeyLoadFailed"));
        })
        .finally(() => {
          if (!cancelled) setKeyLoading(false);
        });
      if (site.newapiConfigured) {
        void invoke<string>("get_site_newapi_token", { id: site.id })
          .then((token) => {
            if (!cancelled) form.setFieldValue("newapiAccessToken", token);
          })
          .catch((error: unknown) => {
            if (cancelled) {
              return;
            }
            setNewapiTokenLoadFailed(true);
            message.error(isAppError(error) ? error.message : t("sites.newapiTokenLoadFailed"));
          });
      }
    } else {
      setKeyLoading(false);
      setNewapiTokenLoadFailed(false);
      setAdvancedOpen(
        forceAdvancedOpen ||
        shouldOpenAdvanced(
          initialValues?.protocol ?? "openai_compatible",
          initialValues?.notes ?? "",
        )
          ? ["advanced"]
          : [],
      );
      form.resetFields();
      form.setFieldsValue({
        protocol: initialValues?.protocol ?? "openai_compatible",
        baseUrls: initialValues?.baseUrls?.length ? initialValues.baseUrls : [""],
        name: initialValues?.name,
        apiKeys: [{ label: "", apiKey: initialValues?.apiKey ?? "" }],
        notes: initialValues?.notes ?? "",
        newapiAccessToken: "",
        newapiUserId: "",
      });
      setProxyHeadersJson("");
      setProxyHeadersError(null);
    }
    return () => {
      cancelled = true;
    };
  }, [open, site, form, initialValues, forceAdvancedOpen, getSiteApiKey, message, t]);

  // 打开时清掉上一次的检测结果；关闭时停止等待（结果不再回填表单）。后端命令仍在
  // 后台跑完——见 handleTestProtocol 里对取消语义的说明。
  useEffect(() => {
    if (open) {
      setProtocolTestResult(null);
      setProtocolTesting(false);
      return;
    }
    protocolTestAbortRef.current?.abort();
    protocolTestAbortRef.current = null;
    protocolTestRunRef.current += 1;
    setProtocolTesting(false);
  }, [open]);

  // 卸载（页面被回收等）时同样丢弃在途结果，避免对已卸载的组件落状态。
  useEffect(
    () => () => {
      protocolTestAbortRef.current?.abort();
      protocolTestRunRef.current += 1;
    },
    [],
  );

  const handleTestProtocol = async () => {
    const values = form.getFieldsValue(["baseUrls", "apiKeys"]);
    const baseUrls = normalizeBaseUrls((values.baseUrls as string[] | undefined) ?? []);
    const keys = normalizeApiKeyDrafts(values.apiKeys);
    if (!baseUrls[0]) {
      message.error(t("sites.baseUrlRequired"));
      return;
    }
    if (keys.length === 0 || !keys[0]?.apiKey) {
      message.error(t("sites.apiKeyRequired"));
      return;
    }
    // 取消 = 丢弃这次等待的结果。Rust 侧的命令没有取消通道（IPC 调用不能中断），
    // 所以点「取消等待」后后端仍会跑完这套检测；前端只是立刻解除绑定：按钮复位、
    // 结果不再回填表单。UI 文案必须如实写成「取消等待」，不能写成「已取消检测」。
    const controller = new AbortController();
    protocolTestAbortRef.current?.abort();
    protocolTestAbortRef.current = controller;
    const runId = protocolTestRunRef.current + 1;
    protocolTestRunRef.current = runId;

    setProtocolTesting(true);
    setProtocolTestResult(null);
    try {
      const result = await invoke<ProtocolDetectionResult>("test_site_connection", {
        baseUrl: baseUrls[0],
        apiKey: keys[0].apiKey,
      });
      if (controller.signal.aborted || protocolTestRunRef.current !== runId) return;
      setProtocolTestResult({
        ok: true,
        protocol: result.detectedProtocol,
        modelCount: result.modelPreview.length,
      });
      // 自动填入检测到的协议
      form.setFieldValue("protocol", result.detectedProtocol);
      message.success(
        t("sites.protocolDetected", {
          protocol: result.detectedProtocol === "openai_compatible" 
            ? t("sites.protocolOpenai") 
            : t("sites.protocolAnthropic"),
          count: result.modelPreview.length,
        })
      );
    } catch (error) {
      if (controller.signal.aborted || protocolTestRunRef.current !== runId) return;
      const failureKind = protocolTestFailureKind(error);
      const detail = errorText(error) || t("sites.protocolTestFailed");
      setProtocolTestResult({ ok: false, failureKind, error: detail });
      message.error(t(PROTOCOL_TEST_FAILURE_KEYS[failureKind], { detail }));
    } finally {
      if (protocolTestRunRef.current === runId) {
        protocolTestAbortRef.current = null;
        setProtocolTesting(false);
      }
    }
  };

  const cancelProtocolTest = () => {
    if (!protocolTestAbortRef.current) return;
    protocolTestAbortRef.current.abort();
    protocolTestAbortRef.current = null;
    protocolTestRunRef.current += 1;
    setProtocolTesting(false);
    setProtocolTestResult({ ok: false, cancelled: true });
  };

  const handleTestNewapi = async () => {
    const values = form.getFieldsValue(["newapiAccessToken", "newapiUserId", "baseUrls"]);
    const baseUrls = normalizeBaseUrls((values.baseUrls as string[] | undefined) ?? []);
    const accessToken = ((values.newapiAccessToken as string | undefined) ?? "").trim();
    const userId = ((values.newapiUserId as string | undefined) ?? "").trim();
    if (!baseUrls[0]) {
      message.error(t("sites.newapiTestMissingBaseUrl"));
      return;
    }
    if (!userId) {
      message.error(t("sites.newapiTestMissingCredentials"));
      return;
    }
    if (!accessToken && !(site?.newapiConfigured && !newapiTokenLoadFailed)) {
      message.error(t("sites.newapiTestMissingCredentials"));
      return;
    }
    setNewapiTesting(true);
    setNewapiTestResult(null);
    try {
      const probe = await invoke<NewApiAccessProbe>("test_newapi_access", {
        input: {
          baseUrl: baseUrls[0],
          accessToken: accessToken || null,
          userId,
          siteId: site?.id ?? null,
        },
      });
      if (probe.ok) {
        setNewapiTestResult({
          ok: true,
          amount:
            probe.remainingUsd != null
              ? formatQuotaAmountLocalized(probe.remainingUsd, probe.unit, t)
              : "-",
        });
      } else {
        setNewapiTestResult({
          ok: false,
          detail: `HTTP ${probe.status}${probe.message ? ` · ${probe.message}` : ""}`,
        });
      }
    } catch (error) {
      setNewapiTestResult({
        ok: false,
        detail: isAppError(error) ? error.message : String(error),
      });
    } finally {
      setNewapiTesting(false);
    }
  };

  const handleOk = async () => {    try {
      const values = await form.validateFields();
      const baseUrls = normalizeBaseUrls(values.baseUrls as string[]);
      const capabilities = mergeCodexCapabilities(
        site?.capabilities ?? initialValues?.capabilities ?? {},
        capabilitiesFromCodexFlags(codexFlags),
      );
      const keys = normalizeApiKeyDrafts(values.apiKeys);
      if (keys.length === 0) {
        message.error(t("sites.apiKey"));
        return;
      }
      const parsedHeaders = parseProxyHeadersJson(proxyHeadersJson);
      if (parsedHeaders.error) {
        setProxyHeadersError(parsedHeaders.error);
        message.error(t("sites.proxyHeadersInvalid", { detail: parsedHeaders.error }));
        return;
      }
      setProxyHeadersError(null);
      // 已配置令牌但解密回填失败时省略字段，避免把令牌意外清空。
      const omitNewapiToken = site?.newapiConfigured === true && newapiTokenLoadFailed;
      const newapiAccessToken = omitNewapiToken
        ? undefined
        : (values.newapiAccessToken?.trim() || "");
      setSaving(true);
      let saved: Site;
      const isCreate = !site;
      if (site) {
        saved = await updateSite(site.id, {
          name: values.name,
          baseUrls,
          baseUrl: baseUrls[0],
          apiKeys: keys,
          protocol: values.protocol as SiteProtocol,
          notes: values.notes || null,
          capabilities,
          newapiAccessToken,
          newapiUserId: values.newapiUserId?.trim() || "",
          proxyHeaders: parsedHeaders.headers ?? [],
        });
        invalidateSiteIconCache(site.id);
      } else {
        saved = await createSite({
          name: values.name,
          baseUrls,
          baseUrl: baseUrls[0],
          apiKey: keys[0]!.apiKey,
          apiKeyLabel: keys[0]!.label,
          extraApiKeys: keys.slice(1).map((row) => ({
            label: row.label,
            apiKey: row.apiKey,
          })),
          protocol: values.protocol,
          notes: values.notes || null,
          capabilities,
          newapiAccessToken: values.newapiAccessToken?.trim() || null,
          newapiUserId: values.newapiUserId?.trim() || null,
          proxyHeaders: parsedHeaders.headers ?? [],
        });
      }
      message.success(isCreate ? t("sites.createSuccess") : t("sites.updateSuccess"));
      onSaved?.(saved, isCreate);
      onClose();
    } catch (e) {
      if (isAppError(e)) message.error(e.message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={site ? t("sites.edit") : t("sites.add")}
      onCancel={onClose}
      onOk={() => void handleOk()}
      confirmLoading={saving}
      okButtonProps={{ disabled: keyLoading }}
      okText={t("sites.save")}
      cancelText={t("sites.cancel")}
      width={560}
      destroyOnHidden
      centered
      mask={{ enabled: true }}
      styles={{
        container: {
          maxHeight: "calc(100vh - 32px)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        },
        body: {
          overflowY: "auto",
          overflowX: "hidden",
          minHeight: 0,
        },
      }}
    >
      <Form form={form} layout="vertical" className="mt-2" requiredMark="optional">
        <Form.Item name="name" label={t("sites.name")} rules={[{ required: true, message: t("sites.name") }]}>
          <Input placeholder="My Relay" allowClear />
        </Form.Item>
        <Form.Item
          label={
            <span className="inline-flex items-center gap-1.5">
              {t("sites.baseUrl")}
              <UrlWritePreviewIcon baseUrl={previewUrl} />
            </span>
          }
          extra={t("sites.baseUrlDefaultHint")}
          required
        >
          <BaseUrlListInput />
        </Form.Item>
        <Form.Item label={t("sites.apiKey")} extra={t("sites.apiKeyCreateHint")} required>
          <ApiKeyListInput disabled={keyLoading} />
        </Form.Item>
        <div className="flex flex-col gap-2">
          <Collapse
            size="small"
            activeKey={advancedOpen}
            onChange={(keys) => setAdvancedOpen(toActiveKeys(keys))}
            items={[
              {
                key: "advanced",
                label: t("sites.advanced"),
                children: (
                  <>
                    {/* 分三组并标注「可选」：高级区原本把 4 类不同人群才需要的字段平铺，
                        普通用户看不出哪些能跳过。分组后每块都自带适用范围。 */}
                    <Divider titlePlacement="start" style={{ marginTop: 0 }}>
                      <span style={{ fontSize: 12, fontWeight: 400 }}>
                        {t("sites.groupSiteInfo")}
                      </span>
                    </Divider>
                    <Form.Item name="protocol" label={t("sites.protocol")} extra={t("sites.protocolHint")}>
                      <Select
                        options={[
                          { value: "openai_compatible", label: t("sites.protocolOpenai") },
                          { value: "anthropic", label: t("sites.protocolAnthropic") },
                        ]}
                      />
                    </Form.Item>
                    <div className="mt-[-12px] mb-3 flex flex-wrap items-center gap-x-3 gap-y-1">
                      <Button
                        size="small"
                        loading={protocolTesting}
                        onClick={() => void handleTestProtocol()}
                      >
                        {t("sites.testConnection")}
                      </Button>
                      {protocolTesting && (
                        <>
                          <ProtocolTestProgress />
                          <Button
                            size="small"
                            type="text"
                            icon={<X size={12} />}
                            aria-label={t("sites.protocolTestCancel")}
                            onClick={cancelProtocolTest}
                          >
                            {t("sites.protocolTestCancel")}
                          </Button>
                        </>
                      )}
                      {!protocolTesting && protocolTestResult && (
                        <Text
                          type={
                            protocolTestResult.ok
                              ? "success"
                              : protocolTestResult.cancelled
                                ? "secondary"
                                : "danger"
                          }
                          style={{ fontSize: 12 }}
                          role={protocolTestResult.ok ? undefined : "alert"}
                        >
                          {protocolTestResult.ok
                            ? t("sites.protocolDetected", {
                                protocol: protocolTestResult.protocol === "openai_compatible"
                                  ? t("sites.protocolOpenai")
                                  : t("sites.protocolAnthropic"),
                                count: protocolTestResult.modelCount,
                              })
                            : protocolTestResult.cancelled
                              ? t("sites.protocolTestCancelled")
                              : t(
                                  PROTOCOL_TEST_FAILURE_KEYS[
                                    protocolTestResult.failureKind ?? "unknown"
                                  ],
                                  { detail: protocolTestResult.error ?? "" },
                                )}                        </Text>
                      )}
                    </div>
                    <Form.Item name="notes" label={t("sites.notes")}>
                      <Input.TextArea rows={2} allowClear />
                    </Form.Item>
                    <Divider titlePlacement="start">
                      <span style={{ fontSize: 12, fontWeight: 400 }}>
                        {t("sites.groupQuota")}
                      </span>
                    </Divider>
                    <Form.Item
                      name="newapiAccessToken"
                      label={t("sites.newapiAccessToken")}
                      extra={
                        site?.newapiConfigured && !newapiTokenLoadFailed
                          ? t("sites.newapiTokenSavedHint")
                          : t("sites.newapiTokenHint")
                      }
                    >
                      <Input.Password autoComplete="new-password" placeholder="Access Token" />
                    </Form.Item>
                    <Form.Item
                      name="newapiUserId"
                      label={t("sites.newapiUserId")}
                      className="!mb-0"
                      extra={t("sites.newapiUserIdHint")}
                    >
                      <Input allowClear placeholder="1" inputMode="numeric" />
                    </Form.Item>
                    {/* 测试按钮紧跟它要用的字段——原先被下面的请求头编辑器隔开，
                        看起来像两个无关的东西。 */}
                    <div className="mt-3 flex items-center gap-3">
                      <Button
                        size="small"
                        loading={newapiTesting}
                        onClick={() => void handleTestNewapi()}
                      >
                        {t("sites.newapiTest")}
                      </Button>
                      {newapiTestResult && (
                        <Text
                          type={newapiTestResult.ok ? "success" : "danger"}
                          style={{ fontSize: 12 }}
                        >
                          {newapiTestResult.ok
                            ? t("sites.newapiTestOk", { amount: newapiTestResult.amount })
                            : t("sites.newapiTestFailed", {
                                detail: newapiTestResult.detail,
                              })}
                        </Text>
                      )}
                    </div>
                    <Divider titlePlacement="start" style={{ marginBottom: 12 }}>
                      <span style={{ fontSize: 12, fontWeight: 400 }}>
                        {t("sites.groupProxy")}
                      </span>
                    </Divider>
                    <ProxyHeaderEditor
                      value={proxyHeadersJson}
                      onChange={(next) => {
                        setProxyHeadersJson(next);
                        if (proxyHeadersError) setProxyHeadersError(null);
                      }}
                      error={proxyHeadersError}
                    />
                  </>
                ),
              },
            ]}
          />
          <Collapse
            size="small"
            activeKey={capOpen}
            onChange={(keys) => setCapOpen(toActiveKeys(keys))}
            items={[
              {
                key: "codex",
                label: t("sites.codexPrivateCapabilities"),
                children: (
                  <CodexCapabilitySwitchList value={codexFlags} onChange={setCodexFlags} />
                ),
              },
            ]}
          />
        </div>
      </Form>
    </Modal>
  );
}
