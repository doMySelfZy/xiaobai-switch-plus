import { sitePresetById, type SitePreset } from "./sitePresets";

/**
 * 服务商识别的单一来源。判据只有 Base URL host，与 Rust
 * `quota_probe::is_opencode_go_base` / `is_modelscope_base` 同一口径：
 * 只接受 https、host 严格相等，避免 `api.opencode.ai`、`opencode.ai.evil.com`
 * 这类形似 host 被误判进专用探测链。
 */
function parseHttpsUrl(baseUrl: string | null | undefined): URL | null {
  if (!baseUrl) return null;
  try {
    const url = new URL(baseUrl.trim());
    return url.protocol === "https:" ? url : null;
  } catch {
    return null;
  }
}

function hasZenGoSegments(url: URL): boolean {
  const segments = url.pathname.split("/").filter(Boolean);
  return segments.some((segment, index) => segment === "zen" && segments[index + 1] === "go");
}

export function isOpenCodeGoBase(baseUrl: string | null | undefined): boolean {
  const url = parseHttpsUrl(baseUrl);
  return url !== null && url.hostname === "opencode.ai" && hasZenGoSegments(url);
}

export function isModelScopeBase(baseUrl: string | null | undefined): boolean {
  const url = parseHttpsUrl(baseUrl);
  return url !== null && url.hostname === "api-inference.modelscope.cn";
}

/** 命中已知服务商时返回其模板描述符，自定义中转返回 null。 */
export function sitePresetFor(baseUrl: string | null | undefined): SitePreset | null {
  if (isOpenCodeGoBase(baseUrl)) return sitePresetById("opencode-go");
  if (isModelScopeBase(baseUrl)) return sitePresetById("modelscope");
  return null;
}

/**
 * 该 Base URL 还需要哪些额外凭据：返回说明文案 key 表示免除 newapi 访问令牌 / userId
 * 并解释为什么，返回 null 表示仍按 newapi 口径提示。只看输入，不看用户点过哪个模板。
 */
export function quotaCredentialHint(baseUrl: string | null | undefined): string | null {
  const preset = sitePresetFor(baseUrl);
  return preset && !preset.requiresNewapiCreds ? preset.quotaNoteKey : null;
}
