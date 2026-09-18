import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  getBrowserQuotaProbeCallCount,
  resetBrowserMock,
  seedTargetStatuses,
} from "@/lib/browserMock";
import { QUOTA_TTL_MS } from "@/lib/quotaProbe";
import { useApplyStore, useSiteStore, useUIStore } from "@/stores";
import { resetQuotaInflight } from "@/stores/siteStore";
import type { TargetLiveStatus } from "@/types/domain";
import { SitesPage } from "./SitesPage";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

function modelTag(modelId: string) {
  return document.querySelector(`[data-model-tag][title="${modelId}"]`);
}

async function seedSite() {
  const site = await useSiteStore.getState().createSite({
    name: "Relay One",
    baseUrl: "https://api.example.com",
    apiKey: "sk-test",
  });
  await useSiteStore.getState().fetchModels(site.id);
  useUIStore.getState().setSelectedSiteId(site.id);
  return site;
}

describe("SitesPage", () => {
  beforeEach(() => {
    resetBrowserMock();
    resetQuotaInflight();
    useSiteStore.setState({
      sites: [],
      modelsBySite: {},
      modelsLoadingBySite: {},
      quotaBySite: {},
      quotaAttemptBySite: {},
      quotaCacheKeyBySite: {},
      quotaAttemptCacheKeyBySite: {},
      quotaLoadingBySite: {},
      loading: false,
      hydrated: false,
      fetchingModels: false,
      error: null,
    });
    useUIStore.setState({ selectedSiteId: null, activePage: "sites", pendingSiteForm: null });
    useApplyStore.setState({
      statuses: [],
      tools: [],
      records: [],
      backups: [],
      applying: false,
      loading: false,
      statusHydrated: false,
      lastResult: null,
    });
  });

  afterEach(() => {
    resetBrowserMock();
  });

  it("renders detail without a card, current-model copy, small add, and apply beside the site name", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const tag = await waitFor(() => {
      const el = modelTag("gpt-4.1");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });

    expect(document.querySelector(".ant-card")).toBeNull();
    expect(screen.queryByText("列表没有？手动输入")).toBeNull();
    expect(screen.getByText("（当前模型：gpt-4.1）")).toBeInTheDocument();

    const search = screen.getByPlaceholderText("搜索模型…");
    expect(search.closest(".ant-input-search")).not.toHaveClass("ant-input-search-small");
    expect(search.closest(".ant-input-affix-wrapper")).not.toHaveClass(
      "ant-input-affix-wrapper-sm",
    );

    expect(tag.querySelector("[data-model-tag-close]")).toBeNull();
    expect(tag.querySelector(".ant-checkbox")).toBeNull();
    expect(tag).toHaveClass("model-tag");
    expect(tag).toHaveAttribute("data-selected", "true");
    expect(tag.style.fontSize).toBe("");
    expect(tag.style.paddingBlock).toBe("");

    const addBtn = screen.getByRole("button", { name: "手动添加" });
    const multiBtn = screen.getByRole("button", { name: "多选" });
    const testBtn = screen.getByRole("button", { name: "测试" });
    const clearBtn = screen.getByRole("button", { name: /清\s*空/ });
    expect(addBtn.className).toMatch(/ant-btn-sm/);
    expect(multiBtn.className).toMatch(/ant-btn-sm/);
    expect(testBtn.className).toMatch(/ant-btn-sm/);
    expect(clearBtn.querySelector("svg.lucide")).toBeTruthy();
    expect(addBtn.compareDocumentPosition(screen.getByText("主模型")) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
    expect(addBtn.compareDocumentPosition(multiBtn) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(multiBtn.compareDocumentPosition(testBtn) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(testBtn.compareDocumentPosition(clearBtn) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const applyBtn = screen.getByRole("button", { name: /去 Agent 应用/ });
    expect(applyBtn.className).toMatch(/ant-btn-sm/);
    const detailName = document.querySelector(".text-base.font-medium");
    expect(detailName?.textContent).toBe("Relay One");
    expect(applyBtn.compareDocumentPosition(detailName as Node) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
  });

  it("opens the test-models modal from the toolbar", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "测试" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "测试" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("测试模型")).toBeInTheDocument();
    expect(within(dialog).getByRole("checkbox", { name: "全选" })).toBeChecked();
    expect(within(dialog).getByRole("button", { name: /立\s*即\s*测\s*试/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /取\s*消/ })).toBeInTheDocument();
  });

  it("opens a modal to add a model manually", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "手动添加" })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "手动添加" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("手动添加模型")).toBeInTheDocument();

    fireEvent.change(within(dialog).getByPlaceholderText("输入 model id"), {
      target: { value: "gpt-5.6-terra" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /保\s*存/ }));

    await waitFor(() => {
      expect(modelTag("gpt-5.6-terra")).toBeTruthy();
    });
    expect(useSiteStore.getState().sites[0]?.selectedModelId).toBe("gpt-5.6-terra");
  });

  it("offers edit and delete on site list context menu", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const item = await screen.findByRole("button", { name: /Relay One/ });
    fireEvent.contextMenu(item);

    expect(await screen.findByRole("menuitem", { name: "编辑站点" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "删除站点" })).toBeInTheDocument();
  });

  it("centers an empty-model hint with a fetch button", async () => {
    await act(async () => {
      const site = await useSiteStore.getState().createSite({
        name: "Empty One",
        baseUrl: "https://api.example.com",
        apiKey: "sk-test",
      });
      useUIStore.getState().setSelectedSiteId(site.id);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const hint = await screen.findByText("暂无模型，请先拉取或手动添加");
    expect(hint.parentElement).toHaveClass("justify-center");
    expect(screen.getByRole("button", { name: "测试" })).toBeDisabled();

    const fetchInEmpty = hint.parentElement?.querySelector("button");
    expect(fetchInEmpty).toBeTruthy();
    expect(fetchInEmpty).toHaveTextContent("拉取模型");
    expect(fetchInEmpty?.className).toMatch(/ant-btn-sm/);
    expect(document.querySelector("[data-model-list]")).toBeTruthy();

    fireEvent.click(fetchInEmpty as HTMLButtonElement);
    await waitFor(() => {
      expect(document.querySelector("[data-model-tag]")).toBeTruthy();
    });
    const toasts = await screen.findAllByText("同步模型成功，本次同步 2 个模型");
    expect(toasts).toHaveLength(1);
    expect(screen.queryByText("成功")).toBeNull();
  });

  it("shows edit and delete from the site row more menu", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const more = await screen.findByRole("button", { name: "更多操作" });
    fireEvent.click(more);

    expect(await screen.findByRole("menuitem", { name: "编辑站点" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "删除站点" })).toBeInTheDocument();
  });

  it("clears all models from the header button", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(modelTag("gpt-4.1")).toBeTruthy();
    });

    const addBtn = screen.getByRole("button", { name: "手动添加" });
    const clearBtn = screen.getByRole("button", { name: /清\s*空/ });
    expect(clearBtn.className).toMatch(/ant-btn-sm/);
    expect(addBtn.compareDocumentPosition(clearBtn) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    fireEvent.click(clearBtn);
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /确\s*认/ }));

    await waitFor(() => {
      expect(document.querySelector("[data-model-tag]")).toBeNull();
    });
    expect(await screen.findByText("清空模型成功")).toBeInTheDocument();
    expect(screen.queryByText("成功")).toBeNull();
    expect(useSiteStore.getState().modelsBySite[useUIStore.getState().selectedSiteId ?? ""]).toEqual(
      [],
    );
  });

  it("opens the route dropdown and can switch the active base url", async () => {
    await act(async () => {
      const created = await useSiteStore.getState().createSite({
        name: "Relay One",
        baseUrl: "https://api.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().updateSite(created.id, {
        baseUrls: ["https://api.example.com", "https://api2.example.com"],
      });
      useUIStore.getState().setSelectedSiteId(created.id);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const trigger = await screen.findByRole("button", { name: "切换线路" });
    fireEvent.click(trigger);

    expect(await screen.findByText("https://api2.example.com")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /测\s*速/ })).toBeInTheDocument();

    fireEvent.click(screen.getByText("https://api2.example.com"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getAllByText("切换线路？").length).toBeGreaterThan(0);
    expect(within(dialog).getByRole("button", { name: /取\s*消/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "跳过应用" })).toBeInTheDocument();
    const option = screen.getByText("https://api2.example.com").closest("button");
    expect(option).toHaveClass("route-option");
    expect(option).toHaveClass("cursor-pointer");
    expect(useSiteStore.getState().sites[0]?.baseUrl).toBe("https://api.example.com");

    fireEvent.click(within(dialog).getByRole("button", { name: /确\s*认/ }));
    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.baseUrl).toBe("https://api2.example.com");
    });
  });

  it("can switch a route without applying to target CLIs", async () => {
    await act(async () => {
      const created = await useSiteStore.getState().createSite({
        name: "Relay One",
        baseUrl: "https://api.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().updateSite(created.id, {
        baseUrls: ["https://api.example.com", "https://api2.example.com"],
      });
      useUIStore.getState().setSelectedSiteId(created.id);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "切换线路" }));
    fireEvent.click(await screen.findByText("https://api2.example.com"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "跳过应用" }));
    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.baseUrl).toBe("https://api2.example.com");
    });
  });

  it("does not switch the route when the confirm dialog is cancelled", async () => {
    await act(async () => {
      const created = await useSiteStore.getState().createSite({
        name: "Relay One",
        baseUrl: "https://api.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().updateSite(created.id, {
        baseUrls: ["https://api.example.com", "https://api2.example.com"],
      });
      useUIStore.getState().setSelectedSiteId(created.id);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "切换线路" }));
    fireEvent.click(await screen.findByText("https://api2.example.com"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /取\s*消/ }));
    expect(useSiteStore.getState().sites[0]?.baseUrl).toBe("https://api.example.com");
  });

  it("removes a model when the tag close icon is clicked", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(modelTag("gpt-4.1")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: "多选" }));

    const tag = modelTag("gpt-4.1") as HTMLElement;
    const close = tag.querySelector("[data-model-tag-close]");
    expect(close).toBeTruthy();
    fireEvent.click(close as Element);

    await waitFor(() => {
      expect(modelTag("gpt-4.1")).toBeNull();
    });
    expect(
      useSiteStore.getState().modelsBySite[useUIStore.getState().selectedSiteId ?? ""]?.some(
        (m) => m.modelId === "gpt-4.1",
      ),
    ).toBe(false);
  });

  it("groups model tags by family prefix", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const gptGroup = await waitFor(() => {
      const el = document.querySelector('[data-model-group="gpt"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const claudeGroup = document.querySelector('[data-model-group="claude"]') as HTMLElement;
    expect(claudeGroup).toBeTruthy();

    const gptModelTag = gptGroup.querySelector('[data-model-tag][title="gpt-4.1"]') as HTMLElement;
    const gptCountTag = gptGroup.querySelector("[data-model-count]") as HTMLElement;
    expect(gptModelTag).toBeTruthy();
    expect(claudeGroup.querySelector('[data-model-tag][title="claude-sonnet-4"]')).toBeTruthy();
    expect(gptCountTag).toBeTruthy();
    expect(gptGroup).toHaveTextContent("1 个模型");
    expect(claudeGroup).toHaveTextContent("1 个模型");
    expect(gptModelTag.style.fontSize).toBe("");
    expect(Number.parseFloat(gptCountTag.style.fontSize)).toBeLessThan(12);
    expect(gptGroup.compareDocumentPosition(claudeGroup) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("enables checkboxes and a floating delete bar in multi-select", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const gptTag = await waitFor(() => {
      const el = modelTag("gpt-4.1");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const primaryBefore = useSiteStore.getState().sites[0]?.selectedModelId;

    fireEvent.click(screen.getByRole("button", { name: "多选" }));
    expect(screen.getByRole("button", { name: "完成" })).toBeInTheDocument();
    expect(gptTag.querySelector(".ant-checkbox")).toBeTruthy();
    const close = gptTag.querySelector("[data-model-tag-close]");
    expect(close).toBeTruthy();
    expect(close).toHaveClass("model-tag-close");
    expect(document.querySelector("[data-model-multi-actions]")).toBeNull();

    fireEvent.click(gptTag);
    const bar = await waitFor(() => {
      const el = document.querySelector("[data-model-multi-actions]");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    expect(bar).toHaveTextContent("已选 1 项");
    expect(bar).toHaveTextContent("删除 1 项");
    expect(useSiteStore.getState().sites[0]?.selectedModelId).toBe(primaryBefore);

    const claudeTag = modelTag("claude-sonnet-4") as HTMLElement;
    const claudeBox = claudeTag.querySelector("input[type='checkbox']") as HTMLInputElement;
    expect(claudeBox).toBeTruthy();
    fireEvent.click(claudeBox);
    expect(bar).toHaveTextContent("已选 2 项");
    expect(bar).toHaveTextContent("删除 2 项");

    fireEvent.click(within(bar).getByRole("button", { name: /删除/ }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /确\s*认/ }));

    await waitFor(() => {
      expect(modelTag("gpt-4.1")).toBeNull();
      expect(modelTag("claude-sonnet-4")).toBeNull();
    });
    expect(await screen.findByText("已删除 2 个模型")).toBeInTheDocument();
    expect(useSiteStore.getState().modelsBySite[useUIStore.getState().selectedSiteId ?? ""]).toEqual(
      [],
    );
  });

  it("exits multi-select and hides tag controls", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const tag = await waitFor(() => {
      const el = modelTag("gpt-4.1");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });

    fireEvent.click(screen.getByRole("button", { name: "多选" }));
    fireEvent.click(tag);
    expect(document.querySelector("[data-model-multi-actions]")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(screen.getByRole("button", { name: "多选" })).toBeInTheDocument();
    expect(tag.querySelector(".ant-checkbox")).toBeNull();
    expect(tag.querySelector("[data-model-tag-close]")).toBeNull();
    expect(document.querySelector("[data-model-multi-actions]")).toBeNull();
  });

  it("shows 停用 on the enable switch when the site is off", async () => {
    let siteId = "";
    await act(async () => {
      const site = await seedSite();
      siteId = site.id;
      await useSiteStore.getState().updateSite(site.id, { enabled: false });
      // 详情面板在模型缓存就绪前显示骨架屏（没有开关），所以先把缓存备好。
      useSiteStore.setState({ modelsBySite: { [site.id]: [] } });
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    // 页面上有两个开关：列表行里的快捷开关（只有 aria-label）和详情面板里带
    // 文字标签的开关。这里断言带「停用」字样的那个。
    const labeled = await waitFor(() => {
      const found = screen
        .getAllByRole("switch")
        .find((el) => el.textContent?.includes("停用"));
      expect(found).toBeTruthy();
      return found!;
    });
    expect(labeled).not.toBeChecked();
    // 列表行里那个快捷开关同样应当反映停用状态。
    expect(screen.getAllByRole("switch").find((el) => el.getAttribute("aria-label") === "启用"))
      .not.toBeChecked();
    expect(siteId).toBeTruthy();
  });

  it("shows a pulsing status dot for enabled sites and a gray one when disabled", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const item = await screen.findByRole("button", { name: /Relay One/ });
    expect(item.querySelector("[data-status='available']")).toBeTruthy();
    expect(item.querySelector(".ant-badge-status-processing")).toBeTruthy();

    await act(async () => {
      const site = useSiteStore.getState().sites[0];
      if (site) await useSiteStore.getState().updateSite(site.id, { enabled: false });
    });

    expect(item.querySelector("[data-status='disabled']")).toBeTruthy();
    expect(item.querySelector(".ant-badge-status-default")).toBeTruthy();
  });

  it("disables a site immediately when no target is using it", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const sw = await screen.findByRole("switch");
    expect(sw).toBeChecked();
    fireEvent.click(sw);

    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.enabled).toBe(false);
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("asks before disabling a site that is applied to a tool", async () => {
    await act(async () => {
      const site = await seedSite();
      seedTargetStatuses([
        appliedStatus("claude_code", site.id, site.name),
        appliedStatus("codex", null, null),
        appliedStatus("pi", site.id, site.name),
      ]);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("switch"));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("停用站点");
    expect(dialog).toHaveTextContent("Claude Code");
    expect(dialog).toHaveTextContent("Pi");
    expect(within(dialog).getByRole("button", { name: /取\s*消/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /跳\s*过/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /清\s*除/ })).toBeInTheDocument();
    // 乐观更新：开关先落下、弹窗用已缓存的状态判断，不再等强制 CLI 探测。
    expect(useSiteStore.getState().sites[0]?.enabled).toBe(false);

    // 「取消」= 不停用：回滚乐观更新。
    fireEvent.click(within(dialog).getByRole("button", { name: /取\s*消/ }));
    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.enabled).toBe(true);
    });
  });

  it("can skip clearing and only disable the site", async () => {
    await act(async () => {
      const site = await seedSite();
      seedTargetStatuses([
        appliedStatus("claude_code", site.id, site.name),
        appliedStatus("codex", null, null),
      ]);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("switch"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /跳\s*过/ }));

    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.enabled).toBe(false);
    });
    expect(useApplyStore.getState().statuses.find((s) => s.kind === "claude_code")?.status).toBe(
      "applied",
    );
  });

  it("clears applied tool config when confirming disable", async () => {
    await act(async () => {
      const site = await seedSite();
      seedTargetStatuses([
        appliedStatus("claude_code", site.id, site.name),
        appliedStatus("codex", site.id, site.name),
        appliedStatus("pi", site.id, site.name),
      ]);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("switch"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /清\s*除/ }));

    await waitFor(() => {
      expect(useSiteStore.getState().sites[0]?.enabled).toBe(false);
    });
    expect(useApplyStore.getState().statuses.every((s) => s.status === "not_applied")).toBe(true);
  });

  it("shows quota in site details when the probe returns a balance", async () => {
    await act(async () => {
      await seedSite();
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("site-quota-row")).toBeInTheDocument();
    });
    // The balance now appears both in the detail row and in the list summary;
    // scope the detail assertion to the quota row.
    expect(within(screen.getByTestId("site-quota-row")).getByText("剩余 $87.50")).toBeInTheDocument();
    expect(screen.getAllByText("剩余 $87.50").length).toBeGreaterThanOrEqual(2);
    expect(document.querySelector(".ant-card")).toBeNull();
  });

  it("shows an actionable quota status when the upstream does not implement billing", async () => {
    await act(async () => {
      const site = await useSiteStore.getState().createSite({
        name: "No Quota",
        baseUrl: "https://no-quota.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().fetchModels(site.id);
      useUIStore.getState().setSelectedSiteId(site.id);
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(modelTag("gpt-4.1")).toBeTruthy();
    });
    await waitFor(() => {
      expect(
        useSiteStore.getState().quotaAttemptBySite[
          useUIStore.getState().selectedSiteId ?? ""
        ]?.status,
      ).toBe("unsupported");
    });
    expect(screen.getByTestId("site-quota-status")).toBeInTheDocument();
    // 完整句子只有详情面板一份；列表行第二行是摘要标签，不再同屏重复。
    expect(screen.getByTestId("site-quota-status-placeholder")).toHaveTextContent("无额度接口");
    expect(screen.getAllByText("此站点不支持自动获取额度")).toHaveLength(1);
  });

  it("uses the latest-attempt TTL when the window regains focus", async () => {
    const site = await act(async () => seedSite());
    const probe = vi.spyOn(useSiteStore.getState(), "probeQuota");

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(probe).toHaveBeenCalledWith(site.id);
    });
    await waitFor(() => {
      expect(useSiteStore.getState().quotaAttemptBySite[site.id]).toBeDefined();
    });
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
    probe.mockClear();

    fireEvent.focus(window);

    await waitFor(() => {
      expect(probe).toHaveBeenCalledWith(site.id);
    });
    expect(getBrowserQuotaProbeCallCount()).toBe(1);

    const attempt = useSiteStore.getState().quotaAttemptBySite[site.id];
    useSiteStore.setState({
      quotaAttemptBySite: {
        ...useSiteStore.getState().quotaAttemptBySite,
        [site.id]: { ...attempt, fetchedAt: Date.now() - QUOTA_TTL_MS },
      },
    });
    fireEvent.focus(window);
    await waitFor(() => {
      expect(getBrowserQuotaProbeCallCount()).toBe(2);
    });
    probe.mockRestore();
  });

  it("prefetches quota for selected site when the page mounts", async () => {
    await act(async () => {
      await useSiteStore.getState().createSite({
        name: "Alpha",
        baseUrl: "https://alpha.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().createSite({
        name: "Beta",
        baseUrl: "https://beta.example.com",
        apiKey: "sk-test",
      });
    });
    const probe = vi.spyOn(useSiteStore.getState(), "probeQuota");

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    // 优化后只探测选中的站点（自动选中第一个），而不是全部站点
    await waitFor(() => {
      expect(getBrowserQuotaProbeCallCount()).toBe(1);
    });
    // Flush one more microtask turn so any late duplicate request would land
    // before the count is asserted again.
    await act(async () => {});
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
    // 应该只探测选中的站点（第一个）
    const selectedId = useUIStore.getState().selectedSiteId;
    expect(selectedId).toBeTruthy();
    expect(probe).toHaveBeenCalledWith(selectedId);
    probe.mockRestore();
  });

  it("does not create a page-local refresh interval", async () => {
    await act(async () => {
      await useSiteStore.getState().createSite({
        name: "Alpha",
        baseUrl: "https://alpha.example.com",
        apiKey: "sk-test",
      });
    });
    const setIntervalSpy = vi.spyOn(window, "setInterval");

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );
    await act(async () => {});
    await waitFor(() => expect(getBrowserQuotaProbeCallCount()).toBe(1));
    expect(setIntervalSpy.mock.calls.some(([, ms]) => ms === 30_000)).toBe(false);
    setIntervalSpy.mockRestore();
  });

  it("persists sidebar order through reorderSites after creating two sites", async () => {
    await act(async () => {
      await useSiteStore.getState().createSite({
        name: "Alpha",
        baseUrl: "https://alpha.example.com",
        apiKey: "sk-test",
      });
      await useSiteStore.getState().createSite({
        name: "Beta",
        baseUrl: "https://beta.example.com",
        apiKey: "sk-test",
      });
    });

    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    const handles = await screen.findAllByRole("button", { name: "拖拽排序" });
    expect(handles).toHaveLength(2);
    expect(handles[0]).toHaveAttribute("aria-roledescription", "sortable");
    expect(handles[0]).toHaveAttribute("title", "拖拽排序");
    expect(document.querySelectorAll("[data-testid='site-drag-handle']")).toHaveLength(2);

    const listNames = () =>
      [...document.querySelectorAll("[data-testid='site-list-item']")].map(
        (el) => el.querySelector(".truncate.text-sm.font-medium")?.textContent,
      );
    expect(listNames()).toEqual(["Alpha", "Beta"]);
    expect(useSiteStore.getState().sites.map((s) => s.name)).toEqual(["Alpha", "Beta"]);

    const reversed = [...useSiteStore.getState().sites].map((s) => s.id).reverse();
    await act(async () => {
      await useSiteStore.getState().reorderSites(reversed);
    });

    expect(useSiteStore.getState().sites.map((s) => s.name)).toEqual(["Beta", "Alpha"]);
    await waitFor(() => {
      expect(listNames()).toEqual(["Beta", "Alpha"]);
    });

    await act(async () => {
      await useSiteStore.getState().loadSites({ force: true });
    });
    expect(useSiteStore.getState().sites.map((s) => s.name)).toEqual(["Beta", "Alpha"]);
    expect(listNames()).toEqual(["Beta", "Alpha"]);
  });

  it("adds a site from the ModelScope template needing only name and key", async () => {
    render(
      <Wrapper>
        <SitesPage />
      </Wrapper>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "按模板添加站点" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "ModelScope 魔搭" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByPlaceholderText("https://api.example.com")).toHaveValue(
      "https://api-inference.modelscope.cn/v1",
    );

    fireEvent.change(within(dialog).getByPlaceholderText("My Relay"), {
      target: { value: "魔搭直连" },
    });
    fireEvent.change(within(dialog).getByPlaceholderText("sk-..."), {
      target: { value: "ms-token" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /保.*存/ }));

    await waitFor(() => {
      const created = useSiteStore.getState().sites.find((site) => site.name === "魔搭直连");
      expect(created?.baseUrl).toBe("https://api-inference.modelscope.cn/v1");
    });
  });
});

function appliedStatus(
  kind: TargetLiveStatus["kind"],
  siteId: string | null,
  siteName: string | null,
): TargetLiveStatus {
  return {
    kind,
    installed: true,
    version: "1.0.0",
    configPath: kind === "claude_code" ? "~/.claude/settings.json" : "~/.codex/config.toml",
    status: siteId ? "applied" : "not_applied",
    appliedSiteId: siteId,
    appliedSiteName: siteName,
    appliedModelId: siteId ? "gpt-4.1" : null,
    providerId: null,
    orphan: false,
    liveSummary: {},
    lastAppliedAt: siteId ? 1 : null,
    staleReason: null,
  };
}
