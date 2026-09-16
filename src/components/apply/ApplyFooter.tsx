import { useState } from "react";
import { App, Button, Modal, theme } from "antd";
import { Check, History, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { isAppError } from "@/lib/invoke";
import type { TargetKind } from "@/types/domain";
import { TargetBackupList } from "./TargetBackupList";

interface Props {
  loading: boolean;
  disabled: boolean;
  target: TargetKind;
  onApply: () => void;
  onRestoreOfficial: () => void | Promise<void>;
}

export function ApplyFooter({ loading, disabled, target, onApply, onRestoreOfficial }: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { modal, message } = App.useApp();
  const [backupOpen, setBackupOpen] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const isManagedProvider = target === "pi" || target === "prime" || target === "zcode";

  const handleRestoreOfficial = () => {
    modal.confirm({
      centered: true,
      title:
        target === "zcode"
          ? t("apply.removeZCodeConfirm")
          : target === "prime"
            ? t("apply.removePrimeConfirm")
            : target === "pi"
              ? t("apply.removePiConfirm")
              : t("apply.restoreOfficialConfirm"),
      content:
        target === "claude_code"
          ? t("apply.restoreOfficialClaudeHint")
          : target === "codex"
            ? t("apply.restoreOfficialCodexHint")
            : target === "zcode"
              ? t("apply.removeZCodeHint")
              : target === "prime"
                ? t("apply.removePrimeHint")
                : t("apply.removePiHint"),
      okText: isManagedProvider ? t("apply.removePiOk") : t("apply.restoreOfficialOk"),
      cancelText: t("common.cancel"),
      okButtonProps: { danger: true, loading: restoring },
      onOk: async () => {
        setRestoring(true);
        try {
          await onRestoreOfficial();
          modal.success({
            centered: true,
            title: isManagedProvider ? t("apply.removePiSuccess") : t("apply.restoreOfficialSuccess"),
            content: (
              <div>
                <div>
                  {target === "claude_code"
                    ? t("apply.restoreOfficialClaudeOk")
                    : target === "codex"
                      ? t("apply.restoreOfficialCodexOk")
                      : target === "zcode"
                        ? t("apply.removeZCodeDone")
                        : target === "prime"
                          ? t("apply.removePrimeDone")
                          : t("apply.removePiDone")}
                </div>
                <div className="mt-2">{t("apply.restartHint")}</div>
              </div>
            ),
            okText: t("common.confirm"),
          });
        } catch (e) {
          message.error(isAppError(e) ? e.message : t("apply.restoreOfficialFailed"));
          throw e;
        } finally {
          setRestoring(false);
        }
      },
    });
  };

  return (
    <div
      data-testid="apply-footer"
      className="shrink-0 px-6 py-3"
      style={{
        borderTop: `1px solid ${token.colorBorderSecondary}`,
        backgroundColor: token.colorBgContainer,
      }}
    >
      <div className="flex items-center gap-2">
        <Button icon={<History size={14} />} onClick={() => setBackupOpen(true)}>
          {t("apply.backupRecords")}
        </Button>
        <Button
          type="primary"
          className="flex-1"
          icon={<Check size={14} />}
          loading={loading}
          disabled={disabled || restoring}
          onClick={onApply}
        >
          {loading ? t("apply.applying") : t("apply.apply")}
        </Button>
        <Button
          icon={<RotateCcw size={14} />}
          loading={restoring}
          disabled={loading}
          onClick={handleRestoreOfficial}
        >
          {isManagedProvider ? t("apply.removePi") : t("apply.restoreOfficial")}
        </Button>
      </div>
      <Modal
        open={backupOpen}
        centered
        destroyOnHidden
        mask={{ enabled: true }}
        width={560}
        title={t("apply.backupRecords")}
        footer={null}
        onCancel={() => setBackupOpen(false)}
      >
        <TargetBackupList target={target} />
      </Modal>
    </div>
  );
}
