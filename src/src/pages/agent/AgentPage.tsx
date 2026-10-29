/**
 * Agent 对话页（阶段 3：Zed 式 Agent 交互 MVP）
 *
 * 与 chat 页的关系：
 * - 复用 chatToolbar store（标题栏选择 provider / 本地模型对本页同样生效）
 * - 复用 workspace store 的项目列表；选择项目 → 后端注册文件工具（读=自动，写=确认卡）
 * - 事件流：Tauri `agent-event`（Envelope：type 标签 + snake_case 载荷）
 * - 权限确认卡：批准/拒绝 → agent_permission_decide，UI 状态由 permission_decided 事件回写
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Virtuoso, type ListRange, type VirtuosoHandle } from 'react-virtuoso';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openWithDefaultApp, openWithEditor } from '@/lib/openService';
import { ArrowRight, AtSign, ChevronDown, CircleAlert, FileText, FolderOpen, PanelLeftClose, PanelLeftOpen, Pencil, Trash, X } from 'lucide-react';
import OrbArt from '@/components/illustrations/OrbArt';

import { useChatToolbarStore } from '@/stores/chatToolbar';
import { useAgentStore, ensureAgentListeners, refreshAgentStatus, nextQueuedFor, type Entry, type QueuedMessage, type UserImage, withId } from '@/stores/agent';
import { useWorkspaceStore } from '@/stores/workspace';
import { isSendEnter } from '@/hooks/useMention';
import { EntryView, EntriesFooter, userMsgPreview, VIRTUOSO_COMPONENTS } from './EntryView';
import { ThinkingButton, ContextUsage, RunningSubtasks } from './StatusWidgets';

/** 运行中状态行的哨兵条目：占据列表最后一项（见 AgentPage 内 items 处注释） */
const STATUS_ITEM = Symbol('statusline');
type ListItem = Entry | typeof STATUS_ITEM;
import ModelPicker from '@/components/agent/ModelPicker';
import { Mascot } from '@/components/agent/Mascot';
import { useMascotStore } from '@/stores/mascot';
import { parseCatCommandInput } from '@/hooks/useCatCommand';
import { useAgentThemeStore } from '@/lib/agentTheme';

import MentionInput, { type MentionInputHandle } from '../chat/MentionInput';
import SettingsSheet from '../chat/SettingsSheet';
import { parseSlashCommand, type UserSkillLite } from '@/hooks/useSlash';
import AgentHistoryPanel, { type AgentSessionMeta } from './AgentHistoryPanel';
import AgentTabs from './AgentTabs';
import LinkPreviewPanel from './LinkPreviewPanel';
import FilePreviewPanel from './FilePreviewPanel';
import { useLinkPreviewStore } from '@/stores/linkPreview';
import { useFilePreviewStore } from '@/stores/filePreview';
import { openFilePath } from '@/components/agent/FilePathText';

import './AgentPage.scss';

// 事件模型与条目类型见 stores/agent.ts（全局会话状态的唯一事实来源）

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
  const pushCat = useAgentStore((s) => s.pushCat);
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
  // ===== 粘贴文件（Finder/资源管理器复制的文件 → 复制到 .astra/pastes/） =====
  // 同为页面级状态，切会话重置；导入后的文件与 spill 同目录，read_file Allow 级可读
  const [pastedFiles, setPastedFiles] = useState<{ path: string; name: string; size: number }[]>([]);
  useEffect(() => {
    setSpill(null);
    setPastedFiles([]);
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

  /** 系统文件粘贴导入：把剪贴板里的文件复制到粘贴目录，返回导入结果。
   *  空列表 = 剪贴板没有文件 URL（截图等图片数据），输入框降级为文本粘贴 */
  const handlePasteFiles = useCallback(async (): Promise<{ path: string; name: string; size: number }[]> => {
    try {
      const res = await invoke<{ path: string; name: string; size: number }[]>(
        'agent_import_pasted_files',
        { projectRoot: projectPath || null },
      );
      if (res.length > 0) {
        setPastedFiles((prev) => [...prev, ...res.filter((f) => !prev.some((p) => p.path === f.path))]);
      }
      return res;
    } catch (e) {
      setError(`导入粘贴文件失败：${String(e)}`);
      return [];
    }
  }, [projectPath, setError]);

  /** 发送前组装粘贴文件：正文换为「先读文件再处理指令」，并取走暂存列表。
   *  displaySuffix 追加到会话气泡展示文本，保留粘贴的可见痕迹 */
  const takePastedFiles = useCallback(
    (text: string): { text: string; displaySuffix: string } => {
      if (pastedFiles.length === 0) return { text, displaySuffix: '' };
      const list = pastedFiles.map((f) => f.path).join('\n');
      const names = pastedFiles.map((f) => f.name).join('、');
      setPastedFiles([]);
      return {
        text: `【用户粘贴的 ${pastedFiles.length} 个文件已复制到：\n${list}\n请先用 read_file 工具读取这些文件，再处理下方指令】\n\n${text}`,
        displaySuffix: `\n📎 粘贴文件：${names}，已让 Agent 先读取`,
      };
    },
    [pastedFiles],
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

  const agentTheme = useAgentThemeStore((s) => s.id);

  // 链接预览面板（agent 回复中的链接拦截后在此用 iframe 打开）
  const linkPreviewOpen = useLinkPreviewStore((s) => s.open);
  const filePreviewOpen = useFilePreviewStore((s) => s.open);
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
   * lastAssistantOfRun：每个 Run 的最后一条 assistant 条目索引（只有它展示底部「复制全文」）。
   */
  const { fullRunTexts, lastAssistantOfRun } = useMemo(() => {
    const map = new Map<number, string>();
    const lastAssistantOfRun = new Set<number>();
    let runStart = 0;
    const flush = (end: number) => {
      const texts: string[] = [];
      let lastIdx = -1;
      for (let i = runStart; i < end; i++) {
        const e = entries[i];
        if (e.kind === 'assistant' && e.text.trim()) {
          texts.push(e.text);
          lastIdx = i;
        }
      }
      if (lastIdx >= 0) lastAssistantOfRun.add(lastIdx);
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
    return { fullRunTexts: map, lastAssistantOfRun };
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
      // 发送冲刺：猫猫先把消息「送出去」（工作级，冷却防连发），随后进入工作姿态
      useMascotStore
        .getState()
        .interact('run', { priority: 'work', key: 'send-sprint', cooldownMs: 5000, holdMs: 1200 });

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

  /** `&猫猫命令` 拦截：本地脚本回应 + 触发吉祥物动画，不经 agent 后端。
   *  返回 true 表示已消费（发送流程终止）；未命中已知命令的 & 文本照常发给 agent */
  const fireCatInteraction = useCallback(
    (raw: string): boolean => {
      const catCmd = parseCatCommandInput(raw);
      if (!catCmd) return false;
      setInput('');
      setError('');
      useMascotStore.getState().interact(catCmd.command.pose, { fx: catCmd.command.fx, bubble: catCmd.command.reply });
      pushUser(raw);
      pushCat(catCmd.command.reply);
      return true;
    },
    [pushUser, pushCat, setInput, setError],
  );

  const handleSend = useCallback(async () => {
    const raw = input.trim();
    if (!raw) return;
    if (fireCatInteraction(raw)) return;
    if (running) return;
    // 有文本时照常走 /技能名 解析
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    const spilled = takeSpill(parsed.text);
    const withFiles = takePastedFiles(spilled.text);
    await doSend(raw + spilled.displaySuffix + withFiles.displaySuffix, parsed.skill, withFiles.text);
  }, [input, running, fireCatInteraction, parseSendInput, doSend, takeSpill, takePastedFiles]);

  /** 失败卡片「重试 / 编辑后重发」：不做回档、不产生归档，完整对话记录保持显示。
   *  重试 = 同一消息原样追加重发；编辑后重发 = 回填输入框改完再发。
   *  两者都是追加到当前对话末尾，模型上下文保留失败尝试的现场 */
  const handleFailureRewind = useCallback(
    async (mode: 'retry' | 'edit') => {
      if (running || settling) return;
      let lastUserIdx = -1;
      for (let i = entries.length - 1; i >= 0; i--) {
        if (entries[i].kind === 'user') {
          lastUserIdx = i;
          break;
        }
      }
      if (lastUserIdx < 0) return;
      const lastUser = entries[lastUserIdx];
      if (lastUser.kind !== 'user') return;
      setEditing(null);
      if (mode === 'edit') {
        setInput(lastUser.text);
        requestAnimationFrame(() => composerRef.current?.focusEnd());
      } else {
        const parsed = parseSendInput(lastUser.text);
        if (parsed) await doSend(lastUser.text, parsed.skill, parsed.text);
      }
    },
    [running, settling, entries, setInput, parseSendInput, doSend],
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
    if (fireCatInteraction(raw)) return; // 猫猫互动运行中也允许（纯前端，不打扰 Run）
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    setEditing(null); // 排队新消息 = 放弃挂起的编辑意图（不回退）
    const spilled = takeSpill(parsed.text);
    const withFiles = takePastedFiles(spilled.text);
    enqueueMessage(raw + spilled.displaySuffix + withFiles.displaySuffix, withFiles.text, parsed.skill);
    setInput('');
  }, [input, fireCatInteraction, parseSendInput, enqueueMessage, setInput, takeSpill, takePastedFiles]);

  /** 打断并优先发送：当前 Run 取消 + 本条插到会话队首，settle 后立即续发。
   *  别的会话占用后端时不能打断（cancel 是全局单活动 Run），退化为普通排队 */
  const handleInterruptSend = useCallback(async () => {
    const raw = input.trim();
    if (!raw) return;
    if (fireCatInteraction(raw)) return; // 猫猫互动不需要打断 Run
    const parsed = parseSendInput(raw);
    if (!parsed) return;
    setEditing(null); // 插队新消息 = 放弃挂起的编辑意图（不回退）
    const st = useAgentStore.getState();
    const interruptible = st.running && !st.settling;
    const spilled = takeSpill(parsed.text);
    const withFiles = takePastedFiles(spilled.text);
    enqueueMessage(raw + spilled.displaySuffix + withFiles.displaySuffix, withFiles.text, parsed.skill, { front: interruptible });
    setInput('');
    if (interruptible) {
      try {
        await invoke('agent_cancel', { sessionId });
      } catch (error) {
        setError(`停止失败：${String(error)}`);
      }
    }
  }, [input, fireCatInteraction, parseSendInput, enqueueMessage, setInput, setError, sessionId, takeSpill, takePastedFiles]);

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
      await openWithDefaultApp(projectPath);
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
    openWithEditor(projectPath, editor.command).catch((e) => {
      setError(`打开编辑器失败: ${e}`);
    });
  }, [projectPath, editor]);

  // 最近使用文件夹（localStorage）；去重置顶，上限 8 条
  const recordRecentProject = useCallback((path: string) => {
    try {
      const recents: string[] = JSON.parse(localStorage.getItem('astra.recentProjects') ?? '[]');
      const next = [path, ...recents.filter((p) => p !== path)].slice(0, 8);
      localStorage.setItem('astra.recentProjects', JSON.stringify(next));
    } catch {
      localStorage.setItem('astra.recentProjects', JSON.stringify([path]));
    }
  }, []);

  // 一键选择文件夹：系统目录选择器，选中即绑定（记录到最近使用）
  const pickProjectFolder = useCallback(async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: '选择项目文件夹',
      });
      if (picked) {
        recordRecentProject(picked);
        openDraftForProject(picked);
        setError('');
      }
    } catch (e) {
      setError(`选择文件夹失败: ${e}`);
    }
  }, [openDraftForProject, recordRecentProject, setError]);

  // ===== 文件夹选择（Zed 式）：底栏文件夹按钮即入口 ——
  //  已绑定时按钮显示文件夹名（过长省略、悬停看全路径），点击弹出
  //  向上菜单（最近文件夹点击即直接切换）；未绑定时点击同样弹菜单，
  //  列表项点击直接绑定。无模式开关、无二级确认 =====
  const [folderMenuOpen, setFolderMenuOpen] = useState(false);
  const folderRootRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!folderMenuOpen) return;
    function onDown(e: MouseEvent) {
      if (!folderRootRef.current?.contains(e.target as Node)) setFolderMenuOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [folderMenuOpen]);

  /** 打开菜单：合并工作空间已知项目 + 最近使用记录（点击项直接绑定，不再确认） */
  const openFolderMenu = useCallback(() => {
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
    setFolderMenuOpen(true);
  }, [projects]);
  const [recentPicks, setRecentPicks] = useState<{ name: string; path: string }[]>([]);

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
    <div className="agent-root" data-agent-theme={agentTheme}>
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

      {/* 右列：标签栏 + 对话流 */}
      <div className="agent-col">
        {/* 多会话标签栏（tab 即 AgentSession 视图；当前会话切换时驻留内存） */}
        <AgentTabs />

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
              // "在底部"判定加容差：默认 4px 太苛刻，流式渲染/亚像素抖动差几像素
              // 就会误判为离开底部、断掉粘底跟随（follow→false）。64px 内都算在底部。
              atBottomThreshold={64}
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
                    isRunFinalAssistant={lastAssistantOfRun.has(i)}
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
            <Mascot onFocusInput={() => composerRef.current?.focusEnd()} />
            <div className="agent-dock">
              {/* 错误提示：与编辑/排队/粘贴 chip 同层，展示在对话框最上方 */}
              {error && (
                <div className="agent-warn">
                  <CircleAlert size={14} strokeWidth={1.8} />
                  {error}
                  <button type="button" onClick={() => setError('')}>
                    <X size={12} strokeWidth={2} />
                  </button>
                </div>
              )}
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
                  <span
                    title={spill.path}
                    className="agent-dock__spill-path"
                    onClick={() => openFilePath(spill.path)}
                  >
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
              {/* 粘贴文件 chip：系统剪贴板文件已复制到粘贴目录，发送时正文只带「先读后处理」指令 */}
              {pastedFiles.map((f) => (
                <div key={f.path} className="agent-dock__spill" role="status">
                  <FileText size={13} strokeWidth={1.8} />
                  <span
                    title={f.path}
                    className="agent-dock__spill-path"
                    onClick={() => openFilePath(f.path)}
                  >
                    粘贴文件 {f.name}（{Math.max(1, Math.round(f.size / 1024))} KB）· 发送时 Agent 将先读取
                  </span>
                  <button
                    type="button"
                    onClick={() => setPastedFiles((prev) => prev.filter((p) => p.path !== f.path))}
                    aria-label="移除粘贴文件"
                    title="移除（文件保留在 .astra/pastes/，不随消息发送）"
                  >
                    <X size={12} strokeWidth={1.8} />
                  </button>
                </div>
              ))}
              <MentionInput
                ref={composerRef}
                value={input}
                onChange={setInput}
                files={files}
                skills={skills}
                onPasteFiles={handlePasteFiles}
                onSpillPaste={handleSpillPaste}
                placeholder={
                  running
                    ? 'Agent 正在回答…Enter 排队发送 · Ctrl/⌘+Enter 打断并优先发送'
                    : projectPath
                      ? '描述任务，例如：读取 @需求.md 并生成方案保存为 txt…'
                      : '纯聊天模式，底栏选择文件夹后可用 @ 引用文件 · / 调用技能…'
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
                  {/* 文件夹：已绑定时按钮显示文件夹名（过长省略、悬停看全路径），
                      点击弹出向上菜单直接切换/解绑；未绑定时列出最近文件夹 */}
                  <div className="agent-dock__folderwrap" ref={folderRootRef}>
                    <button
                      type="button"
                      className={`agent-dock__folder-btn${projectPath ? ' is-bound' : ''}${folderMenuOpen ? ' is-open' : ''}`}
                      onClick={openFolderMenu}
                      title={projectPath ?? '选择文件夹，让助手读取其中的文件'}
                      aria-haspopup="menu"
                      aria-expanded={folderMenuOpen}
                    >
                      <FolderOpen size={15} strokeWidth={1.8} />
                      {projectPath && (
                        <span className="agent-dock__folder-name">
                          {projects.find((p) => p.path === projectPath)?.name ?? projectPath.split('/').pop()}
                        </span>
                      )}
                      {projectPath && (
                        <ChevronDown size={11} strokeWidth={2.2} className={`agent-dock__folder-chevron${folderMenuOpen ? ' is-open' : ''}`} />
                      )}
                    </button>
                    {folderMenuOpen && (
                      <div className="agent-dock__folder-menu" role="menu">
                        {projectPath && (
                          <div className="agent-dock__folder-current" title={projectPath}>
                            {projects.find((p) => p.path === projectPath)?.name ?? projectPath.split('/').pop()}
                          </div>
                        )}
                        {recentPicks.length > 0 && (
                          <div className="agent-dock__folder-list">
                            {recentPicks
                              .filter((p) => p.path !== projectPath)
                              .map((p) => (
                                <button
                                  key={p.path}
                                  type="button"
                                  role="menuitem"
                                  className="agent-dock__folder-item"
                                  title={p.path}
                                  disabled={running}
                                  onClick={() => {
                                    setFolderMenuOpen(false);
                                    openDraftForProject(p.path);
                                    setError('');
                                  }}
                                >
                                  <FolderOpen size={13} strokeWidth={1.8} />
                                  <span className="agent-dock__folder-item-name">{p.name}</span>
                                  <span className="agent-dock__folder-item-path">{p.path}</span>
                                </button>
                              ))}
                          </div>
                        )}
                        <div className="agent-dock__folder-sep" />
                        <button
                          type="button"
                          role="menuitem"
                          className="agent-dock__folder-item"
                          onClick={() => {
                            setFolderMenuOpen(false);
                            void pickProjectFolder();
                          }}
                        >
                          <FolderOpen size={13} strokeWidth={1.8} />
                          选择其他文件夹…
                        </button>
                        {projectPath && (
                          <button
                            type="button"
                            role="menuitem"
                            className="agent-dock__folder-item"
                            onClick={() => {
                              setFolderMenuOpen(false);
                              void openProjectInFolder();
                            }}
                          >
                            <FolderOpen size={13} strokeWidth={1.8} />
                            在 Finder 中打开
                          </button>
                        )}
                        {projectPath && (
                          <button
                            type="button"
                            role="menuitem"
                            className="agent-dock__folder-item"
                            onClick={() => {
                              setFolderMenuOpen(false);
                              openProjectInEditor();
                            }}
                          >
                            <FolderOpen size={13} strokeWidth={1.8} />
                            在编辑器中打开
                          </button>
                        )}
                        {projectPath && (
                          <button
                            type="button"
                            role="menuitem"
                            className="agent-dock__folder-item is-danger"
                            disabled={running}
                            onClick={() => {
                              setFolderMenuOpen(false);
                              openDraftForProject('');
                            }}
                          >
                            移除当前文件夹（转为纯聊天）
                          </button>
                        )}
                      </div>
                    )}
                  </div>
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
                  <RunningSubtasks />
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
      {/* 文件预览面板：产物条目点击「预览」后在此打开（单窗口，新开替换旧内容） */}
      {filePreviewOpen && <FilePreviewPanel />}
    </div>
  );
}
