import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useUIStore } from "@/stores";
import { usePageVisible } from "./usePageVisible";

function setVisibility(state: "visible" | "hidden") {
  vi.spyOn(document, "visibilityState", "get").mockReturnValue(state);
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("usePageVisible", () => {
  beforeEach(() => {
    useUIStore.setState({ activePage: "sites" });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("is true only for the active page", () => {
    const { result, rerender } = renderHook(({ page }) => usePageVisible(page), {
      initialProps: { page: "mcp" as const },
    });

    // 窗口可见，但当前页是 sites：mcp 不算可见。
    expect(result.current).toBe(false);

    act(() => {
      useUIStore.setState({ activePage: "mcp" });
    });
    expect(result.current).toBe(true);

    rerender({ page: "mcp" });
    expect(result.current).toBe(true);

    act(() => {
      useUIStore.setState({ activePage: "sites" });
    });
    expect(result.current).toBe(false);
  });

  it("follows document visibility changes", () => {
    const { result } = renderHook(() => usePageVisible("sites"));
    expect(result.current).toBe(true);

    // KeepAlive 下页面一直挂着：窗口切到后台时必须报不可见。
    act(() => setVisibility("hidden"));
    expect(result.current).toBe(false);

    act(() => setVisibility("visible"));
    expect(result.current).toBe(true);
  });

  it("stops listening to visibilitychange after unmount", () => {
    const remove = vi.spyOn(document, "removeEventListener");
    const { unmount } = renderHook(() => usePageVisible("sites"));

    unmount();

    expect(remove).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
  });
});
