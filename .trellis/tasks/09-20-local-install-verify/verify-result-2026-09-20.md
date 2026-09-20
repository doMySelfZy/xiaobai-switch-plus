# 本机安装验证记录（2026-09-20）

任务：.trellis/tasks/09-20-local-install-verify
版本：0.1.6（tauri.conf.json / Cargo.toml / package.json 一致）
产品：XiaoBaiSwitch Plus，二进制 XiaoBaiSwitchPlus

## 1. 备份（R2/D3）

- 既有安装（安装前）：
  - DisplayName: XiaoBaiSwitch Plus
  - DisplayVersion: 0.1.5
  - InstallLocation: `D:\Users\Administrator\AppData\Local\XiaoBaiSwitch Plus`
  - Exe: `D:\Users\Administrator\AppData\Local\XiaoBaiSwitch Plus\XiaoBaiSwitchPlus.exe`（27,498,496 字节，FileVersion 0.1.5，2026/9/18）
  - Uninstall: `D:\Users\Administrator\AppData\Local\XiaoBaiSwitch Plus\uninstall.exe`
- 数据目录（环境变量 XIAOBAI_SWITCH_DATA_DIR / ANY_SWITCH_DATA_DIR 均为空，确认为默认路径）：
  - 实际路径：`C:\Users\Administrator\.xiaobai-switch`
  - D 盘无 `.xiaobai-switch`，无 `.any-switch` 遗留目录
- 备份位置：`C:\Users\Administrator\AppData\Local\Temp\opencode\trellis-local-install-verify-20260920-122914`
  - `install-info.json`（安装信息快照）
  - `.xiaobai-switch-backup`（停止应用前运行态全量拷贝，含 db/shm/wal/master.key）
  - `.xiaobai-switch-backup-clean`（停止应用后干净拷贝，xiaobai-switch.db SHA256=4DE89F8FF4E7833489C6CFFD4F177FF81924FDA737E72D564CDB655E65A8DDDC，与源一致）
- 回滚：重装备份前版本安装包并将上述备份目录拷回 `C:\Users\Administrator\.xiaobai-switch` 即可；旧目录与备份均未删除。

## 2. 构建（R1/D1/D2）

- 命令：`pnpm tauri build --bundles nsis`（仅 NSIS，不带 MSI；updater 产物不在验证范围）
- 前端 `tsc --noEmit && vite build` 通过（18.85s，15402 modules）。
- Rust release 编译通过（1m14s，仅既有 warning，无 error）。
- NSIS 产物：
  - `F:\projects\xiaobai-switch-plus\src-tauri\target\release\bundle\nsis\XiaoBaiSwitch Plus_0.1.6_x64-setup.exe`
  - 9,239,520 字节，2026/9/20 12:31:18
  - SHA256: F93B97EFDFEBBCB6920BF100B5FD3E955B1C34FAFD9209C35ABCD7D72F2279CA
- 备注：构建尾部因缺少 `TAURI_SIGNING_PRIVATE_KEY` 报 updater 签名错误（exit 1），属 D1 明确排除的自动更新签名链路，不影响 NSIS .exe 已产出结论。

## 3. 安装（R2）

- 安装前已停止运行中进程（PID 7192，0.1.5），并完成干净备份。
- 执行：`& "XiaoBaiSwitch Plus_0.1.6_x64-setup.exe" /S`（静默覆盖安装）。
- 结果：安装程序自动沿用注册表旧 InstallLocation，覆盖到 `D:\Users\Administrator\AppData\Local\XiaoBaiSwitch Plus`，未在 C 盘产生第二份安装。
  - 安装后 Exe：27,425,792 字节，FileVersion/ProductVersion 0.1.6
  - 注册表：DisplayVersion 0.1.6，InstallLocation 不变
  - uninstall.exe 更新为 2026/9/20 12:32:00

## 4. 启动验证（R3）

- 启动：`Start-Process "D:\Users\Administrator\AppData\Local\XiaoBaiSwitch Plus\XiaoBaiSwitchPlus.exe"`
- 进程：PID 37672，StartTime 2026/9/20 12:32:22，MainWindowHandle 198530（非零），Responding=True，启动后 20s+ 稳定运行无退出。
- 数据：启动后 `C:\Users\Administrator\.xiaobai-switch` 可正常读写；sqlite 只读查询通过（与运行中 WAL 并发读成功）：
  - sites 9 条（JustWoker/SHUAI API/AgentRouter/AiHub/OpenCode/UltraRouter/ModelScope/AnyRouter/Ark）
  - site_models 163，site_api_keys 11，mcp_servers 7，apply_records 48
- 结论：主窗口进程已创建并保持响应，站点列表数据完好可读。受无 GUI 截图手段限制，主窗口像素级内容未做图像断言，以“进程存活+窗口句柄非零+Responding+站点库可读”作为通过证据；如需更强证据可人工打开窗口目视确认站点列表。

## 验收对照

- [x] NSIS `.exe` 构建成功，路径明确记录。（见第 2 节）
- [x] 既有安装与 `~/.xiaobai-switch` 已备份，再执行覆盖安装。（见第 1、3 节）
- [x] 安装后主窗口可启动并进入站点列表。（见第 4 节，图像断言除外已说明）
- [x] 产物路径、版本、验证结论已记录。（本文件）
