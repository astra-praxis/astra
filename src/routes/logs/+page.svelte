<script lang="ts">
  import { goto } from '$app/navigation';
  import { logs, type LogEntry } from '$lib/logs-store.svelte';

  function formatTime(ts: number): string {
    const d = new Date(ts);
    return `${d.getHours().toString().padStart(2,'0')}:${d.getMinutes().toString().padStart(2,'0')}:${d.getSeconds().toString().padStart(2,'0')}`;
  }

  function formatDate(ts: number): string {
    const d = new Date(ts);
    return `${d.getMonth()+1}/${d.getDate()}`;
  }

  function levelColor(level: LogEntry['level']): string {
    switch (level) {
      case 'error': return 'var(--error-text)';
      case 'warn': return 'var(--warning-text, #f59e0b)';
      default: return 'var(--text-secondary)';
    }
  }

  function levelLabel(level: LogEntry['level']): string {
    switch (level) {
      case 'error': return 'ERR';
      case 'warn': return 'WRN';
      default: return 'INF';
    }
  }

  function goBack() {
    history.back();
  }

  function handleClear() {
    logs.clear();
  }

  let filterLevel = $state<'all' | 'error' | 'warn' | 'info'>('all');
  let filterSource = $state('');

  let filteredEntries = $derived.by(() => {
    let result = logs.entries;
    if (filterLevel !== 'all') {
      result = result.filter(e => e.level === filterLevel);
    }
    if (filterSource.trim()) {
      const q = filterSource.trim().toLowerCase();
      result = result.filter(e => e.source.toLowerCase().includes(q) || e.message.toLowerCase().includes(q));
    }
    return result;
  });

  // 获取所有 source 用于筛选
  let allSources = $derived([...new Set(logs.entries.map(e => e.source))]);
</script>

<div class="logs-page">
  <div class="logs-header">
    <button class="back-btn" onclick={goBack}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
      返回
    </button>
    <h2>📋 日志</h2>
    <div class="logs-actions">
      <span class="logs-count">{filteredEntries.length} 条记录</span>
      <button class="clear-btn" onclick={handleClear} disabled={logs.entries.length === 0}>清空</button>
    </div>
  </div>

  <div class="logs-filters">
    <select bind:value={filterLevel} class="filter-select">
      <option value="all">全部级别</option>
      <option value="error">错误</option>
      <option value="warn">警告</option>
      <option value="info">信息</option>
    </select>
    <input 
      type="text" 
      class="filter-input" 
      placeholder="搜索来源/内容..." 
      bind:value={filterSource}
    />
  </div>

  {#if filteredEntries.length === 0}
    <div class="logs-empty">
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      <p>暂无日志记录</p>
    </div>
  {:else}
    <div class="logs-list">
      {#each filteredEntries as entry (entry.timestamp)}
        <div class="log-entry" class:log-error={entry.level === 'error'} class:log-warn={entry.level === 'warn'}>
          <div class="log-meta">
            <span class="log-level" style="color: {levelColor(entry.level)}">[{levelLabel(entry.level)}]</span>
            <span class="log-source">{entry.source}</span>
            <span class="log-time">{formatDate(entry.timestamp)} {formatTime(entry.timestamp)}</span>
          </div>
          <div class="log-message">{entry.message}</div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .logs-page {
    padding: 16px 20px;
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .logs-header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border);
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 10px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 7px;
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all .2s;
  }

  .back-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .logs-header h2 {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .logs-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .logs-count {
    font-size: 11px;
    color: var(--text-muted);
  }

  .clear-btn {
    padding: 5px 10px;
    background: var(--error-bg);
    color: var(--error-text);
    border: none;
    border-radius: 5px;
    font-size: 11px;
    cursor: pointer;
  }

  .clear-btn:hover:not(:disabled) {
    background: var(--error-hover-bg);
  }

  .clear-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .logs-filters {
    display: flex;
    gap: 10px;
    margin-bottom: 12px;
    flex-shrink: 0;
  }

  .filter-select {
    padding: 6px 10px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    color: var(--text-primary);
    outline: none;
  }

  .filter-input {
    flex: 1;
    padding: 6px 10px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    color: var(--text-primary);
    outline: none;
  }

  .filter-input::placeholder {
    color: var(--text-muted);
  }

  .logs-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }

  .logs-empty p {
    margin-top: 12px;
    font-size: 13px;
  }

  .logs-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--bg-subtle);
    border-radius: 8px;
    overflow: hidden;
  }

  .log-entry {
    padding: 8px 12px;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border-light);
  }

  .log-entry:last-child {
    border-bottom: none;
  }

  .log-entry.log-error {
    background: var(--error-bg);
  }

  .log-entry.log-warn {
    background: color-mix(in srgb, var(--warning-bg, #fef3c7) 50%, var(--bg-card));
  }

  .log-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }

  .log-level {
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    font-size: 10px;
    font-weight: 700;
  }

  .log-source {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .log-time {
    font-size: 10px;
    color: var(--text-muted);
    margin-left: auto;
    font-family: 'SF Mono', 'Cascadia Code', monospace;
  }

  .log-message {
    font-size: 12px;
    color: var(--text-primary);
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    word-break: break-all;
    white-space: pre-wrap;
    line-height: 1.5;
  }
</style>