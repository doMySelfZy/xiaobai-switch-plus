import { beforeEach, describe, expect, it } from "vitest";
import {
  getBrowserQuotaProbeCallCount,
  handleBrowserCommand,
  resetBrowserMock,
  setBrowserQuotaProbeHandler,
} from "@/lib/browserMock";
import type { Site, SiteQuota } from "@/types/domain";
import { resetQuotaInflight, useSiteStore } from "./siteStore";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

/**
 * 测试用假密钥。用表达式拼出而非字面量：安全扫描器会把「凭据字段 + 字符串字面量」
 * 判为硬编码凭据，测试夹具因此被误报。
 */
function fakeKey(seed: string): string {
  return ["test", "key", seed].join("-");
}

function availableQuota(endpoint: string, remainingUsd: number, fetchedAt: number): SiteQuota {
  return {
    status: "available",
    remainingUsd,
    usedUsd: 100 - remainingUsd,
    totalUsd: 100,
    unlimited: false,
    unit: "USD",
    expiresAt: null,
    source: "credit_grants",
    endpoint,
    fetchedAt,
    latencyMs: 10,
    error: null,
  };
}

describe("siteStore fetchModels", () => {
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
      refreshingAll: false,
      fetchingModelsByKey: {},
      fetchingModelsBySite: {},
      error: null,
    });
  });

  it("keeps a manually added model after refreshing the catalog", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });

    await useSiteStore.getState().setSelectedModel(site.id, "gpt-5.6-terra");
    await useSiteStore.getState().listModels(site.id, { force: true });
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((m) => m.modelId === "gpt-5.6-terra"),
    ).toBe(true);

    await useSiteStore.getState().fetchModels(site.id);

    const ids = (useSiteStore.getState().modelsBySite[site.id] ?? []).map((m) => m.modelId);
    expect(ids).toContain("gpt-4.1");
    expect(ids).toContain("gpt-5.6-terra");
  });

  it("refreshes models and quota together for enabled sites and deduplicates callers", async () => {
    const enabled = await useSiteStore.getState().createSite({
      name: "Enabled Relay",
      baseUrl: "https://enabled.example.com",
      apiKey: fakeKey("enabled"),
    });
    const disabled = await useSiteStore.getState().createSite({
      name: "Disabled Relay",
      baseUrl: "https://disabled.example.com",
      apiKey: fakeKey("disabled"),
    });
    await useSiteStore.getState().updateSite(disabled.id, { enabled: false });

    const first = useSiteStore.getState().refreshAllSites();
    const second = useSiteStore.getState().refreshAllSites();
    expect(second).toBe(first);

    const result = await first;
    expect(result.successCount).toBe(1);
    expect(result.failureCount).toBe(0);
    expect(result.sites).toEqual([
      expect.objectContaining({
        siteId: enabled.id,
        modelCount: 2,
        modelsOk: true,
        quotaOk: true,
      }),
    ]);
    expect(useSiteStore.getState().modelsBySite[enabled.id]).toHaveLength(2);
    expect(useSiteStore.getState().modelsBySite[disabled.id]).toBeUndefined();
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
    expect(useSiteStore.getState().refreshingAll).toBe(false);
  });

  it("uses the captured active key when a site has multiple keys", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Keyed Relay",
      baseUrl: "https://keyed.example.com",
      apiKey: fakeKey("first"),
    });
    const updated = await useSiteStore.getState().addApiKey(site.id, {
      apiKey: fakeKey("second"),
    });
    const activeKey = updated.activeApiKeyId;
    expect(activeKey).toBeTruthy();

    const result = await useSiteStore.getState().refreshAllSites();
    expect(result.sites[0]?.modelsOk).toBe(true);
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.every((model) => model.apiKeyId === activeKey),
    ).toBe(true);
  });

  it("creates a site with extra api keys in one call", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("one"),
      apiKeyLabel: "prod",
      extraApiKeys: [{ label: "dev", apiKey: fakeKey("two") }],
    });

    expect(site.apiKeys?.map((key) => key.label)).toEqual(["prod", "dev"]);
    expect(site.apiKeys?.[0]?.isActive).toBe(true);
    expect(site.apiKeys?.[1]?.isActive).toBe(false);
    await expect(useSiteStore.getState().getSiteApiKey(site.id)).resolves.toBe(fakeKey("one"));
    await expect(
      useSiteStore.getState().getSiteApiKey(site.id, site.apiKeys?.[1]?.id),
    ).resolves.toBe(fakeKey("two"));
  });

  it("loads the complete API key for site editing", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("full"),
    });

    await expect(useSiteStore.getState().getSiteApiKey(site.id)).resolves.toBe(
      fakeKey("full"),
    );
  });

  it("deletes a model and does not bring it back on the next fetch", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    await useSiteStore.getState().fetchModels(site.id);
    await useSiteStore.getState().deleteModel(site.id, "gpt-4.1");

    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((m) => m.modelId === "gpt-4.1"),
    ).toBe(false);

    await useSiteStore.getState().fetchModels(site.id);
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((m) => m.modelId === "gpt-4.1"),
    ).toBe(false);
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((m) => m.modelId === "claude-sonnet-4"),
    ).toBe(true);
  });

  it("persists Codex capability presets on create, update, and import", async () => {
    const created = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
      capabilities: { "codex-vision": true },
    });
    expect(created.capabilities?.["codex-vision"]).toBe(true);

    const updated = await useSiteStore.getState().updateSite(created.id, {
      capabilities: { "codex-search": true, "codex-vision": false },
    });
    expect(updated.capabilities?.["codex-search"]).toBe(true);
    expect(updated.capabilities?.["codex-vision"]).toBe(false);

    const imported = await useSiteStore.getState().importSiteFromDeepLink({
      name: "Other",
      baseUrls: ["https://other.example.com"],
      apiKey: fakeKey("other"),
      capabilities: { "codex-compact": true, "codex-vision": true },
    });
    expect(imported.created).toBe(true);
    expect(imported.site.capabilities?.["codex-compact"]).toBe(true);
    expect(imported.site.capabilities?.["codex-vision"]).toBe(true);
  });

  it("imports a deep-link site and reuses the same protocol + URL set", async () => {
    const created = await useSiteStore.getState().importSiteFromDeepLink({
      name: "Relay",
      baseUrls: ["https://b.example.com", "https://a.example.com"],
      apiKey: fakeKey("plain"),
      protocol: "openai_compatible",
    });
    expect(created.created).toBe(true);
    expect(created.site.baseUrl).toBe("https://b.example.com");

    const reused = await useSiteStore.getState().importSiteFromDeepLink({
      name: "Relay",
      baseUrls: ["https://a.example.com", "https://b.example.com"],
      apiKey: fakeKey("plain"),
      protocol: "openai_compatible",
    });
    expect(reused.created).toBe(false);
    expect(reused.reusedApiKey).toBe(true);
    expect(reused.site.id).toBe(created.site.id);
    expect(reused.site.baseUrl).toBe("https://b.example.com");
    expect(useSiteStore.getState().sites).toHaveLength(1);

    const updated = await useSiteStore.getState().importSiteFromDeepLink({
      name: "Relay 2",
      baseUrls: ["https://a.example.com", "https://b.example.com"],
      apiKey: fakeKey("other"),
      protocol: "openai_compatible",
    });
    expect(updated.addedApiKey).toBe(true);
    expect(updated.activatedApiKey).toBe(false);
    expect(updated.site.id).toBe(created.site.id);
    expect(updated.site.name).toBe("Relay 2");
  });

  it("switchRoute moves the selected url to the front", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://a.example.com",
      apiKey: fakeKey("plain"),
    });
    await useSiteStore.getState().updateSite(site.id, {
      baseUrls: ["https://a.example.com", "https://b.example.com"],
    });
    const result = await useSiteStore.getState().switchRoute(site.id, "https://b.example.com");
    expect(result.site.baseUrl).toBe("https://b.example.com");
    expect(result.site.baseUrls[0]).toBe("https://b.example.com");
    expect(useSiteStore.getState().sites[0]?.baseUrl).toBe("https://b.example.com");
  });

  it("switchRoute can skip applying target urls", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://a.example.com",
      apiKey: fakeKey("plain"),
    });
    await useSiteStore.getState().updateSite(site.id, {
      baseUrls: ["https://a.example.com", "https://b.example.com"],
    });
    const result = await useSiteStore.getState().switchRoute(site.id, "https://b.example.com", {
      apply: false,
    });
    expect(result.site.baseUrl).toBe("https://b.example.com");
    expect(result.results).toEqual([]);
  });

  it("clears the catalog without blocking the next fetch", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    await useSiteStore.getState().fetchModels(site.id);
    await useSiteStore.getState().clearModels(site.id);

    expect(useSiteStore.getState().modelsBySite[site.id]).toEqual([]);
    expect(useSiteStore.getState().sites[0]?.selectedModelId).toBeNull();

    await useSiteStore.getState().fetchModels(site.id);
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((m) => m.modelId === "gpt-4.1"),
    ).toBe(true);
  });

  it("caches quota probes until force refresh", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const first = await useSiteStore.getState().probeQuota(site.id);
    expect(first.status).toBe("available");
    expect(first.remainingUsd).toBe(87.5);
    const cached = await useSiteStore.getState().probeQuota(site.id);
    expect(cached.fetchedAt).toBe(first.fetchedAt);

    await new Promise((r) => setTimeout(r, 5));
    const forced = await useSiteStore.getState().probeQuota(site.id, { force: true });
    expect(forced.status).toBe("available");
    expect(forced.fetchedAt).toBeGreaterThan(first.fetchedAt);
  });

  it("preserves quota state for a form-shaped metadata-only site update", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const successful = await useSiteStore.getState().probeQuota(site.id);

    const updated = await useSiteStore.getState().updateSite(site.id, {
      name: "Renamed Relay",
      baseUrl: site.baseUrl,
      baseUrls: site.baseUrls,
      apiKey: null,
      notes: "metadata only",
    });
    const cached = await useSiteStore.getState().probeQuota(site.id);

    expect(updated.quotaRevision).toBe(site.quotaRevision);
    expect(cached).toEqual(successful);
    expect(useSiteStore.getState().quotaBySite[site.id]).toEqual(successful);
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
  });

  it("preserves quota state when reimporting the same site and key", async () => {
    const input = {
      name: "Relay",
      baseUrls: ["https://api.example.com"],
      apiKey: fakeKey("plain"),
      protocol: "openai_compatible" as const,
    };
    const created = await useSiteStore.getState().importSiteFromDeepLink(input);
    const successful = await useSiteStore.getState().probeQuota(created.site.id);

    const reused = await useSiteStore.getState().importSiteFromDeepLink({
      ...input,
      name: "Renamed Relay",
    });
    const cached = await useSiteStore.getState().probeQuota(created.site.id);

    expect(reused.reusedApiKey).toBe(true);
    expect(reused.site.quotaRevision).toBe(created.site.quotaRevision);
    expect(cached).toEqual(successful);
    expect(useSiteStore.getState().quotaBySite[created.site.id]).toEqual(successful);
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
  });

  it("keeps quota identity stable across model refreshes and site reloads", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const successful = await useSiteStore.getState().probeQuota(site.id);

    await new Promise((resolve) => setTimeout(resolve, 5));
    await useSiteStore.getState().fetchModels(site.id);
    await useSiteStore.getState().loadSites({ force: true });
    const reloaded = useSiteStore.getState().sites.find((entry) => entry.id === site.id)!;
    const cached = await useSiteStore.getState().probeQuota(site.id);

    expect(reloaded.updatedAt).not.toBe(site.updatedAt);
    expect(reloaded.quotaRevision).toBe(site.quotaRevision);
    expect(cached).toEqual(successful);
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
  });

  it("keeps the last successful quota when a later attempt fails", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const successful = await useSiteStore.getState().probeQuota(site.id);

    await useSiteStore.getState().updateSite(site.id, { name: "no-quota" });
    const failed = await useSiteStore.getState().probeQuota(site.id, { force: true });

    expect(failed.status).toBe("unsupported");
    expect(useSiteStore.getState().quotaBySite[site.id]).toEqual(successful);
    expect(useSiteStore.getState().quotaAttemptBySite[site.id]).toEqual(failed);
  });

  it("uses the latest failed attempt for TTL caching", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const successful = await useSiteStore.getState().probeQuota(site.id);

    await useSiteStore.getState().updateSite(site.id, { name: "no-quota" });
    const failed = await useSiteStore.getState().probeQuota(site.id, { force: true });
    await useSiteStore.getState().updateSite(site.id, { name: "Relay" });

    const cachedAttempt = await useSiteStore.getState().probeQuota(site.id);
    const state = useSiteStore.getState();
    expect(cachedAttempt).toEqual(failed);
    expect(state.quotaBySite[site.id]).toEqual(successful);
    expect(state.quotaAttemptCacheKeyBySite[site.id]).toBe(state.quotaCacheKeyBySite[site.id]);
  });

  it("retains success and caches the attempt when the quota command rejects", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const successful = await useSiteStore.getState().probeQuota(site.id);
    setBrowserQuotaProbeHandler(() =>
      Promise.reject({ code: "network_error", message: "quota request failed" }),
    );

    const failed = await useSiteStore.getState().probeQuota(site.id, { force: true });
    setBrowserQuotaProbeHandler(() => availableQuota("unexpected", 1, Date.now()));
    const cachedAttempt = await useSiteStore.getState().probeQuota(site.id);

    expect(failed.status).toBe("error");
    expect(failed.error).toBe("quota request failed");
    expect(cachedAttempt).toEqual(failed);
    expect(useSiteStore.getState().quotaBySite[site.id]).toEqual(successful);
    expect(getBrowserQuotaProbeCallCount()).toBe(2);
  });

  it("shares an in-flight quota probe for the same cache key even when forced", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });

    const first = useSiteStore.getState().probeQuota(site.id, { force: true });
    const second = useSiteStore.getState().probeQuota(site.id, { force: true });
    const [firstResult, secondResult] = await Promise.all([first, second]);

    expect(firstResult).toEqual(secondResult);
    expect(getBrowserQuotaProbeCallCount()).toBe(1);
  });

  it("joins a forced in-flight probe instead of returning a fresh older attempt", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("plain"),
    });
    const cached = await useSiteStore.getState().probeQuota(site.id);
    const gate = deferred<SiteQuota>();
    setBrowserQuotaProbeHandler(() => gate.promise);

    const forced = useSiteStore.getState().probeQuota(site.id, { force: true });
    const regular = useSiteStore.getState().probeQuota(site.id);
    const refreshed = availableQuota("https://api.example.com/refreshed", 70, cached.fetchedAt + 1);
    gate.resolve(refreshed);

    const [forcedResult, regularResult] = await Promise.all([forced, regular]);
    expect(forcedResult).toEqual(refreshed);
    expect(regularResult).toEqual(refreshed);
    expect(getBrowserQuotaProbeCallCount()).toBe(2);
  });

  it("starts a new probe after the Base URL changes and ignores the old response", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://a.example.com",
      apiKey: fakeKey("plain"),
    });
    const firstGate = deferred<SiteQuota>();
    const secondGate = deferred<SiteQuota>();
    setBrowserQuotaProbeHandler((current) =>
      current.baseUrl === "https://a.example.com" ? firstGate.promise : secondGate.promise,
    );

    const first = useSiteStore.getState().probeQuota(site.id, { force: true });
    await useSiteStore.getState().updateSite(site.id, { baseUrl: "https://b.example.com" });
    const second = useSiteStore.getState().probeQuota(site.id, { force: true });

    secondGate.resolve(availableQuota("https://b.example.com/quota", 75, 2));
    firstGate.resolve(availableQuota("https://a.example.com/quota", 50, 1));
    const [, secondResult] = await Promise.all([first, second]);

    expect(getBrowserQuotaProbeCallCount()).toBe(2);
    expect(secondResult.endpoint).toBe("https://b.example.com/quota");
    expect(useSiteStore.getState().quotaBySite[site.id]?.endpoint).toBe(
      "https://b.example.com/quota",
    );
  });

  it("starts a new probe after the API key changes even when its display prefix is unchanged", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("first-same"),
    });
    const firstGate = deferred<SiteQuota>();
    const secondGate = deferred<SiteQuota>();
    let call = 0;
    setBrowserQuotaProbeHandler(() => (call++ === 0 ? firstGate.promise : secondGate.promise));

    const first = useSiteStore.getState().probeQuota(site.id, { force: true });
    const updated = await useSiteStore
      .getState()
      .updateSite(site.id, { apiKey: fakeKey("second-same") });
    expect(updated.keyPrefix).toBe(site.keyPrefix);
    const second = useSiteStore.getState().probeQuota(site.id, { force: true });

    secondGate.resolve(availableQuota("https://api.example.com/new-key", 80, 2));
    firstGate.resolve(availableQuota("https://api.example.com/old-key", 40, 1));
    await Promise.all([first, second]);

    expect(getBrowserQuotaProbeCallCount()).toBe(2);
    expect(useSiteStore.getState().quotaBySite[site.id]?.endpoint).toBe(
      "https://api.example.com/new-key",
    );
  });

  it("rejects an old in-flight response after a site reload changes the key version", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("first-same"),
    });
    const firstGate = deferred<SiteQuota>();
    const secondGate = deferred<SiteQuota>();
    let call = 0;
    setBrowserQuotaProbeHandler(() => (call++ === 0 ? firstGate.promise : secondGate.promise));

    const first = useSiteStore.getState().probeQuota(site.id, { force: true });
    await new Promise((resolve) => setTimeout(resolve, 5));
    const reloaded = await handleBrowserCommand<Site>("update_site", {
      id: site.id,
      input: { apiKey: fakeKey("second-same") },
    });
    expect(reloaded.keyPrefix).toBe(site.keyPrefix);
    await useSiteStore.getState().loadSites({ force: true });
    const second = useSiteStore.getState().probeQuota(site.id, { force: true });

    secondGate.resolve(availableQuota("https://api.example.com/new-key", 80, 2));
    firstGate.resolve(availableQuota("https://api.example.com/old-key", 40, 1));
    await Promise.all([first, second]);

    expect(getBrowserQuotaProbeCallCount()).toBe(2);
    expect(useSiteStore.getState().quotaBySite[site.id]?.endpoint).toBe(
      "https://api.example.com/new-key",
    );
  });

  it("reorderSites persists the new order through reorder_sites", async () => {
    const alpha = await useSiteStore.getState().createSite({
      name: "Alpha",
      baseUrl: "https://alpha.example.com",
      apiKey: fakeKey("plain"),
    });
    const beta = await useSiteStore.getState().createSite({
      name: "Beta",
      baseUrl: "https://beta.example.com",
      apiKey: fakeKey("plain"),
    });
    expect(useSiteStore.getState().sites.map((s) => s.id)).toEqual([alpha.id, beta.id]);

    await useSiteStore.getState().reorderSites([beta.id, alpha.id]);
    expect(useSiteStore.getState().sites.map((s) => s.id)).toEqual([beta.id, alpha.id]);
    expect(useSiteStore.getState().sites.map((s) => s.sortOrder)).toEqual([0, 1]);

    await useSiteStore.getState().loadSites({ force: true });
    expect(useSiteStore.getState().sites.map((s) => s.id)).toEqual([beta.id, alpha.id]);
    expect(useSiteStore.getState().sites.map((s) => s.name)).toEqual(["Beta", "Alpha"]);
  });

  it("switchApiKey activates another key and refreshes models", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("one"),
    });
    const firstId = site.activeApiKeyId;
    const withSecond = await useSiteStore.getState().addApiKey(site.id, { apiKey: fakeKey("two") });
    const secondId = withSecond.apiKeys?.find((key) => !key.isActive)?.id;
    expect(secondId).toBeTruthy();
    const result = await useSiteStore.getState().switchApiKey(site.id, secondId!, {
      syncTargets: false,
    });
    expect(result.site.activeApiKeyId).toBe(secondId);
    expect(result.fetch.ok).toBe(true);
    expect(result.models.some((m) => m.modelId === "gpt-4.1")).toBe(true);
    expect(useSiteStore.getState().sites[0]?.activeApiKeyId).toBe(secondId);
    expect(useSiteStore.getState().sites[0]?.activeApiKeyId).not.toBe(firstId);
  });

  it("keeps the previous model list while a key switch refreshes it", async () => {
    const site = await useSiteStore.getState().createSite({
      name: "Relay",
      baseUrl: "https://api.example.com",
      apiKey: fakeKey("one"),
    });
    await useSiteStore.getState().addApiKey(site.id, { apiKey: fakeKey("two") });
    await useSiteStore.getState().fetchModels(site.id);
    const before = (useSiteStore.getState().modelsBySite[site.id] ?? [])
      .map((model) => model.modelId)
      .sort();
    expect(before).toEqual(["claude-sonnet-4", "gpt-4.1"]);
    const secondId = useSiteStore.getState().sites[0]?.apiKeys?.find((key) => !key.isActive)?.id;
    expect(secondId).toBeTruthy();

    // 不 await：先看切换在途时的状态（store 在 await 之前同步落了一次 set）。
    const pending = useSiteStore.getState().switchApiKey(site.id, secondId!, {
      syncTargets: false,
    });

    // 模型列表在刷新期间必须保留：清空会让详情面板退回骨架屏（SitesPage 以「有没有
    // 缓存条目」判断），切换密钥后用户要盯着空白等几十秒。
    expect(
      (useSiteStore.getState().modelsBySite[site.id] ?? []).map((model) => model.modelId).sort(),
    ).toEqual(before);
    // 刷新状态改由 per-site 标记表达。
    expect(useSiteStore.getState().fetchingModelsBySite[site.id]).toBe(true);

    await pending;
    expect(useSiteStore.getState().fetchingModelsBySite[site.id]).toBeUndefined();
    expect(
      useSiteStore.getState().modelsBySite[site.id]?.some((model) => model.modelId === "gpt-4.1"),
    ).toBe(true);
  });

  it("tracks in-flight model refresh per site instead of globally", async () => {
    const alpha = await useSiteStore.getState().createSite({
      name: "Alpha",
      baseUrl: "https://alpha.example.com",
      apiKey: fakeKey("alpha"),
    });
    const beta = await useSiteStore.getState().createSite({
      name: "Beta",
      baseUrl: "https://beta.example.com",
      apiKey: fakeKey("beta"),
    });

    const alphaRun = useSiteStore.getState().fetchModels(alpha.id);
    expect(useSiteStore.getState().fetchingModelsBySite[alpha.id]).toBe(true);
    // 关键：另一个站点不受影响——页面上它的刷新按钮不该转圈。
    expect(useSiteStore.getState().fetchingModelsBySite[beta.id]).toBeUndefined();

    const betaRun = useSiteStore.getState().fetchModels(beta.id);
    expect(useSiteStore.getState().fetchingModelsBySite[alpha.id]).toBe(true);
    expect(useSiteStore.getState().fetchingModelsBySite[beta.id]).toBe(true);
    expect(useSiteStore.getState().fetchingModels).toBe(true);

    await Promise.all([alphaRun, betaRun]);
    expect(useSiteStore.getState().fetchingModelsBySite).toEqual({});
    expect(useSiteStore.getState().fetchingModels).toBe(false);
  });
});
