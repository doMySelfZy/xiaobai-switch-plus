# Design — 站点表单高级配置重构

## 边界与原则
- 仅改前端 `SiteFormModal.tsx` / `ProxyHeaderEditor.tsx` / i18n / 测试。**后端命令、载荷字段名、加密存储一律不动。**
- 提交给 `createSite`/`updateSite` 的字段集合与语义保持逐字节等价（尤其 `proxyHeaders: ProxyHeader[]`、`newapiAccessToken` 三态、`protocol`）。
- Codex 私有能力区（`CodexCapabilitySwitchList`）零改动。

## R1 协议+测试连接上移
- 把 `SiteFormModal.tsx:537-598` 中「连接协议 Form.Item + 测试连接行（含 ProtocolTestProgress/取消/结果 Text）」整块，从 `Collapse` children 移到基础区：放在 API Key `Form.Item` 之后、`Collapse` 之前。
- `notes` 备注留在「可选配置」内（低频）。
- `handleTestProtocol` / `cancelProtocolTest` / 状态与副作用原样保留（回填 `protocol`、取消只丢结果）。
- `protocolHint` 文案中性化（不再说「下方」）。

## R2 改名 + 去 Divider
- Collapse `label` 由 `t("sites.advanced")` → `t("sites.optional")`；key 仍 `"advanced"`（内部标识，避免牵动 `advancedOpen` 逻辑与 `toActiveKeys`）。
- 删除 children 内 3 条 `Divider`（`:532-536/599-603/646-650`）。余额组、请求头组改为普通块 + 小标题文本（非 Divider）。
- 默认展开逻辑 `advancedOpen` 初始化（`:165-183/233-241`）保留：默认收起，编辑已配置令牌/请求头或 anthropic/notes/forceAdvancedOpen 时展开。

## R3 余额压一行
- 令牌 + 用户 ID 放进一个 `Row`/flex（`gap`，窄容器 wrap），测试按钮移到其下一行。
- 三态令牌语义（`:422-425`）原样保留。

## R4 请求头键值对（最大改动，独立子步）
### 数据形态
- `SiteFormModal`：`proxyHeadersJson: string` → `proxyHeaders: ProxyHeader[]` + `enabledProxyHeaders: boolean`。
  - 反填（`:186-194`）：`get_site_proxy_headers` 返回数组 → `setProxyHeaders(list)`；`enabledProxyHeaders = list.length>0`。
  - 提交（`:414-419/440/459`）：`enabledProxyHeaders ? validate(proxyHeaders) : []`；错误经 `sites.proxyHeadersInvalid` 透出并阻断保存。
  - 创建态重置：`proxyHeaders=[]`、`enabled=false`。
### 编辑器组件
- `ProxyHeaderEditor` 重写为受控行编辑器：props `{ value: ProxyHeader[]; onChange(next: ProxyHeader[]); error: string|null }`；每行 `名称`+`值`+删除，底部「+ 添加请求头」；保留 `proxyHeadersPlaceholders` 占位符提示。
- 启用开关放在 `SiteFormModal` 内（`Checkbox`/`Switch`「启用自定义请求头」），关闭时隐藏编辑器且提交 `[]`。
### 校验复用
- 从 `parseProxyHeadersJson` 抽出 `validateProxyHeaders(rows: ProxyHeader[]): { headers?: ProxyHeader[]; error?: string }`（沿用 `HEADER_NAME_RE`/`CONTROL_CHAR_RE`/`PROTECTED_NAMES`/大小写重名）。
- **保留** `parseProxyHeadersJson`（内部调用 `validateProxyHeaders`），使 `ProxyHeaderEditor.test.ts` 纯函数单测不破。
- 行编辑器对每行做即时轻校验（可选），最终阻断以提交时 `validateProxyHeaders` 为准。

## 测试影响
- `SiteFormModal.test.tsx`：`startProtocolTest`（`:42-49`）去掉点「高级配置」展开；「collapsed by default」用例（`:314-327`）文案改「可选配置」；协议/测试断言按基础区位置调整；新增 1 条请求头行编辑器保存断言（可选）。
- `ProxyHeaderEditor.test.ts`：保留 `parseProxyHeadersJson` 即不动；如加 `validateProxyHeaders` 直测可另补。

## 兼容 / 回滚
- 存量站点：后端 `get_site_proxy_headers` 本就返回结构化数组，前端不再 stringify，读写兼容。
- R1–R3（纯布局）与 R4（数据形态）分两次 commit，便于单独回滚 R4。
