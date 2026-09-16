# Component Guidelines

> 组件、弹窗、图标、可访问性。写新组件前照着抄。

---

## 弹窗（Modal / modal.confirm）

统一属性：`centered`、`destroyOnHidden`、`mask`、宽度 520–560。

`mask` 只开 `enabled`，**不要开 `blur`**：antd 会因此挂上 `.ant-modal-mask-blur`（整窗
`backdrop-filter: blur(4px)`），在 175% 缩放下是一次全视口模糊，收益不抵开销（2026-09-16 性能任务移除）。

```tsx
<Modal
  centered
  destroyOnHidden
  mask={{ enabled: true }}
  width={560}
  open={open}
  title={...}
  onCancel={onClose}
  onOk={() => void handleOk()}
  okText={t("common.save")}
  cancelText={t("common.cancel")}
>
  <Form form={form} layout="vertical">…</Form>
</Modal>
```

确认框用 `modal.confirm({ centered: true, ... })`，来自 `App.useApp()`。

**异步 `onOk` 必须自己吞掉异常**：antd 会把 rejection 重新抛出，结果是弹窗既不提示
也不关闭，控制台一条未处理 Promise。两个真实反例（已修）：

```tsx
// 错误：失败后弹窗卡住、无反馈
onOk: async () => { await deleteServer(id); }

// 正确：捕获并提示
onOk: async () => {
  try { await deleteServer(id); }
  catch (error) { void message.error(errorText(error)); }
}
```

## message / modal / notification

**只从 `App.useApp()` 取**。禁止 `import { message } from "antd"` 直接调用——静态调用
拿不到 `ConfigProvider` 的主题与上下文。

```tsx
const { message, modal } = App.useApp();
```

## 图标按钮与可访问性

图标按钮**必须有 `aria-label`**，否则读屏不可用、测试也定位不到：

```tsx
<Tooltip title={t("common.edit")}>
  <Button type="text" size="small" aria-label={t("common.edit")} icon={<EditOutlined />} onClick={...} />
</Tooltip>
```

## 图标

- 应用外壳 / 侧栏 / 标题栏：`lucide-react`（尺寸 14–18）
- 内容区操作按钮：`@ant-design/icons`
- 客户端品牌图标：`@lobehub/icons`（Prime 无对应图标，用 lucide `Sparkles`）

## 样式

- 布局用 Tailwind 类名（`flex h-full min-h-0 flex-col gap-4`）
- 颜色/间距用 `theme.useToken()`，**不要写死 hex**（品牌色与 Windows 关闭按钮红 `#e81123` 除外）
- 可滚动区域记得 `min-h-0`，否则 flex 子项不会收缩、滚动失效

## 错误文案提取

后端错误是 `{ code, message }` 对象而非 `Error`，统一用一个小工具提取，别到处写
`error instanceof Error ? ... : String(...)`：

```tsx
function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: string } | null)?.message;
  return message ?? String(error);
}
```

## 常见错误

- 删/改操作前不确认就执行——破坏性动作走 `modal.confirm`。
- 用 `destroyOnClose`（已弃用），应用 `destroyOnHidden`。
- 在 `Modal` 外部用 `Form.useForm()` 却在弹窗关闭后不清表单——配合 `destroyOnHidden` 即可。
