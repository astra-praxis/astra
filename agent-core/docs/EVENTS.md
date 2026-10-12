# agent-core 事件协议（schema_version = 2）

> 事件是 agent-core 与宿主（Tauri 前端 / CLI / WebSocket）之间的唯一通信契约。
>
> **破坏性形状变更流程**：`EVENT_SCHEMA_VERSION` + 1 → 重写快照
> （`cargo test -p agent-core write_event_snapshot -- --ignored`）→ 更新本文档 → 通知前端消费方。
>
> 契约锁定：`src/events_testdata/events_schema_v2.snapshot.json`（快照测试
> `event_envelope_json_matches_schema_v2_snapshot`，新增变体有编译期穷尽守卫 + 覆盖守卫）。
>
> **v1 → v2 变更**（PLAN 1.3）：`tool_call_completed.artifacts` 从产物引用
> `{ path, kind }` 升级为完整 Artifact `{ id, kind, location, metadata, preview }`。
> `kind` 取值从 `file_created | file_modified` 改为 7 类语义 kind（见下）。
> **v2 additive 修订**（不 bump 版本）：Envelope 增加 `event_id` / `timestamp`
>（旧载荷缺省可反序列化）；未采纳的评审项见 PLAN 决策记录。

---

## Envelope 信封

每个事件都是一条 Envelope，`type` 字段 + 载荷字段扁平合并（serde flatten）：

```json
{
  "schema_version": 2,
  "event_id": "evt_run_1730000000000_3",
  "timestamp": 1730000000123,
  "session_id": "sess-1",
  "run_id": "run_1730000000000",
  "sequence": 3,
  "type": "message_delta",
  "text": "增量文本"
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `schema_version` | u32 | 协议版本，当前 **2**；反序列化缺省按当前版本处理（旧载荷兼容） |
| `event_id` | string | 全局事件 ID（`evt_{run_id}_{sequence}`）：重放稳定（同一事件重发 id 不变）、跨 Run 唯一；重连去重 / 日志追踪 / replay 定位用 |
| `timestamp` | u64 | Runtime 侧产生时间（ms since epoch）；旧载荷缺省 0。性能分析 / Agent Trace 用，不依赖消费方时钟 |
| `session_id` | string | 会话 ID |
| `run_id` | string | Run ID（`run_<毫秒时间戳>`） |
| `sequence` | u64 | Run 内从 1 递增；顺序以此为准，**不保证 JSON 字段序跨版本稳定** |
| `type` | string | 事件类型 tag（snake_case） |

## 事件一览

### `message_delta`

正在生成的回复文本增量；前端按 sequence 顺序拼接渲染。

| 字段 | 类型 | 说明 |
|---|---|---|
| `text` | string | 文本增量 |

### `thinking_delta`

模型思考过程增量（reasoning 模型的 `reasoning_content`）；前端渲染为可折叠「思考过程」块。无 reasoning 的模型不产生此事件。

| 字段 | 类型 | 说明 |
|---|---|---|
| `text` | string | 思考文本增量 |

### `tool_call_args_delta`

工具调用参数流式增量（写大文件时前端可实时预览）。**同一 `tool_call_id` 的 fragment 按序拼接 = 完整参数 JSON**；模型层 `Delta::ToolCallStart` 会发出一帧 fragment 为空字符串的首帧。

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_call_id` | string | 模型侧调用 ID |
| `tool_name` | string | 工具名 |
| `fragment` | string | 原始 JSON 参数片段 |

### `status_changed`

Run 状态机流转。`status` ∈ `analyzing` / `executing_tool` / `awaiting_permission` / `compacting` / `finalizing`。

### `plan_updated`

任务计划更新（结构化状态机，非模型生成的 markdown）。载荷：`plan: { steps: [{ id, title, state: pending | in_progress | completed | failed }] }`。

> ⚠️ 事件已定义并锁定形状，但驱动机制在 PLAN 1.5 接线，当前 Run 循环**暂不发出**。

### `tool_call_started`

参数已校验后的工具调用开始。Confirm 类工具发出后进入 `awaiting_permission` 等待用户。

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_call_id` | string | 调用 ID |
| `tool_name` | string | 工具名 |
| `args_summary` | string | 参数摘要（每值 ≤60 字符） |
| `args` | object | 完整参数快照；字符串值 >20000 字符截断并追加标注（TextPack 可配）；`None` 时字段省略 |

### `tool_call_completed`

工具执行成功。

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_call_id` | string | 调用 ID |
| `tool_name` | string | 工具名 |
| `result_summary` | string | 结果摘要（≤200 字符） |
| `artifacts` | array | 产物 Artifact 数组（见下表）；**空数组时字段省略** |

**Artifact 结构**（schema v2）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | 进程内唯一，形如 `art_<毫秒>_<序号>`；宿主可用于引用特定产物（审查/复用） |
| `kind` | string | 语义类型：`file_change \| document \| pdf \| image \| command_output \| data \| generic` |
| `location` | string | 产物位置：文件类为相对项目根路径 |
| `metadata` | object | 按 kind 约定的元数据（见下表）；无则省略 |
| `preview` | string | 可选内联预览（如 `edit_file` 的行级 diff）；无则省略 |

**metadata 约定**（尽力而为，字段可缺省）：

| kind | metadata 键 | 当前产出方 |
|---|---|---|
| `file_change` | `mode`（create/overwrite/edit/copy/move）、`bytes`、`insertions`、`deletions`、`from` | write_file / edit_file / copy_path / move_path |
| `document` | `format`（markdown/docx/...）、`bytes` | extract_document / build_docx |
| `pdf` | `pages`、`size` | 暂无（预留） |
| `command_output` | `command`、`exit_code` | 暂无（预留） |

> `diff` 只是 `file_change` 产物 `preview` 字段的内容形式，不是独立 kind。

### `tool_call_failed`

工具执行失败。错误字符串**同时回传模型**（模型可自恢复，Run 不中断）。

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_call_id` / `tool_name` | string | 调用 ID / 工具名（未注册工具时 tool_name 为模型传入名） |
| `error` | string | 错误文案（走 TextPack） |

### `permission_requested`

静态策略判定为 Confirm 时发出；前端展示确认卡，决定经 PermissionBridge 通道回传。

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_call_id` / `tool_name` / `summary` | string | 待确认的调用信息；批准绑定具体 `tool_call_id`，**不扩大范围** |

### `permission_decided`

权限已决定；前端据此更新确认卡状态。`decision` ∈ `allow` / `confirm` / `deny`。

### `run_completed` / `run_failed` / `run_cancelled`

Run 终态三事件（一次 Run 恰好一个）。

| 事件 | 载荷 |
|---|---|
| `run_completed` | `final_text: string` 最终回复全文 |
| `run_failed` | `error: string` 失败原因（含 max_turns 耗尽 / 空闲超时 / 模型错误，文案走 TextPack） |
| `run_cancelled` | 无载荷；取消优先级最高（权限等待 / 流消费 / 工具执行中均立即生效） |

### `compaction_completed`

上下文自动压缩完成：历史末尾已追加摘要消息（Role::Compaction），后续请求按压缩点裁剪视图；压缩失败自动降级为继续原上下文（不重试）。

## 序列化约定

1. tag 一律 snake_case；空 `Vec` / `None` 字段**省略**（`skip_serializing_if`）。
2. 新增字段默认 `#[serde(default)]`，保证旧载荷可反序列化；改字段类型/删除字段 = 破坏性变更。
3. 快照样本即文档样本：`src/events_testdata/events_schema_v2.snapshot.json` 与本文档保持同步。
