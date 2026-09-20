# 实现计划：站点列表全局刷新独立指示器

## 实现步骤

### 1. 状态管理（`src/store/siteStore.ts`）

**新增状态字段：**
```ts
refreshingSiteIds: Set<string>
```

**新增 actions：**
```ts
setRefreshingSiteIds: (ids: string[]) => void
clearRefreshingSiteId: (id: string) => void
clearAllRefreshingStatus: () => void
```

**实现要点：**
- `setRefreshingSiteIds` 覆盖整个集合（用于全局刷新启动）
- `clearRefreshingSiteId` 从集合中移除单个 ID
- `clearAllRefreshingStatus` 清空集合（可选，集合自然清空时不需要显式调用）
- 使用 `Set` 而不是数组，方便增删查

### 2. 刷新逻辑注入（`src/store/siteStore.ts`）

**修改 `refreshAllSites`：**
```ts
// 开始前
const allIds = get().sites.map(s => s.id);
get().setRefreshingSiteIds(allIds);

// 每个站点刷新的 finally 块
.finally(() => {
  get().clearRefreshingSiteId(site.id);
});
```

**修改 `refreshSiteModelsAndQuota`：**
```ts
// 开始前
get().setRefreshingSiteIds([siteId]);

// finally 块
.finally(() => {
  get().clearRefreshingSiteId(siteId);
});
```

### 3. UI 组件（`src/components/SiteList.tsx`）

**在站点行布局中添加刷新指示器：**
```tsx
<div className="flex items-center gap-3">
  {/* 刷新指示器 */}
  {refreshingSiteIds.has(site.id) && (
    <RefreshCw
      size={16}
      className="text-primary animate-spin"
      style={{
        animation: 'spin 1s linear infinite'
      }}
    />
  )}
  
  {/* 开关 */}
  <Switch ... />
</div>
```

**CSS 动画（已有 Tailwind `animate-spin`，可直接使用）：**
- 如需自定义淡入淡出：在组件上添加 `transition-opacity duration-200`

### 4. 跨窗口同步（已过时，已按实际架构修正）

> 原计划里的 `syncStoreToFloating` 不存在，且 `refreshingSiteIds` 不进 persist。
> 实际口径：`refreshingSiteIds` 是内存 `string[]`（主窗口 zustand 状态），
> 跨窗口不直接同步该集合——悬浮窗只读后端余额缓存：
> 统一刷新 / 单刷的余额腿走 `refresh_site_quota`（写后端缓存），完成后 emit
> `sites-refresh-finished`，悬浮窗重读 `get_all_sites_quota`。因此无需 persist
> 序列化，也无需自定义 `storage`（`Set` 方案已废弃，改用 `string[]` + `includes`）。

### 5. 测试检查点

**手动测试：**
1. 点击全局刷新，所有站点行同时出现转圈图标
2. 观察每个站点刷新完成后图标消失
3. 进入站点详情页，点击刷新按钮，该站点行出现转圈图标
4. 打开悬浮窗，触发刷新，主窗口与悬浮窗指示器同步
5. 模拟刷新失败（断网或后端返回错误），确认指示器消失

**回归检查：**
- 站点列表布局无变化（头像、名称、开关位置不变）
- 拖拽排序功能正常
- 全局刷新按钮自身转圈逻辑不受影响

### 6. 文件清单

**需要修改的文件：**
- `src/store/siteStore.ts`（状态 + actions + 刷新逻辑注入）
- `src/components/SiteList.tsx`（UI 渲染）
- 可能需要：`src/lib/sync.ts`（跨窗口同步，如果当前未同步 store）

**需要验证的文件：**
- `src/pages/Sites.tsx`（站点详情页刷新按钮调用链路）
- `src/components/FloatingWindow.tsx`（悬浮窗站点列表）

## 实现顺序

1. ✅ 先实现状态管理（siteStore）
2. ✅ 注入刷新逻辑（全局 + 单个）
3. ✅ UI 渲染刷新指示器
4. ✅ 跨窗口同步调试
5. ✅ 完整测试与回归检查

## 潜在问题与解决方案

**问题 1：Set 序列化**
- Zustand persist 不支持 `Set`，需要自定义 `storage`
- 或者改用 `string[]` 数组，用 `includes` 判断

**问题 2：刷新失败后指示器卡住**
- 确保所有异步调用都有 `finally` 块
- 在 `finally` 中无条件调用 `clearRefreshingSiteId`

**问题 3：跨窗口同步延迟**
- 如果悬浮窗指示器反应慢，检查 `syncStoreToFloating` 是否在每次状态变更时触发
- 可能需要在 `setRefreshingSiteIds` / `clearRefreshingSiteId` 中手动调用同步

**推荐方案：**
- 使用 `string[]` 而不是 `Set<string>`，简化序列化
- 在 store 中提供 `isRefreshing(siteId)` getter：`state.refreshingSiteIds.includes(siteId)`
