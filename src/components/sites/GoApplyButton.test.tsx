import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GoApplyButton } from "./GoApplyButton";
import { useUIStore } from "@/stores";
import i18n from "@/i18n";

const CYCLE_MS = 3000;
const FADE_MS = 180;

describe("GoApplyButton", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("zh-CN");
    useUIStore.setState({ activePage: "sites" });
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    useUIStore.setState({ activePage: "sites" });
  });

  it("rotates the target label while the sites page is visible", () => {
    render(<GoApplyButton onApply={() => undefined} />);
    expect(screen.getByText("去 Claude Code 应用")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(CYCLE_MS + FADE_MS);
    });

    expect(screen.getByText("去 Codex 应用")).toBeInTheDocument();
  });

  it("does not run the rotation timer while the sites page is hidden", () => {
    // SitesPage 被 KeepAlivePages 常驻：切到别的页时它只是 display:none，
    // 定时器不能继续转（每次 setState 都会重渲染按钮）。
    render(<GoApplyButton onApply={() => undefined} />);

    act(() => {
      useUIStore.setState({ activePage: "apply" });
    });
    act(() => {
      vi.advanceTimersByTime(CYCLE_MS * 10);
    });
    expect(screen.getByText("去 Claude Code 应用")).toBeInTheDocument();

    // 切回站点页：轮换恢复。
    act(() => {
      useUIStore.setState({ activePage: "sites" });
    });
    act(() => {
      vi.advanceTimersByTime(CYCLE_MS + FADE_MS);
    });
    expect(screen.getByText("去 Codex 应用")).toBeInTheDocument();
  });
});
