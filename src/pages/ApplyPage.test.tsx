import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetBrowserMock } from "@/lib/browserMock";
import type { TargetLiveStatus } from "@/types/domain";
import { useApplyStore, useSiteStore, useUIStore } from "@/stores";
import { ApplyPage } from "./ApplyPage";
import "@/i18n";

function status(kind: TargetLiveStatus["kind"], applied: boolean): TargetLiveStatus {
  return {
    kind,
    installed: true,
    version: "1.0.0",
    configPath: kind === "claude_code" ? "~/.claude/settings.json" : "~/.codex/config.toml",
    status: applied ? "applied" : "not_applied",
    appliedSiteId: applied ? "site-1" : null,
    appliedSiteName: applied ? "Relay One" : null,
    appliedModelId: applied ? "gpt-4.1" : null,
    providerId: null,
    orphan: false,
    liveSummary: {},
    lastAppliedAt: applied ? 1 : null,
    staleReason: null,
  };
}

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

async function seedSite() {
  const site = await useSiteStore.getState().createSite({
    name: "Relay One",
    baseUrl: "https://api.example.com",
    apiKey: "sk-test",
  });
  useUIStore.getState().setSelectedSiteId(site.id);
  return site;
}

function resetStores() {
  resetBrowserMock();
  useSiteStore.setState({
    sites: [],
    modelsBySite: {},
    modelsLoadingBySite: {},
    loading: false,
    hydrated: true,
    fetchingModels: false,
    error: null,
  });
  useUIStore.setState({
    selectedSiteId: null,
    applyPrefillSiteId: null,
    activePage: "apply",
    applyTab: "claude_code",
  });
  useApplyStore.setState({
    statuses: [],
    tools: [],
    records: [],
    backups: [],
    applying: false,
    loading: false,
    statusHydrated: true,
    lastResult: null,
  });
}

describe("ApplyPage target switch", () => {
  beforeEach(() => {
    resetStores();
  });

  afterEach(() => {
    resetStores();
  });

  it("shows a skeleton immediately when switching to an unvisited target", async () => {
    await seedSite();
    render(
      <Wrapper>
        <ApplyPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("鉴权字段")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("menuitem", { name: "Codex" }));

    expect(document.querySelector("[aria-busy='true']")).toBeTruthy();
    expect(screen.queryByText("将站点全部模型写入 Codex")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("将站点全部模型写入 Codex")).toBeInTheDocument();
    });
    expect(screen.getByText("平台能力")).toBeInTheDocument();
    expect(screen.getByText("跟随站点预设能力")).toBeInTheDocument();
    expect(screen.queryByText("识图支持")).not.toBeInTheDocument();

    fireEvent.mouseDown(screen.getByText("跟随站点预设能力"));
    const custom = await screen.findByText("自定义");
    fireEvent.click(custom);
    expect(screen.getByText("远程压缩")).toBeInTheDocument();
    expect(screen.getByText("识图支持")).toBeInTheDocument();
    expect(screen.getByText("生图支持")).toBeInTheDocument();
    expect(screen.getByText("搜索")).toBeInTheDocument();
  });

  it("keeps a visited target mounted so switching back is instant", async () => {
    await seedSite();
    render(
      <Wrapper>
        <ApplyPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("鉴权字段")).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Codex" }));
    });
    await waitFor(() => {
      expect(screen.getByText("将站点全部模型写入 Codex")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("menuitem", { name: "Claude Code" }));
    expect(document.querySelector("[aria-busy='true']")).toBeNull();
    expect(screen.getByText("鉴权字段")).toBeInTheDocument();
  });

  it("shows a pulsing status dot for applied tools in the sidebar", async () => {
    await seedSite();
    useApplyStore.setState({
      statuses: [status("claude_code", true), status("codex", false)],
      statusHydrated: true,
      loadStatus: async () => {},
    });

    render(
      <Wrapper>
        <ApplyPage />
      </Wrapper>,
    );

    const claude = await screen.findByRole("menuitem", { name: "Claude Code" });
    expect(claude.querySelector("[data-status='available']")).toBeTruthy();
    expect(claude.querySelector(".ant-badge-status-processing")).toBeTruthy();

    const codex = screen.getByRole("menuitem", { name: "Codex" });
    expect(codex.querySelector("[data-status='disabled']")).toBeTruthy();
    expect(codex.querySelector(".ant-badge-status-default")).toBeTruthy();
  });
});
