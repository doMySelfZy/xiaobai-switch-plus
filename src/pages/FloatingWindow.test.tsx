import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetBrowserMock } from "@/lib/browserMock";
import { FloatingWindow, withAlpha } from "./FloatingWindow";
import "@/i18n";

describe("withAlpha", () => {
  it("applies the cap to opaque colors", () => {
    expect(withAlpha("#141414", 0.72)).toBe("rgba(20, 20, 20, 0.72)");
    expect(withAlpha("rgb(20, 20, 20)", 0.72)).toBe("rgba(20, 20, 20, 0.72)");
  });

  it("never raises an already-translucent color", () => {
    // 深色主题下 antd token 常是「很淡的 rgba」，放大透明度会把卡片变成亮白块
    // —— 这正是截图里看到的问题，用这条锁住。
    expect(withAlpha("rgba(255, 255, 255, 0.08)", 0.6)).toBe("rgba(255, 255, 255, 0.08)");
    expect(withAlpha("rgba(255, 255, 255, 0.9)", 0.6)).toBe("rgba(255, 255, 255, 0.6)");
  });

  it("passes through values it cannot parse", () => {
    expect(withAlpha("var(--x)", 0.5)).toBe("var(--x)");
  });
});

// 悬浮窗要操作真实窗口对象；jsdom 里没有，这里替换掉。
const setSize = vi.fn().mockResolvedValue(undefined);
const setPosition = vi.fn().mockResolvedValue(undefined);
const outerPosition = vi.fn().mockResolvedValue({ x: 100, y: 100 });
const startDragging = vi.fn().mockResolvedValue(undefined);
const onMoved = vi.fn().mockResolvedValue(() => undefined);

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setSize,
    setPosition,
    outerPosition,
    startDragging,
    onMoved,
  }),
  LogicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
  LogicalSize: class {
    constructor(
      public width: number,
      public height: number,
    ) {}
  },
  PhysicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
}));

// 跨窗口事件：悬浮窗靠它跟随设置页的改动。
const listeners = new Map<string, () => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: () => void) => {
    listeners.set(event, handler);
    return Promise.resolve(() => listeners.delete(event));
  }),
  emit: vi.fn().mockResolvedValue(undefined),
}));

// 统计后端调用，用来验证自动刷新确实会发请求。
vi.mock("@/lib/invoke", async () => {
  const actual = await vi.importActual<typeof import("@/lib/invoke")>("@/lib/invoke");
  return {
    ...actual,
    invoke: vi.fn((cmd: string, args?: Record<string, unknown>) => actual.invoke(cmd, args)),
  };
});

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

async function invokeMock() {
  const mod = await import("@/lib/invoke");
  return mod.invoke as unknown as ReturnType<typeof vi.fn>;
}

describe("FloatingWindow", () => {
  beforeEach(() => {
    resetBrowserMock();
    setSize.mockClear();
    setPosition.mockClear();
    outerPosition.mockClear();
    outerPosition.mockResolvedValue({ x: 100, y: 100 });
    startDragging.mockClear();
    onMoved.mockClear();
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("lists site balances in the four display shapes", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    // 「不可用」在多个站点上都会出现，断言按站点所在行限定。
    const cell = (name: string) =>
      within(screen.getByText(name).parentElement as HTMLElement);

    expect(await screen.findByText("Relay A")).toBeInTheDocument();
    expect(cell("Relay A").getByText("$42.50")).toBeInTheDocument();
    // 低余额仍显示金额（警示色由样式承担）
    expect(cell("Relay B").getByText("$1.25")).toBeInTheDocument();
    // 无限额
    expect(cell("Unlimited C").getByText("无限")).toBeInTheDocument();
    // 未知
    expect(cell("Unknown D").getByText("不可用")).toBeInTheDocument();
  });

  it("shows the last update time once balances are loaded", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    expect(await screen.findByText("Relay A")).toBeInTheDocument();
    expect(screen.getByText(/最后更新/)).toBeInTheDocument();
  });

  it("collapses into a small orb and persists the state", async () => {
    const invoke = await invokeMock();
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    expect(await screen.findByText("Relay A")).toBeInTheDocument();

    // 收起：整块面板消失，只剩一个可点的小球；尺寸收成球、状态写库。
    fireEvent.click(screen.getByRole("button", { name: /收\s*起/ }));

    await waitFor(() => {
      expect(screen.queryByText("Relay A")).toBeNull();
    });
    expect(screen.getByRole("button", { name: /展\s*开/ })).toBeInTheDocument();
    await waitFor(() => {
      expect(setSize).toHaveBeenCalled();
    });
    expect(invoke).toHaveBeenCalledWith("set_floating_window_collapsed", { collapsed: true });
  });

  it("expands back into the panel when the orb is clicked", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    expect(await screen.findByText("Relay A")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /收\s*起/ }));
    await waitFor(() => {
      expect(screen.queryByText("Relay A")).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: /展\s*开/ }));
    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });
  });

  it("hands the drag over to the system once the pointer passes the threshold", async () => {
    // 回归：之前在 mousemove 里自己 setPosition，既每帧一次 IPC，又用
    // 「按下原点 + 窗口相对位移」推导目标位置（反馈环，窗口追不上光标）。
    // 现在只判断「算不算拖动」，然后交给系统拖动一次。
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    const header = await screen.findByTestId("floating-header");

    fireEvent.mouseDown(header, { button: 0, clientX: 10, clientY: 10 });
    // 阈值（4px）以内不算拖动。
    fireEvent.mouseMove(document, { clientX: 12, clientY: 12 });
    expect(startDragging).not.toHaveBeenCalled();

    // 越过阈值：交给系统，且不再自己算位置。
    fireEvent.mouseMove(document, { clientX: 40, clientY: 30 });
    await waitFor(() => expect(startDragging).toHaveBeenCalledTimes(1));
    expect(setPosition).not.toHaveBeenCalled();

    // 后续移动不再重复调用；系统拖动期间 mousemove 可能根本收不到。
    fireEvent.mouseMove(document, { clientX: 120, clientY: 90 });
    expect(startDragging).toHaveBeenCalledTimes(1);
    expect(setPosition).not.toHaveBeenCalled();
  });

  it("persists the window position read after the drag, not the mouse delta", async () => {
    // 系统拖动期间拿不到鼠标坐标，落盘必须读窗口的真实位置（outerPosition）。
    const invoke = await invokeMock();
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    const header = await screen.findByTestId("floating-header");
    outerPosition.mockResolvedValue({ x: 1234, y: 567 });

    fireEvent.mouseDown(header, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.mouseMove(document, { clientX: 40, clientY: 30 });
    await waitFor(() => expect(startDragging).toHaveBeenCalledTimes(1));

    fireEvent.mouseUp(document);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_floating_window_position", {
        x: 1234,
        y: 567,
      });
    });
  });

  it("does not collapse when a click follows the drag", async () => {
    // 拖动之后 click 仍会触发：不抑制的话拖完窗口就自己收起了。
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    const header = await screen.findByTestId("floating-header");
    expect(await screen.findByText("Relay A")).toBeInTheDocument();

    fireEvent.mouseDown(header, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.mouseMove(document, { clientX: 40, clientY: 30 });
    await waitFor(() => expect(startDragging).toHaveBeenCalledTimes(1));
    fireEvent.mouseUp(document);
    fireEvent.click(header);

    expect(screen.getByText("Relay A")).toBeInTheDocument();

    // 抑制标志必须复位：下一次点击仍能正常收起。
    await new Promise((resolve) => window.setTimeout(resolve, 5));
    fireEvent.click(header);
    await waitFor(() => {
      expect(screen.queryByText("Relay A")).toBeNull();
    });
  });

  it("does not create a refresh interval because App owns the unified timer", async () => {
    const setIntervalSpy = vi.spyOn(window, "setInterval");
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });
    expect(setIntervalSpy.mock.calls.some(([, ms]) => ms !== 50)).toBe(false);
    setIntervalSpy.mockRestore();
  });

  it("performs a manual refresh from the refresh button", async () => {
    const invoke = await invokeMock();
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });
    const refreshButton = screen.getByRole("button", { name: "刷新" });
    fireEvent.click(refreshButton);

    await waitFor(() => {
      expect(
        invoke.mock.calls.some((call) => call[0] === "refresh_sites_quota"),
      ).toBe(true);
    });
  });

  it("does not register the native refresh event bridge in browser mode", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });
    expect(listeners.get("sites-refresh-finished")).toBeUndefined();
  });

  it("closes the window and turns the feature off", async () => {
    const invoke = await invokeMock();
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });

    // 「关闭」应当真正关掉功能——只 hide 的话下次启动它又冒出来，用户会以为关不掉。
    fireEvent.click(screen.getByRole("button", { name: /关\s*闭/ }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("set_floating_window_enabled", { enabled: false });
    });
  });

  it("shows the reason when a site's balance could not be read", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    // 失败站点显示「不可用」，并挂上可查看的原因。
    expect(await screen.findByText("Failed E")).toBeInTheDocument();
    const cell = within(screen.getByText("Failed E").parentElement as HTMLElement);
    expect(cell.getByText("不可用")).toBeInTheDocument();
  });

  it("keeps the panel translucent without paying for a backdrop blur", async () => {
    render(
      <Wrapper>
        <FloatingWindow />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Relay A")).toBeInTheDocument();
    });

    // 窗口是 transparent，背后没有页面内容可采样：backdrop-filter 是白付的合成开销，
    // 已经去掉。底色必须仍然半透明，否则悬浮窗会变成一块实心板。
    // （窗口侧 transparent(true) 那部分在 floating_window.rs 的测试里。）
    const root = screen.getByTestId("floating-panel");
    const style = root.style;
    expect(style.backdropFilter).toBeFalsy();
    // Webkit 前缀版本不在 DOM 类型里，单独取一下，别让它悄悄回来。
    expect(
      (style as CSSStyleDeclaration & { webkitBackdropFilter?: string }).webkitBackdropFilter,
    ).toBeFalsy();
    const bg = style.background;
    const alpha = Number(/rgba\([^)]*,\s*([\d.]+)\)/.exec(bg)?.[1] ?? "1");
    expect(alpha).toBeGreaterThan(0);
    expect(alpha).toBeLessThan(1);
  });
});
