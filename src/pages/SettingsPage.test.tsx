import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetBrowserMock, seedLocalBackups, seedWebDavMock } from "@/lib/browserMock";
import {
  GITHUB_ISSUES_URL,
  GITHUB_RELEASES_URL,
  GITHUB_REPO_URL,
} from "@/lib/constants";
import { useSettingsStore, useUIStore } from "@/stores";
import { resetUpdateCheckerForTests } from "@/hooks/useUpdateChecker";
import { SettingsPage } from "./SettingsPage";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

const pkg = JSON.parse(readFileSync(resolve(import.meta.dirname, "../../package.json"), "utf8")) as {
  version: string;
};

const realSaveSettings = useSettingsStore.getState().saveSettings;

/** 把 store 的 saveSettings 换成计数 spy —— 组件通过选择器读到的就是它，能数出写库次数。 */
function installSaveSettingsSpy() {
  const spy = vi.fn(realSaveSettings);
  useSettingsStore.setState({ saveSettings: spy });
  return spy;
}

describe("SettingsPage network", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "network" });
    useSettingsStore.setState({
      settings: { ...useSettingsStore.getState().settings, proxyMode: "system" },
      loaded: false,
      loading: false,
    });
  });

  afterEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  it("shows proxy modes and custom fields only when custom is selected", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("代理模式")).toBeInTheDocument();
    });
    expect(screen.getByText("系统代理")).toBeInTheDocument();
    expect(screen.queryByText("主机 / IP")).toBeNull();

    fireEvent.mouseDown(screen.getByRole("combobox"));
    fireEvent.click(await screen.findByTitle("自定义"));

    await waitFor(() => {
      expect(screen.getByText("主机 / IP")).toBeInTheDocument();
      expect(screen.getByText("端口")).toBeInTheDocument();
    });
    expect(useSettingsStore.getState().settings.proxyMode).toBe("custom");
    expect(screen.getByText("测速结果有效期")).toBeInTheDocument();
    expect(screen.queryByText("设置已保存")).toBeNull();
  });
});

describe("SettingsPage tray", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
    useSettingsStore.setState({
      settings: useSettingsStore.getState().settings,
      loaded: false,
      loading: false,
    });
  });

  afterEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  it("shows close-to-tray and disables start-in-tray when close-to-tray is off", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("关闭窗口时最小化到托盘")).toBeInTheDocument();
    });
    expect(useSettingsStore.getState().settings.closeToTray).toBe(true);

    const switches = screen.getAllByRole("switch");
    const closeToTray = switches[2];
    const startInTray = switches[3];
    expect(startInTray).not.toBeDisabled();

    fireEvent.click(closeToTray);
    await waitFor(() => {
      expect(useSettingsStore.getState().settings.closeToTray).toBe(false);
    });
    expect(useSettingsStore.getState().settings.startInTray).toBe(false);
    expect(startInTray).toBeDisabled();
  });

  it("persists launch-at-login when the auto-start switch is clicked", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("开机启动")).toBeInTheDocument();
    });
    expect(useSettingsStore.getState().settings.autoStart).toBe(false);

    fireEvent.click(screen.getAllByRole("switch")[1]);
    await waitFor(() => {
      expect(useSettingsStore.getState().settings.autoStart).toBe(true);
    });
  });
});

describe("SettingsPage backup center", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "backup" });
    useSettingsStore.setState({
      settings: useSettingsStore.getState().settings,
      loaded: false,
      loading: false,
    });
  });

  afterEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  it("switches backup targets and keeps local settings in a modal", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    expect(await screen.findByText("本地备份")).toBeInTheDocument();
    expect(screen.getByText("WebDAV")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "创建备份" }));
    expect(await screen.findByText(/xiaobai-switch-backup-.*browser.*\.zip/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "备份设置" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("最大保留数量")).toBeInTheDocument();
    expect(within(dialog).getByDisplayValue("~/.xiaobai-switch/backups/app")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: /取\s*消/ }));

    fireEvent.click(screen.getByText("WebDAV"));
    expect(await screen.findByText("请先配置 WebDAV 连接")).toBeInTheDocument();
  });

  it("restores and batch deletes local snapshots with confirmation", async () => {
    const fileName = "xiaobai-switch-backup-20260827_120000.browser.12345678.zip";
    seedLocalBackups([
      {
        fileName,
        size: 1024,
        createdAt: Date.now(),
        deviceName: "browser",
        reason: "manual",
        appVersion: "0.0.5",
        error: null,
      },
    ]);
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    const table = await screen.findByRole("table");
    expect(await within(table).findByText(fileName)).toBeInTheDocument();
    fireEvent.click(within(table).getByRole("button", { name: "恢复" }));
    expect((await screen.findAllByText("恢复这份应用快照？")).length).toBeGreaterThan(0);
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: /取\s*消/ }),
    );

    fireEvent.click(within(table).getAllByRole("checkbox")[1]);
    fireEvent.click(screen.getByRole("button", { name: "删除 (1)" }));
    expect((await screen.findAllByText("确定删除选中的 1 个备份？")).length).toBeGreaterThan(0);
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: /删\s*除/ }),
    );
    await waitFor(() => {
      expect(within(table).queryByText(fileName)).toBeNull();
    });
  }, 10_000);

  it("saves a connection from the config modal without echoing its password", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByText("WebDAV"));
    fireEvent.click(await screen.findByRole("button", { name: "配置 WebDAV" }));
    let dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/master\.key/)).toBeInTheDocument();

    fireEvent.change(within(dialog).getByLabelText("服务器地址"), {
      target: { value: "https://dav.example.com/" },
    });
    fireEvent.change(within(dialog).getByLabelText("用户名"), { target: { value: "alice" } });
    fireEvent.change(within(dialog).getByLabelText("密码"), { target: { value: "secret" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "测试连接" }));
    expect(await within(dialog).findByText("连接成功")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    fireEvent.click(screen.getByRole("button", { name: "配置 WebDAV" }));
    dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("密码已加密保存，留空将继续使用当前密码")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("密码")).toHaveValue("");
  }, 10_000);

  it("shows an explicit confirmation before restoring a remote snapshot", async () => {
    seedWebDavMock(
      {
        baseUrl: "https://dav.example.com/",
        username: "alice",
        hasPassword: true,
      },
      [
        {
          fileName: "xiaobai-switch-backup-20260827_120000.browser.12345678.zip",
          size: 1024,
          lastModified: new Date().toUTCString(),
          deviceName: "browser",
        },
      ],
    );
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );
    fireEvent.click(await screen.findByText("WebDAV"));
    const remoteTable = await screen.findByRole("table");
    expect(
      await within(remoteTable).findByText(/xiaobai-switch-backup-.*browser.*\.zip/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "恢复" }));
    expect((await screen.findAllByText("恢复这份应用快照？")).length).toBeGreaterThan(0);
    expect(screen.getByText(/Claude\/Codex 配置不会被自动覆盖/)).toBeInTheDocument();
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: /取\s*消/ }),
    );
  }, 10_000);

  it("deletes a remote snapshot after confirmation", async () => {
    seedWebDavMock(
      {
        baseUrl: "https://dav.example.com/",
        username: "alice",
        hasPassword: true,
      },
      [
        {
          fileName: "xiaobai-switch-backup-20260827_120000.browser.12345678.zip",
          size: 1024,
          lastModified: new Date().toUTCString(),
          deviceName: "browser",
        },
      ],
    );
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );
    fireEvent.click(await screen.findByText("WebDAV"));
    const remoteTable = await screen.findByRole("table");
    expect(
      await within(remoteTable).findByText(/xiaobai-switch-backup-.*browser.*\.zip/),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "删除" }));
    expect((await screen.findAllByText("确定删除此备份？")).length).toBeGreaterThan(0);
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: /删\s*除/ }),
    );
    await waitFor(() => {
      expect(within(remoteTable).queryByText(/xiaobai-switch-backup-.*browser.*\.zip/)).toBeNull();
    });
  }, 10_000);
});

describe("SettingsPage about", () => {
  beforeEach(() => {
    resetBrowserMock();
    resetUpdateCheckerForTests();
    useUIStore.setState({ settingsTab: "about" });
    useSettingsStore.setState({
      settings: useSettingsStore.getState().settings,
      loaded: false,
      loading: false,
    });
  });

  afterEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  it("shows the actual package version instead of a hardcoded placeholder", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(useSettingsStore.getState().loaded).toBe(true);
      expect(screen.getByText(pkg.version)).toBeInTheDocument();
      expect(screen.getByText("~/.xiaobai-switch")).toBeInTheDocument();
    });
  });

  it("shows the app logo and GitHub link entries", async () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);

    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByRole("img", { name: "XiaoBaiSwitch Plus" })).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: /GitHub 仓库/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /问题反馈/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /版本发布/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /GitHub 仓库/ }));
    expect(open).toHaveBeenCalledWith(GITHUB_REPO_URL, "_blank", "noopener,noreferrer");

    fireEvent.click(screen.getByRole("button", { name: /问题反馈/ }));
    expect(open).toHaveBeenCalledWith(GITHUB_ISSUES_URL, "_blank", "noopener,noreferrer");

    fireEvent.click(screen.getByRole("button", { name: /版本发布/ }));
    expect(open).toHaveBeenCalledWith(GITHUB_RELEASES_URL, "_blank", "noopener,noreferrer");

    open.mockRestore();
  });

  it("shows update check controls and persists auto-check", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("自动检查更新")).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "检查更新" })).toBeInTheDocument();
    expect(useSettingsStore.getState().settings.autoCheckUpdate).toBe(true);

    fireEvent.click(screen.getByRole("switch"));
    await waitFor(() => {
      expect(useSettingsStore.getState().settings.autoCheckUpdate).toBe(false);
    });
  });

  it("tells the user a manual check needs the desktop app", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "检查更新" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await waitFor(() => {
      expect(screen.getByText("请在桌面客户端中检查更新")).toBeInTheDocument();
    });
  });
});

describe("SettingsPage paths", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "paths" });
    useSettingsStore.setState({ loaded: false, loading: false });
  });

  afterEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  it("shows and persists the Pi Agent directory override", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    const input = await screen.findByPlaceholderText(
      "留空按 PI_CODING_AGENT_DIR → ~/.pi/agent 解析",
    );
    fireEvent.change(input, { target: { value: "/tmp/custom-pi" } });
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => {
      expect(useSettingsStore.getState().settings.piAgentDirOverride).toBe("/tmp/custom-pi");
    });
  });

  it("shows and persists the Prime Agent directory override", async () => {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    const input = await screen.findByPlaceholderText(
      "留空按 PRIME_AGENT_CODING_AGENT_DIR → ~/.prime/agent 解析",
    );
    fireEvent.change(input, { target: { value: "/tmp/custom-prime" } });
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => {
      expect(useSettingsStore.getState().settings.primeAgentDirOverride).toBe("/tmp/custom-prime");
    });
  });
});

describe("SettingsPage number inputs", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "network" });
    useSettingsStore.setState({ loaded: false, loading: false });
  });

  afterEach(() => {
    useSettingsStore.setState({ saveSettings: realSaveSettings });
    resetBrowserMock();
    useUIStore.setState({ settingsTab: "general" });
  });

  /** network 分区在「系统代理」下只有「测速结果有效期」一个数字输入。 */
  async function renderProbeTtlInput() {
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );
    const input = await screen.findByRole("spinbutton");
    await waitFor(() => {
      expect((input as HTMLInputElement).value).toBe("10");
    });
    return input as HTMLInputElement;
  }

  it("writes the database once on blur instead of on every keystroke", async () => {
    const save = installSaveSettingsSpy();
    const input = await renderProbeTtlInput();

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "1" } });
    fireEvent.change(input, { target: { value: "12" } });
    fireEvent.change(input, { target: { value: "120" } });

    expect(save).not.toHaveBeenCalled();
    expect(input.value).toBe("120");

    fireEvent.blur(input);
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({ routeProbeTtlMinutes: 120 });
    expect(useSettingsStore.getState().settings.routeProbeTtlMinutes).toBe(120);
  });

  it("clamps the value on commit, not while typing", async () => {
    const save = installSaveSettingsSpy();
    const input = await renderProbeTtlInput();

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "5000" } });
    expect(input.value).toBe("5000");
    expect(save).not.toHaveBeenCalled();

    fireEvent.blur(input);
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({ routeProbeTtlMinutes: 1440 });
    await waitFor(() => {
      expect(input.value).toBe("1440");
    });
  });

  it("commits on Enter without waiting for blur", async () => {
    const save = installSaveSettingsSpy();
    const input = await renderProbeTtlInput();

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "30" } });
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });

    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({ routeProbeTtlMinutes: 30 });
  });

  it("keeps the draft while editing even when settings echo back", async () => {
    const save = installSaveSettingsSpy();
    const input = await renderProbeTtlInput();

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "45" } });
    // 模拟别处的保存 / 重新拉取把设置回写到 store：编辑中的草稿不能被回显覆盖。
    act(() => {
      useSettingsStore.setState({
        settings: { ...useSettingsStore.getState().settings, routeProbeTtlMinutes: 999 },
      });
    });
    expect(input.value).toBe("45");

    fireEvent.blur(input);
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({ routeProbeTtlMinutes: 45 });
  });

  it("flushes an uncommitted draft when the section unmounts", async () => {
    const save = installSaveSettingsSpy();
    const input = await renderProbeTtlInput();

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "77" } });
    expect(save).not.toHaveBeenCalled();

    act(() => {
      useUIStore.setState({ settingsTab: "about" });
    });
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({ routeProbeTtlMinutes: 77 });
  });

  it("saves the floating window interval on blur in the general section", async () => {
    const save = installSaveSettingsSpy();
    act(() => {
      useUIStore.setState({ settingsTab: "general" });
    });
    render(
      <Wrapper>
        <SettingsPage />
      </Wrapper>,
    );

    const input = (await screen.findByRole("spinbutton")) as HTMLInputElement;
    await waitFor(() => {
      expect(input.value).toBe("5");
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "20" } });
    expect(save).not.toHaveBeenCalled();

    fireEvent.blur(input);
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(1);
    });
    expect(save).toHaveBeenCalledWith({
      floatingWindow: {
        enabled: true,
        autoRefreshMinutes: 20,
        positionX: 100,
        positionY: 100,
        collapsed: false,
      },
    });
  });
});
