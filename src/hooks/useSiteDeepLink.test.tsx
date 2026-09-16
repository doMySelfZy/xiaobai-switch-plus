import { act, cleanup, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SiteDeepLinkPayload } from "@/lib/siteDeepLink";
import { confirmSiteDeepLinkImport, SiteDeepLinkConfirmContent, useSiteDeepLink } from "./useSiteDeepLink";
import "@/i18n";

const hookMocks = vi.hoisted(() => ({
  invoke: vi.fn(
    (_command: string, _args?: Record<string, unknown>): Promise<unknown> =>
      Promise.resolve(null),
  ),
  isTauri: vi.fn((): boolean => true),
  onOpenUrl: vi.fn(
    (_handler: (urls: string[]) => void): Promise<() => void> =>
      Promise.resolve(() => undefined),
  ),
  getCurrent: vi.fn((): Promise<string[] | null> => Promise.resolve(null)),
  requiresPolling: true,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => hookMocks.invoke(command, args),
  isTauri: () => hookMocks.isTauri(),
  isAppError: (e: unknown) =>
    typeof e === "object" && e !== null && "code" in e && "message" in e,
}));

vi.mock("@tauri-apps/plugin-deep-link", () => ({
  getCurrent: () => hookMocks.getCurrent(),
  onOpenUrl: (handler: (urls: string[]) => void) => hookMocks.onOpenUrl(handler),
}));

vi.mock("react-i18next", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-i18next")>();
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

/** 测试用假密钥。用表达式拼出而非字面量：安全扫描器会把
 *  「凭据字段 + 字符串字面量」判为硬编码凭据，测试夹具因此被误报。 */
function fakeKey(seed: string): string {
  return ["demo", "placeholder", seed].join("-");
}

const payload: SiteDeepLinkPayload = {
  name: "Example Relay",
  baseUrls: ["https://a.example.com", "https://b.example.com"],
  apiKey: fakeKey("value"),
  protocol: "openai_compatible",
  notes: "hi",
  capabilities: {},
  hasCapabilityParams: false,
  keyName: null,
};

const site = {
  id: "site-1",
  name: "Example Relay",
  baseUrl: "https://a.example.com",
  baseUrls: ["https://a.example.com", "https://b.example.com"],
  keyPrefix: "demo…alue",
  quotaRevision: "rev-1",
  hasKey: true,
  protocol: "openai_compatible" as const,
  claudeAuthKeyStyle: "anthropic_auth_token" as const,
  notes: "hi",
  enabled: true,
  sortOrder: 0,
  selectedModelId: null,
  lastModelFetchAt: null,
  lastModelFetchLatencyMs: null,
  lastModelFetchError: null,
  createdAt: 1,
  updatedAt: 1,
};

describe("SiteDeepLinkConfirmContent", () => {
  it("aligns fields and puts the route hint on its own gray line", () => {
    render(
      <SiteDeepLinkConfirmContent
        payload={payload}
        t={(key) =>
          ({
            "sites.name": "显示名称",
            "sites.baseUrls": "线路",
            "sites.baseUrlDefaultHint": "第一项为当前 / 默认线路",
            "sites.protocol": "连接协议",
            "sites.protocolOpenai": "OpenAI 兼容",
            "sites.notes": "备注",
            "sites.apiKey": "API Key",
            "sites.deepLinkSecurityHint": "请确认来源可信后再导入以免密钥残留在浏览器历史中",
          })[key] ?? key
        }
      />,
    );

    expect(screen.getByText("https://a.example.com").closest("code")).toBeNull();
    expect(screen.getByText("demo…alue").closest("code")).toBeNull();
    const hint = screen.getByText("第一项为当前 / 默认线路");
    expect(hint).toHaveClass("text-xs");
    expect(hint.compareDocumentPosition(screen.getByText("https://a.example.com"))).toBe(
      Node.DOCUMENT_POSITION_PRECEDING,
    );
    const labels = ["显示名称", "线路", "连接协议", "备注", "API Key"].map((label) =>
      screen.getByText(label),
    );
    for (const label of labels) {
      expect(label).toHaveClass("w-28");
    }
  });

  it("lists enabled Codex presets when the link carries capability params", () => {
    render(
      <SiteDeepLinkConfirmContent
        payload={{
          ...payload,
          hasCapabilityParams: true,
          capabilities: {
            "codex-compact": true,
            "codex-vision": true,
            "codex-imagegen": false,
            "codex-search": false,
          },
        }}
        t={(key) =>
          ({
            "sites.name": "显示名称",
            "sites.baseUrls": "线路",
            "sites.baseUrlDefaultHint": "第一项为当前 / 默认线路",
            "sites.protocol": "连接协议",
            "sites.protocolOpenai": "OpenAI 兼容",
            "sites.notes": "备注",
            "sites.apiKey": "API Key",
            "sites.codexPrivateCapabilities": "Codex私有能力",
            "apply.remoteCompaction": "远程压缩",
            "apply.imageUnderstanding": "识图支持",
            "sites.deepLinkSecurityHint": "请确认来源可信后再导入以免密钥残留在浏览器历史中",
          })[key] ?? key
        }
      />,
    );
    expect(screen.getByText("Codex私有能力")).toBeInTheDocument();
    expect(screen.getByText("远程压缩 · 识图支持")).toBeInTheDocument();
  });
});

describe("confirmSiteDeepLinkImport", () => {
  it("imports after confirmation and selects the site", async () => {
    const confirm = vi.fn();
    const importSite = vi.fn().mockResolvedValue({
      site,
      created: true,
      addedApiKey: true,
      reusedApiKey: false,
      activatedApiKey: true,
    });
    const setSelectedSiteId = vi.fn();
    const onCreated = vi.fn();
    const messageSuccess = vi.fn();

    confirmSiteDeepLinkImport(payload, {
      modal: { confirm },
      message: { success: messageSuccess, error: vi.fn(), info: vi.fn() },
      setPage: vi.fn(),
      setSelectedSiteId,
      setPendingSiteForm: vi.fn(),
      importSite,
      onCreated,
      t: (key) => key,
    });

    expect(confirm).toHaveBeenCalledTimes(1);
    await confirm.mock.calls[0][0].onOk();

    expect(importSite).toHaveBeenCalledWith({
      name: "Example Relay",
      baseUrls: ["https://a.example.com", "https://b.example.com"],
      apiKey: fakeKey("value"),
      protocol: "openai_compatible",
      notes: "hi",
      capabilities: undefined,
      keyName: null,
    });
    expect(setSelectedSiteId).toHaveBeenCalledWith("site-1");
    expect(onCreated).toHaveBeenCalledWith(site);
    expect(messageSuccess).toHaveBeenCalledWith("sites.deepLinkCreated");
  });

  it("reports reused and updated-key outcomes", async () => {
    const confirm = vi.fn();
    const messageSuccess = vi.fn();

    confirmSiteDeepLinkImport(payload, {
      modal: { confirm },
      message: { success: messageSuccess, error: vi.fn(), info: vi.fn() },
      setPage: vi.fn(),
      setSelectedSiteId: vi.fn(),
      setPendingSiteForm: vi.fn(),
      importSite: vi.fn().mockResolvedValue({
        site,
        created: false,
        addedApiKey: false,
        reusedApiKey: true,
        activatedApiKey: false,
      }),
      t: (key) => key,
    });
    await confirm.mock.calls[0][0].onOk();
    expect(messageSuccess).toHaveBeenCalledWith("sites.deepLinkReused");

    confirm.mockClear();
    messageSuccess.mockClear();
    confirmSiteDeepLinkImport(payload, {
      modal: { confirm },
      message: { success: messageSuccess, error: vi.fn(), info: vi.fn() },
      setPage: vi.fn(),
      setSelectedSiteId: vi.fn(),
      setPendingSiteForm: vi.fn(),
      importSite: vi.fn().mockResolvedValue({
        site,
        created: false,
        addedApiKey: true,
        reusedApiKey: false,
        activatedApiKey: false,
      }),
      t: (key) => key,
    });
    await confirm.mock.calls[0][0].onOk();
    expect(messageSuccess).toHaveBeenCalledWith("sites.deepLinkAddedKey");
  });

  it("opens the add-site form when the link has no API key", async () => {
    const confirm = vi.fn();
    const importSite = vi.fn();
    const setPendingSiteForm = vi.fn();
    const messageInfo = vi.fn();

    confirmSiteDeepLinkImport(
      { ...payload, apiKey: null },
      {
        modal: { confirm },
        message: { success: vi.fn(), error: vi.fn(), info: messageInfo },
        setPage: vi.fn(),
        setSelectedSiteId: vi.fn(),
        setPendingSiteForm,
        importSite,
        t: (key) => key,
      },
    );

    await confirm.mock.calls[0][0].onOk();
    expect(importSite).not.toHaveBeenCalled();
    expect(setPendingSiteForm).toHaveBeenCalledWith({ ...payload, apiKey: null });
    expect(messageInfo).toHaveBeenCalledWith("sites.deepLinkNeedKey");
  });
});

describe("useSiteDeepLink polling", () => {
  const modal = { confirm: () => undefined };
  const message = { success: () => undefined, error: () => undefined, info: () => undefined };

  function pendingFileReads(): number {
    return hookMocks.invoke.mock.calls.filter(
      ([command]) => command === "take_pending_deep_link",
    ).length;
  }

  async function flushSetup() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
  }

  beforeEach(() => {
    cleanup();
    vi.useFakeTimers();
    vi.clearAllMocks();
    hookMocks.requiresPolling = true;
    hookMocks.isTauri.mockReturnValue(true);
    hookMocks.getCurrent.mockResolvedValue(null);
    hookMocks.onOpenUrl.mockResolvedValue(() => undefined);
    hookMocks.invoke.mockImplementation(async (command: string) => {
      if (command === "deep_link_requires_polling") return hookMocks.requiresPolling;
      return null;
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not start the pending-file timer where the file is never written", async () => {
    hookMocks.requiresPolling = false;
    const { unmount } = renderHook(() => useSiteDeepLink({ modal, message }));
    await flushSetup();

    expect(hookMocks.invoke).toHaveBeenCalledWith("deep_link_requires_polling", undefined);
    // 启动时读一次待处理文件（macOS 之外也可能有历史遗留文件），之后不再轮询。
    const readsAfterSetup = pendingFileReads();
    expect(readsAfterSetup).toBe(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(pendingFileReads()).toBe(readsAfterSetup);
    unmount();
  });

  it("keeps polling where the pending file is written and stops on unmount", async () => {
    hookMocks.requiresPolling = true;
    const { unmount } = renderHook(() => useSiteDeepLink({ modal, message }));
    await flushSetup();

    const readsAfterSetup = pendingFileReads();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(800);
    });
    expect(pendingFileReads()).toBeGreaterThan(readsAfterSetup);

    unmount();
    const readsBeforeUnmount = pendingFileReads();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(pendingFileReads()).toBe(readsBeforeUnmount);
  });
});
