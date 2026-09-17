# 添加站点列表全局刷新功能与可用性指示器

## Goal

站点列表顶部添加刷新按钮，支持批量刷新所有站点模型和额度，并根据模型获取成功与否显示绿点/红点可用性状态，让用户能够快速验证所有站点的连通性。

## Background

**Current State:**
- 站点列表在 `src/pages/SitesPage.tsx` (lines 457-467) 中，标题栏有「上游站点」和「添加站点」按钮
- 绿点指示器 (`StatusDot` component) 当前只表示站点的启用/禁用状态 (`site.enabled`)，不表示可用性
- 刷新功能只存在于右侧详情面板中：
  - 模型刷新按钮 (RefreshCw icon, `handleFetchModels`, lines 531-540)
  - 额度刷新 (`handleRefreshQuota`, lines 224-234)
- 后端命令：
  - `fetch_site_models`: 获取站点模型列表
  - `probe_site_quota`: 探测额度与可用性
  - `list_sites`: 列出所有站点

**Problem:**
- 用户无法从列表视图快速验证所有站点的连通性
- 必须逐个选中站点才能在详情面板中看到可用性状态
- 绿点只表示「已启用」而非「可用」

## Requirements

### R1: 全局刷新按钮
- 在站点列表标题栏（「上游站点」右侧、「添加站点」左侧）添加刷新按钮
- 图标使用 `RefreshCw` (lucide-react, 与详情面板刷新按钮一致)
- 按钮尺寸 `size="small"`, `type="text"`
- 点击后批量刷新所有站点的模型和额度

### R2: 批量刷新行为
- 刷新按钮点击后，对每个站点并行执行（`Promise.all()`，无并发限制）：
  - 调用 `fetch_site_models(site.id)` 获取模型列表
- **不刷新额度**：额度已有选中站点的自动刷新（每 30 秒）+ 详情面板的手动刷新
- 失败重试：每个站点请求失败时自动重试 1-2 次
- 刷新期间按钮显示 loading 状态 (旋转动画)
- 刷新完成后显示成功消息（如「已刷新 N 个站点」）
- 如果部分站点失败，显示警告消息（如「N 个站点可用，M 个站点不可用」）

### R3: 可用性状态指示器
- `StatusDot` 组件需要支持三种状态：
  1. **绿点 (pulsing)**: 站点已启用 且 模型获取成功
  2. **红点 (static)**: 站点已启用 但 模型获取失败（视为不可用）
  3. **灰点 (static)**: 站点已禁用
- 判定逻辑：
  - `site.enabled === false` → 灰点
  - `site.enabled === true && hasModels` → 绿点
  - `site.enabled === true && !hasModels` → 红点
- Tooltip 显示状态：「可用」/「不可用」/「已禁用」

### R4: 状态持久化策略
- 可用性状态基于内存状态 (`modelsBySite`)，不添加数据库字段
- 冷启动时：所有站点显示灰点（无模型数据），用户需点击刷新按钮
- 运行时：根据 `modelsBySite` 是否有数据判断可用性

## Out of Scope

- 单个站点的独立刷新按钮（已存在于详情面板，不重复添加）
- 自动定时刷新所有站点（现有只自动刷新选中站点的额度）
- 全局刷新按钮刷新额度（额度已有选中站点自动刷新 + 详情面板手动刷新）
- 刷新进度条或逐站点进度提示
- 在列表项中显示详细的错误信息（保持简洁的红点指示即可）

## Acceptance Criteria

- [ ] 站点列表标题栏中有刷新按钮，位于「上游站点」和「添加站点」之间
- [ ] 点击刷新按钮后，所有站点的模型列表被批量刷新（不刷新额度）
- [ ] 刷新期间按钮显示 loading 状态
- [ ] 刷新完成后显示消息提示（成功站点数 / 失败站点数）
- [ ] 已启用且模型获取成功的站点显示绿色 pulsing dot
- [ ] 已启用但模型获取失败的站点显示红色 static dot
- [ ] 已禁用的站点显示灰色 static dot
- [ ] Tooltip 文案对应状态：「可用」/「不可用」/「已禁用」
- [ ] 页面刷新后可用性状态保持（基于 `modelsBySite` 缓存）
- [ ] 失败的站点请求自动重试 1-2 次

## Key Decisions

### D1: 并发控制 ✓
**Decision:** 不限制并发数，使用 `Promise.all()` 并行刷新所有站点
**Rationale:** 不同站点是独立的 API endpoint，不会互相触发 rate limiting；本地网络带宽对于轻量级模型列表请求足够

### D2: 失败重试策略 ✓
**Decision:** 自动重试 1-2 次
**Rationale:** 对临时网络抖动有容错性，避免误报红点；用户体验更好

### D3: 数据库持久化 ✓
**Decision:** 不添加数据库持久化，仅依赖内存状态 (`modelsBySite`)
**Rationale:** 实现简单，无需 schema migration；冷启动时用户点击刷新按钮即可恢复状态

### D4: 刷新范围 ✓
**Decision:** 全局刷新按钮只刷新模型列表，不刷新额度
**Rationale:** 
- 额度已有选中站点的自动刷新（每 30 秒）+ 详情面板的手动刷新
- 可用性判断主要依赖模型列表（能获取模型 = 站点可用）
- 减少不必要的请求

## Notes

- 复用现有的 `handleFetchModels` 和 `handleRefreshQuota` 逻辑
- `StatusDot` 组件需要扩展支持 `status="error"` 状态（红点）
- i18n 需要添加键：`sites.refreshAll`, `sites.available`, `sites.unavailable`, `sites.refreshSuccess`, `sites.refreshPartial`
