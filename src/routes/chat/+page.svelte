<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount, onDestroy, tick } from 'svelte';

  // ===== 类型 =====
  interface RunningModelInfo {
    name: string;
    port: number;
    status: string;
    pid: number | null;
  }
  interface ProviderConfig {
    id: string;
    name: string;
    base_url: string;
    api_key: string;
    active_model: string | null;
  }
  interface ModelInfo {
    id: string;
    owned_by: string | null;
  }
  interface ChatMessage {
    role: 'user' | 'assistant';
    content: string;
    reasoning?: string;
    timestamp: number;
    error?: boolean;
  }
  type ChatSource =
    | { type: 'model'; port: number; model_name: string }
    | { type: 'provider'; provider_id: string; model: string | null };

  interface ChatSession {
    id: string;
    title: string;
    source: ChatSource;
    messages: Array<{ role: string; content: string; reasoning?: string; timestamp?: number; error?: boolean }>;
    created_at: number;
    updated_at: number;
  }

  // ===== 布局 =====
  let sidebarOpen = $state(false);

  // ===== 来源数据 =====
  let runningModels = $state<RunningModelInfo[]>([]);
  let providers = $state<ProviderConfig[]>([]);
  let providerModels = $state<ModelInfo[]>([]);
  let modelsLoading = $state(false);
  let modelsError = $state('');

  // ===== 当前会话 =====
  let currentSessionId = $state<string | null>(null);
  let messages = $state<ChatMessage[]>([]);
  let input = $state('');
  let isSending = $state(false);
  let error = $state('');
  let messagesEl: HTMLElement | null = null;

  let sourceType = $state<'provider' | 'model'>('provider');
  let selectedProviderId = $state<string | null>(null);
  let selectedModelPort = $state<number | null>(null);
  let overrideModelName = $state<string | null>(null);

  // ===== 参数 =====
  let showSettings = $state(false);
  let systemPrompt = $state('你是一个有用的助手，请简洁准确地回答用户问题。');
  let temperature = $state(0.7);
  let maxTokens = $state(4000);

  let sessions = $state<ChatSession[]>([]);

  let unlisteners: Array<() => void> = [];

  // ===== 生命周期 =====
  onMount(async () => {
    await Promise.all([loadSessions(), loadRunningModels(), loadProviders()]);

    if (providers.length > 0) {
      selectedProviderId = providers[0].id;
      handleFetchModels();
    }

    if (runningModels.length > 0) {
      sourceType = 'model';
      selectedModelPort = runningModels[0].port;
    }

    const unChunk = await listen<string>('chat-chunk', (e) => {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.content += e.payload;
        messages = [...messages];
        scrollToBottom();
      }
    });

    const unReasoning = await listen<string>('chat-chunk-reasoning', (e) => {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.reasoning = (last.reasoning ?? '') + e.payload;
        messages = [...messages];
        scrollToBottom();
      }
    });

    const unDone = await listen('chat-done', () => {
      isSending = false;
      saveCurrentSession();
    });

    const unError = await listen<string>('chat-error', (e) => {
      isSending = false;
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant' && !last.content) {
        last.content = e.payload;
        last.error = true;
        messages = [...messages];
      } else {
        error = e.payload;
      }
    });

    unlisteners.push(unChunk, unReasoning, unDone, unError);
  });

  onDestroy(() => {
    unlisteners.forEach((fn) => fn());
  });

  // ===== 派生 =====
  const selectedProvider = $derived(providers.find((p) => p.id === selectedProviderId) ?? null);
  const selectedModel = $derived(runningModels.find((m) => m.port === selectedModelPort) ?? null);

  const currentModelName = $derived.by(() => {
    if (sourceType === 'model') return selectedModel?.name ?? 'local';
    return overrideModelName ?? selectedProvider?.active_model ?? null;
  });

  const currentSourceLabel = $derived.by(() => {
    if (sourceType === 'model') return selectedModel?.name ?? '本地模型';
    return selectedProvider?.name ?? '未选择';
  });

  const canSend = $derived.by(() => {
    if (isSending || input.trim().length === 0) return false;
    if (sourceType === 'model') return !!selectedModelPort;
    return !!selectedProviderId && !!currentModelName;
  });

  // ===== 加载 =====
  async function loadSessions() {
    try {
      sessions = await invoke<ChatSession[]>('list_chat_sessions');
    } catch (e) {
      console.error('加载聊天记录失败', e);
    }
  }

  async function loadRunningModels() {
    try {
      const all = await invoke<RunningModelInfo[]>('list_running_servers');
      runningModels = all.filter((m) => m.status === 'running');
    } catch (e) {
      console.error('加载运行中的模型失败', e);
    }
  }

  async function loadProviders() {
    try {
      providers = await invoke<ProviderConfig[]>('list_providers');
    } catch (e) {
      console.error('加载 providers 失败', e);
    }
  }

  async function handleFetchModels() {
    if (!selectedProvider) {
      modelsError = '请先选择一个 Provider';
      return;
    }
    modelsLoading = true;
    modelsError = '';
    try {
      providerModels = await invoke<ModelInfo[]>('fetch_provider_models', {
        baseUrl: selectedProvider.base_url,
        apiKey: selectedProvider.api_key,
      });
    } catch (e) {
      modelsError = String(e);
      providerModels = [];
    } finally {
      modelsLoading = false;
    }
  }

  function onSwitchType(t: 'provider' | 'model') {
    if (sourceType === t || isSending) return;
    sourceType = t;
    error = '';
    if (t === 'model' && runningModels.length > 0 && !selectedModelPort) {
      selectedModelPort = runningModels[0].port;
    }
    if (t === 'provider' && providers.length > 0 && !selectedProviderId) {
      selectedProviderId = providers[0].id;
      handleFetchModels();
    }
  }

  function onSelectProviderChange() {
    overrideModelName = null;
    providerModels = [];
    if (selectedProviderId) handleFetchModels();
  }

  function newChat() {
    if (isSending) return;
    currentSessionId = null;
    messages = [];
    error = '';
    input = '';
    overrideModelName = null;
    sidebarOpen = false;
  }

  async function selectSession(s: ChatSession) {
    if (isSending) return;
    currentSessionId = s.id;
    error = '';
    input = '';

    messages = (s.messages as ChatMessage[]).map((m) => ({
      role: m.role as 'user' | 'assistant',
      content: m.content,
      reasoning: m.reasoning,
      timestamp: m.timestamp ?? 0,
      error: m.error,
    }));

    if (s.source.type === 'provider') {
      sourceType = 'provider';
      const sid = s.source.provider_id;
      selectedProviderId = sid;
      const provider = providers.find((p) => p.id === sid);
      overrideModelName = s.source.model ?? provider?.active_model ?? null;
      if (provider) handleFetchModels();
    } else if (s.source.type === 'model') {
      sourceType = 'model';
      selectedModelPort = s.source.port;
    }

    sidebarOpen = false;
    await scrollToBottom();
  }

  function deleteSession(id: string, e: Event) {
    e.stopPropagation();
    if (isSending) return;
    invoke('delete_chat_session', { id })
      .then(() => {
        sessions = sessions.filter((s) => s.id !== id);
        if (currentSessionId === id) newChat();
      })
      .catch((err) => {
        error = `删除失败: ${err}`;
      });
  }

  function genTitle(): string {
    const firstUser = messages.find((m) => m.role === 'user');
    if (firstUser) {
      const t = firstUser.content.trim().slice(0, 24);
      return t + (firstUser.content.length > 24 ? '…' : '');
    }
    return '新对话';
  }

  function buildSource(): ChatSource {
    if (sourceType === 'model') {
      return {
        type: 'model',
        port: selectedModelPort!,
        model_name: selectedModel?.name ?? 'local',
      };
    }
    return {
      type: 'provider',
      provider_id: selectedProviderId!,
      model: overrideModelName,
    };
  }

  async function saveCurrentSession() {
    if (messages.length === 0) return;

    const source = buildSource();
    const payloadMessages = messages.map((m) => ({
      role: m.role,
      content: m.content,
      reasoning: m.reasoning,
      timestamp: m.timestamp,
      error: m.error,
    }));

    try {
      const saved = await invoke<ChatSession>('save_chat_session', {
        id: currentSessionId,
        title: genTitle(),
        source,
        messages: payloadMessages,
      });
      currentSessionId = saved.id;

      const idx = sessions.findIndex((s) => s.id === saved.id);
      if (idx >= 0) {
        sessions[idx] = saved;
        sessions = [...sessions].sort((a, b) => b.updated_at - a.updated_at);
      } else {
        sessions = [saved, ...sessions];
      }
    } catch (e) {
      console.error('保存会话失败', e);
    }
  }

  async function handleSend() {
    const text = input.trim();
    if (!text || isSending) return;

    if (sourceType === 'model' && !selectedModelPort) {
      error = '请先选择一个运行中的本地模型';
      return;
    }
    if (sourceType === 'provider') {
      if (!selectedProvider) {
        error = '请先选择一个 API 提供者';
        return;
      }
      if (!currentModelName) {
        error = `请先点击"刷新模型列表"并选择一个模型`;
        return;
      }
    }

    input = '';
    error = '';

    const userMsg: ChatMessage = { role: 'user', content: text, timestamp: Date.now() };
    const placeholder: ChatMessage = { role: 'assistant', content: '', timestamp: Date.now() };
    messages = [...messages, userMsg, placeholder];
    await scrollToBottom();

    isSending = true;

    const context: Array<{ role: string; content: string }> = [];
    if (systemPrompt.trim()) {
      context.push({ role: 'system', content: systemPrompt.trim() });
    }
    for (let i = 0; i < messages.length - 1; i++) {
      if (messages[i].error) continue;
      context.push({ role: messages[i].role, content: messages[i].content });
    }

    let source: ChatSource;
    if (sourceType === 'model') {
      source = buildSource();
    } else {
      source = {
        type: 'provider',
        provider_id: selectedProviderId!,
        model: currentModelName,
      };
    }

    try {
      await invoke('send_chat', {
        req: {
          source,
          messages: context,
          max_tokens: maxTokens,
          temperature,
        },
      });
    } catch (e) {
      isSending = false;
      const last = messages[messages.length - 1];
      if (last && !last.content) {
        last.content = `❌ 调用失败: ${e}`;
        last.error = true;
        messages = [...messages];
      } else {
        error = String(e);
      }
    }
  }

  async function handleStop() {
    try {
      await invoke('stop_chat');
    } catch {
      // ignore
    }
    isSending = false;
  }

  function handleClear() {
    if (isSending) return;
    messages = [];
    error = '';
    currentSessionId = null;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  async function scrollToBottom() {
    await tick();
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  function formatTime(ts: number): string {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    const now = new Date();
    const isToday = d.toDateString() === now.toDateString();
    const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
    if (isToday) return time;
    return `${d.getMonth() + 1}/${d.getDate()} ${time}`;
  }

  function sourceIcon(s: ChatSource): string {
    if (s.type === 'model') return '🖥';
    return '⚡';
  }

  function sourceShortLabel(s: ChatSource): string {
    if (s.type === 'model') return `port ${s.port}`;
    const p = providers.find((x) => x.id === s.provider_id);
    return p?.name ?? 'provider';
  }
</script>

<div class="chat-root" class:with-sidebar={sidebarOpen}>
  <!-- ===== 历史抽屉（右侧滑入） ===== -->
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  {#if sidebarOpen}
    <div class="drawer-veil" onclick={() => (sidebarOpen = false)} role="presentation"></div>
  {/if}
  <aside class="history-drawer" class:open={sidebarOpen}>
    <div class="hd-top">
      <span class="hd-title">对话历史</span>
      <button class="hd-close" onclick={() => (sidebarOpen = false)} aria-label="关闭">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
      </button>
    </div>

    <button class="hd-new" onclick={newChat} disabled={isSending}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
      新建对话
    </button>

    <div class="hd-list">
      {#if sessions.length === 0}
        <div class="hd-empty">
          <span class="hd-empty-icon">✦</span>
          <p>暂无历史记录</p>
        </div>
      {:else}
        {#each sessions as s (s.id)}
          <div
            class="hd-item"
            class:active={s.id === currentSessionId}
            onclick={() => selectSession(s)}
            role="button"
            tabindex="0"
            onkeydown={(e) => e.key === 'Enter' && selectSession(s)}
          >
            <span class="hd-item-icon">{sourceIcon(s.source)}</span>
            <div class="hd-item-text">
              <div class="hd-item-title">{s.title}</div>
              <div class="hd-item-meta">
                <span class="hd-item-source">{sourceShortLabel(s.source)}</span>
                <span class="hd-item-dot">·</span>
                <span class="hd-item-time">{formatTime(s.updated_at)}</span>
              </div>
            </div>
            <button class="hd-item-del" onclick={(e) => deleteSession(s.id, e)} aria-label="删除" title="删除">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            </button>
          </div>
        {/each}
      {/if}
    </div>
  </aside>

  <!-- ===== 主对话区 ===== -->
  <section class="chat-main">
    <!-- 顶部状态栏 -->
    <header class="top-bar">
      <!-- 当前来源 chip -->
      <div class="source-chip">
        <span class="source-chip-dot" class:model={sourceType === 'model'}></span>
        <span class="source-chip-text">{currentSourceLabel}</span>
        {#if currentModelName}
          <span class="source-chip-model">{currentModelName}</span>
        {/if}
      </div>

      <div class="tb-spacer"></div>

      <button class="tb-btn-text" onclick={handleClear} disabled={isSending || messages.length === 0} title="清空当前对话">
        清空
      </button>

      <button class="tb-btn" onclick={() => (sidebarOpen = !sidebarOpen)} title="历史记录" aria-label="历史记录">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="18" x2="21" y2="18"/></svg>
        {#if sessions.length > 0}
          <span class="tb-badge">{sessions.length}</span>
        {/if}
      </button>
    </header>

    <!-- 源选择条（浮动下方选择器） -->
    <div class="source-bar" class:settings-open={showSettings}>
      <div class="seg-control">
        <button class:active={sourceType === 'provider'} onclick={() => onSwitchType('provider')}>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
          API
        </button>
        <button class:active={sourceType === 'model'} onclick={() => onSwitchType('model')}>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
          本地
        </button>
      </div>

      {#if sourceType === 'provider'}
        <select class="picker" bind:value={selectedProviderId} onchange={onSelectProviderChange} disabled={isSending}>
          {#if providers.length === 0}
            <option value={null}>尚未配置 Provider</option>
          {/if}
          {#each providers as p (p.id)}
            <option value={p.id}>{p.name}</option>
          {/each}
        </select>

        <!-- 模型切换 -->
        <div class="model-picker-group">
          <select
            class="picker mono"
            value={currentModelName}
            onchange={(e) => (overrideModelName = (e.target as HTMLSelectElement).value || null)}
            disabled={isSending || modelsLoading}
            title="选择模型"
          >
            {#if !currentModelName}
              <option value="">未选择模型</option>
            {/if}
            {#if modelsLoading}
              <option value="">加载中…</option>
            {/if}
            {#if currentModelName && providerModels.findIndex((m) => m.id === currentModelName) < 0}
              <option value={currentModelName}>{currentModelName}</option>
            {/if}
            {#each providerModels as m (m.id)}
              <option value={m.id}>{m.id}</option>
            {/each}
          </select>
          <button class="refresh-btn" onclick={handleFetchModels} disabled={isSending || modelsLoading} title="刷新模型列表" aria-label="刷新模型">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class:spinning={modelsLoading}><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
          </button>
        </div>
      {:else}
        <select class="picker mono" bind:value={selectedModelPort} disabled={isSending}>
          {#if runningModels.length === 0}
            <option value={null}>无运行中模型</option>
          {/if}
          {#each runningModels as m (m.port)}
            <option value={m.port}>{m.name} :{m.port}</option>
          {/each}
        </select>
        <button class="refresh-btn" onclick={loadRunningModels} disabled={isSending} title="刷新" aria-label="刷新">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
        </button>
      {/if}

      <div class="spacer"></div>

      <button class="settings-trigger" class:active={showSettings} onclick={() => (showSettings = !showSettings)} title="参数" aria-label="参数">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg>
      </button>
    </div>

    {#if showSettings}
      <div class="settings-sheet">
        <div class="ss-field">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label>系统提示</label>
          <textarea bind:value={systemPrompt} rows="2" disabled={isSending} placeholder="为对话设定角色和约束"></textarea>
        </div>
        <div class="ss-row">
          <div class="ss-field-inline">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>温度</label>
            <input type="number" step="0.1" min="0" max="2" bind:value={temperature} disabled={isSending} />
          </div>
          <div class="ss-field-inline">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>最大 Token</label>
            <input type="number" step="100" min="1" bind:value={maxTokens} disabled={isSending} />
          </div>
        </div>
      </div>
    {/if}

    {#if modelsError}
      <div class="floating-warn">⚡ {modelsError}</div>
    {/if}
    {#if error}
      <div class="floating-warn">⚠️ {error}<button onclick={() => (error = '')}>✕</button></div>
    {/if}

    <!-- 消息流 -->
    <div class="messages" bind:this={messagesEl}>
      {#if messages.length === 0}
        <!-- 空状态 - 居中欢迎 -->
        <div class="welcome">
          <div class="welcome-orb"></div>
          <h1 class="welcome-greet">有什么可以帮你？</h1>
          <p class="welcome-sub">
            连接到 <strong>{currentSourceLabel}</strong>
            {#if currentModelName}<span class="welcome-model">{currentModelName}</span>{/if}
            开始对话
          </p>
          <div class="welcome-examples">
            <button onclick={() => (input = '帮我解释一下什么是 Transformer 架构')}>
              <span>💡</span> 解释 Transformer 架构
            </button>
            <button onclick={() => (input = '用 Python 实现一个快速排序')}>
              <span>⚡</span> Python 快速排序
            </button>
            <button onclick={() => (input = '帮我把这段话翻译成英文：今天天气很好')}>
              <span>🌐</span> 中英翻译
            </button>
            <button onclick={() => (input = '给我讲一个关于程序员的冷笑话')}>
              <span>😄</span> 来个冷笑话
            </button>
          </div>
        </div>
      {/if}

      {#each messages as msg, i (i)}
        <article class="msg" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'}>
          {#if msg.reasoning}
            <details class="reasoning">
              <summary><span class="reasoning-dot"></span>推理过程</summary>
              <pre class="reasoning-text">{msg.reasoning}</pre>
            </details>
          {/if}

          <div class="msg-content" class:err={msg.error}>
            {#if !msg.content && isSending && i === messages.length - 1}
              <div class="typing">
                <span></span><span></span><span></span>
              </div>
            {:else}
              <pre class="msg-text">{msg.content}</pre>
            {/if}
          </div>
        </article>
      {/each}
    </div>

    <!-- 输入区 -->
    <div class="dock">
      <div class="composer" class:sending={isSending}>
        <textarea
          bind:value={input}
          onkeydown={handleKeydown}
          disabled={isSending}
          rows="2"
          placeholder={isSending ? '生成中…' : '问点什么 '}
        ></textarea>
        <div class="composer-side">
          <div class="composer-hint">
            <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd> 换行
          </div>
          {#if isSending}
            <button class="stop-btn" onclick={handleStop}>
              <span class="stop-square"></span>
              停止
            </button>
          {:else}
            <button class="send-btn" onclick={handleSend} disabled={!canSend} aria-label="发送">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="5" y1="12" x2="19" y2="12"/><polyline points="12 5 19 12 12 19"/></svg>
            </button>
          {/if}
        </div>
      </div>
    </div>
  </section>
</div>

<style>
  .chat-root {
    --chat-gap: 16px;
    --dock-max: 760px;
    --msg-max: 760px;

    height: 100%;
    padding: 0;
    overflow: hidden;
    display: flex;
    background:
      radial-gradient(ellipse 80% 60% at 50% -10%, var(--accent-light), transparent 70%),
      var(--bg-app);
  }

  /* ===== 历史抽屉 ===== */
  .drawer-veil {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 40;
    animation: fadeIn 0.2s ease;
  }

  .history-drawer {
    position: fixed;
    right: 0;
    top: 0;
    bottom: 0;
    z-index: 41;
    width: 280px;
    max-width: 86vw;
    background: var(--bg-card);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    transform: translateX(100%);
    transition: transform 0.28s cubic-bezier(0.32, 0.72, 0, 1);
    box-shadow: -4px 0 24px rgba(0, 0, 0, 0.08);
  }

  .history-drawer.open {
    transform: translateX(0);
    box-shadow: -4px 0 32px rgba(0, 0, 0, 0.15);
  }

  .hd-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 16px 12px;
  }

  .hd-title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .hd-close {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border: none;
    border-radius: 6px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.15s;
  }

  .hd-close:hover {
    color: var(--accent);
    background: var(--accent-bg);
  }

  .hd-new {
    margin: 0 12px 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 9px 12px;
    background: var(--accent);
    border: none;
    border-radius: 10px;
    font-size: 13px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .hd-new:hover:not(:disabled) {
    background: var(--accent-hover);
    box-shadow: 0 6px 16px var(--accent-shadow);
  }

  .hd-new:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .hd-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px 12px;
  }

  .hd-empty {
    padding: 60px 16px;
    text-align: center;
    color: var(--text-muted);
  }

  .hd-empty-icon {
    font-size: 28px;
    color: var(--accent);
    display: block;
    margin-bottom: 8px;
  }

  .hd-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 8px;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s;
    position: relative;
  }

  .hd-item:hover {
    background: var(--bg-subtle);
  }

  .hd-item.active {
    background: var(--accent-bg);
  }

  .hd-item.active::before {
    content: '';
    position: absolute;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 3px;
    height: 60%;
    background: var(--accent);
    border-radius: 3px 0 0 3px;
  }

  .hd-item-icon {
    font-size: 13px;
    margin-top: 1px;
    opacity: 0.8;
  }

  .hd-item-text {
    flex: 1;
    min-width: 0;
  }

  .hd-item-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-bottom: 3px;
  }

  .hd-item-meta {
    display: flex;
    gap: 4px;
    font-size: 11px;
    color: var(--text-muted);
    align-items: center;
  }

  .hd-item-source {
    font-family: ui-monospace, monospace;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hd-item-del {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: 5px;
    opacity: 0;
    transition: all 0.15s;
    flex-shrink: 0;
    margin-top: -2px;
  }

  .hd-item:hover .hd-item-del,
  .hd-item.active .hd-item-del {
    opacity: 0.7;
  }

  .hd-item-del:hover {
    opacity: 1 !important;
    color: var(--error-text);
    background: var(--error-bg);
  }

  /* ===== 主区 ===== */
  .chat-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }

  /* ===== 顶部状态栏 ===== */
  .top-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 24px;
    border-bottom: 1px solid var(--border-light);
    background: color-mix(in srgb, var(--bg-card) 70%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    position: relative;
    z-index: 5;
  }

  .tb-btn {
    position: relative;
    width: 34px;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: 9px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.18s;
  }

  .tb-btn:hover {
    background: var(--bg-subtle);
    color: var(--text-primary);
  }

  .tb-badge {
    position: absolute;
    top: 3px;
    right: 3px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    background: var(--accent);
    color: white;
    font-size: 10px;
    font-weight: 700;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
  }

  .source-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 500;
  }

  .source-chip-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent-shadow);
    flex-shrink: 0;
  }

  .source-chip-dot.model {
    background: #10b981;
    box-shadow: 0 0 8px rgba(16, 185, 129, 0.4);
  }

  .source-chip-text {
    color: var(--text-primary);
    font-weight: 600;
  }

  .source-chip-model {
    padding-left: 8px;
    margin-left: 4px;
    border-left: 1px solid var(--border);
    color: var(--accent);
    font-family: ui-monospace, monospace;
    font-size: 11px;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tb-spacer { flex: 1; }

  .tb-btn-text {
    padding: 6px 12px;
    background: transparent;
    border: none;
    border-radius: 7px;
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .tb-btn-text:hover:not(:disabled) {
    color: var(--accent);
    background: var(--accent-bg);
  }

  .tb-btn-text:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ===== 源选择条 ===== */
  .source-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 24px;
    flex-wrap: wrap;
  }

  .seg-control {
    display: inline-flex;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 3px;
    gap: 2px;
  }

  .seg-control button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 12px;
    background: transparent;
    border: none;
    border-radius: 7px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.2s;
  }

  .seg-control button:hover {
    color: var(--text-primary);
  }

  .seg-control button.active {
    background: var(--accent);
    color: white;
    box-shadow: 0 2px 6px var(--accent-shadow);
  }

  .picker {
    padding: 7px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 9px;
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    outline: none;
    transition: all 0.15s;
    max-width: 200px;
  }

  .picker.mono {
    font-family: ui-monospace, monospace;
  }

  .picker:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-ring);
  }

  .picker:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .model-picker-group {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .refresh-btn {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 9px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .refresh-btn:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
    transform: rotate(-15deg);
  }

  .refresh-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinning {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .spacer { flex: 1; }

  .settings-trigger {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 9px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .settings-trigger:hover {
    color: var(--accent);
    border-color: var(--accent);
  }

  .settings-trigger.active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }

  /* 设置抽屉 */
  .settings-sheet {
    margin: 0 24px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: slideDown 0.2s ease;
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .ss-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ss-field label,
  .ss-field-inline label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .ss-field textarea {
    width: 100%;
    padding: 8px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    outline: none;
    font-family: inherit;
    resize: vertical;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }

  .ss-field textarea:focus {
    border-color: var(--accent);
  }

  .ss-row {
    display: flex;
    gap: 14px;
  }

  .ss-field-inline {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ss-field-inline input {
    padding: 7px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }

  .ss-field-inline input:focus {
    border-color: var(--accent);
  }

  /* 警告条 */
  .floating-warn {
    margin: 10px 24px 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 14px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 10px;
    color: var(--error-text);
    font-size: 13px;
  }

  .floating-warn button {
    margin-left: auto;
    background: transparent;
    border: none;
    color: var(--error-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 0 4px;
  }

  /* ===== 消息流 ===== */
  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 24px 24px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    scroll-behavior: smooth;
  }

  /* ===== 空状态 ===== */
  .welcome {
    margin: auto;
    text-align: center;
    padding: 40px 20px;
    max-width: 600px;
  }

  .welcome-orb {
    width: 80px;
    height: 80px;
    border-radius: 50%;
    margin: 0 auto 24px;
    background: var(--accent-gradient);
    position: relative;
    box-shadow: 0 12px 40px var(--accent-shadow);
    animation: orbFloat 4s ease-in-out infinite;
  }

  .welcome-orb::before {
    content: '';
    position: absolute;
    inset: -8px;
    border-radius: 50%;
    background: var(--accent-gradient);
    opacity: 0.25;
    filter: blur(16px);
    animation: orbPulse 4s ease-in-out infinite;
  }

  @keyframes orbFloat {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-8px); }
  }

  @keyframes orbPulse {
    0%, 100% { transform: scale(1); opacity: 0.25; }
    50% { transform: scale(1.15); opacity: 0.4; }
  }

  .welcome-greet {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 8px;
    letter-spacing: -0.02em;
  }

  .welcome-sub {
    font-size: 14px;
    color: var(--text-muted);
    margin-bottom: 32px;
  }

  .welcome-sub strong {
    color: var(--text-secondary);
    font-weight: 600;
  }

  .welcome-model {
    display: inline-block;
    margin-left: 6px;
    padding: 2px 8px;
    background: var(--accent-bg);
    color: var(--accent);
    border-radius: 6px;
    font-family: ui-monospace, monospace;
    font-size: 12px;
  }

  .welcome-examples {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    text-align: left;
  }

  .welcome-examples button {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
    font-family: inherit;
  }

  .welcome-examples button:hover {
    border-color: var(--accent);
    color: var(--accent);
    transform: translateY(-2px);
    box-shadow: 0 6px 16px var(--shadow-md);
  }

  .welcome-examples button span {
    font-size: 18px;
  }

  /* ===== 单条消息 ===== */
  .msg {
    max-width: var(--msg-max);
    width: 100%;
    margin: 0 auto;
    padding: 10px 0;
    animation: msgEnter 0.3s ease;
  }

  @keyframes msgEnter {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* 用户：右侧实心 */
  .msg.user {
    display: flex;
    justify-content: flex-end;
  }

  .msg.user .msg-content {
    background: var(--accent-gradient);
    color: white;
    border-radius: 18px 18px 4px 18px;
    padding: 11px 16px;
    max-width: 80%;
    box-shadow: 0 4px 14px var(--accent-shadow);
  }

  /* 助手：左侧无气泡 贴左 */
  .msg.assistant {
    display: flex;
    justify-content: flex-start;
  }

  .msg.assistant .msg-content {
    padding: 11px 16px 11px 14px;
    border-left: 3px solid var(--accent);
    border-radius: 0 12px 12px 0;
    max-width: 90%;
    background: var(--bg-card);
    color: var(--text-primary);
  }

  .msg.assistant .msg-content.err {
    border-left-color: var(--error-text);
    background: var(--error-bg);
    color: var(--error-text);
  }

  .msg-content {
    display: inline-block;
  }

  .msg-text {
    margin: 0;
    font-family: inherit;
    font-size: 14.5px;
    line-height: 1.7;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* 推理 */
  .reasoning {
    margin-bottom: 8px;
    padding: 8px 14px;
    background: var(--bg-subtle);
    border-radius: 10px;
    font-size: 12px;
    color: var(--text-muted);
    border: 1px dashed var(--border);
  }

  .reasoning summary {
    cursor: pointer;
    user-select: none;
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--text-secondary);
    list-style: none;
  }

  .reasoning summary::-webkit-details-marker { display: none; }

  .reasoning-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.4s ease infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(0.85); }
  }

  .reasoning-text {
    margin: 8px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, monospace;
  }

  /* 打字指示器 */
  .typing {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    padding: 4px 0;
  }

  .typing span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    animation: typingBounce 1.2s infinite;
  }

  .typing span:nth-child(2) { animation-delay: 0.15s; }
  .typing span:nth-child(3) { animation-delay: 0.3s; }

  @keyframes typingBounce {
    0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
    30% { transform: translateY(-4px); opacity: 1; }
  }

  /* ===== 输入停靠 ===== */
  .dock {
    padding: 12px 24px 20px;
    flex-shrink: 0;
  }

  .composer {
    max-width: var(--dock-max);
    margin: 0 auto;
    background: var(--bg-card);
    border: 1.5px solid var(--border);
    border-radius: 18px;
    padding: 12px 14px 8px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: all 0.2s;
    box-shadow: 0 4px 24px var(--shadow-md);
  }

  .composer:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 4px var(--accent-ring), 0 6px 28px var(--shadow-md);
  }

  .composer.sending {
    opacity: 0.92;
  }

  .composer textarea {
    width: 100%;
    border: none;
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 14.5px;
    font-family: inherit;
    line-height: 1.5;
    resize: none;
    outline: none;
    min-height: 24px;
    box-sizing: border-box;
    caret-color: var(--accent);
  }

  .composer textarea::placeholder {
    color: var(--text-placeholder);
    opacity: 1;
  }

  .composer-side {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .composer-hint {
    font-size: 11px;
    color: var(--text-muted);
  }

  .composer-hint kbd {
    display: inline-block;
    padding: 1px 5px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 10px;
    font-family: ui-monospace, monospace;
    color: var(--text-secondary);
  }

  .send-btn {
    width: 34px;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--accent);
    border: none;
    border-radius: 10px;
    color: white;
    cursor: pointer;
    transition: all 0.18s;
    flex-shrink: 0;
  }

  .send-btn:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: scale(1.06);
  }

  .send-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
    transform: none;
  }

  .stop-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 10px;
    color: var(--error-text);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
    flex-shrink: 0;
  }

  .stop-btn:hover {
    background: var(--error-hover-bg);
  }

  .stop-square {
    width: 10px;
    height: 10px;
    background: var(--error-text);
    border-radius: 2px;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  /* 响应式 */
  @media (max-width: 640px) {
    .source-bar {
      padding: 8px 16px;
    }

    .welcome-examples {
      grid-template-columns: 1fr;
    }

    .messages {
      padding: 16px 16px 4px;
    }

    .dock {
      padding: 8px 16px 16px;
    }

    .top-bar {
      padding: 10px 16px;
    }

    .composer-hint { display: none; }
  }
</style>
