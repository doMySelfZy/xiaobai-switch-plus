import { describe, expect, it, vi } from "vitest";
import type { TFunction } from "i18next";
import type { SiteQuota } from "@/types/domain";
import {
  clampQuotaPercent,
  formatQuotaAmountParts,
  formatQuotaAmount,
  formatQuotaAmountLocalized,
  formatQuotaUpdatedText,
  formatUsd,
  isBalanceQuotaSummary,
  isQuotaCacheFresh,
  normalizeQuotaUnit,
  QUOTA_TTL_MS,
  quotaCacheKey,
  quotaRemainingPercent,
  quotaRemainingTone,
  quotaTone,
  quotaWindowLabelKey,
  quotaWindowShortLabelKey,
  shouldShowExpiry,
  windowRemainingPercent,
} from "./quotaProbe";

function quota(partial: Partial<SiteQuota>): SiteQuota {
  return {
    status: "available",
    remainingUsd: 87.5,
    usedUsd: 12.5,
    totalUsd: 100,
    unlimited: false,
    expiresAt: null,
    source: "credit_grants",
    endpoint: "https://api.example.com/v1/dashboard/billing/credit_grants",
    fetchedAt: 1,
    latencyMs: 10,
    error: null,
    ...partial,
  };
}

describe("quotaProbe helpers", () => {
  it("formats USD with a dollar sign and two decimals", () => {
    expect(formatUsd(12.5)).toBe("$12.50");
    expect(formatUsd(0)).toBe("$0.00");
  });

  it("formats CNY display amounts from token usage", () => {
    expect(formatQuotaAmount(999.693074, "CNY")).toBe("¥999.69");
    expect(formatQuotaAmount(1000, "cny")).toBe("¥1,000.00");
  });

  it("returns a localizable unit key for raw quota amounts", () => {
    expect(formatQuotaAmountParts(108_886_337, "quota")).toEqual({
      value: "108,886,337.00",
      unit: null,
      unitI18nKey: "sites.quotaUnitRaw",
    });
    expect(formatQuotaAmountParts(24_035, "RAW_QUOTA")).toEqual({
      value: "24,035.00",
      unit: null,
      unitI18nKey: "sites.quotaUnitRaw",
    });
  });

  it("normalizes every magicube spelling onto one unit", () => {
    expect(normalizeQuotaUnit("MAGICUBE")).toBe("MAGICUBE");
    expect(normalizeQuotaUnit("magicubes")).toBe("MAGICUBE");
    expect(normalizeQuotaUnit("魔粒")).toBe("MAGICUBE");
  });

  it("formats magicube balances as points with no currency sign", () => {
    expect(formatQuotaAmountParts(1000.5, "MAGICUBE")).toEqual({
      value: "1,000.5",
      unit: null,
      unitI18nKey: "sites.quotaUnitMagicube",
    });
    // 上游给几位小数就显示几位，但最多 2 位。
    expect(formatQuotaAmountParts(42, "MAGICUBE").value).toBe("42");
    expect(formatQuotaAmountParts(1234.567, "MAGICUBE").value).toBe("1,234.57");

    const zh = vi.fn(
      (key: string) => (key === "sites.quotaUnitMagicube" ? "魔粒" : key),
    ) as unknown as TFunction;
    const en = vi.fn(
      (key: string) => (key === "sites.quotaUnitMagicube" ? "Magicubes" : key),
    ) as unknown as TFunction;
    const zhText = formatQuotaAmountLocalized(1000.5, "MAGICUBE", zh);
    const enText = formatQuotaAmountLocalized(1000.5, "MAGICUBE", en);
    expect(zhText).toBe("1,000.5 魔粒");
    expect(enText).toBe("1,000.5 Magicubes");
    // 魔粒既不是金额也不是次数，任何货币符号都会读错口径。
    expect(zhText).not.toMatch(/[$¥]/);
    expect(enText).not.toMatch(/[$¥]/);
  });

  it("labels a magicube amount as magicube in the non-i18n formatter", () => {
    // formatQuotaAmount 只有硬编码回退名；曾经所有 unitI18nKey 都被印成 RAW_QUOTA。
    expect(formatQuotaAmount(1000.5, "MAGICUBE")).toBe("1,000.5 MAGICUBE");
    expect(formatQuotaAmount(24_035, "RAW_QUOTA")).toBe("24,035.00 RAW_QUOTA");
  });

  it("builds a cache key only from quota-relevant site configuration", () => {
    expect(
      quotaCacheKey({
        id: "site-1",
        baseUrl: "https://api.example.com",
        quotaRevision: "credential-2",
      }),
    ).toBe("site-1:https://api.example.com:credential-2");
  });

  it("treats quota snapshots as fresh inside the TTL window", () => {
    const now = 1_000_000;
    expect(isQuotaCacheFresh(quota({ fetchedAt: now - QUOTA_TTL_MS + 1 }), now)).toBe(true);
    expect(isQuotaCacheFresh(quota({ fetchedAt: now - QUOTA_TTL_MS }), now)).toBe(false);
  });

  it("hides expiry that is missing, past, or far-future", () => {
    const now = 1_700_000_000;
    expect(shouldShowExpiry(null, now)).toBe(false);
    expect(shouldShowExpiry(now - 10, now)).toBe(false);
    expect(shouldShowExpiry(now + 86_400, now)).toBe(true);
    expect(shouldShowExpiry(Date.UTC(2099, 0, 1) / 1000, now)).toBe(false);
  });

  it("computes remaining percent and tone", () => {
    expect(quotaRemainingPercent(quota({ remainingUsd: 87.5, totalUsd: 100 }))).toBe(87.5);
    expect(quotaTone(quota({ remainingUsd: 87.5, totalUsd: 100 }))).toBe("ok");
    // 余额与窗口共用同一套剩余阈值：剩 25% 中性、剩 15% 告警、剩 5% 危险。
    expect(quotaTone(quota({ remainingUsd: 25, totalUsd: 100 }))).toBe("ok");
    expect(quotaTone(quota({ remainingUsd: 15, totalUsd: 100 }))).toBe("warn");
    expect(quotaTone(quota({ remainingUsd: 5, totalUsd: 100 }))).toBe("danger");
    expect(quotaTone(quota({ remainingUsd: 0, totalUsd: 100 }))).toBe("danger");
    // 无总额（无限额度哨兵值）时退化为绝对金额：不足 1 美元仍提醒。
    expect(quotaTone(quota({ remainingUsd: 0.4, totalUsd: null }))).toBe("warn");
    expect(quotaRemainingPercent(quota({ unlimited: true, totalUsd: null }))).toBeNull();
  });

  it("resolves window label keys with a null fallback", () => {
    expect(quotaWindowLabelKey("rolling")).toBe("sites.quotaWindowRolling");
    expect(quotaWindowLabelKey("weekly")).toBe("sites.quotaWindowWeekly");
    expect(quotaWindowLabelKey("monthly")).toBe("sites.quotaWindowMonthly");
    expect(quotaWindowLabelKey("daily")).toBeNull();
  });

  it("clamps window percents into 0-100", () => {
    expect(clampQuotaPercent(83.4)).toBe(83.4);
    expect(clampQuotaPercent(-5)).toBe(0);
    expect(clampQuotaPercent(120)).toBe(100);
  });

  it("maps window kinds to short list labels", () => {
    expect(quotaWindowShortLabelKey("rolling")).toBe("sites.quotaWindowShortRolling");
    expect(quotaWindowShortLabelKey("weekly")).toBe("sites.quotaWindowShortWeekly");
    expect(quotaWindowShortLabelKey("monthly")).toBe("sites.quotaWindowShortMonthly");
    expect(quotaWindowShortLabelKey("daily")).toBeNull();
  });

  it("converts usage percent into remaining percent, clamped", () => {
    expect(windowRemainingPercent(0)).toBe(100);
    expect(windowRemainingPercent(83)).toBe(17);
    expect(windowRemainingPercent(100)).toBe(0);
    // 上游偶尔会报 >100 或负数，仍要落在 0-100 内。
    expect(windowRemainingPercent(140)).toBe(0);
    expect(windowRemainingPercent(-10)).toBe(100);
  });

  it("tones remaining percent at 20% and 10% thresholds", () => {
    expect(quotaRemainingTone(100)).toBe("neutral");
    expect(quotaRemainingTone(20.1)).toBe("neutral");
    expect(quotaRemainingTone(20)).toBe("warn");
    expect(quotaRemainingTone(10.1)).toBe("warn");
    expect(quotaRemainingTone(10)).toBe("danger");
    expect(quotaRemainingTone(0)).toBe("danger");
  });

  it("recognizes balance-style summaries only", () => {
    expect(isBalanceQuotaSummary(quota({ remainingUsd: 12.34, unit: "USD" }))).toBe(true);
    expect(isBalanceQuotaSummary(quota({ remainingUsd: null }))).toBe(false);
    expect(isBalanceQuotaSummary(quota({ unlimited: true }))).toBe(false);
    expect(isBalanceQuotaSummary(quota({ status: "unsupported" }))).toBe(false);
    expect(
      isBalanceQuotaSummary(
        quota({
          windows: [{ kind: "rolling", usagePercent: 50, resetAt: null, limitUsd: null }],
        }),
      ),
    ).toBe(false);
  });

  it("formats localized amounts with the unit label", () => {
    const t = vi.fn((key: string) => (key === "sites.quotaUnitRaw" ? "额度点数" : key)) as unknown as TFunction;
    expect(formatQuotaAmountLocalized(12.5, "USD", t)).toBe("$12.50");
    expect(formatQuotaAmountLocalized(24_035, "RAW_QUOTA", t)).toBe("24,035.00 额度点数");
    expect(formatQuotaAmountLocalized(999.69, "CNY", t)).toBe("¥999.69");
  });

  it("builds relative updated text through the translate function", () => {
    const t = vi.fn((key: string, opts?: Record<string, unknown>) => {
      if (key === "sites.quotaUpdatedJustNow") return "刚刚更新";
      if (key === "sites.quotaMinutesAgo") return `${opts?.count} 分钟前`;
      if (key === "sites.quotaUpdated") return `更新于 ${opts?.time}`;
      return key;
    }) as unknown as TFunction;
    expect(formatQuotaUpdatedText(Date.now(), t)).toBe("刚刚更新");
    expect(formatQuotaUpdatedText(Date.now() - 5 * 60_000, t)).toBe("更新于 5 分钟前");
  });
});
