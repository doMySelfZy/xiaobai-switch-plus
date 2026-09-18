import type { SiteProtocol } from "@/types/domain";

export type SitePresetId = "custom" | "opencode-go" | "modelscope";

export interface SitePreset {
  id: SitePresetId;
  /** 展示名 i18n key */
  nameKey: string;
  baseUrls: string[];
  protocol: SiteProtocol;
  requiresNewapiCreds: boolean;
  /** 免除 newapi 凭据时，说明为什么不用填；null 表示仍按 newapi 口径提示。 */
  quotaNoteKey: string | null;
}

/**
 * 「添加站点」的预填目录。纯数据、不含判定逻辑：命中哪家服务商由
 * `siteProviderKinds.ts` 按 Base URL host 决定，模板本身不写入站点记录。
 */
export const SITE_PRESETS: SitePreset[] = [
  {
    id: "custom",
    nameKey: "sites.presetCustom",
    baseUrls: [],
    protocol: "openai_compatible",
    requiresNewapiCreds: true,
    quotaNoteKey: null,
  },
  {
    id: "opencode-go",
    nameKey: "sites.presetOpencodeGo",
    baseUrls: ["https://opencode.ai/zen/go/v1"],
    protocol: "openai_compatible",
    requiresNewapiCreds: false,
    quotaNoteKey: "sites.quotaNoteOpenCodeGo",
  },
  {
    id: "modelscope",
    nameKey: "sites.presetModelscope",
    baseUrls: ["https://api-inference.modelscope.cn/v1"],
    protocol: "openai_compatible",
    requiresNewapiCreds: false,
    quotaNoteKey: "sites.quotaNoteModelscope",
  },
];

export function sitePresetById(id: string | null | undefined): SitePreset | null {
  return SITE_PRESETS.find((preset) => preset.id === id) ?? null;
}
