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
    category: string;
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
  let selectedFolder = $state<string | null>(null);

  let runningServers = $state<Record<string, DevServerInfo>>({});
  let activeKeys = $derived(new Set(
    Object.values(runningServers).filter(s => s.status === 'running').map(s => `${s.card_name}:${s.subdir}`)
  ));
  let logsMap = $state<Record<string, { stream: string; line: string }[]>>({});
  let installing = $state<Record<string, boolean>>({});
  let cmdLogs = $state<Record<string, string[]>>({});
  let unlisteners: UnlistenFn[] = [];

  let mainCards = $derived(cards.filter(c => c.category === 'main'));
  let templateCards = $derived(cards.filter(c => c.category === 'template'));
  let cardList = $derived(cards.filter(c => c.category === 'card'));

  let runningList = $derived(
    Object.values(runningServers).filter(s => s.status === 'running').sort((a, b) => a.started_at - b.started_at)
  );
  let runningBase = $derived(runningList.filter(s => {
    const c = cards.find(c => c.folder_name === s.card_name);
    return c && (c.category === 'main' || c.category === 'template');
  }));
  let runningCards = $derived(runningList.filter(s => {
    const c = cards.find(c => c.folder_name === s.card_name);
    return !c || c.category === 'card';
  }));

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

    const unst = await listen<{ server_id: string }>('dev-server-stopped', (e) => {
      const ns = { ...runningServers }; delete ns[e.payload.server_id]; runningServers = ns;
      delete logsMap[e.payload.server_id];
    });
    unlisteners.push(unst);

    try {
      cards = await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath });
      const servers = await invoke<DevServerInfo[]>('list_dev_servers');
      const sMap: Record<string, DevServerInfo> = {};
      for (const s of servers) sMap[s.id] = s;
      runningServers = sMap;
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
    return c?.sub_dirs.find(s => s.key === server.subdir)?.label ?? server.subdir;
  }
  function formatTime(ts: number): string {
    const d = new Date(ts * 1000);
    return `${d.getHours().toString().padStart(2,'0')}:${d.getMinutes().toString().padStart(2,'0')}:${d.getSeconds().toString().padStart(2,'0')}`;
  }
  function serverLogs(id: string) { return logsMap[id] || []; }

  // 停止所有卡片服务
  async function stopAllCards() {
    for (const s of runningCards) {
      await invoke<boolean>('stop_dev_server', { serverId: s.id });
      const ns = { ...runningServers }; delete ns[s.id]; runningServers = ns;
    }
  }

  // 返回：优先返回到项目详情页（通过 ?from= 携带的项目路径）
  function goBack() {
    const params = new URLSearchParams(window.location.search);
    const from = params.get('from');
    if (from) {
      goto(`/?detail=${encodeURIComponent(from)}`);
    } else {
      goto('/');
    }
  }
</script>

<div class="dev-mode-page">
  <!-- 导航栏 -->
  <div class="dev-nav">
    <button class="back-btn" onclick={goBack}>
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

  {#if error}
    <div class="dev-error">{error} <button class="error-dismiss" onclick={() => error = ''}>✕</button></div>
  {/if}

  {#if loading}
    <div class="dev-loading"><div class="spinner"></div><span>正在扫描卡片目录...</span></div>
  {:else}
    <!-- 两栏布局 -->
    <div class="two-col">
      <!-- 左栏：基础服务 -->
      <div class="col-left">
        <div class="col-header">
          <span class="col-title">基础服务</span>
          <span class="col-hint">主站点 + 模板</span>
        </div>

        {#each mainCards as card (card.folder_name)}
          {@const cardRunning = cardHasRunning(card)}
          <div class="base-card" class:base-running={cardRunning}>
            <div class="base-card-head">
              <span class="base-card-name">{card.display_name}</span>
              {#if cardRunning}<span class="run-dot-sm"></span>{/if}
            </div>
            <div class="base-card-folder">{card.folder_name}</div>
            <div class="base-card-actions">
              {#each card.sub_dirs as subDir (subDir.key)}
                {#if subDir.has_package_json}
                  <div class="base-action-row">
                    <span class="base-action-label">{subDir.label}</span>
                    {#if isRunning(card, subDir)}
                      <button class="btn-sm btn-stop" onclick={() => { const id = serverIdFor(card, subDir); if (id) stopDev(id); }}>停止</button>
                    {:else}
                      <button class="btn-sm btn-start" onclick={() => startDev(card, subDir)}>启动</button>
                    {/if}
                    <button class="btn-sm btn-install" onclick={() => runInstall(card, subDir)} disabled={installing[`${card.folder_name}:${subDir.key}`]}>
                      {installing[`${card.folder_name}:${subDir.key}`] ? '...' : 'Install'}
                    </button>
                  </div>
                {/if}
              {/each}
            </div>
            <!-- 运行中日志 -->
            {#each card.sub_dirs as subDir (subDir.key)}
              {#if isRunning(card, subDir)}
                {@const sid = serverIdFor(card, subDir)}
                {#if sid}
                  <div class="base-log">
                    <div class="base-log-head">
                      <span class="run-dot-sm"></span>
                      <span class="base-log-sub">{subDir.label}</span>
                      <span class="base-log-time">{formatTime(runningServers[sid]?.started_at ?? 0)}</span>
                      <button class="log-stop-btn" onclick={() => stopDev(sid)} title="停止">
                        <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                      </button>
                    </div>
                    <div class="base-log-body">
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
                <div class="mini-log">
                  {#each cmdLogs[`${subDir.work_dir}:install`] as line}
                    <div class="log-line">{line}</div>
                  {/each}
                </div>
              {/if}
            {/each}
          </div>
        {/each}

        {#each templateCards as card (card.folder_name)}
          {@const cardRunning = cardHasRunning(card)}
          <div class="base-card" class:base-running={cardRunning}>
            <div class="base-card-head">
              <span class="base-card-name">{card.display_name}</span>
              <span class="base-card-badge">模板</span>
              {#if cardRunning}<span class="run-dot-sm"></span>{/if}
            </div>
            <div class="base-card-folder">{card.folder_name}</div>
            <div class="base-card-actions">
              {#each card.sub_dirs as subDir (subDir.key)}
                {#if subDir.has_package_json}
                  <div class="base-action-row">
                    <span class="base-action-label">{subDir.label}</span>
                    {#if isRunning(card, subDir)}
                      <button class="btn-sm btn-stop" onclick={() => { const id = serverIdFor(card, subDir); if (id) stopDev(id); }}>停止</button>
                    {:else}
                      <button class="btn-sm btn-start" onclick={() => startDev(card, subDir)}>启动</button>
                    {/if}
                    <button class="btn-sm btn-install" onclick={() => runInstall(card, subDir)} disabled={installing[`${card.folder_name}:${subDir.key}`]}>
                      {installing[`${card.folder_name}:${subDir.key}`] ? '...' : 'Install'}
                    </button>
                  </div>
                {/if}
              {/each}
            </div>
            {#each card.sub_dirs as subDir (subDir.key)}
              {#if isRunning(card, subDir)}
                {@const sid = serverIdFor(card, subDir)}
                {#if sid}
                  <div class="base-log">
                    <div class="base-log-head">
                      <span class="run-dot-sm"></span>
                      <span class="base-log-sub">{subDir.label}</span>
                      <span class="base-log-time">{formatTime(runningServers[sid]?.started_at ?? 0)}</span>
                      <button class="log-stop-btn" onclick={() => stopDev(sid)} title="停止">
                        <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                      </button>
                    </div>
                    <div class="base-log-body">
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
            {#each card.sub_dirs as subDir (subDir.key)}
              {#if cmdLogs[`${subDir.work_dir}:install`]?.length > 0}
                <div class="mini-log">
                  {#each cmdLogs[`${subDir.work_dir}:install`] as line}
                    <div class="log-line">{line}</div>
                  {/each}
                </div>
              {/if}
            {/each}
          </div>
        {/each}
      </div>

      <!-- 右栏：卡片 -->
      <div class="col-right">
        <div class="col-header">
          <span class="col-title">卡片</span>
          <span class="col-hint">{cardList.length} 个</span>
          {#if runningCards.length > 0}
            <button class="btn-sm btn-stop-all" onclick={stopAllCards}>全部停止</button>
          {/if}
        </div>

        <!-- 卡片标签 -->
        <div class="card-tags">
          {#each cardList as card (card.folder_name)}
            {@const cardRunning = cardHasRunning(card)}
            <button class="card-tag" class:tag-active={selectedFolder === card.folder_name} class:tag-running={cardRunning} onclick={() => selectedFolder = card.folder_name}>
              {#if cardRunning}<span class="tag-dot"></span>{/if}
              {card.display_name}
            </button>
          {/each}
        </div>

        <!-- 选中卡片操作 -->
        {#if selectedCard}
          <div class="card-panel">
            <div class="card-panel-head">
              <span class="card-panel-name">{selectedCard.display_name}</span>
              <span class="card-panel-folder">{selectedCard.folder_name}</span>
            </div>
            <div class="card-panel-actions">
              {#each selectedCard.sub_dirs as subDir (subDir.key)}
                {#if subDir.has_package_json}
                  <div class="card-action-row">
                    <span class="card-action-label">{subDir.label}</span>
                    <span class="card-action-path" title={subDir.work_dir}>{subDir.work_dir.split('/').slice(-3).join('/')}</span>
                    <div class="card-action-btns">
                      {#if isRunning(selectedCard, subDir)}
                        <button class="btn-sm btn-stop" onclick={() => { const id = serverIdFor(selectedCard, subDir); if (id) stopDev(id); }}>停止</button>
                      {:else}
                        <button class="btn-sm btn-switch" onclick={() => quickSwitch(selectedCard, subDir)} disabled={switching}>
                          {switching ? '⏳' : '⇄'} 切换
                        </button>
                        <button class="btn-sm btn-start-sm" onclick={() => startDev(selectedCard, subDir)}>启动</button>
                      {/if}
                      <button class="btn-sm btn-install" onclick={() => runInstall(selectedCard, subDir)} disabled={installing[`${selectedCard.folder_name}:${subDir.key}`]}>
                        {installing[`${selectedCard.folder_name}:${subDir.key}`] ? '...' : 'Install'}
                      </button>
                    </div>
                  </div>
                {/if}
              {/each}
            </div>
            <!-- 安装日志 -->
            {#each selectedCard.sub_dirs as subDir (subDir.key)}
              {#if cmdLogs[`${subDir.work_dir}:install`]?.length > 0}
                <div class="mini-log">
                  {#each cmdLogs[`${subDir.work_dir}:install`] as line}
                    <div class="log-line">{line}</div>
                  {/each}
                </div>
              {/if}
            {/each}
          </div>
        {/if}

        <!-- 运行中的卡片服务 -->
        {#if runningCards.length > 0}
          <div class="running-cards">
            {#each runningCards as srv (srv.id)}
              {@const sc = serverCard(srv)}
              <div class="run-card">
                <div class="run-card-head">
                  <span class="run-dot"></span>
                  <span class="run-name">{sc?.display_name ?? srv.card_name}</span>
                  <span class="run-sub">{serverSubDirLabel(srv)}</span>
                  <span class="run-time">{formatTime(srv.started_at)}</span>
                  <button class="log-stop-btn" onclick={() => stopDev(srv.id)} title="停止">
                    <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                  </button>
                </div>
                <div class="run-card-body">
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
      </div>
    </div>

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
  .dev-mode-page { padding: 16px 20px; height: 100%; display: flex; flex-direction: column; }

  /* ===== 导航栏 ===== */
  .dev-nav {
    display: flex; align-items: center; gap: 10px;
    margin-bottom: 16px; padding-bottom: 12px;
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .back-btn {
    display: inline-flex; align-items: center; gap: 5px;
    padding: 6px 10px; background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 7px; font-size: 12px; color: var(--text-secondary);
    cursor: pointer; transition: all .2s; white-space: nowrap;
  }
  .back-btn:hover { background: var(--bg-card-hover); color: var(--text-primary); }
  .dev-nav-info { display: flex; align-items: center; gap: 6px; color: var(--accent); }
  .dev-nav-title { font-size: 15px; font-weight: 700; color: var(--text-primary); }
  .dev-nav-badge {
    font-size: 10px; padding: 2px 7px; border-radius: 10px;
    background: var(--accent-bg); color: var(--accent);
    border: 1px solid var(--accent-ring); font-weight: 600;
  }
  .refresh-btn {
    margin-left: auto; display: flex; align-items: center; justify-content: center;
    width: 32px; height: 32px; border-radius: 7px;
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
    justify-content: center; gap: 12px; padding: 60px 0; color: var(--text-secondary);
  }
  .spinner {
    width: 24px; height: 24px;
    border: 3px solid var(--border); border-top-color: var(--accent);
    border-radius: 50%; animation: spin .8s linear infinite;
  }
  .empty-state svg { opacity: .35; }
  .empty-state h3 { margin: 0; font-size: 16px; color: var(--text-primary); }
  .empty-state p { margin: 0; font-size: 13px; }

  .dev-error {
    padding: 10px 14px; background: var(--error-bg); border: 1px solid var(--error-border);
    border-radius: 8px; color: var(--error-text); font-size: 12px;
    display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px;
  }
  .error-dismiss { background: none; border: none; color: var(--error-text); cursor: pointer; font-size: 13px; }

  /* ===== 两栏布局 ===== */
  .two-col {
    display: grid; grid-template-columns: 280px 1fr; gap: 16px;
    flex: 1; min-height: 0; overflow: hidden;
  }
  .col-left, .col-right {
    display: flex; flex-direction: column; gap: 10px;
    overflow-y: auto; padding-right: 4px;
  }

  .col-header {
    display: flex; align-items: center; gap: 8px; flex-shrink: 0;
  }
  .col-title { font-size: 13px; font-weight: 700; color: var(--text-primary); }
  .col-hint { font-size: 11px; color: var(--text-muted); }

  /* ===== 左栏：基础卡片 ===== */
  .base-card {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 10px; padding: 14px; flex-shrink: 0;
  }
  .base-running { border-color: var(--success-text); box-shadow: 0 0 0 1px var(--success-text); }
  .base-card-head { display: flex; align-items: center; gap: 6px; margin-bottom: 2px; }
  .base-card-name { font-size: 14px; font-weight: 700; color: var(--text-primary); }
  .base-card-badge {
    font-size: 10px; padding: 1px 6px; border-radius: 4px;
    background: var(--success-bg); color: var(--success-text); font-weight: 600;
  }
  .base-card-folder {
    font-size: 10px; font-family: 'SF Mono','Cascadia Code',monospace;
    color: var(--text-muted); margin-bottom: 10px;
  }
  .base-card-actions { display: flex; flex-direction: column; gap: 6px; }
  .base-action-row { display: flex; align-items: center; gap: 6px; }
  .base-action-label { font-size: 12px; font-weight: 600; color: var(--text-primary); min-width: 54px; }
  .run-dot-sm {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 4px rgba(76,175,80,.7); flex-shrink: 0;
  }

  /* ===== 通用按钮 ===== */
  .btn-sm {
    display: inline-flex; align-items: center; justify-content: center;
    gap: 3px; padding: 4px 10px; border-radius: 6px; font-size: 11px;
    font-weight: 600; cursor: pointer; transition: all .15s; border: none; white-space: nowrap;
  }
  .btn-start { background: var(--success-bg); color: var(--success-text); }
  .btn-start:hover { background: var(--accent-bg); }
  .btn-switch { background: var(--accent); color: #fff; }
  .btn-switch:hover:not(:disabled) { background: var(--accent-hover); }
  .btn-switch:disabled { opacity: .5; cursor: wait; }
  .btn-start-sm { background: var(--bg-subtle); color: var(--text-secondary); border: 1px solid var(--border); }
  .btn-start-sm:hover { background: var(--bg-card-hover); color: var(--text-primary); }
  .btn-stop { background: var(--error-bg); color: var(--error-text); }
  .btn-stop:hover { background: var(--error-hover-bg); }
  .btn-install { background: var(--bg-subtle); color: var(--text-muted); border: 1px solid var(--border); }
  .btn-install:hover:not(:disabled) { background: var(--bg-card-hover); }
  .btn-install:disabled { opacity: .5; cursor: not-allowed; }
  .btn-stop-all {
    margin-left: auto; padding: 3px 8px; border-radius: 5px;
    background: var(--error-bg); color: var(--error-text);
    border: none; font-size: 10px; font-weight: 600; cursor: pointer;
  }
  .btn-stop-all:hover { background: var(--error-hover-bg); }

  /* ===== 日志 ===== */
  .base-log {
    margin-top: 8px; border: 1px solid var(--border-light);
    border-radius: 7px; overflow: hidden; background: var(--bg-subtle);
  }
  .base-log-head {
    display: flex; align-items: center; gap: 5px;
    padding: 5px 10px; background: var(--bg-card);
    border-bottom: 1px solid var(--border-light);
  }
  .base-log-sub { font-size: 11px; font-weight: 600; color: var(--text-secondary); }
  .base-log-time { font-size: 10px; color: var(--text-muted); margin-left: auto; font-family: 'SF Mono','Cascadia Code',monospace; }
  .log-stop-btn {
    display: flex; align-items: center; justify-content: center;
    width: 18px; height: 18px; border-radius: 4px; border: none;
    background: var(--error-bg); color: var(--error-text);
    cursor: pointer; transition: all .15s; padding: 0;
  }
  .log-stop-btn:hover { background: var(--error-hover-bg); }
  .base-log-body { padding: 6px 10px; max-height: 150px; overflow-y: auto; }
  .mini-log {
    margin-top: 6px; padding: 6px 10px; background: var(--bg-subtle);
    border: 1px solid var(--border-light); border-radius: 7px;
    max-height: 100px; overflow-y: auto;
  }
  .log-line {
    font-family: 'SF Mono','Cascadia Code',monospace;
    font-size: 11px; color: var(--text-secondary); line-height: 1.5;
    white-space: pre-wrap; word-break: break-all;
  }
  .log-stderr { color: var(--error-text); }
  .log-empty {
    font-family: 'SF Mono','Cascadia Code',monospace;
    font-size: 11px; color: var(--text-muted); font-style: italic;
  }

  /* ===== 右栏：卡片标签 ===== */
  .card-tags { display: flex; flex-wrap: wrap; gap: 6px; flex-shrink: 0; }
  .card-tag {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 5px 11px; border-radius: 6px; font-size: 12px;
    font-weight: 500; background: var(--bg-card); border: 1px solid var(--border);
    color: var(--text-secondary); cursor: pointer; transition: all .15s;
  }
  .card-tag:hover { background: var(--bg-card-hover); border-color: var(--border-strong); color: var(--text-primary); }
  .tag-active { background: var(--accent-bg); border-color: var(--accent); color: var(--accent); font-weight: 600; }
  .tag-running { border-color: var(--success-text); }
  .tag-running.tag-active { border-color: var(--accent); }
  .tag-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 4px rgba(76,175,80,.7); flex-shrink: 0;
  }

  /* ===== 选中卡片面板 ===== */
  .card-panel {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 10px; padding: 14px; flex-shrink: 0;
  }
  .card-panel-head { display: flex; align-items: baseline; gap: 8px; margin-bottom: 10px; }
  .card-panel-name { font-size: 14px; font-weight: 700; color: var(--text-primary); }
  .card-panel-folder {
    font-size: 10px; font-family: 'SF Mono','Cascadia Code',monospace;
    color: var(--text-muted);
  }
  .card-panel-actions { display: flex; flex-direction: column; gap: 7px; }
  .card-action-row { display: flex; align-items: center; gap: 8px; }
  .card-action-label { font-size: 12px; font-weight: 600; color: var(--text-primary); min-width: 54px; }
  .card-action-path {
    font-size: 10px; font-family: 'SF Mono','Cascadia Code',monospace;
    color: var(--text-muted); flex: 1;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0;
  }
  .card-action-btns { display: flex; gap: 5px; flex-shrink: 0; }

  /* ===== 运行中的卡片服务 ===== */
  .running-cards { display: flex; flex-direction: column; gap: 8px; }
  .run-card {
    background: var(--bg-subtle); border: 1px solid var(--border-light);
    border-radius: 9px; overflow: hidden;
  }
  .run-card-head {
    display: flex; align-items: center; gap: 6px;
    padding: 6px 12px; background: var(--bg-card);
    border-bottom: 1px solid var(--border-light);
  }
  .run-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 4px rgba(76,175,80,.6); flex-shrink: 0;
  }
  .run-name { font-size: 12px; font-weight: 600; color: var(--text-primary); }
  .run-sub { font-size: 10px; color: var(--text-muted); }
  .run-time {
    font-size: 10px; color: var(--text-muted); margin-left: auto;
    font-family: 'SF Mono','Cascadia Code',monospace;
  }
  .run-card-body { padding: 6px 12px; max-height: 180px; overflow-y: auto; }
</style>
