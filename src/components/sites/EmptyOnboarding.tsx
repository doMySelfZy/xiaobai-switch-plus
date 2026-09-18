import { Button, Steps, theme } from "antd";
import { useTranslation } from "react-i18next";
import { useUIStore } from "@/stores";
import { AddSitePresetButton } from "./AddSitePresetButton";
import type { SitePresetId } from "@/lib/sitePresets";
import appIconUrl from "../../../assets/brand/app-icon-1024.png?url";

interface Props {
  /** 走配置向导（等价于自定义模板）。 */
  onAdd: () => void;
  /** 按服务商模板直接打开添加表单。 */
  onAddPreset: (presetId: SitePresetId) => void;
}

export function EmptyOnboarding({ onAdd, onAddPreset }: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const setWizardOpen = useUIStore((s) => s.setWizardOpen);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-6 p-8 text-center">
      <div className="flex flex-col items-center">
        <img src={appIconUrl} alt={t("app.name")} width={88} height={88} draggable={false} />
        <h2 className="mt-3 text-xl font-semibold" style={{ color: token.colorText }}>
          {t("onboarding.welcome")}
        </h2>
        <p className="mt-2 max-w-md text-sm" style={{ color: token.colorTextSecondary }}>
          {t("onboarding.welcomeDesc")}
        </p>
      </div>
      <Steps
        direction="horizontal"
        size="small"
        className="max-w-lg"
        items={[
          { title: t("onboarding.step1") },
          { title: t("onboarding.step2") },
          { title: t("onboarding.step3") },
        ]}
      />
      <div className="flex flex-col items-center gap-2">
        <Button
          type="primary"
          size="large"
          onClick={() => {
            setWizardOpen(true);
            onAdd();
          }}
        >
          {t("sites.startWizard")}
        </Button>
        <AddSitePresetButton
          label={t("sites.addByPreset")}
          type="text"
          size="large"
          onSelect={onAddPreset}
        />
      </div>
    </div>
  );
}
