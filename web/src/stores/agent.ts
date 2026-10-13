/**
 * Agent 会话全局状态（zustand）
 *
 * 目的：Agent 页切走时实时对话不丢——Tauri 事件监听为模块级单例（ensureAgentListeners），
 * 事件持续写入 store；运行中的 Run、条目流、当前项目绑定全局保留，重新进入 /agent 即恢复。
 *
 * 职责边界：
 * - store 持有会话数据与运行状态：sessionId / entries / running / status / error /
 *   projectPath / input / sessions（历史列表）
 * - 输入弹层（@/ /）、历史面板开关、模型参数（systemPrompt 等）是易失 UI 状态，留在页面组件
 * - agent_start_run / agent_cancel / agent_permission_decide 的调用留在页面组件
 *   （依赖表单与模型选择校验），store 只承接结果状态
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useWorkspaceStore } from '@/stores/workspace';
import type { AgentSessionMeta } from '@/pages/AgentHistoryPanel';
import type { Artifact } from '@/components/agent/ArtifactList';

// ===== 事件模型（与 agent-core Envelope 对齐，type 标签 snake_case） =====

export interface AgentEnvelope {
  /** 旧载荷可省略这些 additive 元数据；排序只依赖 sequence。 */
  schema_version?: number;
  event_id?: string;
  timestamp?: number;
  session_id: string;
  run_id: string;
  sequence: number;
  type: string;
  text?: string;
  status?: string;
  tool_call_id?: string;
  tool_name?: string;
  args_summary?: string;
  /** 完整参数快照（超长字符串已截断）：写文件内容 / 完整命令预览用 */
  args?: Record<string, unknown>;
  result_summary?: string;
  error?: string;
  summary?: string;
  final_text?: string;
  decision?: string;
  artifacts?: Artifact[];
  /** 工具参数流式片段（tool_call_args_delta） */
  fragment?: string;
  /** usage_updated 载荷（字段均可缺省） */
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  max_tokens?: number;
}

/** 上下文用量（usage_updated 写入；新会话/加载会话重置为 null） */
export interface AgentUsage {
  input?: number;
  output?: number;
  total?: number;
  max?: number;
}

export type ToolStatus = 'running' | 'completed' | 'failed' | 'denied' | 'interrupted';

export type PermState = 'pending' | 'allowed' | 'denied' | 'expired';

export type Entry =
  | { kind: 'notice'; text: string }
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string; streaming: boolean }
  | {
      /** 模型思考过程（reasoning），流式追加；参考 Zed 的 thinking 块 */
      kind: 'thinking';
      text: string;
      /** 思考结束（后续事件到达即视为结束）；历史恢复的条目缺省视为已完成 */
      done?: boolean;
    }
  | {
      kind: 'tool';
      callId: string;
      toolName: string;
      argsSummary: string;
      /** 完整参数快照（超长已截断）：内容预览用 */
      args?: Record<string, unknown>;
      status: ToolStatus;
      result?: string;
      artifacts?: Artifact[];
      /** Confirm 工具的确认状态：内嵌在工具卡里，不再单出一张权限卡 */
      perm?: { state: PermState; summary: string };
      /** 参数流式生成中的原始 JSON 累计（tool_call_args_delta 到来时更新） */
      argsStreaming?: string;
    }
  | {
      kind: 'permission';
      callId: string;
      toolName: string;
      summary: string;
      state: PermState;
    }
  | {
      kind: 'question';
      questionId: string;
      question: string;
      options: string[];
      allowFreeText: boolean;
      /** 已回答的答案（verbatim 展示） */
      answer?: string;
      /** 用户取消/拒绝回答 */
      declined?: boolean;
    }
  | { kind: 'terminal'; outcome: 'completed' | 'failed' | 'cancelled'; text?: string };

export function newSessionId(): string {
  return `agent-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** 解析自动压缩阈值：null/非法 → 默认 60000；'0' 合法（= 关闭） */
function parseAutoCompact(raw: string | null | undefined): number {
  if (raw === null || raw === undefined || raw === '') return 60000;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 60000;
}

/** 终态和历史回放共用：不让遗留卡片继续等待或可操作。 */
export function normalizeEntries(entries: Entry[]): Entry[] {
  return entries.map((e): Entry => {
    if (e.kind === 'assistant') return e.streaming ? { ...e, streaming: false } : e;
    if (e.kind === 'thinking') return e.done ? e : { ...e, done: true };
    if (e.kind === 'tool') return {
      ...e,
      status: e.status === 'running' ? 'interrupted' : e.status,
      perm: e.perm?.state === 'pending' ? { ...e.perm, state: 'expired' } : e.perm,
    };
    if (e.kind === 'permission' && e.state === 'pending') return { ...e, state: 'expired' };
    if (e.kind === 'question' && e.answer === undefined && !e.declined) return { ...e, declined: true };
    return e;
  });
}

export interface RunWindow {
  activeRunId: string | null;
  lastSequence: number;
  /** 按 session 保存墓碑；切换历史后也不能重新绑定已结束 Run。 */
  endedRuns: ReadonlyMap<string, ReadonlySet<string>>;
}

type RunState = RunWindow & { sessionId: string; running: boolean; status: string | null; entries: Entry[]; usage: AgentUsage | null };

/** usage_updated 载荷 → 用量快照（缺省字段保持 undefined） */
function readUsage(env: AgentEnvelope): AgentUsage {
  return {
    input: env.input_tokens,
    output: env.output_tokens,
    total: env.total_tokens,
    max: env.max_tokens,
  };
}

function closeRun(state: RunState): RunWindow {
  const endedRuns = new Map(state.endedRuns);
  if (state.activeRunId !== null) {
    endedRuns.set(state.sessionId, new Set([...(endedRuns.get(state.sessionId) ?? []), state.activeRunId]));
  }
  return { activeRunId: null, lastSequence: 0, endedRuns };
}

export function transitionRun(state: RunState, running: boolean, status: string | null): RunState {
  if (running && state.running) return { ...state, status };
  return {
    ...state, ...closeRun(state), running, status: running ? status : null,
    entries: normalizeEntries(state.entries),
  };
}

function isTerminal(env: AgentEnvelope): boolean {
  return ['run_completed', 'run_failed', 'run_cancelled'].includes(env.type);
}

/** 返回原引用表示拒收；run_id 为不透明字符串，绝不解析。 */
export function reduceAgentEvent(state: RunState, env: AgentEnvelope): RunState {
  if (!state.running || env.session_id !== state.sessionId || !env.run_id ||
      state.endedRuns.get(env.session_id)?.has(env.run_id) ||
      (state.activeRunId !== null && state.activeRunId !== env.run_id) ||
      !Number.isSafeInteger(env.sequence) || env.sequence <= state.lastSequence) return state;
  const next = {
    ...state,
    activeRunId: env.run_id,
    lastSequence: env.sequence,
    entries: applyEnvelope(state.entries, env),
    status: env.type === 'status_changed' ? env.status ?? null : state.status,
    usage: env.type === 'usage_updated' ? readUsage(env) : state.usage ?? null,
  };
  return isTerminal(env) ? transitionRun(next, false, null) : next;
}

/** Envelope → Entry 归并（纯函数，返回新数组） */
export function applyEnvelope(prev: Entry[], env: AgentEnvelope): Entry[] {
  const next = applyEnvelopeInner(prev, env);
  // 思考完成判定：任何非 thinking 增量到达，当前流式思考即已结束。
  // 注意不能只看末尾条目——后到的事件会先在 thinking 后面插入新条目，
  // 所以要扫描所有未完成的 thinking 条目（UI 据此自动收起思考块）
  if (env.type !== 'thinking_delta' && next.some((e) => e.kind === 'thinking' && !e.done)) {
    return next.map((e) => e.kind === 'thinking' && !e.done ? { ...e, done: true } : e);
  }
  return next;
}

function applyEnvelopeInner(prev: Entry[], env: AgentEnvelope): Entry[] {
  switch (env.type) {
    case 'message_delta': {
      const text = env.text ?? '';
      const last = prev[prev.length - 1];
      if (last && last.kind === 'assistant' && last.streaming) {
        const next = [...prev];
        next[next.length - 1] = { ...last, text: last.text + text };
        return next;
      }
      return [...prev, { kind: 'assistant', text, streaming: true }];
    }
    case 'thinking_delta': {
      // 思考增量：追加到末尾的 thinking 条目（参考 Zed 的 thinking 块）
      const text = env.text ?? '';
      const last = prev[prev.length - 1];
      if (last && last.kind === 'thinking') {
        const next = [...prev];
        next[next.length - 1] = { ...last, text: last.text + text };
        return next;
      }
      return [...prev, { kind: 'thinking', text }];
    }
    case 'tool_call_args_delta': {
      // 参数流式生成：fragment 为空 = 首帧（建卡），非空 = 追加原始 JSON 片段。
      // 后续 tool_call_started（参数完整）会用 argsSummary/args 覆盖预览
      const fragment = env.fragment ?? '';
      const idx = prev.findIndex((e) => e.kind === 'tool' && e.callId === env.tool_call_id);
      if (idx >= 0) {
        const next = [...prev];
        const e = next[idx];
        if (e.kind === 'tool') {
          next[idx] = { ...e, argsStreaming: (e.argsStreaming ?? '') + fragment };
        }
        return next;
      }
      return [
        ...prev,
        {
          kind: 'tool' as const,
          callId: env.tool_call_id!,
          toolName: env.tool_name ?? '',
          argsSummary: '',
          status: 'running' as const,
          argsStreaming: fragment,
        },
      ];
    }
    case 'tool_call_started': {
      const rest = prev.filter((e) => !(e.kind === 'tool' && e.callId === env.tool_call_id));
      return [
        ...rest,
        {
          kind: 'tool',
          callId: env.tool_call_id!,
          toolName: env.tool_name!,
          argsSummary: env.args_summary ?? '',
          args: env.args,
          status: 'running',
        },
      ];
    }
    case 'tool_call_completed':
      return prev.map((e) =>
        e.kind === 'tool' && e.callId === env.tool_call_id
          ? {
              ...e,
              status: 'completed',
              result: env.result_summary,
              artifacts: env.artifacts,
            }
          : e,
      );
    case 'tool_call_failed':
      return prev.map((e) => {
        if (e.kind !== 'tool' || e.callId !== env.tool_call_id) return e;
        const denied = env.error?.includes('拒绝') ?? false;
        return { ...e, status: denied ? ('denied' as const) : ('failed' as const), result: env.error };
      });
    case 'permission_requested': {
      // 确认 UI 内嵌到同 callId 的工具卡（Zed 模式），不再单独出一张权限卡；
      // 工具卡还没到（理论上不应发生）时才落回独立权限条目，且按 callId 去重
      const toolIdx = prev.findIndex((e) => e.kind === 'tool' && e.callId === env.tool_call_id);
      if (toolIdx >= 0) {
        return prev.map((e, i) =>
          i === toolIdx && e.kind === 'tool'
            ? { ...e, perm: { state: 'pending' as const, summary: env.summary ?? '' } }
            : e,
        );
      }
      const exists = prev.some((e) => e.kind === 'permission' && e.callId === env.tool_call_id);
      if (exists) return prev;
      return [
        ...prev,
        {
          kind: 'permission',
          callId: env.tool_call_id!,
          toolName: env.tool_name!,
          summary: env.summary ?? '',
          state: 'pending',
        },
      ];
    }
    case 'permission_decided':
      return prev.map((e) => {
        if (e.kind === 'tool' && e.callId === env.tool_call_id && e.perm) {
          return {
            ...e,
            perm: { ...e.perm, state: env.decision === 'allow' ? ('allowed' as const) : ('denied' as const) },
          };
        }
        if (e.kind === 'permission' && e.callId === env.tool_call_id) {
          return { ...e, state: env.decision === 'allow' ? ('allowed' as const) : ('denied' as const) };
        }
        return e;
      });
    case 'run_completed':
    case 'run_failed':
    case 'run_cancelled': {
      const closed = normalizeEntries(prev);
      if (env.type === 'run_completed') {
        const last = closed[closed.length - 1];
        if (last && last.kind === 'assistant' && last.text.trim()) {
          return closed;
        }
        // 没有文本产出的 Run（如全部工具失败）：补一条终态
        return [...closed, { kind: 'terminal', outcome: 'completed' }];
      }
      if (env.type === 'run_failed') {
        return [...closed, { kind: 'terminal', outcome: 'failed', text: env.error }];
      }
      return [...closed, { kind: 'terminal', outcome: 'cancelled' }];
    }
    case 'compaction_completed':
      return [...prev, { kind: 'notice', text: '上下文已压缩' }];
    default:
      return prev;
  }
}

/** agent-question 事件载荷（ask_user 提问卡） */
export interface AgentQuestionPayload {
  session_id: string;
  question_id: string;
  question: string;
  options: string[];
  allow_free_text: boolean;
}

// ===== 思考模式（thinking_enabled / thinking_effort 随 agent_start_run 下发） =====

export const THINKING_EFFORTS = ['minimal', 'low', 'medium', 'high'] as const;
export type ThinkingEffort = (typeof THINKING_EFFORTS)[number];

/** 解析持久化的力度值：非法/缺省回退 'medium' */
export function parseThinkingEffort(raw: string | null | undefined): ThinkingEffort {
  return (THINKING_EFFORTS as readonly string[]).includes(raw ?? '') ? (raw as ThinkingEffort) : 'medium';
}

// ===== Store =====

interface AgentState extends RunWindow {
  sessionId: string;
  entries: Entry[];
  input: string;
  running: boolean;
  settling: boolean;
  sessionBusy: boolean;
  status: string | null;
  persistenceErrors: Record<string, string>;
  error: string;
  /** 项目绑定（空 = 纯聊天）；顶部工具栏选择，@ 只负责引用文件 */
  projectPath: string;
  /** 历史会话列表（save/list/delete 后经 refreshSessions 刷新） */
  sessions: AgentSessionMeta[];
  /** 放权模式：开启后所有确认类操作自动批准，不再弹确认卡 */
  autoApprove: boolean;
  /** Agent 最大续轮数（防无限工具循环预算，默认 50） */
  maxTurns: number;
  /** 上下文自动压缩阈值（估算 token 数，默认 60000，0 = 关闭） */
  autoCompactTokens: number;
  /** Local preference; existing sessions retain their backend-bound consent. */
  imageRecognition: boolean;
  setImageRecognition: (enabled: boolean) => void;
  /** 上下文用量（usage_updated 写入；新会话/加载会话重置为 null） */
  usage: AgentUsage | null;
  /** 思考模式开关（localStorage 持久化；发送时随 agent_start_run 下发，下次发送即生效） */
  thinkingEnabled: boolean;
  setThinkingEnabled: (v: boolean) => void;
  /** 思考力度（localStorage 持久化） */
  thinkingEffort: ThinkingEffort;
  setThinkingEffort: (v: ThinkingEffort) => void;

  setInput: (v: string) => void;
  setError: (v: string) => void;
  setProjectPath: (v: string) => void;
  /** 切换放权模式（同步到后端 + localStorage 持久化） */
  setAutoApprove: (v: boolean) => void;
  /** 设置最大续轮数（localStorage 持久化，新会话生效） */
  setMaxTurns: (v: number) => void;
  /** 设置自动压缩阈值（localStorage 持久化，0 = 关闭） */
  setAutoCompactTokens: (v: number) => void;
  setRunState: (running: boolean, status: string | null) => void;
  pushUser: (text: string) => void;
  pushTerminal: (outcome: 'completed' | 'failed' | 'cancelled', text?: string) => void;
  /** 收到 ask_user 提问事件（追加问题卡条目） */
  pushQuestion: (q: AgentQuestionPayload) => void;
  /** 回答问题卡：answer 为 null 表示取消（本地立即置态，后端 oneshot 解析） */
  answerQuestion: (questionId: string, answer: string | null) => Promise<void>;
  /** 新对话：重置会话（旧会话保存由调用方先行调用 saveSession） */
  startNewChat: () => void;
  newChat: () => Promise<void>;
  deleteSession: (id: string) => Promise<void>;
  /** 恢复 UI 历史；续聊时后端从 SQLite 加载模型上下文。 */
  loadSession: (s: AgentSessionMeta) => void;
  /** 保存会话到后端并刷新列表（默认保存当前会话；空条目跳过） */
  saveSession: (id?: string) => Promise<void>;
  refreshSessions: () => Promise<void>;
}

export const useAgentStore = create<AgentState>((set, get) => ({
  sessionId: newSessionId(),
  activeRunId: null,
  lastSequence: 0,
  endedRuns: new Map(),
  entries: [],
  input: '',
  running: false,
  settling: false,
  sessionBusy: false,
  persistenceErrors: {},
  status: null,
  error: '',
  projectPath: '',
  sessions: [],
  autoApprove: typeof localStorage !== 'undefined' && localStorage.getItem('astra-agent-autoapprove') === '1',
  maxTurns: Number(localStorage?.getItem('astra-agent-maxturns')) || 50,
  autoCompactTokens: parseAutoCompact(localStorage?.getItem('astra-agent-autocompact')),
  imageRecognition: typeof localStorage !== 'undefined' && localStorage.getItem('astra-agent-image-recognition') === '1',
  setImageRecognition: (enabled) => {
    localStorage.setItem('astra-agent-image-recognition', enabled ? '1' : '0');
    set({ imageRecognition: enabled });
  },
  usage: null,
  thinkingEnabled: typeof localStorage !== 'undefined' && localStorage.getItem('astra-agent-thinking') === '1',
  setThinkingEnabled: (v) => {
    localStorage.setItem('astra-agent-thinking', v ? '1' : '0');
    set({ thinkingEnabled: v });
  },
  thinkingEffort: parseThinkingEffort(typeof localStorage === 'undefined' ? null : localStorage.getItem('astra-agent-effort')),
  setThinkingEffort: (v) => {
    localStorage.setItem('astra-agent-effort', v);
    set({ thinkingEffort: v });
  },

  setInput: (v) => set({ input: v }),
  setMaxTurns: (v) => {
    const n = Number.isFinite(v) && v >= 1 ? Math.floor(v) : 50;
    set({ maxTurns: n });
    localStorage.setItem('astra-agent-maxturns', String(n));
  },
  setAutoCompactTokens: (v) => {
    const n = Number.isFinite(v) && v >= 0 ? Math.floor(v) : 60000;
    set({ autoCompactTokens: n });
    localStorage.setItem('astra-agent-autocompact', String(n));
  },
  setError: (v) => set((s) => ({ error: v || persistenceMessage(s.persistenceErrors[s.sessionId]) })),
  setProjectPath: (v) => set({ projectPath: v }),
  setAutoApprove: (v) => {
    set({ autoApprove: v });
    localStorage.setItem('astra-agent-autoapprove', v ? '1' : '0');
    invoke('agent_set_auto_all', { enabled: v }).catch(() => {
      // Tauri 不可用（纯 web 调试）时仅保留前端状态
    });
  },
  setRunState: (running, status) => set((s) => ({ ...transitionRun(s, running, status), settling: false })),
  pushUser: (text) => set((s) => ({ entries: [...s.entries, { kind: 'user', text }] })),
  pushTerminal: (outcome, text) =>
    set((s) => ({ ...transitionRun(s, false, null), entries: [...normalizeEntries(s.entries), { kind: 'terminal', outcome, text }] })),
  pushQuestion: (q) =>
    set((s) =>
      // 幂等防御：同一 questionId 只入列一次（防事件层重复投递）
      !s.running || s.settling || q.session_id !== s.sessionId ||
      s.entries.some((e) => e.kind === 'question' && e.questionId === q.question_id)
        ? s
        : {
            entries: [
              ...s.entries,
              {
                kind: 'question',
                questionId: q.question_id,
                question: q.question,
                options: q.options,
                allowFreeText: q.allow_free_text,
              },
            ],
          },
    ),
  answerQuestion: async (questionId, answer) => {
    const before = get();
    const question = before.entries.find((e) => e.kind === 'question' && e.questionId === questionId);
    if (!before.running || before.settling || question?.kind !== 'question' || question.answer !== undefined || question.declined) return;
    const runId = before.activeRunId;
    const sessionId = before.sessionId;
    // 本地先置态（回答/取消后卡片禁用），后端 oneshot 解析由 invoke 完成
    set((s) => ({
      entries: s.entries.map((e) =>
        e.kind === 'question' && e.questionId === questionId
          ? answer === null
            ? { ...e, declined: true }
            : { ...e, answer }
          : e,
      ),
    }));
    try {
      await invoke('agent_question_answer', { questionId, answer });
    } catch (e) {
      // 提交失败必须可见：否则工具挂起等回答，用户看到的是「永远在分析」
      console.error('回答提交失败', e);
      set((s) => s.sessionId !== sessionId || s.activeRunId !== runId || !s.running || s.settling ? s : ({
        error: `回答提交失败：${String(e)}`,
        entries: s.entries.map((en) =>
          en.kind === 'question' && en.questionId === questionId
            ? { ...en, answer: undefined, declined: undefined }
            : en,
        ),
      }));
    }
  },
  startNewChat: () =>
    set((s) => s.running || s.sessionBusy ? s : ({ ...transitionRun(s, false, null), settling: false, sessionId: newSessionId(), entries: [], error: '', input: '', usage: null })),
  newChat: async () => {
    const st = get();
    if (st.running || st.sessionBusy) return;
    set({ sessionBusy: true });
    try {
      await st.saveSession();
      await invoke('agent_reset_session', { sessionId: st.sessionId });
      set({ sessionBusy: false });
      get().startNewChat();
    } catch (error) {
      get().setError(`新对话失败：${String(error)}`);
    } finally {
      set({ sessionBusy: false });
    }
  },
  deleteSession: async (id) => {
    if (get().running || get().sessionBusy) return;
    set({ sessionBusy: true });
    try {
      await invoke('delete_agent_session', { id });
      const persistenceErrors = { ...get().persistenceErrors };
      delete persistenceErrors[id];
      set({ persistenceErrors });
      await get().refreshSessions();
      set({ sessionBusy: false });
      if (get().sessionId === id) get().startNewChat();
    } catch (error) {
      get().setError(`删除会话失败：${String(error)}`);
    } finally {
      set({ sessionBusy: false });
    }
  },
  loadSession: (s) =>
    set((current) => current.running || current.sessionBusy ? current : ({
      ...transitionRun(current, false, null),
      settling: false,
      sessionId: s.id,
      entries: normalizeEntries((s.entries as Entry[]) ?? []),
      projectPath: s.project_path ?? '',
      status: null,
      error: persistenceMessage(current.persistenceErrors[s.id]),
      usage: null,
    })),
  refreshSessions: async () => {
    try {
      set({ sessions: await invoke<AgentSessionMeta[]>('list_agent_sessions') });
    } catch {
      // 后端不可用（本地调试）时忽略
    }
  },
  saveSession: async (id) => {
    const { entries, projectPath } = get();
    if (entries.length === 0) return;
    const firstUser = entries.find((e): e is Extract<Entry, { kind: 'user' }> => e.kind === 'user');
    const title = firstUser ? firstUser.text.slice(0, 30) : 'Agent 会话';
    const projectName =
      useWorkspaceStore.getState().projects.find((p) => p.path === projectPath)?.name ?? '';
    try {
      await invoke('save_agent_session', {
        id: id ?? get().sessionId,
        projectPath,
        projectName,
        title,
        entries,
      });
      await get().refreshSessions();
    } catch (e) {
      get().setError(`保存页面历史失败：${String(e)}`);
      throw e;
    }
  },
}));

function persistenceMessage(error?: string | null): string {
  return error ? `模型上下文保存失败：${error}。未保存的上下文仍在内存中，请勿退出应用；后续运行可重试保存。` : '';
}

interface AgentSettledPayload {
  session_id: string;
  persistence_error: string | null;
}

function settleRun(payload: AgentSettledPayload): void {
  const st = useAgentStore.getState();
  const persistenceErrors = { ...st.persistenceErrors };
  const previousMessage = persistenceMessage(persistenceErrors[payload.session_id]);
  if (payload.persistence_error) persistenceErrors[payload.session_id] = payload.persistence_error;
  else delete persistenceErrors[payload.session_id];
  const current = st.sessionId === payload.session_id;
  useAgentStore.setState({
    ...(current ? {
      ...transitionRun(st, false, null),
      settling: false,
      error: persistenceMessage(payload.persistence_error) || (st.error === previousMessage ? '' : st.error),
    } : {}),
    persistenceErrors,
  });
  if (current && st.running) void st.saveSession().catch(() => {});
}

// Status is a snapshot, not an event: never let an older request overwrite a new Run/settlement.
let lifecycleRevision = 0;
let statusRequest = 0;
useAgentStore.subscribe((next, prev) => {
  if (next.sessionId !== prev.sessionId || next.running !== prev.running ||
      next.settling !== prev.settling || next.sessionBusy !== prev.sessionBusy ||
      next.activeRunId !== prev.activeRunId || next.lastSequence !== prev.lastSequence ||
      next.persistenceErrors !== prev.persistenceErrors) lifecycleRevision++;
});

export async function refreshAgentStatus(): Promise<void> {
  const request = ++statusRequest;
  const revision = lifecycleRevision;
  if (useAgentStore.getState().sessionBusy) return;
  try {
    const status = await invoke<{
      running: boolean; session_id: string | null; persistence_errors: Record<string, string>;
    }>('agent_status');
    if (request !== statusRequest || revision !== lifecycleRevision) return;
    const st = useAgentStore.getState();
    if (!status.running) {
      settleRun({ session_id: st.sessionId, persistence_error: status.persistence_errors[st.sessionId] ?? null });
      useAgentStore.setState({ persistenceErrors: status.persistence_errors });
    } else if (status.session_id) {
      const sameSession = status.session_id === st.sessionId;
      useAgentStore.setState({
        sessionId: status.session_id,
        persistenceErrors: status.persistence_errors,
        running: true,
        // A recovered stream may have missed its terminal event; wait for the host barrier.
        settling: sameSession ? st.settling : true,
        status: sameSession ? st.status ?? 'analyzing' : 'persisting',
        ...(sameSession ? {} : { activeRunId: null, lastSequence: 0, entries: [] }),
        error: persistenceMessage(status.persistence_errors[status.session_id]) || st.error,
      });
    }
  } catch (error) {
    if (request === statusRequest && revision === lifecycleRevision) {
      useAgentStore.getState().setError(`恢复运行状态失败：${String(error)}`);
    }
  }
}

// ===== 模块级单例事件监听 =====

let listenersPromise: Promise<void> | undefined;

/** 页面 mount 时可预热；发送前必须 await。失败向调用方传播且允许重试。 */
export function ensureAgentListeners(): Promise<void> {
  if (listenersPromise) return listenersPromise;
  listenersPromise = (async () => {
    // 注册期间/失败回滚期间的半套监听器不能消费事件。
    let ready = false;
    const results = await Promise.allSettled([
      Promise.resolve().then(() => listen<AgentQuestionPayload>('agent-question', (e) => {
        if (ready) useAgentStore.getState().pushQuestion(e.payload);
      })),
      Promise.resolve().then(() => listen<AgentEnvelope>('agent-event', (e) => {
        if (!ready) return;
        const st = useAgentStore.getState();
        if (st.settling) return;
        const next = reduceAgentEvent(st, e.payload);
        if (next === st) return;
        useAgentStore.setState(isTerminal(e.payload)
          ? { ...next, running: true, settling: true, status: 'persisting' }
          : next);
      })),
      Promise.resolve().then(() => listen<AgentSettledPayload>('agent-run-settled', (e) => {
        if (ready) settleRun(e.payload);
      })),
    ]);
    const errors: unknown[] = [];
    for (const result of results) {
      if (result.status === 'rejected') errors.push(result.reason);
    }
    if (errors.length) {
      for (const result of results) {
        if (result.status === 'fulfilled') {
          try { result.value(); } catch (error) { errors.push(error); }
        }
      }
      throw new AggregateError(errors, 'Agent 事件监听注册失败');
    }
    ready = true;
  })().catch((error: unknown) => {
    listenersPromise = undefined;
    throw error;
  });
  return listenersPromise;
}
