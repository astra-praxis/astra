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
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import {
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleX,
  ExternalLink,
  FileText,
  FolderOpen,
  History,
  Sparkles,
  Loader2,
  Plus,
  Square,
  Wrench,
  X,
} from 'lucide-react';
import OrbArt from '@/components/illustrations/OrbArt';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import {
  useAgentStore,
  ensureAgentListeners,
  type Entry,
} from '@/stores/agent';
import { useWorkspaceStore } from '@/stores/workspace';
import { isSendEnter, useMention } from '@/hooks/useMention';
import ChatToolbar from '@/components/chat/ChatToolbar';
import MarkdownMessage from './chat/MarkdownMessage';
import FileMentionPopup from './chat/FileMentionPopup';
import SlashPopup from './chat/SlashPopup';
import SettingsSheet from './chat/SettingsSheet';
import { parseSlashCommand, useSlash, type UserSkillLite } from '@/hooks/useSlash';
import AgentHistoryPanel, { type AgentSessionMeta } from './AgentHistoryPanel';
import './AgentPage.scss';

// 事件模型与条目类型见 stores/agent.ts（全局会话状态的唯一事实来源）

const STATUS_LABEL: Record<string, string> = {
  analyzing: '分析中',
  executing_tool: '执行工具',
  awaiting_permission: '等待确认',
  finalizing: '收尾中',
};

// 权限确认卡文案按工具区分（未收录的工具用兜底文案）
const PERM_ACTION_LABEL: Record<string, string> = {
  write_file: '请求写入文件',
  shell_execute: '请求执行命令',
  build_docx: '请求生成 docx 文档',
  extract_document: '请求转换文档为 Markdown',
  read_file: '请求读取文件',
  list_files: '请求列出目录',
};

export default function AgentPage() {
  // ===== 全局会话状态（zustand；切页不丢，事件由模块级单例监听持续写入） =====
  const sessionId = useAgentStore((s) => s.sessionId);
  const entries = useAgentStore((s) => s.entries);
  const input = useAgentStore((s) => s.input);
  const setInput = useAgentStore((s) => s.setInput);
  const running = useAgentStore((s) => s.running);
  const status = useAgentStore((s) => s.status);
  const error = useAgentStore((s) => s.error);
  const setError = useAgentStore((s) => s.setError);
  const projectPath = useAgentStore((s) => s.projectPath);
  const setProjectPath = useAgentStore((s) => s.setProjectPath);
  const sessions = useAgentStore((s) => s.sessions);
  const refreshSessions = useAgentStore((s) => s.refreshSessions);
  const saveSession = useAgentStore((s) => s.saveSession);
  const pushUser = useAgentStore((s) => s.pushUser);
  const pushTerminal = useAgentStore((s) => s.pushTerminal);
  const setRunState = useAgentStore((s) => s.setRunState);

  // 事件监听为模块级单例：首次进入本页时挂载，卸载后不拆除（切页对话继续）
  useEffect(() => {
    ensureAgentListeners();
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

  // ===== @ 文件引用（与 chat 页共用 useMention，条目为文件相对路径；不再绑定项目） =====
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const mention = useMention(files, textareaRef, undefined, setInput);

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
  const slash = useSlash(skills, textareaRef, setInput);

  // 气泡内 @文件 chip 解析用的文件集合（files 变化时重建）
  const fileSet = useMemo(() => new Set(files), [files]);

  // ===== 历史面板开关（易失 UI 状态，留在组件内） =====
  const [historyOpen, setHistoryOpen] = useState(false);

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

  // ===== 参数（SettingsSheet；system_prompt 传给 agent_start_run，温度/token 后端暂不支持） =====
  const [systemPrompt, setSystemPrompt] = useState('你是一个有用的助手，请简洁准确地回答用户问题。');
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(4000);

  const selectedProvider = providers.find((p) => p.id === selectedProviderId) ?? null;
  const selectedModel = runningModels.find((m) => m.port === selectedModelPort) ?? null;
  const modelName =
    sourceType === 'model'
      ? (selectedModel?.name ?? 'local')
      : (overrideModelName ?? selectedProvider?.active_model ?? null);
  const sourceLabel =
    sourceType === 'model' ? (selectedModel?.name ?? '本地模型') : (selectedProvider?.name ?? '未选择');

  const canSend =
    !running &&
    input.trim().length > 0 &&
    (sourceType === 'model' ? !!selectedModelPort : !!selectedProviderId && !!modelName);

  // ===== 事件订阅：见 stores/agent.ts 的模块级单例（ensureAgentListeners） =====

  // ===== 动作 =====
  const scrollToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      const el = document.querySelector('.agent-main .agent-entries');
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, []);

  useEffect(() => {
    scrollToBottom();
  }, [entries, scrollToBottom]);

  const handleSend = useCallback(async () => {
    const raw = input.trim();
    if (!raw || running) return;

    // /技能名 解析：命中已知技能 → 剥离前缀，后端注入技能内容；/xxx 非法 → 提示
    let text = raw;
    let skillName: string | null = null;
    if (raw.startsWith('/')) {
      const parsed = parseSlashCommand(raw, skills);
      if (parsed) {
        skillName = parsed.skill;
        text = parsed.rest;
        if (!text) {
          setError('请输入任务内容，例如 /demo 把需求整理成方案');
          return;
        }
      } else {
        const name = raw.match(/^\/([\w\u4e00-\u9fff.-]+)/)?.[1] ?? '';
        setError(`未知技能 /${name}（可在 设置 → 我的 Skills 页面创建或导入）`);
        return;
      }
    }

    const st = useChatToolbarStore.getState();
    if (st.sourceType === 'model' && !st.selectedModelPort) {
      setError('请先在标题栏选择一个运行中的本地模型');
      return;
    }
    if (st.sourceType === 'provider') {
      const provider = st.providers.find((p) => p.id === st.selectedProviderId) ?? null;
      const name = st.overrideModelName ?? provider?.active_model ?? null;
      if (!provider) {
        setError('请先在标题栏选择一个 API 提供者');
        return;
      }
      if (!name) {
        setError('请先选择模型（标题栏 → 对话设置）');
        return;
      }
    }

    setInput('');
    setError('');
    pushUser(raw); // 会话记录保留原始 /技能名 文本，回放时可读
    setRunState(true, 'analyzing');

    try {
      await invoke('agent_start_run', {
        req: {
          session_id: sessionId,
          user_text: text,
          skill: skillName,
          provider_id: st.sourceType === 'provider' ? st.selectedProviderId : null,
          model_port: st.sourceType === 'model' ? st.selectedModelPort : null,
          model:
            st.sourceType === 'provider'
              ? (st.overrideModelName ?? selectedProvider?.active_model ?? null)
              : null,
          project_root: projectPath || null,
          system_prompt: systemPrompt.trim() || null,
        },
      });
    } catch (e) {
      setRunState(false, null);
      pushTerminal('failed', String(e));
    }
  }, [input, running, projectPath, selectedProvider, systemPrompt, skills, sessionId, setInput, setError, pushUser, setRunState]);

  const handleStop = useCallback(async () => {
    try {
      await invoke('agent_cancel');
    } catch {
      // ignore
    }
    setRunState(false, null);
  }, [setRunState]);

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

  // ===== @ 提及 + / 技能输入处理 =====

  function handleTextareaChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const el = e.currentTarget;
    setInput(el.value);
    mention.detect(el.value, el.selectionStart);
    slash.detect(el.value, el.selectionStart);
  }

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
        setProjectPath(picked);
        setError('');
      }
    } catch (e) {
      setError(`选择文件夹失败: ${e}`);
    }
  }, []);

  // ===== 标题栏挂载点（AppLayout 在 /agent 路由下渲染的 portal 容器） =====
  const [tbLeft, setTbLeft] = useState<HTMLElement | null>(null);
  const [tbRight, setTbRight] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setTbLeft(document.getElementById('agent-titlebar-left'));
    setTbRight(document.getElementById('agent-titlebar-right'));
  }, []);

  const newChat = useCallback(async () => {
    if (running) return;
    const st = useAgentStore.getState();
    void st.saveSession(st.sessionId); // 旧会话落盘后重置（saveSession 内部读 get()，拿到的是保存时的快照）
    st.startNewChat();
    invoke('agent_reset_session', { sessionId: st.sessionId }).catch(() => {
      // 后端不可用时忽略（本地调试）
    });
  }, [running]);

  /** 从历史面板恢复会话（只读回放 + 续聊重新建 Thread） */
  const selectSession = useCallback(
    (s: AgentSessionMeta) => {
      if (running) return;
      const st = useAgentStore.getState();
      void st.saveSession(st.sessionId);
      st.loadSession(s);
      setError('');
    },
    [running, setError],
  );

  const deleteSession = useCallback(
    (id: string) => {
      invoke('delete_agent_session', { id })
        .then(() => {
          void refreshSessions();
          if (useAgentStore.getState().sessionId === id) void newChat();
        })
        .catch((e) => setError(`删除会话失败: ${e}`));
    },
    [newChat, refreshSessions, setError],
  );

  // ===== 渲染 =====
  return (
    <div className="agent-root">
      {/* 标题栏功能：portal 到 AppLayout 的 /agent 标题栏挂载点
          左侧 = 项目文件夹（编辑器式）；右侧 = 模型显示 + 新对话 + 历史（Zed 式） */}
      {tbLeft &&
        createPortal(
          projectPath ? (
            <div className="agent-tb__project">
              <FolderOpen size={14} strokeWidth={1.8} />
              <button
                type="button"
                className="agent-tb__project-name"
                onClick={() => void openProjectInFolder()}
                title={`${projectPath}（点击在 Finder 中打开）`}
              >
                {projects.find((p) => p.path === projectPath)?.name ?? projectPath.split('/').pop()}
              </button>
              <button
                type="button"
                className="agent-tb__iconbtn"
                onClick={openProjectInEditor}
                title="在编辑器中打开"
              >
                <ExternalLink size={13} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className="agent-tb__iconbtn"
                onClick={() => setProjectPath('')}
                disabled={running}
                title="解绑项目（转为纯聊天）"
              >
                <X size={13} strokeWidth={2} />
              </button>
            </div>
          ) : (
            <>
              <button type="button" className="agent-tb__pick" onClick={() => void pickProjectFolder()}>
                <FolderOpen size={14} strokeWidth={1.8} />
                选择项目文件夹
              </button>
              <span className="agent-tb__hint">选择项目后可用 @ 引用文件 · 未选项目为纯聊天</span>
            </>
          ),
          tbLeft,
        )}
      {tbRight &&
        createPortal(
          <>
            <span className="agent-tb__model" title="当前模型（在输入框下方切换）">
              {sourceLabel}
              {modelName ? ` · ${modelName}` : ''}
            </span>
            <button
              type="button"
              className="agent-tb__iconbtn"
              onClick={() => void newChat()}
              disabled={running}
              title="新对话（保留历史，丢弃当前上下文）"
            >
              <Plus size={15} strokeWidth={2} />
            </button>
            <button
              type="button"
              className={`agent-tb__iconbtn${historyOpen ? ' is-active' : ''}`}
              onClick={() => setHistoryOpen((o) => !o)}
              title={historyOpen ? '隐藏历史面板' : '显示历史面板'}
            >
              <History size={15} strokeWidth={1.8} />
            </button>
          </>,
          tbRight,
        )}

      {error && (
        <div className="agent-warn">
          <CircleAlert size={14} strokeWidth={1.8} />
          {error}
          <button type="button" onClick={() => setError('')}>
            <X size={12} strokeWidth={2} />
          </button>
        </div>
      )}

      {/* 主体：左对话流 + 右历史面板 */}
      <div className="agent-body">
        <div className="agent-main">
          <div className="agent-entries">
            {entries.length === 0 && (
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
            )}

            {entries.map((e, i) => (
              <EntryView key={i} entry={e} onDecide={handleDecide} skills={skills} fileSet={fileSet} />
            ))}

            {running && (
              <div className="agent-statusline">
                <Loader2 size={13} strokeWidth={2} className="agent-statusline__spin" />
                {STATUS_LABEL[status ?? ''] ?? '运行中'}
                <button type="button" onClick={() => void handleStop()}>
                  停止
                </button>
              </div>
            )}
          </div>

          {/* 参数设置面板（悬浮在输入区上方） */}
          {showSettings && (
            <SettingsSheet
              systemPrompt={systemPrompt}
              temperature={temperature}
              maxTokens={maxTokens}
              disabled={running}
              onSystemPromptChange={setSystemPrompt}
              onTemperatureChange={setTemperature}
              onMaxTokensChange={setMaxTokens}
            />
          )}

          {/* 输入区（@ 提及 / / 技能弹层与输入框为兄弟节点，对齐 chat 页布局） */}
          <div className="agent-dock-wrap">
            {mention.active && files.length > 0 && (
              <FileMentionPopup
                query={mention.query}
                files={mention.filtered}
                selectedIndex={mention.selectedIndex}
                onSelect={(f) => setInput(mention.pick(f, input))}
              />
            )}
            {slash.active && skills.length > 0 && !mention.active && (
              <SlashPopup
                query={slash.query}
                skills={slash.filtered}
                selectedIndex={slash.selectedIndex}
                onSelect={(s) => setInput(slash.pick(s, input))}
              />
            )}
            <div className="agent-dock">
              <textarea
                ref={textareaRef}
                className="agent-dock__input"
                value={input}
                placeholder={
                  projectPath
                    ? '描述任务，例如：读取 @需求.md 并生成方案保存为 txt…'
                    : '纯聊天模式，顶部选择项目后可用 @ 引用文件 · / 调用技能…'
                }
                disabled={running}
                onChange={handleTextareaChange}
                onKeyDown={(e) => {
                  // @ 提及 / / 技能弹出时优先处理导航；处理过的按键不再走发送
                  if (mention.handleKeydown(e)) return;
                  if (slash.handleKeydown(e)) return;
                  if (isSendEnter(e)) {
                    e.preventDefault();
                    void handleSend();
                  }
                }}
              />
              {/* 底栏：模型切换（原标题栏功能移入）+ 提示 + 设置/清空 + 发送/停止 */}
              <div className="agent-dock__bar">
                <ChatToolbar side="left" />
                <span className="agent-dock__spacer" />
                <div className="agent-dock__hint">
                  <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd> 换行 · <kbd>@</kbd> 引用文件
                </div>
                {running ? (
                  <button type="button" className="agent-dock__stop" onClick={() => void handleStop()}>
                    <span className="agent-dock__stop-square" />
                    停止
                  </button>
                ) : (
                  <button
                    type="button"
                    className="agent-dock__send"
                    disabled={!canSend}
                    onClick={() => void handleSend()}
                    aria-label="发送"
                  >
                    <ArrowRight size={16} strokeWidth={2.5} />
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>

        {historyOpen && (
          <AgentHistoryPanel
            sessions={sessions}
            currentSessionId={sessionId}
            running={running}
            onSelect={(s) => selectSession(s)}
            onDelete={deleteSession}
          />
        )}
      </div>
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

function EntryView({
  entry,
  onDecide,
  skills,
  fileSet,
}: {
  entry: Entry;
  onDecide: (callId: string, allow: boolean) => Promise<void>;
  skills: UserSkillLite[];
  fileSet: Set<string>;
}) {
  switch (entry.kind) {
    case 'user': {
      const { refs, body } = parseUserRefs(entry.text, skills, fileSet);
      return (
        <div className="aentry aentry--user">
          <div className="aentry__bubble">
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
        <div className={`aentry aentry--assistant${entry.streaming ? ' is-streaming' : ''}`}>
          {entry.text ? (
            <div className="aentry__md">
              <MarkdownMessage content={entry.text} />
            </div>
          ) : (
            <div className="aentry__typing">
              <span />
              <span />
              <span />
            </div>
          )}
        </div>
      );
    case 'tool':
      return <ToolEntry entry={entry} onDecide={onDecide} />;
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
              {entry.state === 'allowed' ? '已批准' : '已拒绝'}
            </div>
          )}
        </div>
      );
    }
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
            </>
          )}
          {entry.outcome === 'cancelled' && (
            <>
              <Square size={11} strokeWidth={2} /> 已取消
            </>
          )}
        </div>
      );
  }
}

// ===== 工具卡片（结果默认折叠，点击展开，避免大段文件/终端输出刷屏） =====

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
                  : '失败'}
        </span>
        {hasBody && (
          <ChevronDown size={12} strokeWidth={2} className={`aentry__tool-chev${open ? ' rotated' : ''}`} />
        )}
      </button>
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
            <div className="aentry__artifacts">
              {entry.artifacts.map((a) => (
                <span key={a.path} className="aentry__artifact">
                  <ChevronRight size={11} strokeWidth={2} />
                  {a.path}
                </span>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
