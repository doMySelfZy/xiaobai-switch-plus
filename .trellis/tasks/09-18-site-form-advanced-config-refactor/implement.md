# Implement — 站点表单高级配置重构

## 阶段 A：纯布局（R1–R3，低风险）
1. **i18n 增删**（zh-CN + en-US，sites 段）
   - 新增 `optional`（可选配置 / Optional settings）。
   - `protocolHint` 去「下方」措辞。
   - 暂不动请求头键（阶段 B 再加）。
2. **SiteFormModal：协议+测试上移**
   - 把协议 `Form.Item` + 测试连接行整块移到 API Key 之后、`Collapse` 之前。
   - `notes` 留在折叠内。
3. **SiteFormModal：改名 + 去 Divider**
   - Collapse label → `t("sites.optional")`；删 3 条 `Divider`；余额/请求头组改小标题文本。
   - 保留 `advancedOpen` 智能展开逻辑不动。
4. **SiteFormModal：余额压一行**
   - 令牌 + 用户 ID 并排（flex wrap），测试按钮下移一行。
5. **测试**：`SiteFormModal.test.tsx` 的 `startProtocolTest` 去点展开；「默认收起」用例文案改「可选配置」。
6. **验证 A**：`pnpm typecheck` + `pnpm test:run`（SiteFormModal 相关）→ 通过后 **commit A**。

## 阶段 B：请求头键值对（R4，独立 commit）
7. **ProxyHeaderEditor 抽校验**：新增 `validateProxyHeaders(rows)`；`parseProxyHeadersJson` 内部改调它，保留导出与单测。
8. **ProxyHeaderEditor 重写**：受控行编辑器 `{value: ProxyHeader[]; onChange; error}`，行=名称+值+删除，底部「+ 添加请求头」，保留占位符提示。
9. **SiteFormModal 状态改造**：`proxyHeadersJson:string` → `proxyHeaders:ProxyHeader[]` + `enabledProxyHeaders:boolean`；反填用数组、创建态重置、提交走 `enabled?validate:[]`、错误经 `proxyHeadersInvalid` 阻断。
10. **i18n 补键**：`enableProxyHeaders`、`addProxyHeader`、`proxyHeaderName`、`proxyHeaderValue`；`proxyHeaders` 标签改「本地代理请求头」。
11. **测试**：更新/新增请求头行编辑器保存与校验断言；确认 `ProxyHeaderEditor.test.ts` 仍过。
12. **验证 B**：`pnpm typecheck` + `pnpm test:run` + `pnpm build` → **commit B**。

## 验证命令
- `pnpm typecheck`
- `pnpm test:run`（关注 `SiteFormModal.test.tsx`、`ProxyHeaderEditor.test.ts`）
- `pnpm build`
- 手动：dev 服务器打开添加/编辑站点，核对协议位置、可选配置展开、余额一行、请求头键值对与校验、Codex 私有能力未变。

## 回滚点
- 阶段 A、B 各一 commit；B 出问题可单独 revert 回到 A 的纯布局状态。
- 关键不可破坏项（改动前后 diff 自查）：`handleTestProtocol` 回填 `protocol`、令牌三态（`:422-425`）、提交字段名、Codex 私有能力区。

## 风险
- R4 string→数组连锁（反填/提交/校验/测试）；务必保证 `get_site_proxy_headers` 返回类型与 `ProxyHeader[]` 对齐（`src/types/proxy.ts:7-11`）。
- 测试对 collapse 文案强依赖，改名后逐条核对。
