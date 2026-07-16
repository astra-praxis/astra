<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  // ===== 类型 =====
  interface WorkspaceConfig {
    name: string;
    path: string;
  }

  interface AppSettings {
    editor: { name: string; command: string };
    workspaces: WorkspaceConfig[];
    active_workspace: string | null;
    scan_depth: number;
  }

  interface NodeModulesInfo {
    path: string;
    size_bytes: number;
    size_display: string;
    project_name: string;
    has_pnpm_lock: boolean;
  }

  interface CleanLogItem {
    project_name: string;
    path: string;
    success: boolean;
    size: string;
    message: string;
  }

  // ===== 状态 =====
  let activeWorkspace = $state<string | null>(null);
  let isLoading = $state(true);

  // 扫描状态
  let nmScanning = $state(false);
  let nmList = $state<NodeModulesInfo[]>([]);
  let nmScanned = $state(false);

  // 清理状态
  let nmCleaning = $state(false);
  let nmCleanLogs = $state<CleanLogItem[]>([]);
  let nmCleanIndex = $state(0);
  let nmCleanStopped = $state(false);
  let nmDone = $state(false);

  // 计算
  let nmTotalSize = $derived(nmList.reduce((s, r) => s + r.size_bytes, 0));
  let nmProjectCount = $derived(new Set(nmList.map(r => r.project_name)).size);
  let nmCleanCount = $derived(nmCleanLogs.filter(l => l.success && !l.message.includes('⏭')).length);
  let nmSkippedCount = $derived(nmCleanLogs.filter(l => l.message.includes('⏭')).length);
  let nmFailedCount = $derived(nmCleanLogs.filter(l => !l.success).length);
  let nmCleanedSize = $derived(
    nmCleanLogs.filter(l => l.success).reduce((s, l) => {
      const item = nmList.find(n => n.path === l.path);
      return s + (item?.size_bytes || 0);
    }, 0)
  );

  function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + units[i];
  }

  // ===== 初始化：加载工作空间 =====
  onMount(async () => {
    try {
      const settings = await invoke<AppSettings>('get_settings');
      activeWorkspace = settings.active_workspace;
    } catch (e) {
      console.error('加载设置失败:', e);
    } finally {
      isLoading = false;
    }
  });

  // ===== 扫描 =====
  async function scanNodeModules() {
    if (!activeWorkspace || nmScanning) return;
    nmScanning = true;
    nmList = [];
    nmScanned = false;
    nmCleanLogs = [];
    nmCleanIndex = 0;
    nmCleanStopped = false;
    nmDone = false;
    try {
      const result = await invoke<NodeModulesInfo[]>('scan_node_modules', { workspacePath: activeWorkspace, maxDepth: 5 });
      nmList = result;
      nmScanned = true;
    } catch (e) {
      console.error('扫描失败:', e);
    } finally {
      nmScanning = false;
    }
  }

  // ===== 逐项清理 =====
  async function cleanAll() {
    if (nmList.length === 0 || nmCleaning) return;
    nmCleaning = true;
    nmCleanLogs = [];
    nmCleanIndex = 0;
    nmCleanStopped = false;
    nmDone = false;

    for (let i = 0; i < nmList.length; i++) {
      if (nmCleanStopped) break;
      nmCleanIndex = i + 1;
      const item = nmList[i];

      if (item.has_pnpm_lock) {
        nmCleanLogs = [...nmCleanLogs, {
          project_name: item.project_name,
          path: item.path,
          success: true,
          size: item.size_display,
          message: `⏭ ${item.project_name} — 已跳过 (pnpm 项目)`
        }];
      } else {
        try {
          await invoke('clean_node_modules', { paths: [item.path] });
          nmCleanLogs = [...nmCleanLogs, {
            project_name: item.project_name,
            path: item.path,
            success: true,
            size: item.size_display,
            message: `✓ ${item.project_name} — ${item.size_display}`
          }];
        } catch (e) {
          nmCleanLogs = [...nmCleanLogs, {
            project_name: item.project_name,
            path: item.path,
            success: false,
            size: item.size_display,
            message: `✗ ${item.project_name} — 失败: ${e}`
          }];
        }
      }

      // 滚动到日志底部
      const logEl = document.querySelector('.nm-log');
      if (logEl) {
        await new Promise(r => setTimeout(r, 10));
        logEl.scrollTop = logEl.scrollHeight;
      }
    }

    nmCleaning = false;
    if (!nmCleanStopped) nmDone = true;
    // 用 tick 触发更新
    await new Promise(r => setTimeout(r, 0));
  }

  function stopCleaning() {
    nmCleanStopped = true;
  }
</script>

<div class="nm-page">
  <!-- 页头 -->
  <div class="page-header">
    <div class="header-left">
      <button class="btn-back" onclick={() => goto('/tools')} title="返回工具箱">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6" /></svg>
      </button>
      <h1>node_modules 清理</h1>
    </div>
  </div>

  <!-- 工作空间信息 -->
  <div class="ws-info">
    {#if isLoading}
      <span class="ws-loading">加载中...</span>
    {:else if activeWorkspace}
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
      <span class="ws-path" title={activeWorkspace}>{activeWorkspace}</span>
    {:else}
      <span class="ws-empty">尚未设置工作空间，请前往<a href="/settings" class="ws-link">设置</a>页面配置</span>
    {/if}
  </div>

  <!-- 扫描按钮 -->
  <div class="scan-section">
    <button class="scan-btn" onclick={scanNodeModules} disabled={!activeWorkspace || nmScanning || nmCleaning}>
      {#if nmScanning}
        <div class="spinner"></div>
        正在扫描...
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
        扫描 node_modules
      {/if}
    </button>
    {#if nmScanned && nmList.length > 0}
      <button class="clean-all-btn" onclick={cleanAll} disabled={nmCleaning || nmDone}>
        {#if nmCleaning}
          <div class="spinner"></div>
          清理中 {nmCleanIndex}/{nmList.length} ...
        {:else if nmDone}
          清理完成
        {:else}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          一键清理全部 ({nmList.length})
        {/if}
      </button>
      {#if nmCleaning}
        <button class="stop-btn" onclick={stopCleaning}>停止</button>
      {/if}
    {/if}
  </div>

  <!-- 扫描结果 -->
  {#if nmScanned}
    {#if nmList.length === 0}
      <div class="nm-empty">
        <span class="nm-empty-icon">📦</span>
        <p>当前工作空间中未找到 node_modules 文件夹</p>
      </div>
    {:else}
      <div class="nm-stats">
        <span class="stat"><strong>{nmProjectCount}</strong> 个项目</span>
        <span class="stat-divider">·</span>
        <span class="stat"><strong>{nmList.length}</strong> 个 node_modules</span>
        <span class="stat-divider">·</span>
        <span class="stat">总计 <strong>{formatBytes(nmTotalSize)}</strong></span>
        {#if nmList.filter(n => n.has_pnpm_lock).length > 0}
          <span class="stat-divider">·</span>
          <span class="stat"><strong>{nmList.filter(n => n.has_pnpm_lock).length}</strong> 个 pnpm</span>
        {/if}
        {#if nmCleanCount > 0}
          <span class="stat-divider">·</span>
          <span class="stat stat-cleaned">已清理 <strong>{formatBytes(nmCleanedSize)}</strong></span>
        {/if}
      </div>

      <!-- 进度条 -->
      {#if nmCleaning || nmDone}
        <div class="progress-bar-track">
          <div class="progress-bar-fill" style="width: {nmList.length > 0 ? (nmCleanIndex / nmList.length) * 100 : 0}%"></div>
          <span class="progress-text">{nmCleanIndex}/{nmList.length}</span>
        </div>
        {#if nmFailedCount > 0}
          <div class="progress-errors">{nmFailedCount} 个删除失败</div>
        {/if}
      {/if}

      <!-- 日志 -->
      {#if nmCleanLogs.length > 0}
        <div class="nm-log" class:has-content={nmCleanLogs.length > 0}>
          {#each nmCleanLogs as log}
            <div class="nm-log-item" class:nm-log-success={log.success} class:nm-log-error={!log.success}>
              <span class="nm-log-message">{log.message}</span>
            </div>
          {/each}
          {#if nmDone && nmFailedCount === 0 && nmSkippedCount === 0}
            <div class="nm-log-item nm-log-done">
              <span class="nm-log-message">🎉 全部清理完成，共释放 {formatBytes(nmCleanedSize)}</span>
            </div>
          {/if}
          {#if nmDone && nmFailedCount === 0 && nmSkippedCount > 0}
            <div class="nm-log-item nm-log-done">
              <span class="nm-log-message">🎉 清理完成，{nmSkippedCount} 个 pnpm 项目已跳过，释放 {formatBytes(nmCleanedSize)}</span>
            </div>
          {/if}
          {#if nmDone && nmFailedCount > 0}
            <div class="nm-log-item nm-log-error">
              <span class="nm-log-message">⚠️ 清理完成，{nmFailedCount} 个失败，{nmSkippedCount} 个跳过</span>
            </div>
          {/if}
        </div>
      {/if}

      <!-- 列表 -->
      <div class="nm-list">
        <div class="nm-list-header">
          <span>路径</span>
          <span>大小</span>
        </div>
        {#each nmList as nm (nm.path)}
          {@const cleaned = nmCleanLogs.find(l => l.path === nm.path && l.success)}
          <div class="nm-item" class:nm-item-cleaned={!!cleaned} class:nm-item-pnpm={nm.has_pnpm_lock}>
            <div class="nm-item-left">
              <span class="nm-item-project">{nm.project_name}</span>
              <span class="nm-item-sep">/</span>
              <span class="nm-item-folder">node_modules</span>
            </div>
            <div class="nm-item-right">
              <span class="nm-item-size">{nm.size_display}</span>
              {#if cleaned}
                <span class="nm-item-badge">已删除</span>
              {:else if nm.has_pnpm_lock}
                <span class="nm-badge-pnpm">pnpm</span>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .nm-page {
    padding: 24px 32px;
    max-width: 800px;
    margin: 0 auto;
  }

  .page-header {
    margin-bottom: 16px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .btn-back {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-back:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .page-header h1 {
    font-size: 22px;
    font-weight: 600;
    color: var(--text-primary);
  }

  /* 工作空间信息 */
  .ws-info {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    margin-bottom: 20px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .ws-path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ws-empty {
    color: var(--text-muted);
  }

  .ws-link {
    color: var(--accent);
    text-decoration: none;
  }

  .ws-link:hover {
    text-decoration: underline;
  }

  .ws-loading {
    color: var(--text-muted);
  }

  /* 扫描和清理按钮区 */
  .scan-section {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    margin-bottom: 20px;
  }

  .scan-btn, .clean-all-btn, .stop-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 10px 20px;
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .scan-btn {
    background: var(--accent-gradient);
    color: white;
  }

  .scan-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 2px 8px var(--accent-shadow);
  }

  .clean-all-btn {
    background: linear-gradient(135deg, #ff5252, #ff1744);
    color: white;
  }

  .clean-all-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(255,23,68,0.3);
  }

  .stop-btn {
    background: var(--bg-input);
    color: var(--text-secondary);
    border: 1px solid var(--border);
  }

  .stop-btn:hover {
    background: var(--border-light);
  }

  .scan-btn:disabled, .clean-all-btn:disabled, .stop-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* 空状态 */
  .nm-empty {
    text-align: center;
    padding: 48px 20px;
    color: var(--text-secondary);
  }

  .nm-empty-icon {
    font-size: 48px;
    display: block;
    margin-bottom: 12px;
  }

  .nm-empty p {
    margin: 0;
    font-size: 15px;
  }

  /* 统计信息 */
  .nm-stats {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 12px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    margin-bottom: 16px;
    font-size: 14px;
    color: var(--text-secondary);
  }

  .stat strong {
    color: var(--text-primary);
  }

  .stat-divider {
    color: var(--border);
  }

  .stat-cleaned strong {
    color: #16a34a;
  }

  /* 进度条 */
  .progress-bar-track {
    position: relative;
    height: 24px;
    background: var(--bg-input);
    border-radius: 12px;
    overflow: hidden;
    margin-bottom: 12px;
  }

  .progress-bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #ff5252, #ff1744);
    border-radius: 12px;
    transition: width 0.3s ease;
  }

  .progress-text {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .progress-errors {
    font-size: 13px;
    color: #ff1744;
    margin-bottom: 12px;
    padding-left: 4px;
  }

  /* 日志 */
  .nm-log {
    max-height: 260px;
    overflow-y: auto;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 4px;
    margin-bottom: 16px;
    font-family: monospace;
    font-size: 13px;
  }

  .nm-log-item {
    padding: 6px 12px;
    border-radius: 6px;
    line-height: 1.5;
  }

  .nm-log-success {
    color: #16a34a;
  }

  .nm-log-error {
    color: #ff1744;
  }

  .nm-log-done {
    color: var(--accent);
    font-weight: 600;
    border-top: 1px solid var(--border);
    margin-top: 4px;
    padding-top: 10px;
  }

  /* 列表 */
  .nm-list {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }

  .nm-list-header {
    display: flex;
    justify-content: space-between;
    padding: 10px 16px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
  }

  .nm-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    transition: background 0.15s;
  }

  .nm-item:last-child {
    border-bottom: none;
  }

  .nm-item:hover {
    background: var(--bg-subtle);
  }

  .nm-item-cleaned {
    opacity: 0.45;
  }

  .nm-item-left {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }

  .nm-item-project {
    color: var(--text-primary);
    font-weight: 500;
    font-size: 14px;
  }

  .nm-item-sep {
    color: var(--text-muted);
    font-size: 14px;
  }

  .nm-item-folder {
    color: var(--accent);
    font-size: 14px;
  }

  .nm-item-right {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
    margin-left: 16px;
  }

  .nm-item-size {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .nm-item-pnpm {
    opacity: 0.6;
  }

  .nm-badge-pnpm {
    font-size: 10px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(96, 165, 250, 0.15);
    color: #60a5fa;
    letter-spacing: 0.3px;
  }

  .nm-item-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(34,197,94,0.12);
    color: #16a34a;
  }

  @media (max-width: 640px) {
    .nm-page {
      padding: 16px;
    }
  }
</style>
