import { useEffect, useState } from "react";
import { App, Form, Modal } from "antd";
import { useTranslation } from "react-i18next";
import type { ProbeSiteApiKeyResult, Site } from "@/types/domain";
import { useSiteStore } from "@/stores";
import { copyText } from "@/lib/copyText";
import { invoke, isAppError } from "@/lib/invoke";
import {
  ApiKeyListInput,
  loadSiteKeyDrafts,
  normalizeApiKeyDrafts,
  type ApiKeyDraft,
} from "./ApiKeyListInput";

interface Props {
  open: boolean;
  site: Site;
  onClose: () => void;
}

export function SiteApiKeyManageModal({ open, site, onClose }: Props) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const getSiteApiKey = useSiteStore((s) => s.getSiteApiKey);
  const updateSite = useSiteStore((s) => s.updateSite);
  const liveSite = useSiteStore((s) => s.sites.find((item) => item.id === site.id) ?? site);
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const [keyLoading, setKeyLoading] = useState(false);
  const [testingIndex, setTestingIndex] = useState<number | null>(null);
  const siteId = site.id;

  useEffect(() => {
    if (!open) {
      form.resetFields();
      setTestingIndex(null);
      return;
    }
    let cancelled = false;
    const current = useSiteStore.getState().sites.find((item) => item.id === siteId);
    if (!current) return;
    setKeyLoading(true);
    form.setFieldsValue({
      apiKeys: [{ id: current.activeApiKeyId ?? "", label: "", apiKey: "" }],
    });
    void loadSiteKeyDrafts(current, getSiteApiKey)
      .then((apiKeys) => {
        if (!cancelled) form.setFieldValue("apiKeys", apiKeys);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        message.error(isAppError(error) ? error.message : t("sites.apiKeyLoadFailed"));
      })
      .finally(() => {
        if (!cancelled) setKeyLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, siteId, form, getSiteApiKey, message, t]);

  const rowDraft = (index: number): ApiKeyDraft | undefined => {
    const rows = form.getFieldValue("apiKeys") as ApiKeyDraft[] | undefined;
    return rows?.[index];
  };

  const handleCopy = async (index: number) => {
    const secret = String(rowDraft(index)?.apiKey ?? "").trim();
    if (!secret) {
      message.error(t("sites.apiKey"));
      return;
    }
    try {
      await copyText(secret);
      message.success(t("common.copied"));
    } catch (e) {
      message.error(isAppError(e) ? e.message : String(e));
    }
  };

  const handleTest = async (index: number) => {
    const secret = String(rowDraft(index)?.apiKey ?? "").trim();
    if (!secret) {
      message.error(t("sites.apiKey"));
      return;
    }
    setTestingIndex(index);
    try {
      const result = await invoke<ProbeSiteApiKeyResult>("probe_site_api_key", {
        siteId: liveSite.id,
        apiKey: secret,
      });
      message.success(t("sites.testKeySuccess", { count: result.modelCount }));
    } catch (e) {
      message.error(
        t("sites.testKeyFailed", { error: isAppError(e) ? e.message : String(e) }),
      );
    } finally {
      setTestingIndex(null);
    }
  };

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      const keys = normalizeApiKeyDrafts(values.apiKeys);
      if (keys.length === 0) {
        message.error(t("sites.apiKey"));
        return;
      }
      setSaving(true);
      const saved = await updateSite(liveSite.id, { apiKeys: keys });
      const drafts = await loadSiteKeyDrafts(saved, getSiteApiKey);
      form.setFieldValue("apiKeys", drafts);
      message.success(t("sites.updateSuccess"));
      // 保存完成就关闭：按钮写的是「保存」，用户点完期待这个弹窗收起来。
      // 还要继续改可以重新打开——留一个不关的窗口反而像卡住了。
      onClose();
    } catch (e) {
      if (e && typeof e === "object" && "errorFields" in e) return;
      message.error(isAppError(e) ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      centered
      destroyOnHidden
      mask={{ enabled: true }}
      width={560}
      title={t("sites.manageKeys")}
      onCancel={onClose}
      okText={t("sites.save")}
      cancelText={t("sites.cancel")}
      confirmLoading={saving}
      okButtonProps={{ disabled: keyLoading }}
      onOk={() => void handleSave()}
      styles={{
        container: {
          maxHeight: "calc(100vh - 32px)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        },
        body: {
          overflowY: "auto",
          overflowX: "hidden",
          minHeight: 0,
        },
      }}
    >
      <Form form={form} layout="vertical" requiredMark="optional">
        <Form.Item label={t("sites.apiKey")} extra={t("sites.apiKeyCreateHint")} required>
          <ApiKeyListInput
            disabled={keyLoading}
            testingIndex={testingIndex}
            onCopy={(index) => void handleCopy(index)}
            onTest={(index) => void handleTest(index)}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
}
