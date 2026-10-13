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

// ===== 多会话（SESSION_RUNTIME_SPEC S0：retained 等价实现）=====

export type SessionPhase = 'draft' | 'active';

/** 驻留内存的非激活会话快照（= Zed retained_threads 条目，结构兼容 RunState） */
export interface ParkedSession {
  sessionId: string;
  isDraft: boolean;
  entries: Entry[];
  input: string;
  running: boolean;
  settling: boolean;
  status: string | null;
  activeRunId: string | null;
  lastSequence: number;
  endedRuns: ReadonlyMap<string, ReadonlySet<string>>;
  projectPath: string;
  usage: AgentUsage | null;
  error: string;
  lastActive: number;
}

/** 打开 tab 的软上限（超出时按 LRU 清理可清理的驻留会话，不清则允许超出） */
export const MAX_RETAINED_TABS = 8;

/** 会话是否有待用户决策的卡片（权限/问题）——LRU 保护 + tab 徽标 */
export function hasPendingDecision(entries: Entry[]): boolean {
  return entries.some(
    (e) =>
      (e.kind === 'tool' && e.perm?.state === 'pending') ||
      (e.kind === 'permission' && e.state === 'pending') ||
      (e.kind === 'question' && e.answer === undefined && !e.declined),
  );
}

/** 当前占用后端准入（活动 Run）的会话；null = 空闲。单活动 Run 期间用于多 tab 发送互斥 */
export function runningSessionId(
  st: Pick<AgentState, 'sessionId' | 'running' | 'parked'>,
): string | null {
  if (st.running) return st.sessionId;
  for (const p of Object.values(st.parked)) if (p.running) return p.sessionId;
  return null;
}

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
  /** checkpoint 恢复后用后端回传的历史重建条目（仅当会话一致且无运行中任务） */
  restoreEntries: (sessionId: string, entries: Entry[]) => void;
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
  /** 保存会话到后端并刷新列表（默认保存当前会话；空条目跳过；snapshot 用于保存驻留会话） */
  saveSession: (id?: string, snapshot?: { entries: Entry[]; projectPath: string }) => Promise<void>;
  refreshSessions: () => Promise<void>;

  // ===== 多会话（S0）：tab 即 AgentSession 视图，数据驻留 store 不销毁 =====
  /** 非激活会话快照；事件按 session_id 路由写入，切回零重建 */
  parked: Record<string, ParkedSession>;
  /** 打开的会话标签（易失 UI 状态，不持久化） */
  openTabs: string[];
  activeTabId: string | null;
  /** 草稿：新会话未发送首条消息（不保存、不进历史、LRU 可清） */
  isDraft: boolean;
  /** 新建会话标签：驻留当前会话，flat 重置为全新草稿 */
  newTab: () => void;
  /** 切换激活 tab：驻留当前 + 恢复目标（驻留快照或历史） */
  setTabActive: (id: string) => void;
  /** 从历史打开会话（同 setTabActive，meta 可不在列表中） */
  activateSession: (meta: AgentSessionMeta) => void;
  /** 关闭 tab：保存 + 释放后端内存；运行中/待决策拒绝 */
  closeTab: (id: string) => boolean;
  /** 打开绑定项目的草稿会话（工作空间页「在 Agent 打开」入口） */
  openDraftForProject: (path: string) => void;
}

const INITIAL_SESSION_ID = newSessionId();

export const useAgentStore = create<AgentState>((set, get) => ({
  sessionId: INITIAL_SESSION_ID,
  isDraft: true,
  parked: {},
  // 新打开时默认无 tab：隐式草稿不发消息不出 tab，点 + 或首条消息后转正
  openTabs: [],
  activeTabId: null,
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
  setProjectPath: (v) => {
    // 记录最近使用的项目文件夹（去重、最多 5 个；空 = 解绑不记录）
    if (v) {
      try {
        const list: string[] = JSON.parse(localStorage.getItem('astra.recentProjects') ?? '[]');
        const next = [v, ...list.filter((p) => p !== v)].slice(0, 5);
        localStorage.setItem('astra.recentProjects', JSON.stringify(next));
      } catch {
        // localStorage 不可用时仅跳过记录
      }
    }
    set({ projectPath: v });
  },
  setAutoApprove: (v) => {
    set({ autoApprove: v });
    localStorage.setItem('astra-agent-autoapprove', v ? '1' : '0');
    invoke('agent_set_auto_all', { enabled: v }).catch(() => {
      // Tauri 不可用（纯 web 调试）时仅保留前端状态
    });
  },
  setRunState: (running, status) =>
    set((s) => ({
      ...transitionRun(s, running, status),
      settling: false,
      ...(running
        ? {
            isDraft: false,
            // 隐式草稿开始运行：补上 tab 转正
            openTabs: s.openTabs.includes(s.sessionId) ? s.openTabs : [...s.openTabs, s.sessionId],
            activeTabId: s.activeTabId ?? s.sessionId,
          }
        : {}),
    })),
  pushUser: (text) =>
    set((s) => ({
      entries: [...s.entries, { kind: 'user', text }],
      isDraft: false,
      // 首条消息触发：隐式草稿补上 tab 转正
      openTabs: s.openTabs.includes(s.sessionId) ? s.openTabs : [...s.openTabs, s.sessionId],
      activeTabId: s.activeTabId ?? s.sessionId,
    })),
  /** checkpoint 恢复：后端已回退，前端用回退后的消息序列重建会话视图（不计入当前 Run） */
  restoreEntries: (sessionId, entries) =>
    set((s) =>
      s.running || s.sessionBusy || s.sessionId !== sessionId
        ? s
        : ({
            ...transitionRun(s, false, null),
            settling: false,
            entries,
            usage: null,
            status: null,
          }),
    ),
  pushTerminal: (outcome, text) =>
    set((s) => ({ ...transitionRun(s, false, null), entries: [...normalizeEntries(s.entries), { kind: 'terminal', outcome, text }] })),
  pushQuestion: (q) =>
    set((s) => {
      // 幂等防御：同一 questionId 只入列一次（防事件层重复投递）
      const dup = (entries: Entry[]) =>
        entries.some((e) => e.kind === 'question' && e.questionId === q.question_id);
      const card = {
        kind: 'question' as const,
        questionId: q.question_id,
        question: q.question,
        options: q.options,
        allowFreeText: q.allow_free_text,
      };
      if (q.session_id === s.sessionId) {
        if (!s.running || s.settling || dup(s.entries)) return s;
        return { entries: [...s.entries, card] };
      }
      // 非激活会话：问题卡路由到驻留快照（切回后可见、可答）
      const p = s.parked[q.session_id];
      if (!p || !p.running || p.settling || dup(p.entries)) return s;
      return {
        parked: { ...s.parked, [p.sessionId]: { ...p, entries: [...p.entries, card], lastActive: Date.now() } },
      };
    }),
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
    set((s) => {
      if (s.running || s.sessionBusy) return s;
      const id = newSessionId();
      const hadTab = s.openTabs.includes(s.sessionId);
      return {
        ...transitionRun(s, false, null),
        settling: false,
        sessionId: id,
        entries: [],
        error: '',
        input: '',
        usage: null,
        isDraft: true,
        openTabs: hadTab ? s.openTabs.map((t) => (t === s.sessionId ? id : t)) : [...s.openTabs, id],
        activeTabId: id,
      };
    }),
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
      set((s) => {
        const openTabs = s.openTabs.filter((t) => t !== id);
        return {
          persistenceErrors,
          parked: omitKey(s.parked, id),
          openTabs,
          activeTabId: s.activeTabId === id ? openTabs[openTabs.length - 1] ?? null : s.activeTabId,
        };
      });
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
  saveSession: async (id, snapshot) => {
    const entries = snapshot ? snapshot.entries : get().entries;
    const projectPath = snapshot ? snapshot.projectPath : get().projectPath;
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

  // ===== 多会话动作（S0：swap 模型实现 retained，tab 状态并入本 store）=====

  newTab: () => {
    if (get().sessionBusy) return;
    set((s) => {
      // 已在空草稿上：无 tab（新打开的隐式草稿）则补上 tab；已有 tab 则无需再开
      if (s.entries.length === 0 && !s.running) {
        if (s.openTabs.includes(s.sessionId)) return s;
        return { openTabs: [...s.openTabs, s.sessionId], activeTabId: s.sessionId };
      }
      // 驻留当前会话（运行中也可驻留，事件路由保证不丢）；新草稿继承项目绑定
      let parked = { ...s.parked, [s.sessionId]: snapshotFlat(s) };
      let openTabs = [...s.openTabs, newSessionId()];
      ({ parked, openTabs } = evictOverflow({ parked, openTabs }));
      const id = openTabs[openTabs.length - 1];
      return {
        ...transitionRun(s, false, null),
        settling: false,
        sessionId: id,
        entries: [],
        input: '',
        usage: null,
        error: '',
        isDraft: true,
        parked,
        openTabs,
        activeTabId: id,
      };
    });
  },

  setTabActive: (id) => {
    const st = get();
    if (st.sessionBusy) return;
    if (id === st.sessionId) {
      set({ activeTabId: id });
      return;
    }
    const swap = swapToSession(st, id);
    if (swap) set(swap);
    else set({ error: '会话不存在' });
  },

  activateSession: (meta) => {
    const st = get();
    if (st.sessionBusy) return;
    if (meta.id === st.sessionId) {
      set({ activeTabId: meta.id });
      return;
    }
    const swap = swapToSession(st, meta.id, meta);
    if (swap) set(swap);
    else set({ error: '会话不存在' });
  },

  closeTab: (id) => {
    const st = get();
    if (st.sessionBusy) return false;
    if (id === st.sessionId) {
      if (st.running || st.settling) return false;
      const idx = st.openTabs.indexOf(id);
      const remaining = st.openTabs.filter((t) => t !== id);
      // 关闭前落库 + 释放后端内存（fire-and-forget；关闭动作本身不阻塞）
      void saveSessionSnapshot(id, { entries: st.entries, projectPath: st.projectPath });
      invoke('agent_reset_session', { sessionId: id }).catch(() => {});
      if (remaining.length === 0 || idx < 0) {
        // 最后一个 tab：重置为全新草稿且无 tab（回到「隐式草稿」状态），保留项目绑定
        const nid = newSessionId();
        set((s) => ({
          ...transitionRun(s, false, null),
          settling: false,
          sessionId: nid,
          entries: [],
          input: '',
          usage: null,
          error: '',
          isDraft: true,
          parked: omitKey(s.parked, id),
          openTabs: [],
          activeTabId: null,
        }));
        return true;
      }
      // swap 到邻居：把关闭中的会话当作空草稿（tab 丢弃、不驻留）
      const neighbor = st.openTabs[idx + 1] ?? st.openTabs[idx - 1];
      const swap = swapToSession({ ...st, entries: [], isDraft: true, running: false, settling: false }, neighbor);
      if (swap) {
        set((s) => ({ ...swap, parked: omitKey(s.parked, id) }));
      } else {
        // 邻居无法解析（防御）：回退到无 tab 的隐式草稿
        const nid = newSessionId();
        set((s) => ({
          ...transitionRun(s, false, null),
          settling: false,
          sessionId: nid,
          entries: [],
          input: '',
          usage: null,
          error: '',
          isDraft: true,
          parked: omitKey(s.parked, id),
          openTabs: [],
          activeTabId: null,
        }));
      }
      return true;
    }
    const p = st.parked[id];
    if (p && (p.running || p.settling)) return false;
    set((s) => {
      const openTabs = s.openTabs.filter((t) => t !== id);
      return {
        parked: omitKey(s.parked, id),
        openTabs,
        activeTabId: s.activeTabId === id ? openTabs[openTabs.length - 1] ?? null : s.activeTabId,
      };
    });
    if (p) {
      void saveSessionSnapshot(id, p);
      invoke('agent_reset_session', { sessionId: id }).catch(() => {});
    }
    return true;
  },

  openDraftForProject: (path) => {
    const st = get();
    if (st.sessionBusy) return;
    if (st.entries.length === 0 && !st.running) {
      // 当前已是空草稿：直接改绑项目（无 tab 时保持隐式，不提前出 tab）
      set({ projectPath: path });
      return;
    }
    get().newTab();
    set({ projectPath: path });
  },
}));

function persistenceMessage(error?: string | null): string {
  return error ? `模型上下文保存失败：${error}。未保存的上下文仍在内存中，请勿退出应用；后续运行可重试保存。` : '';
}

// ===== 多会话模块级 helpers（函数声明提升，供 store 动作与事件层调用）=====

/** flat 会话中参与驻留快照的字段（与 ParkedSession 一一对应） */
type FlatSession = Pick<
  AgentState,
  | 'sessionId' | 'isDraft' | 'entries' | 'input' | 'running' | 'settling' | 'status'
  | 'activeRunId' | 'lastSequence' | 'endedRuns' | 'projectPath' | 'usage' | 'error'
>;

function snapshotFlat(st: FlatSession): ParkedSession {
  return { ...st, lastActive: Date.now() };
}

/** 驻留快照 → flat 状态（去掉 lastActive） */
function hydrateFromParked(p: ParkedSession): FlatSession {
  return {
    sessionId: p.sessionId,
    isDraft: p.isDraft,
    entries: p.entries,
    input: p.input,
    running: p.running,
    settling: p.settling,
    status: p.status,
    activeRunId: p.activeRunId,
    lastSequence: p.lastSequence,
    endedRuns: p.endedRuns,
    projectPath: p.projectPath,
    usage: p.usage,
    error: p.error,
  };
}

/** 历史会话 → flat 状态（loadSession 语义：后端续聊时从 SQLite 加载上下文） */
function hydrateFromMeta(s: AgentSessionMeta, persistenceError?: string | null): FlatSession {
  return {
    sessionId: s.id,
    isDraft: false,
    entries: normalizeEntries((s.entries as Entry[]) ?? []),
    input: '',
    running: false,
    settling: false,
    status: null,
    activeRunId: null,
    lastSequence: 0,
    endedRuns: new Map(),
    projectPath: s.project_path ?? '',
    usage: null,
    error: persistenceMessage(persistenceError),
  };
}

/** 驻留当前 flat 并恢复目标会话；返回 null 表示目标不存在 */
function swapToSession(
  st: AgentState,
  targetId: string,
  meta?: AgentSessionMeta,
): Partial<AgentState> | null {
  let parked = st.parked;
  let openTabs = st.openTabs;
  // 空草稿不驻留：其 tab 被目标顶替；运行中/有内容的会话打快照保活
  if (st.entries.length > 0 || st.running) {
    parked = { ...parked, [st.sessionId]: snapshotFlat(st) };
  } else {
    openTabs = openTabs.filter((t) => t !== st.sessionId);
  }
  const ensureTab = (tabs: string[]): string[] =>
    tabs.includes(targetId) ? tabs : [...tabs, targetId];
  const p = parked[targetId];
  if (p) {
    return {
      ...hydrateFromParked(p),
      parked: omitKey(parked, targetId),
      openTabs: ensureTab(openTabs),
      activeTabId: targetId,
    };
  }
  const found = meta ?? st.sessions.find((s) => s.id === targetId);
  if (found) {
    return {
      ...hydrateFromMeta(found, st.persistenceErrors[targetId]),
      endedRuns: st.endedRuns,
      parked,
      openTabs: ensureTab(openTabs),
      activeTabId: targetId,
    };
  }
  return null;
}

/** tab 超上限时按 LRU 清理可清理的驻留会话（先落库再释放后端内存；无物可清则允许超出） */
function evictOverflow(
  st: Pick<AgentState, 'parked' | 'openTabs'>,
): Pick<AgentState, 'parked' | 'openTabs'> {
  let { parked, openTabs } = st;
  if (openTabs.length <= MAX_RETAINED_TABS) return { parked, openTabs };
  const evictable = Object.values(parked)
    .filter((p) => !p.running && !p.settling && !p.isDraft && !hasPendingDecision(p.entries))
    .sort((a, b) => a.lastActive - b.lastActive);
  for (const p of evictable) {
    if (openTabs.length <= MAX_RETAINED_TABS) break;
    openTabs = openTabs.filter((t) => t !== p.sessionId);
    parked = omitKey(parked, p.sessionId);
    void saveSessionSnapshot(p.sessionId, p);
    invoke('agent_reset_session', { sessionId: p.sessionId }).catch(() => {});
  }
  return { parked, openTabs };
}

/** 保存驻留/关闭中的会话快照（fire-and-forget；失败可见于 error 栏） */
async function saveSessionSnapshot(id: string, snap: { entries: Entry[]; projectPath: string }): Promise<void> {
  if (snap.entries.length === 0) return;
  try {
    const firstUser = snap.entries.find((e): e is Extract<Entry, { kind: 'user' }> => e.kind === 'user');
    const title = firstUser ? firstUser.text.slice(0, 30) : 'Agent 会话';
    const projectName =
      useWorkspaceStore.getState().projects.find((p) => p.path === snap.projectPath)?.name ?? '';
    await invoke('save_agent_session', {
      id,
      projectPath: snap.projectPath,
      projectName,
      title,
      entries: snap.entries,
    });
    await useAgentStore.getState().refreshSessions();
  } catch (e) {
    useAgentStore.getState().setError(`保存会话失败：${String(e)}`);
  }
}

function omitKey(obj: Record<string, ParkedSession>, key: string): Record<string, ParkedSession> {
  const next = { ...obj };
  delete next[key];
  return next;
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
  const parked = st.parked[payload.session_id];
  useAgentStore.setState({
    ...(current ? {
      ...transitionRun(st, false, null),
      settling: false,
      error: persistenceMessage(payload.persistence_error) || (st.error === previousMessage ? '' : st.error),
    } : parked ? {
      // 驻留会话的 Run 结算：更新快照并落库（切回时零重建）
      parked: {
        ...st.parked,
        [parked.sessionId]: {
          ...parked,
          ...transitionRun(parked, false, null),
          settling: false,
          lastActive: Date.now(),
        },
      },
    } : {}),
    persistenceErrors,
  });
  if (current && st.running) void st.saveSession().catch(() => {});
  else if (parked && parked.running) void saveSessionSnapshot(payload.session_id, parked);
}

// Status is a snapshot, not an event: never let an older request overwrite a new Run/settlement.
let lifecycleRevision = 0;
let statusRequest = 0;
useAgentStore.subscribe((next, prev) => {
  if (next.sessionId !== prev.sessionId || next.running !== prev.running ||
      next.settling !== prev.settling || next.sessionBusy !== prev.sessionBusy ||
      next.activeRunId !== prev.activeRunId || next.lastSequence !== prev.lastSequence ||
      next.persistenceErrors !== prev.persistenceErrors ||
      next.parked !== prev.parked || next.openTabs !== prev.openTabs) lifecycleRevision++;
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
      // 驻留中的运行会话一并结算（切走期间后端已把 Run 跑完）
      for (const p of Object.values(st.parked)) {
        if (p.running) {
          settleRun({ session_id: p.sessionId, persistence_error: status.persistence_errors[p.sessionId] ?? null });
        }
      }
      useAgentStore.setState({ persistenceErrors: status.persistence_errors });
    } else if (status.session_id) {
      const sid = status.session_id;
      if (sid === st.sessionId) {
        useAgentStore.setState({
          persistenceErrors: status.persistence_errors,
          running: true,
          // A recovered stream may have missed its terminal event; wait for the host barrier.
          settling: st.settling,
          status: st.status ?? 'analyzing',
          error: persistenceMessage(status.persistence_errors[sid]) || st.error,
        });
      } else if (st.parked[sid]) {
        // 运行中的会话当前处于驻留态：保持事件路由生效
        const p = st.parked[sid]!;
        useAgentStore.setState((s) => ({
          parked: {
            ...s.parked,
            [sid]: { ...p, running: true, settling: true, status: p.status ?? 'persisting', lastActive: Date.now() },
          },
          persistenceErrors: status.persistence_errors,
        }));
      } else {
        // 应用重启恢复：未知的运行中会话 → 建驻留快照并开 tab（终端事件到达后落库）
        const recovered: ParkedSession = {
          sessionId: sid,
          isDraft: false,
          entries: [],
          input: '',
          running: true,
          settling: true,
          status: 'persisting',
          activeRunId: null,
          lastSequence: 0,
          endedRuns: st.endedRuns,
          projectPath: '',
          usage: null,
          error: persistenceMessage(status.persistence_errors[sid]),
          lastActive: Date.now(),
        };
        useAgentStore.setState((s) => ({
          parked: { ...s.parked, [sid]: recovered },
          openTabs: s.openTabs.includes(sid) ? s.openTabs : [...s.openTabs, sid],
          persistenceErrors: status.persistence_errors,
        }));
      }
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
        if (e.payload.session_id === st.sessionId) {
          if (st.settling) return;
          const next = reduceAgentEvent(st, e.payload);
          if (next === st) return;
          useAgentStore.setState(isTerminal(e.payload)
            ? { ...next, running: true, settling: true, status: 'persisting' }
            : next);
          return;
        }
        // 非激活会话：路由到驻留快照（运行中的 parked 会话持续吸收事件，切回零重建）
        const p = st.parked[e.payload.session_id];
        if (!p || p.settling) return;
        const next = reduceAgentEvent(p, e.payload);
        if (next === p) return;
        useAgentStore.setState((s) => ({
          parked: {
            ...s.parked,
            [p.sessionId]: isTerminal(e.payload)
              ? { ...p, ...next, running: true, settling: true, status: 'persisting', lastActive: Date.now() }
              : { ...p, ...next, lastActive: Date.now() },
          },
        }));
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
