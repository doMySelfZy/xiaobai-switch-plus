import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { AgentUpdateStatus, BatchUpdateResult } from '@/types/agentUpdate';

/**
 * 版本检查结果的缓存时长。
 *
 * 每次检查都要为每个 agent 起 npm 子进程再查 registry，而 `ApplyPage` 每次进页面
 * 都会调 `checkUpdates()`：没有这个窗口就是「点一下页面 = 一轮 npm」。窗口内直接复用
 * 上次结果，需要绕过时传 `{ force: true }`（手动刷新）。
 */
export const UPDATE_CHECK_TTL_MS = 30 * 60 * 1000;

export interface UpdateCheckOptions {
  /** 跳过新鲜度检查，强制重新检查。 */
  force?: boolean;
}

/** 在途去重：并发调用共用同一次检查，不再重复起子进程。 */
let inFlightCheck: Promise<void> | null = null;

interface AgentUpdateStore {
  updateStatuses: Record<string, AgentUpdateStatus>;
  checking: boolean;
  updating: boolean;
  lastCheckTime: number | null;

  // Getters
  hasAnyUpdate: () => boolean;
  updateCount: () => number;
  updatableAgents: () => AgentUpdateStatus[];
  getUpdateStatus: (kind: string) => AgentUpdateStatus | undefined;
  isUpdating: (kind: string) => boolean;

  // Actions
  checkUpdates: (options?: UpdateCheckOptions) => Promise<void>;
  updateAgent: (kind: string) => Promise<string | undefined>;
  batchUpdate: (kinds: string[]) => Promise<BatchUpdateResult>;
  updateAll: () => Promise<BatchUpdateResult>;
  clearUpdateStatus: (kind: string) => void;
  reset: () => void;
}

export const useAgentUpdateStore = create<AgentUpdateStore>((set, get) => ({
  updateStatuses: {},
  checking: false,
  updating: false,
  lastCheckTime: null,

  hasAnyUpdate: () => {
    return Object.values(get().updateStatuses).some((s) => s.hasUpdate);
  },

  updateCount: () => {
    return Object.values(get().updateStatuses).filter((s) => s.hasUpdate).length;
  },

  updatableAgents: () => {
    return Object.values(get().updateStatuses).filter((s) => s.hasUpdate);
  },

  getUpdateStatus: (kind: string) => {
    return get().updateStatuses[kind];
  },

  isUpdating: () => {
    return get().updating;
  },

  checkUpdates: async (options?: UpdateCheckOptions) => {
    if (inFlightCheck) return inFlightCheck;

    const lastCheckTime = get().lastCheckTime;
    const fresh =
      lastCheckTime !== null && Date.now() - lastCheckTime < UPDATE_CHECK_TTL_MS;
    if (fresh && !options?.force) return;

    set({ checking: true });
    const run = (async () => {
      try {
        const statuses = await invoke<AgentUpdateStatus[]>('check_agent_updates');
        const statusMap: Record<string, AgentUpdateStatus> = {};
        statuses.forEach((status) => {
          statusMap[status.kind] = status;
        });
        set({ updateStatuses: statusMap, lastCheckTime: Date.now() });
      } catch (error) {
        console.error('Failed to check agent updates:', error);
      }
    })();
    inFlightCheck = run;
    try {
      await run;
    } finally {
      if (inFlightCheck === run) inFlightCheck = null;
      set({ checking: false });
    }
  },

  updateAgent: async (kind: string) => {
    set({ updating: true });
    try {
      const newVersion = await invoke<string>('update_agent', { kind });

      // Update local status
      const statuses = { ...get().updateStatuses };
      if (statuses[kind]) {
        statuses[kind] = {
          ...statuses[kind],
          currentVersion: newVersion,
          hasUpdate: false,
          lastCheckAt: Date.now(),
        };
        set({ updateStatuses: statuses });
      }

      return newVersion;
    } catch (error) {
      console.error(`Failed to update agent ${kind}:`, error);
      throw error;
    } finally {
      set({ updating: false });
    }
  },

  batchUpdate: async (kinds: string[]) => {
    set({ updating: true });
    try {
      const result = await invoke<BatchUpdateResult>('batch_update_agents', { kinds });

      // Update local statuses for successful updates
      const statuses = { ...get().updateStatuses };
      result.successes.forEach((kind) => {
        if (statuses[kind]) {
          statuses[kind] = {
            ...statuses[kind],
            hasUpdate: false,
            lastCheckAt: Date.now(),
          };
        }
      });
      set({ updateStatuses: statuses });

      return result;
    } catch (error) {
      console.error('Failed to batch update agents:', error);
      throw error;
    } finally {
      set({ updating: false });
    }
  },

  updateAll: async () => {
    const ids = get().updatableAgents().map((s) => s.kind);
    return get().batchUpdate(ids);
  },

  clearUpdateStatus: (kind: string) => {
    const statuses = { ...get().updateStatuses };
    const filtered = Object.fromEntries(Object.entries(statuses).filter(([k]) => k !== kind));
    set({ updateStatuses: filtered });
  },

  reset: () => {
    set({
      updateStatuses: {},
      checking: false,
      updating: false,
      lastCheckTime: null,
    });
  },
}));
