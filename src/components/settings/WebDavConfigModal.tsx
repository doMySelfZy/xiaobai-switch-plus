import { useEffect, useState } from "react";
import {
  App,
  Button,
  Checkbox,
  Divider,
  Form,
  Input,
  InputNumber,
  Modal,
  Select,
  Switch,
  Tag,
  Typography,
} from "antd";
import { useTranslation } from "react-i18next";
import { invoke, isAppError } from "@/lib/invoke";
import type {
  SaveWebDavConfigInput,
  TestWebDavConnectionInput,
  WebDavConfigView,
} from "@/types/domain";

const { Text } = Typography;

interface FormValues {
  baseUrl: string;
  username: string;
  password?: string;
  remotePath: string;
  acceptInvalidCerts: boolean;
  autoSyncEnabled: boolean;
  syncIntervalMinutes: number;
  maxRemoteBackups: number;
}

interface WebDavConfigModalProps {
  open: boolean;
  config: WebDavConfigView;
  onCancel: () => void;
  onSaved: (config: WebDavConfigView) => void | Promise<void>;
}

function valuesToConnection(values: FormValues): TestWebDavConnectionInput {
  return {
    baseUrl: values.baseUrl.trim(),
    username: values.username.trim(),
    password: values.password || null,
    remotePath: values.remotePath.trim(),
    acceptInvalidCerts: values.acceptInvalidCerts,
  };
}

export function WebDavConfigModal({
  open,
  config,
  onCancel,
  onSaved,
}: WebDavConfigModalProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm<FormValues>();
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testResult, setTestResult] = useState<"success" | "error" | null>(null);
  const acceptInvalidCerts = Form.useWatch("acceptInvalidCerts", form);

  useEffect(() => {
    if (!open) return;
    form.setFieldsValue({
      baseUrl: config.baseUrl,
      username: config.username,
      password: "",
      remotePath: config.remotePath,
      acceptInvalidCerts: config.acceptInvalidCerts,
      autoSyncEnabled: config.autoSyncEnabled,
      syncIntervalMinutes: config.syncIntervalMinutes,
      maxRemoteBackups: config.maxRemoteBackups,
    });
    setTestResult(null);
  }, [config, form, open]);

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const values = await form.validateFields([
        "baseUrl",
        "username",
        "password",
        "remotePath",
      ]);
      await invoke("test_webdav_connection", { input: valuesToConnection(values as FormValues) });
      setTestResult("success");
      message.success(t("settings.webdav.testSuccess"));
    } catch (error) {
      setTestResult("error");
      if (isAppError(error)) message.error(error.message);
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      const values = await form.validateFields();
      const input: SaveWebDavConfigInput = {
        ...valuesToConnection(values),
        autoSyncEnabled: values.autoSyncEnabled,
        syncIntervalMinutes: values.syncIntervalMinutes,
        maxRemoteBackups: values.maxRemoteBackups,
      };
      const saved = await invoke<WebDavConfigView>("save_webdav_config", { input });
      await onSaved(saved);
      message.success(t("settings.webdav.saveSuccess"));
    } catch (error) {
      if (isAppError(error)) message.error(error.message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={t("settings.webdav.configTitle")}
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
      <Text type="secondary" style={{ fontSize: 12, display: "block", marginBottom: 12 }}>
        {t("settings.webdav.securityWarningBody")}
      </Text>
      <Form<FormValues> form={form} layout="vertical">
        <Form.Item
          name="baseUrl"
          label={t("settings.webdav.baseUrl")}
          rules={[{ required: true }]}
        >
          <Input allowClear placeholder="https://dav.example.com/dav/" />
        </Form.Item>
        <div className="flex gap-4">
          <Form.Item
            name="username"
            label={t("settings.webdav.username")}
            className="flex-1"
            rules={[{ required: true }]}
          >
            <Input allowClear />
          </Form.Item>
          <Form.Item
            name="password"
            label={t("settings.webdav.password")}
            className="flex-1"
            rules={config.hasPassword ? [] : [{ required: true }]}
            extra={
              config.hasPassword ? (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t("settings.webdav.passwordSavedHint")}
                </Text>
              ) : undefined
            }
          >
            <Input.Password allowClear autoComplete="new-password" />
          </Form.Item>
        </div>
        <Form.Item
          name="remotePath"
          label={t("settings.webdav.remotePath")}
          rules={[{ required: true }]}
        >
          <Input allowClear placeholder="xiaobai-switch" />
        </Form.Item>
        <div className="mb-4 flex items-center gap-4">
          <Form.Item name="acceptInvalidCerts" valuePropName="checked" noStyle>
            <Checkbox>{t("settings.webdav.acceptInvalidCerts")}</Checkbox>
          </Form.Item>
          <Button loading={testing} onClick={() => void handleTest()}>
            {t("settings.webdav.test")}
          </Button>
          {testResult && (
            <Tag color={testResult === "success" ? "success" : "error"}>
              {t(
                testResult === "success"
                  ? "settings.webdav.testSuccess"
                  : "settings.webdav.testFailed",
              )}
            </Tag>
          )}
        </div>
        {acceptInvalidCerts && (
          <Text type="danger" style={{ fontSize: 12, display: "block", marginBottom: 12 }}>
            {t("settings.webdav.invalidCertWarning")}
          </Text>
        )}
        <Divider />
        <Form.Item
          name="autoSyncEnabled"
          label={t("settings.webdav.autoSync")}
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>
        <div className="flex gap-4">
          <Form.Item
            name="syncIntervalMinutes"
            label={t("settings.webdav.interval")}
            className="flex-1"
            extra={t("settings.webdav.intervalExtra")}
          >
            <Select
              style={{ width: 200 }}
              options={[15, 30, 60, 120, 360, 720, 1440].map((value) => ({
                value,
                label: t("settings.webdav.intervalMinutes", { count: value }),
              }))}
            />
          </Form.Item>
          <Form.Item
            name="maxRemoteBackups"
            label={t("settings.webdav.retention")}
            className="flex-1"
            extra={t("settings.webdav.retentionExtra")}
          >
            <InputNumber
              min={1}
              max={100}
              precision={0}
              style={{ width: 160 }}
              addonAfter={t("settings.webdav.perDevice")}
            />
          </Form.Item>
        </div>
      </Form>
    </Modal>
  );
}
