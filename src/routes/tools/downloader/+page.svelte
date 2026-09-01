<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { formatSize, formatSpeed, formatRelativeSec } from '$lib/format';
  import Modal from '$lib/ui/Modal.svelte';

  // ===== 类型 =====
  interface DownloadTask {
    id: string;
    url: string;
    filename: string;
    save_path: string;
    file_path: string;
    total_size: number;
    downloaded_size: number;
    status: string;
    error: string | null;
    created_at: number;
    started_at: number | null;
    completed_at: number | null;
    speed: number;
  }

  interface ProgressEvent {
    id: string;
    downloaded: number;
    total: number;
    speed: number;
    progress: number;
  }

  // 最终下载事件（下载结束：完成/失败，含时间戳）
  interface FinalEvent {
    id: string;
    status: string;
    filename: string;
    file_path: string;
    finished_at: number | null;
    total: number;
    error: string | null;
  }

  // ===== 状态 =====
  let tasks = $state<DownloadTask[]>([]);
  let loading = $state(true);
  let error = $state('');

  // 新建下载弹窗
  let showAddModal = $state(false);
  let newUrl = $state('');
  let newFilename = $state('');
  let newPath = $state('');
  let formError = $state('');
  let formSaving = $state(false);

  // 进度跟踪
  let progressMap = $state<Record<string, ProgressEvent>>({});

  // 事件监听器
  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFinal: UnlistenFn | null = null;

  // 最终下载事件提示（完成/失败，带时间）
  let finalNotice = $state<{ type: 'success' | 'error'; message: string } | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  function showFinalNotice(type: 'success' | 'error', message: string) {
    finalNotice = { type, message };
    if (noticeTimer) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (finalNotice = null), 5000);
  }

  onMount(async () => {
    await loadTasks();

    // 监听下载进度
    unlistenProgress = await listen<ProgressEvent>('download-progress', (event) => {
      progressMap[event.payload.id] = event.payload;
    });

    // 监听下载完成
    unlistenComplete = await listen<{ id: string }>('download-complete', () => {
      loadTasks();
    });

    // 监听最终下载事件（完成/失败，含时间戳）
    unlistenFinal = await listen<FinalEvent>('download-final', (event) => {
      const p = event.payload;
      const time = p.finished_at ? formatRelativeSec(p.finished_at) : '';
      if (p.status === 'Completed') {
        showFinalNotice('success', `下载完成「${p.filename}」${time}`);
      } else if (p.status === 'Failed') {
        showFinalNotice('error', `下载失败「${p.filename}」${time}${p.error ? `：${p.error}` : ''}`);
      }
      loadTasks();
    });
  });

  onDestroy(() => {
    unlistenProgress?.();
    unlistenComplete?.();
    unlistenFinal?.();
  });

  async function loadTasks() {
    loading = true;
    error = '';
    try {
      tasks = await invoke<DownloadTask[]>('list_downloads');
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  }

  function getProgress(taskId: string): ProgressEvent | undefined {
    return progressMap[taskId];
  }

  // ===== 新增下载 =====
  function openAddModal() {
    newUrl = '';
    newFilename = '';
    newPath = '';
    formError = '';
    formSaving = false;
    showAddModal = true;
  }

  async function addDownload() {
    formError = '';
    if (!newUrl.trim()) {
      formError = '请输入下载链接';
      return;
    }

    formSaving = true;
    try {
      await invoke('add_download', {
        url: newUrl.trim(),
        filename: newFilename.trim() || null,
        savePath: newPath.trim() || null,
      });
      showAddModal = false;
      await loadTasks();
    } catch (e) {
      formError = `添加失败: ${e}`;
    } finally {
      formSaving = false;
    }
  }

  // ===== 操作 =====
  async function pauseTask(taskId: string) {
    try {
      await invoke('pause_download', { taskId });
      await loadTasks();
    } catch (e) {
      error = `暂停失败: ${e}`;
    }
  }

  async function retryTask(taskId: string) {
    try {
      await invoke('retry_download', { taskId });
      await loadTasks();
    } catch (e) {
      error = `重试失败: ${e}`;
    }
  }

  async function deleteTask(taskId: string, deleteFile: boolean = false) {
    try {
      await invoke('delete_download', { taskId, deleteFile });
      delete progressMap[taskId];
      await loadTasks();
    } catch (e) {
      error = `删除失败: ${e}`;
    }
  }

  async function openFolder(path: string) {
    try {
      await invoke('open_download_folder', { path });
    } catch (e) {
      error = `打开文件夹失败: ${e}`;
    }
  }

  async function openFile(path: string) {
    try {
      await invoke('open_download_file', { path });
    } catch (e) {
      error = `打开文件失败: ${e}`;
    }
  }

  // 清空记录（二次确认，防止误操作）
  let confirmClear = $state(false);

  async function clearHistory() {
    if (!confirmClear) {
      confirmClear = true;
      return;
    }
    confirmClear = false;
    try {
      await invoke('clear_download_history');
      progressMap = {};
      await loadTasks();
    } catch (e) {
      error = `清空失败: ${e}`;
    }
  }

  // ===== 状态显示 =====
  function getStatusInfo(status: string) {
    switch (status) {
      case 'Pending':
        return { text: '等待中', class: 's-pending' };
      case 'Downloading':
        return { text: '下载中', class: 's-downloading' };
      case 'Paused':
        return { text: '已暂停', class: 's-paused' };
      case 'Completed':
        return { text: '已完成', class: 's-completed' };
      case 'Failed':
        return { text: '失败', class: 's-failed' };
      case 'Cancelled':
        return { text: '已取消', class: 's-cancelled' };
      default:
        return { text: status, class: '' };
    }
  }

  function getProgressBar(task: DownloadTask): number {
    const progress = getProgress(task.id);
    if (progress) return progress.progress;
    if (task.total_size > 0) return Math.round((task.downloaded_size / task.total_size) * 100);
    return 0;
  }
</script>

<div class="downloader-page">
  <!-- 页头 -->
  <div class="page-header">
    <div class="header-left">
      <button class="btn-back" onclick={() => goto('/tools')}>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <line x1="19" y1="12" x2="5" y2="12" />
          <polyline points="12 19 5 12 12 5" />
        </svg>
        返回
      </button>
      <h1>下载记录</h1>
    </div>
    <div class="header-actions">
      <button class="btn-clear" class:confirming={confirmClear} onclick={clearHistory}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="3 6 5 6 21 6" />
          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          <line x1="10" y1="11" x2="10" y2="17" />
          <line x1="14" y1="11" x2="14" y2="17" />
        </svg>
        {confirmClear ? '确认清空？' : '清空记录'}
      </button>
      <button class="btn-refresh" onclick={loadTasks} disabled={loading}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="23 4 23 10 17 10" />
          <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
        </svg>
        刷新
      </button>
      <button class="btn-add" onclick={openAddModal}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        新建下载
      </button>
    </div>
  </div>

  {#if error}
    <div class="error-banner">
      <span>⚠️</span>
      <span>{error}</span>
      <button class="error-dismiss" onclick={() => (error = '')}>✕</button>
    </div>
  {/if}

  <!-- 最终下载事件提示（完成/失败，带时间） -->
  {#if finalNotice}
    <div class="final-notice {finalNotice.type}">
      <span>{finalNotice.type === 'success' ? '✅' : '⚠️'}</span>
      <span>{finalNotice.message}</span>
      <button class="error-dismiss" onclick={() => (finalNotice = null)}>✕</button>
    </div>
  {/if}

  <!-- 加载 -->
  {#if loading}
    <div class="loading">
      <div class="spinner"></div>
      <span>正在加载...</span>
    </div>
  {/if}

  <!-- 空状态 -->
  {#if !loading && tasks.length === 0 && !error}
    <div class="empty-state">
      <div class="empty-icon-big">📥</div>
      <h3>暂无下载记录</h3>
      <p>点击「新建下载」按钮开始第一个下载</p>
    </div>
  {/if}

  <!-- 下载列表 -->
  {#if !loading && tasks.length > 0}
    <div class="tasks-count">{tasks.length} 个任务</div>
    <div class="tasks-list">
      {#each tasks as task (task.id)}
        {@const statusInfo = getStatusInfo(task.status)}
        {@const progressValue = getProgressBar(task)}
        {@const prog = getProgress(task.id)}
        <div class="task-card">
          <div class="task-main">
            <div class="task-info">
              <div class="task-header-row">
                <h3 class="task-filename">{task.filename}</h3>
                <span class="task-status {statusInfo.class}">{statusInfo.text}</span>
              </div>
              <div class="task-url" title={task.url}>{task.url}</div>
              <div class="task-meta">
                {#if task.status === 'Downloading' && prog}
                  <span class="meta-speed">{formatSpeed(prog.speed)}</span>
                  <span class="meta-divider">·</span>
                  <span class="meta-size">{formatSize(prog.downloaded)} / {formatSize(prog.total)}</span>
                {:else if task.total_size > 0}
                  <span class="meta-size">{formatSize(task.downloaded_size)} / {formatSize(task.total_size)}</span>
                {:else if task.status === 'Downloading'}
                  <span class="meta-size">{formatSize(task.downloaded_size)}</span>
                {/if}
                <span class="meta-divider">·</span>
                <span class="meta-time">{formatRelativeSec(task.created_at)}</span>
                {#if task.status === 'Completed' && task.completed_at}
                  <span class="meta-divider">·</span>
                  <span class="meta-time">完成于 {formatRelativeSec(task.completed_at)}</span>
                {/if}
              </div>
            </div>

            <!-- 进度条 -->
            {#if task.status === 'Downloading' || task.status === 'Paused' || (task.status === 'Pending')}
              <div class="progress-bar-wrapper">
                <div class="progress-bar">
                  <div class="progress-fill {task.status === 'Paused' ? 'paused' : ''}" style="width: {progressValue}%"></div>
                </div>
                <span class="progress-text">{progressValue}%</span>
              </div>
            {/if}

            <!-- 错误信息 -->
            {#if task.error}
              <div class="task-error">
                <span>⚠️</span>
                <span>{task.error}</span>
              </div>
            {/if}
          </div>

          <div class="task-actions">
            <!-- 下载中 → 暂停 -->
            {#if task.status === 'Downloading'}
              <button class="btn-action btn-pause" onclick={() => pauseTask(task.id)} title="暂停">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="6" y="4" width="4" height="16" />
                  <rect x="14" y="4" width="4" height="16" />
                </svg>
                暂停
              </button>
            {/if}

            <!-- 暂停/失败/取消 → 继续/重试 -->
            {#if task.status === 'Paused'}
              <button class="btn-action btn-resume" onclick={() => invoke('start_download', { taskId: task.id }).then(loadTasks)} title="继续">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polygon points="5 3 19 12 5 21 5 3" />
                </svg>
                继续
              </button>
            {/if}

            {#if task.status === 'Failed' || task.status === 'Cancelled'}
              <button class="btn-action btn-retry" onclick={() => retryTask(task.id)} title="重试">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="23 4 23 10 17 10" />
                  <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
                </svg>
                重试
              </button>
            {/if}

            <!-- 已完成 → 打开文件/文件夹 -->
            {#if task.status === 'Completed'}
              <button class="btn-action btn-file" onclick={() => openFile(task.file_path)} title="打开文件">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
                打开文件
              </button>
              <button class="btn-action btn-folder" onclick={() => openFolder(task.save_path)} title="打开文件夹">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
            {/if}

            <!-- 删除（带确认） -->
            <div class="btn-group">
              <button class="btn-action btn-delete-sm" onclick={() => deleteTask(task.id, false)} title="删除任务">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="3 6 5 6 21 6" />
                  <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                </svg>
              </button>
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- 新建下载弹窗 -->
{#if showAddModal}
  <Modal title="新建下载" onClose={() => (showAddModal = false)}>
    <div class="modal-body">
      {#if formError}
        <div class="form-error">{formError}</div>
      {/if}
      <div class="form-group">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>下载链接 <span class="required">*</span></label>
        <input
          type="text"
          bind:value={newUrl}
          placeholder="https://example.com/file.zip"
        />
      </div>
      <div class="form-group">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>文件名 <span class="optional-tag">(可选)</span></label>
        <input
          type="text"
          bind:value={newFilename}
          placeholder="留空自动从 URL 提取"
        />
      </div>
      <div class="form-group">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>保存路径 <span class="optional-tag">(可选)</span></label>
        <input
          type="text"
          bind:value={newPath}
          placeholder="留空使用系统下载目录"
        />
      </div>
    </div>
    <div class="modal-footer">

      <button class="btn-save" onclick={addDownload} disabled={formSaving}>
        {formSaving ? '添加中...' : '开始下载'}
      </button>
    </div>
  </Modal>
{/if}

<style>
  .downloader-page {
    padding: 24px;
    max-width: 960px;
    margin: 0 auto;
  }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 24px;
    flex-wrap: wrap;
    gap: 12px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .header-left h1 {
    font-size: 24px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .btn-back {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-back:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn-refresh, .btn-add {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-clear {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    background: var(--bg-subtle);
    border: 1px solid var(--error-border);
    color: var(--error-text);
  }

  .btn-clear:hover {
    background: var(--error-bg);
  }

  .btn-clear.confirming {
    background: var(--error-text);
    border-color: var(--error-text);
    color: #fff;
  }

  .btn-refresh {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    color: var(--text-secondary);
  }

  .btn-refresh:hover:not(:disabled) {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .btn-refresh:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-add {
    background: var(--accent);
    border: 1px solid transparent;
    color: white;
  }

  .btn-add:hover {
    background: var(--accent-hover);
  }

  .error-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 10px;
    color: var(--error-text);
    font-size: 14px;
    margin-bottom: 16px;
  }

  .error-dismiss {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--error-text);
    cursor: pointer;
    font-size: 16px;
    padding: 2px;
  }

  .final-notice {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-radius: 10px;
    font-size: 14px;
    margin-bottom: 16px;
    border: 1px solid;
  }

  .final-notice.success {
    background: var(--success-bg);
    border-color: var(--success-text);
    color: var(--success-text);
  }

  .final-notice.error {
    background: var(--error-bg);
    border-color: var(--error-border);
    color: var(--error-text);
  }

  .final-notice .error-dismiss {
    color: inherit;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 60px;
    color: var(--text-muted);
    font-size: 14px;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 80px 24px;
    text-align: center;
  }

  .empty-icon-big {
    font-size: 64px;
    margin-bottom: 16px;
  }

  .empty-state h3 {
    font-size: 18px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .empty-state p {
    font-size: 14px;
    color: var(--text-muted);
    margin: 8px 0 0;
  }

  .tasks-count {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 12px;
  }

  .task-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    margin-bottom: 8px;
    transition: border-color 0.2s;
  }

  .task-card:hover {
    border-color: var(--border-strong);
  }

  .task-main {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .task-info {
    flex: 1;
  }

  .task-header-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }

  .task-filename {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
    word-break: break-all;
  }

  .task-status {
    font-size: 11px;
    font-weight: 500;
    padding: 2px 8px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .s-pending { background: var(--bg-subtle); color: var(--text-muted); }
  .s-downloading { background: rgba(59, 130, 246, 0.12); color: #2563eb; }
  .s-paused { background: rgba(245, 158, 11, 0.12); color: #d97706; }
  .s-completed { background: rgba(34, 197, 94, 0.12); color: #16a34a; }
  .s-failed { background: var(--error-bg); color: var(--error-text); }
  .s-cancelled { background: var(--bg-subtle); color: var(--text-muted); }

  .task-url {
    font-size: 13px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-bottom: 4px;
  }

  .task-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    color: var(--text-muted);
  }

  .meta-divider {
    color: var(--border);
  }

  /* 进度条 */
  .progress-bar-wrapper {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .progress-bar {
    flex: 1;
    height: 6px;
    background: var(--bg-subtle);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 3px;
    transition: width 0.3s;
  }

  .progress-fill.paused {
    background: #d97706;
  }

  .progress-text {
    font-size: 13px;
    color: var(--text-muted);
    min-width: 40px;
    text-align: right;
  }

  .task-error {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 6px;
    font-size: 13px;
    color: var(--error-text);
  }

  /* 操作按钮 */
  .task-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .btn-group {
    margin-left: auto;
  }

  .btn-action {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 10px;
    background: var(--bg-subtle);
    border: 1px solid var(--border-light);
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-action:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
    border-color: var(--border);
  }

  .btn-pause:hover { color: #d97706; border-color: #d97706; }
  .btn-resume:hover { color: #2563eb; border-color: #2563eb; }
  .btn-retry:hover { color: var(--accent); border-color: var(--accent); }
  .btn-file:hover { color: #16a34a; border-color: #16a34a; }
  .btn-folder:hover { color: var(--accent); border-color: var(--accent); }
  .btn-delete-sm:hover { color: var(--error-text); border-color: var(--error-border); }

</style>