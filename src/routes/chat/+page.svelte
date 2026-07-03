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
  interface ChatMessage {
    role: 'user' | 'assistant';
    content: string;
    reasoning?: string;      // 推理内容（可选）
    timestamp: number;
    error?: boolean;
  }

  // ===== 状态 =====
  let mode = $state<'model' | 'provider'>('provider');
  let runningModels = $state<RunningModelInfo[]>([]);
  let providers = $state<ProviderConfig[]>([]);
  let selectedModelPort = $state<number | null>(null);
  let selectedProviderId = $state<string | null>(null);

  // 默认系统提示（可编辑）
  let systemPrompt = $state('你是一个有用的助手，请简洁准确地回答用户问题。');
  let showSettings = $state(false);
  let temperature = $state(0.7);
  let maxTokens = $state(4000);

  // 对话状态
  let messages = $state<ChatMessage[]>([]);
  let input = $state('');
  let isSending = $state(false);
  let error = $state('');
  let messagesEl: HTMLElement | null = null;

  // 事件监听清理
  let unlisteners: Array<() => void> = [];

  // ===== 生命周期 =====
  onMount(async () => {
    await Promise.all([loadRunningModels(), loadProviders()]);

    // 设置默认选择
    if (providers.length > 0) {
      selectedProviderId = providers[0].id;
    }

    // 切换到模型模式如果有运行中的模型
    if (runningModels.length > 0) {
      mode = 'model';
      selectedModelPort = runningModels[0].port;
    }

    // 注册事件监听
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
      }
    });

    const unDone = await listen('chat-done', () => {
      isSending = false;
    });

    const unError = await listen<string>('chat-error', (e) => {
      isSending = false;
      // 在最后一条 assistant 消息上标记错误，或在 banner 显示
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

  // ===== 加载来源数据 =====
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

  // ===== 派生 =====
  const selectedProvider = $derived(
    providers.find((p) => p.id === selectedProviderId) ?? null
  );
  const selectedModel = $derived(
    runningModels.find((m) => m.port === selectedModelPort) ?? null
  );
  const canSend = $derived(
    !isSending &&
      input.trim().length > 0 &&
      (mode === 'model' ? !!selectedModelPort : !!selectProviderUsable())
  );

  // provider 模式下是否能用（必须有 active_model）
  function selectProviderUsable(): boolean {
    return !!selectedProvider && !!selectedProvider.active_model;
  }

  // ===== 动作 =====
  async function scrollToBottom() {
    await tick();
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  async function handleSend() {
    const text = input.trim();
    if (!text || isSending) return;

    // 校验
    if (mode === 'model' && !selectedModelPort) {
      error = '请先选择一个运行中的本地模型（在「模型管理」页面启动）';
      return;
    }
    if (mode === 'provider') {
      if (!selectedProvider) {
        error = '请先选择一个 API 提供者';
        return;
      }
      if (!selectedProvider.active_model) {
        error = `Provider "${selectedProvider.name}" 没有设置活动模型，请到 API 接口详情页选择`;
        return;
      }
    }

    input = '';
    error = '';

    const userMsg: ChatMessage = {
      role: 'user',
      content: text,
      timestamp: Date.now(),
    };
    const placeholder: ChatMessage = {
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    };
    messages = [...messages, userMsg, placeholder];
    await scrollToBottom();

    isSending = true;

    // 构造发送的完整上下文
    const context: Array<{ role: string; content: string }> = [];
    if (systemPrompt.trim()) {
      context.push({ role: 'system', content: systemPrompt.trim() });
    }
    // 取 history + user，排除 placeholder
    for (let i = 0; i < messages.length - 1; i++) {
      context.push({ role: messages[i].role, content: messages[i].content });
    }

    // 构造 source
    const source =
      mode === 'model'
        ? {
            type: 'model',
            port: selectedModelPort!,
            model_name: selectedModel?.name ?? 'local',
          }
        : {
            type: 'provider',
            provider_id: selectedProviderId!,
            model: null,
          };

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
      // send_chat 失败前已经 emit 了 chat-error，但健壮起见也兜底
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
  }

  function handleKeydown(e: KeyboardEvent) {
    // Enter 发送（Shift+Enter 换行）
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  function onModeSwitch(newMode: 'model' | 'provider') {
    if (newMode === mode) return;
    if (isSending) return;
    mode = newMode;
    if (newMode === 'model' && runningModels.length > 0 && !selectedModelPort) {
      selectedModelPort = runningModels[0].port;
    }
    if (newMode === 'provider' && providers.length > 0 && !selectedProviderId) {
      selectedProviderId = providers[0].id;
    }
    error = '';
  }
</script>

<div class="chat-page">
  <!-- 顶部工具栏 -->
  <div class="toolbar">
    <div class="toolbar-left">
      <!-- 模式切换 -->
      <div class="mode-tabs">
        <button
          class="mode-tab"
          class:active={mode === 'provider'}
          onclick={() => onModeSwitch('provider')}
        >
          🔌 API 提供者
        </button>
        <button
          class="mode-tab"
          class:active={mode === 'model'}
          onclick={() => onModeSwitch('model')}
        >
          🤖 本地模型
        </button>
      </div>

      <!-- 资源选择 -->
      <div class="source-select">
        {#if mode === 'provider'}
          <select bind:value={selectedProviderId} disabled={isSending}>
            {#if providers.length === 0}
              <option value={null}>尚未配置 Provider</option>
            {/if}
            {#each providers as p (p.id)}
              <option value={p.id}>{p.name}{p.active_model ? ` · ${p.active_model}` : ''}</option>
            {/each}
          </select>
          {#if selectedProvider && !selectedProvider.active_model}
            <span class="warn-inline">未设置模型</span>
          {/if}
        {:else}
          <select bind:value={selectedModelPort} disabled={isSending}>
            {#if runningModels.length === 0}
              <option value={null}>没有运行中的模型</option>
            {/if}
            {#each runningModels as m (m.port)}
              <option value={m.port}>{m.name} · 端口 {m.port}</option>
            {/each}
          </select>
        {/if}
      </div>
    </div>

    <div class="toolbar-right">
      <button class="btn-icon" class:active={showSettings} onclick={() => (showSettings = !showSettings)} title="参数设置">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
      </button>
      <button class="btn-icon" onclick={() => loadRunningModels()} title="刷新模型列表" disabled={isSending}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
      </button>
      <button class="btn-icon" onclick={handleClear} title="清空对话" disabled={isSending || messages.length === 0}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
      </button>
    </div>
  </div>

  <!-- 设置面板 -->
  {#if showSettings}
    <div class="settings-panel">
      <div class="setting-row">
        <label>系统提示</label>
        <textarea
          bind:value={systemPrompt}
          rows="2"
          disabled={isSending}
          placeholder="为对话设定角色和约束"
        ></textarea>
      </div>
      <div class="setting-row-inline">
        <div class="setting-item">
          <label>温度 (temperature)</label>
          <input type="number" step="0.1" min="0" max="2" bind:value={temperature} disabled={isSending} />
        </div>
        <div class="setting-item">
          <label>最大 Token</label>
          <input type="number" step="100" min="1" bind:value={maxTokens} disabled={isSending} />
        </div>
      </div>
    </div>
  {/if}

  {#if error}
    <div class="error-banner">
      <span>⚠️ {error}</span>
      <button class="error-dismiss" onclick={() => (error = '')}>✕</button>
    </div>
  {/if}

  <!-- 消息区 -->
  <div class="messages" bind:this={messagesEl}>
    {#if messages.length === 0}
      <div class="empty">
        <div class="empty-icon">💭</div>
        <p>开始与模型对话吧</p>
        <p class="empty-hint">
          {#if mode === 'provider'}
            使用 {selectedProvider?.name ?? 'API 提供者'} · {selectedProvider?.active_model ?? '未选择模型'}
          {:else}
            使用本地模型 {selectedModel?.name ?? ''}
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
            {#if msg.reasoning}
              {#if msg.content}
                <pre class="content-pre">{msg.content}</pre>
              {:else if isSending && i === messages.length - 1}
                <span class="generating">思考中…</span>
              {/if}
            {:else if !msg.content && isSending && i === messages.length - 1}
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
      placeholder={isSending ? '正在生成回复...' : '输入消息，Enter 发送，Shift+Enter 换行'}
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
</div>

<style>
  .chat-page {
    max-width: 900px;
    margin: 0 auto;
    height: 100%;
    display: flex;
    flex-direction: column;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* 顶部工具栏 */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 0;
    flex-wrap: wrap;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
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
    padding: 6px 14px;
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

  .source-select select {
    padding: 7px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    font-family: ui-monospace, monospace;
    cursor: pointer;
    max-width: 280px;
    outline: none;
    transition: border-color 0.2s;
  }

  .source-select select:focus {
    border-color: var(--accent);
  }

  .warn-inline {
    margin-left: 6px;
    font-size: 12px;
    color: var(--error-text);
  }

  /* 按钮图标 */
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

  /* 设置面板 */
  .settings-panel {
    background: var(--bg-card);
    border: 1px solid var(--border-light);
    border-radius: 12px;
    padding: 16px;
    margin-bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: fadeIn 0.2s ease;
  }

  .setting-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .setting-row label,
  .setting-item label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .setting-row textarea,
  .setting-row input {
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

  .setting-row textarea:focus,
  .setting-row input:focus,
  .setting-item input:focus {
    border-color: var(--accent);
  }

  .setting-row-inline {
    display: flex;
    gap: 16px;
  }

  .setting-item {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
  }

  .setting-item input {
    padding: 7px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    font-family: inherit;
    outline: none;
  }

  /* 错误提示 */
  .error-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 16px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 10px;
    color: var(--error-text);
    margin-bottom: 12px;
    font-size: 13px;
  }

  .error-dismiss {
    background: none;
    border: none;
    color: var(--error-muted);
    cursor: pointer;
    font-size: 14px;
  }

  /* 消息区 */
  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 12px 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .empty {
    text-align: center;
    padding: 60px 20px;
    color: var(--text-muted);
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

  /* 单条消息 */
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
  }

  .dot-typing span:nth-child(2) { animation-delay: 0.2s; }
  .dot-typing span:nth-child(3) { animation-delay: 0.4s; }

  @keyframes blink {
    0%, 80%, 100% { opacity: 0.2; }
    40% { opacity: 1; }
  }

  /* 输入区 */
  .composer {
    border-top: 1px solid var(--border-light);
    padding: 12px 0 4px;
    display: flex;
    flex-direction: column;
    gap: 10px;
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

  @media (max-width: 600px) {
    .toolbar {
      flex-direction: column;
      align-items: stretch;
    }

    .toolbar-left {
      flex-direction: column;
      align-items: stretch;
    }

    .source-select select {
      max-width: none;
    }

    .setting-row-inline {
      flex-direction: column;
      gap: 8px;
    }
  }
</style>
