import { beforeEach, describe, expect, it } from "vitest";
import { useProxyStore } from "@/stores/proxyStore";
import { resetBrowserMock } from "@/lib/browserMock";

describe("proxyStore", () => {
  beforeEach(() => {
    resetBrowserMock();
    useProxyStore.setState({ status: null, requests: [], loading: false });
  });

  it("reports a stopped proxy with takeover flags intact", async () => {
    await useProxyStore.getState().loadStatus();
    const status = useProxyStore.getState().status;
    expect(status?.running).toBe(false);
    expect(status?.port).toBe(18087);
    expect(status?.targets.map((item) => item.target)).toEqual([
      "claude_code",
      "codex",
      "pi",
      "prime",
    ]);
    expect(status?.targets.every((item) => !item.takeover)).toBe(true);
  });

  it("start and stop flip the running flag", async () => {
    await useProxyStore.getState().start();
    expect(useProxyStore.getState().status?.running).toBe(true);

    await useProxyStore.getState().stop();
    expect(useProxyStore.getState().status?.running).toBe(false);
  });

  it("refuses takeover while the proxy is stopped", async () => {
    await useProxyStore.getState().loadStatus();
    await expect(
      useProxyStore.getState().setTakeover("claude_code", true),
    ).rejects.toMatchObject({ code: "proxy_not_running" });
  });

  it("enables takeover per target once running and keeps others direct", async () => {
    await useProxyStore.getState().start();
    await useProxyStore.getState().setTakeover("codex", true);

    const targets = useProxyStore.getState().status?.targets ?? [];
    const codex = targets.find((item) => item.target === "codex");
    const claude = targets.find((item) => item.target === "claude_code");
    expect(codex?.takeover).toBe(true);
    expect(claude?.takeover).toBe(false);
    // 接管地址带路径口令与目标段，且是回环地址。
    expect(codex?.clientBaseUrl).toContain("/t/codex");
    expect(codex?.clientBaseUrl?.startsWith("http://127.0.0.1:")).toBe(true);
  });

  it("clears the request log", async () => {
    await useProxyStore.getState().clearRequests();
    expect(useProxyStore.getState().requests).toEqual([]);
  });
});
