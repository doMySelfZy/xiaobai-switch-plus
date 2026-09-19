# Quality Guidelines

> i18n、测试、断言质量。这里的要求以「能被验证」为准。

---

## i18n（强制）

- 所有用户可见文案走 `t("...")`；技术标识符（模型 id、环境变量名、路径）除外。
- **插值必须双花括号**：`t("mcp.existingImportAll", { count })` 对应文案
  `"全部纳管（{{count}}）"`。写成 `{count}` 会把占位符原样显示在界面上。
- `zh-CN.json` 与 `en-US.json` **必须成对更新**。改完跑一次键对比：

```bash
python -c "
import json
zh=json.load(open('src/i18n/locales/zh-CN.json',encoding='utf-8'))['mcp']
en=json.load(open('src/i18n/locales/en-US.json',encoding='utf-8'))['mcp']
print('zh-only:',sorted(set(zh)-set(en))); print('en-only:',sorted(set(en)-set(zh)))
"
```

- 别留「写了但没人用」的 key——它们会让下一个人以为某处已接入 i18n。删功能时**连带它的
  文案一起删**（含键名里看不出来、只有正文提到的那种，例如列举目标清单的提示句）。
  判断死键的办法：`grep -rn '"<ns>\.<key>"' src/` 零命中即死。
- **删 key 要防重复块**：`zh-CN.json` / `en-US.json` 顶层的 `rules` 与 `proxy` 各有两份
  内容相同的块（`JSON.parse` 后者覆盖前者）。只删一处等于没删。用「改前改后唯一键路径集合
  做差」的脚本核对，别只靠肉眼数行。

## 测试命令

| 范围 | 命令 |
|------|------|
| 前端全量 | `pnpm test:run` |
| 类型检查 | `pnpm typecheck` |
| 单个文件 | `npx vitest run src/pages/McpPage.test.tsx` |
| Rust | `cd src-tauri && cargo test` |

本机已知的 2 个既有失败：`generateUpdaterManifest.test.ts` 与
`validateUpdaterSigningSecret.test.ts`（shebang 解析问题，与本仓库改动无关）。
报告结果时要说明它们，不要把红说成绿。

## 断言质量：必须是「空断言」的对立面

写完后自问：**把实现改坏，这个用例会失败吗？** 不会失败就是空断言。

推荐做法——写完关键测试后做一次**变异验证**：临时把实现改坏（删掉分支、反转条件），
确认对应用例真的红。本项目对以下行为都做过变异验证：

- MCP 托管条目清理（去掉接管后客户端里会同时出现两条同名 MCP）
- 扫描不泄漏密钥值（改为保留 env 后用例立即失败并打印泄漏内容）
- 悬浮窗定时刷新与卸载清理（去掉 setInterval 后用例失败）
- 跨窗口设置变更订阅（取消订阅后用例失败）
- ZCode 目标退役：未知目标绑定被跳过（改回 `unwrap_or(ClaudeCode)` 立即变红）、
  清洗幂等（把备份挪到写盘之后立即变红）

### 用例写法要点

- 断言要**限定范围**：`within(row).getByText(...)`，否则页面别处出现同文案就多匹配。
- 按钮名用容忍空白的正则：antd 会在两个汉字间插空格，`/保\s*存/` 而不是 `"保存"`。
- 图标按钮查不到名 → 先给它加 `aria-label`（既是可访问性修复，也让测试可写）。
- 中文文案断言用中文（测试默认 `zh-CN`），别用英文 key 猜显示内容。

## 提交前

- `pnpm typecheck` + 相关测试跑过，再声称完成；未跑完不得说「已完成」。
- 别声称验证过没验证的东西——UI 观感、内存占用这类要真机看的，如实标注为待确认。
