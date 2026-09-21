import { Button, Modal, Typography, theme } from "antd";
import { useTranslation } from "react-i18next";

export interface ConflictContext {
  serverName: string;
  targetLabel: string;
  /** 库里的定义（本工具），已格式化为可读 JSON 字符串。 */
  mineText: string;
  /** 客户端里现有的定义（用户手配），已格式化；密钥只含键名，不含值。 */
  theirsText: string;
}

interface ConflictModalProps {
  open: boolean;
  context: ConflictContext | null;
  /** 用客户端的定义反向入库（使两边一致）。 */
  onAdoptTheirs: () => void;
  /** 该客户端先跳过，不改动用户手配。 */
  onSkip: () => void;
  onCancel: () => void;
}

function DefinitionPane({
  title,
  text,
  accent,
}: {
  title: string;
  text: string;
  accent: string;
}) {
  return (
    <div style={{ border: `1px solid ${accent}`, borderRadius: 10, overflow: "hidden" }}>
      <div style={{ background: accent, color: "#fff", fontSize: 12, fontWeight: 600, padding: "6px 12px" }}>
        {title}
      </div>
      <pre
        style={{
          margin: 0,
          padding: 12,
          fontSize: 12,
          fontFamily: "Consolas, monospace",
          whiteSpace: "pre-wrap",
          lineHeight: 1.5,
        }}
      >
        {text}
      </pre>
    </div>
  );
}

/**
 * 同名冲突解决弹窗（R5，D10）。
 *
 * 左右对比「库里定义」与「客户端现有定义」（仅 config + 密钥键名，值永不出后端）。
 * 只有两个出口：用客户端的反向入库、或该客户端先跳过。
 * **没有「用我的覆盖」**——守 AGENTS.md「绝不用库里旧版本覆盖用户改动」红线。
 */
export function ConflictModal({
  open,
  context,
  onAdoptTheirs,
  onSkip,
  onCancel,
}: ConflictModalProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  return (
    <Modal
      open={open}
      centered
      destroyOnHidden
      mask={{ enabled: true }}
      width={640}
      title={context ? t("mcp.conflictTitle", { name: context.serverName }) : t("mcp.conflictTitleBare")}
      onCancel={onCancel}
      footer={[
        <Button key="skip" onClick={onSkip}>
          {t("mcp.conflictSkip")}
        </Button>,
        <Button key="adopt" type="primary" onClick={onAdoptTheirs}>
          {t("mcp.conflictAdoptTheirs")}
        </Button>,
      ]}
    >
      {context && (
        <div className="flex flex-col gap-3">
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("mcp.conflictDesc", { target: context.targetLabel })}
          </Typography.Text>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <DefinitionPane
              title={t("mcp.conflictMine")}
              text={context.mineText}
              accent={token.colorPrimary}
            />
            <DefinitionPane
              title={t("mcp.conflictTheirs", { target: context.targetLabel })}
              text={context.theirsText}
              accent={token.colorWarning}
            />
          </div>
        </div>
      )}
    </Modal>
  );
}
