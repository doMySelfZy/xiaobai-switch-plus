import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetBrowserMock, seedWebDavMock } from "@/lib/browserMock";
import * as invokeMod from "@/lib/invoke";
import { useUIStore } from "@/stores";
import { BackupQuickPopover } from "./BackupQuickPopover";
import "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

describe("BackupQuickPopover", () => {
  beforeEach(() => {
    resetBrowserMock();
    useUIStore.setState({ activePage: "sites", settingsTab: "general" });
  });

  afterEach(() => {
    resetBrowserMock();
    vi.restoreAllMocks();
  });

  it("creates local snapshots and routes unconfigured WebDAV users to backup settings", async () => {
    render(
      <Wrapper>
        <BackupQuickPopover />
      </Wrapper>,
    );

    fireEvent.click(screen.getByRole("button", { name: "应用数据备份" }));
    expect(await screen.findByText("数据包含解密主密钥，请仅使用可信的 WebDAV 账户。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /立即同步/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /恢复云端历史数据包/ })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /创建本地快照/ }));
    expect(await screen.findByText("本地应用备份已创建")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /前往备份设置配置 WebDAV/ }));
    await waitFor(() => {
      expect(useUIStore.getState().activePage).toBe("settings");
      expect(useUIStore.getState().settingsTab).toBe("backup");
    });
  });

  it("localizes the fingerprint algorithm mismatch instead of showing the backend text", async () => {
    // 快捷备份气泡也是用户实际会点到的同步入口：算法不兼容同样必须走 i18n，
    // 否则这里会漏出面向诊断的英文原文（AC9 说的是"设置界面"，但用户看到的是哪里就修哪里）。
    seedWebDavMock({ baseUrl: "https://dav.example.com/", username: "alice", hasPassword: true }, []);
    const realInvoke = invokeMod.invoke;
    vi.spyOn(invokeMod, "invoke").mockImplementation(async (cmd, args) => {
      if (cmd === "sync_now") {
        throw {
          code: "sync_algorithm_mismatch",
          message:
            "remote fingerprint algorithm 2 does not match local 1; upgrade the other device",
        };
      }
      return realInvoke(cmd, args);
    });

    render(
      <Wrapper>
        <BackupQuickPopover />
      </Wrapper>,
    );

    fireEvent.click(screen.getByRole("button", { name: "应用数据备份" }));
    const syncButton = await screen.findByRole("button", { name: /立即同步/ });
    await waitFor(() => expect(syncButton).toBeEnabled());
    fireEvent.click(syncButton);

    expect(await screen.findByText(/同步已暂停：对端设备的数据指纹算法/)).toBeInTheDocument();
    expect(screen.queryByText(/upgrade the other device/)).toBeNull();
  });

  it("restores the newest remote snapshot from the title-bar popover", async () => {
    const latestFile = "xiaobai-switch-backup-20260829_090000.browser.aaaaaaaa.zip";
    const olderFile = "xiaobai-switch-backup-20260827_120000.browser.12345678.zip";
    seedWebDavMock(
      {
        baseUrl: "https://dav.example.com/",
        username: "alice",
        hasPassword: true,
      },
      [
        {
          fileName: latestFile,
          size: 2048,
          lastModified: "Sat, 29 Aug 2026 09:00:00 GMT",
          deviceName: "browser",
        },
        {
          fileName: olderFile,
          size: 1024,
          lastModified: "Thu, 27 Aug 2026 12:00:00 GMT",
          deviceName: "browser",
        },
      ],
    );
    const invokeSpy = vi.spyOn(invokeMod, "invoke");

    render(
      <Wrapper>
        <BackupQuickPopover />
      </Wrapper>,
    );

    fireEvent.click(screen.getByRole("button", { name: "应用数据备份" }));
    const restoreBtn = await screen.findByRole("button", { name: /恢复云端历史数据包/ });
    await waitFor(() => {
      expect(restoreBtn).toBeEnabled();
    });

    fireEvent.click(restoreBtn);
    expect((await screen.findAllByText("恢复这份应用快照？")).length).toBeGreaterThan(0);
    expect(screen.getByText(new RegExp(latestFile.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))).toBeInTheDocument();

    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: /恢\s*复/ }));
    await waitFor(() => {
      expect(invokeSpy).toHaveBeenCalledWith("restore_webdav_backup", { fileName: latestFile });
    });
  });
});
