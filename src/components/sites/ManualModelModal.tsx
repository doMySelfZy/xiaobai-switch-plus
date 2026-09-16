import { useEffect } from "react";
import { App, Form, Input, Modal } from "antd";
import { useTranslation } from "react-i18next";
import type { Site } from "@/types/domain";
import { useSiteStore } from "@/stores";
import { isAppError } from "@/lib/invoke";

interface Props {
  open: boolean;
  site: Site | null;
  onClose: () => void;
}

export function ManualModelModal({ open, site, onClose }: Props) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const setSelectedModel = useSiteStore((s) => s.setSelectedModel);
  const [form] = Form.useForm<{ modelId: string }>();

  useEffect(() => {
    if (!open) return;
    form.resetFields();
  }, [open, form]);

  const handleOk = async () => {
    if (!site) return;
    try {
      const values = await form.validateFields();
      const modelId = values.modelId.trim();
      await setSelectedModel(site.id, modelId);
      message.success(t("sites.addModelSuccess"));
      onClose();
    } catch (e) {
      if (isAppError(e)) message.error(e.message);
    }
  };

  return (
    <Modal
      open={open}
      title={t("sites.addModelTitle")}
      onCancel={onClose}
      onOk={() => void handleOk()}
      okText={t("sites.save")}
      cancelText={t("sites.cancel")}
      width={520}
      destroyOnHidden
      centered
      mask={{ enabled: true }}
    >
      <Form form={form} layout="vertical" className="mt-2" requiredMark="optional">
        <Form.Item
          name="modelId"
          label={t("sites.modelId")}
          rules={[
            { required: true, message: t("sites.modelId") },
            { whitespace: true, message: t("sites.modelId") },
          ]}
        >
          <Input placeholder={t("sites.manualModelPlaceholder")} allowClear autoFocus />
        </Form.Item>
      </Form>
    </Modal>
  );
}
