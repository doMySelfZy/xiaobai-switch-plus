import { useEffect, useState } from "react";
import { Button } from "antd";
import ClaudeCode from "@lobehub/icons/es/ClaudeCode";
import Codex from "@lobehub/icons/es/Codex";
import Pi from "@lobehub/icons/es/Pi";
import { SquareTerminal, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import { usePageVisible } from "@/hooks/usePageVisible";
import type { ApplyTargetTab } from "@/stores";

const CYCLE_MS = 3000;
const FADE_MS = 180;
const TABS: ApplyTargetTab[] = ["claude_code", "codex", "pi", "prime", "zcode"];

const TAB_LABEL_KEYS: Record<ApplyTargetTab, string> = {
  claude_code: "sites.goApplyClaude",
  codex: "sites.goApplyCodex",
  pi: "sites.goApplyPi",
  prime: "sites.goApplyPrime",
  zcode: "sites.goApplyZCode",
};

const TAB_ICONS: Record<ApplyTargetTab, React.ReactNode> = {
  claude_code: <ClaudeCode size={14} />,
  codex: <Codex size={14} />,
  pi: <Pi size={14} />,
  prime: <Sparkles size={14} />,
  zcode: <SquareTerminal size={14} />,
};

function nextTab(current: ApplyTargetTab): ApplyTargetTab {
  return TABS[(TABS.indexOf(current) + 1) % TABS.length];
}

interface Props {
  disabled?: boolean;
  onApply: (tab: ApplyTargetTab) => void;
}

export function GoApplyButton({ disabled, onApply }: Props) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<ApplyTargetTab>("claude_code");
  const [leaving, setLeaving] = useState(false);
  /**
   * SitesPage 被 KeepAlivePages 常驻（display:none），站点页不在前台时这个 3 秒轮换
   * 定时器仍会跑，每次 setState 都会重渲染按钮。只在本页真正可见时才转。
   */
  const visible = usePageVisible("sites");

  useEffect(() => {
    if (!visible) {
      // 隐藏时归位：否则中途隐藏会把文字停在淡出状态，切回来是空的。
      setLeaving(false);
      return;
    }
    const reduce =
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let fadeTimer: number | null = null;
    const id = window.setInterval(() => {
      if (reduce) {
        setTab(nextTab);
        return;
      }
      setLeaving(true);
      fadeTimer = window.setTimeout(() => {
        fadeTimer = null;
        setTab(nextTab);
        setLeaving(false);
      }, FADE_MS);
    }, CYCLE_MS);
    return () => {
      window.clearInterval(id);
      if (fadeTimer !== null) window.clearTimeout(fadeTimer);
    };
  }, [visible]);

  const label = t(TAB_LABEL_KEYS[tab]);
  const icon = TAB_ICONS[tab];

  return (
    <Button type="primary" size="small" disabled={disabled} onClick={() => onApply(tab)}>
      <span
        className="go-apply-swap inline-flex items-center gap-1 whitespace-nowrap"
        data-leaving={leaving ? "true" : "false"}
      >
        {icon}
        {label}
      </span>
    </Button>
  );
}
