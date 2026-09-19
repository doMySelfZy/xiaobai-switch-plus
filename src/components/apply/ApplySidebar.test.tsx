import { fireEvent, render, screen } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import { beforeEach, describe, expect, it } from "vitest";
import { useApplyStore, useUIStore } from "@/stores";
import { ApplySidebar } from "./ApplySidebar";
import "@/i18n";

describe("ApplySidebar", () => {
  beforeEach(() => {
    useUIStore.setState({ applyTab: "claude_code" });
    useApplyStore.setState({
      statuses: [
        {
          kind: "pi",
          installed: true,
          version: "0.84.3",
          configPath: "/tmp/models.json",
          status: "applied",
          appliedSiteId: "site-1",
          appliedSiteName: "Site",
          appliedModelId: "model-a",
          providerId: "xiaobai_site1",
          orphan: false,
          liveSummary: {},
          lastAppliedAt: 1,
          staleReason: null,
        },
      ],
    });
  });

  it("renders the third Pi tab with its icon and selects it", () => {
    render(
      <ConfigProvider>
        <AntdApp>
          <ApplySidebar />
        </AntdApp>
      </ConfigProvider>,
    );
    const pi = screen.getByText("Pi");
    expect(pi.closest("li")?.querySelector('[data-icon="pi"]')).toBeTruthy();
    fireEvent.click(pi);
    expect(useUIStore.getState().applyTab).toBe("pi");
  });

  it("renders the Prime tab with its icon and selects it", () => {
    render(
      <ConfigProvider>
        <AntdApp>
          <ApplySidebar />
        </AntdApp>
      </ConfigProvider>,
    );
    const prime = screen.getByText("Prime");
    expect(prime.closest("li")?.querySelector('[data-icon="prime"]')).toBeTruthy();
    fireEvent.click(prime);
    expect(useUIStore.getState().applyTab).toBe("prime");
  });
});
