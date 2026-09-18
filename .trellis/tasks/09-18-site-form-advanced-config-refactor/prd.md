# 重构站点表单高级配置为「可选配置」并优化布局

## Goal

按已确认的原型（`prototypes/site-form-advanced-config.html` 右侧），重构「添加/编辑站点」对话框的高级配置区，让新用户一眼看到协议自动检测、降低「高级」心理门槛、并把手填 JSON 的请求头改成键值对编辑器。**只动布局与控件形态，字段、校验语义、后端命令、提交载荷全部不变。**

## Background（当前实现，来自代码调研）

当前 `SiteFormModal.tsx` 高级配置是一个 `Collapse`（label「高级配置」），内部用 3 条 `Divider` 分「站点信息 / 余额查询 / 本地代理请求头」三组：
- 连接协议 `Select` + 测试连接按钮 + 进度/结果文案 + 备注，全埋在折叠里（`SiteFormModal.tsx:537-598`）。
- 余额查询：NewAPI 令牌、用户 ID 各占整行 + 测试按钮（`:604-645`）。
- 请求头：`ProxyHeaderEditor` 是 **JSON textarea**，靠 `parseProxyHeadersJson` 解析（`ProxyHeaderEditor.tsx:38-132`）。

痛点：协议/测试连接藏太深、Divider 三层嵌套、"高级配置"命名劝退、手填 JSON 易错。

**Codex 私有能力**（`:664-677`，`CodexCapabilitySwitchList`，滑动开关）经确认**不在本次改动范围**，保持原样。

## Requirements

### R1 — 连接协议 + 测试连接提到基础区
- 把「连接协议」`Select` 与「测试连接」按钮（含 `ProtocolTestProgress`、取消等待、结果 `Text`）从折叠区移到**基础信息区**，紧跟 API Key 之后。
- 协议 `Select` 选项文案不变；`protocolHint` 去掉「下方」这类布局耦合措辞，改中性。
- **必须保留** `handleTestProtocol` 成功后 `form.setFieldValue("protocol", detected)` 的回填副作用（`:318`）与取消=只丢结果、后端仍跑完的语义（`:295-297`）。

### R2 — 「高级配置」改名「可选配置」并去内部 Divider
- Collapse label：`sites.advanced`「高级配置」→ 新键 `sites.optional`「可选配置」。
- 移除组内 3 条 `Divider`（站点信息/余额/请求头三组标题），内容扁平成一组；`groupSiteInfo` 键随协议/备注移出后不再使用。
- **保留** 现有智能默认展开逻辑：`shouldOpenAdvanced` + 编辑态 `newapiConfigured || proxyHeaderCount>0 || forceAdvancedOpen`（`:165-183`）——默认仍收起，除非有值。

### R3 — 余额查询压成一行
- NewAPI 访问令牌 + 用户 ID 并排一行（窄容器可换行），测试按钮紧跟其下。
- **必须保留** 编辑态令牌三态语义：未填=不改 / 空串=清除 / 解密失败时 `undefined`=保护（`:422-425`）；`newapiTokenSavedHint` 等条件文案不变。

### R4 — 请求头 JSON → 键值对 + 启用开关
- 新增「启用自定义请求头」`Switch`/`Checkbox` 控制是否展开编辑器；关闭时提交 `proxyHeaders: []`。
- 编辑器改为**行式键值对**：每行 `名称` + `值` + 删除，底部「+ 添加请求头」。
- **复用 `parseProxyHeadersJson` 的校验规则**（RFC9110 名称、控制字符、`PROTECTED_NAMES` 受保护头、大小写重名），错误仍经 `sites.proxyHeadersInvalid` 透出；保留 `{{...}}` 占位符提示（`proxyHeadersPlaceholders`）。
- `SiteFormModal` 侧状态由 `proxyHeadersJson: string` 改为结构化 `ProxyHeader[]`；反填直接用 `get_site_proxy_headers` 返回的数组（不再 `JSON.stringify`）；提交走同一套校验产出 `ProxyHeader[]`。

### R5 — i18n 与测试同步
- zh/en 新增：`sites.optional`、`sites.enableProxyHeaders`、`sites.addProxyHeader`、`sites.proxyHeaderName`、`sites.proxyHeaderValue` 等；`proxyHeaders` 标签由「请求头配置（JSON）」改为「本地代理请求头」。
- `protocolHint` 文案中性化（去「下方」）。
- 更新 `SiteFormModal.test.tsx`：`startProtocolTest` helper 不再点「高级配置」展开（协议已在基础区）；「默认收起高级配置」用例改为断言「可选配置」；协议/测试相关断言按新位置调整。`ProxyHeaderEditor.test.ts` 若保留 `parseProxyHeadersJson` 则不动。

## Acceptance Criteria

- [ ] 打开「添加站点」，未展开任何折叠即可看到「连接协议」+「测试连接」；点测试连接能正常检测并回填协议、显示进度/结果/取消。
- [ ] 折叠区标题为「可选配置」；其内无 Divider 分组标题；默认收起，编辑已配置令牌/请求头的站点时自动展开。
- [ ] 余额查询令牌 + 用户 ID 在一行内；编辑态不填令牌保存后令牌不被清空（三态保护）。
- [ ] 请求头为「启用开关 + 键值对行」；填非法头名/受保护头/重名时报错且阻止保存；关闭开关保存后 `proxyHeaders` 为空。
- [ ] 编辑已有 JSON 请求头的站点，能正确反填为键值对行并可继续编辑保存。
- [ ] Codex 私有能力区完全不变（仍是滑动开关）。
- [ ] `pnpm typecheck` 通过；`pnpm test:run` 相关用例通过（不含既有无关失败）。

## Out of Scope
- Codex 私有能力区任何改动。
- 后端 Rust 命令、载荷字段名、加密/存储逻辑改动。
- 站点表单整体分步向导化（原型未采纳）。

## Technical Notes / Risks
- 最大风险点：R4 请求头状态形态 string→数组 的连锁改动（反填、提交、校验、测试）。可拆成独立子步：先交付 R1–R3（纯布局，低风险），R4 随后。
- `ProxyHeaderEditor` 的 `parseProxyHeadersJson` 是纯函数 + 有独立单测，重构时**保留该函数**（行编辑器内部复用其校验），避免丢规则。
- 测试对「点标题展开 collapse」强依赖，R2 改名后断言文案要同步。

## Open Questions
- 无（原型已确认方向）。R4 是否拆成第二次提交，在实现阶段决定。
