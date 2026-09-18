import { describe, expect, it } from "vitest";
import {
  isOpenCodeGoBase,
  isModelScopeBase,
  quotaCredentialHint,
  sitePresetFor,
} from "./siteProviderKinds";
import { SITE_PRESETS, sitePresetById } from "./sitePresets";

describe("isOpenCodeGoBase", () => {
  // 用例与 src-tauri/src/quota_probe/mod.rs 的 opencode_go_base_detection 逐条对齐
  it("accepts the official zen/go channel over https", () => {
    expect(isOpenCodeGoBase("https://opencode.ai/zen/go/v1")).toBe(true);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/go")).toBe(true);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/go/v1/")).toBe(true);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/go?x=1")).toBe(true);
    expect(isOpenCodeGoBase("  https://opencode.ai/zen/go/v1  ")).toBe(true);
  });

  it("rejects look-alike hosts, non-https schemes and non-adjacent path segments", () => {
    expect(isOpenCodeGoBase("https://opencode.ai/zen/v1")).toBe(false);
    expect(isOpenCodeGoBase("https://opencode.ai")).toBe(false);
    expect(isOpenCodeGoBase("https://api.opencode.ai/zen/go/v1")).toBe(false);
    expect(isOpenCodeGoBase("https://opencode.ai.evil.com/zen/go/v1")).toBe(false);
    expect(isOpenCodeGoBase("http://opencode.ai/zen/go/v1")).toBe(false);
    expect(isOpenCodeGoBase("ftp://opencode.ai/zen/go/v1")).toBe(false);
    expect(isOpenCodeGoBase("opencode.ai/zen/go/v1")).toBe(false);
    expect(isOpenCodeGoBase("not a url")).toBe(false);
    expect(isOpenCodeGoBase("")).toBe(false);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/gopher")).toBe(false);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/go-v2/v1")).toBe(false);
    expect(isOpenCodeGoBase("https://opencode.ai/zen/goose/v1")).toBe(false);
  });
});

describe("isModelScopeBase", () => {
  it("accepts the official API-Inference host over https", () => {
    expect(isModelScopeBase("https://api-inference.modelscope.cn/v1")).toBe(true);
    expect(isModelScopeBase("https://api-inference.modelscope.cn")).toBe(true);
    expect(isModelScopeBase("https://api-inference.modelscope.cn/v1/")).toBe(true);
    expect(isModelScopeBase("https://api-inference.modelscope.cn/v1?x=1")).toBe(true);
    expect(isModelScopeBase("  https://api-inference.modelscope.cn/v1  ")).toBe(true);
  });

  it("rejects the console host, look-alike hosts and non-https schemes", () => {
    expect(isModelScopeBase("https://modelscope.cn/v1")).toBe(false);
    expect(isModelScopeBase("https://www.modelscope.cn/v1")).toBe(false);
    expect(isModelScopeBase("modelscope.cn")).toBe(false);
    expect(isModelScopeBase("https://api-inference.modelscope.cn.evil.com/v1")).toBe(false);
    expect(isModelScopeBase("https://evil.com/api-inference.modelscope.cn/v1")).toBe(false);
    expect(isModelScopeBase("http://api-inference.modelscope.cn/v1")).toBe(false);
    expect(isModelScopeBase("not a url")).toBe(false);
    expect(isModelScopeBase("")).toBe(false);
  });

  it("ignores the port, same as Rust Url::host_str", () => {
    expect(isModelScopeBase("https://api-inference.modelscope.cn:8443/v1")).toBe(true);
  });
});

describe("sitePresetFor", () => {
  it("maps a recognized host back to its preset", () => {
    expect(sitePresetFor("https://opencode.ai/zen/go/v1")?.id).toBe("opencode-go");
    expect(sitePresetFor("https://api-inference.modelscope.cn/v1")?.id).toBe("modelscope");
  });

  it("returns null for custom relays and malformed input", () => {
    expect(sitePresetFor("https://relay.example.com/v1")).toBeNull();
    expect(sitePresetFor("https://opencode.ai/v1")).toBeNull();
    expect(sitePresetFor("")).toBeNull();
    expect(sitePresetFor(undefined)).toBeNull();
  });
});

describe("quotaCredentialHint", () => {
  it("returns an explanation only when newapi credentials are waived", () => {
    expect(quotaCredentialHint("https://opencode.ai/zen/go/v1")).toBeTruthy();
    expect(quotaCredentialHint("https://api-inference.modelscope.cn/v1")).toBeTruthy();
  });

  it("returns null for a custom host so the newapi group stays visible", () => {
    expect(quotaCredentialHint("https://relay.example.com/v1")).toBeNull();
    expect(quotaCredentialHint("")).toBeNull();
  });
});

describe("SITE_PRESETS", () => {
  it("offers custom plus the two documented vendors", () => {
    expect(SITE_PRESETS.map((preset) => preset.id)).toEqual([
      "custom",
      "opencode-go",
      "modelscope",
    ]);
  });

  it("resolves presets by id and tolerates unknown ids", () => {
    expect(sitePresetById("modelscope")?.baseUrls).toEqual([
      "https://api-inference.modelscope.cn/v1",
    ]);
    expect(sitePresetById("nope")).toBeNull();
  });

  // 预填的 Base URL 必须能被 host 判定回指同一个模板，否则用户改完 host 后免除提示会跳变。
  it("prefills a base URL that resolves back to itself", () => {
    for (const preset of SITE_PRESETS) {
      if (preset.baseUrls.length === 0) continue;
      expect(sitePresetFor(preset.baseUrls[0])?.id).toBe(preset.id);
    }
  });

  it("explains every credential waiver", () => {
    for (const preset of SITE_PRESETS) {
      if (preset.requiresNewapiCreds) continue;
      expect(preset.quotaNoteKey, preset.id).toBeTruthy();
    }
  });
});
