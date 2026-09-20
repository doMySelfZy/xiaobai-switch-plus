import type { TargetKind } from "@/types/domain";

/** 四个应用目标（固定顺序，用于展示与勾选）。 */
export const MCP_TARGETS: TargetKind[] = ["claude_code", "codex", "pi", "prime"];

export const MCP_TARGET_LABEL_KEYS: Record<TargetKind, string> = {
  claude_code: "mcp.targetClaudeCode",
  codex: "mcp.targetCodex",
  pi: "mcp.targetPi",
  prime: "mcp.targetPrime",
};
