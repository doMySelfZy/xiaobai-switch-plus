import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSiteStore } from "@/stores";
import type { ProtocolDetectionResult, Site } from "@/types/domain";
import { SiteFormModal } from "./SiteFormModal";
import "@/i18n";

// 连接测试没有 browser mock，必须在 invoke 这一层控制时序（进度 / 取消 / 迟到结果）。
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: invokeMock };
});

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

/** 表单打开时只会读这两个次要命令，其余命令直接失败，避免测试静默通过。 */
function defaultInvokeImpl(cmd: string): Promise<unknown> {
  if (cmd === "get_site_proxy_headers") return Promise.resolve([]);
  if (cmd === "get_site_newapi_token") return Promise.resolve("");
  return Promise.reject({ code: "internal", message: `unexpected command: ${cmd}` });
}

/** 展开高级配置、填好必填项并点「测试连接」。 */
function startProtocolTest() {
  fireEvent.click(screen.getByText("高级配置"));
  fireEvent.change(screen.getByPlaceholderText("https://api.example.com"), {
    target: { value: "https://api.example.com" },
  });
  fireEvent.change(screen.getByPlaceholderText("sk-..."), { target: { value: "sk-one" } });
  fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
}

function sampleSite(): Site {
  return {
    id: "s1",
    name: "Relay",
    baseUrl: "https://a.example.com",
    baseUrls: ["https://a.example.com", "https://b.example.com"],
    keyPrefix: "sk-t…",
    quotaRevision: "rev-1",
    hasKey: true,
    protocol: "openai_compatible",
    claudeAuthKeyStyle: "anthropic_auth_token",
    notes: null,
    enabled: true,
    sortOrder: 0,
    selectedModelId: null,
    lastModelFetchAt: null,
    lastModelFetchLatencyMs: null,
    lastModelFetchError: null,
    createdAt: 1,
    updatedAt: 1,
    activeApiKeyId: "k1",
    apiKeys: [
      {
        id: "k1",
        label: "K 1",
        keyPrefix: "sk-t…",
        isActive: true,
        quotaRevision: "rev-1",
        selectedModelId: null,
        lastModelFetchAt: null,
        lastModelFetchLatencyMs: null,
        lastModelFetchError: null,
      },
    ],
  };
}

const originalGetSiteApiKey = useSiteStore.getState().getSiteApiKey;

describe("SiteFormModal base url list", () => {
  beforeEach(() => {
    useSiteStore.setState({
      getSiteApiKey: vi.fn(() => new Promise<string>(() => undefined)),
    });
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => defaultInvokeImpl(cmd));
  });

  afterEach(() => {
    useSiteStore.setState({ getSiteApiKey: originalGetSiteApiKey });
  });

  it("starts with one api key row and can add another", () => {
    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );

    const keyInput = screen.getByPlaceholderText("sk-...");
    expect(keyInput).toHaveAttribute("type", "text");
    expect(screen.queryByRole("button", { name: "Show" })).not.toBeInTheDocument();
    expect(screen.getAllByPlaceholderText("sk-...")).toHaveLength(1);
    fireEvent.click(screen.getAllByRole("button", { name: "添加密钥" })[0]);
    expect(screen.getAllByPlaceholderText("sk-...")).toHaveLength(2);
    expect(screen.getByText("可一次添加多个密钥，第一项为当前密钥")).toBeInTheDocument();
  });

  it("creates a site with multiple api keys in one save", async () => {
    const originalCreateSite = useSiteStore.getState().createSite;
    const createSite = vi.fn().mockResolvedValue(sampleSite());
    useSiteStore.setState({ createSite });

    try {
      render(
        <Wrapper>
          <SiteFormModal open site={null} onClose={() => undefined} />
        </Wrapper>,
      );

      fireEvent.change(screen.getByPlaceholderText("My Relay"), {
        target: { value: "Relay" },
      });
      fireEvent.change(screen.getByPlaceholderText("https://api.example.com"), {
        target: { value: "https://api.example.com" },
      });
      fireEvent.change(screen.getByPlaceholderText("sk-..."), {
        target: { value: "sk-one" },
      });
      fireEvent.click(screen.getAllByRole("button", { name: "添加密钥" })[0]);
      fireEvent.change(screen.getAllByPlaceholderText("sk-...")[1]!, {
        target: { value: "sk-two" },
      });
      fireEvent.click(screen.getByRole("button", { name: /保.*存/ }));

      await waitFor(() => expect(createSite).toHaveBeenCalledTimes(1));
      expect(createSite).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Relay",
          apiKey: "sk-one",
          extraApiKeys: [{ label: null, apiKey: "sk-two" }],
        }),
      );
    } finally {
      useSiteStore.setState({ createSite: originalCreateSite });
    }
  });

  it("starts with one row and can add another", () => {
    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );

    expect(screen.getAllByPlaceholderText("https://api.example.com")).toHaveLength(1);
    fireEvent.click(screen.getAllByRole("button", { name: "添加线路" })[0]);
    expect(screen.getAllByPlaceholderText("https://api.example.com")).toHaveLength(2);
    expect(screen.getByText("第一项为当前 / 默认线路")).toBeInTheDocument();
  });

  it("wires dnd-kit sortable handles on each base url row", () => {
    render(
      <Wrapper>
        <SiteFormModal open site={sampleSite()} onClose={() => undefined} />
      </Wrapper>,
    );

    const grips = screen.getAllByRole("button", { name: "线路" });
    expect(grips).toHaveLength(2);
    expect(grips[0]).toHaveAttribute("aria-roledescription", "sortable");
    expect(grips[1]).toHaveAttribute("aria-roledescription", "sortable");
    expect(grips[0]).toHaveAttribute("aria-disabled", "false");

    const inputs = screen.getAllByPlaceholderText("https://api.example.com");
    expect(inputs[0]).toHaveValue("https://a.example.com");
    expect(inputs[1]).toHaveValue("https://b.example.com");
  });

  it("shows the complete saved key when editing a site", async () => {
    const getSiteApiKey = vi.fn().mockResolvedValue("sk-full-secret");
    useSiteStore.setState({ getSiteApiKey });
    render(
      <Wrapper>
        <SiteFormModal open site={sampleSite()} onClose={() => undefined} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByPlaceholderText("sk-...")).toHaveValue("sk-full-secret");
    });
    const apiKeyInput = screen.getByPlaceholderText("sk-...");
    expect(apiKeyInput).toHaveAttribute("type", "text");
    expect(screen.queryByRole("button", { name: "Show" })).not.toBeInTheDocument();
    expect(getSiteApiKey).toHaveBeenCalledWith("s1", "k1");
  });

  it("does not write the complete key back when it is unchanged", async () => {
    const originalUpdateSite = useSiteStore.getState().updateSite;
    useSiteStore.setState({
      getSiteApiKey: vi.fn().mockResolvedValue("sk-full-secret"),
    });
    const updateSite = vi.fn().mockResolvedValue(sampleSite());
    useSiteStore.setState({ updateSite });

    try {
      render(
        <Wrapper>
          <SiteFormModal open site={sampleSite()} onClose={() => undefined} />
        </Wrapper>,
      );

      await waitFor(() => {
        expect(screen.getByPlaceholderText("sk-...")).toHaveValue("sk-full-secret");
      });
      fireEvent.click(screen.getByRole("button", { name: /保.*存/ }));

      await waitFor(() => expect(updateSite).toHaveBeenCalledTimes(1));
      expect(updateSite).toHaveBeenCalledWith(
        "s1",
        expect.objectContaining({
          apiKeys: [{ id: "k1", label: "K 1", apiKey: "sk-full-secret" }],
        }),
      );
    } finally {
      useSiteStore.setState({ updateSite: originalUpdateSite });
    }
  });

  it("writes a replacement key when the complete saved key is overwritten", async () => {
    const originalUpdateSite = useSiteStore.getState().updateSite;
    useSiteStore.setState({
      getSiteApiKey: vi.fn().mockResolvedValue("sk-full-secret"),
    });
    const updateSite = vi.fn().mockResolvedValue(sampleSite());
    useSiteStore.setState({ updateSite });

    try {
      render(
        <Wrapper>
          <SiteFormModal open site={sampleSite()} onClose={() => undefined} />
        </Wrapper>,
      );

      await waitFor(() => {
        expect(screen.getByPlaceholderText("sk-...")).toHaveValue("sk-full-secret");
      });
      fireEvent.change(screen.getByPlaceholderText("sk-..."), {
        target: { value: "sk-replacement" },
      });
      fireEvent.click(screen.getByRole("button", { name: /保.*存/ }));

      await waitFor(() => expect(updateSite).toHaveBeenCalledTimes(1));
      expect(updateSite).toHaveBeenCalledWith(
        "s1",
        expect.objectContaining({
          apiKeys: [{ id: "k1", label: "K 1", apiKey: "sk-replacement" }],
        }),
      );
    } finally {
      useSiteStore.setState({ updateSite: originalUpdateSite });
    }
  });

  it("edits multiple keys in the same list used for create", async () => {
    const originalUpdateSite = useSiteStore.getState().updateSite;
    const updateSite = vi.fn().mockResolvedValue(sampleSite());
    useSiteStore.setState({
      getSiteApiKey: vi.fn().mockResolvedValue("sk-full-secret"),
      updateSite,
    });

    try {
      render(
        <Wrapper>
          <SiteFormModal open site={sampleSite()} onClose={() => undefined} />
        </Wrapper>,
      );

      await waitFor(() => {
        expect(screen.getByPlaceholderText("sk-...")).toHaveValue("sk-full-secret");
      });
      fireEvent.click(screen.getAllByRole("button", { name: "添加密钥" })[0]);
      fireEvent.change(screen.getAllByPlaceholderText("sk-...")[1]!, {
        target: { value: "sk-two" },
      });
      fireEvent.click(screen.getByRole("button", { name: /保.*存/ }));

      await waitFor(() => expect(updateSite).toHaveBeenCalledTimes(1));
      expect(updateSite).toHaveBeenCalledWith(
        "s1",
        expect.objectContaining({
          apiKeys: [
            { id: "k1", label: "K 1", apiKey: "sk-full-secret" },
            { id: null, label: null, apiKey: "sk-two" },
          ],
        }),
      );
    } finally {
      useSiteStore.setState({ updateSite: originalUpdateSite });
    }
  });

  it("keeps advanced config and Codex capabilities collapsed by default", () => {
    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );

    expect(screen.getByText("高级配置")).toBeInTheDocument();
    expect(screen.getByText("Codex私有能力")).toBeInTheDocument();
    expect(document.querySelector(".ant-collapse-item-active")).toBeNull();
    expect(screen.queryByText("连接协议")).not.toBeInTheDocument();
    expect(screen.queryByText("备注")).not.toBeInTheDocument();
    expect(screen.queryByText("识图支持")).not.toBeInTheDocument();
  });

  it("reveals protocol and notes after expanding advanced config", () => {
    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );

    fireEvent.click(screen.getByText("高级配置"));
    expect(screen.getByText("连接协议")).toBeInTheDocument();
    expect(screen.getByText("备注")).toBeInTheDocument();
    expect(screen.queryByText("识图支持")).not.toBeInTheDocument();
  });

  it("expands advanced config when notes or a non-default protocol are set", () => {
    render(
      <Wrapper>
        <SiteFormModal
          open
          site={{ ...sampleSite(), protocol: "anthropic", notes: "keep this" }}
          onClose={() => undefined}
        />
      </Wrapper>,
    );

    expect(screen.getByText("连接协议")).toBeInTheDocument();
    expect(screen.getByDisplayValue("keep this")).toBeInTheDocument();
    expect(screen.queryByText("识图支持")).not.toBeInTheDocument();
  });

  it("expands Codex capabilities when a preset is already on", () => {
    render(
      <Wrapper>
        <SiteFormModal
          open
          site={{ ...sampleSite(), capabilities: { "codex-vision": true } }}
          onClose={() => undefined}
        />
      </Wrapper>,
    );

    expect(document.querySelector(".ant-collapse-item-active")).not.toBeNull();
    expect(screen.getByText("识图支持")).toBeInTheDocument();
    expect(screen.queryByText("连接协议")).not.toBeInTheDocument();
  });

  it("keeps the dialog inside the viewport when content grows", () => {
    render(
      <Wrapper>
        <SiteFormModal
          open
          site={{ ...sampleSite(), capabilities: { "codex-vision": true } }}
          onClose={() => undefined}
        />
      </Wrapper>,
    );

    const container = document.querySelector(".ant-modal-container");
    expect(container).toHaveStyle({ maxHeight: "calc(100vh - 32px)" });
    const body = document.querySelector(".ant-modal-body");
    expect(body).toHaveStyle({ overflowY: "auto" });
  });

  it("prefills create form from a deep-link payload", () => {
    render(
      <Wrapper>
        <SiteFormModal
          open
          site={null}
          initialValues={{
            name: "Imported",
            baseUrls: ["https://a.example.com", "https://b.example.com"],
            protocol: "anthropic",
            notes: "from link",
          }}
          onClose={() => undefined}
        />
      </Wrapper>,
    );

    expect(screen.getByDisplayValue("Imported")).toBeInTheDocument();
    expect(screen.getByDisplayValue("from link")).toBeInTheDocument();
    const inputs = screen.getAllByPlaceholderText("https://api.example.com");
    expect(inputs[0]).toHaveValue("https://a.example.com");
    expect(inputs[1]).toHaveValue("https://b.example.com");
  });

  it("shows staged progress during detection and drops the result after 取消等待", async () => {
    const pending = deferred<ProtocolDetectionResult>();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "test_site_connection" ? pending.promise : defaultInvokeImpl(cmd),
    );

    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );
    startProtocolTest();

    expect(await screen.findByText("正在尝试 OpenAI / Anthropic 鉴权组合…")).toBeInTheDocument();
    expect(screen.getByText(/已等待 \d+ 秒/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "取消等待" }));
    expect(await screen.findByText("已停止等待（检测仍在后台完成）")).toBeInTheDocument();
    // 取消入口与进度都撤销（主按钮的 loading 图标在 jsdom 里停在退场动画上，不断言它）。
    expect(screen.queryByRole("button", { name: "取消等待" })).toBeNull();
    expect(screen.queryByText("正在尝试 OpenAI / Anthropic 鉴权组合…")).toBeNull();
    expect(screen.queryByText(/已等待/)).toBeNull();

    // 后端命令没有取消通道，仍会跑完：迟到的结果不得回填表单，也不该弹成功提示。
    await act(async () => {
      pending.resolve({
        detectedProtocol: "anthropic",
        modelPreview: [],
        endpoint: "https://api.example.com/v1/models",
      });
      await pending.promise;
    });
    expect(screen.getByText("已停止等待（检测仍在后台完成）")).toBeInTheDocument();
    expect(screen.queryByText(/检测到/)).toBeNull();
  });

  it("tells an auth rejection apart from an unreachable site", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "test_site_connection"
        ? Promise.reject({ code: "unauthorized", message: "unauthorized" })
        : defaultInvokeImpl(cmd),
    );

    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );
    startProtocolTest();

    const dialog = screen.getByRole("dialog");
    expect(
      await within(dialog).findByText("认证被拒绝：请检查 API Key 是否与该站点匹配"),
    ).toBeInTheDocument();
  });

  it("reports a timeout as unreachable instead of as bad credentials", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "test_site_connection"
        ? Promise.reject({ code: "timeout", message: "request timed out" })
        : defaultInvokeImpl(cmd),
    );

    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );
    startProtocolTest();

    const dialog = screen.getByRole("dialog");
    expect(
      await within(dialog).findByText("无法连接站点：网络错误或请求超时"),
    ).toBeInTheDocument();
  });

  it("classifies an aggregated detection failure carrying HTTP 401 as an auth rejection", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "test_site_connection"
        ? Promise.reject({
            code: "protocol_detection_failed",
            message: "HTTP 401 from https://api.example.com/v1/models",
          })
        : defaultInvokeImpl(cmd),
    );

    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );
    startProtocolTest();

    const dialog = screen.getByRole("dialog");
    expect(
      await within(dialog).findByText("认证被拒绝：请检查 API Key 是否与该站点匹配"),
    ).toBeInTheDocument();
  });

  it("falls back to the technical detail for an unclassified failure", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "test_site_connection"
        ? Promise.reject({
            code: "protocol_detection_failed",
            message: "Could not detect protocol. Both OpenAI and Anthropic endpoints failed.",
          })
        : defaultInvokeImpl(cmd),
    );

    render(
      <Wrapper>
        <SiteFormModal open site={null} onClose={() => undefined} />
      </Wrapper>,
    );
    startProtocolTest();

    const dialog = screen.getByRole("dialog");
    expect(
      await within(dialog).findByText(
        /连接测试失败：Could not detect protocol/,
      ),
    ).toBeInTheDocument();
  });
});
