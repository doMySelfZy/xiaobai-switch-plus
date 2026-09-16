import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
  isAppError: (e: unknown) =>
    typeof e === "object" && e !== null && "code" in e && "message" in e,
  isTauri: () => true,
}));

/** TTL 与在途去重都是模块级状态，每个测试重新加载模块，避免互相污染。 */
async function freshStore() {
  vi.resetModules();
  const { useMcpUpdateStore } = await import("./mcpUpdateStore");
  return useMcpUpdateStore;
}

const STATUS = [
  {
    id: "srv-1",
    name: "Filesystem",
    currentVersion: "1.0.0",
    latestVersion: "1.1.0",
    hasUpdate: true,
    lastCheckAt: 1,
  },
];

describe("mcpUpdateStore.checkUpdates", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(STATUS);
  });

  it("reuses a fresh result instead of rerunning npm on every page entry", async () => {
    const store = await freshStore();

    await store.getState().checkUpdates();
    await store.getState().checkUpdates();

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(store.getState().updateStatuses).toEqual(STATUS);
    expect(store.getState().lastCheckTime).not.toBeNull();
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

    await expect(store.getState().checkUpdates()).rejects.toThrow("npm not found");
    expect(store.getState().checking).toBe(false);

    await store.getState().checkUpdates();
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });
});
