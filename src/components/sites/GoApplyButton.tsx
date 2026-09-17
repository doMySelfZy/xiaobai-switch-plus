import { Button } from "antd";
import { Rocket } from "lucide-react";
import { useTranslation } from "react-i18next";

interface GoApplyButtonProps {
  disabled?: boolean;
  onApply: () => void;
}

/**
 * 统一的「去 Agent 应用」按钮。
 * 点击后跳转到应用中心，默认选中第一个目标（claude_code）。
 */
export function GoApplyButton({ disabled, onApply }: GoApplyButtonProps) {
  const { t } = useTranslation();

  return (
    <Button
      type="primary"
      size="small"
      icon={<Rocket size={14} />}
      disabled={disabled}
      onClick={onApply}
    >
      {t("sites.goApply")}
    </Button>
  );
}
