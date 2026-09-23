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
  Tag,
  Tooltip,
  Typography,
  theme,
} from "antd";
import {
  InfoCircleOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  SyncOutlined,
} from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { invoke } from "@/lib/invoke";
import { useMcpStore } from "@/stores";
import { useUIStore } from "@/stores/uiStore";
import { useMcpUpdateStore } from "@/stores/mcpUpdateStore";
import { McpSidebar, MCP_AGENT_TABS } from "@/components/mcp/McpSidebar";
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
import { McpCard, type ClientState } from "./mcp/McpCard";
import { UnmanagedMcpCard } from "./mcp/UnmanagedMcpCard";
import { ConflictModal, type ConflictContext } from "./mcp/ConflictModal";
import {
  MCP_TARGETS as TARGETS,
  MCP_TARGET_LABEL_KEYS as TARGET_LABEL_KEYS,
} from "./mcp/targets";

const KIND_OPTIONS: { label: string; value: McpKind }[] = [
  { value: "stdio", label: "stdio" },
  { value: "sse", label: "SSE" },
  { value: "http", label: "HTTP" },
];

/** 高级层保留的「其余配置字段」：command/args/url 由简单层负责，这里只放额外键。 */
const SIMPLE_CONFIG_KEYS = ["command", "args", "url"] as const;

/** 添加来源：手动填写 / 从仓库安装。 */
type AddSource = "manual" | "registry";

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
 * 不会带着整页一起重渲染。父组件只通过 onQueryChange 记一份 ref。
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
  // 按字段订阅：任一 store 更新不再整页重渲染。
  const servers = useMcpStore((s) => s.servers);
  const loading = useMcpStore((s) => s.loading);
  const drift = useMcpStore((s) => s.drift);
  const loadServers = useMcpStore((s) => s.loadServers);
  const loadDrift = useMcpStore((s) => s.loadDrift);
  const getServer = useMcpStore((s) => s.getServer);
  const saveServer = useMcpStore((s) => s.saveServer);
  const deleteServer = useMcpStore((s) => s.deleteServer);
  const applyServers = useMcpStore((s) => s.applyServers);
  const searchRegistry = useMcpStore((s) => s.searchRegistry);
  const discoverRegistry = useMcpStore((s) => s.discoverRegistry);
  const scanExisting = useMcpStore((s) => s.scanExisting);
  const importScanned = useMcpStore((s) => s.importScanned);

  // per-agent 外壳：当前选中的 agent（不含 Prime）。
  const mcpTab = useUIStore((s) => s.mcpTab);

  const updateStatuses = useMcpUpdateStore((s) => s.updateStatuses);
  const checking = useMcpUpdateStore((s) => s.checking);
  const updating = useMcpUpdateStore((s) => s.updating);
  const hasAnyUpdate = useMcpUpdateStore((s) => s.hasAnyUpdate);
  const updateCount = useMcpUpdateStore((s) => s.updateCount);
  const checkUpdates = useMcpUpdateStore((s) => s.checkUpdates);
  const updateSingleServer = useMcpUpdateStore((s) => s.updateServer);
  const updateAll = useMcpUpdateStore((s) => s.updateAll);

  const [open, setOpen] = useState(false);
  const [addSource, setAddSource] = useState<AddSource>("manual");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [targetPaths, setTargetPaths] = useState<[TargetKind, string][]>([]);
  const [mineSearch, setMineSearch] = useState("");
  // 「其它客户端已有的 MCP」扫描结果。null = 还没扫描过。
  const [scanOutcome, setScanOutcome] = useState<ScanOutcome | null>(null);
  // 重新检测进行中：驱动按钮 spinner，手动触发时还给一条完成提示。
  const [scanning, setScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  // 某条 MCP 的某个客户端开关正在写盘（防连点，逐卡独立 spinner）。
  const [busyToggle, setBusyToggle] = useState<{ id: string; target: TargetKind } | null>(null);
  // 「需重新应用」横幅的一键重写盘正在进行。
  const [reapplying, setReapplying] = useState(false);
  const [form] = Form.useForm<FormValues>();
  const kind = Form.useWatch("kind", form);

  // 从官方仓库安装时：草稿 + 待填的必填项
  const [draft, setDraft] = useState<RegistryInstallDraft | null>(null);
  const [requiredValues, setRequiredValues] = useState<Record<string, string>>({});
  const [requiredErrors, setRequiredErrors] = useState<Record<string, boolean>>({});
  // 免密钥条目在没有任何已有目标时，弹窗问一次装到哪儿（不先入库不应用）。
  const [targetPicker, setTargetPicker] = useState<RegistryCandidate | null>(null);
  const [pickerTargets, setPickerTargets] = useState<TargetKind[]>([...MCP_AGENT_TABS]);

  // 同名冲突解决弹窗
  const [conflict, setConflict] = useState<ConflictContext | null>(null);
  const [conflictLoc, setConflictLoc] = useState<{ target: ScannedMcp["target"]; key: string } | null>(null);

  // 仓库搜索状态：查询词只记在 ref 里（输入框自身持有 state）。
  const queryRef = useRef("");
  const [localOnly, setLocalOnly] = useState(true);
  const [searching, setSearching] = useState(false);
  const [candidates, setCandidates] = useState<RegistryCandidate[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [searched, setSearched] = useState(false);

  const targetLabel = useCallback(
    (target: TargetKind) => t(TARGET_LABEL_KEYS[target] ?? target),
    [t],
  );

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

  // ---------------------------------------------------------------------
  // 仓库搜索
  // ---------------------------------------------------------------------

  /** 一次补齐的目标条数：仓库远程条目多，只看本地时单页往往只剩两三条。 */
  const FILL_TARGET = 20;

  /** 首次进入/切换「只看本地」时，用常见类目词拉「常用推荐」而不是空列表。 */
  const browseRecent = useCallback(async (onlyLocal: boolean) => {
    setSearching(true);
    try {
      const result = await discoverRegistry({ localOnly: onlyLocal, minResults: FILL_TARGET });
      setCandidates(result.candidates);
      setNextCursor(result.nextCursor ?? null);
      setSearched(true);
    } catch (error) {
      void message.error(errorText(error));
    } finally {
      setSearching(false);
    }
  }, [discoverRegistry, message]);

  const runScan = useCallback(
    async (options?: { notify?: boolean }) => {
      setScanning(true);
      try {
        setScanOutcome(await scanExisting());
        if (options?.notify) void message.success(t("mcp.rescanDone"));
      } catch (error) {
        void message.error(errorText(error));
      } finally {
        setScanning(false);
      }
      // 扫描后接管关系/漂移都可能变，顺手刷新漂移标记。
      void loadDrift();
    },
    [scanExisting, message, t, loadDrift],
  );

  useEffect(() => {
    void loadServers();
    void invoke<[TargetKind, string][]>("mcp_target_paths")
      .then(setTargetPaths)
      .catch(() => setTargetPaths([]));
    // 一进来就扫一次本地已有配置：读本地文件，开销很小，直接看到能纳管什么。
    void runScan();
    void checkUpdates().catch((error) => {
      console.error("Failed to check updates on mount:", error);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadServers]);

  // ---------------------------------------------------------------------
  // 表单
  // ---------------------------------------------------------------------

  const resetForm = () => {
    setDraft(null);
    setRequiredValues({});
    setRequiredErrors({});
  };

  const openCreate = () => {
    setEditingId(null);
    setAddSource("manual");
    resetForm();
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
    // 打开时顺带拉一次仓库「常用推荐」，切到仓库页签就有内容。
    if (!searched) void browseRecent(localOnly);
    setOpen(true);
  };

  const openEdit = useCallback(async (id: string) => {
    try {
      const server = await getServer(id);
      setEditingId(server.id);
      setAddSource("manual");
      resetForm();
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
    setAddSource("manual");
    form.setFieldsValue({
      name: install.name,
      kind: install.kind,
      enabled: true,
      targets,
      ...splitConfig(install.config as Record<string, unknown>),
      env: JSON.stringify(install.env ?? {}, null, 2),
      headers: JSON.stringify(install.headers ?? {}, null, 2),
    });
  };

  /**
   * 仓库条目的目标推断：把已有 MCP 已经配置的目标作为默认值。
   * 一键安装不会只保存不应用；没有任何已配置目标时返回空数组。
   */
  const preferredTargets = (): TargetKind[] => {
    const targets = new Set<TargetKind>();
    servers.forEach((server) => server.targets.forEach((target) => targets.add(target)));
    return MCP_AGENT_TABS.filter((target) => targets.has(target));
  };

  /** 把表单值 + 仓库必填项组装成待保存的输入。 */
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
      void runScan();
      return true;
    } catch (error) {
      void message.error(errorText(error));
      return false;
    }
  };

  /** 直接安装：仓库条目信息齐全、目标明确时不弹表单，一步装好。 */
  const installDirect = async (candidate: RegistryCandidate, targets: TargetKind[]) => {
    const install = candidate.draft;
    if (!install) return;
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
      if (await persist(input)) setOpen(false);
    } catch (error) {
      void message.error(errorText(error));
    }
  };

  /**
   * 一键安装分流：
   * - 需要填密钥 → 切到手动表单（必填项逐项填）；
   * - 免密钥但没有任何已有目标 → 小弹窗问一次装到哪儿；
   * - 否则直接装好并沿用已有目标。
   */
  const installFromRegistry = async (candidate: RegistryCandidate) => {
    const install = candidate.draft;
    if (!install) return;

    if ((install.requiredFields?.length ?? 0) > 0) {
      openFromRegistry(candidate, preferredTargets());
      return;
    }

    const targets = preferredTargets();
    if (targets.length === 0) {
      setPickerTargets([...MCP_AGENT_TABS]);
      setTargetPicker(candidate);
      return;
    }

    await installDirect(candidate, targets);
  };

  const handleSave = async () => {
    let values: FormValues;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }

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
            void runScan();
          } catch (error) {
            void message.error(errorText(error));
          }
        },
      });
    },
    [modal, t, token.colorTextTertiary, deleteServer, message, showApplyOutcome, runScan],
  );

  const runSearch = async (cursor?: string | null) => {
    const term = queryRef.current.trim();
    setSearching(true);
    try {
      const result = await searchRegistry(term, { cursor, localOnly, minResults: FILL_TARGET });
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

  /** 可纳管的「野生」条目：排除托管的、已纳管过的、同名冲突的。 */
  const importable = useMemo(
    () =>
      (scanOutcome?.entries ?? []).filter(
        (entry) => !entry.managed && !entry.importedId && !entry.nameConflict,
      ),
    [scanOutcome],
  );

  /** (归一化名, 目标) → 同名冲突：卡片据此把该客户端开关标为冲突态。 */
  const conflictSet = useMemo(() => {
    const set = new Set<string>();
    for (const entry of scanOutcome?.entries ?? []) {
      if (entry.nameConflict) set.add(`${entry.name}::${entry.target}`);
    }
    return set;
  }, [scanOutcome]);

  const clientStatesOf = useCallback(
    (server: McpServerSummary): Record<TargetKind, ClientState> => {
      const states = {} as Record<TargetKind, ClientState>;
      for (const target of TARGETS) {
        if (conflictSet.has(`${server.name}::${target}`)) states[target] = "conflict";
        else if (server.enabled && server.targets.includes(target)) states[target] = "on";
        else states[target] = "off";
      }
      return states;
    },
    [conflictSet],
  );

  const importEntries = useCallback(
    async (locators: McpImportLocator[]) => {
      if (locators.length === 0) return;
      setImporting(true);
      try {
        const result = await importScanned(locators);
        const alreadyCount = result.alreadyImported?.length ?? 0;
        if (result.failed.length === 0) {
          void message.success(t("mcp.existingImportSuccess", { count: result.imported.length }));
          if (alreadyCount > 0) {
            void message.info(t("mcp.existingAlreadyImported", { count: alreadyCount }));
          }
        } else if (result.imported.length > 0) {
          void message.warning(
            t("mcp.existingImportPartial", {
              ok: result.imported.length,
              failed: result.failed.length,
            }),
          );
        } else {
          void message.error(
            `${t("mcp.existingImportFailed")}: ${result.failed[0]?.message ?? ""}`,
          );
        }
        // 纳管即接管：入库后立即写盘。某客户端里的同名条目在扫描后被改成与库记录不等价时，
        // 接管会跳过该目标并原样保留文件——提示用户哪些客户端没接管成功。
        const takeoverFailed = (result.apply?.results ?? []).filter((r) => !r.ok);
        if (takeoverFailed.length > 0) {
          void message.warning(
            t("mcp.existingImportTakeoverFailed", {
              targets: takeoverFailed.map((r) => targetLabel(r.target)).join("、"),
            }),
          );
        }
        setScanOutcome(await scanExisting());
        void loadDrift();
      } catch (error) {
        void message.error(errorText(error));
      } finally {
        setImporting(false);
      }
    },
    [importScanned, scanExisting, message, t, targetLabel, loadDrift],
  );

  const confirmImportAll = () => {
    modal.confirm({
      centered: true,
      title: t("mcp.existingImportConfirmTitle", { count: importableForTab.length }),
      content: (
        <div className="flex flex-col gap-1">
          {importableForTab.map((entry) => (
            <div key={`${entry.target}:${entry.key}`} style={{ fontSize: 13 }}>
              {entry.name}
              <Typography.Text type="secondary" style={{ fontSize: 12, marginLeft: 6 }}>
                {targetLabel(entry.target as TargetKind)}
              </Typography.Text>
            </div>
          ))}
          <Typography.Text type="secondary" style={{ fontSize: 12, marginTop: 4 }}>
            {t("mcp.existingImportConfirmNote")}
          </Typography.Text>
        </div>
      ),
      okText: t("common.confirm"),
      cancelText: t("common.cancel"),
      onOk: () => {
        void importEntries(
          importableForTab.map((entry) => ({ target: entry.target, key: entry.key })),
        );
      },
    });
  };

  // ---------------------------------------------------------------------
  // 更新
  // ---------------------------------------------------------------------

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
      content: t("mcp.updateAllConfirm", { count: updateCount() }),
      onOk: async () => {
        try {
          const { successes, failures } = await updateAll();
          if (failures.length === 0) {
            void message.success(t("mcp.updateAllSuccess", { count: successes.length }));
          } else if (successes.length > 0) {
            void message.warning(
              t("mcp.updateAllPartial", { success: successes.length, failed: failures.length }),
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
      await checkUpdates({ force: true });
      void message.success(t("mcp.checkUpdatesSuccess"));
    } catch (error) {
      void message.error(errorText(error));
    }
  };

  // ---------------------------------------------------------------------
  // 卡片交互：总开关 / 逐客户端开关 / 冲突解决
  // ---------------------------------------------------------------------

  /** 卡片上的总开关：直接落库；禁用即从各目标写盘清单摘除（清理走后端 sweep）。 */
  const handleToggleEnabled = useCallback(
    async (record: McpServerSummary, enabled: boolean) => {
      try {
        const server = await getServer(record.id);
        const { sweep } = await saveServer({
          id: server.id,
          name: server.name,
          kind: server.kind,
          enabled,
          targets: server.targets,
          config: server.config,
          env: server.env,
          headers: server.headers,
        });
        void message.success(t("common.success"));
        showApplyOutcome(sweep);
        void loadDrift();
      } catch (error) {
        void message.error(errorText(error));
      }
    },
    [getServer, saveServer, message, showApplyOutcome, t, loadDrift],
  );

  /** 「需重新应用」横幅：把 DB 期望态一次性重写盘到该客户端（含清理孤儿）。 */
  const handleReapply = useCallback(
    async (target: TargetKind) => {
      setReapplying(true);
      try {
        const result = await applyServers([target]);
        const failed = result.results.filter((item) => !item.ok);
        if (failed.length === 0) void message.success(t("mcp.reapplySuccess"));
        else showApplyOutcome(result);
        void runScan();
      } catch (error) {
        void message.error(errorText(error));
      } finally {
        setReapplying(false);
      }
    },
    [applyServers, message, t, showApplyOutcome, runScan],
  );

  /** 逐客户端开关：改 targets → save，后端 sweep 负责写盘/清理。 */
  const handleToggleClient = useCallback(
    async (record: McpServerSummary, target: TargetKind, next: boolean) => {
      setBusyToggle({ id: record.id, target });
      try {
        const server = await getServer(record.id);
        const targets = next
          ? Array.from(new Set([...server.targets, target]))
          : server.targets.filter((item) => item !== target);
        const { sweep } = await saveServer({
          id: server.id,
          name: server.name,
          kind: server.kind,
          enabled: server.enabled,
          targets,
          config: server.config,
          env: server.env,
          headers: server.headers,
        });
        const failed = sweep.results.filter((item) => !item.ok);
        if (failed.length > 0) showApplyOutcome(sweep);
        // 应用后接管关系变了，重扫让 adoptable/冲突态刷新。
        void runScan();
      } catch (error) {
        void message.error(errorText(error));
      } finally {
        setBusyToggle(null);
      }
    },
    [getServer, saveServer, message, showApplyOutcome, runScan],
  );

  /** 打开同名冲突对比：左库内、右客户端；只取 config + 键名，密钥值不出后端。 */
  const openConflict = useCallback(
    async (record: McpServerSummary, target: TargetKind) => {
      const entry = (scanOutcome?.entries ?? []).find(
        (item) => item.name === record.name && item.target === target && item.nameConflict,
      );
      if (!entry) return;
      try {
        const mine = await getServer(record.id);
        const mineText = JSON.stringify(
          {
            config: mine.config,
            envKeys: Object.keys(mine.env ?? {}),
            headerKeys: Object.keys(mine.headers ?? {}),
          },
          null,
          2,
        );
        const theirsText = JSON.stringify(
          { config: entry.config, envKeys: entry.envKeys, headerKeys: entry.headerKeys },
          null,
          2,
        );
        setConflictLoc({ target: entry.target, key: entry.key });
        setConflict({
          serverName: record.name,
          targetLabel: targetLabel(target),
          mineText,
          theirsText,
        });
      } catch (error) {
        void message.error(errorText(error));
      }
    },
    [scanOutcome, getServer, targetLabel, message],
  );

  /** 用客户端的定义反向入库（后端撞重名会如实报错，用户可改名后重试）。 */
  const adoptTheirs = () => {
    const loc = conflictLoc;
    setConflict(null);
    setConflictLoc(null);
    if (loc) void importEntries([{ target: loc.target, key: loc.key }]);
  };

  const skipConflict = () => {
    setConflict(null);
    setConflictLoc(null);
  };

  // per-agent 视图：只列归属当前 agent 的 MCP（含该目标上的同名冲突），再按名称过滤。
  const visibleServers = useMemo(() => {
    const query = mineSearch.trim().toLowerCase();
    return servers.filter((server) => {
      const belongs =
        server.targets.includes(mcpTab) || conflictSet.has(`${server.name}::${mcpTab}`);
      if (!belongs) return false;
      if (query && !server.name.toLowerCase().includes(query)) return false;
      return true;
    });
  }, [servers, mineSearch, mcpTab, conflictSet]);

  // 当前 agent 的漂移状态 / 可纳管条目 / 扫描告警。
  const tabDrift = useMemo(() => drift.find((item) => item.target === mcpTab), [drift, mcpTab]);
  const importableForTab = useMemo(
    () => importable.filter((entry) => entry.target === mcpTab),
    [importable, mcpTab],
  );
  const tabWarnings = useMemo(
    () => (scanOutcome?.warnings ?? []).filter((warning) => warning.target === mcpTab),
    [scanOutcome, mcpTab],
  );

  const editing = editingId !== null;
  const modalTitle = editing
    ? t("mcp.edit")
    : draft
      ? t("mcp.registryInstall")
      : t("mcp.add");

  return (
    <div className="flex h-full min-h-0">
      <div
        className="h-full w-56 shrink-0"
        style={{ borderRight: "1px solid var(--border-color)", backgroundColor: token.colorBgContainer }}
      >
        <McpSidebar />
      </div>
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-auto p-6"
        style={{ backgroundColor: token.colorBgElevated }}
      >
        <div className="flex flex-wrap items-center gap-2">
          <Typography.Title level={4} style={{ margin: 0 }}>
            {targetLabel(mcpTab)}
            {hasAnyUpdate() && (
              <Tag color="orange" style={{ marginLeft: 8, fontSize: 12 }}>
                {t("mcp.updatesAvailable", { count: updateCount() })}
              </Tag>
            )}
          </Typography.Title>
          <span style={{ flex: 1 }} />
          <Tooltip title={t("mcp.existingRescan")}>
            <Button
              icon={<ReloadOutlined spin={scanning} />}
              loading={scanning}
              onClick={() => void runScan({ notify: true })}
            >
              {t("mcp.existingRescan")}
            </Button>
          </Tooltip>
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
            <Button icon={<SyncOutlined />} onClick={() => void handleUpdateAll()}>
              {t("mcp.updateAll")} ({updateCount()})
            </Button>
          )}
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
            {t("mcp.add")}
          </Button>
        </div>

        <Input
          value={mineSearch}
          onChange={(event) => setMineSearch(event.target.value)}
          placeholder={t("mcp.mineSearchPlaceholder")}
          allowClear
          prefix={<SearchOutlined style={{ color: token.colorTextTertiary }} />}
          style={{ maxWidth: 280 }}
        />

        {/* 需重新应用：DB 期望态 ≠ 客户端实际态时的一键重写盘横幅（R3）。 */}
        {tabDrift?.drift && (
          <Alert
            type="warning"
            showIcon
            message={t("mcp.driftBannerTitle")}
            description={
              <div className="flex flex-col gap-1" style={{ fontSize: 12 }}>
                <span>
                  {t("mcp.driftBannerDetail", {
                    write: tabDrift.toWrite,
                    clean: tabDrift.toClean,
                  })}
                </span>
                {tabDrift.conflicts.length > 0 && (
                  <span>
                    {t("mcp.driftBannerConflicts", { names: tabDrift.conflicts.join("、") })}
                  </span>
                )}
                {tabDrift.error && (
                  <span style={{ color: token.colorError }}>{tabDrift.error}</span>
                )}
              </div>
            }
            action={
              <Button
                size="small"
                type="primary"
                loading={reapplying}
                onClick={() => void handleReapply(mcpTab)}
              >
                {t("mcp.reapply")}
              </Button>
            }
          />
        )}

      {/* 当前 agent 的托管 MCP 列表（逐目标单开关） */}
      {loading && servers.length === 0 ? (
        <Card loading />
      ) : visibleServers.length === 0 ? (
        <Card>
          <Empty description={servers.length > 0 ? t("mcp.mineNoResults") : t("mcp.emptyTitle")} />
        </Card>
      ) : (
        <div className="flex flex-col gap-2">
          {visibleServers.map((server) => (
            <McpCard
              key={server.id}
              server={server}
              target={mcpTab}
              state={clientStatesOf(server)[mcpTab]}
              updateStatus={updateStatuses.find((item) => item.id === server.id)}
              updating={updating[server.id] || false}
              busy={busyToggle?.id === server.id && busyToggle.target === mcpTab}
              targetLabel={targetLabel}
              onToggleClient={(record, target, next) => void handleToggleClient(record, target, next)}
              onResolveConflict={(record, target) => void openConflict(record, target)}
              onToggleEnabled={(record, enabled) => void handleToggleEnabled(record, enabled)}
              onEdit={(id) => void openEdit(id)}
              onDelete={handleDelete}
              onUpdate={(id) => void handleUpdate(id)}
            />
          ))}
        </div>
      )}

      {/* 扫描到的「野生」MCP：一进来自动列出，一键纳管收编。 */}
      {tabWarnings.length > 0 && (
        <div className="flex flex-col gap-1">
          {tabWarnings.map((warning) => (
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
        </div>
      )}

      {importableForTab.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <Typography.Text strong style={{ fontSize: 13 }}>
              {t("mcp.unmanagedSectionTitle", { count: importableForTab.length })}
            </Typography.Text>
            <span style={{ flex: 1 }} />
            <Button
              size="small"
              type="primary"
              loading={importing}
              onClick={confirmImportAll}
            >
              {t("mcp.existingImportAll", { count: importableForTab.length })}
            </Button>
          </div>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.unmanagedSectionDesc")}
          </Typography.Text>
          {importableForTab.map((entry) => (
            <UnmanagedMcpCard
              key={`${entry.target}:${entry.key}`}
              entry={entry}
              importing={importing}
              targetLabel={targetLabel}
              onImport={(item) => void importEntries([{ target: item.target, key: item.key }])}
            />
          ))}
        </div>
      )}

      {/* 密钥落盘 + 各客户端配置路径提示 */}
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
      </div>

      {/* 免密钥仓库条目：没有已有目标时问一次装到哪儿 */}
      <Modal
        centered
        destroyOnHidden
        mask={{ enabled: true }}
        width={520}
        open={targetPicker !== null}
        title={t("mcp.registryTargetPickerTitle")}
        onCancel={() => setTargetPicker(null)}
        onOk={() => {
          const picked = targetPicker;
          if (!picked) return;
          setTargetPicker(null);
          void installDirect(picked, pickerTargets);
        }}
        okText={t("mcp.registryInstall")}
        cancelText={t("common.cancel")}
        okButtonProps={{ disabled: pickerTargets.length === 0 }}
      >
        <div className="flex flex-col gap-2">
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.registryTargetPickerDesc", { name: targetPicker?.name ?? "" })}
          </Typography.Text>
          <Checkbox.Group
            value={pickerTargets}
            onChange={(values) => setPickerTargets(values as TargetKind[])}
            options={MCP_AGENT_TABS.map((target) => ({ label: targetLabel(target), value: target }))}
          />
        </div>
      </Modal>

      <ConflictModal
        open={conflict !== null}
        context={conflict}
        onAdoptTheirs={adoptTheirs}
        onSkip={skipConflict}
        onCancel={skipConflict}
      />

      {/* 添加 / 编辑：新建时可切「手动填写 / 从仓库安装」 */}
      <Modal
        centered
        destroyOnHidden
        mask={{ enabled: true }}
        width={560}
        open={open}
        title={modalTitle}
        onCancel={() => setOpen(false)}
        footer={
          addSource === "registry" && !editing && !draft
            ? [
                <Button key="close" onClick={() => setOpen(false)}>
                  {t("common.cancel")}
                </Button>,
              ]
            : undefined
        }
        onOk={() => void handleSave()}
        okText={t("common.save")}
        cancelText={t("common.cancel")}
      >
        {!editing && !draft && (
          <Segmented
            block
            style={{ marginBottom: 16 }}
            value={addSource}
            onChange={(value) => setAddSource(value as AddSource)}
            options={[
              { value: "manual", label: t("mcp.addSourceManual") },
              { value: "registry", label: t("mcp.addSourceRegistry") },
            ]}
          />
        )}

        {addSource === "registry" && !editing && !draft ? (
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
                style={{ maxHeight: 320, overflow: "auto" }}
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
                            {install &&
                              (requiredCount > 0 ? (
                                <Tag color="orange">
                                  {t("mcp.registryNeedsSecrets", { count: requiredCount })}
                                </Tag>
                              ) : (
                                <Tag color="green">{t("mcp.registryNoSecrets")}</Tag>
                              ))}
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
                            {!local && host && (
                              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                                {t("mcp.registryRemoteHost", { host })}
                              </Typography.Text>
                            )}
                            {install && (
                              <Typography.Text code style={{ fontSize: 12 }}>
                                {describeDraft(install, t)}
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
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("mcp.registryNoResults")} />
            )}

            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {t("mcp.registrySourceHint")}
            </Typography.Text>
          </div>
        ) : (
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
                options={MCP_AGENT_TABS.map((target) => ({ label: targetLabel(target), value: target }))}
              />
            </Form.Item>

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
        )}
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
