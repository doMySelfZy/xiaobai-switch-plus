import { theme } from "antd";
import { useEffect } from "react";
import { ApplySidebar } from "@/components/apply/ApplySidebar";
import { ApplyPanelSkeleton } from "@/components/apply/ApplyPanelSkeleton";
import { ClaudeApplyPanel } from "@/components/apply/ClaudeApplyPanel";
import { CodexApplyPanel } from "@/components/apply/CodexApplyPanel";
import { PiApplyPanel } from "@/components/apply/PiApplyPanel";
import { PrimeApplyPanel } from "@/components/apply/PrimeApplyPanel";
import { AgentUpdateButton } from "@/components/apply/AgentUpdateButton";
import { useDeferredTabContent } from "@/hooks/useDeferredTabContent";
import { useApplyStore, useSiteStore, useUIStore } from "@/stores";
import { useAgentUpdateStore } from "@/stores/agentUpdateStore";

/**
 * Sidebar selection commits immediately. The first visit to a target shows
 * a skeleton while the heavy form mounts off-screen; later visits reuse the
 * already-mounted panel (display:none) so Claude ↔ Codex stays instant.
 */
export function ApplyPage() {
  const { token } = theme.useToken();
  const applyTab = useUIStore((s) => s.applyTab);
  const loadSites = useSiteStore((s) => s.loadSites);
  const ensureApplyData = useApplyStore((s) => s.ensureApplyData);
  const checkUpdates = useAgentUpdateStore((s) => s.checkUpdates);
  const { mounted, showSkeleton } = useDeferredTabContent(applyTab);

  useEffect(() => {
    void loadSites({ soft: true });
    void ensureApplyData();
    void checkUpdates();
  }, [loadSites, ensureApplyData, checkUpdates]);

  return (
    <div className="flex h-full min-h-0">
      <div
        className="h-full w-56 shrink-0"
        style={{ borderRight: "1px solid var(--border-color)", backgroundColor: token.colorBgContainer }}
      >
        <ApplySidebar />
      </div>
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        style={{ backgroundColor: token.colorBgElevated }}
      >
        {showSkeleton && <ApplyPanelSkeleton />}

        {mounted.has("claude_code") && (
          <div
            className="h-full min-h-0"
            style={{
              display: applyTab === "claude_code" && !showSkeleton ? "flex" : "none",
              flexDirection: "column",
            }}
            aria-hidden={applyTab !== "claude_code"}
          >
            <AgentUpdateButton />
            <ClaudeApplyPanel />
          </div>
        )}
        {mounted.has("codex") && (
          <div
            className="h-full min-h-0"
            style={{
              display: applyTab === "codex" && !showSkeleton ? "flex" : "none",
              flexDirection: "column",
            }}
            aria-hidden={applyTab !== "codex"}
          >
            <AgentUpdateButton />
            <CodexApplyPanel />
          </div>
        )}
        {mounted.has("pi") && (
          <div
            className="h-full min-h-0"
            style={{
              display: applyTab === "pi" && !showSkeleton ? "flex" : "none",
              flexDirection: "column",
            }}
            aria-hidden={applyTab !== "pi"}
          >
            <AgentUpdateButton />
            <PiApplyPanel />
          </div>
        )}
        {mounted.has("prime") && (
          <div
            className="h-full min-h-0"
            style={{
              display: applyTab === "prime" && !showSkeleton ? "flex" : "none",
              flexDirection: "column",
            }}
            aria-hidden={applyTab !== "prime"}
          >
            <AgentUpdateButton />
            <PrimeApplyPanel />
          </div>
        )}
      </div>
    </div>
  );
}
