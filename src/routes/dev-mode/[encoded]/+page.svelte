<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { logs } from '$lib/logs-store.svelte';
  import { favoriteStore } from '$lib/favorite-store.svelte';

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
  let failedServers = $state<Record<string, number>>({}); // server_id → 退出码（启动失败）
  let failedInfo = $state<Record<string, DevServerInfo>>({}); // 失败服务器的信息
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

  // 检查是否是从快捷入口进入（无返回按钮）
  let isQuickMode = $derived(new URLSearchParams(window.location.search).get('quick') === '1');
  // 是否已收藏当前项目
  let isFavorited = $derived(favoriteStore.favorite?.path === projectPath);

  // 切换收藏状态
  async function toggleFavorite() {
    if (isFavorited) {
      await favoriteStore.clear();
    } else {
      await favoriteStore.set(projectPath, projectName);
    }
  }

  onMount(() => {
    favoriteStore.load();
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
      // 记录日志
      if (e.payload.command === 'install') {
        if (e.payload.success) {
          logs.info('install', `安装完成 (${e.payload.work_dir})`);
        } else {
          logs.error('install', `安装失败 (退出码 ${e.payload.exit_code}): ${e.payload.work_dir}`);
        }
      }
    });
    unlisteners.push(uncdone);

    const unst = await listen<{ server_id: string; exit_code: number; success: boolean }>('dev-server-stopped', (e) => {
      const info = runningServers[e.payload.server_id];
      const ns = { ...runningServers }; delete ns[e.payload.server_id]; runningServers = ns;
      // 内部记录失败信息（不在页面显示）
      if (!e.payload.success) {
        failedServers = { ...failedServers, [e.payload.server_id]: e.payload.exit_code };
        if (info) failedInfo = { ...failedInfo, [e.payload.server_id]: info };
        // 记录错误日志
        logs.error('dev-server', `${info?.card_name ?? e.payload.server_id} 启动失败 (退出码 ${e.payload.exit_code})`);
      } else {
        delete logsMap[e.payload.server_id];
        logs.info('dev-server', `${info?.card_name ?? e.payload.server_id} 已停止`);
      }
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
    // 清除该卡片之前的失败记录
    for (const sid of Object.keys(failedServers)) {
      const info = failedInfo[sid];
      if (info && info.card_name === card.folder_name && info.subdir === subDir.key) {
        const fs = { ...failedServers }; delete fs[sid]; failedServers = fs;
        const fi = { ...failedInfo }; delete fi[sid]; failedInfo = fi;
        delete logsMap[sid];
      }
    }
    logs.info('dev-server', `启动 ${card.display_name}/${subDir.label}`);
    try {
      const id = await invoke<string>('start_dev_server', {
        workDir: subDir.work_dir, cardName: card.folder_name, subdirKey: subDir.key,
      });
      const info: DevServerInfo = {
        id, card_name: card.folder_name, subdir: subDir.key, work_dir: subDir.work_dir,
        command: 'pnpm dev', started_at: Math.floor(Date.now()/1000), status: 'running', pid: null,
      };
      runningServers = { ...runningServers, [id]: info };
    } catch (e) { error = `启动失败: ${e}`; logs.error('dev-server', `启动失败: ${e}`); }
  }

  async function stopDev(serverId: string) {
    const info = runningServers[serverId];
    try {
      await invoke<boolean>('stop_dev_server', { serverId });
      const ns = { ...runningServers }; delete ns[serverId]; runningServers = ns;
      logs.info('dev-server', `${info?.card_name ?? serverId} 已停止`);
    } catch (e) { error = `停止失败: ${e}`; logs.error('dev-server', `停止失败: ${e}`); }
  }

  async function runInstall(card: DevCardInfo, subDir: DevSubDir) {
    const k = `${card.folder_name}:${subDir.key}`;
    installing = { ...installing, [k]: true };
    cmdLogs[`${subDir.work_dir}:install`] = [];
    logs.info('install', `${card.display_name}/${subDir.label} pnpm install`);
    try {
      await invoke<string>('run_card_command', { workDir: subDir.work_dir, subdirKey: subDir.key, commandName: 'install' });
    } catch (e) { error = `安装失败: ${e}`; logs.error('install', `${card.display_name}/${subDir.label} 安装失败: ${e}`); }
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
  // ===== 开发配置（Cookie / 代理地址） =====
  interface DevConfig {
    cookie: string;
    proxy_target: string;
    port: number | null;
  }

  let showConfig = $state(false);
  let configLoading = $state(false);
  let configSaving = $state(false);
  let configError = $state('');
  let configData = $state<DevConfig | null>(null);
  let cookieInput = $state(''); // 用户粘贴的原始内容
  let cookiePreview = $state(''); // 提取后的 WISCPSID=xxx

  // 从粘贴文本中提取 WISCPSID=xxx
  function extractWiscpsid(input: string): string {
    const m = input.match(/WISCPSID=([^\s;]+)/i);
    if (m) return `WISCPSID=${m[1]}`;
    // 如果只粘贴了 UUID-like 值
    if (/^[a-f0-9-]{8,}$/i.test(input.trim())) return `WISCPSID=${input.trim()}`;
    return input.trim();
  }

  function onCookieInput() {
    cookiePreview = extractWiscpsid(cookieInput);
  }

  // 配置按钮按下的主卡片路径
  let configMasterPath = $state('');
  async function openConfig(masterPath: string) {
    configMasterPath = masterPath;
    showConfig = true;
    configLoading = true;
    configError = '';
    cookieInput = '';
    cookiePreview = '';
    try {
      configData = await invoke<DevConfig>('read_dev_config', { masterPath });
      cookieInput = configData.cookie;
      cookiePreview = configData.cookie;
    } catch (e) {
      configError = `读取配置失败: ${e}`;
    } finally {
      configLoading = false;
    }
  }

  async function saveConfig() {
    if (!cookiePreview) return;
    configSaving = true;
    configError = '';
    try {
      await invoke('save_dev_cookie', { masterPath: configMasterPath, cookie: cookiePreview });
      showConfig = false;
    } catch (e) {
      configError = `保存失败: ${e}`;
    } finally {
      configSaving = false;
    }
  }

  async function openLogin() {
    if (!configData?.proxy_target) return;
    try {
      await invoke('open_login_url', { url: configData.proxy_target });
    } catch (e) {
      configError = `打开浏览器失败: ${e}`;
    }
  }
</script>

<div class="dev-mode-page">
  <!-- 导航栏 -->
  <div class="dev-nav">
    {#if !isQuickMode}
      <button class="back-btn" onclick={goBack}>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
        返回
      </button>
    {/if}
    <div class="dev-nav-info">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
      <span class="dev-nav-title">{projectName}</span>
      <span class="dev-nav-badge">开发模式</span>
    </div>
    <button class="fav-btn" class:fav-active={isFavorited} onclick={toggleFavorite} title={isFavorited ? '取消收藏' : '收藏为快捷入口'}>
      {#if isFavorited}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26"/></svg>
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26"/></svg>
      {/if}
    </button>
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
              <button class="btn-config" onclick={() => openConfig(card.path)} title="配置 Cookie / 代理地址">⚙</button>
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
            <!-- 运行中状态条 -->
            {#each card.sub_dirs as subDir (subDir.key)}
              {#if isRunning(card, subDir)}
                {@const sid = serverIdFor(card, subDir)}
                {#if sid}
                  <div class="running-status-bar">
                    <span class="run-dot-sm"></span>
                    <span class="running-status-label">{subDir.label} 运行中</span>
                    <span class="running-status-time">{formatTime(runningServers[sid]?.started_at ?? 0)}</span>
                    <button class="log-stop-btn" onclick={() => stopDev(sid)} title="停止">
                      <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                    </button>
                  </div>
                {/if}
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
                  <div class="running-status-bar">
                    <span class="run-dot-sm"></span>
                    <span class="running-status-label">{subDir.label} 运行中</span>
                    <span class="running-status-time">{formatTime(runningServers[sid]?.started_at ?? 0)}</span>
                    <button class="log-stop-btn" onclick={() => stopDev(sid)} title="停止">
                      <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                    </button>
                  </div>
                {/if}
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

  <!-- 开发配置弹窗 -->
  {#if showConfig}
    <div class="config-overlay" onclick={() => showConfig = false} role="presentation">
      <div class="config-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="开发配置" tabindex="-1">
        <div class="config-header">
          <h2>🔑 开发配置</h2>
          <button class="config-close" onclick={() => showConfig = false} aria-label="关闭">✕</button>
        </div>

        {#if configLoading}
          <div class="config-body"><div class="spinner"></div><span>正在读取配置...</span></div>
        {:else if configData}
          <div class="config-body">
            {#if configError}
              <div class="config-error">{configError} <button class="error-dismiss" onclick={() => configError = ''}>✕</button></div>
            {/if}

            <!-- 后端代理地址 + 打开登录 -->
            <div class="config-section">
              <label class="config-label">
                🌐 后端代理地址
                {#if configData.port}<span class="config-port">: {configData.port}</span>{/if}
              </label>
              <div class="proxy-row">
                <input class="config-proxy-input" value={configData.proxy_target} readonly />
                <button class="btn-open-login" onclick={openLogin} disabled={!configData.proxy_target} title="在浏览器中打开登录">
                  打开登录
                </button>
              </div>
            </div>

            <!-- Cookie 配置 -->
            <div class="config-section">
              <label class="config-label" for="cookie-input">🍪 登录 Cookie</label>
              <p class="config-hint">点「打开登录」→ 登录后端 → F12 复制 Cookie → 粘贴到下方</p>
              <textarea
                id="cookie-input"
                class="config-textarea"
                bind:value={cookieInput}
                oninput={onCookieInput}
                placeholder="粘贴完整 Cookie（会自动提取 WISCPSID）..."
                rows="3"
              ></textarea>

              {#if cookiePreview && cookiePreview !== cookieInput.trim()}
                <div class="cookie-preview">
                  <span class="cookie-preview-label">✅ 提取结果：</span>
                  <code>{cookiePreview}</code>
                </div>
              {:else if cookiePreview}
                <div class="cookie-preview">
                  <span class="cookie-preview-label">当前 Cookie：</span>
                  <code>{cookiePreview}</code>
                </div>
              {/if}
            </div>
          </div>

          <div class="config-footer">
            <button class="btn-cancel" onclick={() => showConfig = false}>取消</button>
            <button class="btn-save" onclick={saveConfig} disabled={configSaving || !cookiePreview}>
              {configSaving ? '保存中...' : '保存 Cookie'}
            </button>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .dev-mode-page { padding: 16px 20px; height: 100%; display: flex; flex-direction: column; overflow-x: hidden; }

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
  .fav-btn {
    display: flex; align-items: center; justify-content: center;
    width: 32px; height: 32px; border-radius: 7px;
    background: var(--bg-card); border: 1px solid var(--border);
    color: var(--text-muted); cursor: pointer; transition: all .2s;
    margin-left: 8px;
  }
  .fav-btn:hover { background: var(--bg-card-hover); color: var(--warning-text, #f59e0b); }
  .fav-btn.fav-active { color: var(--warning-text, #f59e0b); border-color: var(--warning-text, #f59e0b); }
  .fav-btn.fav-active:hover { color: var(--error-text); border-color: var(--error-text); }
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

  /* ===== 运行状态条（替代日志面板） ===== */
  .running-status-bar {
    display: flex; align-items: center; gap: 6px;
    margin-top: 8px; padding: 5px 10px;
    background: var(--bg-subtle); border: 1px solid var(--border-light);
    border-radius: 7px;
  }
  .running-status-label { font-size: 11px; font-weight: 600; color: var(--text-secondary); }
  .running-status-time {
    font-size: 10px; color: var(--text-muted); margin-left: auto;
    font-family: 'SF Mono','Cascadia Code',monospace;
  }
  .log-stop-btn {
    display: flex; align-items: center; justify-content: center;
    width: 18px; height: 18px; border-radius: 4px; border: none;
    background: var(--error-bg); color: var(--error-text);
    cursor: pointer; transition: all .15s; padding: 0;
  }
  .log-stop-btn:hover { background: var(--error-hover-bg); }

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
    padding: 8px 12px; background: var(--bg-card);
  }
  .run-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--success-text); box-shadow: 0 0 4px rgba(76,175,80,.6); flex-shrink: 0;
  }
  .run-name { font-size: 12px; font-weight: 600; color: var(--text-primary); }
  .run-sub { font-size: 10px; color: var(--text-muted); }
  .run-time { font-size: 10px; color: var(--text-muted); margin-left: auto; font-family: 'SF Mono','Cascadia Code',monospace; }
  .run-time {
    font-size: 10px; color: var(--text-muted); margin-left: auto;
    font-family: 'SF Mono','Cascadia Code',monospace;
  }

  /* ===== 配置按钮 ===== */
  .btn-config {
    width: 20px; height: 20px; border-radius: 5px; border: none;
    background: var(--bg-subtle); color: var(--text-muted);
    font-size: 13px; line-height: 1; cursor: pointer; transition: all .15s;
    display: flex; align-items: center; justify-content: center;
  }
  .btn-config:hover { background: var(--accent-bg); color: var(--accent); }

  /* ===== 配置弹窗 ===== */
  .config-overlay {
    position: fixed; inset: 0; background: rgba(0,0,0,.45);
    display: flex; align-items: center; justify-content: center; z-index: 1000;
  }
  .config-modal {
    width: 480px; max-width: 90vw; max-height: 85vh; overflow-y: auto; overflow-x: hidden;
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 12px; box-shadow: 0 8px 32px rgba(0,0,0,.2);
  }
  .config-header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 16px 20px; border-bottom: 1px solid var(--border);
  }
  .config-header h2 { font-size: 15px; font-weight: 700; color: var(--text-primary); margin: 0; }
  .config-close {
    width: 28px; height: 28px; border-radius: 6px; border: none;
    background: none; color: var(--text-muted); cursor: pointer; font-size: 15px;
  }
  .config-close:hover { background: var(--bg-card-hover); color: var(--text-primary); }

  .config-body { padding: 18px 20px; display: flex; flex-direction: column; gap: 18px; }
  .config-error {
    padding: 8px 12px; background: var(--error-bg); border: 1px solid var(--error-border);
    border-radius: 7px; color: var(--error-text); font-size: 12px;
    display: flex; align-items: center; justify-content: space-between;
  }

  .config-section { display: flex; flex-direction: column; gap: 6px; }
  .config-label {
    font-size: 13px; font-weight: 600; color: var(--text-primary);
    display: flex; align-items: center; gap: 4px;
  }
  .config-port { font-size: 11px; font-weight: 400; color: var(--text-muted); }
  .config-hint { font-size: 11px; color: var(--text-muted); margin: 0; }

  .proxy-row { display: flex; gap: 8px; }
  .config-proxy-input {
    flex: 1; padding: 7px 10px; font-size: 12px; font-family: monospace;
    background: var(--bg-subtle); border: 1px solid var(--border);
    border-radius: 7px; color: var(--text-secondary); outline: none;
    min-width: 0;
  }
  .btn-open-login {
    padding: 7px 14px; font-size: 12px; font-weight: 600; white-space: nowrap;
    background: var(--accent); color: #fff; border: none; border-radius: 7px;
    cursor: pointer; transition: all .15s;
  }
  .btn-open-login:hover:not(:disabled) { background: var(--accent-hover); }
  .btn-open-login:disabled { opacity: .5; cursor: not-allowed; }

  .config-textarea {
    width: 100%; padding: 8px 10px; font-size: 12px; font-family: monospace;
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 7px; color: var(--text-primary); outline: none; resize: vertical;
    transition: border-color .2s; box-sizing: border-box;
  }
  .config-textarea:focus { border-color: var(--accent); }
  .config-textarea::placeholder { color: var(--text-muted); }

  .cookie-preview {
    padding: 8px 10px; background: var(--success-bg); border: 1px solid var(--success-border, var(--success-text));
    border-radius: 7px; font-size: 11px; display: flex; align-items: center; gap: 6px;
    word-break: break-all;
  }
  .cookie-preview-label { color: var(--success-text); font-weight: 600; white-space: nowrap; }
  .cookie-preview code { color: var(--text-primary); font-family: monospace; }

  .config-footer {
    display: flex; justify-content: flex-end; gap: 10px;
    padding: 12px 20px; border-top: 1px solid var(--border);
  }
  .config-footer .btn-cancel {
    padding: 8px 16px; font-size: 13px; font-weight: 500;
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 7px; color: var(--text-secondary); cursor: pointer; transition: all .15s;
  }
  .config-footer .btn-cancel:hover { background: var(--bg-card-hover); }
  .config-footer .btn-save {
    padding: 8px 16px; font-size: 13px; font-weight: 600; white-space: nowrap;
    background: var(--accent); color: #fff; border: none; border-radius: 7px;
    cursor: pointer; transition: all .15s;
  }
  .config-footer .btn-save:hover:not(:disabled) { background: var(--accent-hover); }
  .config-footer .btn-save:disabled { opacity: .5; cursor: not-allowed; }
</style>
