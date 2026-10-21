/**
 * Agent 对话页（阶段 3：Zed 式 Agent 交互 MVP）
 *
 * 与 chat 页的关系：
 * - 复用 chatToolbar store（标题栏选择 provider / 本地模型对本页同样生效）
 * - 复用 workspace store 的项目列表；选择项目 → 后端注册文件工具（读=自动，写=确认卡）
 * - 事件流：Tauri `agent-event`（Envelope：type 标签 + snake_case 载荷）
 * - 权限确认卡：批准/拒绝 → agent_permission_decide，UI 状态由 permission_decided 事件回写
 */
import { forwardRef, memo, useCallback, useEffect, useMemo, useRef, useState, type HTMLAttributes } from 'react';
import { Virtuoso, type ListRange, type VirtuosoHandle } from 'react-virtuoso';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import {
  ArrowRight,
  AtSign,
  Brain,
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleX,
  FileText,
  FolderOpen,
  HelpCircle,
  Loader2,
  MonitorUp,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  RotateCcw,
  Copy,
  Sparkles,
  Trash,
  Wrench,
  X,
} from 'lucide-react';
import OrbArt from '@/components/illustrations/OrbArt';

import { useChatToolbarStore } from '@/stores/chatToolbar';
import {
  useAgentStore,
  ensureAgentListeners,
  refreshAgentStatus,
  nextQueuedFor,
  THINKING_EFFORTS,
  type AgentUsage,
  type Entry,
  type QueuedMessage,
  type ThinkingEffort,
  type UserImage,
  userImageUrl,
  withId,
} from '@/stores/agent';
import { useWorkspaceStore } from '@/stores/workspace';
import { isSendEnter } from '@/hooks/useMention';

/** 运行中状态行的哨兵条目：占据列表最后一项（见 AgentPage 内 items 处注释） */
const STATUS_ITEM = Symbol('statusline');
type ListItem = Entry | typeof STATUS_ITEM;
import ModelPicker from '@/components/agent/ModelPicker';
import MarkdownMessage from './chat/MarkdownMessage';
import MentionInput, { type MentionInputHandle } from './chat/MentionInput';
import SettingsSheet from './chat/SettingsSheet';
import { parseSlashCommand, type UserSkillLite } from '@/hooks/useSlash';
import AgentHistoryPanel, { type AgentSessionMeta } from './AgentHistoryPanel';
import AgentTabs from './AgentTabs';
import LinkPreviewPanel from './LinkPreviewPanel';
import { useLinkPreviewStore } from '@/stores/linkPreview';
import { StreamBlock } from '@/components/agent/StreamBlock';
import { ArtifactList } from '@/components/agent/ArtifactList';
import './AgentPage.scss';

// 事件模型与条目类型见 stores/agent.ts（全局会话状态的唯一事实来源）

const STATUS_LABEL: Record<string, string> = {
  analyzing: '分析中',
  executing_tool: '执行工具',
  awaiting_permission: '等待确认',
  finalizing: '收尾中',
  compacting: '压缩上下文',
  persisting: '保存上下文',
};

// 权限确认卡文案按工具区分（未收录的工具用兜底文案）
const PERM_ACTION_LABEL: Record<string, string> = {
  write_file: '请求写入文件',
  edit_file: '请求修改文件',
  copy_path: '请求复制文件/目录',
  move_path: '请求移动/重命名',
  create_directory: '请求创建目录',
  shell_execute: '请求执行命令',
  shell_background: '请求后台启动进程',
  shell_kill: '请求停止后台进程',
  build_docx: '请求生成 docx 文档',
  extract_document: '请求转换文档为 Markdown',
  fetch: '请求抓取网页',
  read_file: '请求读取文件',
  list_files: '请求列出目录',
};

/** token 数缩写（1234 → 1.2k，1500000 → 1.5M），用于无上限时的用量展示 */
function humanizeTokens(n: number): string {
  if (n >= 1_000_000) {
    const v = n / 1_000_000;
    return `${Number.isInteger(v) ? v : v.toFixed(1)}M`;
  }
  if (n >= 1_000) {
    const v = n / 1_000;
    return `${Number.isInteger(v) ? v : v.toFixed(1)}k`;
  }
  return String(n);
}

/** 后端 ChatMessage JSON → 前端条目（多模态 M1：用户消息中的 image 块一并还原）。
 *  原 checkpoint 恢复入口已移除，保留供「编辑重发」等历史回退能力复用 */
function chatMessageToEntries(messages: Array<Record<string, unknown>>): Entry[] {
  const entries: Entry[] = [];
  for (const message of messages) {
    const role = message['role'];
    const content = message['content'];
    if (!Array.isArray(content)) continue;
    const textParts: string[] = [];
    const images: UserImage[] = [];
    for (const block of content) {
      if (!block || typeof block !== 'object') continue;
      const b = block as Record<string, unknown>;
      if (typeof b['text'] === 'string') {
        textParts.push(b['text']);
      } else if (b['type'] === 'image' && b['image'] && typeof b['image'] === 'object') {
        // MessageContent::Image 的 JSON 形状：{ type:'image', image:{ data, width, height } }
        const img = b['image'] as Record<string, unknown>;
        if (typeof img['data'] === 'string' && typeof img['width'] === 'number' && typeof img['height'] === 'number') {
          images.push({ data: img['data'], width: img['width'], height: img['height'] });
        }
      }
    }
    const text = textParts.join('');
    if (role === 'user') {
      if (!text && images.length === 0) continue;
      entries.push(
        withId(images.length > 0 ? { kind: 'user' as const, text, images } : { kind: 'user' as const, text }),
      );
    } else if (role === 'assistant' && text) {
      entries.push(withId({ kind: 'assistant' as const, text, streaming: false }));
    }
  }
  return entries;
}

const EFFORT_LABEL: Record<ThinkingEffort, string> = {
  minimal: '最省',
  low: '低',
  medium: '中',
  high: '高',
};

/**
 * 思考模式 SplitButton（Zed 式）：主按钮切换开关（开 = accent 高亮），
 * 右侧小 chevron 展开力度菜单。菜单复用现有 popup 样式模式（绝对定位 + 外点关闭）。
 */
function ThinkingButton(props: {
  enabled: boolean;
  effort: ThinkingEffort;
  onToggle: () => void;
  onEffortChange: (e: ThinkingEffort) => void;
}) {
  const { enabled, effort, onToggle, onEffortChange } = props;
  const [menuOpen, setMenuOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // 菜单打开时监听外点关闭（capture 不需要，pointerdown 先于 click 即可）
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [menuOpen]);

  return (
    <div ref={rootRef} className={`agent-dock__thinking${enabled ? ' is-on' : ''}`}>
      <button
        type="button"
        className="agent-dock__thinking-main"
        onClick={onToggle}
        aria-pressed={enabled}
        title={`思考模式：${enabled ? '开' : '关'} · 力度 ${EFFORT_LABEL[effort]}（模型/网关需支持思考，不支持时请求报错）`}
      >
        <Brain size={14} strokeWidth={1.8} />
      </button>
      <button
        type="button"
        className="agent-dock__thinking-chev"
        onClick={() => setMenuOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label="选择思考力度"
        title={`思考力度：${EFFORT_LABEL[effort]}`}
      >
        <ChevronDown size={11} strokeWidth={2.4} className={menuOpen ? 'agent-dock__thinking-chev-up' : ''} />
      </button>
      {menuOpen && (
        <div className="agent-dock__effort-menu" role="menu">
          {THINKING_EFFORTS.map((e) => (
            <button
              key={e}
              type="button"
              role="menuitemradio"
              aria-checked={effort === e}
              onClick={() => {
                onEffortChange(e);
                setMenuOpen(false);
              }}
            >
              <span>{EFFORT_LABEL[e]}</span>
              {effort === e && <Check size={13} strokeWidth={2.5} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * 上下文用量指示：有 max 显示环形进度 + 百分比（≥85% 警示色）；
 * 无 max 有 total 显示缩写；usage 为 null 不渲染。tooltip 给 input/output/total 明细。
 */
function ContextUsage({ usage }: { usage: AgentUsage | null }) {
  if (!usage) return null;
  const { input, output, total, max } = usage;
  const used = total ?? (input != null && output != null ? input + output : undefined);
  const pct = max != null && max > 0 && used != null ? Math.min(used / max, 1) : null;
  if (pct == null && total == null) return null;
  const detail = `输入 ${input ?? '—'} · 输出 ${output ?? '—'} · 合计 ${total ?? '—'}${max != null ? ` · 上限 ${max}` : ''}`;
  const hot = pct != null && pct >= 0.85;
  const radius = 6;
  const circumference = 2 * Math.PI * radius;
  return (
    <span className={`agent-dock__usage${hot ? ' is-hot' : ''}`} title={detail}>
      {pct != null ? (
        <>
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <circle className="agent-dock__usage-track" cx="8" cy="8" r={radius} />
            <circle
              className="agent-dock__usage-arc"
              cx="8"
              cy="8"
              r={radius}
              strokeDasharray={`${circumference * pct} ${circumference}`}
            />
          </svg>
          <span className="agent-dock__usage-pct">{Math.round(pct * 100)}%</span>
        </>
      ) : (
        <span className="agent-dock__usage-total">{humanizeTokens(total!)}</span>
      )}
    </span>
  );
}

export default function AgentPage() {
  // ===== 全局会话状态（zustand；切页不丢，事件由模块级单例监听持续写入） =====
  const sessionId = useAgentStore((s) => s.sessionId);
  const entries = useAgentStore((s) => s.entries);
  const input = useAgentStore((s) => s.input);
  const setInput = useAgentStore((s) => s.setInput);
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const settling = useAgentStore((s) => s.settling || s.sessionBusy);
  const status = useAgentStore((s) => s.status);
  const error = useAgentStore((s) => s.error);
  const setError = useAgentStore((s) => s.setError);
  const projectPath = useAgentStore((s) => s.projectPath);
  const openDraftForProject = useAgentStore((s) => s.openDraftForProject);
  const sessions = useAgentStore((s) => s.sessions);
  const refreshSessions = useAgentStore((s) => s.refreshSessions);
  const usage = useAgentStore((s) => s.usage);
  const thinkingEnabled = useAgentStore((s) => s.thinkingEnabled);
  const thinkingEffort = useAgentStore((s) => s.thinkingEffort);
  const setThinkingEnabled = useAgentStore((s) => s.setThinkingEnabled);
  const setThinkingEffort = useAgentStore((s) => s.setThinkingEffort);
  const imageRecognition = useAgentStore((s) => s.imageRecognition);

  const pushUser = useAgentStore((s) => s.pushUser);
  const pushTerminal = useAgentStore((s) => s.pushTerminal);
  const setRunState = useAgentStore((s) => s.setRunState);
  const queue = useAgentStore((s) => s.queue);
  const enqueueMessage = useAgentStore((s) => s.enqueueMessage);
  const removeQueued = useAgentStore((s) => s.removeQueued);
  /** 当前会话的排队消息（队列全局共享，切 tab 回来仍可见） */
  const queuedHere = useMemo(() => queue.filter((q) => q.sessionId === sessionId), [queue, sessionId]);

  // 事件监听为模块级单例：首次进入本页时挂载，卸载后不拆除（切页对话继续）
  useEffect(() => {
    void ensureAgentListeners().then(() => refreshAgentStatus()).catch((error) => setError(`事件监听失败：${String(error)}`));
  }, [setError]);

  // 应用重启后同步放权级别到后端（切换时 setAutoMode 已实时同步）
  useEffect(() => {
    invoke('agent_set_auto_mode', { mode: useAgentStore.getState().autoMode }).catch(() => {});
  }, []);

  // ===== 项目选择（绑定在 store；顶部工具栏选择文件夹，@ 只负责引用文件） =====
  const projects = useWorkspaceStore((s) => s.projects);
  const editor = useWorkspaceStore((s) => s.editor);

  // ===== 项目文件列表（@ 引用候选；随项目切换重新加载） =====
  const [files, setFiles] = useState<string[]>([]);
  useEffect(() => {
    if (!projectPath) {
      setFiles([]);
      return;
    }
    let cancelled = false;
    invoke<string[]>('list_project_files', { path: projectPath })
      .then((list) => {
        if (!cancelled) setFiles(list);
      })
      .catch(() => {
        // 扫描失败（无权限/目录消失）时回退为空候选，不阻塞输入
        if (!cancelled) setFiles([]);
      });
    return () => {
      cancelled = true;
    };
  }, [projectPath]);

  // ===== / 技能调用（用户技能库；/技能名 指定技能，后端注入技能内容） =====
  const [skills, setSkills] = useState<UserSkillLite[]>([]);
  useEffect(() => {
    invoke<UserSkillLite[]>('list_user_skills')
      .then((list) =>
        setSkills(list.map((s) => ({ name: s.name, description: s.description }))),
      )
      .catch(() => {
        // 后端不可用（本地调试）时忽略
      });
  }, []);

  // 气泡内 @文件 chip 解析用的文件集合（files 变化时重建）
  const fileSet = useMemo(() => new Set(files), [files]);

  // 输入框（contenteditable chip 编辑器，@/ 弹层与 tag 渲染均内聚在组件内）
  const composerRef = useRef<MentionInputHandle | null>(null);

  // ===== 粘贴落盘（SpillPaste）：超长粘贴改写为文件，发送时让 Agent 先读全文 =====
  // 仅页面级状态（切 tab 换会话时重置），不进全局 store；有项目时写在
  // <project>/.astra/pastes/，纯聊天时写在 ~/.astra/pastes/（后者的文件工具
  // 以主目录为 root，read_file 可直接读），均 Allow 级可读
  const [spill, setSpill] = useState<{ path: string; size: number } | null>(null);
  useEffect(() => {
    setSpill(null);
  }, [sessionId]);

  const handleSpillPaste = useCallback(
    async (content: string): Promise<boolean> => {
      try {
        const res = await invoke<{ spilled: boolean; path: string | null; size: number }>(
          'agent_spill_paste',
          { projectRoot: projectPath || null, content },
        );
        if (!res.spilled || !res.path) return false;
        setSpill({ path: res.path, size: res.size });
        return true;
      } catch (e) {
        setError(`粘贴内容落盘失败：${String(e)}`);
        return false;
      }
    },
    [projectPath, setError],
  );

  /** 发送前组装：有暂存粘贴时把正文换成「先读文件再处理指令」，并取走暂存。
   *  displaySuffix 追加到会话气泡展示文本，保留粘贴的可见痕迹 */
  const takeSpill = useCallback(
    (text: string): { text: string; displaySuffix: string } => {
      if (!spill) return { text, displaySuffix: '' };
      const kb = Math.max(1, Math.round(spill.size / 1024));
      const name = spill.path.split(/[\\/]/).pop() ?? spill.path;
      setSpill(null);
      return {
        text: `【用户粘贴的超长内容（${kb} KB）已保存为文件：${spill.path}\n请先用 read_file 工具读取该文件全文，再处理下方指令】\n\n${text}`,
        displaySuffix: `\n📎 超长粘贴已存为 ${name}（${kb} KB），已让 Agent 先读取全文再处理`,
      };
    },
    [spill],
  );

  // ===== 历史面板开关（持久化，重启后保持上次的开合状态） =====
  const [historyOpen, setHistoryOpen] = useState(() => localStorage.getItem('astra.history.open') === '1');

  // 链接预览面板（agent 回复中的链接拦截后在此用 iframe 打开）
  const linkPreviewOpen = useLinkPreviewStore((s) => s.open);
  const toggleHistory = useCallback(() => {
    setHistoryOpen((o) => {
      localStorage.setItem('astra.history.open', o ? '0' : '1');
      return !o;
    });
  }, []);

  useEffect(() => {
    void refreshSessions();
  }, [refreshSessions]);

  // ===== 模型来源（复用标题栏 toolbar 的选择） =====
  const sourceType = useChatToolbarStore((s) => s.sourceType);
  const selectedProviderId = useChatToolbarStore((s) => s.selectedProviderId);
  const selectedModelPort = useChatToolbarStore((s) => s.selectedModelPort);
  const overrideModelName = useChatToolbarStore((s) => s.overrideModelName);
  const providers = useChatToolbarStore((s) => s.providers);
  const runningModels = useChatToolbarStore((s) => s.runningModels);
  const showSettings = useChatToolbarStore((s) => s.showSettings);

  // ===== 参数（SettingsSheet；system_prompt 传给 agent_start_run，温度/token 后端暂不支持已移除） =====
  const [systemPrompt, setSystemPrompt] = useState('');

  const selectedProvider = providers.find((p) => p.id === selectedProviderId) ?? null;
  const selectedModel = runningModels.find((m) => m.port === selectedModelPort) ?? null;
  const modelName =
    sourceType === 'model'
      ? (selectedModel?.name ?? 'local')
      : (overrideModelName ?? selectedProvider?.active_model ?? null);

  // P3 并发运行：每个会话独立跑 Run，其他 tab 的运行不再阻塞本会话发送
  const canSend =
    !running &&
    input.trim().length > 0 &&
    (sourceType === 'model' ? !!selectedModelPort : !!selectedProviderId && !!modelName);

  // ===== 事件订阅：见 stores/agent.ts 的模块级单例（ensureAgentListeners） =====

  // ===== 滚动（react-virtuoso 虚拟化）：只渲染视口内条目；followOutput 承担粘底跟随，
  // 上滚离开底部自动暂停、滚回底部自动恢复（Zed 列表 Tail 语义），取代手写 outputScroll =====
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const [follow, setFollow] = useState(true);

  // 状态行哨兵：运行中追加在列表末尾，作为普通数据条目参与虚拟化。
  // 不用 components.Footer —— followOutput 的粘底对齐只针对最后一条数据条目
  // （内部 scrollToIndex({ index: 'LAST', align: 'end' })），Footer 会留在
  // 可视区外被裁掉，滚动条也到不了真正的底部
  const items = useMemo<ListItem[]>
    (() => (running ? [...entries, STATUS_ITEM] : entries), [entries, running]);

  // ===== 右侧圆点导轨：用户消息快速定位 =====
  const userMsgIdx = useMemo(
    () => entries.map((e, i) => (e.kind === 'user' ? i : -1)).filter((i) => i >= 0),
    [entries],
  );

  /**
   * 每条助手消息对应的「本轮完整回复」：一个 Run 里助手输出会被工具调用/思考块
   * 切成多段 assistant 条目，复制时按 Run（相邻两条用户消息之间）聚合所有段。
   * 仅多段时写入 map；单段直接用 entry.text 即可。
   */
  const fullRunTexts = useMemo(() => {
    const map = new Map<number, string>();
    let runStart = 0;
    const flush = (end: number) => {
      const texts: string[] = [];
      for (let i = runStart; i < end; i++) {
        const e = entries[i];
        if (e.kind === 'assistant' && e.text.trim()) texts.push(e.text);
      }
      if (texts.length > 1) {
        const joined = texts.join('\n\n');
        for (let i = runStart; i < end; i++) {
          if (entries[i].kind === 'assistant') map.set(i, joined);
        }
      }
      runStart = end + 1;
    };
    for (let i = 0; i < entries.length; i++) {
      if (entries[i].kind === 'user') flush(i);
    }
    flush(entries.length);
    return map;
  }, [entries]);
  const [activeDot, setActiveDot] = useState(-1);

  /** 可见范围变化时计算活动提问：视口顶部之上最近的用户消息（未过任何提问时为 -1）。
   *  虚拟化后视口外条目不在 DOM，改用 Virtuoso 的 rangeChanged，不再测 offsetTop */
  const handleRangeChanged = useCallback(
    (range: ListRange) => {
      let active = -1;
      for (let n = 0; n < userMsgIdx.length; n++) {
        if (userMsgIdx[n] <= range.startIndex) active = n;
        else break;
      }
      setActiveDot(active);
    },
    [userMsgIdx],
  );

  /** 点击圆点：脱离粘底跟随并定位到对应用户消息（scrollToIndex 取代 DOM 查询） */
  const jumpToEntry = useCallback((idx: number) => {
    setFollow(false);
    virtuosoRef.current?.scrollToIndex({ index: idx, align: 'start', behavior: 'smooth' });
  }, []);

  // 会话切换：重置粘底跟随（Virtuoso 由 initialTopMostItemIndex 定位到末尾）
  useEffect(() => {
    setFollow(true);
  }, [sessionId]);

  /** /技能名 前缀解析：命中已知技能 → 剥离前缀；/xxx 非法 → 提示并返回 null */
  const parseSendInput = useCallback(
    (raw: string): { text: string; skill: string | null } | null => {
      if (!raw.startsWith('/')) return { text: raw, skill: null };
      const parsed = parseSlashCommand(raw, skills);
      if (parsed) {
        if (!parsed.rest) {
          setError('请输入任务内容，例如 /demo 把需求整理成方案');
          return null;
        }
        return { text: parsed.rest, skill: parsed.skill };
      }
      const name = raw.match(/^\/([\w\u4e00-\u9fff.-]+)/)?.[1] ?? '';
      setError(`未知技能 /${name}（可在 设置 → 我的 Skills 页面创建或导入）`);
      return null;
    },
    [skills, setError],
  );

  // ===== 编辑重发（回退是延迟的：确认时只记意图，发送时才真正回退，取消零损失） =====
  // turnsBack：目标用户消息是倒数第几条（1 = 最后一条）
  const [editing, setEditing] = useState<{ turnsBack: number; original: string } | null>(null);
  // 会话切换即放弃编辑意图（输入框内容随会话驻留，不会误发到别的会话）
  useEffect(() => {
    setEditing(null);
  }, [sessionId]);

  /** 回退到倒数第 N 条用户消息之前并重建会话视图（后端截断 thread + 落盘） */
  const rewindToUserTurn = useCallback(
    async (turnsBack: number, restoreWorkspace: boolean) => {
      const result = await invoke<{
        thread_seq: number;
        workspace_restored: boolean;
        messages: Array<Record<string, unknown>>;
      }>('agent_rewind_to_user_message', { sessionId, turnsBack, restoreWorkspace });
      useAgentStore.getState().restoreEntries(sessionId, chatMessageToEntries(result.messages));
      void useAgentStore.getState().saveSession(sessionId);
      return result;
    },
    [sessionId],
  );

  /** 实际发送（队列续发与手动发送共用）：onAdmitted 在全部守卫通过、
   *  即将开跑时调用 —— 队列路径借此保证「先准入后出队」，守卫拦截时消息不丢 */
  const doSend = useCallback(
    async (raw: string, skillName: string | null, text: string, onAdmitted?: () => void) => {
      const st = useChatToolbarStore.getState();
      if (st.sourceType === 'model' && !st.selectedModelPort) {
        setError('请先在标题栏选择一个运行中的本地模型');
        return;
      }
      if (st.sourceType === 'provider') {
        const provider = st.providers.find((p) => p.id === st.selectedProviderId) ?? null;
        const name = st.overrideModelName ?? provider?.active_model ?? null;
        if (!provider) {
          setError('请先在输入框下方选择一个 API 提供者');
          return;
        }
        if (!name) {
          setError('请先在输入框下方选择模型');
          return;
        }
      }

      try {
        await ensureAgentListeners();
      } catch (error) {
        setError(`事件监听失败：${String(error)}`);
        return;
      }
      if (useAgentStore.getState().running || useAgentStore.getState().sessionBusy || useAgentStore.getState().sessionId !== sessionId) return;
      // 编辑重发：真正发送前才执行回退（取消编辑则什么都不丢）。失败时不发送、
      // 保留编辑意图，用户可直接重试
      if (editing) {
        try {
          await rewindToUserTurn(editing.turnsBack, false);
        } catch (e) {
          setError(`回退失败，消息未发送：${String(e)}`);
          return;
        }
        setEditing(null);
      }
      setFollow(true);
      setInput('');
      setError('');
      onAdmitted?.();
      pushUser(raw); // 会话记录保留原始 /技能名 文本，回放时可读
      setRunState(true, 'analyzing');

      try {
        await invoke('agent_start_run', {
          req: {
            session_id: sessionId,
            user_text: text,
            images: [],
            skill: skillName,
            provider_id: st.sourceType === 'provider' ? st.selectedProviderId : null,
            model_port: st.sourceType === 'model' ? st.selectedModelPort : null,
            model:
              st.sourceType === 'provider'
                ? (st.overrideModelName ?? selectedProvider?.active_model ?? null)
                : null,
            project_root: projectPath || null,
            system_prompt: systemPrompt.trim() || null,
            max_turns: useAgentStore.getState().maxTurns,
            auto_compact_tokens: useAgentStore.getState().autoCompactTokens,
            image_recognition: useAgentStore.getState().imageRecognition,
            thinking_enabled: useAgentStore.getState().thinkingEnabled,
            thinking_effort: useAgentStore.getState().thinkingEffort,
          },
        });
      } catch (e) {
        setRunState(false, null);
        pushTerminal('failed', String(e));
      }
    },
    [projectPath, selectedProvider, systemPrompt, sessionId, setInput, setError, pushUser, pushTerminal, setRunState, setFollow, editing, rewindToUserTurn],
  );

  const handleSend = useCallback(async () => {
    const raw = input.trim();
    if (!raw || running) return;
    // 有文本时照常走 /技能名 解析
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    const spilled = takeSpill(parsed.text);
    await doSend(raw + spilled.displaySuffix, parsed.skill, spilled.text);
  }, [input, running, parseSendInput, doSend, takeSpill]);

  /** 失败卡片「重试 / 编辑后重发」：回退到出错前（含工作区还原）后重发或回填。
   *  找不到快照时降级为仅回退对话（非 git 项目 / 本功能上线前的会话） */
  const handleFailureRewind = useCallback(
    async (mode: 'retry' | 'edit') => {
      if (running || settling) return;
      const lastUser = [...entries].reverse().find(
        (e): e is Extract<Entry, { kind: 'user' }> => e.kind === 'user',
      );
      if (!lastUser) return;
      setEditing(null);
      try {
        const result = await rewindToUserTurn(1, true);
        if (!result.workspace_restored) {
          setError('已回退对话（项目文件未还原：找不到该轮的快照）');
        }
        if (mode === 'edit') {
          setInput(lastUser.text);
          requestAnimationFrame(() => composerRef.current?.focusEnd());
        } else {
          const parsed = parseSendInput(lastUser.text);
          if (parsed) await doSend(lastUser.text, parsed.skill, parsed.text);
        }
      } catch (e) {
        setError(`回退失败：${String(e)}`);
      }
    },
    [running, settling, sessionId, entries, rewindToUserTurn, setError, setInput, parseSendInput, doSend],
  );

  /** 历史用户消息「编辑重发」：只记回退意图并回填输入框（真正回退发生在发送时，
   *  取消零损失）。其后有值得保留的内容（成功回复 / 完成终态）时弹确认 */
  const handleEditUserMessage = useCallback(
    (idx: number) => {
      if (running || settling) return;
      const pos = userMsgIdx.indexOf(idx);
      const entry = entries[idx];
      if (pos < 0 || entry.kind !== 'user') return;
      const turnsBack = userMsgIdx.length - pos;
      const dropsContent = entries
        .slice(idx + 1)
        .some(
          (e) =>
            (e.kind === 'assistant' && e.text.trim()) ||
            (e.kind === 'terminal' && e.outcome === 'completed'),
        );
      if (dropsContent) {
        const turns = turnsBack - 1;
        const ok = window.confirm(
          turns > 0
            ? `编辑此消息将丢弃其后 ${turns} 轮对话（项目文件不会还原）。继续？`
            : '将回退本轮对话后重新发送（项目文件不会还原）。继续？',
        );
        if (!ok) return;
      }
      setEditing({ turnsBack, original: entry.text });
      setInput(entry.text);
      requestAnimationFrame(() => composerRef.current?.focusEnd());
    },
    [running, settling, sessionId, userMsgIdx, entries, setInput],
  );

  /** 取消编辑：只清意图与输入框，对话不做任何回退 */
  const cancelEditing = useCallback(() => {
    setEditing(null);
    setInput('');
  }, [setInput]);

  /** Run 进行中把输入排队（回答结束后由队列续发 effect 自动发出） */
  const handleQueueSend = useCallback(() => {
    const raw = input.trim();
    if (!raw) return;
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    setEditing(null); // 排队新消息 = 放弃挂起的编辑意图（不回退）
    const spilled = takeSpill(parsed.text);
    enqueueMessage(raw + spilled.displaySuffix, spilled.text, parsed.skill);
    setInput('');
  }, [input, parseSendInput, enqueueMessage, setInput, takeSpill]);

  /** 打断并优先发送：当前 Run 取消 + 本条插到会话队首，settle 后立即续发。
   *  别的会话占用后端时不能打断（cancel 是全局单活动 Run），退化为普通排队 */
  const handleInterruptSend = useCallback(async () => {
    const raw = input.trim();
    if (!raw) return;
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    setEditing(null); // 插队新消息 = 放弃挂起的编辑意图（不回退）
    const st = useAgentStore.getState();
    const interruptible = st.running && !st.settling;
    const spilled = takeSpill(parsed.text);
    enqueueMessage(raw + spilled.displaySuffix, spilled.text, parsed.skill, { front: interruptible });
    setInput('');
    if (interruptible) {
      try {
        await invoke('agent_cancel', { sessionId });
      } catch (error) {
        setError(`停止失败：${String(error)}`);
      }
    }
  }, [input, parseSendInput, enqueueMessage, setInput, setError, sessionId, takeSpill]);

  /** 编辑排队消息（Zed move_queued_message_to_main_editor 语义）：
   *  从队列移除，原文回填主输入框并聚焦到末尾，改完再自行发送 */
  const editQueued = useCallback((q: QueuedMessage) => {
    removeQueued(q.id);
    setInput(q.raw);
    requestAnimationFrame(() => {
      composerRef.current?.focusEnd();
    });
  }, [removeQueued, setInput]);

  /** 立即发送指定排队消息（Zed send_queued_message_now 语义）：
   *  本会话空闲时直接发；本会话运行中则取消当前 Run 后立即发（消息插队首，
   *  settled 后由续发 effect 发出）。其他会话的运行与本会话无关（P3 并发） */
  const sendQueuedNow = useCallback(async (q: QueuedMessage) => {
    const st = useAgentStore.getState();
    const interruptible = st.running && !st.settling;
    setEditing(null); // 手动发队首 = 放弃挂起的编辑意图（不回退）
    removeQueued(q.id);
    if (interruptible) {
      enqueueMessage(q.raw, q.text, q.skill, { front: true });
      try {
        await invoke('agent_cancel', { sessionId: q.sessionId });
      } catch (error) {
        setError(`停止失败：${String(error)}`);
      }
    } else if (!st.running && !st.settling && !st.sessionBusy) {
      void doSend(q.raw, q.skill, q.text);
    } else {
      enqueueMessage(q.raw, q.text, q.skill, { front: true });
    }
  }, [removeQueued, enqueueMessage, doSend, setError]);

  // ===== 队列续发：Run 结束后自动发出当前会话的队首消息。
  //  上一条以 failed 终态收尾时暂停自动续发（防模型配置错误时连环烧完队列），
  //  用户手动发送成功一次后恢复。其他会话的运行与本会话无关（P3 并发），
  //  不再阻塞续发。
  const lastEntry = entries[entries.length - 1];
  const lastFailed = lastEntry?.kind === 'terminal' && lastEntry.outcome === 'failed';
  const doSendRef = useRef(doSend);
  doSendRef.current = doSend;
  useEffect(() => {
    if (running || settling || lastFailed || editing) return;
    const head = nextQueuedFor(useAgentStore.getState().queue, sessionId);
    if (!head) return;
    const timer = setTimeout(() => {
      const st = useAgentStore.getState();
      if (st.running || st.settling || st.sessionBusy) return;
      if (st.queue.find((q) => q.id === head.id) == null) return; // 排队期间被删除
      st.removeQueued(head.id);
      void doSendRef.current(head.raw, head.skill, head.text);
    }, 120);
    return () => clearTimeout(timer);
  }, [running, settling, queue, sessionId, lastFailed, editing]);

  const handleStop = useCallback(async () => {
    if (useAgentStore.getState().settling || useAgentStore.getState().sessionBusy) return;
    try {
      // 只取消本会话的 Run（P3 并发：其他 tab 的运行不受影响）。
      // 启动准备阶段（checkpoint / 存储写入）尚未登记 active 时取消是空操作：
      // 明确告知而不是静默失败，让用户知道为什么没有反应。
      const cancelled = await invoke<boolean>('agent_cancel', { sessionId });
      if (!cancelled) setError('没有可停止的运行中任务（若刚发送，仍在准备阶段，请稍候）');
    } catch (error) {
      setError(`停止失败：${String(error)}`);
    }
    // Keep admission occupied until agent-run-settled, including cancellation drain.
  }, [setError, sessionId]);

  const handleDecide = useCallback(
    async (callId: string, allow: boolean) => {
      try {
        await invoke('agent_permission_decide', {
          toolCallId: callId,
          decision: allow ? 'allow' : 'deny',
        });
      } catch (e) {
        setError(String(e));
      }
    },
    [setError],
  );

  const handleAnswer = useCallback(
    (questionId: string, answer: string | null) => {
      void useAgentStore.getState().answerQuestion(questionId, answer);
    },
    [],
  );

  // ===== 项目目录操作 =====
  const openProjectInFolder = useCallback(async () => {
    if (!projectPath) return;
    try {
      await openPath(projectPath);
    } catch (e) {
      setError(`打开文件夹失败: ${e}`);
    }
  }, [projectPath]);

  const openProjectInEditor = useCallback(() => {
    if (!projectPath) return;
    if (!editor.command) {
      setError('请先在「设置」页面配置默认编辑器');
      return;
    }
    invoke('open_in_editor', { path: projectPath, editorCommand: editor.command }).catch((e) => {
      setError(`打开编辑器失败: ${e}`);
    });
  }, [projectPath, editor]);

  // 未选项目时的入口：系统文件夹选择器（对齐一般编辑器的「打开文件夹」）
  const pickProjectFolder = useCallback(async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: '选择项目文件夹',
      });
      if (picked) {
        openDraftForProject(picked);
        setError('');
      }
    } catch (e) {
      setError(`选择文件夹失败: ${e}`);
    }
  }, []);

  // ===== 标题栏挂载点（AppLayout 在 /agent 路由下渲染的 portal 容器） =====
  const [tbLeft, setTbLeft] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setTbLeft(document.getElementById('agent-titlebar-left'));
  }, []);

  // ===== 项目切换菜单（Zed 式：点项目名弹出最近项目 + 打开文件夹等操作） =====
  const [projMenuOpen, setProjMenuOpen] = useState(false);
  const projRootRef = useRef<HTMLDivElement | null>(null);
  // 未选项目时的「选择项目文件夹」入口：同样支持最近使用文件夹下拉（Zed 式）
  const [pickMenuOpen, setPickMenuOpen] = useState(false);
  const [recentPicks, setRecentPicks] = useState<{ name: string; path: string }[]>([]);
  const pickRootRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!projMenuOpen && !pickMenuOpen) return;
    function onDown(e: MouseEvent) {
      if (!projRootRef.current?.contains(e.target as Node)) setProjMenuOpen(false);
      if (!pickRootRef.current?.contains(e.target as Node)) setPickMenuOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [projMenuOpen, pickMenuOpen]);

  /** 未选项目入口：合并工作空间已知项目 + 最近使用记录，总是弹菜单（无记录时仅剩「选择其他文件夹…」） */
  const openPickMenu = useCallback(() => {
    const known = projects.map((p) => ({ name: p.name, path: p.path }));
    let recents: string[] = [];
    try {
      recents = JSON.parse(localStorage.getItem('astra.recentProjects') ?? '[]');
    } catch {
      recents = [];
    }
    const seen = new Set(known.map((p) => p.path));
    const extras = (Array.isArray(recents) ? recents : [])
      .filter((p) => typeof p === 'string' && p && !seen.has(p))
      .map((p) => ({ name: p.split('/').pop() || p, path: p }));
    setRecentPicks([...known, ...extras]);
    setPickMenuOpen(true);
  }, [projects]);

  /** 新建会话标签：当前会话驻留内存，切回零重建 */
  const newTab = useCallback(() => {
    useAgentStore.getState().newTab();
  }, []);

  /** 恢复页面历史；续聊模型上下文由后端 SQLite 加载。切换时当前会话驻留内存。 */
  const selectSession = useCallback(
    (s: AgentSessionMeta) => {
      const st = useAgentStore.getState();
      if (st.sessionBusy) return;
      if (st.entries.length > 0) void st.saveSession(st.sessionId).catch(() => {});
      st.activateSession(s);
      setError('');
    },
    [setError],
  );

  const archiveSession = useCallback((id: string, archived: boolean) => {
    void useAgentStore.getState().archiveSession(id, archived);
  }, []);

  // ===== 渲染 =====
  return (
    <div className="agent-root">
      {/* 标题栏功能：portal 到 AppLayout 的 /agent 标题栏挂载点
          左侧 = 项目文件夹（编辑器式）；右侧 = 模型显示 + 新对话 + 历史（Zed 式） */}
      {tbLeft &&
        createPortal(
          projectPath ? (
            <div className="agent-tb__project" ref={projRootRef}>
              <button
                type="button"
                className="agent-tb__project-name"
                onClick={() => setProjMenuOpen((o) => !o)}
                title="切换项目"
              >
                <FolderOpen size={14} strokeWidth={1.8} />
                <span className="agent-tb__project-label">
                  {projects.find((p) => p.path === projectPath)?.name ?? projectPath.split('/').pop()}
                </span>
                <ChevronDown size={11} strokeWidth={2.2} className={`agent-tb__project-chevron${projMenuOpen ? ' is-open' : ''}`} />
              </button>

              {projMenuOpen && (
                <div className="agent-tb__proj-menu" role="menu">
                  <div className="agent-tb__proj-list">
                    {/* 当前目录可能不在项目列表（手动选的任意文件夹）：置顶展示 */}
                    {!projects.some((p) => p.path === projectPath) && (
                      <button type="button" role="menuitem" className="agent-tb__proj-item is-active" disabled>
                        <span className="agent-tb__proj-item-name">{projectPath.split('/').pop()}</span>
                        <span className="agent-tb__proj-item-path">{projectPath}</span>
                        <Check size={13} strokeWidth={2.2} />
                      </button>
                    )}
                    {projects.map((p) => (
                      <button
                        key={p.path}
                        type="button"
                        role="menuitem"
                        className={`agent-tb__proj-item${p.path === projectPath ? ' is-active' : ''}`}
                        disabled={running}
                        title={p.path}
                        onClick={() => {
                          openDraftForProject(p.path);
                          setError('');
                          setProjMenuOpen(false);
                        }}
                      >
                        <span className="agent-tb__proj-item-name">{p.name}</span>
                        {p.path === projectPath && <Check size={13} strokeWidth={2.2} />}
                      </button>
                    ))}
                  </div>

                  <div className="agent-tb__proj-sep" />
                  <button
                    type="button"
                    role="menuitem"
                    className="agent-tb__proj-item"
                    onClick={() => {
                      setProjMenuOpen(false);
                      void pickProjectFolder();
                    }}
                  >
                    选择其他文件夹…
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    className="agent-tb__proj-item"
                    onClick={() => {
                      setProjMenuOpen(false);
                      void openProjectInFolder();
                    }}
                  >
                    在 Finder 中打开
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    className="agent-tb__proj-item"
                    onClick={() => {
                      setProjMenuOpen(false);
                      openProjectInEditor();
                    }}
                  >
                    在编辑器中打开
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    className="agent-tb__proj-item is-danger"
                    disabled={running}
                    onClick={() => {
                      setProjMenuOpen(false);
                      openDraftForProject('');
                    }}
                  >
                    解绑项目（转为纯聊天）
                  </button>
                </div>
              )}
            </div>
          ) : (
            <div className="agent-tb__pickwrap" ref={pickRootRef}>
              <button type="button" className="agent-tb__pick" onClick={openPickMenu}>
                <FolderOpen size={14} strokeWidth={1.8} />
                选择项目文件夹
              </button>
              {pickMenuOpen && (
                <div className="agent-tb__proj-menu" role="menu">
                  {recentPicks.length > 0 && (
                    <>
                      <div className="agent-tb__proj-list">
                        {recentPicks.map((p) => (
                          <button
                            key={p.path}
                            type="button"
                            role="menuitem"
                            className="agent-tb__proj-item"
                            title={p.path}
                            onClick={() => {
                              openDraftForProject(p.path);
                              setError('');
                              setPickMenuOpen(false);
                            }}
                          >
                            <span className="agent-tb__proj-item-name">{p.name}</span>
                            <span className="agent-tb__proj-item-path">{p.path}</span>
                          </button>
                        ))}
                      </div>
                      <div className="agent-tb__proj-sep" />
                    </>
                  )}
                  <button
                    type="button"
                    role="menuitem"
                    className="agent-tb__proj-item"
                    onClick={() => {
                      setPickMenuOpen(false);
                      void pickProjectFolder();
                    }}
                  >
                    选择其他文件夹…
                  </button>
                </div>
              )}
            </div>
          ),
          tbLeft,
        )}

      {/* 历史面板：全高左侧栏（Zed 式），与右列并排；开合时右列整体（tab + 对话）一起右移/左移 */}
      {historyOpen && (
        <AgentHistoryPanel
          sessions={sessions}
          currentSessionId={sessionId}
          projectPath={projectPath}
          onSelect={(s) => selectSession(s)}
          onArchive={archiveSession}
          onNewChat={newTab}
        />
      )}

      {/* 右列：标签栏 + 错误提示 + 对话流 */}
      <div className="agent-col">
        {/* 多会话标签栏（tab 即 AgentSession 视图；当前会话切换时驻留内存） */}
        <AgentTabs />

        {error && (
          <div className="agent-warn">
            <CircleAlert size={14} strokeWidth={1.8} />
            {error}
            <button type="button" onClick={() => setError('')}>
              <X size={12} strokeWidth={2} />
            </button>
          </div>
        )}

        <div className="agent-main">
          {entries.length === 0 ? (
            // 空会话不需要虚拟化：保留原布局（flex 撑满 + 内容居中）
            <div className="agent-entries" tabIndex={0}>
              <div className="agent-entries__content">
                <div className="agent-empty">
                  <div className="agent-empty__art">
                    <OrbArt size={128} />
                  </div>
                  <h2>Agent 模式</h2>
                  <p>
                    选择项目文件夹后，输入 @ 引用项目内文件，Agent 可以读取并按你的指令执行任务。
                    <br />
                    写文件操作会先请求确认；不选项目则为纯聊天。
                  </p>
                </div>
              </div>
            </div>
          ) : (
            <Virtuoso
              key={sessionId}
              ref={virtuosoRef}
              className="agent-entries"
              tabIndex={0}
              data={items}
              initialTopMostItemIndex={items.length - 1}
              followOutput={follow ? 'auto' : false}
              atBottomStateChange={setFollow}
              rangeChanged={handleRangeChanged}
              increaseViewportBy={{ top: 480, bottom: 480 }}
              components={VIRTUOSO_COMPONENTS}
              itemContent={(i, e) =>
                e === STATUS_ITEM ? (
                  <EntriesFooter />
                ) : (
                  <EntryView
                    entry={e}
                    index={i}
                    isLast={i === entries.length - 1}
                    fullText={fullRunTexts.get(i)}
                    onDecide={handleDecide}
                    onAnswer={handleAnswer}
                    onEdit={handleEditUserMessage}
                    onFailureRewind={handleFailureRewind}
                    skills={skills}
                    fileSet={fileSet}
                    linkPreview
                  />
                )
              }
            />
          )}

          {/* 右侧圆点导轨：每个用户消息一颗，点击快速定位（悬停预览原文） */}
          {userMsgIdx.length > 0 && (
            <div className="agent-rail" role="navigation" aria-label="用户消息导航">
              {userMsgIdx.map((idx, n) => (
                <div key={idx} className="agent-rail__slot">
                  <span className="agent-rail__tip">{userMsgPreview(entries[idx]?.kind === 'user' ? entries[idx].text : undefined)}</span>
                  <button
                    type="button"
                    className={`agent-rail__dot${activeDot === n ? ' is-active' : ''}`}
                    title={`第 ${n + 1} 条提问`}
                    aria-label={`定位到第 ${n + 1} 条提问`}
                    onClick={() => jumpToEntry(idx)}
                  />
                </div>
              ))}
            </div>
          )}

          {/* 参数设置面板（悬浮在输入区上方）；图片识别开关在全局设置页 */}
          {showSettings && (
            <SettingsSheet
              systemPrompt={systemPrompt}
              disabled={running}
              onSystemPromptChange={setSystemPrompt}
            />
          )}


          {/* 输入区（@ 提及 / / 技能弹层内聚在 MentionInput 内，浮层绝对定位于输入框上方） */}
          <div className="agent-dock-wrap">
            <div className="agent-dock">
              {editing && (
                <div className="agent-dock__editing" role="status">
                  <Pencil size={12} strokeWidth={1.8} />
                  <span>
                    {editing.turnsBack > 1
                      ? `正在编辑历史消息，发送后将丢弃其后 ${editing.turnsBack - 1} 轮对话`
                      : '正在编辑最后一条消息，发送后将替换本轮对话'}
                  </span>
                  <button
                    type="button"
                    onClick={cancelEditing}
                    aria-label="取消编辑"
                    title="取消编辑（不做任何回退）"
                  >
                    <X size={12} strokeWidth={2} />
                  </button>
                </div>
              )}
              {queuedHere.length > 0 && (
                <div className="agent-dock__queue" role="list" aria-label="排队消息">
                  {queuedHere.map((q, i) => (
                    <div key={q.id} className="agent-dock__queue-item" role="listitem">
                      <span className="agent-dock__queue-badge">{i === 0 ? '下一条' : `#${i + 1}`}</span>
                      <span className="agent-dock__queue-text" title={q.raw}>{q.text || q.raw}</span>
                      <div className="agent-dock__queue-actions">
                        <button
                          type="button"
                          className="agent-dock__queue-btn"
                          onClick={() => editQueued(q)}
                          aria-label="编辑排队消息"
                          title="编辑（放回输入框）"
                        >
                          <Pencil size={14} strokeWidth={1.8} />
                        </button>
                        <button
                          type="button"
                          className="agent-dock__queue-btn is-danger"
                          onClick={() => removeQueued(q.id)}
                          aria-label="移除排队消息"
                          title="移除"
                        >
                          <Trash size={14} strokeWidth={1.8} />
                        </button>
                        <button
                          type="button"
                          className={`agent-dock__queue-send${i === 0 ? ' is-next' : ''}`}
                          disabled={settling}
                          onClick={() => void sendQueuedNow(q)}
                          aria-label="立即发送"
                          title={
                            running
                              ? '打断当前回答并立即发送'
                              : '立即发送'
                          }
                        >
                          立即发送
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
              {/* 粘贴落盘 chip：超长粘贴已写入项目文件，发送时正文只带「先读后处理」指令 */}
              {spill && (
                <div className="agent-dock__spill" role="status">
                  <FileText size={13} strokeWidth={1.8} />
                  <span title={spill.path}>
                    粘贴内容已存为 {spill.path.split(/[\\/]/).pop()}（{Math.max(1, Math.round(spill.size / 1024))} KB）· 发送时 Agent 将先读取全文
                  </span>
                  <button
                    type="button"
                    onClick={() => setSpill(null)}
                    aria-label="移除粘贴文件"
                    title="移除（文件保留在 .astra/pastes/，不随消息发送）"
                  >
                    <X size={12} strokeWidth={1.8} />
                  </button>
                </div>
              )}
              <MentionInput
                ref={composerRef}
                value={input}
                onChange={setInput}
                files={files}
                skills={skills}
                onSpillPaste={handleSpillPaste}
                placeholder={
                  running
                    ? 'Agent 正在回答…Enter 排队发送 · Ctrl/⌘+Enter 打断并优先发送'
                    : projectPath
                      ? '描述任务，例如：读取 @需求.md 并生成方案保存为 txt…'
                      : '纯聊天模式，顶部选择项目后可用 @ 引用文件 · / 调用技能…'
                }
                onKeyDown={(e) => {
                  // Zed 式：输入框为空时按 ↑ 把队尾消息拉回编辑器（等价 move_to_main_editor）
                  if (e.key === 'ArrowUp' && input.length === 0 && queuedHere.length > 0) {
                    e.preventDefault();
                    editQueued(queuedHere[queuedHere.length - 1]);
                    return;
                  }
                  if (isSendEnter(e)) {
                    e.preventDefault();
                    if (e.metaKey || e.ctrlKey) {
                      void handleInterruptSend();
                    } else if (running) {
                      handleQueueSend();
                    } else {
                      void handleSend();
                    }
                  }
                }}
              />
              {/* 底栏（Zed thread_view 式 justify-between 两组）：左 = 上下文/思考模式，右 = 用量 + 模型 + 发送 */}
              <div className="agent-dock__bar">
                <div className="agent-dock__bar-left">
                  {/* 历史面板开关（Zed 式推拉门）：从标题栏移入对话框底栏最左 */}
                  <button
                    type="button"
                    className={`agent-dock__iconbtn${historyOpen ? ' is-active' : ''}`}
                    onClick={toggleHistory}
                    title={historyOpen ? '隐藏历史面板' : '显示历史面板'}
                    aria-label={historyOpen ? '隐藏历史面板' : '显示历史面板'}
                  >
                    {historyOpen ? (
                      <PanelLeftClose size={15} strokeWidth={1.8} />
                    ) : (
                      <PanelLeftOpen size={15} strokeWidth={1.8} />
                    )}
                  </button>
                  {projectPath && (
                    <button
                      type="button"
                      className="agent-dock__iconbtn"
                      disabled={running}
                      onClick={() => composerRef.current?.insertText('@')}
                      title="添加上下文（@ 引用文件）"
                      aria-label="添加上下文"
                    >
                      <AtSign size={15} strokeWidth={1.8} />
                    </button>
                  )}
                  {/* 图片理解：模型经 read_file 直接读项目内图片（需图片识别开启，视觉模型） */}
                  <ThinkingButton
                    enabled={thinkingEnabled}
                    effort={thinkingEffort}
                    onToggle={() => setThinkingEnabled(!thinkingEnabled)}
                    onEffortChange={setThinkingEffort}
                  />
                </div>
                <div className="agent-dock__bar-right">
                  <ContextUsage usage={usage} />
                  <ModelPicker />
                  {running ? (
                    <>
                      {input.trim() && (
                        <button
                          type="button"
                          className="agent-dock__send"
                          disabled={settling}
                          onClick={handleQueueSend}
                          aria-label="排队发送"
                          title="排队发送，回答结束后自动发出 · Ctrl/⌘+Enter 打断并优先发送"
                        >
                          <ArrowRight size={16} strokeWidth={2.5} />
                        </button>
                      )}
                      <button type="button" className="agent-dock__stop" disabled={settling} onClick={() => void handleStop()}>
                        <span className="agent-dock__stop-square" />
                        停止
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      className="agent-dock__send"
                      disabled={!canSend}
                      onClick={() => void handleSend()}
                      aria-label="发送"
                      title="Enter 发送 · Shift+Enter 换行 · @ 引用文件"
                    >
                      <ArrowRight size={16} strokeWidth={2.5} />
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 链接预览面板：agent 回复中的链接拦截后在此用 iframe 打开 */}
      {linkPreviewOpen && <LinkPreviewPanel />}
    </div>
  );
}

// ===== 单条条目渲染 =====

interface UserRef {
  kind: 'skill' | 'file';
  name: string;
}

/**
 * 解析用户消息里的 /技能名（前缀，须命中已知技能）与 @文件相对路径（任意位置，须命中项目文件列表），
 * 命中的以 chip 呈现并从正文中移除；未命中的原样保留。
 * 历史回放的条目走同一解析，样式一致。
 */
function parseUserRefs(
  text: string,
  skills: UserSkillLite[],
  fileSet: Set<string>,
): { refs: UserRef[]; body: string } {
  let body = text;
  const refs: UserRef[] = [];

  // 前缀 /技能名（与发送时 parseSlashCommand 同规则）
  const sm = body.match(/^\/([\w\u4e00-\u9fff.-]+)(?:\s+([\s\S]*))?$/);
  if (sm && skills.some((s) => s.name === sm[1])) {
    refs.push({ kind: 'skill', name: sm[1] });
    body = sm[2] ?? '';
  }

  // @文件路径 token（命中项目文件才提为 chip，否则原样保留）
  body = body.replace(/@([\w\u4e00-\u9fff./-]+)/g, (full, name: string) => {
    // 贪婪匹配可能吞掉句尾点（如 @a.tsx.），去尾点后再比对；命中的 chip 只取文件路径，尾点留在正文
    const bare = name.replace(/\.+$/, '');
    const hit = fileSet.has(name) ? name : fileSet.has(bare) ? bare : null;
    if (!hit) return full;
    refs.push({ kind: 'file', name: hit });
    return name.slice(hit.length);
  });

  return { refs, body: body.trim() };
}

/** 圆点悬停预览：压成单行、限 60 字符 */
function userMsgPreview(text: string | undefined): string {
  if (!text) return '';
  const oneLine = text.replace(/\s+/g, ' ').trim();
  return oneLine.length > 60 ? `${oneLine.slice(0, 60)}…` : oneLine;
}

/** 消息级复制按钮（用户气泡 / 助手消息右上角，hover 浮现） */
function MessageCopyButton({ text, className = '' }: { text: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className={`aentry__copy${copied ? ' is-copied' : ''}${className ? ` ${className}` : ''}`}
      title={copied ? '已复制' : '复制原文'}
      onClick={async (e) => {
        e.stopPropagation();
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 1600);
        } catch {
          // 剪贴板不可用时忽略
        }
      }}
    >
      {copied ? <Check size={12} strokeWidth={2.2} /> : <Copy size={12} strokeWidth={1.8} />}
    </button>
  );
}

// 助手消息底部的常驻复制（区别于用户气泡右上角的悬浮复制）：非流式时才出现
function AssistantCopyFooter({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="aentry__foot">
      <button
        type="button"
        className={`aentry__foot-copy${copied ? ' is-copied' : ''}`}
        title={copied ? '已复制' : '复制全文'}
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1600);
          } catch {
            // 剪贴板不可用时忽略
          }
        }}
      >
        {copied ? <Check size={12} strokeWidth={2.2} /> : <Copy size={12} strokeWidth={1.8} />}
        {copied ? '已复制' : '复制'}
      </button>
    </div>
  );
}

/**
 * 运行中状态指示：三点脉冲 loading（状态文案与停止操作已按需求移除；
 * 悬停 title 保留当前状态，取消走输入框右侧的停止按钮）
 */
// Virtuoso 条目包装：条目是普通块级元素（非 flex 子项），原 __content 的 flex gap 失效，
// 间距改由条目自身 padding（padding-bottom 纳入条目测量高度，滚动定位不累积误差）
const EntryItem = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  function EntryItem(props, ref) {
    return <div ref={ref} {...props} className="agent-entries__item" />;
  },
);

// 运行中的状态条：作为列表末尾哨兵条目渲染（不用 components.Footer ——
// followOutput 的粘底对齐只针对最后一条数据条目，Footer 会留在可视区外）。
// 必须用模块级稳定组件订阅 store，
// 不能在 AgentPage 渲染时内联定义（会因组件引用变化反复重挂导致闪烁）
function EntriesFooter() {
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const status = useAgentStore((s) => s.status);
  if (!running) return null;
  return <StatusLine status={status} />;
}

const VIRTUOSO_COMPONENTS = { Item: EntryItem };

function StatusLine({ status }: { status: string | null }) {
  return (
    <div className="agent-statusline" title={(status && STATUS_LABEL[status]) || '运行中'}>
      <span className="agent-statusline__dot" />
      <span className="agent-statusline__dot" />
      <span className="agent-statusline__dot" />
    </div>
  );
}

const EntryView = memo(function EntryView({
  entry,
  index,
  isLast,
  fullText,
  onDecide,
  onAnswer,
  onEdit,
  onFailureRewind,
  skills,
  fileSet,
  linkPreview = false,
}: {
  entry: Entry;
  /** 条目在会话流中的序号：写入 data-idx，供右侧圆点导轨快速定位 */
  index: number;
  /** 是否为会话流最后一条：失败卡片的重试/编辑动作只在末尾提供
   *  （更早的失败轮次走消息 hover 的「编辑重发」） */
  isLast?: boolean;
  /** 本轮 Run 聚合后的完整回复（多段 assistant 时存在）；复制用 */
  fullText?: string;
  onDecide: (callId: string, allow: boolean) => Promise<void>;
  onAnswer: (questionId: string, answer: string | null) => void;
  /** hover 编辑重发（仅用户消息） */
  onEdit?: (index: number) => void;
  /** 失败卡片「重试 / 编辑后重发」 */
  onFailureRewind?: (mode: 'retry' | 'edit') => void;
  skills: UserSkillLite[];
  fileSet: Set<string>;
  /** 开启链接拦截：assistant 消息里的 http(s) 链接在右侧预览面板打开 */
  linkPreview?: boolean;
}) {
  switch (entry.kind) {
    case 'user': {
      const { refs, body } = parseUserRefs(entry.text, skills, fileSet);
      return (
        <div className="aentry aentry--user" data-idx={index}>
          <div className="aentry__bubble">
            {onEdit && (
              <button
                type="button"
                className="aentry__editbtn"
                title="编辑重发"
                aria-label="编辑重发"
                onClick={() => onEdit(index)}
              >
                <Pencil size={12} strokeWidth={1.8} />
              </button>
            )}
            <MessageCopyButton text={entry.text} />
            {entry.images && entry.images.length > 0 && (
              <div className="aentry__imgs">
                {entry.images.map((img, i) => (
                  <img
                    key={i}
                    className="aentry__img"
                    src={userImageUrl(img)}
                    alt={`附图 ${i + 1}`}
                    loading="lazy"
                  />
                ))}
              </div>
            )}
            {refs.length > 0 && (
              <div className="aentry__refs">
                {refs.map((r, i) =>
                  r.kind === 'skill' ? (
                    <span key={i} className="aentry__ref aentry__ref--skill" title={`技能 ${r.name}`}>
                      <Sparkles size={11} strokeWidth={2} />/{r.name}
                    </span>
                  ) : (
                    <span key={i} className="aentry__ref aentry__ref--file" title={`文件 ${r.name}`}>
                      <FileText size={11} strokeWidth={2} />{r.name}
                    </span>
                  ),
                )}
              </div>
            )}
            {body && <div className="aentry__text">{body}</div>}
          </div>
        </div>
      );
    }
    case 'assistant':
      // 对齐 chat 页：助手消息用 MarkdownMessage 渲染（GFM + 代码高亮），不加气泡。
      // 不渲染独立光标 span：markdown 最后一块是块级元素（p/ul/pre），
      // 光标会掉到下一行而非紧跟文字；chat 页同样只在空文本时显示 typing
      return (
        <div className={`aentry aentry--assistant${entry.streaming ? ' is-streaming' : ''}`} data-idx={index}>
          {entry.text ? (
            <>
              <div className="aentry__md">
                <MarkdownMessage content={entry.text} streaming={entry.streaming} linkPreview={linkPreview} />
              </div>
              {!entry.streaming && <AssistantCopyFooter text={fullText ?? entry.text} />}
            </>
          ) : (
            <div className="aentry__typing">
              <span />
              <span />
              <span />
            </div>
          )}
        </div>
      );
    case 'thinking':
      return <StreamBlock text={entry.text} done={entry.done} title="思考过程" icon={<Brain size={13} strokeWidth={1.8} />} />;
    case 'tool':
      return <ToolEntry entry={entry} onDecide={onDecide} />;
    case 'question':
      return <QuestionEntry entry={entry} onAnswer={onAnswer} />;
    case 'permission': {
      const pending = entry.state === 'pending';
      const label = PERM_ACTION_LABEL[entry.toolName] ?? '请求执行操作';
      return (
        <div className={`aentry aentry--permission is-${entry.state}`}>
          <div className="aentry__perm-head">
            <CircleAlert size={14} strokeWidth={1.8} />
            <b>{entry.toolName}</b> {label}
          </div>
          <code className="aentry__perm-summary">{entry.summary}</code>
          {pending ? (
            <div className="aentry__perm-actions">
              <button type="button" className="is-allow" onClick={() => void onDecide(entry.callId, true)}>
                <Check size={13} strokeWidth={2} />
                批准
              </button>
              <button type="button" className="is-deny" onClick={() => void onDecide(entry.callId, false)}>
                <X size={13} strokeWidth={2} />
                拒绝
              </button>
            </div>
          ) : (
            <div className={`aentry__perm-state is-${entry.state}`}>
              {entry.state === 'allowed' ? '已批准' : entry.state === 'expired' ? '已失效' : '已拒绝'}
            </div>
          )}
        </div>
      );
    }
    case 'notice':
      return <div className="aentry aentry--terminal" role="status">{entry.text}</div>;
    case 'terminal':
      return (
        <div className={`aentry aentry--terminal is-${entry.outcome}`}>
          {entry.outcome === 'completed' && (
            <>
              <CircleCheck size={13} strokeWidth={1.8} /> 任务完成
            </>
          )}
          {entry.outcome === 'failed' && (
            <>
              <CircleX size={13} strokeWidth={1.8} /> 任务失败{entry.text ? `：${entry.text}` : ''}
              {entry.text?.includes('UnsupportedModel') && (
                <div className="aentry__hint">
                  所选模型与当前模型服务端点不兼容（如火山编码计划端点仅支持计划内编码模型）。
                  请在模型列表选择兼容模型，或改用标准 API 端点后新建会话。
                </div>
              )}
              {isLast && onFailureRewind && (
                <div className="aentry__rewind">
                  <button type="button" className="aentry__rewind-btn" onClick={() => onFailureRewind('retry')}>
                    <RotateCcw size={12} strokeWidth={1.8} />
                    重试
                  </button>
                  <button type="button" className="aentry__rewind-btn" onClick={() => onFailureRewind('edit')}>
                    <Pencil size={12} strokeWidth={1.8} />
                    编辑后重发
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      );
  }
});

// ===== 提问卡（ask_user：选项按钮 + 自由文本；答案 verbatim 回传模型） =====

function QuestionEntry({
  entry,
  onAnswer,
}: {
  entry: Extract<Entry, { kind: 'question' }>;
  onAnswer: (questionId: string, answer: string | null) => void;
}) {
  const [freeText, setFreeText] = useState('');
  const answered = entry.answer !== undefined || entry.declined;
  return (
    <div className="aentry aentry--question is-answered">
      <div className="aentry__q-head">
        <HelpCircle size={14} strokeWidth={1.8} />
        <span>{entry.question}</span>
      </div>
      {answered ? (
        <div className={`aentry__q-state${entry.declined ? ' is-declined' : ''}`}>
          {entry.declined ? '未回答' : `你的回答：${entry.answer}`}
        </div>
      ) : (
        <div className="aentry__q-body">
          {entry.options.length > 0 && (
            <div className="aentry__q-options">
              {entry.options.map((o) => (
                <button
                  key={o}
                  type="button"
                  onClick={() => onAnswer(entry.questionId, o)}
                >
                  {o}
                </button>
              ))}
            </div>
          )}
          {entry.allowFreeText && (
            <div className="aentry__q-free">
              <input
                type="text"
                value={freeText}
                placeholder={entry.options.length > 0 ? '或输入自定义回答…' : '输入你的回答…'}
                onChange={(e) => setFreeText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && freeText.trim()) {
                    onAnswer(entry.questionId, freeText.trim());
                  }
                }}
              />
              <button
                type="button"
                className="is-allow"
                disabled={!freeText.trim()}
                onClick={() => onAnswer(entry.questionId, freeText.trim())}
              >
                <Check size={13} strokeWidth={2} />
                回答
              </button>
            </div>
          )}
          <div className="aentry__q-actions">
            <button type="button" className="is-deny" onClick={() => onAnswer(entry.questionId, null)}>
              <X size={13} strokeWidth={2} />
              取消回答
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ===== 工具卡片（结果默认折叠，点击展开，避免大段文件/终端输出刷屏） =====

/**
 * 从不完整的参数 JSON 中宽松提取可预览文本（流式生成中 args 未到齐）。
 * 逐字符扫描顶层 "key": "value" 对，容忍尾部被截断的字符串。
 * 返回 undefined 表示暂无可展示内容（如 JSON 还没出现任何键）。
 */
export function extractPartialArgsPreview(
  toolName: string,
  raw: string,
): string | undefined {
  if (!raw) return undefined;
  const pairs: Array<[string, string]> = [];
  let i = 0;
  const n = raw.length;
  while (i < n) {
    // 找下一个键的起始引号（跳过 { , 空白）
    if (raw[i] !== '"') {
      i += 1;
      continue;
    }
    const keyStart = i + 1;
    const keyEnd = raw.indexOf('"', keyStart);
    if (keyEnd < 0) break;
    const key = raw.slice(keyStart, keyEnd);
    let j = keyEnd + 1;
    while (j < n && raw[j] !== ':') j += 1;
    j += 1; // 跳过冒号
    while (j < n && raw[j] === ' ') j += 1;
    if (j >= n) break;
    if (raw[j] === '"') {
      // 字符串值：可能未闭合（尾部截断）
      let value = '';
      let k = j + 1;
      let closed = false;
      while (k < n) {
        const ch = raw[k];
        if (ch === '\\' && k + 1 < n) {
          // 转义：还原常见控制符，避免预览里出现 \n 字面量
          const esc = raw[k + 1];
          value += esc === 'n' ? '\n' : esc === 't' ? '\t' : esc;
          k += 2;
          continue;
        }
        if (ch === '"') {
          closed = true;
          break;
        }
        value += ch;
        k += 1;
      }
      pairs.push([key, value]);
      i = closed ? k + 1 : n; // 未闭合说明参数已截断，到此为止
    } else {
      // 非字符串值（数字/布尔）：取到逗号或结尾
      let k = j;
      while (k < n && raw[k] !== ',' && raw[k] !== '}') k += 1;
      pairs.push([key, raw.slice(j, k).trim()]);
      i = k;
    }
  }
  if (pairs.length === 0) return undefined;
  // 按工具形态组织预览：与 contentPreview 的约定一致
  const get = (k: string) => pairs.find(([key]) => key === k)?.[1];
  if (toolName === 'write_file' || toolName === 'read_file') {
    const content = get('content');
    return content !== undefined ? content : pairs.map(([k, v]) => `${k}: ${v}`).join('\n');
  }
  if (toolName === 'edit_file') {
    const oldT = get('old_text');
    const newT = get('new_text');
    if (oldT !== undefined || newT !== undefined) {
      return `- ${oldT ?? ''}\n+ ${newT ?? ''}`;
    }
  }
  if (toolName === 'shell_execute') {
    const cmd = get('command');
    return cmd !== undefined ? cmd : undefined;
  }
  return pairs.map(([k, v]) => `${k}: ${v}`).join('\n');
}

function ToolEntry({
  entry,
  onDecide,
}: {
  entry: Extract<Entry, { kind: 'tool' }>;
  onDecide: (callId: string, allow: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const hasBody = !!(entry.result || (entry.artifacts && entry.artifacts.length > 0));
  const perm = entry.perm;
  // 写文件类工具展示完整内容，shell 展示完整命令，edit 展示新旧文本对照
  // （args 由 tool_call_started 事件携带，旧会话无此字段）
  const contentPreview =
    entry.args && typeof entry.args.content === 'string'
      ? entry.args.content
      : entry.toolName === 'edit_file' &&
        entry.args &&
        typeof entry.args.old_text === 'string' &&
        typeof entry.args.new_text === 'string'
        ? `- ${entry.args.old_text}\n+ ${entry.args.new_text}`
        : entry.toolName === 'shell_execute' && entry.args && typeof entry.args.command === 'string'
          ? entry.args.command
          : undefined;
  // 参数还在流式生成中（tool_call_started 未到）：宽松解析不完整 JSON 展示已生成部分。
  // 同一工具卡：args 到齐后切 contentPreview，流式阶段展示 streamingPreview
  const streamingPreview =
    !contentPreview && entry.argsStreaming !== undefined
      ? extractPartialArgsPreview(entry.toolName, entry.argsStreaming)
      : undefined;
  const previewText = contentPreview ?? streamingPreview;
  // 内容已完成 = args 已到齐（区别于流式生成中）；驱动 StreamBlock 自动收起
  const previewDone = contentPreview !== undefined;
  const previewTitle =
    entry.toolName === 'write_file'
      ? '写入内容'
      : entry.toolName === 'read_file'
        ? '文件内容'
        : entry.toolName === 'edit_file'
          ? '修改对照'
          : entry.toolName === 'shell_execute' || entry.toolName === 'shell_background'
            ? '命令'
            : '参数';
  // shell_background 完成后：从结果中取 task_id，提供「预览」入口（host 从日志检测地址后开窗）
  const bgTaskId =
    entry.toolName === 'shell_background' && entry.status === 'completed' && entry.result
      ? (entry.result.match(/task_id=(\d+)/)?.[1] ?? null)
      : null;
  const [previewHint, setPreviewHint] = useState('');
  const openPreview = useCallback(async () => {
    if (!bgTaskId) return;
    setPreviewHint('');
    try {
      await invoke('agent_open_preview', { taskId: Number(bgTaskId) });
    } catch (e) {
      setPreviewHint(String(e).replace(/^.*错误:/, '').trim());
    }
  }, [bgTaskId]);
  return (
    <div className={`aentry aentry--tool is-${entry.status}${perm?.state === 'pending' ? ' is-perm' : ''}`}>
      <button
        type="button"
        className="aentry__tool-head"
        onClick={() => hasBody && setOpen((v) => !v)}
        title={hasBody ? (open ? '收起' : '展开结果') : undefined}
      >
        {entry.status === 'running' ? (
          <Loader2 size={13} strokeWidth={2} className="aentry__spin" />
        ) : entry.status === 'completed' ? (
          <CircleCheck size={13} strokeWidth={1.8} />
        ) : entry.status === 'denied' ? (
          <CircleX size={13} strokeWidth={1.8} />
        ) : (
          <CircleAlert size={13} strokeWidth={1.8} />
        )}
        <Wrench size={12} strokeWidth={1.8} />
        <code className="aentry__tool-name">{entry.toolName}</code>
        <span className="aentry__tool-args">{entry.argsSummary}</span>
        <span className="aentry__tool-status">
          {perm?.state === 'pending'
            ? '等待确认'
            : entry.status === 'running'
              ? '执行中'
              : entry.status === 'completed'
                ? '完成'
                : entry.status === 'denied'
                  ? '已拒绝'
                  : entry.status === 'interrupted' ? '已中断' : '失败'}
        </span>
        {hasBody && (
          <ChevronDown size={12} strokeWidth={2} className={`aentry__tool-chev${open ? ' rotated' : ''}`} />
        )}
      </button>
      {bgTaskId && (
        <div className="aentry__preview">
          <button type="button" className="aentry__preview-btn" onClick={() => void openPreview()}>
            <MonitorUp size={12} strokeWidth={1.8} />
            预览
          </button>
          {previewHint && <span className="aentry__preview-hint">{previewHint}</span>}
        </div>
      )}
      {previewText && (
        <StreamBlock
          variant="tool"
          title={previewTitle}
          icon={<FileText size={12} strokeWidth={1.8} />}
          text={previewText}
          done={previewDone}
        />
      )}
      {/* Confirm 工具的批准/拒绝直接内嵌在卡片里 */}
      {perm?.state === 'pending' && (
        <div className="aentry__perm-inline">
          <code className="aentry__perm-summary">{perm.summary}</code>
          <div className="aentry__perm-actions">
            <button type="button" className="is-allow" onClick={() => void onDecide(entry.callId, true)}>
              <Check size={13} strokeWidth={2} />
              批准
            </button>
            <button type="button" className="is-deny" onClick={() => void onDecide(entry.callId, false)}>
              <X size={13} strokeWidth={2} />
              拒绝
            </button>
          </div>
        </div>
      )}
      {open && hasBody && (
        <div className="aentry__tool-body">
          {entry.result && <pre className="aentry__tool-result">{entry.result}</pre>}
          {entry.artifacts && entry.artifacts.length > 0 && (
            <ArtifactList artifacts={entry.artifacts} />
          )}
        </div>
      )}
    </div>
  );
}
