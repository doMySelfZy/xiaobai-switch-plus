## Goal

在本机（Windows）构建安装包并验证安装启动可用。

## Confirmed Facts

- Tauri 2 + React 19 + antd 6，`package.json:8-11` 构建链为 `tsc --noEmit && vite build`，前端产物 `dist/`，`src-tauri/tauri.conf.json:7-12` 指定 `beforeBuildCommand: pnpm build`。
- 产品名 `XiaoBaiSwitch Plus`，二进制 `XiaoBaiSwitchPlus`，版本 `0.1.6`（`tauri.conf.json:3-5`，`Cargo.toml:3`，`package.json:4`）。
- `tauri.conf.json:48-72` 打包 `targets: all`，`createUpdaterArtifacts: true`，Windows WebView2 为 `embedBootstrapper/silent`，macOS `signingIdentity: -`。
- 更新只指向自有 Releases（`tauri.conf.json:86-88`），与安装包验证无直接关系。

## Requirements

- R1：在本机产出 Windows NSIS 安装产物（`--bundles nsis`，仅 `.exe`）。
- R2：安装前备份既有安装信息与 `~/.xiaobai-switch` 数据，再覆盖安装。
- R3：安装后启动主窗口并进入站点列表。
- R4：记录产物路径、版本（0.1.6）、验证结论。

## Acceptance Criteria

- [ ] NSIS `.exe` 构建成功，路径明确记录。
- [ ] 既有安装与 `~/.xiaobai-switch` 已备份，再执行覆盖安装。
- [ ] 安装后主窗口可启动并进入站点列表。
- [ ] 产物路径、版本、验证结论已记录。

## Out of Scope

- 自动更新签名全链路（`latest.json`/签名验证）、MSI、跨平台（macOS/Linux）打包、上架发布。

## Decisions

- D1：仅验 Windows 安装包，不连带 updater 产物（用户已确认）。
- D2：仅验 NSIS .exe，不验 MSI（用户已确认）。
- D3：先备份再覆盖安装（用户已确认）。

## Risks / Deferred

- 覆盖安装会替换本机现有版本，靠事前备份兜底；回滚即重装备份前版本并恢复数据目录。
