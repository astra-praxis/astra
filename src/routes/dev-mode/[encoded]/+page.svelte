<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';

  interface DevSubDir {
    label: string;
    key: string;
    work_dir: string;
    has_package_json: boolean;
  }

  interface DevCardInfo {
    name: string;
    path: string;
    sub_dirs: DevSubDir[];
  }

  interface DevServerInfo {
    id: string;
    card_name: string;
    subdir: string;
    work_dir: string;
    command: string;
    started_at: number;
    status: string;
    pid: number | null;
  }

  let { params } = $props();
  let projectPath = $derived(decodeURIComponent(params.encoded));
  let projectName = $derived(projectPath.split('/').pop() || '');

  let cards = $state<DevCardInfo[]>([]);
  let loading = $state(true);
  let error = $state('');

  // 运行中的服务器映射：server_id -> DevServerInfo
  let runningServers = $state<Record<string, DevServerInfo>>({});
  // 按 "card_name:subdir_key" 汇总正在运行的项
  let activeKeys = $derived(new Set(
    Object.values(runningServers)
      .filter(s => s.status === 'running')
      .map(s => `${s.card_name}:${s.subdir}`)
  ));

  // 日志：每台服务器独立的日志列表
  let logsMap = $state<Record<string, { stream: string; line: string }[]>>({});

  // 每个卡片子目录的安装状态
  let installing = $state<Record<string, boolean>>({});

  // 命令执行日志
  let cmdLogs = $state<Record<string, string[]>>({});

  let unlisteners: UnlistenFn[] = [];

  onMount(() => {
    init();

    return () => {
      for (const u of unlisteners) u();
    };
  });

  async function init() {
    // 监听服务器日志
    const unlog = await listen<{
      server_id: string; card_name: string; subdir_key: string;
      work_dir: string; stream: string; line: string;
    }>('dev-server-log', (event) => {
      const { server_id, stream, line } = event.payload;
      if (!logsMap[server_id]) logsMap[server_id] = [];
      logsMap[server_id] = [...logsMap[server_id], { stream, line }];
    });
    unlisteners.push(unlog);

    // 监听命令日志
    const uncmd = await listen<{
      work_dir: string; subdir_key: string; command: string;
      stream: string; line: string;
    }>('dev-cmd-line', (event) => {
      const key = `${event.payload.work_dir}:${event.payload.command}`;
      if (!cmdLogs[key]) cmdLogs[key] = [];
      cmdLogs[key] = [...cmdLogs[key], `[${event.payload.stream}] ${event.payload.line}`];
    });
    unlisteners.push(uncmd);

    // 监听命令完成
    const uncdone = await listen<{
      work_dir: string; subdir_key: string; command: string;
      exit_code: number; success: boolean;
    }>('dev-cmd-done', (event) => {
      const key = `${event.payload.work_dir}:${event.payload.command}`;
      if (!cmdLogs[key]) cmdLogs[key] = [];
      cmdLogs[key] = [...cmdLogs[key], `--- 命令完成 (退出码: ${event.payload.exit_code}) ---`];
      // 清除安装状态
      const card = cards.find(c => c.sub_dirs.some(sd => sd.work_dir === event.payload.work_dir));
      if (card && event.payload.command === 'install') {
        const subDir = card.sub_dirs.find(sd => sd.work_dir === event.payload.work_dir);
        if (subDir) {
          const installKey = `${card.name}:${subDir.key}`;
          installing = { ...installing, [installKey]: false };
        }
      }
    });
    unlisteners.push(uncdone);

    // 监听服务器停止（移除已停止的服务器）
    const unst = await listen<{
      server_id: string; exit_code?: number; success?: boolean; error?: string;
    }>('dev-server-stopped', (event) => {
      const { server_id } = event.payload;
      const newServers = { ...runningServers };
      delete newServers[server_id];
      runningServers = newServers;
    });
    unlisteners.push(unst);

    // 加载数据
    try {
      cards = await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath });
      const servers = await invoke<DevServerInfo[]>('list_dev_servers');
      const sMap: Record<string, DevServerInfo> = {};
      for (const s of servers) {
        sMap[s.id] = s;
      }
      runningServers = sMap;
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  }

  function isRunning(card: DevCardInfo, subDir: DevSubDir): boolean {
    return activeKeys.has(`${card.name}:${subDir.key}`);
  }

  function serverIdFor(card: DevCardInfo, subDir: DevSubDir): string | undefined {
    return Object.values(runningServers).find(s => s.card_name === card.name && s.subdir === subDir.key)?.id;
  }

  async function startDev(card: DevCardInfo, subDir: DevSubDir) {
    try {
      const id = await invoke<string>('start_dev_server', {
        workDir: subDir.work_dir,
        cardName: card.name,
        subdirKey: subDir.key,
      });
      runningServers = {
        ...runningServers,
        [id]: {
          id,
          card_name: card.name,
          subdir: subDir.key,
          work_dir: subDir.work_dir,
          command: 'pnpm dev',
          started_at: Math.floor(Date.now() / 1000),
          status: 'running',
          pid: null,
        },
      };
    } catch (e) {
      error = `启动失败: ${e}`;
    }
  }

  async function stopDev(serverId: string) {
    try {
      await invoke<boolean>('stop_dev_server', { serverId });
      const newServers = { ...runningServers };
      delete newServers[serverId];
      runningServers = newServers;
    } catch (e) {
      error = `停止失败: ${e}`;
    }
  }

  async function runInstall(card: DevCardInfo, subDir: DevSubDir) {
    const installKey = `${card.name}:${subDir.key}`;
    installing = { ...installing, [installKey]: true };
    const cmdKey = `${subDir.work_dir}:install`;
    cmdLogs[cmdKey] = [];
    try {
      await invoke<string>('run_card_command', {
        workDir: subDir.work_dir,
        subdirKey: subDir.key,
        commandName: 'install',
      });
    } catch (e) {
      error = `安装失败: ${e}`;
    }
  }

  function formatTime(ts: number): string {
    const d = new Date(ts * 1000);
    return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}`;
  }

  function serverLogs(serverId: string) {
    return logsMap[serverId] || [];
  }

  function subDirTags(card: DevCardInfo): string {
    return card.sub_dirs.map(sd => sd.label).join(' · ');
  }
</script>

<div class="dev-mode-page">
  <!-- 导航栏 -->
  <div class="dev-nav">
    <button class="back-btn" onclick={() => goto('/')}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
      返回工作空间
    </button>
    <div class="dev-nav-info">
      <div class="dev-nav-icon">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
      </div>
      <span class="dev-nav-title">{projectName}</span>
      <span class="dev-nav-badge">开发模式</span>
    </div>
  </div>

  {#if loading}
    <div class="dev-loading">
      <div class="spinner"></div>
      <span>正在扫描卡片目录...</span>
    </div>
  {:else if error}
    <div class="dev-error">{error}</div>
  {:else}
    <!-- 卡片列表 -->
    <div class="card-grid">
      {#each cards as card (card.path)}
        <div class="card-item">
          <div class="card-item-header">
            <div class="card-item-name">
              {card.name}
              {#if card.name === 'casp-portal-webapps'}
                <span class="card-badge-main">主项目</span>
              {/if}
            </div>
            <div class="card-item-dirs">{subDirTags(card)}</div>
          </div>

          <div class="card-item-actions">
            {#each card.sub_dirs as subDir (subDir.key)}
              <div class="subdir-row">
                <span class="subdir-label">{subDir.label}</span>
                <div class="subdir-path" title={subDir.work_dir}>{subDir.work_dir.split('/').slice(-3).join('/')}</div>
                <div class="subdir-btns">
                  {#if isRunning(card, subDir)}
                    <button class="btn-stop" onclick={() => { const id = serverIdFor(card, subDir); if (id) stopDev(id); }}>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>
                      停止
                    </button>
                  {:else if subDir.has_package_json}
                    <button class="btn-start" onclick={() => startDev(card, subDir)}>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><polygon points="8,5 19,12 8,19"/></svg>
                      启动
                    </button>
                  {/if}
                  {#if subDir.has_package_json}
                    <button class="btn-install" onclick={() => runInstall(card, subDir)} disabled={installing[`${card.name}:${subDir.key}`]}>
                      {installing[`${card.name}:${subDir.key}`] ? '安装中...' : 'pnpm install'}
                    </button>
                  {/if}
                </div>
              </div>
            {/each}
          </div>

          <!-- 运行中的服务器日志 -->
          {#each card.sub_dirs as subDir (subDir.key)}
            {#if isRunning(card, subDir)}
              {@const sid = serverIdFor(card, subDir)}
              {#if sid}
                <div class="server-log">
                  <div class="server-log-header">
                    <span class="server-log-status running"></span>
                    <span class="server-log-title">{subDir.label} 运行中</span>
                    <span class="server-log-time">{formatTime(runningServers[sid]?.started_at ?? 0)}</span>
                    <span class="server-log-pid">PID: {runningServers[sid]?.pid ?? '-'}</span>
                  </div>
                  <div class="server-log-content">
                    {#each serverLogs(sid) as log}
                      <div class="log-line" class:log-stderr={log.stream === 'stderr'}>{log.line}</div>
                    {/each}
                    {#if serverLogs(sid).length === 0}
                      <div class="log-empty">等待输出...</div>
                    {/if}
                  </div>
                </div>
              {/if}
            {/if}
          {/each}

          <!-- 安装日志 -->
          {#each card.sub_dirs as subDir (subDir.key)}
            {#if cmdLogs[`${subDir.work_dir}:install`]?.length > 0}
              {@const logs = cmdLogs[`${subDir.work_dir}:install`]}
              {@const isInstalling = installing[`${card.name}:${subDir.key}`]}
              <div class="server-log">
                <div class="server-log-header">
                  <span class="server-log-status" class:installing={isInstalling} class:completed={!isInstalling}></span>
                  <span class="server-log-title">{isInstalling ? `${subDir.label} 安装中` : `${subDir.label} 安装完成`}</span>
                </div>
                <div class="server-log-content">
                  {#each logs as line}
                    <div class="log-line">{line}</div>
                  {/each}
                </div>
              </div>
            {/if}
          {/each}
        </div>
      {/each}
    </div>

    {#if cards.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
        </div>
        <h3>未找到卡片目录</h3>
        <p>此项目下没有包含前端项目（web/ / mobile/）的子目录</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .dev-mode-page {
    padding: 24px;
    max-width: 1000px;
    margin: 0 auto;
  }

  .dev-nav {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 28px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
  }

  .back-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .dev-nav-info {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .dev-nav-icon {
    display: flex;
    align-items: center;
    color: var(--accent);
  }

  .dev-nav-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .dev-nav-badge {
    font-size: 11px;
    padding: 3px 10px;
    background: var(--accent-bg);
    color: var(--accent);
    border: 1px solid var(--accent-ring);
    border-radius: 20px;
    font-weight: 600;
  }

  .dev-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    padding: 80px 0;
    color: var(--text-secondary);
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .dev-error {
    padding: 16px 20px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 10px;
    color: var(--error-text);
    font-size: 14px;
  }

  .card-grid {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .card-item {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 20px;
    transition: box-shadow 0.2s;
  }

  .card-item:hover {
    box-shadow: 0 2px 12px var(--shadow-md);
  }

  .card-item-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .card-item-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .card-badge-main {
    font-size: 10px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--accent-bg);
    color: var(--accent);
    border: 1px solid var(--accent-ring);
    font-weight: 600;
    font-family: -apple-system, sans-serif;
    white-space: nowrap;
  }

  .card-item-dirs {
    font-size: 12px;
    color: var(--text-secondary);
    background: var(--bg-subtle);
    padding: 4px 10px;
    border-radius: 6px;
  }

  .card-item-actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 8px;
  }

  .subdir-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 0;
  }

  .subdir-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    min-width: 70px;
  }

  .subdir-path {
    font-size: 12px;
    color: var(--text-muted);
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .subdir-btns {
    display: flex;
    gap: 8px;
    white-space: nowrap;
  }

  .btn-start,
  .btn-stop,
  .btn-install {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
    border: none;
    white-space: nowrap;
  }

  .btn-start {
    background: var(--success-bg);
    color: var(--success-text);
  }

  .btn-start:hover {
    background: var(--accent-bg);
  }

  .btn-stop {
    background: var(--error-bg);
    color: var(--error-text);
  }

  .btn-stop:hover {
    background: var(--error-hover-bg);
  }

  .btn-install {
    background: var(--bg-subtle);
    color: var(--text-secondary);
    border: 1px solid var(--border);
  }

  .btn-install:hover:not(:disabled) {
    background: var(--bg-card-hover);
  }

  .btn-install:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .server-log {
    margin-top: 12px;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    overflow: hidden;
    background: var(--bg-subtle);
  }

  .server-log-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border-light);
  }

  .server-log-status {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .server-log-status.running {
    background: var(--success-text);
    box-shadow: 0 0 6px rgba(76, 175, 80, 0.6);
  }

  .server-log-status.installing {
    background: var(--accent);
    box-shadow: 0 0 6px var(--accent-shadow);
  }

  .server-log-status.completed {
    background: var(--success-text);
    opacity: 0.6;
  }

  .server-log-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .server-log-time {
    font-size: 11px;
    color: var(--text-muted);
  }

  .server-log-pid {
    font-size: 11px;
    color: var(--text-muted);
    margin-left: auto;
    font-family: 'SF Mono', 'Cascadia Code', monospace;
  }

  .server-log-content {
    padding: 10px 14px;
    max-height: 200px;
    overflow-y: auto;
  }

  .log-line {
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .log-stderr {
    color: var(--error-text);
  }

  .log-empty {
    font-family: 'SF Mono', 'Cascadia Code', monospace;
    font-size: 12px;
    color: var(--text-muted);
    font-style: italic;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 80px 0;
    text-align: center;
    color: var(--text-secondary);
  }

  .empty-icon {
    margin-bottom: 16px;
    opacity: 0.4;
  }

  .empty-state h3 {
    margin: 0 0 8px 0;
    font-size: 18px;
    color: var(--text-primary);
  }

  .empty-state p {
    margin: 0;
    font-size: 14px;
  }
</style>
