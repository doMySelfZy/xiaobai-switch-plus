import { Button, Dropdown } from "antd";
import type { ButtonProps } from "antd";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SITE_PRESETS, sitePresetById, type SitePresetId } from "@/lib/sitePresets";

interface Props extends Pick<ButtonProps, "size" | "type" | "color" | "variant" | "block"> {
  label: string;
  onSelect: (presetId: SitePresetId) => void;
}

/**
 * 「添加站点」的模板入口。只产出预填项，创建仍走同一个 SiteFormModal，
 * 因此这里不持有任何表单状态。
 */
export function AddSitePresetButton({ label, onSelect, ...buttonProps }: Props) {
  const { t } = useTranslation();

  return (
    <Dropdown
      trigger={["click"]}
      menu={{
        items: SITE_PRESETS.map((preset) => ({ key: preset.id, label: t(preset.nameKey) })),
        onClick: ({ key }) => {
          const preset = sitePresetById(String(key));
          if (preset) onSelect(preset.id);
        },
      }}
    >
      <Button icon={<Plus size={14} />} aria-label={label} {...buttonProps}>
        {label}
      </Button>
    </Dropdown>
  );
}
