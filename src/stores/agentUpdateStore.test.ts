import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

/** TTL 与在途去重都是模块级状态，每个测试重新加载模块，避免互相污染。 */
async function freshStore() {
  vi.resetModules();
  const { useAgentUpdateStore } = await import("./agentUpdateStore");
  return useAgentUpdateStore;
}

const STATUS = [
  {
    kind: "claude_code",
    name: "Claude Code",
    currentVersion: "1.0.0",
    latestVersion: "1.1.0",
    hasUpdate: true,
    lastCheckAt: 1,
  },
];

describe("agentUpdateStore.checkUpdates", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(STATUS);
  });

  it("reuses a fresh result instead of rerunning npm on every page entry", async () => {
    const store = await freshStore();

    await store.getState().checkUpdates();
    await store.getState().checkUpdates();

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(store.getState().updateStatuses.claude_code?.latestVersion).toBe("1.1.0");
  });

  it("still checks for real when the caller forces a refresh", async () => {
    const store = await freshStore();

    await store.getState().checkUpdates();
    await store.getState().checkUpdates({ force: true });

    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it("shares one in-flight check between concurrent callers", async () => {
    let release!: (value: unknown) => void;
    invokeMock.mockImplementation(
      () => new Promise((resolve) => { release = resolve; }),
    );
    const store = await freshStore();

    const first = store.getState().checkUpdates();
    const second = store.getState().checkUpdates();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    release(STATUS);
    await Promise.all([first, second]);
    expect(store.getState().checking).toBe(false);
  });

  it("retries after a failure instead of caching the empty result", async () => {
    const store = await freshStore();
    invokeMock.mockRejectedValueOnce(new Error("npm not found"));

    // 检查失败不抛出（页面挂载路径不该炸），但也不能进入新鲜窗口。
    await store.getState().checkUpdates();
    await store.getState().checkUpdates();

    expect(invokeMock).toHaveBeenCalledTimes(2);
  });
});
