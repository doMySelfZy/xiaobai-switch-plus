import { useState } from "react";
import { App, Button, Dropdown, Modal, Spin, theme } from "antd";
import { Check, ChevronDown, CircleAlert, Settings2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Site } from "@/types/domain";
import { useSiteStore } from "@/stores";
import { isAppError } from "@/lib/invoke";
import { activeApiKey, siteApiKeys } from "@/lib/siteApiKey";
import { SiteApiKeyManageModal } from "./SiteApiKeyManageModal";

interface Props {
  site: Site;
}

export function SiteApiKeySwitcher({ site }: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();
  const switchApiKey = useSiteStore((s) => s.switchApiKey);
  const probeQuota = useSiteStore((s) => s.probeQuota);
  const fetching = useSiteStore((s) => s.fetchingModelsByKey);
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [pendingKeyId, setPendingKeyId] = useState<string | null>(null);
  const keys = siteApiKeys(site);
  const active = activeApiKey(site);
  const loading = Boolean(active && fetching[`${site.id}:${active.id}`]);

  const applySwitch = async (apiKeyId: string, syncTargets: boolean) => {
    try {
      const result = await switchApiKey(site.id, apiKeyId, { syncTargets });
      if (!result.fetch.ok) {
        message.warning(t("sites.keySwitchFetchFailed"));
      } else if (!syncTargets) {
        message.success(t("sites.keySwitchSkipped"));
      } else {
        const failed = result.results.filter((r) => !r.ok);
        if (failed.length > 0) message.warning(t("sites.keySwitchPartial"));
        else if (result.results.length > 0) message.success(t("sites.keySwitchSuccess"));
        else message.success(t("sites.keySwitchSiteOnly"));
      }
      void probeQuota(site.id, { force: true }).catch(() => null);
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
    }
  };

  const handleSelect = (apiKeyId: string) => {
    if (apiKeyId === active?.id) {
      setOpen(false);
      return;
    }
    setOpen(false);
    setPendingKeyId(apiKeyId);
  };

  const closeConfirm = () => setPendingKeyId(null);

  const confirmSwitch = (syncTargets: boolean) => {
    if (!pendingKeyId) return;
    const apiKeyId = pendingKeyId;
    setPendingKeyId(null);
    void applySwitch(apiKeyId, syncTargets);
  };

  const panel = (
    <div
      className="min-w-[220px] overflow-hidden rounded-lg py-1"
      style={{
        background: token.colorBgElevated,
        boxShadow: token.boxShadowSecondary,
        border: `1px solid ${token.colorBorderSecondary}`,
        maxWidth: 360,
        ["--route-option-hover" as string]: token.colorFillTertiary,
        ["--route-option-active" as string]: token.colorPrimaryBg,
        ["--route-option-active-hover" as string]: token.colorPrimaryBgHover,
      }}
    >
      {keys.map((key) => (
        <button
          key={key.id}
          type="button"
          className="route-option flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-left text-xs"
          data-active={key.isActive ? "true" : "false"}
          style={{ color: token.colorText }}
          onClick={() => handleSelect(key.id)}
        >
          <span className="inline-flex w-3.5 shrink-0" style={{ color: token.colorPrimary }}>
            {key.isActive ? <Check size={12} /> : null}
          </span>
          <span className="min-w-0 flex-1 truncate">{key.label}</span>
          <span className="shrink-0 font-mono opacity-60">{key.keyPrefix}</span>
        </button>
      ))}
    </div>
  );

  return (
    <>
      <div className="flex min-w-0 items-center gap-2">
        <Dropdown
          open={open}
          onOpenChange={setOpen}
          trigger={["click"]}
          popupRender={() => panel}
          destroyOnHidden
        >
          <button
            type="button"
            className="inline-flex max-w-full min-w-0 cursor-pointer items-center gap-1 text-left"
            aria-label={t("sites.switchKey")}
            style={{ color: token.colorText }}
            onMouseEnter={() => setHover(true)}
            onMouseLeave={() => setHover(false)}
          >
            <span
              className="min-w-0 truncate"
              style={{
                borderBottom: `1px dashed ${hover || open ? token.colorPrimary : token.colorTextSecondary}`,
                paddingBottom: 1,
              }}
            >
              {active ? `${active.label} · ${active.keyPrefix}` : site.keyPrefix || "—"}
            </span>
            {loading ? (
              <Spin size="small" />
            ) : (
              <ChevronDown size={14} className="shrink-0" style={{ color: token.colorTextTertiary }} />
            )}
          </button>
        </Dropdown>
        <Button
          type="link"
          size="small"
          className="shrink-0 px-0"
          icon={<Settings2 size={12} />}
          onClick={() => setManageOpen(true)}
        >
          {t("sites.manageKeys")}
        </Button>
      </div>
      <SiteApiKeyManageModal open={manageOpen} site={site} onClose={() => setManageOpen(false)} />
      <Modal
        open={Boolean(pendingKeyId)}
        centered
        closable
        destroyOnHidden
        mask={{ enabled: true }}
        width={560}
        title={
          <span className="inline-flex items-center gap-2">
            <CircleAlert size={18} style={{ color: token.colorWarning }} />
            {t("sites.keySwitchTitle")}
          </span>
        }
        onCancel={closeConfirm}
        footer={[
          <Button key="skip" onClick={() => confirmSwitch(false)}>
            {t("sites.keySwitchSkip")}
          </Button>,
          <Button key="sync" type="primary" onClick={() => confirmSwitch(true)}>
            {t("sites.keySwitchSync")}
          </Button>,
        ]}
      >
        {t("sites.keySwitchHint")}
      </Modal>
    </>
  );
}
