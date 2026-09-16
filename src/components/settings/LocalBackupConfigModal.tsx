import { useEffect, useState } from "react";
import { App, Form, Input, InputNumber, Modal, Typography } from "antd";
import { FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "@/stores";

const { Text } = Typography;

interface LocalBackupConfigModalProps {
  open: boolean;
  directory: string;
  onCancel: () => void;
  onOpenDirectory: () => void;
}

export function LocalBackupConfigModal({
  open,
  directory,
  onCancel,
  onOpenDirectory,
}: LocalBackupConfigModalProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const settings = useSettingsStore((state) => state.settings);
  const saveSettings = useSettingsStore((state) => state.saveSettings);
  const [form] = Form.useForm<{ maxBackupCopies: number }>();
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) form.setFieldsValue({ maxBackupCopies: settings.maxBackupCopies });
  }, [form, open, settings.maxBackupCopies]);

  const handleSave = async () => {
    setSaving(true);
    try {
      const values = await form.validateFields();
      await saveSettings({ maxBackupCopies: values.maxBackupCopies });
      message.success(t("settings.saved"));
      onCancel();
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={t("settings.backupCenter.localSettings")}
      centered
      destroyOnHidden
      mask={{ enabled: true }}
      width={520}
      okText={t("settings.save")}
      cancelText={t("common.cancel")}
      confirmLoading={saving}
      onOk={() => void handleSave()}
      onCancel={onCancel}
    >
      <Form form={form} layout="vertical">
        <Form.Item
          name="maxBackupCopies"
          label={t("settings.maxBackupCopies")}
          extra={
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t("settings.maxBackupCopiesHint")}
            </Text>
          }
          rules={[{ required: true }]}
        >
          <InputNumber
            min={1}
            max={200}
            precision={0}
            addonAfter={
              <span style={{ whiteSpace: "nowrap" }}>{t("settings.maxBackupCopiesUnit")}</span>
            }
            style={{ width: 200 }}
          />
        </Form.Item>
        <Form.Item label={t("settings.backupCenter.directory")}>
          <Input
            readOnly
            value={directory}
            addonAfter={
              <FolderOpen
                size={14}
                style={{ cursor: directory ? "pointer" : "not-allowed" }}
                onClick={() => {
                  if (directory) onOpenDirectory();
                }}
              />
            }
          />
        </Form.Item>
      </Form>
    </Modal>
  );
}
