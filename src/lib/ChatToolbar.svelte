<script lang="ts">
  import { toolbarState, onSwitchType, onSelectProviderChange, handleFetchModels, refreshRunningModels } from './chat-state.svelte';
</script>

<div class="toolbar-row">
  <div class="tb-left">
    <div class="seg-control">
      <button
        class:active={toolbarState.sourceType === "provider"}
        onclick={() => onSwitchType("provider")}
        title="API 提供者"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-4l-3 9L9 3l-3 9H2" /></svg>
      </button>
      <button
        class:active={toolbarState.sourceType === "model"}
        onclick={() => onSwitchType("model")}
        title="本地模型"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2" /><line x1="8" y1="21" x2="16" y2="21" /><line x1="12" y1="17" x2="12" y2="21" /></svg>
      </button>
    </div>

    {#if toolbarState.sourceType === "provider"}
      <select
        class="picker"
        bind:value={toolbarState.selectedProviderId}
        onchange={onSelectProviderChange}
        disabled={toolbarState.isSending}
        title="选择 Provider"
      >
        {#if toolbarState.providers.length === 0}
          <option value={null}>尚未配置</option>
        {/if}
        {#each toolbarState.providers as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>

      <div class="picker-group">
        <select class="picker mono" value={toolbarState.overrideModelName} onchange={(e) => (toolbarState.overrideModelName = (e.target as HTMLSelectElement).value || null)} disabled={toolbarState.isSending || toolbarState.modelsLoading} title="选择模型">
          {#if !toolbarState.overrideModelName}
            <option value="">未选模型</option>
          {/if}
          {#if toolbarState.modelsLoading}
            <option value="">加载中…</option>
          {/if}
          {#each toolbarState.providerModels as m (m.id)}
            <option value={m.id}>{m.id}</option>
          {/each}
        </select>
        <button class="icon-btn sm" onclick={handleFetchModels} disabled={toolbarState.isSending || toolbarState.modelsLoading} title="刷新模型列表" aria-label="刷新模型">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class:spinning={toolbarState.modelsLoading}><polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" /></svg>
        </button>
      </div>
    {:else}
      <select class="picker mono" bind:value={toolbarState.selectedModelPort} disabled={toolbarState.isSending} title="选择本地模型">
        {#if toolbarState.runningModels.length === 0}
          <option value={null}>无运行模型</option>
        {/if}
        {#each toolbarState.runningModels as m (m.port)}
          <option value={m.port}>{m.name} :{m.port}</option>
        {/each}
      </select>
      <button class="icon-btn sm" onclick={refreshRunningModels} disabled={toolbarState.isSending} title="刷新" aria-label="刷新">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" /></svg>
      </button>
    {/if}
  </div>

  <div class="tb-right">
    <button class="icon-btn" class:active={toolbarState.showSettings} onclick={() => (toolbarState.showSettings = !toolbarState.showSettings)} title="参数设置" aria-label="参数设置">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
    </button>
    <button class="icon-btn" onclick={toolbarState.onClear} disabled={toolbarState.isSending} title="清空对话" aria-label="清空对话">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg>
    </button>
    <button class="icon-btn" onclick={toolbarState.onToggleSidebar} title="历史记录" aria-label="历史记录">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="6" x2="21" y2="6" /><line x1="3" y1="12" x2="21" y2="12" /><line x1="3" y1="18" x2="21" y2="18" /></svg>
      {#if toolbarState.sessionsCount > 0}
        <span class="tb-badge">{toolbarState.sessionsCount}</span>
      {/if}
    </button>
  </div>
</div>

<style>
  .toolbar-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 100%;
    padding: 0 4px;
  }
  .tb-left {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }
  .tb-right {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  :global(.icon-btn) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--sidebar-text);
    cursor: pointer;
    transition: all 0.15s;
    flex-shrink: 0;
  }
  :global(.icon-btn:hover:not(:disabled)) {
    background: var(--sidebar-hover-bg);
    color: var(--sidebar-text-hover);
  }
  :global(.icon-btn.active) {
    color: var(--accent);
  }
  :global(.icon-btn:disabled) {
    opacity: 0.4;
    cursor: not-allowed;
  }
  :global(.icon-btn.sm) {
    width: 26px;
    height: 26px;
  }
  :global(.tb-badge) {
    position: absolute;
    top: 2px;
    right: 2px;
    min-width: 14px;
    height: 14px;
    padding: 0 4px;
    background: var(--accent);
    color: white;
    font-size: 10px;
    font-weight: 700;
    border-radius: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
  }
  .seg-control {
    display: flex;
    background: var(--bg-input);
    border-radius: 6px;
    overflow: hidden;
    flex-shrink: 0;
  }
  .seg-control button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 28px;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.15s;
  }
  .seg-control button:hover { color: var(--text-primary); }
  .seg-control button.active {
    background: var(--bg-card);
    color: var(--accent);
    box-shadow: 0 1px 3px rgba(0,0,0,0.1);
  }
  .picker {
    height: 28px;
    padding: 0 24px 0 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    color: var(--text-primary);
    outline: none;
    cursor: pointer;
    max-width: 200px;
    min-width: 80px;
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg width='10' height='6' viewBox='0 0 10 6' fill='none' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M1 1l4 4 4-4' stroke='%23888' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 6px center;
  }
  .picker:hover { border-color: var(--accent); }
  .picker.mono { font-family: monospace; }
  .picker:disabled { opacity: 0.5; cursor: not-allowed; }
  .picker-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .spinning { animation: spin 0.7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
