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

// ===== 事件模型（与 agent-core Envelope 对齐，type 标签 snake_case） =====

export interface AgentEnvelope {
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
  artifacts?: Array<{ path: string; kind: string }>;
}

export type ToolStatus = 'running' | 'completed' | 'failed' | 'denied';

export type PermState = 'pending' | 'allowed' | 'denied';

export type Entry =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string; streaming: boolean }
  | {
      kind: 'tool';
      callId: string;
      toolName: string;
      argsSummary: string;
      /** 完整参数快照（超长已截断）：内容预览用 */
      args?: Record<string, unknown>;
      status: ToolStatus;
      result?: string;
      artifacts?: Array<{ path: string; kind: string }>;
      /** Confirm 工具的确认状态：内嵌在工具卡里，不再单出一张权限卡 */
      perm?: { state: PermState; summary: string };
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

/** Envelope → Entry 归并（纯函数，返回新数组） */
export function applyEnvelope(prev: Entry[], env: AgentEnvelope): Entry[] {
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
      // 结束流式中的助手条目
      const closed = prev.map((e) =>
        e.kind === 'assistant' && e.streaming ? { ...e, streaming: false } : e,
      );
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
      return [...prev, { kind: 'terminal', outcome: 'completed', text: '上下文已压缩' }];
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

// ===== Store =====

interface AgentState {
  sessionId: string;
  entries: Entry[];
  input: string;
  running: boolean;
  status: string | null;
  error: string;
  /** 项目绑定（空 = 纯聊天）；顶部工具栏选择，@ 只负责引用文件 */
  projectPath: string;
  /** 历史会话列表（save/list/delete 后经 refreshSessions 刷新） */
  sessions: AgentSessionMeta[];
  /** 一键允许项目内 shell（ confined 在项目根的 shell_execute 跳过确认） */
  autoShell: boolean;
  /** Agent 最大续轮数（防无限工具循环预算，默认 50） */
  maxTurns: number;
  /** 上下文自动压缩阈值（估算 token 数，默认 60000，0 = 关闭） */
  autoCompactTokens: number;

  setInput: (v: string) => void;
  setError: (v: string) => void;
  setProjectPath: (v: string) => void;
  /** 切换项目内 shell 自动允许（同步到后端 + localStorage 持久化） */
  setAutoShell: (v: boolean) => void;
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
  /** 从历史面板恢复会话（只读回放 + 续聊重新建 Thread） */
  loadSession: (s: AgentSessionMeta) => void;
  /** 保存会话到后端并刷新列表（默认保存当前会话；空条目跳过） */
  saveSession: (id?: string) => Promise<void>;
  refreshSessions: () => Promise<void>;
}

export const useAgentStore = create<AgentState>((set, get) => ({
  sessionId: newSessionId(),
  entries: [],
  input: '',
  running: false,
  status: null,
  error: '',
  projectPath: '',
  sessions: [],
  autoShell: typeof localStorage !== 'undefined' && localStorage.getItem('astra-agent-autoshell') === '1',
  maxTurns: Number(localStorage?.getItem('astra-agent-maxturns')) || 50,
  autoCompactTokens: parseAutoCompact(localStorage?.getItem('astra-agent-autocompact')),

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
  setError: (v) => set({ error: v }),
  setProjectPath: (v) => set({ projectPath: v }),
  setAutoShell: (v) => {
    set({ autoShell: v });
    localStorage.setItem('astra-agent-autoshell', v ? '1' : '0');
    const { projectPath } = get();
    invoke('agent_set_auto_shell', {
      enabled: v,
      projectRoot: v && projectPath ? projectPath : null,
    }).catch(() => {
      // Tauri 不可用（纯 web 调试）时仅保留前端状态
    });
  },
  setRunState: (running, status) => set({ running, status }),
  pushUser: (text) => set((s) => ({ entries: [...s.entries, { kind: 'user', text }] })),
  pushTerminal: (outcome, text) =>
    set((s) => ({ entries: [...s.entries, { kind: 'terminal', outcome, text }] })),
  pushQuestion: (q) =>
    set((s) =>
      // 幂等防御：同一 questionId 只入列一次（防事件层重复投递）
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
      set((s) => ({
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
    set({ sessionId: newSessionId(), entries: [], status: null, error: '', input: '' }),
  loadSession: (s) =>
    set({
      sessionId: s.id,
      entries: (s.entries as Entry[]) ?? [],
      projectPath: s.project_path ?? '',
      status: null,
      error: '',
    }),
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
      console.error('保存会话失败', e);
    }
  },
}));

// ===== 模块级单例事件监听 =====

let listenersAttached = false;

/**
 * 挂载 agent-event 监听（幂等）。必须在页面组件 mount 时调用一次；
 * 页面卸载后监听不拆除，事件持续写入 store，实现「切页对话不丢」。
 */
export function ensureAgentListeners(): void {
  if (listenersAttached) return;
  listenersAttached = true;

  // ask_user 提问事件（独立于 Envelope 流，载荷自带 session_id 过滤旧会话）。
  // 必须与 agent-event 同级注册一次，不能嵌套在回调里（否则每条事件都叠加一个监听器）。
  listen<AgentQuestionPayload>('agent-question', (e) => {
    const q = e.payload;
    if (q.session_id !== useAgentStore.getState().sessionId) return;
    useAgentStore.getState().pushQuestion(q);
  }).catch(() => {});

  listen<AgentEnvelope>('agent-event', (e) => {
    const env = e.payload;
    const st = useAgentStore.getState();
    if (env.session_id !== st.sessionId) return; // 旧会话残余事件

    if (env.type === 'status_changed') {
      useAgentStore.setState({ status: env.status ?? null });
      return;
    }

    if (
      env.type === 'run_completed' ||
      env.type === 'run_failed' ||
      env.type === 'run_cancelled'
    ) {
      // 先归并终态条目（setState 同步生效），再收运行标志、落盘
      useAgentStore.setState((s) => ({ entries: applyEnvelope(s.entries, env) }));
      useAgentStore.setState({ running: false, status: null });
      void st.saveSession();
      return;
    }

    useAgentStore.setState((s) => ({ entries: applyEnvelope(s.entries, env) }));
  }).catch(() => {
    // Tauri 不可用（纯 web 调试）时允许下次重试
    listenersAttached = false;
  });
}
