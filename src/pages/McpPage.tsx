import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Collapse,
  Empty,
  Form,
  Input,
  List,
  Modal,
  Segmented,
  Space,
  Switch,
  Table,
  Tag,
  Tooltip,
  Typography,
  theme,
} from "antd";
import {
  CloudDownloadOutlined,
  CloudUploadOutlined,
  DeleteOutlined,
  EditOutlined,
  ImportOutlined,
  InfoCircleOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  SyncOutlined,
} from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { invoke } from "@/lib/invoke";
import { useMcpStore } from "@/stores";
import { useMcpUpdateStore } from "@/stores/mcpUpdateStore";
import type {
  McpImportLocator,
  McpKind,
  McpServerInput,
  McpServerSummary,
  RegistryCandidate,
  RegistryInstallDraft,
  RegistryRequiredField,
  ScanOutcome,
  ScannedMcp,
} from "@/types/mcp";
import type { TargetKind } from "@/types/domain";

const TARGETS: TargetKind[] = ["claude_code", "codex", "pi", "prime", "zcode"];

const TARGET_LABEL_KEYS: Record<TargetKind, string> = {
  claude_code: "mcp.targetClaudeCode",
  codex: "mcp.targetCodex",
  pi: "mcp.targetPi",
  prime: "mcp.targetPrime",
  zcode: "mcp.targetZCode",
};

const KIND_OPTIONS: { label: string; value: McpKind }[] = [
  { value: "stdio", label: "stdio" },
  { value: "sse", label: "SSE" },
  { value: "http", label: "HTTP" },
];

/** 高级层保留的「其余配置字段」：command/args/url 由简单层负责，这里只放额外键。 */
const SIMPLE_CONFIG_KEYS = ["command", "args", "url"] as const;

interface FormValues {
  name: string;
  kind: McpKind;
  enabled: boolean;
  targets: TargetKind[];
  command: string;
  argsText: string;
  url: string;
  extraConfig: string;
  env: string;
  headers: string;
}

function parseJsonObject(
  value: string,
  field: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): Record<string, unknown> {
  const trimmed = (value ?? "").trim();
  if (!trimmed) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    throw new Error(t("mcp.invalidJson", { field }));
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error(t("mcp.invalidJson", { field }));
  }
  return parsed as Record<string, unknown>;
}

/** 把已保存的 config 拆成简单层字段 + 其余字段（保留 cwd 之类的高级键）。 */
function splitConfig(config: Record<string, unknown> | undefined) {
  const source = config ?? {};
  const rest: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(source)) {
    if (!(SIMPLE_CONFIG_KEYS as readonly string[]).includes(key)) rest[key] = value;
  }
  const args = Array.isArray(source.args)
    ? source.args.filter((item): item is string => typeof item === "string")
    : [];
  return {
    command: typeof source.command === "string" ? source.command : "",
    argsText: args.join("\n"),
    url: typeof source.url === "string" ? source.url : "",
    extraConfig: Object.keys(rest).length > 0 ? JSON.stringify(rest, null, 2) : "{}",
  };
}

function parseArgs(text: string): string[] {
  return (text ?? "")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

/** 界面上如实展示这个 MCP 会被怎么运行 / 连到哪里。 */
function describeDraft(draft: RegistryInstallDraft, t: (key: string) => string) {
  const command = draft.config.command;
  if (draft.kind === "stdio" && typeof command === "string") {
    const args = Array.isArray(draft.config.args)
      ? draft.config.args.filter((item): item is string => typeof item === "string")
      : [];
    return `${t("mcp.registryWillRun")} ${[command, ...args].join(" ")}`;
  }
  const url = draft.config.url;
  return typeof url === "string" ? `${t("mcp.registryWillConnect")} ${url}` : "";
}

/** 扫描条目的启动方式摘要：本地是命令行，远程是地址。 */
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

function hostOf(url: string): string | null {
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
}

interface RegistrySearchBarProps {
  searching: boolean;
  onSubmit: (query: string) => void;
  onQueryChange: (query: string) => void;
}

/**
 * 仓库搜索框：输入值留在组件内部，击键只重渲染这个输入框，
 * 不会带着整页（仓库结果列表 + 已保存 MCP 表格）一起重渲染。
 * 父组件只通过 onQueryChange 记一份 ref（供「只看本地」切换时重查用）。
 */
function RegistrySearchBar({ searching, onSubmit, onQueryChange }: RegistrySearchBarProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");

  const submit = () => onSubmit(value);

  return (
    <Space.Compact style={{ width: "100%" }}>
      <Input
        value={value}
        onChange={(event) => {
          setValue(event.target.value);
          onQueryChange(event.target.value);
        }}
        onPressEnter={submit}
        placeholder={t("mcp.registrySearchPlaceholder")}
        allowClear
      />
      <Button type="primary" icon={<SearchOutlined />} loading={searching} onClick={submit}>
        {searching ? t("mcp.registrySearching") : t("mcp.registrySearch")}
      </Button>
    </Space.Compact>
  );
}

export function McpPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message, modal } = App.useApp();
  // 按字段订阅：任一 store 更新不再整页重渲染（表格与仓库列表都很贵）。
  const servers = useMcpStore((s) => s.servers);
  const loading = useMcpStore((s) => s.loading);
  const loadServers = useMcpStore((s) => s.loadServers);
  const getServer = useMcpStore((s) => s.getServer);
  const saveServer = useMcpStore((s) => s.saveServer);
  const deleteServer = useMcpStore((s) => s.deleteServer);
  const applyServers = useMcpStore((s) => s.applyServers);
  const searchRegistry = useMcpStore((s) => s.searchRegistry);
  const discoverRegistry = useMcpStore((s) => s.discoverRegistry);
  const scanExisting = useMcpStore((s) => s.scanExisting);
  const importScanned = useMcpStore((s) => s.importScanned);

  const updateStatuses = useMcpUpdateStore((s) => s.updateStatuses);
  const checking = useMcpUpdateStore((s) => s.checking);
  const updating = useMcpUpdateStore((s) => s.updating);
  const hasAnyUpdate = useMcpUpdateStore((s) => s.hasAnyUpdate);
  const updateCount = useMcpUpdateStore((s) => s.updateCount);
  const checkUpdates = useMcpUpdateStore((s) => s.checkUpdates);
  const updateSingleServer = useMcpUpdateStore((s) => s.updateServer);
  const updateAll = useMcpUpdateStore((s) => s.updateAll);

  const [open, setOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [targetPaths, setTargetPaths] = useState<[TargetKind, string][]>([]);
  // 「其它客户端已有的 MCP」扫描结果。null = 还没扫描过。
  const [scanOutcome, setScanOutcome] = useState<ScanOutcome | null>(null);
  const [scanning, setScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [form] = Form.useForm<FormValues>();
  const kind = Form.useWatch("kind", form);

  // 从官方仓库安装时：草稿 + 待填的必填项
  const [draft, setDraft] = useState<RegistryInstallDraft | null>(null);
  const [requiredValues, setRequiredValues] = useState<Record<string, string>>({});
  const [requiredErrors, setRequiredErrors] = useState<Record<string, boolean>>({});

  // 仓库搜索状态：查询词只记在 ref 里（输入框自身持有 state），切换「只看本地」时用它重查。
  const queryRef = useRef("");
  const [localOnly, setLocalOnly] = useState(true);
  const [searching, setSearching] = useState(false);
  const [candidates, setCandidates] = useState<RegistryCandidate[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [searched, setSearched] = useState(false);

  useEffect(() => {
    void loadServers();
    void invoke<[TargetKind, string][]>("mcp_target_paths")
      .then(setTargetPaths)
      .catch(() => setTargetPaths([]));
    // 首次打开就直接给出内容：空查询表示「浏览最近更新的 MCP」，避免进来是一片空白。
    void browseRecent(true);
    // 顺带扫一次本地已有配置：读本地文件，开销很小，用户一进来就能看到能纳管什么。
    void runScan();
    // 检查更新
    void checkUpdates().catch((error) => {
      console.error('Failed to check updates on mount:', error);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadServers]);

  const targetLabel = useCallback(
    (target: TargetKind) => t(TARGET_LABEL_KEYS[target] ?? target),
    [t],
  );

  // ---------------------------------------------------------------------
  // 表单
  // ---------------------------------------------------------------------

  const openCreate = () => {
    setEditingId(null);
    setDraft(null);
    setRequiredValues({});
    setRequiredErrors({});
    form.setFieldsValue({
      name: "",
      kind: "stdio",
      enabled: true,
      targets: [],
      command: "",
      argsText: "",
      url: "",
      extraConfig: "{}",
      env: "{}",
      headers: "{}",
    });
    setOpen(true);
  };

  const openEdit = useCallback(async (id: string) => {
    try {
      const server = await getServer(id);
      setEditingId(server.id);
      setDraft(null);
      setRequiredValues({});
      setRequiredErrors({});
      form.setFieldsValue({
        name: server.name,
        kind: server.kind,
        enabled: server.enabled,
        targets: server.targets,
        ...splitConfig(server.config),
        env: JSON.stringify(server.env ?? {}, null, 2),
        headers: JSON.stringify(server.headers ?? {}, null, 2),
      });
      setOpen(true);
    } catch (error) {
      void message.error(errorText(error));
    }
  }, [getServer, form, message]);

  /** 从仓库条目进入表单：自动填好命令/地址，只留必填密钥给用户。 */
  const openFromRegistry = (candidate: RegistryCandidate, targets: TargetKind[]) => {
    const install = candidate.draft;
    if (!install) return;
    setEditingId(null);
    setDraft(install);
    setRequiredValues({});
    setRequiredErrors({});
    form.setFieldsValue({
      name: install.name,
      kind: install.kind,
      enabled: true,
      targets,
      ...splitConfig(install.config as Record<string, unknown>),
      env: JSON.stringify(install.env ?? {}, null, 2),
      headers: JSON.stringify(install.headers ?? {}, null, 2),
    });
    setOpen(true);
  };

  /**
   * 仓库条目的目标推断：不新增 MCP 时，把已有 MCP 已经配置的目标作为默认值。
   *
   * 这样一键安装不会只保存不应用——用户是在已经用起来的客户端上加装，不需要每次重新勾。
   * 没有任何已配置目标时返回空数组，由调用方决定是直接装还是先问一句。
   */
  const preferredTargets = (): TargetKind[] => {
    const targets = new Set<TargetKind>();
    servers.forEach((server) => server.targets.forEach((target) => targets.add(target)));
    return TARGETS.filter((target) => targets.has(target));
  };

  /**
   * 把表单值 + 仓库必填项组装成待保存的输入。
   * 供弹窗保存与一键安装共用，避免两条路径的校验/拼装逻辑分叉。
   */
  const buildServerInput = (
    values: FormValues,
    options: { id?: string; targets: TargetKind[] },
  ): McpServerInput => {
    const extra = parseJsonObject(values.extraConfig, t("mcp.extraConfig"), t);
    const env = parseJsonObject(values.env, t("mcp.env"), t);
    const headers = parseJsonObject(values.headers, t("mcp.headers"), t);

    for (const field of draft?.requiredFields ?? []) {
      const value = (requiredValues[field.name] ?? "").trim();
      if (!value) continue;
      if (field.kind === "env") env[field.name] = value;
      else headers[field.name] = value;
    }

    const config: Record<string, unknown> = { ...extra };
    if (values.kind === "stdio") {
      const command = (values.command ?? "").trim();
      if (command) config.command = command;
      const args = parseArgs(values.argsText);
      if (args.length > 0) config.args = args;
    } else {
      const url = (values.url ?? "").trim();
      if (url) config.url = url;
    }

    return {
      id: options.id,
      name: values.name.trim(),
      kind: values.kind,
      enabled: values.enabled,
      targets: options.targets,
      config,
      env,
      headers,
    };
  };

  /** 保存并反馈结果。返回是否成功，供一键安装决定要不要再问用户。 */
  const persist = async (input: McpServerInput): Promise<boolean> => {
    try {
      const { sweep } = await saveServer(input);
      setDraft(null);
      void message.success(t("common.success"));
      if (sweep.results.some((item) => !item.ok)) {
        void message.warning(t("mcp.applyPartial"));
      }
      showApplyOutcome(sweep);
      return true;
    } catch (error) {
      void message.error(errorText(error));
      return false;
    }
  };

  /**
   * 一键安装：仓库条目已带全部启动信息、且不需要用户填任何东西时直接装好。
   * 需要填密钥、或还没配置过任何目标（不知道该装到哪儿）时，才打开表单。
   */
  const installFromRegistry = async (candidate: RegistryCandidate) => {
    const install = candidate.draft;
    if (!install) return;

    const targets = preferredTargets();
    const needsInput =
      (install.requiredFields?.length ?? 0) > 0 || targets.length === 0;

    if (needsInput) {
      openFromRegistry(candidate, targets);
      return;
    }

    // 名称冲突交给后端报错，这里不预判——用户可以直接改名重试。
    const values: FormValues = {
      name: install.name,
      kind: install.kind,
      enabled: true,
      targets,
      ...splitConfig(install.config as Record<string, unknown>),
      env: JSON.stringify(install.env ?? {}, null, 2),
      headers: JSON.stringify(install.headers ?? {}, null, 2),
    };
    setRequiredValues({});
    try {
      const input = buildServerInput(values, { targets, id: undefined });
      await persist(input);
    } catch (error) {
      // draft 为空时 buildServerInput 不会用到仓库必填项；解析失败只可能是预填值异常。
      void message.error(errorText(error));
    }
  };

  const handleSave = async () => {
    let values: FormValues;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }

    // 必填项由仓库声明，缺任何一个都不该让用户以为装好了。
    if (draft) {
      const missing: Record<string, boolean> = {};
      for (const field of draft.requiredFields) {
        if (!(requiredValues[field.name] ?? "").trim()) missing[field.name] = true;
      }
      setRequiredErrors(missing);
      if (Object.keys(missing).length > 0) {
        void message.error(t("mcp.registryRequiredMissing"));
        return;
      }
    }

    let input: McpServerInput;
    try {
      input = buildServerInput(values, { id: editingId ?? undefined, targets: values.targets ?? [] });
    } catch (error) {
      void message.error(errorText(error));
      return;
    }

    if (await persist(input)) setOpen(false);
  };

  const showApplyOutcome = useCallback((result: {
    results: { target: TargetKind; ok: boolean; message: string }[];
  }) => {
    if (result.results.length === 0) return;
    modal.info({
      centered: true,
      title: t("mcp.applyResultTitle"),
      width: 520,
      content: (
        <div className="flex flex-col gap-2">
          {result.results.map((item) => (
            <div key={item.target}>
              {item.ok ? (
                <span>
                  {t("mcp.targetSuccess", { target: targetLabel(item.target) })}
                  {item.message ? ` — ${item.message}` : ""}
                </span>
              ) : (
                <span style={{ color: token.colorError }}>
                  {t("mcp.targetFailed", {
                    target: targetLabel(item.target),
                    message: item.message,
                  })}
                </span>
              )}
            </div>
          ))}
        </div>
      ),
      okText: t("common.confirm"),
    });
  }, [modal, t, targetLabel, token.colorError]);

  const handleDelete = useCallback(
    (record: McpServerSummary) => {
      modal.confirm({
        centered: true,
        title: t("mcp.delete"),
        content: (
          <div>
            <div>{t("mcp.deleteConfirm", { name: record.name })}</div>
            <div style={{ marginTop: 8, color: token.colorTextTertiary }}>
              {t("mcp.deleteCleansTargets")}
            </div>
          </div>
        ),
        okButtonProps: { danger: true },
        onOk: async () => {
          try {
            const result = await deleteServer(record.id);
            void message.success(t("common.success"));
            showApplyOutcome(result);
          } catch (error) {
            void message.error(errorText(error));
          }
        },
      });
    },
    [modal, t, token.colorTextTertiary, deleteServer, message, showApplyOutcome],
  );

  // ---------------------------------------------------------------------
  // 仓库搜索
  // ---------------------------------------------------------------------

  /** 一次补齐的目标条数：仓库远程条目多，只看本地时单页往往只剩两三条。 */
  const FILL_TARGET = 20;

  /** 首次进入/切换「只看本地」时，用常见类目词拉「热门」而不是空列表。 */
  const browseRecent = async (onlyLocal: boolean) => {
    setSearching(true);
    try {
      const result = await discoverRegistry({ localOnly: onlyLocal, minResults: FILL_TARGET });
      setCandidates(result.candidates);
      setNextCursor(result.nextCursor ?? null);
      setSearched(true);
    } catch (error) {
      // 首次加载失败不该打断其它功能（手动添加仍可用），只提示一次。
      void message.error(errorText(error));
    } finally {
      setSearching(false);
    }
  };

  const runSearch = async (cursor?: string | null) => {
    const term = queryRef.current.trim();
    setSearching(true);
    try {
      const result = await searchRegistry(term, {
        cursor,
        localOnly,
        minResults: FILL_TARGET,
      });
      setCandidates((current) =>
        cursor ? [...current, ...result.candidates] : result.candidates,
      );
      setNextCursor(result.nextCursor ?? null);
      setSearched(true);
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setSearching(false);
    }
  };

  const toggleLocalOnly = (checked: boolean) => {
    setLocalOnly(checked);
    // 过滤在后端做，切换后必须重查，否则列表和开关会对不上。
    setCandidates([]);
    setNextCursor(null);
    if (queryRef.current.trim()) void runSearchAgain(checked);
    else void browseRecent(checked);
  };

  const runSearchAgain = async (onlyLocal: boolean) => {
    setSearching(true);
    try {
      const result = await searchRegistry(queryRef.current.trim(), {
        localOnly: onlyLocal,
        minResults: FILL_TARGET,
      });
      setCandidates(result.candidates);
      setNextCursor(result.nextCursor ?? null);
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setSearching(false);
    }
  };

  // ---------------------------------------------------------------------
  // 扫描并纳管其它客户端已有的 MCP
  // ---------------------------------------------------------------------

  /** 可纳管的条目：排除本工具托管的、以及已纳管过的。 */
  const importable = useMemo(
    () => (scanOutcome?.entries ?? []).filter((entry) => !entry.managed && !entry.importedId),
    [scanOutcome],
  );

  const runScan = async () => {
    setScanning(true);
    try {
      setScanOutcome(await scanExisting());
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setScanning(false);
    }
  };

  const importEntries = async (locators: McpImportLocator[]) => {
    if (locators.length === 0) return;
    setImporting(true);
    try {
      const result = await importScanned(locators);
      if (result.failed.length === 0) {
        void message.success(
          t("mcp.existingImportSuccess", { count: result.imported.length }),
        );
      } else if (result.imported.length > 0) {
        void message.warning(
          t("mcp.existingImportPartial", {
            ok: result.imported.length,
            failed: result.failed.length,
          }),
        );
      } else {
        // 全失败时把第一条原因带出来，否则用户不知道卡在哪。
        void message.error(
          `${t("mcp.existingImportFailed")}: ${result.failed[0]?.message ?? ""}`,
        );
      }
      // 重新扫描，让「已纳管」标记立刻刷新。
      setScanOutcome(await scanExisting());
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setImporting(false);
    }
  };

  // ---------------------------------------------------------------------
  // 应用目标
  // ---------------------------------------------------------------------

  const activeTargets = useMemo(() => {
    const set = new Set<TargetKind>();
    servers
      .filter((server) => server.enabled)
      .forEach((server) => server.targets.forEach((target) => set.add(target)));
    return TARGETS.filter((target) => set.has(target));
  }, [servers]);

  const handleUpdate = useCallback(
    async (id: string) => {
      try {
        await updateSingleServer(id);
        void message.success(t("mcp.updateSuccess"));
        await loadServers();
      } catch (error) {
        void message.error(errorText(error));
      }
    },
    [updateSingleServer, message, t, loadServers],
  );

  const handleUpdateAll = async () => {
    modal.confirm({
      centered: true,
      title: t("mcp.updateAllTitle"),
      content: t("mcp.updateAllConfirm", { count: updateCount }),
      onOk: async () => {
        try {
          const { successes, failures } = await updateAll();
          if (failures.length === 0) {
            void message.success(t("mcp.updateAllSuccess", { count: successes.length }));
          } else if (successes.length > 0) {
            void message.warning(
              t("mcp.updateAllPartial", {
                success: successes.length,
                failed: failures.length,
              }),
            );
          } else {
            void message.error(t("mcp.updateAllFailed"));
          }
          await loadServers();
        } catch (error) {
          void message.error(errorText(error));
        }
      },
    });
  };

  const handleCheckUpdates = async () => {
    try {
      // 手动刷新绕过 store 的新鲜度窗口：窗口内直接复用缓存会让按钮看起来「点了没反应」。
      await checkUpdates({ force: true });
      void message.success(t("mcp.checkUpdatesSuccess"));
    } catch (error) {
      void message.error(errorText(error));
    }
  };

  const applyTargets = async (targets: TargetKind[]) => {
    if (targets.length === 0) {
      void message.warning(t("mcp.noTargets"));
      return;
    }
    setApplying(true);
    try {
      const result = await applyServers(targets);
      const failed = result.results.filter((item) => !item.ok);
      if (failed.length === 0) {
        void message.success(t("mcp.applySuccess"));
      } else if (failed.length < result.results.length) {
        void message.warning(t("mcp.applyPartial"));
      } else {
        void message.error(t("mcp.applyFailed"));
      }
      showApplyOutcome(result);
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setApplying(false);
    }
  };

  // 列定义只在语言/更新状态/回调变化时重建；输入与其它本地 state 不再让表格重新生成 columns。
  const columns = useMemo(
    () => [
    {
      title: t("mcp.name"),
      dataIndex: "name",
      key: "name",
      render: (name: string, record: McpServerSummary) => {
        const status = updateStatuses.find((s) => s.id === record.id);
        return (
          <Space size={4}>
            <Typography.Text strong>{name}</Typography.Text>
            {status?.hasUpdate && (
              <Tag color="orange" style={{ fontSize: 11 }}>
                {t("mcp.hasUpdate")}
              </Tag>
            )}
          </Space>
        );
      },
    },
    {
      title: t("mcp.kind"),
      dataIndex: "kind",
      key: "kind",
      width: 90,
      render: (item: McpKind) => <Tag>{item}</Tag>,
    },
    {
      title: t("mcp.targets"),
      dataIndex: "targets",
      key: "targets",
      render: (targets: TargetKind[]) =>
        targets.length === 0 ? (
          <Typography.Text type="secondary">—</Typography.Text>
        ) : (
          <Space size={4} wrap>
            {targets.map((target) => (
              <Tag key={target}>{targetLabel(target)}</Tag>
            ))}
          </Space>
        ),
    },
    {
      title: t("mcp.enabled"),
      dataIndex: "enabled",
      key: "enabled",
      width: 90,
      render: (enabled: boolean) =>
        enabled ? <Tag color="green">{t("mcp.enabled")}</Tag> : <Tag>{t("mcp.disabled")}</Tag>,
    },
    {
      title: t("common.actions"),
      key: "actions",
      render: (_: unknown, record: McpServerSummary) => {
        const status = updateStatuses.find((s) => s.id === record.id);
        const isUpdating = updating[record.id] || false;
        return (
          <Space size={0}>
            {status?.hasUpdate && (
              <Tooltip title={t("mcp.update")}>
                <Button
                  type="text"
                  size="small"
                  aria-label={t("mcp.update")}
                  icon={<SyncOutlined spin={isUpdating} />}
                  loading={isUpdating}
                  onClick={() => void handleUpdate(record.id)}
                />
              </Tooltip>
            )}
            <Tooltip title={t("common.edit")}>
              <Button
                type="text"
                size="small"
                aria-label={t("common.edit")}
                icon={<EditOutlined />}
                onClick={() => void openEdit(record.id)}
              />
            </Tooltip>
            <Tooltip title={t("common.delete")}>
              <Button
                type="text"
                size="small"
                danger
                aria-label={t("common.delete")}
                icon={<DeleteOutlined />}
                onClick={() => handleDelete(record)}
              />
            </Tooltip>
          </Space>
        );
      },
    },
    ],
    [t, targetLabel, updateStatuses, updating, handleUpdate, openEdit, handleDelete],
  );

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-6">
      <div className="flex items-center justify-between gap-4">
        <div>
          <Typography.Title level={4} style={{ margin: 0 }}>
            {t("mcp.title")}
            {hasAnyUpdate() && (
              <Tag color="orange" style={{ marginLeft: 8, fontSize: 12 }}>
                {t("mcp.updatesAvailable", { count: updateCount() })}
              </Tag>
            )}
          </Typography.Title>
          <Typography.Text type="secondary">{t("mcp.emptyDesc")}</Typography.Text>
        </div>
        <Space>
          <Tooltip title={t("mcp.checkUpdates")}>
            <Button
              icon={<ReloadOutlined spin={checking} />}
              loading={checking}
              onClick={() => void handleCheckUpdates()}
            >
              {t("mcp.checkUpdates")}
            </Button>
          </Tooltip>
          {hasAnyUpdate() && (
            <Button
              type="default"
              icon={<SyncOutlined />}
              onClick={() => void handleUpdateAll()}
            >
              {t("mcp.updateAll")} ({updateCount()})
            </Button>
          )}
          <Button
            type="primary"
            icon={<CloudUploadOutlined />}
            loading={applying}
            disabled={activeTargets.length === 0}
            onClick={() => void applyTargets(activeTargets)}
          >
            {t("mcp.applyToTargets")}
          </Button>
          <Tooltip title={t("mcp.manualAddHint")}>
            <Button icon={<PlusOutlined />} onClick={openCreate}>
              {t("mcp.manualAdd")}
            </Button>
          </Tooltip>
        </Space>
      </div>

      {/* 主入口：从官方仓库装 */}
      <Card
        size="small"
        title={
          <Space>
            <CloudDownloadOutlined />
            <span>{t("mcp.registryTitle")}</span>
          </Space>
        }
      >
        <div className="flex flex-col gap-3">
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.registryDesc")}
          </Typography.Text>
          <RegistrySearchBar
            searching={searching}
            onQueryChange={(value) => {
              queryRef.current = value;
            }}
            onSubmit={(value) => {
              queryRef.current = value;
              setCandidates([]);
              setNextCursor(null);
              void runSearch(null);
            }}
          />

          <Space size={8} wrap>
            <Switch checked={localOnly} size="small" onChange={toggleLocalOnly} />
            <Typography.Text style={{ fontSize: 12 }}>{t("mcp.registryLocalOnly")}</Typography.Text>
            <Tooltip title={t("mcp.registryLocalOnlyHint")}>
              <InfoCircleOutlined style={{ color: token.colorTextTertiary, fontSize: 12 }} />
            </Tooltip>
          </Space>

          {candidates.length > 0 && (
            <List
              size="small"
              dataSource={candidates}
              renderItem={(candidate) => {
                const install = candidate.draft;
                const local =
                  install?.kind === "stdio" && typeof install.config.command === "string";
                const url = typeof install?.config.url === "string" ? install.config.url : "";
                const host = url ? hostOf(url) : null;
                const requiredCount = install?.requiredFields.length ?? 0;
                return (
                  <List.Item
                    actions={[
                      <Button
                        key="install"
                        type="primary"
                        size="small"
                        disabled={!install}
                        onClick={() => void installFromRegistry(candidate)}
                      >
                        {t("mcp.registryInstall")}
                      </Button>,
                    ]}
                  >
                    <List.Item.Meta
                      title={
                        <Space size={6} wrap>
                          <Typography.Text strong>{candidate.name}</Typography.Text>
                          <Tag color={local ? "green" : "orange"}>
                            {local ? t("mcp.registryLocal") : t("mcp.registryRemote")}
                          </Tag>
                          {candidate.version && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              v{candidate.version}
                            </Typography.Text>
                          )}
                          {!install && <Tag>{t("mcp.registryUnsupported")}</Tag>}
                        </Space>
                      }
                      description={
                        <div className="flex flex-col gap-1">
                          <span>{candidate.description}</span>
                          {/* 远程条目要如实说明请求会发到哪个域名 */}
                          {!local && host && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {t("mcp.registryRemoteHost", { host })}
                            </Typography.Text>
                          )}
                          {requiredCount > 0 && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {t("mcp.registryRequiredTitle")}
                            </Typography.Text>
                          )}
                        </div>
                      }
                    />
                  </List.Item>
                );
              }}
            />
          )}

          {candidates.length > 0 && nextCursor && (
            <Button size="small" loading={searching} onClick={() => void runSearch(nextCursor)}>
              {t("mcp.registryLoadMore")}
            </Button>
          )}

          {searched && !searching && candidates.length === 0 && (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("mcp.registryNoResults")}
            />
          )}

          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.registrySourceHint")}
          </Typography.Text>
        </div>
      </Card>

      <Card
        size="small"
        title={
          <Space>
            <ImportOutlined />
            <span>{t("mcp.existingTitle")}</span>
          </Space>
        }
        extra={
          <Space>
            {importable.length > 0 && (
              <Button
                size="small"
                type="primary"
                loading={importing}
                onClick={() =>
                  void importEntries(
                    importable.map((entry) => ({ target: entry.target, key: entry.key })),
                  )
                }
              >
                {t("mcp.existingImportAll", { count: importable.length })}
              </Button>
            )}
            <Button
              size="small"
              icon={<SearchOutlined />}
              loading={scanning}
              onClick={() => void runScan()}
            >
              {scanOutcome ? t("mcp.existingRescan") : t("mcp.existingScan")}
            </Button>
          </Space>
        }
      >
        <div className="flex flex-col gap-2">
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.existingDesc")}
          </Typography.Text>

          {/* 单个客户端读不了不影响其它客户端，逐条提示 */}
          {(scanOutcome?.warnings ?? []).map((warning) => (
            <Typography.Text
              key={`${warning.target}:${warning.message}`}
              type="warning"
              style={{ fontSize: 12 }}
            >
              {t("mcp.existingWarning", {
                target: targetLabel(warning.target as TargetKind),
                message: warning.message,
              })}
            </Typography.Text>
          ))}

          {scanOutcome && importable.length === 0 && scanOutcome.entries.length === 0 && (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("mcp.existingEmpty")}
            />
          )}

          {(scanOutcome?.entries.length ?? 0) > 0 && (
            <List
              size="small"
              dataSource={scanOutcome?.entries ?? []}
              rowKey={(entry) => `${entry.target}:${entry.key}`}
              renderItem={(entry) => {
                const keys = [...entry.envKeys, ...entry.headerKeys];
                return (
                  <List.Item
                    actions={
                      entry.managed || entry.importedId
                        ? [
                            <Tag key="state" color={entry.managed ? "blue" : "green"}>
                              {entry.managed
                                ? t("mcp.existingManagedTag")
                                : t("mcp.existingImported")}
                            </Tag>,
                          ]
                        : [
                            <Button
                              key="import"
                              size="small"
                              loading={importing}
                              onClick={() =>
                                void importEntries([{ target: entry.target, key: entry.key }])
                              }
                            >
                              {t("mcp.existingImport")}
                            </Button>,
                          ]
                    }
                  >
                    <List.Item.Meta
                      title={
                        <Space size={6} wrap>
                          <Typography.Text strong>{entry.name}</Typography.Text>
                          {/* ScanTarget 与 TargetKind 取值一致，展示名可复用 */}
                          <Tag>{targetLabel(entry.target as TargetKind)}</Tag>
                        </Space>
                      }
                      description={
                        <div className="flex flex-col gap-1">
                          <Typography.Text code style={{ fontSize: 12 }}>
                            {describeScanned(entry)}
                          </Typography.Text>
                          {/* 只列键名，值从不离开后端 */}
                          {keys.length > 0 && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {t("mcp.existingKeys", { keys: keys.join(", ") })}
                            </Typography.Text>
                          )}
                        </div>
                      }
                    />
                  </List.Item>
                );
              }}
            />
          )}

          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.existingNote")}
          </Typography.Text>
        </div>
      </Card>

      {servers.length === 0 && !loading ? (
        <Card>
          <Empty description={t("mcp.emptyTitle")} />
        </Card>
      ) : (
        <Table
          columns={columns}
          dataSource={servers}
          rowKey="id"
          loading={loading}
          pagination={false}
          size="small"
        />
      )}

      <Card size="small" styles={{ body: { display: "flex", gap: 8, alignItems: "flex-start" } }}>
        <InfoCircleOutlined style={{ color: token.colorTextTertiary, marginTop: 2 }} />
        <div>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.secretsNotice")}
          </Typography.Text>
          <details style={{ marginTop: 6 }}>
            <summary style={{ cursor: "pointer", fontSize: 12, color: token.colorTextTertiary }}>
              {t("mcp.pathsTitle")}
            </summary>
            <div style={{ marginTop: 4 }}>
              {targetPaths.map(([target, path]) => (
                <div key={target} style={{ fontSize: 12 }}>
                  <Typography.Text type="secondary">
                    {targetLabel(target)}: <Typography.Text code>{path}</Typography.Text>
                  </Typography.Text>
                </div>
              ))}
            </div>
          </details>
        </div>
      </Card>

      <Modal
        centered
        destroyOnHidden
        mask={{ enabled: true }}
        width={560}
        open={open}
        title={editingId ? t("mcp.edit") : draft ? t("mcp.registryInstall") : t("mcp.manualAdd")}
        onCancel={() => setOpen(false)}
        onOk={() => void handleSave()}
        okText={t("common.save")}
        cancelText={t("common.cancel")}
      >
        <Form form={form} layout="vertical">
          {draft && (
            <Alert
              type="info"
              showIcon
              style={{ marginBottom: 12 }}
              message={t("mcp.registryFrom", { name: draft.displayName })}
              description={
                <div className="flex flex-col gap-1">
                  <span style={{ fontSize: 12 }}>{describeDraft(draft, t)}</span>
                  {draft.repositoryUrl && (
                    <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                      {draft.repositoryUrl}
                    </Typography.Text>
                  )}
                </div>
              }
            />
          )}

          {/* 仓库声明必填的字段：逐项让用户填，而不是丢一堆 JSON 让他猜 */}
          {draft && draft.requiredFields.length > 0 && (
            <Card size="small" style={{ marginBottom: 12 }}>
              <Typography.Text strong style={{ fontSize: 12 }}>
                {t("mcp.registryRequiredTitle")}
              </Typography.Text>
              <div className="mt-2 flex flex-col gap-3">
                {draft.requiredFields.map((field: RegistryRequiredField) => (
                  <div key={`${field.kind}:${field.name}`}>
                    <Typography.Text code style={{ fontSize: 12 }}>
                      {field.name}
                    </Typography.Text>
                    {field.secret && (
                      <Tag color="red" style={{ marginLeft: 6 }}>
                        {t("mcp.registryRequiredSecret")}
                      </Tag>
                    )}
                    {field.description && (
                      <div>
                        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                          {field.description}
                        </Typography.Text>
                      </div>
                    )}
                    <Input.Password
                      visibilityToggle
                      aria-label={field.name}
                      status={requiredErrors[field.name] ? "error" : undefined}
                      value={requiredValues[field.name] ?? ""}
                      onChange={(event) =>
                        setRequiredValues((current) => ({
                          ...current,
                          [field.name]: event.target.value,
                        }))
                      }
                      style={{ marginTop: 4 }}
                    />
                  </div>
                ))}
              </div>
            </Card>
          )}

          <Form.Item
            name="name"
            label={t("mcp.name")}
            rules={[
              { required: true, message: t("mcp.nameRequired") },
              { pattern: /^[A-Za-z0-9_-]+$/, message: t("mcp.nameRule") },
            ]}
          >
            <Input allowClear placeholder="filesystem" />
          </Form.Item>

          <Form.Item name="kind" label={t("mcp.kind")}>
            <Segmented options={KIND_OPTIONS} />
          </Form.Item>

          <Form.Item name="targets" label={t("mcp.targets")}>
            <Checkbox.Group
              options={TARGETS.map((target) => ({ label: targetLabel(target), value: target }))}
            />
          </Form.Item>

          {/* 简单层：只问「怎么启动」或「连哪里」 */}
          {kind === "stdio" ? (
            <>
              <Form.Item name="command" label={t("mcp.command")}>
                <Input allowClear placeholder="npx" />
              </Form.Item>
              <Form.Item name="argsText" label={t("mcp.args")} extra={t("mcp.argsHint")}>
                <Input.TextArea autoSize={{ minRows: 2, maxRows: 6 }} />
              </Form.Item>
            </>
          ) : (
            <Form.Item name="url" label={t("mcp.url")}>
              <Input allowClear placeholder="https://example.com/mcp" />
            </Form.Item>
          )}

          <Form.Item name="enabled" valuePropName="checked">
            <Checkbox>{t("mcp.enabled")}</Checkbox>
          </Form.Item>

          {/* 箭头放到行尾、去掉头部的内边距，让「高级配置」与上方 Form 标签左对齐 */}
          <Collapse
            ghost
            expandIconPosition="end"
            styles={{ header: { paddingInline: 0 } }}
            items={[
              {
                key: "advanced",
                label: t("mcp.advancedSection"),
                children: (
                  <>
                    <Form.Item name="env" label={t("mcp.env")}>
                      <Input.TextArea autoSize={{ minRows: 2, maxRows: 8 }} />
                    </Form.Item>
                    <Form.Item
                      name="headers"
                      label={t("mcp.headers")}
                      extra={kind === "stdio" ? t("mcp.headersStdioHint") : undefined}
                    >
                      <Input.TextArea autoSize={{ minRows: 2, maxRows: 8 }} />
                    </Form.Item>
                    <Form.Item
                      name="extraConfig"
                      label={t("mcp.extraConfig")}
                      extra={t("mcp.extraConfigHint")}
                    >
                      <Input.TextArea autoSize={{ minRows: 2, maxRows: 8 }} />
                    </Form.Item>
                  </>
                ),
              },
            ]}
          />
        </Form>
      </Modal>
    </div>
  );
}

/** 统一的错误文案提取：AppError 走 message，其它错误退回字符串。 */
function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: string } | null)?.message;
  return message ?? String(error);
}
