# 添加站点预置模板（opencode-go / 魔搭）

## Background

用户诉求原话：「按剩余次数，或者能把 opencode、modelscope 这种做成添加的模板，这种模版添加不需要配置令牌+userid」。

调研后拆成两件事，价值差别很大：

- **「剩余次数」在魔搭官方口径里不存在**：ModelScope OpenAPI 规范里 Magicube 分组只有 `GET /openapi/v1/magicubes/balance`，返回 `{total_balance, available_balance, frozen_amount}`（number，带小数，`data` 不在 required 内）。这是**魔粒账户余额**，官方未提供「模型 → 魔粒」换算，也未确认旧的 `modelscope-ratelimit-requests-*` 每日次数头是否废弃。因此本任务只做「显示魔粒余额」，不冒充剩余调用次数。
- **模板能做到真正的「一把凭据」**：官方规范 `https://modelscope.cn/.well-known/openapi.json` 全库只有一种 `bearerAuth`（描述即 "ModelScope API Token"，在 `modelscope.cn/my/myaccesstoken` 生成），**API-Inference 和 magicubes 用的是同一把令牌**。所以魔搭站点不需要 newapi 那套「访问令牌 + userId」第二对凭据，只填站点自己的 API Key 就能既推理又查余额。

现状证据：

- 站点上**没有任何服务商身份字段**，全靠 Base URL 猜：前端 `src/types/domain.ts:100-130`、Rust `src-tauri/src/domain/mod.rs:1066-1093`、`sites` 表全列 `src-tauri/src/db/migrate.rs:46-63`。
- 额度探测的支路优先级写死在 `src-tauri/src/quota_probe/mod.rs:1695-1789`：`is_opencode_go_base` 命中 → 专用端点并**提前 return**（`:1704-1720`）；否则空 key → `empty_key_result()`（`:1722-1724`）；否则 newapi 凭据齐全才查 `/api/user/self`（`:1731-1744`，凭据构造见 `src-tauri/src/commands/quota.rs:74-79`）。
- `is_opencode_go_base` 是**严格 host 白名单**（`quota_probe/mod.rs:188-201`：https + `host == "opencode.ai"` + path 含相邻 `zen`/`go` 段），边界由测试钉死（`:3771-3785`）；前端在 `src/lib/browserMock.ts:672-685` 有一份**平行实现**，改判定必须同步两处。
- 「预填表单」这条通路已经存在并被深链使用：`src/hooks/useSiteDeepLink.tsx:135-139` 在缺 apiKey 时转成 `setPendingSiteForm(payload)`，`src/pages/SitesPage.tsx:89-102` 把它变成 `SiteFormInitialValues` 打开表单；该类型只有 6 个字段（`src/components/sites/SiteFormModal.tsx:101-108`）。
- 生产代码里 ModelScope 相关为**零**（全仓搜 `modelscope|magicube|魔搭` 只命中规划文档与原型 mock）。

## Goals

1. 用户在「添加站点」时可以选一个服务商模板，最少只需填**名字 + 一把 API Key**。
2. 魔搭站点能显示**魔粒余额**，并且明确标成魔粒而不是金额。
3. 模板不引入第二套凭据：命中模板的站点在表单里**不再要求** newapi 访问令牌 / userId。

## Non-Goals

- 不显示「剩余调用次数」，不把魔粒换算成次数或人民币。
- 不给站点新增服务商/provider 数据库列（见 design.md 的识别方式决策）。
- 不改 `SiteQuota` 的金额字段语义，不改刷新调度与并发上限。
- 不新建一套与深链并行的预设格式。
- 不做 opencode-go 以外的第二方「匿名公开端点」支持。

## Requirements

### R1. 模板目录与入口

- 至少提供三个条目：`自定义（OpenAI 兼容）`（等价于现状）、`OpenCode Go`、`ModelScope 魔搭`。
- 模板目录是前端常量，字段至少含：展示名、Base URL、连接协议、是否需要 newapi 凭据、额度说明文案 key。
- 入口在现有「添加站点」路径上，不新增第二个站点创建弹窗。
- 选模板后用户仍可自由改 Base URL / 协议，模板不得锁死任何字段。

### R2. 预填行为

- 模板通过既有的 `SiteFormInitialValues` 通路注入，不新增第二个表单状态源。
- 命中模板时，高级配置里的「额度」组（`SiteFormModal.tsx:612-631`）不再要求填写，且说明为什么不用填。
- 免除判断随**当前输入的 Base URL**变化，不随「用户点过哪个模板」固化：用户手填同一个官方 host 时应获得同样待遇。

### R3. 魔搭魔粒余额探测

- 新增 ModelScope host 识别与专用探测支路，与通用 billing / Sub2API 探测**隔离**，沿用 opencode-go 已有的「命中即提前 return」形状。
- 请求：`GET https://modelscope.cn/openapi/v1/magicubes/balance`，`Authorization: Bearer <站点 active API key>`；不读取也不要求 `newapi_access_token` / `newapi_user_id`。
- 必须容错：`data` 缺失、字段缺失、非数值、`success: false`，各自落到明确的 `QuotaProbeStatus`，不得把解析失败静默当成余额 0。
- 401 与网络失败的区分要与现有 `unauthorized` / `error` 语义一致。
- 余额以**点数口径**展示，单位为「魔粒」，中英文都要有；不得出现 `$` / `¥` 符号。
- `src/lib/browserMock.ts` 的平行实现同步跟上，invoke 契约与 Rust command 保持一致。

### R4. 兼容与安全

- 不改动兼容红线里的任何协议值；`capabilities` 继续只承载布尔能力位，不塞字符串。
- 魔搭的令牌是用户真实凭据：不得写日志、不得进错误文本、列表与详情只用 `keyPrefix`。
- 深链导入（`xiaobaiswitchplus://sites`）行为不变。

## Acceptance Criteria

- [ ] 用魔搭模板添加站点，全流程只填「名字 + API Key」两项即可保存并出现在列表里。
- [ ] 魔搭站点余额行显示「N 魔粒」，鼠标悬停/详情面板能看到这是魔粒余额而不是金额，也看不到任何 `$`/`¥`。
- [ ] 魔搭站点的表单里看不到「访问令牌 / userId」必填提示；填了官方 host 的自定义站点同样如此。
- [ ] 令牌无效时显示鉴权失败语义；接口返回畸形 JSON 时落到数据异常语义，不显示 0 魔粒。
- [ ] OpenCode Go 模板与现有 `is_opencode_go_base` 行为无回归（相关测试保持通过）。
- [ ] Base URL 改成第三方 host 后，免除提示消失、走标准探测链。
- [ ] `pnpm typecheck`、`pnpm test:run`、`src-tauri` 下 `cargo test` 通过（已知无关失败：`generateUpdaterManifest.test.ts`、`validateUpdaterSigningSecret.test.ts`）。
