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

  // ===== 整体布局 =====
  let sidebarOpen = $state(true);

  // ===== 来源数据 =====
  let runningModels = $state<RunningModelInfo[]>([]);
  let providers = $state<ProviderConfig[]>([]);
  let providerModels = $state<ModelInfo[]>([]);
  let modelsLoading = $state(false);
  let modelsError = $state('');

  // ===== 当前会话状态 =====
  let currentSessionId = $state<string | null>(null);
  let messages = $state<ChatMessage[]>([]);
  let input = $state('');
  let isSending = $state(false);
  let error = $state('');
  let messagesEl: HTMLElement | null = null;

  // 模式来源（当前编辑中，尚未属于 session 时）
  // type: 'provider' | 'model'
  let sourceType = $state<'provider' | 'model'>('provider');
  let selectedProviderId = $state<string | null>(null);
  let selectedModelPort = $state<number | null>(null);
  // Provider 模式下覆盖的模型名
  let overrideModelName = $state<string | null>(null);

  // ===== 参数设置 =====
  let showSettings = $state(false);
  let systemPrompt = $state('你是一个有用的助手，请简洁准确地回答用户问题。');
  let temperature = $state(0.7);
  let maxTokens = $state(4000);

  // ===== 历史记录 =====
  let sessions = $state<ChatSession[]>([]);

  // ===== 事件监听清理 =====
  let unlisteners: Array<() => void> = [];

  // ===== 生命周期 =====
  onMount(async () => {
    await Promise.all([loadSessions(), loadRunningModels(), loadProviders()]);

    // 默认选中第一个 provider
    if (providers.length > 0) {
      selectedProviderId = providers[0].id;
      // 自动拉取可用模型
      handleFetchModels();
    }

    // 如果有运行中的本地模型，默认切到 model 模式
    if (runningModels.length > 0) {
      sourceType = 'model';
      selectedModelPort = runningModels[0].port;
    }

    // 注册流式事件
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
      // 流结束后自动保存
      saveCurrentSession();
    });

    const unError = await listen<string>('chat-error', (e) => {
      isSending = false;
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant' && !last.content) {
        last.content = `❌ ${e.payload}`;
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

  // ===== 派生值 =====
  const selectedProvider = $derived(providers.find((p) => p.id === selectedProviderId) ?? null);
  const selectedModel = $derived(runningModels.find((m) => m.port === selectedModelPort) ?? null);

  // 当前生效的模型名（用于显示和发送）
  const currentModelName = $derived.by(() => {
    if (sourceType === 'model') {
      return selectedModel?.name ?? 'local';
    }
    // provider 模式
    return overrideModelName ?? selectedProvider?.active_model ?? null;
  });

  const canSend = $derived.by(() => {
    if (isSending || input.trim().length === 0) return false;
    if (sourceType === 'model') return !!selectedModelPort;
    // provider 模式必须有可用模型
    return !!selectedProviderId && !!currentModelName;
  });

  // ===== 数据加载 =====
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

  // ===== Provider 可用模型获取 =====
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

  // ===== 模式切换 =====
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

  // 切换 Provider 后自动拉模型
  function onSelectProviderChange() {
    overrideModelName = null;
    providerModels = [];
    if (selectedProviderId) handleFetchModels();
  }

  // ===== 会话操作 =====
  function newChat() {
    if (isSending) return;
    currentSessionId = null;
    messages = [];
    error = '';
    input = '';
    overrideModelName = null;
  }

  async function selectSession(s: ChatSession) {
    if (isSending) return;
    currentSessionId = s.id;
    error = '';
    input = '';

    // 加载消息
    messages = (s.messages as ChatMessage[]).map((m) => ({
      role: m.role as 'user' | 'assistant',
      content: m.content,
      reasoning: m.reasoning,
      timestamp: m.timestamp ?? 0,
      error: m.error,
    }));

    // 恢复 source 状态
    if (s.source.type === 'provider') {
      sourceType = 'provider';
      const sid = s.source.provider_id;
      selectedProviderId = sid;
      // 如果 source 里带了具体 model，用它；否则用 provider.active_model
      const provider = providers.find((p) => p.id === sid);
      overrideModelName = s.source.model ?? provider?.active_model ?? null;
      // 拉取模型列表（给切换模型用）
      if (provider) {
        handleFetchModels();
      }
    } else if (s.source.type === 'model') {
      sourceType = 'model';
      selectedModelPort = s.source.port;
    }

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

  // 自动生成标题
  function genTitle(): string {
    const firstUser = messages.find((m) => m.role === 'user');
    if (firstUser) {
      const t = firstUser.content.trim().slice(0, 24);
      return t + (firstUser.content.length > 24 ? '…' : '');
    }
    return '新对话';
  }

  // 构造当前 source
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

  // 持久化当前会话
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

      // 更新本地 sessions 列表
      const idx = sessions.findIndex((s) => s.id === saved.id);
      if (idx >= 0) {
        sessions[idx] = saved;
        // 排序
        sessions = [...sessions].sort((a, b) => b.updated_at - a.updated_at);
      } else {
        sessions = [saved, ...sessions];
      }
    } catch (e) {
      console.error('保存会话失败', e);
    }
  }

  // ===== 发送 =====
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
        error = `该 Provider 没有活动模型，请先点击"刷新模型列表"并选择一个模型`;
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

    // 构造请求上下文（含系统提示）
    const context: Array<{ role: string; content: string }> = [];
    if (systemPrompt.trim()) {
      context.push({ role: 'system', content: systemPrompt.trim() });
    }
    for (let i = 0; i < messages.length - 1; i++) {
      if (messages[i].error) continue;
      context.push({ role: messages[i].role, content: messages[i].content });
    }

    // ChatSource 中 model 字段用当前生效模型
    let source: ChatSource;
    if (sourceType === 'model') {
      source = buildSource();
    } else {
      source = {
        type: 'provider',
        provider_id: selectedProviderId!,
        model: currentModelName, // 显式带上当前模型
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

  // ===== 辅助 =====
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

  function sourceLabel(s: ChatSource): string {
    if (s.type === 'model') {
      return `🤖 端口 ${s.port}`;
    }
    const p = providers.find((x) => x.id === s.provider_id);
    return `🔌 ${p?.name ?? 'Provider'}`;
  }
</script>

<div class="chat-page" class:sidebar-collapsed={!sidebarOpen}>
  <!-- ====== 左侧：历史记录 ====== -->
  <aside class="sessions-bar">
    <div class="sb-header">
      <button class="btn-new" onclick={newChat} disabled={isSending} title="新建对话">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
        <span>新建对话</span>
      </button>
      <button class="btn-collapse" onclick={() => (sidebarOpen = !sidebarOpen)} title={sidebarOpen ? '收起' : '展开'}>
        {#if sidebarOpen}
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
        {:else}
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
        {/if}
      </button>
    </div>

    {#if sidebarOpen}
      <div class="sb-list">
        {#if sessions.length === 0}
          <div class="sb-empty">
            <span>💬</span>
            <p>暂无聊天记录</p>
          </div>
        {:else}
          {#each sessions as s (s.id)}
            <div
              class="sb-item"
              class:active={s.id === currentSessionId}
              onclick={() => selectSession(s)}
              role="button"
              tabindex="0"
            >
              <div class="sb-item-main">
                <div class="sb-item-title">{s.title}</div>
                <div class="sb-item-meta">
                  <span class="sb-source">{sourceLabel(s.source)}</span>
                  <span class="sb-time">{formatTime(s.updated_at)}</span>
                </div>
              </div>
              <button
                class="sb-del"
                onclick={(e) => deleteSession(s.id, e)}
                title="删除"
                aria-label="删除"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
  </aside>

  <!-- ====== 右侧：主对话区 ====== -->
  <main class="main-area">
    <!-- 工具栏 -->
    <div class="toolbar">
      <div class="tb-left">
        <!-- 模式切换 -->
        <div class="mode-tabs">
          <button class="mode-tab" class:active={sourceType === 'provider'} onclick={() => onSwitchType('provider')}>
            🔌 API 提供者
          </button>
          <button class="mode-tab" class:active={sourceType === 'model'} onclick={() => onSwitchType('model')}>
            🤖 本地模型
          </button>
        </div>

        {#if sourceType === 'provider'}
          <!-- Provider 选择 -->
          <select bind:value={selectedProviderId} onchange={onSelectProviderChange} disabled={isSending} class="sel-provider">
            {#if providers.length === 0}
              <option value={null}>尚未配置 Provider</option>
            {/if}
            {#each providers as p (p.id)}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>

          <!-- 模型切换（工具栏核心） -->
          <div class="model-switcher">
            <select
              value={currentModelName}
              onchange={(e) => (overrideModelName = (e.target as HTMLSelectElement).value)}
              disabled={isSending || modelsLoading}
              class="sel-model"
            >
              {#if !currentModelName}
                <option value="" disabled>未选择模型</option>
              {/if}
              {#if modelsLoading}
                <option value="">加载中...</option>
              {/if}
              {#if currentModelName && providerModels.findIndex((m) => m.id === currentModelName) < 0}
                <option value={currentModelName}>{currentModelName}</option>
              {/if}
              {#each providerModels as m (m.id)}
                <option value={m.id}>{m.id}</option>
              {/each}
            </select>

            <button class="btn-refresh-models" onclick={handleFetchModels} disabled={isSending || modelsLoading} title="刷新模型列表">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class:spinning={modelsLoading}><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
            </button>

            {#if providerModels.length > 0 && !modelsLoading}
              <span class="model-count">{providerModels.length} 个</span>
            {/if}
          </div>
        {:else}
          <!-- 本地模型选择 -->
          <select bind:value={selectedModelPort} disabled={isSending} class="sel-model">
            {#if runningModels.length === 0}
              <option value={null}>无运行中的模型</option>
            {/if}
            {#each runningModels as m (m.port)}
              <option value={m.port}>{m.name} · :{m.port}</option>
            {/each}
          </select>
          <button class="btn-refresh-models" onclick={loadRunningModels} disabled={isSending} title="刷新运行中的模型">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
          </button>
        {/if}
      </div>

      <div class="tb-right">
        <button class="btn-icon" class:active={showSettings} onclick={() => (showSettings = !showSettings)} title="参数设置">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
        </button>
        <button class="btn-icon" onclick={handleClear} title="清空当前对话" disabled={isSending || messages.length === 0}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
        </button>
      </div>
    </div>

    {#if modelsError}
      <div class="warn-banner">⚠️ 模型列表获取失败：{modelsError}</div>
    {/if}

    {#if error}
      <div class="warn-banner">⚠️ {error}<button class="wb-dismiss" onclick={() => (error = '')}>✕</button></div>
    {/if}

    <!-- 参数设置面板 -->
    {#if showSettings}
      <div class="settings-panel">
        <div class="sp-row">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label>系统提示</label>
          <textarea bind:value={systemPrompt} rows="2" disabled={isSending} placeholder="为对话设定角色和约束"></textarea>
        </div>
        <div class="sp-row-inline">
          <div class="sp-item">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>温度</label>
            <input type="number" step="0.1" min="0" max="2" bind:value={temperature} disabled={isSending} />
          </div>
          <div class="sp-item">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>最大 Token</label>
            <input type="number" step="100" min="1" bind:value={maxTokens} disabled={isSending} />
          </div>
        </div>
      </div>
    {/if}

    <!-- 消息列表 -->
    <div class="messages" bind:this={messagesEl}>
      {#if messages.length === 0}
        <div class="empty">
          <div class="empty-icon">💭</div>
          <p>开始与模型对话吧</p>
          <p class="empty-hint">
            {#if sourceType === 'provider' && selectedProvider}
              使用 {selectedProvider.name} · {currentModelName ?? '未选择模型'}
            {:else if sourceType === 'model'}
              使用本地模型 {selectedModel?.name ?? ''}
            {:else}
              请先选择一个来源
            {/if}
          </p>
        </div>
      {/if}

      {#each messages as msg, i (i)}
        <div class="msg-row" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'}>
          <div class="msg-avatar">{msg.role === 'user' ? '🧑' : '🤖'}</div>
          <div class="msg-body">
            {#if msg.reasoning}
              <details class="reasoning">
                <summary>💭 推理过程</summary>
                <pre>{msg.reasoning}</pre>
              </details>
            {/if}
            <div class="msg-content" class:error-msg={msg.error}>
              {#if !msg.content && isSending && i === messages.length - 1}
                <span class="generating">
                  <span class="dot-typing"><span>.</span><span>.</span><span>.</span></span>
                </span>
              {:else}
                <pre class="content-pre">{msg.content}</pre>
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </div>

    <!-- 输入区 -->
    <div class="composer">
      <textarea
        bind:value={input}
        onkeydown={handleKeydown}
        disabled={isSending}
        rows="2"
        placeholder={isSending ? '正在生成回复…' : '输入消息，Enter 发送，Shift+Enter 换行'}
      ></textarea>
      <div class="composer-actions">
        {#if isSending}
          <button class="btn-stop" onclick={handleStop}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
            停止生成
          </button>
        {:else}
          <button class="btn-send" onclick={handleSend} disabled={!canSend}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
            发送
          </button>
        {/if}
      </div>
    </div>
  </main>
</div>

<style>
  .chat-page {
    height: 100%;
    display: flex;
    gap: 0;
    margin: -32px;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  /* ===== 左侧历史 ===== */
  .sessions-bar {
    width: 240px;
    flex-shrink: 0;
    background: var(--bg-card);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    transition: width 0.25s ease;
    overflow: hidden;
  }

  .chat-page.sidebar-collapsed .sessions-bar {
    width: 48px;
  }

  .sb-header {
    padding: 12px;
    border-bottom: 1px solid var(--border-light);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .btn-new {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 12px;
    background: var(--accent);
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
    overflow: hidden;
  }

  .btn-new:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: translateY(-1px);
    box-shadow: 0 4px 10px var(--accent-shadow);
  }

  .btn-new:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .sidebar-collapsed .btn-new span {
    display: none;
  }

  .btn-collapse {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-secondary);
    cursor: pointer;
    flex-shrink: 0;
    transition: all 0.2s;
  }

  .btn-collapse:hover {
    color: var(--accent);
    border-color: var(--accent);
  }

  .sb-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .sb-empty {
    text-align: center;
    padding: 40px 12px;
    color: var(--text-muted);
    font-size: 13px;
  }

  .sb-empty span {
    font-size: 28px;
    display: block;
    margin-bottom: 8px;
  }

  .sb-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.15s;
    border: 1px solid transparent;
  }

  .sb-item:hover {
    background: var(--bg-subtle);
  }

  .sb-item.active {
    background: var(--accent-bg);
    border-color: var(--accent);
  }

  .sb-item-main {
    flex: 1;
    min-width: 0;
  }

  .sb-item-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-bottom: 4px;
  }

  .sb-item-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }

  .sb-source {
    background: var(--bg-subtle);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sb-item.active .sb-source {
    background: var(--bg-card);
  }

  .sb-del {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: 4px;
    opacity: 0;
    transition: all 0.15s;
    flex-shrink: 0;
  }

  .sb-item:hover .sb-del {
    opacity: 1;
  }

  .sb-del:hover {
    background: var(--error-bg);
    color: var(--error-text);
  }

  /* ===== 右侧主区 ===== */
  .main-area {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 16px 32px 12px;
  }

  /* ===== 工具栏 ===== */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-light);
    flex-wrap: wrap;
  }

  .tb-left {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .tb-right {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .mode-tabs {
    display: flex;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 3px;
    gap: 2px;
  }

  .mode-tab {
    padding: 6px 12px;
    background: none;
    border: none;
    border-radius: 7px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
  }

  .mode-tab:hover {
    color: var(--text-primary);
  }

  .mode-tab.active {
    background: var(--accent);
    color: white;
  }

  .sel-provider,
  .sel-model {
    padding: 6px 10px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    outline: none;
    transition: border-color 0.2s;
    font-family: ui-monospace, monospace;
    max-width: 220px;
  }

  .sel-provider:focus,
  .sel-model:focus {
    border-color: var(--accent);
  }

  .model-switcher {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .btn-refresh-models {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    flex-shrink: 0;
  }

  .btn-refresh-models:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
  }

  .btn-refresh-models:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinning {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .model-count {
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-subtle);
    padding: 2px 8px;
    border-radius: 6px;
  }

  .btn-icon {
    width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    flex-shrink: 0;
  }

  .btn-icon:hover:not(:disabled) {
    background: var(--bg-subtle);
    color: var(--accent);
    border-color: var(--accent);
  }

  .btn-icon.active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }

  .btn-icon:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ===== 警告条 ===== */
  .warn-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 14px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 8px;
    color: var(--error-text);
    margin-top: 12px;
    font-size: 13px;
  }

  .wb-dismiss {
    background: none;
    border: none;
    color: var(--error-muted);
    cursor: pointer;
    font-size: 14px;
  }

  /* ===== 设置面板 ===== */
  .settings-panel {
    background: var(--bg-card);
    border: 1px solid var(--border-light);
    border-radius: 12px;
    padding: 14px;
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .sp-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sp-row label,
  .sp-item label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .sp-row textarea {
    width: 100%;
    padding: 8px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    font-family: inherit;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.2s;
    resize: vertical;
  }

  .sp-row textarea:focus,
  .sp-item input:focus {
    border-color: var(--accent);
  }

  .sp-row-inline {
    display: flex;
    gap: 16px;
  }

  .sp-item {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
  }

  /* ===== 消息区 ===== */
  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 16px 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-height: 0;
  }

  .empty {
    text-align: center;
    padding: 60px 20px;
    color: var(--text-muted);
    margin: auto;
  }

  .empty-icon {
    font-size: 56px;
    margin-bottom: 12px;
  }

  .empty p {
    font-size: 15px;
    margin-bottom: 4px;
  }

  .empty-hint {
    font-size: 13px;
    color: var(--text-muted);
  }

  /* 消息行 */
  .msg-row {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }

  .msg-row.user {
    flex-direction: row-reverse;
  }

  .msg-avatar {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border-radius: 50%;
    font-size: 16px;
  }

  .msg-row.assistant .msg-avatar {
    background: var(--accent-bg);
  }

  .msg-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .msg-row.user .msg-body {
    align-items: flex-end;
  }

  .msg-content {
    display: inline-block;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border-light);
    border-radius: 12px;
    max-width: 100%;
    box-shadow: 0 1px 2px var(--shadow-sm);
    color: var(--text-primary);
  }

  .msg-row.user .msg-content {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .msg-content.error-msg {
    background: var(--error-bg);
    border-color: var(--error-border);
    color: var(--error-text);
  }

  .content-pre {
    margin: 0;
    font-family: inherit;
    font-size: 14px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    color: inherit;
  }

  /* 推理过程 */
  .reasoning {
    background: var(--bg-subtle);
    border-radius: 8px;
    padding: 6px 12px;
    font-size: 12px;
    color: var(--text-primary);
    max-width: 100%;
  }

  .reasoning summary {
    cursor: pointer;
    user-select: none;
  }

  .reasoning pre {
    margin: 8px 0 4px;
    padding: 6px 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, monospace;
  }

  .generating {
    color: var(--text-secondary);
    font-size: 14px;
  }

  .dot-typing span {
    animation: blink 1.4s infinite both;
    display: inline-block;
    font-weight: bold;
  }

  .dot-typing span:nth-child(2) { animation-delay: 0.2s; }
  .dot-typing span:nth-child(3) { animation-delay: 0.4s; }

  @keyframes blink {
    0%, 80%, 100% { opacity: 0.2; }
    40% { opacity: 1; }
  }

  /* ===== 输入区 ===== */
  .composer {
    border-top: 1px solid var(--border-light);
    padding: 12px 0 4px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex-shrink: 0;
  }

  .composer textarea {
    width: 100%;
    padding: 12px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    font-size: 14px;
    font-family: inherit;
    color: var(--text-primary);
    resize: none;
    outline: none;
    transition: border-color 0.2s;
    min-height: 44px;
  }

  .composer textarea:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-ring);
  }

  .composer textarea::placeholder {
    color: var(--text-muted);
  }

  .composer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .btn-send {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 20px;
    background: var(--accent);
    border: none;
    border-radius: 8px;
    color: white;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-send:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: translateY(-1px);
    box-shadow: 0 4px 12px var(--accent-shadow);
  }

  .btn-send:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-stop {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 8px;
    color: var(--error-text);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-stop:hover {
    background: var(--error-hover-bg);
  }

  /* ===== 响应式 ===== */
  @media (max-width: 768px) {
    .chat-page {
      flex-direction: column;
    }

    .sessions-bar {
      width: 100%;
      max-height: 200px;
      border-right: none;
      border-bottom: 1px solid var(--border);
    }

    .chat-page.sidebar-collapsed .sessions-bar {
      width: 100%;
      max-height: 48px;
    }

    .main-area {
      padding: 12px 16px;
    }

    .toolbar {
      flex-direction: column;
      align-items: stretch;
    }

    .tb-left {
      flex-direction: column;
      align-items: stretch;
    }

    .sel-provider,
    .sel-model {
      max-width: none;
    }

    .sp-row-inline {
      flex-direction: column;
      gap: 8px;
    }
  }
</style>
