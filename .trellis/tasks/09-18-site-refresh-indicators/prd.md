# 站点列表全局刷新独立指示器

## Goal

全局刷新时每个站点行显示独立转圈指示器，单个站点完成后指示器消失

## Background

当前全局刷新只在标题栏刷新按钮显示转圈状态，用户无法看到各个站点的刷新进度。需要在每个站点行右侧（开关左侧）显示独立的刷新指示器，让用户清楚看到哪些站点正在刷新、哪些已完成。

**原型参考：** `.trellis/prototypes/site-list-refresh-indicators.html`

## Requirements

### 功能需求

1. **全局刷新触发**
   - 点击标题栏全局刷新按钮时，所有站点行右侧同时出现刷新指示器（蓝色转圈图标）
   - 全局刷新按钮自身继续保持转圈状态

2. **单个站点刷新完成**
   - 该站点的刷新指示器淡出消失（带淡入淡出 + 缩放动画，持续 0.2s）
   - 其他正在刷新的站点指示器保持显示

3. **全局刷新完成**
   - 所有站点刷新指示器消失后，全局刷新按钮停止转圈

4. **手动刷新单个站点**
   - 用户点击站点详情页的刷新按钮时，该站点行也应显示刷新指示器
   - 刷新完成后指示器消失

### 视觉规范

- **刷新指示器**
  - 图标：Lucide `RefreshCw`，尺寸 16px
  - 颜色：`token.colorPrimary`（`#1677ff`）
  - 动画：1s 匀速无限旋转
  - 出现/消失：淡入淡出 + 缩放（0.2s ease）
  - 位置：站点行右侧，开关按钮左侧，间距 12px

- **布局约束**
  - 指示器不刷新时完全隐藏（`opacity: 0`，不占布局空间或使用条件渲染）
  - 刷新时 `opacity: 1` + `scale(1)`
  - 不影响现有拖拽手柄、头像、站点信息、开关的布局

### 状态管理

需要在 `siteStore` 中维护：

```ts
// 新增状态字段
refreshingSiteIds: Set<string>  // 正在刷新的站点 ID 集合
```

需要提供：

```ts
// 新增 action
setRefreshingSiteIds(ids: string[])  // 设置正在刷新的站点列表
clearRefreshingSiteId(id: string)    // 移除单个站点刷新状态
clearAllRefreshingStatus()           // 清空所有刷新状态
```

### 实现约束

1. **全局刷新调用链路**
   - `refreshAllSites()` 开始时，收集所有站点 ID 并调用 `setRefreshingSiteIds(allIds)`
   - 每个站点刷新完成后调用 `clearRefreshingSiteId(siteId)`
   - 所有站点完成后 `refreshingSiteIds` 自动清空

2. **单个站点刷新**
   - 站点详情页刷新按钮触发的 `refreshSiteModelsAndQuota(siteId)` 也需要：
     - 开始时 `setRefreshingSiteIds([siteId])`
     - 完成后 `clearRefreshingSiteId(siteId)`

3. **跨窗口同步**
   - `refreshingSiteIds` 需要在主窗口与悬浮窗之间同步
   - 使用现有的 `syncStoreToFloating` 机制

4. **错误处理**
   - 刷新失败时也必须调用 `clearRefreshingSiteId`，避免指示器卡住

## Acceptance Criteria

- [ ] 点击全局刷新按钮，所有站点行同时显示刷新指示器
- [ ] 每个站点刷新完成后，其指示器带动画消失
- [ ] 所有站点完成后，全局刷新按钮停止转圈
- [ ] 手动刷新单个站点时，该站点行显示刷新指示器
- [ ] 刷新指示器不影响现有布局（拖拽、头像、开关位置不变）
- [ ] 悬浮窗与主窗口的刷新指示器状态同步
- [ ] 刷新失败时指示器也能正常消失，不会卡住

## Non-Goals

- 不改动全局刷新按钮自身的转圈逻辑
- 不改动站点详情页的刷新按钮 UI（只调用状态管理）
- 不新增站点行内的单独刷新按钮（指示器是只读的进度提示）
