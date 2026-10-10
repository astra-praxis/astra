/**
 * 对话页面（编排层）
 * 移植自 src/routes/chat/+page.svelte。
 *
 * 与原版对应关系：
 * - $state → useState；$derived → 渲染中计算；$effect/onMount/onDestroy → useEffect
 * - 流式事件 chat-chunk / chat-chunk-reasoning / chat-done / chat-error
 *   通过 rxjs Subject 组织（本项目指定的 rxjs 使用位置）
 * - toolbarState（chat-state.svelte.ts）→ useChatToolbarStore
 * - workspaceStore → useWorkspaceStore
 * - 标题栏功能（模型显示/新对话/历史）通过 portal 渲染到 AppLayout 挂载点；
 *   模型切换与设置/清空在输入框底栏（与 Agent 页同布局）
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Subject } from 'rxjs';
import { History, Plus } from 'lucide-react';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import { useWorkspaceStore } from '@/stores/workspace';
import ChatToolbar from '@/components/chat/ChatToolbar';
import type { ChatMessage, ChatSession, ChatSource } from './types';
import { buildWorkspaceContext, parseMentions } from './workspace-context';
import HistoryDrawer from './HistoryDrawer';
import SettingsSheet from './SettingsSheet';
import Welcome from './Welcome';
import ChatMessageView from './ChatMessageView';
import Composer from './Composer';
import './ChatPage.scss';

// ===== 流式事件 =====

type StreamEventType = 'chunk' | 'reasoning' | 'done' | 'error';

interface StreamEvent {
  type: StreamEventType;
  payload?: string;
}

export default function ChatPage() {
  // ===== 布局 =====
  const [sidebarOpen, setSidebarOpen] = useState(false);

  // ===== 当前会话 =====
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [isSending, setIsSendingState] = useState(false);
  const [error, setError] = useState('');

  // ===== 参数（温度/最大 Token 已移除，后端 ChatRequest 两字段均为 Option，不传用默认） =====
  const [systemPrompt, setSystemPrompt] = useState('你是一个有用的助手，请简洁准确地回答用户问题。');

  const [sessions, setSessions] = useState<ChatSession[]>([]);

  // ===== 工具栏 store（与标题栏 ChatToolbar 共享） =====
  const sourceType = useChatToolbarStore((s) => s.sourceType);
  const selectedProviderId = useChatToolbarStore((s) => s.selectedProviderId);
  const selectedModelPort = useChatToolbarStore((s) => s.selectedModelPort);
  const overrideModelName = useChatToolbarStore((s) => s.overrideModelName);
  const showSettings = useChatToolbarStore((s) => s.showSettings);
  const providers = useChatToolbarStore((s) => s.providers);
  const runningModels = useChatToolbarStore((s) => s.runningModels);
  const setIsSendingStore = useChatToolbarStore((s) => s.setIsSending);
  const setSessionsCount = useChatToolbarStore((s) => s.setSessionsCount);

  const workspacePath = useWorkspaceStore((s) => s.activePath);

  // ===== 派生 =====
  const selectedProvider = providers.find((p) => p.id === selectedProviderId) ?? null;
  const selectedModel = runningModels.find((m) => m.port === selectedModelPort) ?? null;

  const currentModelName =
    sourceType === 'model'
      ? (selectedModel?.name ?? 'local')
      : (overrideModelName ?? selectedProvider?.active_model ?? null);

  const currentSourceLabel =
    sourceType === 'model' ? (selectedModel?.name ?? '本地模型') : (selectedProvider?.name ?? '未选择');

  const canSend =
    !isSending &&
    input.trim().length > 0 &&
    (sourceType === 'model' ? !!selectedModelPort : !!selectedProviderId && !!currentModelName);

  // ===== Refs（供事件订阅/回调读取最新值，避免闭包过期） =====
  const messagesElRef = useRef<HTMLDivElement | null>(null);
  const messagesRef = useRef(messages);
  const currentSessionIdRef = useRef(currentSessionId);
  const isSendingRef = useRef(isSending);
  const streamSubjectRef = useRef<Subject<StreamEvent> | null>(null);
  if (!streamSubjectRef.current) streamSubjectRef.current = new Subject<StreamEvent>();
  const saveRef = useRef<() => void>(() => {});

  useEffect(() => {
    messagesRef.current = messages;
  });
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId;
  });
  useEffect(() => {
    isSendingRef.current = isSending;
  });

  const scrollToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      const el = messagesElRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, []);

  /** 同步发送状态到本地 state + toolbar store */
  const setSending = useCallback((v: boolean) => {
    isSendingRef.current = v;
    setIsSendingState(v);
    useChatToolbarStore.getState().setIsSending(v);
  }, []);

  // ===== 生命周期 =====
  useEffect(() => {
    // 加载会话历史
    invoke<ChatSession[]>('list_chat_sessions')
      .then(setSessions)
      .catch((e) => console.error('加载聊天记录失败', e));

    // 优先复用 toolbar store 的已选值；若 provider 已选但模型列表为空则补拉一次
    const st = useChatToolbarStore.getState();
    if (st.sourceType === 'provider' && st.selectedProviderId && st.providerModels.length === 0 && !st.modelsLoading) {
      void st.fetchProviderModels();
    }
  }, []);

  // ===== 会话管理 =====
  function genTitle(msgs: ChatMessage[]): string {
    const firstUser = msgs.find((m) => m.role === 'user');
    if (firstUser) {
      const t = firstUser.content.trim().slice(0, 24);
      return t + (firstUser.content.length > 24 ? '…' : '');
    }
    return '新对话';
  }

  function buildSource(): ChatSource {
    const st = useChatToolbarStore.getState();
    if (st.sourceType === 'model') {
      const model = st.runningModels.find((m) => m.port === st.selectedModelPort);
      return { type: 'model', port: st.selectedModelPort!, model_name: model?.name ?? 'local' };
    }
    return { type: 'provider', provider_id: st.selectedProviderId!, model: st.overrideModelName };
  }

  const saveCurrentSession = useCallback(async () => {
    const msgs = messagesRef.current;
    if (msgs.length === 0) return;

    const payloadMessages = msgs.map((m) => ({
      role: m.role,
      content: m.content,
      reasoning: m.reasoning,
      timestamp: m.timestamp,
      error: m.error,
      favorite: m.favorite,
    }));

    try {
      const saved = await invoke<ChatSession>('save_chat_session', {
        id: currentSessionIdRef.current,
        title: genTitle(msgs),
        source: buildSource(),
        messages: payloadMessages,
      });
      currentSessionIdRef.current = saved.id;
      setCurrentSessionId(saved.id);

      setSessions((prev) => {
        const idx = prev.findIndex((s) => s.id === saved.id);
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = saved;
          next.sort((a, b) => b.updated_at - a.updated_at);
          return next;
        }
        return [saved, ...prev];
      });
    } catch (e) {
      console.error('保存会话失败', e);
    }
  }, []);

  useEffect(() => {
    saveRef.current = () => void saveCurrentSession();
  }, [saveCurrentSession]);

  // ===== 流式事件订阅（rxjs Subject 组织） =====
  useEffect(() => {
    const subject = streamSubjectRef.current!;

    const listeners: Promise<UnlistenFn>[] = [
      listen<string>('chat-chunk', (e) => subject.next({ type: 'chunk', payload: e.payload })),
      listen<string>('chat-chunk-reasoning', (e) => subject.next({ type: 'reasoning', payload: e.payload })),
      listen('chat-done', () => subject.next({ type: 'done' })),
      listen<string>('chat-error', (e) => subject.next({ type: 'error', payload: e.payload })),
    ];

    const subscription = subject.subscribe((ev) => {
      switch (ev.type) {
        case 'chunk':
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (!last || last.role !== 'assistant') return prev;
            return [...prev.slice(0, -1), { ...last, content: last.content + (ev.payload ?? '') }];
          });
          scrollToBottom();
          break;

        case 'reasoning':
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (!last || last.role !== 'assistant') return prev;
            return [...prev.slice(0, -1), { ...last, reasoning: (last.reasoning ?? '') + (ev.payload ?? '') }];
          });
          scrollToBottom();
          break;

        case 'done':
          setSending(false);
          saveRef.current();
          break;

        case 'error': {
          setSending(false);
          const msgs = messagesRef.current;
          const last = msgs[msgs.length - 1];
          if (last && last.role === 'assistant' && !last.content) {
            setMessages((prev) => {
              const arr = [...prev];
              const l = arr[arr.length - 1];
              if (l && l.role === 'assistant' && !l.content) {
                arr[arr.length - 1] = { ...l, content: ev.payload ?? '', error: true };
              }
              return arr;
            });
          } else {
            setError(ev.payload ?? '');
          }
          break;
        }
      }
    });

    return () => {
      subscription.unsubscribe();
      Promise.all(listeners).then((fns) => fns.forEach((fn) => fn()));
    };
  }, [scrollToBottom, setSending]);

  // ===== 标题栏挂载点（AppLayout 在 /chat 路由下渲染的 portal 容器） =====
  const [tbRight, setTbRight] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setTbRight(document.getElementById('chat-titlebar-right'));
  }, []);

  // ===== 状态同步到工具栏 store =====
  useEffect(() => {
    setIsSendingStore(isSending);
  }, [isSending, setIsSendingStore]);

  useEffect(() => {
    setSessionsCount(sessions.length);
  }, [sessions.length, setSessionsCount]);

  // 来源切换时清空错误（对应原版 onSwitchType 中的 error = ''）
  const prevSourceTypeRef = useRef(sourceType);
  useEffect(() => {
    if (prevSourceTypeRef.current !== sourceType) {
      prevSourceTypeRef.current = sourceType;
      setError('');
    }
  }, [sourceType]);

  // ===== 发送 =====
  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || isSendingRef.current) return;

    const st = useChatToolbarStore.getState();

    if (st.sourceType === 'model' && !st.selectedModelPort) {
      setError('请先选择一个运行中的本地模型');
      return;
    }
    if (st.sourceType === 'provider') {
      const provider = st.providers.find((p) => p.id === st.selectedProviderId) ?? null;
      const modelName = st.overrideModelName ?? provider?.active_model ?? null;
      if (!provider) {
        setError('请先选择一个 API 提供者');
        return;
      }
      if (!modelName) {
        setError('请先点击"刷新模型列表"并选择一个模型');
        return;
      }
    }

    setInput('');
    setError('');

    const userMsg: ChatMessage = { role: 'user', content: text, timestamp: Date.now() };
    const placeholder: ChatMessage = { role: 'assistant', content: '', timestamp: Date.now(), isFresh: true };
    const newMessages = [...messagesRef.current, userMsg, placeholder];
    messagesRef.current = newMessages;
    setMessages(newMessages);
    scrollToBottom();

    setSending(true);

    const context: Array<{ role: string; content: string }> = [];

    // 构建增强的系统提示（包含工作空间信息）
    const enhancedSystemPrompt = systemPrompt.trim() + buildWorkspaceContext();
    if (enhancedSystemPrompt) {
      context.push({ role: 'system', content: enhancedSystemPrompt });
    }

    // 解析用户消息中的 @ 提及，如果有提及则补充项目信息
    const mentions = parseMentions(text);
    let enhancedUserContent = text;
    if (mentions.length > 0) {
      const mentionInfo = mentions.map((p) => `【引用项目】${p.name}: ${p.path}`).join('\n');
      enhancedUserContent = `${text}\n\n${mentionInfo}`;
    }

    for (let i = 0; i < newMessages.length - 1; i++) {
      const m = newMessages[i];
      if (m.error) continue;
      if (i === newMessages.length - 2 && m.role === 'user') {
        // 最后一条用户消息使用增强版本
        context.push({ role: 'user', content: enhancedUserContent });
      } else {
        context.push({ role: m.role, content: m.content });
      }
    }

    let source: ChatSource;
    if (st.sourceType === 'model') {
      source = buildSource();
    } else {
      const provider = st.providers.find((p) => p.id === st.selectedProviderId);
      source = {
        type: 'provider',
        provider_id: st.selectedProviderId!,
        model: st.overrideModelName ?? provider?.active_model ?? null,
      };
    }

    try {
      await invoke('send_chat', {
        req: {
          source,
          messages: context,
        },
      });
    } catch (e) {
      setSending(false);
      const last = messagesRef.current[messagesRef.current.length - 1];
      if (last && !last.content) {
        setMessages((prev) => {
          const arr = [...prev];
          const l = arr[arr.length - 1];
          if (l && !l.content) {
            arr[arr.length - 1] = { ...l, content: `❌ 调用失败: ${e}`, error: true };
          }
          return arr;
        });
      } else {
        setError(String(e));
      }
    }
  }, [input, systemPrompt, scrollToBottom, setSending]);

  const handleStop = useCallback(async () => {
    try {
      await invoke('stop_chat');
    } catch {
      // ignore
    }
    setSending(false);
  }, [setSending]);

  const newChat = useCallback(() => {
    if (isSendingRef.current) return;
    currentSessionIdRef.current = null;
    setCurrentSessionId(null);
    setMessages([]);
    messagesRef.current = [];
    setError('');
    setInput('');
    useChatToolbarStore.getState().setOverrideModelName(null);
    setSidebarOpen(false);
  }, []);

  const selectSession = useCallback(
    async (s: ChatSession) => {
      if (isSendingRef.current) return;
      currentSessionIdRef.current = s.id;
      setCurrentSessionId(s.id);
      setError('');
      setInput('');

      const restored: ChatMessage[] = (s.messages as ChatMessage[]).map((m) => ({
        role: m.role as 'user' | 'assistant',
        content: m.content,
        reasoning: m.reasoning,
        timestamp: m.timestamp ?? 0,
        error: m.error,
        favorite: m.favorite,
        showReasoning: false,
        isFresh: false, // 历史消息不自动执行 action
      }));
      setMessages(restored);
      messagesRef.current = restored;

      const st = useChatToolbarStore.getState();
      const src = s.source;
      if (src.type === 'provider') {
        st.setSourceType('provider');
        const provider = st.providers.find((p) => p.id === src.provider_id);
        st.selectProvider(src.provider_id);
        st.setOverrideModelName(src.model ?? provider?.active_model ?? null);
      } else if (src.type === 'model') {
        st.setSourceType('model');
        st.setSelectedModelPort(src.port);
      }

      setSidebarOpen(false);
      scrollToBottom();
    },
    [scrollToBottom],
  );

  const deleteSession = useCallback(
    (id: string) => {
      if (isSendingRef.current) return;
      invoke('delete_chat_session', { id })
        .then(() => {
          setSessions((prev) => prev.filter((s) => s.id !== id));
          if (currentSessionIdRef.current === id) newChat();
        })
        .catch((err) => {
          setError(`删除失败: ${err}`);
        });
    },
    [newChat],
  );

  // ===== 消息操作 =====
  const copyMessage = useCallback((msg: ChatMessage) => {
    navigator.clipboard.writeText(msg.content).catch(() => {
      // ignore
    });
  }, []);

  const toggleFavorite = useCallback((msg: ChatMessage) => {
    setMessages((prev) => prev.map((m) => (m === msg ? { ...m, favorite: !m.favorite } : m)));
    // 自动保存
    saveRef.current();
  }, []);

  const toggleReasoning = useCallback((index: number) => {
    setMessages((prev) => prev.map((m, i) => (i === index ? { ...m, showReasoning: !m.showReasoning } : m)));
  }, []);

  // ===== 渲染 =====
  return (
    <div className={`chat-root${sidebarOpen ? ' with-sidebar' : ''}`}>
      {/* 标题栏右侧：模型显示 + 新对话 + 历史（Zed 式，portal 到 AppLayout 挂载点） */}
      {tbRight &&
        createPortal(
          <>
            <span className="agent-tb__model" title="当前模型（在输入框下方切换）">
              {currentSourceLabel}
              {currentModelName ? ` · ${currentModelName}` : ''}
            </span>
            <button
              type="button"
              className="agent-tb__iconbtn"
              onClick={newChat}
              disabled={isSending}
              title="新对话"
            >
              <Plus size={15} strokeWidth={2} />
            </button>
            <button
              type="button"
              className={`agent-tb__iconbtn${sidebarOpen ? ' is-active' : ''}`}
              onClick={() => setSidebarOpen((o) => !o)}
              title={sidebarOpen ? '隐藏历史面板' : '历史会话'}
            >
              <History size={15} strokeWidth={1.8} />
            </button>
          </>,
          tbRight,
        )}

      <HistoryDrawer
        open={sidebarOpen}
        sessions={sessions}
        currentSessionId={currentSessionId}
        isSending={isSending}
        providers={providers}
        onClose={() => setSidebarOpen(false)}
        onNewChat={newChat}
        onSelect={(s) => void selectSession(s)}
        onDelete={deleteSession}
      />

      {/* ===== 主对话区 ===== */}
      <section className="chat-main">
        {showSettings && (
          <SettingsSheet
            systemPrompt={systemPrompt}
            disabled={isSending}
            onSystemPromptChange={setSystemPrompt}
          />
        )}

        {error && (
          <div className="floating-warn">
            ⚠️ {error}
            <button type="button" onClick={() => setError('')}>
              ✕
            </button>
          </div>
        )}

        {/* 消息流 */}
        <div className="messages" ref={messagesElRef}>
          {messages.length === 0 && (
            <Welcome sourceLabel={currentSourceLabel} modelName={currentModelName} onPick={setInput} />
          )}

          {messages.map((msg, i) => (
            <ChatMessageView
              key={i}
              msg={msg}
              isLast={i === messages.length - 1}
              isSending={isSending}
              workspacePath={workspacePath}
              onCopy={copyMessage}
              onToggleFavorite={toggleFavorite}
              onToggleReasoning={() => toggleReasoning(i)}
            />
          ))}
        </div>

        {/* 输入区（含 @ 提及弹窗；底栏左侧模型切换、右侧设置/清空） */}
        <div className="dock">
          <Composer
            value={input}
            onValueChange={setInput}
            disabled={isSending}
            canSend={canSend}
            onSend={handleSend}
            onStop={handleStop}
            toolbarSlot={<ChatToolbar side="left" />}
          />
        </div>
      </section>
    </div>
  );
}
