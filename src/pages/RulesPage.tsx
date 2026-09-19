import { useCallback, useEffect, useState } from "react";
import { Alert, Button, Checkbox, Input, Space, Tag, Typography, theme } from "antd";
import { CheckCircleFilled, CloseCircleFilled, InfoCircleOutlined, SaveOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "@/components/settings/SettingsGroup";
import { useRulesStore } from "@/stores";
import type { AgentRulesApplyResult, AgentRulesTargetResult } from "@/types/rules";
import type { TargetKind } from "@/types/domain";

const TARGETS: TargetKind[] = ["claude_code", "codex", "pi", "prime"];

const TARGET_LABEL_KEYS: Record<TargetKind, string> = {
  claude_code: "rules.targetClaudeCode",
  codex: "rules.targetCodex",
  pi: "rules.targetPi",
  prime: "rules.targetPrime",
};

/** invoke 抛出的错误对象形状（与 McpPage 的用法一致）。 */
function errorText(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

export function RulesPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  // 按字段订阅：任一字段更新不再整页重渲染（与 Proxy/MCP/Skills 页保持一致）。
  const body = useRulesStore((s) => s.body);
  const targets = useRulesStore((s) => s.targets);
  const loading = useRulesStore((s) => s.loading);
  const paths = useRulesStore((s) => s.paths);
  const load = useRulesStore((s) => s.load);
  const loadPaths = useRulesStore((s) => s.loadPaths);
  const save = useRulesStore((s) => s.save);

  const [draft, setDraft] = useState("");
  const [selected, setSelected] = useState<TargetKind[]>([]);
  const [saving, setSaving] = useState(false);
  const [result, setResult] = useState<AgentRulesApplyResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void load();
    void loadPaths();
  }, [load, loadPaths]);

  // 载入（或换数据源）后把编辑区填成库里那份；保存中也保持用户正在敲的内容。
  useEffect(() => {
    setDraft(body);
    setSelected(targets);
  }, [body, targets]);

  const targetLabel = useCallback(
    (target: TargetKind) => t(TARGET_LABEL_KEYS[target] ?? target),
    [t],
  );

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const applied = await save(draft, selected);
      setResult(applied);
      await loadPaths();
    } catch (err) {
      setResult(null);
      setError(errorText(err));
    } finally {
      setSaving(false);
    }
  };

  const renderResult = (item: AgentRulesTargetResult) => (
    <Space key={item.target} size={6} style={{ display: "flex" }}>
      {item.ok ? (
        <CheckCircleFilled style={{ color: token.colorSuccess }} />
      ) : (
        <CloseCircleFilled style={{ color: token.colorError }} />
      )}
      <Typography.Text style={{ fontSize: 12 }}>
        {targetLabel(item.target)}
        {" · "}
        {item.ok
          ? item.changed
            ? t("rules.resultWritten")
            : t("rules.resultUnchanged")
          : t("rules.resultFailed", { message: item.message })}
      </Typography.Text>
    </Space>
  );

  const shadowed = paths.find((item) => item.target === "codex")?.shadowedBy;
  const nothingToApply = draft.trim().length === 0 || selected.length === 0;

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-6">
      <div style={{ maxWidth: 880, width: "100%" }}>
        <Typography.Title level={4} style={{ marginTop: 0, marginBottom: 4 }}>
          {t("rules.title")}
        </Typography.Title>
        <Typography.Text type="secondary" style={{ fontSize: 13 }}>
          {t("rules.subtitle")}
        </Typography.Text>

        <div style={{ marginTop: 16 }}>
          <SettingsGroup
            title={t("rules.contentTitle")}
            extra={
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {t("rules.contentHint")}
              </Typography.Text>
            }
          >
            <Input.TextArea
              aria-label={t("rules.bodyLabel")}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder={t("rules.placeholder")}
              spellCheck={false}
              autoSize={{ minRows: 12, maxRows: 28 }}
              style={{
                fontFamily:
                  "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace",
                fontSize: 13,
                lineHeight: 1.6,
              }}
            />
          </SettingsGroup>

          <SettingsGroup title={t("rules.targetsTitle")}>
            <Checkbox.Group
              value={selected}
              onChange={(values) => setSelected(values as TargetKind[])}
            >
              <Space orientation="vertical" size={6}>
                {TARGETS.map((target) => (
                  <Checkbox key={target} value={target}>
                    {targetLabel(target)}
                  </Checkbox>
                ))}
              </Space>
            </Checkbox.Group>
            <div style={{ marginTop: 12 }}>
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {nothingToApply ? t("rules.emptyMeansClear") : t("rules.saveAppliesImmediately")}
              </Typography.Text>
            </div>
          </SettingsGroup>

          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <Button
              type="primary"
              icon={<SaveOutlined />}
              loading={saving || loading}
              onClick={() => void handleSave()}
            >
              {t("rules.save")}
            </Button>
            {result && (
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {t("rules.appliedAt", {
                  time: new Date(result.appliedAt).toLocaleTimeString(),
                })}
              </Typography.Text>
            )}
          </div>

          {error && (
            <Alert
              type="error"
              showIcon
              style={{ marginTop: 12 }}
              title={t("rules.saveFailed")}
              description={error}
            />
          )}

          {result && result.results.length > 0 && (
            <Alert
              type={result.results.every((item) => item.ok) ? "success" : "warning"}
              showIcon
              style={{ marginTop: 12 }}
              title={t("rules.resultTitle")}
              description={
                <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                  {result.results.map(renderResult)}
                </div>
              }
            />
          )}

          <SettingsGroup title={t("rules.pathsTitle")} style={{ marginTop: 20 }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              {paths.map((item) => (
                <div key={item.target} style={{ display: "flex", gap: 8, alignItems: "center" }}>
                  <Tag style={{ marginInlineEnd: 0, minWidth: 96, textAlign: "center" }}>
                    {targetLabel(item.target)}
                  </Tag>
                  <Typography.Text code style={{ fontSize: 12, wordBreak: "break-all" }}>
                    {item.path}
                  </Typography.Text>
                  <Typography.Text
                    type="secondary"
                    style={{ fontSize: 12, whiteSpace: "nowrap" }}
                  >
                    {item.exists ? t("rules.pathExists") : t("rules.pathMissing")}
                  </Typography.Text>
                </div>
              ))}
            </div>

            {shadowed && (
              <Alert
                type="warning"
                showIcon
                style={{ marginTop: 10 }}
                title={t("rules.codexShadowTitle")}
                description={t("rules.codexShadowDesc", { path: shadowed })}
              />
            )}

            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <InfoCircleOutlined
                style={{ color: token.colorTextTertiary, marginTop: 3 }}
              />
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {t("rules.plaintextNotice")}
              </Typography.Text>
            </div>
          </SettingsGroup>
        </div>
      </div>
    </div>
  );
}
