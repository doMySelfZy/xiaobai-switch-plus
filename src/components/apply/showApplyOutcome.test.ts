import { describe, expect, it, vi } from "vitest";
import i18n from "@/i18n";
import { applyResultBodyKey, showApplyException, showApplyOutcome } from "./showApplyOutcome";

function modalApi() {
  return {
    success: vi.fn(),
    error: vi.fn(),
  };
}

describe("showApplyOutcome", () => {
  it("maps each target to a localized success body, not the backend English string", () => {
    expect(applyResultBodyKey("claude_code")).toBe("apply.resultClaudeOk");
    expect(applyResultBodyKey("codex")).toBe("apply.resultCodexOk");
    expect(applyResultBodyKey("pi")).toBe("apply.resultPiOk");
    expect(applyResultBodyKey("prime")).toBe("apply.resultPrimeOk");

    const modal = modalApi();
    showApplyOutcome(modal, i18n.t.bind(i18n), {
      target: "claude_code",
      ok: true,
      status: "applied",
      backupPaths: [],
      message: "Claude Code settings.json updated. Restart Claude Code / terminal.",
    });

    expect(modal.success).toHaveBeenCalledTimes(1);
    const cfg = modal.success.mock.calls[0][0] as { title: string; content: unknown };
    expect(cfg.title).toBe("应用成功");
    expect(JSON.stringify(cfg.content)).toContain("已写入 Claude Code 的 settings.json。");
    expect(JSON.stringify(cfg.content)).toContain("请重启终端或重新打开对应 CLI 工具使配置生效。");
    expect(JSON.stringify(cfg.content)).not.toContain("settings.json updated");
    expect(modal.error).not.toHaveBeenCalled();
  });

  it("shows a localized error modal without the backend English payload", () => {
    const modal = modalApi();
    showApplyOutcome(modal, i18n.t.bind(i18n), {
      target: "codex",
      ok: false,
      status: "failed",
      backupPaths: [],
      message: "atomic write failed: permission denied",
    });

    expect(modal.error).toHaveBeenCalledTimes(1);
    const cfg = modal.error.mock.calls[0][0] as { title: string; content: string };
    expect(cfg.title).toBe("应用失败");
    expect(cfg.content).toBe("写入目标配置失败，请检查状态后重试。");
    expect(cfg.content).not.toContain("permission denied");
  });

  it("translates AppError codes on unexpected apply exceptions", () => {
    const modal = modalApi();
    showApplyException(modal, i18n.t.bind(i18n), {
      code: "lock_busy",
      message: "target is locked",
    });
    expect(modal.error.mock.calls[0][0]).toMatchObject({
      title: "应用失败",
      content: "目标正在被写入，请稍后重试",
    });
  });
});
