# Type Safety

> 前后端类型契约。跨语言边界的错误不会在编译期暴露，只能靠约定。

---

## 命令契约

`invoke` 用泛型标注返回类型，参数对象用后端要求的 camelCase 键名：

```tsx
const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
const result = await invoke<McpSaveResult>("save_mcp_server", { input });
await invoke("set_floating_window_collapsed", { collapsed: next });
```

Rust 侧一律 `#[serde(rename_all = "camelCase")]`，前端类型逐字对应。
**字段名写错不会报错**（serde 忽略未知字段），表现为「传了但没生效」——
改这类字段时前后端一起改。

## 类型放哪

- `src/types/domain.ts`：与后端 `domain/mod.rs` 对应的核心模型
- `src/types/<域>.ts`：按域拆分（如 `types/mcp.ts`）
- 类型只在测试里用时也放同一处，不要就地定义

## 少用 as 断言

`as` 会绕过检查，字段改名后不会报错。优先：

- **类型守卫**：`isAppError(e): e is AppError`
- **可选链 + 兜底**：`settings.floatingWindow?.autoRefreshMinutes ?? 5`
- 确实无法收窄时才用 `as`，并写一句为什么

## 联合类型

领域枚举（目标客户端、MCP 类型等）用字符串联合而不是裸 `string`，并在切换分支用
穷尽映射表，新增取值时能一眼看出哪里要补：

```tsx
const TARGET_LABEL_KEYS: Record<TargetKind, string> = {
  claude_code: "mcp.targetClaudeCode",
  codex: "mcp.targetCodex",
  pi: "mcp.targetPi",
  prime: "mcp.targetPrime",
};
```

映射**函数**（返回 i18n key 那种）不要写默认兜底臂。兜底臂在联合收缩后会变成
「未知值冒充某个合法值」——用户会把一个目标的数据看成另一个目标的。用穷尽守卫：

```tsx
export function targetKindLabelKey(kind: TargetKind): "apply.targetClaude" | … | "apply.targetPrime" {
  if (kind === "claude_code") return "apply.targetClaude";
  // …四个都返回
  // 加新目标却忘加分支 → 这里编译失败；运行时真收到不认识的名字 → 原样回显裸 token
  const unreachable: never = kind;
  return unreachable;
}
```

读取处配 `?? target` 兜底（`t(TARGET_LABEL_KEYS[target] ?? target)`），这样后端多回一个
本版本不认识的目标时显示裸 token，而不是 `t(undefined)`。

## 收缩联合时编译器照不到的三类点

删掉一个枚举成员后，下面三类**不会报错**，必须人工核对（ZCode 退役实测）：

1. **`export` 的死代码**：`noUnusedLocals` 不管 `export`。面板删了，配套的
   `hydrateXxxForm` / `XXX_OPTIONS` / 解析函数会安静地留着。
2. **可选字段**：`site.xxxApiType?: …` 没人读也没人写，类型与序列化都不报错。
3. **i18n 键**：`t("apply.resultZCodeOk")` 这种字符串引用编译器看不见；键删了引用还在
   （或反过来留一堆死键）。`src/i18n/locales/*.json` 顶层的 `rules` 与 `proxy` 各有
   **两份重复块**（内容相同、后者覆盖前者），删键要在四个块里都删一次才真消失。

## 可选字段与「缺省即兼容」

后端新增字段一律带 `#[serde(default)]`，前端类型标可选并在读取处给默认值——
老数据库/老备份反序列化时不会炸。

## 与 Rust 的对应关系

| Rust | TypeScript |
|------|-----------|
| `Option<T>` | `T \| null`（serde 输出 `null`）或 `T \| undefined`（字段可能不存在） |
| `Vec<T>` | `T[]` |
| `i64`（时间戳毫秒） | `number` |
| `serde_json::Value` | `Record<string, unknown>` |
| `#[serde(rename_all = "camelCase")]` | 驼峰字段名 |

## 常见错误

- 在 store 外用 `as` 把 `unknown` 直接转成具体类型——错误形状会在运行时才炸。
- 类型写了可选但代码按必填用（`settings.floatingWindow.xxx`）→ 编译期不报，
  运行时空指针。加 `?.`。
