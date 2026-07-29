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
    display_name: string;
    folder_name: string;
    path: string;
    sub_dirs: DevSubDir[];
    category: string; // template / main / card
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
  let refreshing = $state(false);

  // 当前选中的卡片（folder_name）
  let selectedFolder = $state<string | null>(null);

  let runningServers = $state<Record<string, DevServerInfo>>({});
  let activeKeys = $derived(new Set(
    Object.values(runningServers)
      .filter(s => s.status === 'running')
      .map(s => `${s.card_name}:${s.subdir}`)
  ));

  let logsMap = $state<Record<string, { stream: string; line: string }[]>>({});
  let installing = $state<Record<string, boolean>>({});
  let cmdLogs = $state<Record<string, string[]>>({});
  let unlisteners: UnlistenFn[] = [];

  // 分类
  let mainCards = $derived(cards.filter(c => c.category === 'main'));
  let templateCards = $derived(cards.filter(c => c.category === 'template'));
  let cardList = $derived(cards.filter(c => c.category === 'card'));

  // 运行中列表
  let runningList = $derived(
    Object.values(runningServers)
      .filter(s => s.status === 'running')
      .sort((a, b) => a.started_at - b.started_at)
  );
  let runningBase = $derived(runningList.filter(s => {
    const c = cards.find(c => c.folder_name === s.card_name);
    return c && (c.category === 'main' || c.category === 'template');
  }));
  let runningCards = $derived(runningList.filter(s => {
    const c = cards.find(c => c.folder_name === s.card_name);
    return !c || c.category === 'card';
  }));

  // 选中的卡片（派生）
  let selectedCard = $derived(cards.find(c => c.folder_name === selectedFolder) ?? null);

  onMount(() => {
    init();
    return () => { for (const u of unlisteners) u(); };
  });

  async function init() {
    const unlog = await listen<{ server_id: string; stream: string; line: string }>('dev-server-log', (e) => {
      const { server_id, stream, line } = e.payload;
      if (!logsMap[server_id]) logsMap[server_id] = [];
      logsMap[server_id] = [...logsMap[server_id], { stream, line }];
    });
    unlisteners.push(unlog);

    const uncmd = await listen<{ work_dir: string; command: string; stream: string; line: string }>('dev-cmd-line', (e) => {
      const key = `${e.payload.work_dir}:${e.payload.command}`;
      if (!cmdLogs[key]) cmdLogs[key] = [];
      cmdLogs[key] = [...cmdLogs[key], `[${e.payload.stream}] ${e.payload.line}`];
    });
    unlisteners.push(uncmd);

    const uncdone = await listen<{ work_dir: string; command: string; exit_code: number; success: boolean }>('dev-cmd-done', (e) => {
      const key = `${e.payload.work_dir}:${e.payload.command}`;
      if (!cmdLogs[key]) cmdLogs[key] = [];
      cmdLogs[key] = [...cmdLogs[key], `─── 完成 (退出码 ${e.payload.exit_code}) ───`];
    });
    unlisteners.push(uncdone);

    const unst = await listen<{ server_id: string; exit_code?: number; success?: boolean }>('dev-server-stopped', (e) => {
      const ns = { ...runningServers };
      delete ns[e.payload.server_id];
      runningServers = ns;
      delete logsMap[e.payload.server_id];
    });
    unlisteners.push(unst);

    try {
      cards = await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath });
      const servers = await invoke<DevServerInfo[]>('list_dev_servers');
      const sMap: Record<string, DevServerInfo> = {};
      for (const s of servers) sMap[s.id] = s;
      runningServers = sMap;
      // 默认选中第一个卡片
      if (cardList.length > 0) selectedFolder = cardList[0].folder_name;
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  }

  function isRunning(card: DevCardInfo, subDir: DevSubDir): boolean {
    return activeKeys.has(`${card.folder_name}:${subDir.key}`);
  }
  function cardHasRunning(card: DevCardInfo): boolean {
    return card.sub_dirs.some(sd => isRunning(card, sd));
  }
  function serverIdFor(card: DevCardInfo, subDir: DevSubDir): string | undefined {
    return Object.values(runningServers).find(s => s.card_name === card.folder_name && s.subdir === subDir.key)?.id;
  }

  async function startDev(card: DevCardInfo, subDir: DevSubDir) {
    try {
      const id = await invoke<string>('start_dev_server', {
        workDir: subDir.work_dir, cardName: card.folder_name, subdirKey: subDir.key,
      });
      runningServers = { ...runningServers, [id]: {
        id, card_name: card.folder_name, subdir: subDir.key, work_dir: subDir.work_dir,
        command: 'pnpm dev', started_at: Math.floor(Date.now()/1000), status: 'running', pid: null,
      }};
    } catch (e) { error = `启动失败: ${e}`; }
  }

  async function stopDev(serverId: string) {
    try {
      await invoke<boolean>('stop_dev_server', { serverId });
      const ns = { ...runningServers }; delete ns[serverId]; runningServers = ns;
    } catch (e) { error = `停止失败: ${e}`; }
  }

  async function runInstall(card: DevCardInfo, subDir: DevSubDir) {
    const k = `${card.folder_name}:${subDir.key}`;
    installing = { ...installing, [k]: true };
    cmdLogs[`${subDir.work_dir}:install`] = [];
    try {
      await invoke<string>('run_card_command', { workDir: subDir.work_dir, subdirKey: subDir.key, commandName: 'install' });
    } catch (e) { error = `安装失败: ${e}`; }
  }

  function clearInstall(card: DevCardInfo, subDir: DevSubDir) {
    installing = { ...installing, [`${card.folder_name}:${subDir.key}`]: false };
  }

  // 切换卡片：停止所有运行中的卡片服务，启动新的
  let switching = $state(false);
  async function quickSwitch(card: DevCardInfo, subDir: DevSubDir) {
    if (isRunning(card, subDir)) return;
    switching = true;
    try {
      for (const s of runningCards) {
        await invoke<boolean>('stop_dev_server', { serverId: s.id });
        const ns = { ...runningServers }; delete ns[s.id]; runningServers = ns;
      }
      await startDev(card, subDir);
    } catch (e) { error = `切换失败: ${e}`; }
    finally { switching = false; }
  }

  async function refresh() {
    refreshing = true;
    try { cards = await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath }); }
    catch (e) { error = `刷新失败: ${e}`; }
    finally { refreshing = false; }
  }

  function serverCard(server: DevServerInfo): DevCardInfo | undefined {
    return cards.find(c => c.folder_name === server.card_name);
  }
  function serverSubDirLabel(server: DevServerInfo): string {
    const c = serverCard(server);
    const sd = c?.sub_dirs.find(s => s.key === server.subdir);
    return sd?.label ?? server.subdir;
  }
  function formatTime(ts: number): string {
    const d = new Date(ts * 1000);
    return `${d.getHours().toString().padStart(2,'0')}:${d.getMinutes().toString().padStart(2,'0')}:${d.getSeconds().toString().padStart(2,'0')}`;
  }
  function serverLogs(id: string) { return logsMap[id] || []; }
</script>

<div class="dev-mode-page">
  <!-- 导航栏 -->
  <div class="dev-nav">
    <button class="back-btn" onclick={() => goto('/')}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
      返回
    </button>
    <div class="dev-nav-info">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
      <span class="dev-nav-title">{projectName}</span>
      <span class="dev-nav-badge">开发模式</span>
    </div>
    <button class="refresh-btn" onclick={refresh} disabled={refreshing} title="刷新目录">
      {#if refreshing}
        <div class="btn-spinner-sm"></div>
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9"/><polyline points="21 3 21 9 15 9"/></svg>
      {/if}
    </button>
  </div>

  {#if loading}
    <div class="dev-loading"><div class="spinner"></div><span>正在扫描卡片目录...</span></div>
  {:else if error}
    <div class="dev-error">{error} <button class="error-dismiss" onclick={() => error = ''}>✕</button></div>
  {:else}
    <!-- 标签选择区 -->
    <div class="tag-section">
      {#if mainCards.length > 0 || templateCards.length > 0}
        <div class="tag-group">
          <span class="tag-group-label">基础</span>
          <div class="tag-items">
            {#each mainCards as card (card.folder_name)}
              <button class="tag" class:tag-active={selectedFolder === card.folder_name} class:tag-running={cardHasRunning(card)} onclick={() => selectedFolder = card.folder_name}>
                {#if cardHasRunning(card)}<span class="tag-dot"></span>{/if}
                {card.display_name}
              </button>
            {/each}
            {#each templateCards as card (card.folder_name)}
              <button class="tag" class:tag-active={selectedFolder === card.folder_name} class:tag-running={cardHasRunning(card)} onclick={() => selectedFolder = card.folder_name}>
                {#if cardHasRunning(card)}<span class="tag-dot"></span>{/if}
                {card.display_name}
              </button>
            {/each}
          </div>
        </div>
      {/if}
      <div class="tag-group">
        <span class="tag-group-label">卡片</span>
        <div class="tag-items">
          {#each cardList as card (card.folder_name)}
            <button class="tag" class:tag-active={selectedFolder === card.folder_name} class:tag-running={cardHasRunning(card)} onclick={() => selectedFolder = card.folder_name}>
              {#if cardHasRunning(card)}<span class="tag-dot"></span>{/if}
              {card.display_name}
            </button>
          {/each}
        </div>
      </div>
    </div>

    <!-- 选中卡片操作面板 -->
    {#if selectedCard}
      <div class="control-panel">
        <div class="control-panel-header">
          <div class="control-panel-title">
            <span class="cp-name">{selectedCard.display_name}</span>
            <span class="cp-folder">{selectedCard.folder_name}</span>
          </div>
        </div>
        <div class="control-panel-actions">
          {#each selectedCard.sub_dirs as subDir (subDir.key)}
            {#if subDir.has_package_json}
              <div class="cp-subdir">
                <span class="cp-subdir-label">{subDir.label}</span>
                <span class="cp-subdir-path" title={subDir.work_dir}>{subDir.work_dir.split('/').slice(-3).join('/')}</span>
                <div class="cp-subdir-btns">
                  {#if isRunning(selectedCard, subDir)}
                    <button class="btn-action btn-stop" onclick={() => { const id = serverIdFor(selectedCard, subDir); if (id) stopDev(id); }}>停止</button>
                  {:else}
                    {#if selectedCard.category === 'card'}
                      <button class="btn-action btn-switch" onclick={() => quickSwitch(selectedCard, subDir)} disabled={switching}>
                        {#if switching}⏳{:else}⇄{/if} 切换
                      </button>
                      <button class="btn-action btn-start-sm" onclick={() => startDev(selectedCard, subDir)}>启动</button>
                    {:else}
                      <button class="btn-action btn-start" onclick={() => startDev(selectedCard, subDir)}>启动</button>
                    {/if}
                  {/if}
                  <button class="btn-action btn-install" onclick={() => runInstall(selectedCard, subDir)} disabled={installing[`${selectedCard.folder_name}:${subDir.key}`]}>
                    {installing[`${selectedCard.folder_name}:${subDir.key}`] ? '安装中' : 'install'}
                  </button>
                </div>
              </div>
              <!-- 安装日志 -->
              {#if cmdLogs[`${subDir.work_dir}:install`]?.length > 0}
                <div class="mini-log" data-collapsed={installing[`${selectedCard.folder_name}:${subDir.key}`] ? 'false' : 'true'}>
                  {#each cmdLogs[`${subDir.work_dir}:install`] as line}
                    <div class="log-line">{line}</div>
                  {/each}
                </div>
              {/if}
            {/if}
          {/each}
        </div>
      </div>
    {/if}

    <!-- 运行中：基础服务 -->
    {#if runningBase.length > 0}
      <div class="running-section">
        <div class="running-section-header">基础服务</div>
        {#each runningBase as srv (srv.id)}
          {@const sc = serverCard(srv)}
          <div class="run-card">
            <div class="run-card-header">
              <span class="run-dot"></span>
              <span class="run-name">{sc?.display_name ?? srv.card_name}</span>
              <span class="run-sub">{serverSubDirLabel(srv)}</span>
              <span class="run-time">{formatTime(srv.started_at)}</span>
              <button class="run-stop-btn" onclick={() => stopDev(srv.id)} title="停止">
                <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
              </button>
            </div>
            <div class="run-log-body">
              {#each serverLogs(srv.id) as log}
                <div class="log-line" class:log-stderr={log.stream === 'stderr'}>{log.line}</div>
              {/each}
              {#if serverLogs(srv.id).length === 0}
                <div class="log-empty">等待输出...</div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <!-- 运行中：卡片服务 -->
    {#if runningCards.length > 0}
      <div class="running-section">
        <div class="running-section-header">卡片服务</div>
        {#each runningCards as srv (srv.id)}
          {@const sc = serverCard(srv)}
          <div class="run-card">
            <div class="run-card-header">
              <span class="run-dot"></span>
              <span class="run-name">{sc?.display_name ?? srv.card_name}</span>
              <span class="run-sub">{serverSubDirLabel(srv)}</span>
              <span class="run-time">{formatTime(srv.started_at)}</span>
              <button class="run-stop-btn" onclick={() => stopDev(srv.id)} title="停止">
                <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
              </button>
            </div>
            <div class="run-log-body">
              {#each serverLogs(srv.id) as log}
                <div class="log-line" class:log-stderr={log.stream === 'stderr'}>{log.line}</div>
              {/each}
              {#if serverLogs(srv.id).length === 0}
                <div class="log-empty">等待输出...</div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if cards.length === 0}
      <div class="empty-state">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
        <h3>未找到卡片目录</h3>
        <p>此项目下没有包含前端项目的子目录</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .dev-mode-page {
    padding: 20px;
    max-width: 900px;
    margin: 0 auto;
  }

  /* ===== 导航栏 ===== */
  .dev-nav {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 20px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all .2s;
    white-space: nowrap;
  }
  .back-btn:hover { background: var(--bg-card-hover); color: var(--text-primary); }

  .dev-nav-info { display: flex; align-items: center; gap: 8px; color: var(--accent); }
  .dev-nav-title { font-size: 16px; font-weight: 700; color: var(--text-primary); }
  .dev-nav-badge {
    font-size: 10px; padding: 2px 8px; border-radius: 10px;
    background: var(--accent-bg); color: var(--accent);
    border: 1px solid var(--accent-ring); font-weight: 600;
  }

  .refresh-btn {
    margin-left: auto;
    display: flex; align-items: center; justify-content: center;
    width: 34px; height: 34px; border-radius: 8px;
    background: var(--bg-card); border: 1px solid var(--border);
    color: var(--text-secondary); cursor: pointer; transition: all .2s;
  }
  .refresh-btn:hover:not(:disabled) { background: var(--bg-card-hover); color: var(--text-primary); }
  .refresh-btn:disabled { opacity: .5; cursor: not-allowed; }
  .btn-spinner-sm {
    width: 14px; height: 14px;
    border: 2px solid var(--border-strong); border-top-color: var(--accent);
    border-radius: 50%; animation: spin .6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .dev-loading, .empty-state {
    display: flex; flex-direction: column; align-items: center;
    justify-content: center; gap: 12px; padding: 60px 0;
    color: var(--text-secondary);
  }
  .spinner {
    width: 28px; height: 28px;
    border: 3px solid var(--border); border-top-color: var(--accent);
    border-radius: 50%; animation: spin .8s linear infinite;
  }
  .empty-state svg { opacity: .35; }
  .empty-state h3 { margin: 0; font-size: 17px; color: var(--text-primary); }
  .empty-state p { margin: 0; font-size: 14px; }

  .dev-error {
    padding: 14px 16px; background: var(--error-bg); border: 1px solid var(--error-border);
    border-radius: 10px; color: var(--error-text); font-size: 13px;
    display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px;
  }
  .error-dismiss { background: none; border: none; color: var(--error-text); cursor: pointer; font-size: 14px; }

  /* ===== 标签选择区 ===== */
  .tag-section { margin-bottom: 20px; }
  .tag-group { display: flex; align-items: flex-start; gap: 10px; margin-bottom: 10px; }
  .tag-group-label {
    font-size: 11px; font-weight: 700; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: .05em;
    min-width: 32px; padding-top: 7px; flex-shrink: 0;
  }
  .tag-items { display: flex; flex-wrap: wrap; gap: 7px; }

  .tag {
    display: inline-flex; align-items: center; gap: 5px;
    padding: 6px 13px; border-radius: 7px; font-size: 13px;
    font-weight: 500; background: var(--bg-card); border: 1px solid var(--border);
    color: var(--text-secondary); cursor: pointer; transition: all .15s;
    white-space: nowrap;
  }
  .tag:hover { background: var(--bg-card-hover); border-color: var(--border-strong); color: var(--text-primary); }
  .tag-active {
    background: var(--accent-bg); border-color: var(--accent);
    color: var(--accent); font-weight: 600;
  }
  .tag-running { border-color: var(--success-text); }
  .tag-running.tag-active { border-color: var(--accent); }
  .tag-dot {
    width: 7px; height: 7px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 4px rgba(76,175,80,.7);
    flex-shrink: 0;
  }

  /* ===== 操作面板 ===== */
  .control-panel {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 12px; padding: 18px 20px; margin-bottom: 20px;
  }
  .control-panel-header { margin-bottom: 14px; }
  .control-panel-title { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; }
  .cp-name { font-size: 16px; font-weight: 700; color: var(--text-primary); }
  .cp-folder {
    font-size: 11px; font-family: 'SF Mono','Cascadia Code',monospace;
    color: var(--text-muted);
  }

  .control-panel-actions { display: flex; flex-direction: column; gap: 8px; }
  .cp-subdir {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 0;
  }
  .cp-subdir-label { font-size: 13px; font-weight: 600; color: var(--text-primary); min-width: 65px; }
  .cp-subdir-path {
    font-size: 11px; font-family: 'SF Mono','Cascadia Code',monospace;
    color: var(--text-muted); flex: 1;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0;
  }
  .cp-subdir-btns { display: flex; gap: 6px; flex-shrink: 0; }

  .btn-action {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 6px 14px; border-radius: 7px; font-size: 12px; font-weight: 600;
    cursor: pointer; transition: all .15s; border: none; white-space: nowrap;
  }
  .btn-switch { background: var(--accent); color: #fff; }
  .btn-switch:hover:not(:disabled) { background: var(--accent-hover); }
  .btn-switch:disabled { opacity: .5; cursor: wait; }
  .btn-start { background: var(--success-bg); color: var(--success-text); }
  .btn-start:hover { background: var(--accent-bg); }
  .btn-start-sm { background: var(--bg-subtle); color: var(--text-secondary); border: 1px solid var(--border); }
  .btn-start-sm:hover { background: var(--bg-card-hover); color: var(--text-primary); }
  .btn-stop { background: var(--error-bg); color: var(--error-text); }
  .btn-stop:hover { background: var(--error-hover-bg); }
  .btn-install { background: var(--bg-subtle); color: var(--text-secondary); border: 1px solid var(--border); }
  .btn-install:hover:not(:disabled) { background: var(--bg-card-hover); }
  .btn-install:disabled { opacity: .5; cursor: not-allowed; }

  .mini-log {
    margin: 4px 0 8px; padding: 8px 12px; background: var(--bg-subtle);
    border-radius: 6px; border: 1px solid var(--border-light);
    max-height: 120px; overflow-y: auto;
  }

  /* ===== 运行服务区 ===== */
  .running-section { margin-bottom: 20px; }
  .running-section-header {
    font-size: 12px; font-weight: 700; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: .05em;
    margin-bottom: 10px; padding-left: 4px;
  }

  .run-card {
    background: var(--bg-subtle); border: 1px solid var(--border-light);
    border-radius: 10px; overflow: hidden; margin-bottom: 10px;
  }
  .run-card-header {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 14px; background: var(--bg-card);
    border-bottom: 1px solid var(--border-light);
  }
  .run-dot {
    width: 7px; height: 7px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 5px rgba(76,175,80,.6);
    flex-shrink: 0;
  }
  .run-name { font-size: 13px; font-weight: 600; color: var(--text-primary); }
  .run-sub { font-size: 11px; color: var(--text-muted); }
  .run-time {
    font-size: 11px; color: var(--text-muted); margin-left: auto;
    font-family: 'SF Mono','Cascadia Code',monospace;
  }
  .run-stop-btn {
    display: flex; align-items: center; justify-content: center;
    width: 22px; height: 22px; border-radius: 5px; border: none;
    background: var(--error-bg); color: var(--error-text);
    cursor: pointer; transition: all .15s; padding: 0;
  }
  .run-stop-btn:hover { background: var(--error-hover-bg); }

  .run-log-body {
    padding: 8px 14px; max-height: 200px; overflow-y: auto;
  }
  .log-line {
    font-family: 'SF Mono','Cascadia Code',monospace;
    font-size: 12px; color: var(--text-secondary); line-height: 1.55;
    white-space: pre-wrap; word-break: break-all;
  }
  .log-stderr { color: var(--error-text); }
  .log-empty {
    font-family: 'SF Mono','Cascadia Code',monospace;
    font-size: 12px; color: var(--text-muted); font-style: italic;
  }
</style>
