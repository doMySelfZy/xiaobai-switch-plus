# 更新日志 / Changelog

本文件记录面向用户的重要变更。每次发布的逐条提交记录由 CI（git-cliff）自动生成并附在 [GitHub Releases](https://github.com/doMySelfZy/xiaobai-switch-plus/releases) 上；本文件保留人工整理的版本要点。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [v0.1.6] - 2026-09-22

### 🚀 新功能

- **mcp**：MCP 统一管理改版——单列表 + 三页签 + 常驻应用面板；纳管关联到所有已存在客户端，纳管即接管、删除可清干净不再回流未纳管
- **sites**：新增站点服务商预置模板，支持魔搭 / 魔粒余额探测
- **sites**：站点列表新增全局刷新按钮、三态可用性指示器与独立刷新指示器；额度缺失原因改用摘要短标签
- **sites**：「去应用」按钮改为固定的 Agent 入口，并完善可用性指示

### 🐛 Bug 修复

- **sites**：修复测试按钮空值静默失败、备注清空不生效、全局刷新时刷新指示器显示，以及 available 状态下无限额度 / 额度未知的兜底
- **mcp**：纳管与同步改按内容身份判重，杜绝同一服务器存两行
- **i18n**：补全三处目标清单遗漏的 Prime 文案；自定义站点预设去掉「OpenAI 兼容」后缀
- **backup**：适配 `finalize_backup_dir` 新签名

### ⚡ 性能提升

- **ui**：全部弹窗遮罩去掉 `backdrop-filter`；隐藏页面不再后台探测，长列表页改细粒度订阅
- **backend**：写配置命令移出 UI 线程，缩短锁范围与超时
- **net**：探测并行化、子进程超时、检查结果可缓存
- **sites**：配额轮询延长到 30s 并只轮询选中站点
- **mcp**：MCP discovery 并发限制到 3
- **models**：模型探测超时从 10s 降到 5s
- **apply**：备份剪枝批量处理，避免重复扫描

### 🔨 重构

- **sites**：站点表单高级配置重构，请求头改键值对编辑器
- **quota**：余额刷新改为逐站命令并保留失败尝试记录

### 💥 移除

- **targets**：移除 ZCode 应用目标，存量与落盘痕迹一次性收口
- **floating**：彻底删除悬浮窗功能

**完整提交记录**：<https://github.com/doMySelfZy/xiaobai-switch-plus/compare/v0.1.5...v0.1.6>

---

早于 v0.1.6 的版本记录见 [GitHub Releases](https://github.com/doMySelfZy/xiaobai-switch-plus/releases)。
