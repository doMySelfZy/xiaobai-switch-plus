import { render, screen, waitFor } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetBrowserMock } from "@/lib/browserMock";
import { useProxyStore } from "@/stores/proxyStore";
import { ProxyPage } from "./ProxyPage";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

describe("ProxyPage", () => {
  beforeEach(() => {
    resetBrowserMock();
    useProxyStore.setState({ status: null, requests: [], loading: false });
  });

  afterEach(() => {
    resetBrowserMock();
    useProxyStore.setState({ status: null, requests: [], loading: false });
  });

  it("renders the proxy service card with listen address and port", async () => {
    render(<ProxyPage />, { wrapper: Wrapper });

    expect(await screen.findByText("本地代理")).toBeTruthy();
    await waitFor(() => {
      expect(screen.getByText(/127\.0\.0\.1:18087/)).toBeTruthy();
    });
    // 未运行时给出明确提示，避免用户以为已经在转发。
    expect(await screen.findByText(/代理未运行/)).toBeTruthy();
  });

  it("shows all four takeover targets with their bound state", async () => {
    render(<ProxyPage />, { wrapper: Wrapper });

    await waitFor(() => {
      const targets = useProxyStore.getState().status?.targets ?? [];
      expect(targets.map((item) => item.target)).toEqual([
        "claude_code",
        "codex",
        "pi",
        "prime",
      ]);
    });
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("Pi")).toBeTruthy();
    expect(screen.getByText("Prime")).toBeTruthy();
  });

  it("states that the log never records headers, bodies or keys", async () => {
    render(<ProxyPage />, { wrapper: Wrapper });
    expect(
      await screen.findByText(/不记录请求头、请求体或密钥/),
    ).toBeTruthy();
  });

  it("starts the proxy through the status switch", async () => {
    render(<ProxyPage />, { wrapper: Wrapper });
    // 页面里有 5 个开关（服务 + 四个目标），服务开关是标题行右侧那个，取第一个。
    const switches = await screen.findAllByRole("switch");
    expect(switches.length).toBe(5);
    expect(useProxyStore.getState().status?.running).toBe(false);

    switches[0]!.click();

    await waitFor(() => {
      expect(useProxyStore.getState().status?.running).toBe(true);
    });
  });
});
