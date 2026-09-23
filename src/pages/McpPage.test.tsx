import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  handleBrowserCommand,
  resetBrowserMock,
  setBrowserMcpTakeoverFailTargets,
} from "@/lib/browserMock";
import { useMcpStore } from "@/stores/mcpStore";
import { useUIStore } from "@/stores/uiStore";
import { McpPage } from "./McpPage";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

function resetMcpStore() {
  useMcpStore.setState({ servers: [], loading: false, drift: [] });
  // per-agent 外壳默认停在 Claude Code。
  useUIStore.setState({ mcpTab: "claude_code" });
}

/** antd 会在两个汉字之间插入空格，这里统一用容忍空白的匹配。 */
function saveButton(): HTMLElement {
  return screen.getByRole("button", { name: /保\s*存/ });
}

/** 顶栏「添加 MCP 服务」按钮：打开新建弹窗。 */
function addButton(): HTMLElement {
  return screen.getByRole("button", { name: /添加 MCP 服务/ });
}

/** 点击左侧 agent 侧栏切换当前目标。 */
function switchTab(name: RegExp) {
  fireEvent.click(screen.getByRole("menuitem", { name }));
}

/** 打开新建弹窗并停在默认的「手动填写」页。 */
async function openManualAdd(): Promise<void> {
  fireEvent.click(addButton());
  await screen.findByLabelText("服务名称");
}

/** 打开新建弹窗并切到「从仓库安装」页。 */
async function openRegistry(): Promise<void> {
  fireEvent.click(addButton());
  fireEvent.click(await screen.findByRole("radio", { name: /从仓库安装/ }));
}

function managedCard(name: string): HTMLElement {
  const card = screen.getByText(name).closest('[data-testid="mcp-card"]');
  if (!card) throw new Error(`managed card not found: ${name}`);
  return card as HTMLElement;
}

function unmanagedCard(name: string): HTMLElement {
  const card = screen.getByText(name).closest('[data-testid="mcp-unmanaged-card"]');
  if (!card) throw new Error(`unmanaged card not found: ${name}`);
  return card as HTMLElement;
}

async function seedServer(overrides: Record<string, unknown> = {}) {
  return handleBrowserCommand("save_mcp_server", {
    input: {
      name: "demo",
      kind: "stdio",
      enabled: true,
      targets: ["claude_code"],
      config: { command: "npx" },
      env: {},
      headers: {},
      ...overrides,
    },
  });
}

/** 展开弹窗内的高级折叠区，才能看到 env / headers / 其他字段。 */
async function openAdvanced() {
  fireEvent.click(await screen.findByText("高级配置"));
}

describe("McpPage", () => {
  beforeEach(() => {
    resetBrowserMock();
    resetMcpStore();
  });

  afterEach(() => {
    resetBrowserMock();
    resetMcpStore();
  });

  it("shows the empty state and explains where configs are written", async () => {
    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    expect(await screen.findByText("还没有 MCP 服务")).toBeInTheDocument();
    expect(screen.getByText(/环境变量与 Headers 在本应用内加密保存/)).toBeInTheDocument();
    // 路径须知列出各客户端目标（信息性，仍含 Prime 路径）。
    expect(screen.getByText("/Users/demo/.claude.json")).toBeInTheDocument();
    expect(screen.getByText("/Users/demo/.prime/agent/settings.json")).toBeInTheDocument();
  });

  it("lists a saved server under its agent tab with a single per-agent toggle", async () => {
    await seedServer({ targets: ["claude_code", "prime"] });

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    expect(await screen.findByText("demo")).toBeInTheDocument();
    const card = managedCard("demo");
    // per-agent 卡：只有「主启用」+「应用到本 agent」两个开关。
    expect(within(card).getAllByRole("switch")).toHaveLength(2);
    expect(within(card).getByRole("switch", { name: /应用到 Claude Code/ })).toBeChecked();
    expect(within(card).queryByRole("switch", { name: /应用到 Codex/ })).toBeNull();
  });

  it("hides a server under agents it is not applied to", async () => {
    await seedServer({ targets: ["claude_code"] });

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    await screen.findByText("demo");
    switchTab(/Codex/);
    await waitFor(() => {
      expect(screen.queryByText("demo")).toBeNull();
    });
  });

  it("rejects a server name with characters that would break config keys", async () => {
    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    await openManualAdd();
    fireEvent.change(screen.getByLabelText("服务名称"), { target: { value: "bad name" } });
    fireEvent.click(saveButton());

    await waitFor(() => {
      expect(screen.getByText("只能使用字母、数字、下划线和短横线")).toBeInTheDocument();
    });
  });

  it("reports invalid JSON instead of saving it", async () => {
    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    await openManualAdd();
    fireEvent.change(screen.getByLabelText("服务名称"), { target: { value: "filesystem" } });
    await openAdvanced();
    fireEvent.change(screen.getByLabelText("环境变量"), { target: { value: "{not json" } });
    fireEvent.click(saveButton());

    await waitFor(() => {
      expect(screen.getByText(/不是合法的 JSON/)).toBeInTheDocument();
    });
    expect(useMcpStore.getState().servers).toHaveLength(0);
  });

  it("round-trips an edited server back into the simple form", async () => {
    const saved = (await seedServer({
      targets: ["claude_code"],
      config: { command: "npx", args: ["-y", "pkg"], cwd: "/tmp/work" },
      env: { TOKEN: "placeholder-value" },
    })) as { server: { id: string } };

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    await screen.findByText("demo");
    fireEvent.click(within(managedCard("demo")).getByRole("button", { name: /编\s*辑/ }));

    const nameInput = (await screen.findByLabelText("服务名称")) as HTMLInputElement;
    await waitFor(() => {
      expect(nameInput.value).toBe("demo");
    });
    expect((screen.getByLabelText("启动命令") as HTMLInputElement).value).toBe("npx");
    expect((screen.getByLabelText("启动参数") as HTMLTextAreaElement).value).toBe("-y\npkg");

    await openAdvanced();
    expect((screen.getByLabelText("其他配置字段") as HTMLTextAreaElement).value).toContain("cwd");
    expect((screen.getByLabelText("环境变量") as HTMLTextAreaElement).value).toContain("TOKEN");
    expect(saved.server.id).toBeTruthy();
  });

  it("switches the launch field between command and url by kind", async () => {
    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    await openManualAdd();
    expect(screen.getByLabelText("启动命令")).toBeInTheDocument();
    expect(screen.queryByLabelText("服务地址")).toBeNull();

    fireEvent.click(screen.getByRole("radio", { name: "HTTP" }));
    await waitFor(() => {
      expect(screen.getByLabelText("服务地址")).toBeInTheDocument();
    });
    expect(screen.queryByLabelText("启动命令")).toBeNull();
  });

  describe("per-agent toggles", () => {
    it("reflects an applied server's toggle as on", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await screen.findByText("demo");
      const claude = within(managedCard("demo")).getByRole("switch", {
        name: /应用到 Claude Code/,
      });
      expect(claude).toBeChecked();
    });

    it("adds a target through the edit form's target checkboxes", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await screen.findByText("demo");
      fireEvent.click(within(managedCard("demo")).getByRole("button", { name: /编\s*辑/ }));
      await screen.findByLabelText("服务名称");
      fireEvent.click(screen.getByRole("checkbox", { name: "Codex" }));
      fireEvent.click(saveButton());

      await waitFor(() => {
        expect(useMcpStore.getState().servers[0].targets).toContain("codex");
      });
    });

    it("removes a target when the client toggle is switched off", async () => {
      await seedServer({ targets: ["claude_code", "codex"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await screen.findByText("demo");
      const claude = within(managedCard("demo")).getByRole("switch", {
        name: /应用到 Claude Code/,
      });
      fireEvent.click(claude);

      await waitFor(() => {
        expect(useMcpStore.getState().servers[0].targets).not.toContain("claude_code");
      });
      expect(useMcpStore.getState().servers[0].targets).toContain("codex");
    });

    it("toggles the master switch off and persists it", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await screen.findByText("demo");
      const master = within(managedCard("demo")).getByRole("switch", { name: /启用 demo/ });
      expect(master).toBeChecked();
      fireEvent.click(master);

      await waitFor(() => {
        expect(useMcpStore.getState().servers[0].enabled).toBe(false);
      });
    });

    it("filters servers by name within the agent", async () => {
      await seedServer({ name: "alpha", targets: ["claude_code"] });
      await seedServer({ name: "beta", targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      expect(await screen.findByText("alpha")).toBeInTheDocument();
      expect(screen.getByText("beta")).toBeInTheDocument();
      fireEvent.change(screen.getByPlaceholderText(/搜索 MCP 名称/), { target: { value: "alp" } });
      await waitFor(() => {
        expect(screen.queryByText("beta")).toBeNull();
      });
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });
  });

  describe("add from registry", () => {
    it("shows recommended entries on the registry source", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      expect(await screen.findByText("io.github.example/filesystem")).toBeInTheDocument();
    });

    it("searches the registry and lists local entries first", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "example" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));

      expect(await screen.findByText("io.github.example/filesystem")).toBeInTheDocument();
      // 「只看本地运行」默认开启：远程条目和不可安装条目都不该出现。
      expect(screen.queryByText("ai.example/hosted-memory")).toBeNull();
      expect(screen.queryByText("io.example/not-installable")).toBeNull();
    });

    it("falls back to an empty note when the query matches nothing", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-such-server-xyz" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));

      expect(await screen.findByText("没有找到匹配的 MCP 服务")).toBeInTheDocument();
    });

    it("installs in one click when the entry needs no user input", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-config-needed" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(2);
      });
      const saved = useMcpStore.getState().servers.find((s) => s.name === "no-config-needed");
      expect(saved?.targets).toEqual(["claude_code"]);
    });

    it("prefills the form when the entry requires input", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      // 切回表单：名称、命令预填好，必填密钥单列出来。
      const nameInput = (await screen.findByLabelText("服务名称")) as HTMLInputElement;
      expect(nameInput.value).toBe("filesystem");
      expect((screen.getByLabelText("启动命令") as HTMLInputElement).value).toBe("npx");
      expect(screen.getByText("API_KEY")).toBeInTheDocument();
      expect(screen.getByText("敏感信息")).toBeInTheDocument();
    });

    it("asks for targets once when a secret-free entry has nowhere to go", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-config-needed" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      expect(await screen.findByText("选择应用目标")).toBeInTheDocument();
      const picker = (await screen.findAllByRole("dialog")).find((d) =>
        within(d).queryByText("选择应用目标"),
      )!;
      fireEvent.click(within(picker).getByRole("button", { name: /^安\s*装$/ }));

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(1);
      });
      // per-agent 外壳排除 Prime：默认目标只含 Claude / Codex / Pi。
      expect(useMcpStore.getState().servers[0].targets).toEqual(["claude_code", "codex", "pi"]);
    });

    it("blocks saving until registry-declared required fields are filled", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      fireEvent.click(saveButton());
      await waitFor(() => {
        expect(screen.getByText("请先填写上面列出的必填项")).toBeInTheDocument();
      });
      expect(useMcpStore.getState().servers).toHaveLength(0);
    });

    it("saves the registry-declared required value into the server", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      await openRegistry();
      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      const requiredInput = (await screen.findByLabelText("API_KEY")) as HTMLInputElement;
      fireEvent.change(requiredInput, { target: { value: "the-token" } });
      fireEvent.click(saveButton());

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(1);
      });
      const saved = await useMcpStore.getState().getServer(useMcpStore.getState().servers[0].id);
      expect(saved.env.API_KEY).toBe("the-token");
      expect(saved.config.command).toBe("npx");
    });
  });

  describe("unmanaged (wild) entries", () => {
    it("lists an agent's hand-configured servers under its tab", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // Claude tab: existing-db 是 claude_code 上的野生条目。
      expect(await screen.findByText("existing-db")).toBeInTheDocument();
      // existing-fs 属于 Codex，切过去才出现。
      switchTab(/Codex/);
      expect(await screen.findByText("existing-fs")).toBeInTheDocument();
    });

    it("shows which keys an entry needs without exposing values", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // existing-db（claude_code）声明需要 DB_URL，只显示键名不显示值。
      expect(await screen.findByText(/需要 DB_URL/)).toBeInTheDocument();
      expect(screen.queryByText(/DB_URL=/)).toBeNull();
    });

    it("excludes app-managed entries from the wild list", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      // 托管条目（managed-one）不算野生：不出现纳管卡片。
      expect(screen.queryByText("managed-one")).toBeNull();
      // Codex 页只有 existing-fs 一条可纳管：一个纳管按钮。
      expect(screen.getAllByRole("button", { name: /^纳\s*管$/ })).toHaveLength(1);
    });

    it("adopts a single wild entry into the managed list", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      const card = unmanagedCard("existing-fs");
      fireEvent.click(within(card).getByRole("button", { name: /^纳\s*管$/ }));

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(1);
      });
      expect(useMcpStore.getState().servers[0].name).toBe("existing-fs");
    });

    it("removes the entry from the wild list after adoption (takeover round-trip)", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      const card = unmanagedCard("existing-fs");
      fireEvent.click(within(card).getByRole("button", { name: /^纳\s*管$/ }));

      // 纳管后重扫：existing-fs 不再作为「未纳管」野生条目出现。
      await waitFor(() => {
        const stillWild = screen
          .queryAllByText("existing-fs")
          .some((el) => el.closest('[data-testid="mcp-unmanaged-card"]'));
        expect(stillWild).toBe(false);
      });
    });

    it("warns when takeover fails for a client during adoption", async () => {
      // 该客户端里的同名条目在扫描后被改成与库记录不等价 → 接管跳过、原样保留，提示用户。
      setBrowserMcpTakeoverFailTargets(["codex"]);
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      const card = unmanagedCard("existing-fs");
      fireEvent.click(within(card).getByRole("button", { name: /^纳\s*管$/ }));

      expect(await screen.findByText(/接管失败/)).toBeInTheDocument();
    });

    it("adopts all wild entries of the current agent at once", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      fireEvent.click(screen.getByRole("button", { name: /全部纳管/ }));

      const dialog = await screen.findByRole("dialog");
      expect(within(dialog).getByText("existing-fs")).toBeInTheDocument();
      fireEvent.click(within(dialog).getByRole("button", { name: /确\s*认/ }));

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(1);
      });
    });
  });

  describe("same-name conflicts", () => {
    it("marks a client toggle as conflicting and opens the comparison", async () => {
      // 库里有同名(existing-fs)但内容不同(命令不同)的托管记录 → codex 上是同名冲突。
      await seedServer({
        name: "existing-fs",
        config: { command: "other-cmd" },
        targets: ["codex"],
      });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      const card = managedCard("existing-fs");
      // 冲突横幅出现，附「对比」按钮。
      const compare = await within(card).findByRole("button", { name: /对\s*比/ });
      fireEvent.click(compare);

      // 冲突弹窗：左右对比 + 两个出口，没有「用我的覆盖」。
      expect(await screen.findByText(/解决同名冲突/)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /用客户端的（反向入库）/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /该客户端先跳过/ })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /用我的覆盖/ })).toBeNull();
    });

    it("skips the client without touching the library on skip", async () => {
      await seedServer({
        name: "existing-fs",
        config: { command: "other-cmd" },
        targets: ["codex"],
      });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      switchTab(/Codex/);
      await screen.findByText("existing-fs");
      fireEvent.click(
        await within(managedCard("existing-fs")).findByRole("button", { name: /对\s*比/ }),
      );
      fireEvent.click(await screen.findByRole("button", { name: /该客户端先跳过/ }));

      // 跳过只关弹窗：库内那条记录原样保留。
      await waitFor(() => {
        expect(screen.queryByText(/解决同名冲突/)).toBeNull();
      });
      const record = useMcpStore.getState().servers.find((s) => s.name === "existing-fs");
      expect(record).toBeDefined();
      expect(record?.targets).toContain("codex");
    });
  });
});
