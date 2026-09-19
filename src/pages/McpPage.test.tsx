import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { handleBrowserCommand, resetBrowserMock } from "@/lib/browserMock";
import { useMcpStore } from "@/stores/mcpStore";
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
  useMcpStore.setState({ servers: [], loading: false });
}

/** antd 会在两个汉字之间插入空格，这里统一用容忍空白的匹配。 */
function saveButton(): HTMLElement {
  return screen.getByRole("button", { name: /保\s*存/ });
}

function manualAddButton(): HTMLElement {
  return screen.getByRole("button", { name: /手动添加/ });
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

/** 展开高级折叠区，才能看到 env / headers / 其他字段。 */
async function openAdvanced() {
  const toggle = await screen.findByText("高级配置");
  fireEvent.click(toggle);
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
    expect(screen.getByText("/Users/demo/.claude.json")).toBeInTheDocument();
    expect(screen.getByText("/Users/demo/.prime/agent/settings.json")).toBeInTheDocument();
  });

  it("lists saved servers with their targets", async () => {
    await seedServer({ targets: ["claude_code", "prime"] });

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    expect(await screen.findByText("demo")).toBeInTheDocument();
    // 扫描区也会出现同名目标标签，断言限定在已保存表格的那一行内。
    const row = screen.getByText("demo").closest("tr")!;
    expect(within(row as HTMLElement).getByText("Claude Code")).toBeInTheDocument();
    expect(within(row as HTMLElement).getByText("Prime")).toBeInTheDocument();
  });

  it("rejects a server name with characters that would break config keys", async () => {
    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("button", { name: /手动添加/ }));
    const nameInput = await screen.findByLabelText("服务名称");
    fireEvent.change(nameInput, { target: { value: "bad name" } });
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

    fireEvent.click(manualAddButton());
    fireEvent.change(await screen.findByLabelText("服务名称"), {
      target: { value: "filesystem" },
    });
    await openAdvanced();
    const envInput = screen.getByLabelText("环境变量");
    fireEvent.change(envInput, { target: { value: "{not json" } });
    fireEvent.click(saveButton());

    await waitFor(() => {
      expect(screen.getByText(/不是合法的 JSON/)).toBeInTheDocument();
    });
    expect(useMcpStore.getState().servers).toHaveLength(0);
  });

  it("applies only the targets that enabled servers point at", async () => {
    await seedServer({ targets: ["claude_code"] });
    await seedServer({ name: "remote", targets: ["codex"] });

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("button", { name: /应用到目标/ }));

    const dialogs = await screen.findAllByRole("dialog");
    const text = dialogs.map((dialog) => dialog.textContent ?? "").join("\n");
    expect(text).toMatch(/Claude Code 应用成功/);
    expect(text).toMatch(/Codex 应用成功/);
  });

  it("disables apply when no enabled server selects a target", async () => {
    await seedServer({ enabled: false, targets: ["claude_code"] });

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    const applyButton = await screen.findByRole("button", { name: /应用到目标/ });
    await waitFor(() => {
      expect(applyButton).toBeDisabled();
    });
  });

  it("round-trips an edited server back into the simple form", async () => {
    const saved = (await seedServer({
      targets: ["codex"],
      config: { command: "npx", args: ["-y", "pkg"], cwd: "/tmp/work" },
      env: { TOKEN: "placeholder-value" },
    })) as { server: { id: string } };

    render(
      <Wrapper>
        <McpPage />
      </Wrapper>,
    );

    const row = (await screen.findByText("demo")).closest("tr")!;
    fireEvent.click(within(row).getByRole("button", { name: /编\s*辑/ }));

    const nameInput = (await screen.findByLabelText("服务名称")) as HTMLInputElement;
    await waitFor(() => {
      expect(nameInput.value).toBe("demo");
    });
    // 简单层直接显示命令，不用去翻 JSON。
    expect((screen.getByLabelText("启动命令") as HTMLInputElement).value).toBe("npx");
    expect((screen.getByLabelText("启动参数") as HTMLTextAreaElement).value).toBe("-y\npkg");

    // 高级层保留 cwd 这类额外字段，且已保存的密钥回填供编辑。
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

    fireEvent.click(manualAddButton());
    expect(await screen.findByLabelText("启动命令")).toBeInTheDocument();
    expect(screen.queryByLabelText("服务地址")).toBeNull();

    // 切成 HTTP 后只问地址，不再问命令。
    fireEvent.click(screen.getByRole("radio", { name: "HTTP" }));
    await waitFor(() => {
      expect(screen.getByLabelText("服务地址")).toBeInTheDocument();
    });
    expect(screen.queryByLabelText("启动命令")).toBeNull();
  });

  describe("registry install", () => {
    it("shows entries on first open instead of an empty panel", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // 不点搜索也要有内容：默认拉「最近更新」，避免进来一片空白。
      expect(await screen.findByText("io.github.example/filesystem")).toBeInTheDocument();
    });

    it("searches the registry and lists local entries first", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "example" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));

      expect(await screen.findByText("io.github.example/filesystem")).toBeInTheDocument();
      // 「只看本地运行」默认开启：远程条目和不可安装条目都不该出现。
      expect(screen.queryByText("ai.example/hosted-memory")).toBeNull();
      expect(screen.queryByText("io.example/not-installable")).toBeNull();
    });

    it("falls back to recent entries when the query matches nothing", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-such-server-xyz" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));

      expect(await screen.findByText("没有找到匹配的 MCP 服务")).toBeInTheDocument();
    });

    it("shows remote entries with the host their requests go to", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // 关掉「只看本地运行」才会出现远程条目。
      fireEvent.click(await screen.findByRole("switch"));
      fireEvent.change(screen.getByPlaceholderText(/搜索，例如 github/), {
        target: { value: "hosted" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));

      expect(await screen.findByText("ai.example/hosted-memory")).toBeInTheDocument();
      expect(screen.getByText("请求将发送到 mcp.example.ai")).toBeInTheDocument();
      // 「远程服务」标签在多条远程条目上都会出现，按条目范围断言。
      const row = screen.getByText("ai.example/hosted-memory").closest(".ant-list-item")!;
      expect(within(row as HTMLElement).getByText("远程服务")).toBeInTheDocument();
    });

    it("installs in one click when the entry needs no user input", async () => {
      // 已有 MCP 配置过目标，一键安装才知道该装到哪儿。
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-config-needed" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      // 不需要用户填任何东西 → 不弹表单，直接装好并沿用已有目标。
      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(2);
      });
      expect(screen.queryByRole("dialog")).toBeNull();
      const saved = useMcpStore
        .getState()
        .servers.find((item) => item.name === "no-config-needed");
      expect(saved?.targets).toEqual(["claude_code"]);
    });

    it("opens the form when the entry requires input", async () => {
      await seedServer({ targets: ["claude_code"] });

      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      // 仓库声明了必填项 → 必须让用户填，表单要打开。
      expect(await screen.findByRole("dialog")).toBeInTheDocument();
      expect(screen.getByText("API_KEY")).toBeInTheDocument();
    });

    it("opens the form when no target is configured yet", async () => {
      // 一个 MCP 都没有时无法推断装到哪儿，必须问用户。
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "no-config-needed" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      expect(await screen.findByRole("dialog")).toBeInTheDocument();
      expect(screen.getByLabelText("服务名称")).toBeInTheDocument();
    });

    it("prefills the form from the registry entry", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      // 名称、命令、参数都自动填好了，用户不需要懂 command/args。
      const nameInput = (await screen.findByLabelText("服务名称")) as HTMLInputElement;
      expect(nameInput.value).toBe("filesystem");
      expect((screen.getByLabelText("启动命令") as HTMLInputElement).value).toBe("npx");
      expect((screen.getByLabelText("启动参数") as HTMLTextAreaElement).value).toContain(
        "@modelcontextprotocol/server-filesystem",
      );
      // 如实展示将要运行的命令。
      expect(screen.getByText(/将运行：/)).toBeInTheDocument();
      // 必填项单独列出来，而不是丢一个 JSON 让用户猜。
      expect(screen.getByText("API_KEY")).toBeInTheDocument();
      expect(screen.getByText("敏感信息")).toBeInTheDocument();
    });

    it("blocks saving until registry-declared required fields are filled", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.change(await screen.findByPlaceholderText(/搜索，例如 github/), {
        target: { value: "filesystem" },
      });
      fireEvent.click(screen.getByRole("button", { name: /搜\s*索/ }));
      fireEvent.click(await screen.findByRole("button", { name: /安\s*装/ }));

      // 名称等基础字段已由仓库填好，但必填密钥为空。
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
      const saved = await useMcpStore.getState().getServer(
        useMcpStore.getState().servers[0].id,
      );
      expect(saved.env.API_KEY).toBe("the-token");
      expect(saved.config.command).toBe("npx");
    });

    it("keeps manual add available as a fallback", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // 仓库不可用时手动路径必须还在。
      fireEvent.click(manualAddButton());
      const nameInput = (await screen.findByLabelText("服务名称")) as HTMLInputElement;
      expect(nameInput.value).toBe("");
      expect(screen.getByLabelText("启动命令")).toBeInTheDocument();
    });
  });

  describe("importing existing MCP servers", () => {
    it("lists hand-configured servers from other clients on open", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // 打开即扫描，用户能立刻看到别的客户端里配了什么。
      expect(await screen.findByText("existing-fs")).toBeInTheDocument();
      expect(screen.getByText("existing-db")).toBeInTheDocument();
    });

    it("shows which keys an entry needs without exposing values", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      // 只列键名，密钥值从不离开后端。
      expect(await screen.findByText(/需要 DB_URL/)).toBeInTheDocument();
      expect(screen.queryByText(/DB_URL=/)).toBeNull();
    });

    it("marks servers this app manages instead of offering import", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      expect(await screen.findByText("managed-one")).toBeInTheDocument();
      expect(screen.getByText("本工具管理")).toBeInTheDocument();
      // 两条可纳管，托管那条没有单条纳管按钮（批量按钮不计入）。
      expect(screen.getAllByRole("button", { name: /^纳\s*管$/ })).toHaveLength(2);
    });

    it("imports a single entry and marks it as imported", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      const row = (await screen.findByText("existing-fs")).closest(".ant-list-item")!;
      fireEvent.click(within(row as HTMLElement).getByRole("button", { name: /^纳\s*管$/ }));

      // 导入后进入已保存列表，并在扫描结果里标成「已纳管」。
      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(1);
      });
      await waitFor(() => {
        expect(screen.getAllByText("已纳管").length).toBeGreaterThan(0);
      });
    });

    it("imports all remaining entries at once", async () => {
      render(
        <Wrapper>
          <McpPage />
        </Wrapper>,
      );

      fireEvent.click(await screen.findByRole("button", { name: /全部纳管/ }));

      await waitFor(() => {
        expect(useMcpStore.getState().servers).toHaveLength(2);
      });
      // 全部纳管后不再提供批量入口。
      await waitFor(() => {
        expect(screen.queryByRole("button", { name: /全部纳管/ })).toBeNull();
      });
    });
  });
});
