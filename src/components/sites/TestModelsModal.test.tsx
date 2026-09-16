import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetBrowserMock } from "@/lib/browserMock";
import { useSiteStore } from "@/stores";
import type { Site, SiteModel } from "@/types/domain";
import { TestModelsModal } from "./TestModelsModal";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

function extraModel(siteId: string, modelId: string): SiteModel {
  return {
    id: modelId,
    siteId,
    modelId,
    displayName: modelId,
    ownedBy: null,
    raw: null,
    isManual: true,
  };
}

async function seedSite() {
  const site = await useSiteStore.getState().createSite({
    name: "Relay One",
    baseUrl: "https://api.example.com",
    apiKey: "sk-test",
  });
  await useSiteStore.getState().fetchModels(site.id);
  return {
    site: useSiteStore.getState().sites[0] as Site,
    models: useSiteStore.getState().modelsBySite[site.id] ?? [],
  };
}

describe("TestModelsModal", () => {
  beforeEach(() => {
    resetBrowserMock();
    useSiteStore.setState({
      sites: [],
      modelsBySite: {},
      modelsLoadingBySite: {},
      loading: false,
      hydrated: false,
      fetchingModels: false,
      error: null,
    });
  });

  afterEach(() => {
    resetBrowserMock();
  });

  it("groups models and selects all by default", async () => {
    const { site, models } = await seedSite();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={() => undefined} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("测试模型")).toBeInTheDocument();
    expect(document.querySelector('[data-model-group="gpt"]')).toBeTruthy();
    expect(document.querySelector('[data-model-group="claude"]')).toBeTruthy();
    expect(within(dialog).getByRole("checkbox", { name: "gpt-4.1" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "claude-sonnet-4" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "全选" })).toBeChecked();
    expect(within(dialog).getByText("已选 2 / 2")).toBeInTheDocument();
  });

  it("marks select-all indeterminate after unchecking one row", async () => {
    const { site, models } = await seedSite();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={() => undefined} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "gpt-4.1" }));
    expect(within(dialog).getByRole("checkbox", { name: "gpt-4.1" })).not.toBeChecked();
    const selectAll = within(dialog).getByRole("checkbox", { name: "全选" });
    expect(selectAll).not.toBeChecked();
    expect(selectAll.closest("label") ?? selectAll.parentElement).toBeTruthy();
    expect(
      dialog.querySelector(".ant-checkbox-indeterminate") ||
        selectAll.closest(".ant-checkbox-wrapper")?.querySelector(".ant-checkbox-indeterminate"),
    ).toBeTruthy();
    expect(within(dialog).getByText("已选 1 / 2")).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("checkbox", { name: "全选" }));
    expect(within(dialog).getByRole("checkbox", { name: "gpt-4.1" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "claude-sonnet-4" })).toBeChecked();
  });

  it("toggles only the models in a group", async () => {
    const { site, models } = await seedSite();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={() => undefined} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "gpt" }));
    expect(within(dialog).getByRole("checkbox", { name: "gpt-4.1" })).not.toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "claude-sonnet-4" })).toBeChecked();
  });

  it("runs a serial test and shows latency on success", async () => {
    const { site, models } = await seedSite();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={() => undefined} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "立即测试" }));

    await waitFor(() => {
      expect(document.querySelector('[data-model-probe-row="gpt-4.1"]')).toHaveAttribute(
        "data-probe-status",
        "ok",
      );
      expect(document.querySelector('[data-model-probe-row="claude-sonnet-4"]')).toHaveAttribute(
        "data-probe-status",
        "ok",
      );
    });
    expect(within(dialog).getAllByText("12ms").length).toBeGreaterThan(0);
    expect(await screen.findByText("全部通过（2）")).toBeInTheDocument();
  });

  it("shows a truncated error with a tooltip for a failed model", async () => {
    const { site } = await seedSite();
    const fail = extraModel(site.id, "gpt-fail-long");
    render(
      <Wrapper>
        <TestModelsModal
          open
          site={site}
          models={[fail]}
          onClose={() => undefined}
        />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "立即测试" }));

    const error = await waitFor(() => {
      const el = dialog.querySelector("[data-probe-error]");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    expect(error).toHaveClass("truncate");
    expect(error.textContent?.length).toBeGreaterThan(20);

    fireEvent.mouseEnter(error);
    const tip = await screen.findByRole("tooltip");
    expect(tip.textContent).toMatch(/very long diagnostic payload/);
  });

  it("disables test now when nothing is selected", async () => {
    const { site, models } = await seedSite();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={() => undefined} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "全选" }));
    expect(within(dialog).getByRole("button", { name: "立即测试" })).toBeDisabled();
  });

  it("cancels the pending result frame when the dialog is closed mid-run", async () => {
    const { site, models } = await seedSite();
    const onClose = vi.fn();
    render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={onClose} />
      </Wrapper>,
    );

    const dialog = await screen.findByRole("dialog");
    // 结果按帧批量提交：把 rAF 钉住，观察关闭时是否把未执行的帧回调撤掉。
    const rafSpy = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((() => 7) as unknown as typeof window.requestAnimationFrame);
    const cancelSpy = vi.spyOn(window, "cancelAnimationFrame");

    fireEvent.click(within(dialog).getByRole("button", { name: "立即测试" }));
    await waitFor(() => {
      expect(rafSpy).toHaveBeenCalled();
    });

    fireEvent.click(within(dialog).getByRole("button", { name: /取\s*消/ }));
    expect(cancelSpy).toHaveBeenCalledWith(7);
    expect(onClose).toHaveBeenCalled();

    rafSpy.mockRestore();
    cancelSpy.mockRestore();
  });

  it("clears results and restores selection after close and reopen", async () => {
    const { site, models } = await seedSite();
    const onClose = vi.fn();
    const view = render(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={onClose} />
      </Wrapper>,
    );

    let dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "gpt-4.1" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "立即测试" }));
    await waitFor(() => {
      expect(document.querySelector('[data-probe-status="ok"]')).toBeTruthy();
    });

    await waitFor(() => {
      expect(within(dialog).getByRole("button", { name: /立\s*即\s*测\s*试/ })).not.toHaveClass(
        "ant-btn-loading",
      );
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /取\s*消/ }));
    expect(onClose).toHaveBeenCalled();

    view.rerender(
      <Wrapper>
        <TestModelsModal open={false} site={site} models={models} onClose={onClose} />
      </Wrapper>,
    );
    await act(async () => {
      await Promise.resolve();
    });
    view.rerender(
      <Wrapper>
        <TestModelsModal open site={site} models={models} onClose={onClose} />
      </Wrapper>,
    );

    dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("checkbox", { name: "gpt-4.1" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "claude-sonnet-4" })).toBeChecked();
    expect(document.querySelector('[data-probe-status="ok"]')).toBeNull();
  });
});
