# agent-core 测试规格（能力闭环验收）

> 版本 v1（2026-10，采纳自外部测试评审，结合项目现状做了覆盖映射）。
>
> 原则：**不测函数，测能力闭环**——验证 Agent 是否真的能完成任务闭环，
> 而非逐函数覆盖。每个能力域 = 一组闭环场景，每个场景标注现状与缺口。
>
> 状态图例：✅ 已有测试覆盖 · 🔨 部分覆盖 · ⬜ 缺口（标注优先级与归属阶段）
> 优先级：P0 = 阶段一地基（当前） · P1 = 阶段二/三 · P2 = 阶段四/五

---

## 覆盖总览

| # | 能力域 | 现状 | 主要缺口 |
|---|---|---|---|
| 1 | Runtime 生命周期（Thread/Run） | ✅ | 并行 Run 语义按设计排除（同 Session 新 Run 取消旧 Run） |
| 2 | Event System | 🔨 | 缺省字段/未知字段兼容测试、sequence 单调性显式断言（本轮补） |
| 3 | Model Adapter | ✅ | rig 真实链路走 live_net（ignored）+ e2e example，CI 外 |
| 4 | Agent Loop | ✅ | 多工具中途失败恢复已有 Thread 级测试 |
| 5 | Tool System | 🔨 | 工具 panic 隔离 ⬜（P1）；shell 超时已有（pandoc 60s 模式） |
| 6 | Permission | ✅ | 等待确认无超时（设计如此：取消经 RunCancelled） |
| 7 | Context Management | ✅ | — |
| 8 | Plan | 🔨 | upsert 纯函数已测；run_loop 驱动未接线（1.5） |
| 9 | Artifact | ✅（v2 形状） | version/history 生命周期为预留字段位，不测 |
| 10 | Host Adapter | 🔨 | src-tauri 仅 cargo check；CLI e2e 手动；WebSocket 属宿主层 |
| 11 | Error Handling | 🔨 | 模型超时/非法 JSON 聚合 ⬜（P1，openai_sse 有单点解析测试） |
| 12 | Performance | ⬜ | P2，SDK 化后再做（100 并发 Run / 大文件流式） |

---

## 1. Runtime 生命周期（Thread / Run）

> 注：评审稿的「Session 创建」对应本项目的 `Thread`（会话层）；
> 独立 Session/agent_id 抽象按 PLAN 决策推迟到多 Agent 阶段，不在此测。

| 场景 | 验证点 | 现状 |
|---|---|---|
| 会话多轮消息累积 | send 后消息序列正确、多轮顺序执行 | ✅ `thread::send_accumulates_messages_after_run_completes` / `send_twice_runs_sequential_turns` |
| Run ID 唯一且同 Run 一致 | `run_*` 格式、单 Run 内一致 | ✅ `run::envelopes_carry_session_and_run_ids` |
| 并发请求语义 | 同 Session 新 send 取消进行中 Run（取消优先） | ✅ `thread::new_send_cancels_previous_active_run`（设计上**不支持**同 Session 并行 Run，多并行属多 Thread） |
| Run 取消 | 启动前取消不调后端；流中取消发 `run_cancelled` | ✅ `cancelled_before_start_never_calls_backend` / `cancel_mid_stream_stops_and_emits_event` |

## 2. Event System（P0）

| 场景 | 验证点 | 现状 |
|---|---|---|
| Envelope 序列化/反序列化 | JSON 形状锁定、tag snake_case | ✅ `event_envelope_json_matches_schema_v2_snapshot` + 快照契约（穷尽守卫 + 覆盖守卫） |
| schema_version 缺省兼容 | 旧载荷无该字段 → 按当前版本处理 | ✅（serde default；快照含版本字段） |
| **缺省字段兼容（v2 新增字段）** | 无 `event_id`/`timestamp` 的旧载荷可反序列化 | ✅ 本轮补：`envelope_tolerates_legacy_payload_without_v2_fields` |
| **未知字段忽略** | 反序列化忽略额外字段（前向兼容契约） | ✅ 本轮补：`envelope_ignores_unknown_fields` |
| **sequence 单调性** | 从 1 开始、严格递增、不重复 | ✅ 本轮补：`sequence_starts_at_one_and_monotonically_increases` |
| event_id 重放稳定 | 同 (run_id, sequence) → 同 id；跨 sequence 不同 | ✅ `envelope_event_id_is_replay_stable` |
| Event Replay（重放） | 事件流落盘 → 重放 → 状态一致 | ⬜ P1，依赖阶段三 ThreadStore（事件持久化）后补 `tests/replay.rs` |
| 时间戳 | timestamp 为 Runtime 侧产生时间 | ✅ `envelope_carries_meta_and_event_tag`（with_meta_at 注入验证） |

## 3. Model Adapter

| 场景 | 验证点 | 现状 |
|---|---|---|
| 流式文本解析 | delta 拼接、DONE/空帧跳过 | ✅ `openai_sse::sse_text_delta_parsed` 等 |
| Thinking（reasoning） | deepseek `reasoning_content` / object 两种风格；无 reasoning 不产生 | ✅ `sse_reasoning_content_parsed_deepseek_style` / `sse_reasoning_object_style_parsed` |
| Tool Call 流式解析 | 分片拼装完整参数、多 tool call | ✅ `sse_tool_call_start_fragment` / `chunked_fragments_across_lines_assemble_to_full_call` |
| 错误载荷 | SSE error 帧 → ModelError | ✅ `sse_error_payload_is_model_error` |
| 真实服务冒烟 | rig 链路 + 网络工具 | 🔨 `tests/live_net.rs`（ignored，手动跑）+ `examples/e2e.rs` 场景探针 |

## 4. Agent Loop（P0）

| 场景 | 验证点 | 现状 |
|---|---|---|
| 纯文本闭环 | message_delta… → run_completed | ✅ `text_only_run_completes_with_events` |
| 工具循环闭环 | tool_call → 执行 → 结果回传 → 续轮 → 终答 | ✅ `tool_loop_executes_and_returns_final_text` |
| 多工具单轮 | 多个 tool_use 顺序执行、消息序列正确 | ✅ `multiple_tool_calls_in_one_turn_execute_sequentially` |
| 工具前文本保留 | assistant 先说话再调工具不丢文本 | ✅ `assistant_turn_text_before_tool_use_is_preserved` |
| max_turns 预算 | 无限循环模型 → `run_failed`（budget_exceeded） | ✅ `budget_exceeded_after_max_turns` |
| 中途失败恢复 | 失败保留部分消息，下次 send 继续 | ✅ `thread::failed_run_keeps_partial_messages_but_next_send_continues` |

## 5. Tool System

| 场景 | 验证点 | 现状 |
|---|---|---|
| Registry | 注册/查询/definitions/空注册表 | ✅ `tool::registry_*`（注：重复注册当前是覆盖语义非拒绝，SDK 化时定为拒绝或告警，见缺口 G-1） |
| Schema 声明 | 每工具 parameters_schema 可单测 | ✅（各工具 schema 为纯函数 + registry definitions 测试） |
| 参数校验 | required 缺失/null 拒绝 | ✅ `permissions::args_validation` 系列 |
| 执行失败恢复 | 未注册工具失败但 Run 继续；错误回传模型 | ✅ `unregistered_tool_fails_but_run_continues` / `tool_error_is_returned_to_model_not_abort_run` |
| 超时 | pandoc 60s 超时模式 | ✅ `doc_tools`（shell 工具超时待 shell 工具加进度时一并看，v3 打包） |
| **panic 隔离** | 工具 panic 不击穿 runtime | ⬜ G-2（P1）：`tokio::spawn` + catch 或 catch_unwind 包装工具执行，配测试 |
| 内置工具行为 | fs/doc/net/skill 工具单测 | ✅ fs_basic/fs_tools/doc_tools/net_tools/skill_tools 各模块 |

## 6. Permission System

| 场景 | 验证点 | 现状 |
|---|---|---|
| 静态策略判定 | read 允 / write 确认 / 未知拒绝 / 自定义覆盖 | ✅ `readonly_tools_allow` / `write_tools_confirm` / `unknown_tools_deny` / `custom_policy_overrides_defaults` |
| Confirm 流程 | allow 执行 / deny 拦截，事件正确 | ✅ `confirm_allow_executes_tool` / `confirm_deny_blocks_execution` / `deny_policy_blocks_execution_without_confirm` |
| 路径安全 | 路径逃逸 / symlink 逃逸 / 写目标逃逸 | ✅ `path_escape_detected` / `symlink_escape_detected*` |
| 批准绑定 tool_call_id | 批准不扩大范围 | ✅ 设计 + 桥测试（`tool::request_permission_*`）；端到端「允许 A 后 B 仍需确认」由 confirm_deny 用例覆盖语义 |
| 等待确认超时 | — | 设计上无超时（用户思考时间不可预估）；取消经 RunCancelled ✅（`tool::request_permission_cancel_*` 三时序） |
| 风险分级/preview | risk_level 等增强字段 | ⬜ 阶段二 PolicyEngine（PLAN 2.x），事件随 v3 打包 |

## 7. Context Management

| 场景 | 验证点 | 现状 |
|---|---|---|
| 消息序列 | user/assistant/tool 顺序严格 | ✅ `openai_sse::multi_turn_tool_loop_order_strictly_preserved` + thread 系列 |
| 自动压缩 | 阈值触发 → compaction_completed → 后续请求裁剪 | ✅ `run_loop_compacts_when_threshold_exceeded` / `request_view_*` / `should_compact_*` |
| 压缩失败降级 | 失败后继续原上下文不重试 | ✅ `run_loop_disables_compaction_after_failure_but_run_continues` |
| Token 预算 | CJK 估算、超限裁剪 | ✅ `estimate_tokens_*` / `request_view_retains_budget_and_truncates_overflow` |

## 8. Plan（P0 部分）

| 场景 | 验证点 | 现状 |
|---|---|---|
| Plan 状态机 | upsert 就地更新/追加有序/未知查询 | ✅ `plan_upsert_*` / `plan_state_of_*` |
| 事件形状 | plan_updated 已锁定快照 | ✅（快照含该变体；⚠️ 当前 Run 循环**暂不发出**，见文档标注） |
| run_loop 驱动 | 复杂任务产生 plan_updated 且状态同步 | ⬜ 阶段一 1.5 接线后补闭环测试 |

## 9. Artifact

| 场景 | 验证点 | 现状 |
|---|---|---|
| 文件产物 | id/kind/location/metadata/preview 形状 | ✅ `tool_output_artifacts_roundtrip` / `artifact_new_generates_unique_ids` |
| kind 序列化 | snake_case 七类 | ✅ `artifact_kind_serializes_snake_case` |
| edit_file diff 产物 | insertions/deletions × 命中次数、preview 含 -/+ 行 | ✅ `edit_file_replaces_unique_match` + `diff::*` 四用例 |
| 事件携带 | completed 事件 artifacts 形状（空省略） | ✅ `empty_artifacts_are_skipped_in_completed_event` + 快照 |
| version/history 生命周期 | — | 预留字段位不实现（PLAN 3.1），不测 |

## 10. Host Adapter

| 场景 | 验证点 | 现状 |
|---|---|---|
| 宿主无关性 | core 不依赖 tauri/gpui | ✅ 独立 crate 构建 + skill 铁律评审 |
| Tauri 集成 | ThreadConfig 字面量构造、事件桥 | 🔨 `cargo check` 基线；交互链路属 src-tauri 手动验收 |
| CLI | examples/e2e.rs 场景探针（text/tool/allow/deny/cancel） | 🔨 手动跑（`cargo run --example e2e`），非断言测试 |
| WebSocket/SSE 断线重连 | 事件恢复、去重（event_id） | ⬜ 宿主层职责；core 侧已备 event_id/sequence，重连去重测试随 replay（P1） |

## 11. Error Handling

| 场景 | 验证点 | 现状 |
|---|---|---|
| 后端错误 → run_failed | 请求失败/流中失败 | ✅ `backend_error_propagates_as_run_failed` / `mid_stream_error_yields_run_failed` |
| SSE 错误帧分类 | error payload → ModelError | ✅ `sse_error_payload_is_model_error` |
| 取消优先 | 三处 select 覆盖（流/工具/权限） | ✅ thread/run/tool 各取消时序测试 |
| **非法 JSON 参数** | 模型产出坏 JSON → 工具失败不崩溃 | 🔨 由 schema 校验拒绝（`args_validation`）；坏 JSON 直接进工具的路径无显式用例，归入 G-2 一并补 |
| **工具 panic** | runtime 存活 | ⬜ G-2（P1） |

## 12. Performance（P2，SDK 化后）

⬜ 100 并发 Run（多 Thread）内存/延迟/事件不丢；100MB 文件流式写不阻塞。
当前单用户桌面场景无此需求，SDK 发布前再建 `tests/bench.rs` 或 criterion。

---

## 当前活跃缺口清单

| ID | 缺口 | 优先级 | 归属 |
|---|---|---|---|
| G-1 | Registry 重复注册语义（覆盖 or 拒绝）+ 测试 | P1 | 阶段一收尾 |
| G-2 | 工具 panic 隔离 + 坏 JSON 参数路径测试 | P1 | 阶段一收尾/阶段二前 |
| G-3 | Replay 测试（事件落盘→重放→状态一致） | P1 | 阶段三 ThreadStore 后 |
| G-4 | Plan run_loop 驱动闭环测试 | P0（随 1.5） | 阶段一 1.5 |
| G-5 | live_net 纳入定期手动冒烟清单 | P2 | 持续 |
| G-6 | PolicyEngine 风险分级事件（v3 打包） | P1 | 阶段二 |
| G-7 | Performance 基准 | P2 | SDK 发布前 |

## 验收基线

- `cargo test -p agent-core` 全绿（当前 **174**，+1 ignored 快照再生成 +4 live_net ignored）
- 快照契约：改事件形状 → `EVENT_SCHEMA_VERSION` bump → 快照重生成 → 本文档同步
- src-tauri `cargo check` 通过（宿主构造兼容）
- 新增能力必须先在本清单登记场景再写测试（防「测函数」回潮）
