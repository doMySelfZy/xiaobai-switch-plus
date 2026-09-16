import type { TFunction } from "i18next";
import { invoke } from "@/lib/invoke";
import type { Site, SiteQuota } from "@/types/domain";

export const QUOTA_TTL_MS = 5 * 60 * 1000;

/** 列表额度摘要的自动刷新间隔（仅页面可见时轮询，只轮询当前选中站点以降低噪音）。 */
export const SITE_QUOTA_AUTO_REFRESH_MS = 30 * 1000;

export async function probeSiteQuota(siteId: string): Promise<SiteQuota> {
  return invoke<SiteQuota>("probe_site_quota", { siteId });
}

export function quotaCacheKey(
  site: Pick<Site, "id" | "baseUrl" | "quotaRevision">,
): string {
  return `${site.id}:${site.baseUrl}:${site.quotaRevision}`;
}

export function isQuotaCacheFresh(quota: SiteQuota, now = Date.now()): boolean {
  return now - quota.fetchedAt < QUOTA_TTL_MS;
}

export function formatUsd(amount: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(amount);
}

export function normalizeQuotaUnit(unit: string | null | undefined): string {
  const raw = (unit ?? "USD").trim().toUpperCase();
  if (raw === "RMB" || raw === "CNY" || raw === "¥" || raw === "元") return "CNY";
  if (raw === "$" || raw === "USD") return "USD";
  if (raw === "QUOTA" || raw === "RAW_QUOTA") return "RAW_QUOTA";
  return raw || "USD";
}

export type QuotaUnitI18nKey = "sites.quotaUnitRaw";

export interface FormattedQuotaAmountParts {
  value: string;
  unit: string | null;
  unitI18nKey: QuotaUnitI18nKey | null;
}

function formatQuotaNumber(amount: number): string {
  return new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(amount);
}

export function formatQuotaAmountParts(
  amount: number,
  unit?: string | null,
): FormattedQuotaAmountParts {
  const normalized = normalizeQuotaUnit(unit);
  if (normalized === "CNY") {
    return {
      value: new Intl.NumberFormat("zh-CN", {
        style: "currency",
        currency: "CNY",
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      }).format(amount),
      unit: null,
      unitI18nKey: null,
    };
  }
  if (normalized === "USD") {
    return { value: formatUsd(amount), unit: null, unitI18nKey: null };
  }
  if (normalized === "RAW_QUOTA") {
    return {
      value: formatQuotaNumber(amount),
      unit: null,
      unitI18nKey: "sites.quotaUnitRaw",
    };
  }
  return {
    value: formatQuotaNumber(amount),
    unit: normalized,
    unitI18nKey: null,
  };
}

export function formatQuotaAmount(amount: number, unit?: string | null): string {
  const parts = formatQuotaAmountParts(amount, unit);
  if (parts.unit) return `${parts.value} ${parts.unit}`;
  if (parts.unitI18nKey) return `${parts.value} RAW_QUOTA`;
  return parts.value;
}

export function shouldShowExpiry(
  expiresAt: number | null,
  nowSec = Date.now() / 1000,
): boolean {
  if (expiresAt == null || !Number.isFinite(expiresAt) || expiresAt <= nowSec) {
    return false;
  }
  return new Date(expiresAt * 1000).getUTCFullYear() < 2099;
}

export function formatExpiryDate(expiresAt: number, locale: string): string {
  return new Intl.DateTimeFormat(locale === "zh-CN" ? "zh-CN" : "en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(new Date(expiresAt * 1000));
}

export function quotaRemainingPercent(quota: SiteQuota): number | null {
  if (quota.unlimited || quota.totalUsd == null || quota.totalUsd <= 0) return null;
  const remaining =
    quota.remainingUsd ??
    (quota.usedUsd != null ? quota.totalUsd - quota.usedUsd : null);
  if (remaining == null) return null;
  return Math.max(0, Math.min(100, (remaining / quota.totalUsd) * 100));
}

export type QuotaTone = "ok" | "warn" | "danger";

/**
 * 余额站点的告警色。能算出剩余百分比时走统一的 [`quotaRemainingTone`]
 * （与窗口型站点同一套阈值）；拿到总额是「无限额度」哨兵值时退化为绝对金额判断。
 */
export function quotaTone(quota: SiteQuota): QuotaTone {
  const remaining = quota.remainingUsd;
  if (remaining == null) return "ok";
  if (remaining <= 0) return "danger";
  const percent = quotaRemainingPercent(quota);
  if (percent != null) {
    const tone = quotaRemainingTone(percent);
    if (tone === "danger") return "danger";
    if (tone === "warn") return "warn";
  }
  if (remaining < 1) return "warn";
  return "ok";
}

export function emptyUnsupportedQuota(): SiteQuota {
  return {
    status: "unsupported",
    remainingUsd: null,
    usedUsd: null,
    totalUsd: null,
    unlimited: false,
    unit: null,
    expiresAt: null,
    source: null,
    endpoint: null,
    fetchedAt: Date.now(),
    latencyMs: 0,
    error: null,
  };
}

/* ---- Shared summary helpers (SiteQuotaRow detail + SiteListItem list) ---- */

const quotaWindowLabelKeys: Record<string, string> = {
  rolling: "sites.quotaWindowRolling",
  weekly: "sites.quotaWindowWeekly",
  monthly: "sites.quotaWindowMonthly",
};

export function quotaWindowLabelKey(kind: string): string | null {
  return quotaWindowLabelKeys[kind] ?? null;
}

const quotaWindowShortLabelKeys: Record<string, string> = {
  rolling: "sites.quotaWindowShortRolling",
  weekly: "sites.quotaWindowShortWeekly",
  monthly: "sites.quotaWindowShortMonthly",
};

/** 列表等窄容器用的窗口短标签 ("5h" / "周" / "月")。 */
export function quotaWindowShortLabelKey(kind: string): string | null {
  return quotaWindowShortLabelKeys[kind] ?? null;
}

export function clampQuotaPercent(percent: number): number {
  return Math.max(0, Math.min(100, percent));
}

/**
 * 上游只报「已用百分比」，界面统一展示「剩余百分比」——口径与余额型站点
 * （只显示剩余金额）一致，避免同一列表里两种相反的百分比语义。
 */
export function windowRemainingPercent(usagePercent: number): number {
  return clampQuotaPercent(100 - usagePercent);
}

export type QuotaRemainingTone = "neutral" | "warn" | "danger";

/**
 * 剩余百分比告警阈值（全应用唯一一处）：剩不到两成提醒、不到一成告警。
 * 等价于「已用达到 80% / 90%」，与列表改造前的告警时机保持一致。
 */
export function quotaRemainingTone(remainingPercent: number): QuotaRemainingTone {
  if (remainingPercent <= 10) return "danger";
  if (remainingPercent <= 20) return "warn";
  return "neutral";
}

/** Balance-style summary: available with a concrete remaining amount and no usage windows. */
export function isBalanceQuotaSummary(
  quota: SiteQuota,
): quota is SiteQuota & { remainingUsd: number } {
  return (
    quota.status === "available" &&
    (quota.windows ?? []).length === 0 &&
    !quota.unlimited &&
    quota.remainingUsd != null
  );
}

/** Formats an amount with its unit (i18n key resolved through `t`). */
export function formatQuotaAmountLocalized(
  amount: number,
  unit: string | null | undefined,
  t: TFunction,
): string {
  const parts = formatQuotaAmountParts(amount, unit);
  const unitText = parts.unitI18nKey ? t(parts.unitI18nKey) : parts.unit;
  return unitText ? `${parts.value} ${unitText}` : parts.value;
}

/** Relative "updated" text: <1 minute = just now, else N minutes ago. */
export function formatQuotaUpdatedText(fetchedAt: number, t: TFunction): string {
  const mins = Math.max(0, Math.round((Date.now() - fetchedAt) / 60_000));
  if (mins < 1) return t("sites.quotaUpdatedJustNow");
  return t("sites.quotaUpdated", { time: t("sites.quotaMinutesAgo", { count: mins }) });
}
