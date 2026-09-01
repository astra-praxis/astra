<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type {
    AppSettings,
    EditorSetting,
    ProjectCard,
    ProjectDetail,
    WorkspaceConfig,
  } from '$lib/home/types';
  import ProjectCardView from '$lib/home/ProjectCard.svelte';
  import ProjectDetailView from '$lib/home/ProjectDetail.svelte';
  import CreateProjectModal from '$lib/home/CreateProjectModal.svelte';

  // ===== 状态 =====
  let workspacePath = $state('');
  let projects = $state<ProjectCard[]>([]);
  let loading = $state(false);
  let error = $state('');
  let searchQuery = $state('');
  let editorSetting = $state<EditorSetting>({ name: '', command: '' });
  let workspaces = $state<WorkspaceConfig[]>([]);

  // ===== 页面加载时自动读取设置并扫描 =====
  onMount(async () => {
    await loadAndScan();
    // 如果带有 ?detail= 参数（如从开发模式页面返回），自动打开项目详情
    const searchParams = new URLSearchParams(window.location.search);
    const detailPath = searchParams.get('detail');
    if (detailPath) {
      showDetail({ path: detailPath } as ProjectCard);
      // 清除查询参数，避免刷新时重复打开
      window.history.replaceState(null, '', window.location.pathname);
    }
  });

  // ===== 加载设置并扫描 =====
  async function loadAndScan() {
    try {
      const settings = await invoke<AppSettings>('get_settings');
      editorSetting = settings.editor;
      workspaces = settings.workspaces;

      if (settings.active_workspace) {
        workspacePath = settings.active_workspace;
        scanWorkspace(settings.active_workspace);
      }
    } catch (e) {
      console.error('加载设置失败:', e);
    }
  }

  // ===== 切换工作空间 =====
  async function switchWorkspace(path: string) {
    try {
      await invoke('set_active_workspace', { path });
      workspacePath = path;
      scanWorkspace(path);
    } catch (e) {
      error = `切换工作空间失败: ${e}`;
    }
  }

  // ===== 扫描工作空间 =====
  async function scanWorkspace(path: string) {
    loading = true;
    error = '';
    searchQuery = '';

    try {
      const result = await invoke<ProjectCard[]>('scan_workspace', { path });
      projects = result;
    } catch (e) {
      error = `扫描失败: ${e}`;
      projects = [];
    } finally {
      loading = false;
    }
  }

  // ===== 点击卡片打开编辑器 =====
  async function openProject(path: string) {
    if (!editorSetting.command) {
      error = '请先在「设置」页面配置默认编辑器';
      return;
    }
    try {
      await invoke('open_in_editor', { path, editorCommand: editorSetting.command });
    } catch (e) {
      error = `打开编辑器失败: ${e}`;
    }
  }

  // ===== 计算属性 =====
  let filteredProjects = $derived.by(() => {
    let list = projects;

    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      list = list.filter(p =>
        p.name.toLowerCase().includes(q) ||
        p.path.toLowerCase().includes(q)
      );
    }

    return list;
  });

  // ===== 自动聚焦搜索框 =====
  function focusOnMount(node: HTMLInputElement) {
    node.focus();
  }

  // ===== 全局快捷键 =====
  function handleKeydown(e: KeyboardEvent) {
    // Cmd+K / Ctrl+K → 聚焦搜索
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      const searchInput = document.querySelector('.hero-search-input');
      if (searchInput) (searchInput as HTMLInputElement).focus();
    }
  }

  // ===== 新建项目弹窗 =====
  let showCreateModal = $state(false);

  // ===== 详情视图状态 =====
  let selectedProject = $state<ProjectDetail | null>(null);
  let detailLoading = $state(false);

  function openCreateModal() {
    showCreateModal = true;
  }

  function closeCreateModal() {
    showCreateModal = false;
    error = '';
  }

  function handleProjectCreated(newProject: ProjectCard) {
    projects = [...projects, newProject];
    projects.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    showCreateModal = false;
    error = '';
  }

  // ===== 项目详情 =====
  async function showDetail(project: ProjectCard) {
    // 滚动到顶部
    document.querySelector('.content')?.scrollTo(0, 0);
    detailLoading = true;
    error = '';
    try {
      const detail = await invoke<ProjectDetail>('get_project_detail', { path: project.path });
      selectedProject = detail;
    } catch (e) {
      error = `加载详情失败: ${e}`;
    } finally {
      detailLoading = false;
    }
  }

  function backToWorkspace() {
    selectedProject = null;
    document.querySelector('.content')?.scrollTo(0, 0);
  }

  function openEditorForPath(path: string) {
    if (!editorSetting.command) {
      error = '请先在「设置」页面配置默认编辑器';
      return;
    }
    invoke('open_in_editor', { path, editorCommand: editorSetting.command }).catch(e => {
      error = `打开编辑器失败: ${e}`;
    });
  }
</script>

<div class="workspace-page" onkeydown={handleKeydown} role="presentation" tabindex="-1">
  <!-- 未设置工作空间 -->
  {#if !workspacePath && !loading}
    <div class="no-workspace">
      <div class="no-workspace-icon">📂</div>
      <h3>尚未设置工作空间</h3>
      <p>前往设置页面选择一个目录作为工作空间</p>
      <a href="/settings" class="btn-primary">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
        前往设置
      </a>
    </div>
  {/if}

  <!-- 错误提示 -->
  {#if error}
    <div class="error-banner">
      <div class="error-icon">⚠️</div>
      <span>{error}</span>
      <button class="error-dismiss" onclick={() => error = ''}>✕</button>
    </div>
  {/if}

  <!-- 加载状态 -->
  {#if loading}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner-ring"></div>
        <div class="loading-text">
          <span class="loading-title">正在扫描工作空间</span>
          <span class="loading-desc">遍历目录并检查 README.md...</span>
        </div>
      </div>
    </div>
  {/if}

  <!-- 工作空间内容区 -->
  {#if selectedProject || detailLoading}
    <ProjectDetailView
      project={selectedProject ?? ({ name: '', path: '', has_readme: false, readme_preview: '', sub_items: [] } as ProjectDetail)}
      loading={detailLoading}
      editor={editorSetting}
      onBack={backToWorkspace}
      onOpenEditor={openEditorForPath}
    />
  {:else if !loading && workspacePath}
    <!-- 居中搜索区 -->
    <div class="hero-search">
      <div class="hero-search-inner">
        <div class="hero-search-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg></div>
        <input type="text" class="hero-search-input" placeholder="搜索项目名称..." bind:value={searchQuery} use:focusOnMount />
        {#if searchQuery}<button class="hero-search-clear" onclick={() => searchQuery = ''}>✕</button>{/if}
      </div>
      <div class="hero-meta">
        <!-- 工作空间选择器 -->
        <div class="ws-selector">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
          <select class="ws-select" onchange={(e) => switchWorkspace((e.target as HTMLSelectElement).value)}>
            {#each workspaces as ws}
              <option value={ws.path} selected={ws.path === workspacePath}>{ws.name}</option>
            {/each}
          </select>
        </div>
        <span class="hero-dot">·</span>
        <span class="hero-count">{projects.length} 个项目</span>
        <span class="hero-dot">·</span>
        <a href="/settings" class="hero-link">管理工作空间</a>
        <span class="hero-dot">·</span>
        <button class="hero-create" onclick={openCreateModal}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>新建项目</button>
      </div>
    </div>
    <!-- 搜索无结果 -->
    {#if filteredProjects.length === 0 && projects.length > 0}
      <div class="no-results">
        <span class="no-results-icon">🔍</span>
        <h3>未找到匹配的项目</h3>
        <p>尝试使用不同的关键词搜索，或 <button class="link-btn" onclick={() => searchQuery = ''}>清除搜索</button></p>
      </div>
    {/if}
    <!-- 项目卡片网格 -->
    {#if filteredProjects.length > 0}
      <div class="card-grid">
        {#each filteredProjects as project (project.path)}
          <ProjectCardView
            {project}
            searchQuery={searchQuery}
            editorName={editorSetting.name}
            onOpen={showDetail}
            onOpenProject={openProject}
          />
        {/each}
      </div>
    {/if}
    <!-- 空工作空间 -->
    {#if projects.length === 0}
      <div class="empty-state">
        <div class="empty-illustration"><span class="empty-icon">📭</span></div>
        <h3>这里还没有项目</h3>
        <p>该目录下没有找到任何子文件夹</p>
      </div>
    {/if}
  {/if}

  <!-- ===== 新建项目弹窗 ===== -->
  {#if showCreateModal}
    <CreateProjectModal
      workspacePath={workspacePath}
      onClose={closeCreateModal}
      onCreated={handleProjectCreated}
      onError={(msg) => (error = msg)}
    />
  {/if}
</div>

<style>
  .workspace-page {
    margin: 0 auto;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ========== 未设置工作空间 ========== */
  .no-workspace {
    text-align: center;
    padding: 80px 20px;
    background: var(--bg-card);
    border-radius: 16px;
    border: 2px dashed var(--border);
    margin-top: 20px;
  }

  .no-workspace-icon {
    font-size: 56px;
    margin-bottom: 16px;
    display: block;
  }

  .no-workspace h3 {
    font-size: 20px;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .no-workspace p {
    color: var(--text-muted);
    font-size: 15px;
    margin-bottom: 24px;
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 12px 24px;
    background: var(--accent-gradient);
    border: none;
    border-radius: 12px;
    font-size: 15px;
    font-weight: 600;
    color: white;
    text-decoration: none;
    cursor: pointer;
    transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .btn-primary:hover {
    transform: translateY(-2px);
    box-shadow: 0 8px 20px var(--accent-shadow-hover);
  }

  /* ========== 错误 ========== */
  .error-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 18px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 12px;
    color: var(--error-text);
    margin-bottom: 20px;
    font-size: 14px;
    animation: slideDown 0.3s ease;
  }

  .error-icon {
    font-size: 18px;
    flex-shrink: 0;
  }

  .error-dismiss {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--error-muted);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 16px;
  }

  .error-dismiss:hover {
    background: var(--error-hover-bg);
    color: var(--error-text);
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-12px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ========== 加载 ========== */
  .loading-container {
    display: flex;
    justify-content: center;
    padding: 60px 0;
  }

  .loading-card {
    display: flex;
    align-items: center;
    gap: 20px;
    background: var(--bg-card);
    padding: 32px 40px;
    border-radius: 16px;
    box-shadow: 0 2px 8px var(--shadow-md);
  }

  .spinner-ring {
    width: 36px;
    height: 36px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .loading-title {
    font-weight: 600;
    color: var(--text-primary);
    font-size: 16px;
  }

  .loading-desc {
    color: var(--text-muted);
    font-size: 13px;
  }

  /* ========== 居中搜索区 ========== */
  .hero-search {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 60px 20px 40px;
    animation: fadeIn 0.4s ease;
  }

  .hero-search-inner {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    max-width: 560px;
    padding: 16px 24px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 16px;
    box-shadow: 0 2px 8px var(--shadow-sm), 0 8px 24px var(--shadow-sm);
    transition: all 0.3s ease;
  }

  .hero-search-inner:focus-within {
    border-color: var(--accent);
    box-shadow: 0 2px 8px var(--accent-light), 0 8px 32px var(--accent-shadow);
  }

  .hero-search-icon {
    color: var(--text-muted);
    flex-shrink: 0;
    display: flex;
  }

  .hero-search-input {
    flex: 1;
    border: none;
    outline: none;
    font-size: 20px;
    font-weight: 500;
    color: var(--text-primary);
    background: none;
    min-width: 0;
  }

  .hero-search-input::placeholder {
    color: var(--text-placeholder);
    font-weight: 400;
  }

  .hero-search-clear {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 8px;
    font-size: 18px;
    flex-shrink: 0;
    transition: all 0.2s;
  }

  .hero-search-clear:hover {
    color: var(--text-secondary);
    background: var(--border-light);
  }

  .hero-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 16px;
    font-size: 14px;
    color: var(--text-muted);
  }

  .hero-count {
    font-weight: 500;
  }

  .hero-dot {
    color: var(--border);
  }

  .hero-create {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: none;
    border: none;
    font-size: 14px;
    font-weight: 500;
    color: var(--accent);
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: all 0.2s;
  }

  .hero-create:hover {
    background: var(--accent-bg);
    color: var(--accent-hover);
  }

  .hero-link {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
    text-decoration: none;
    transition: color 0.2s;
  }

  .hero-link:hover {
    color: var(--accent);
  }

  /* 工作空间选择器 */
  .ws-selector {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .ws-selector svg {
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .ws-select {
    appearance: none;
    -webkit-appearance: none;
    background: transparent;
    border: none;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    cursor: pointer;
    padding: 4px 6px;
    border-radius: 6px;
    outline: none;
    transition: all 0.2s;
  }

  .ws-select option {
    background: var(--bg-card);
    color: var(--text-primary);
  }

  .ws-select:hover {
    background: var(--border-light);
  }

  .ws-select:focus {
    background: var(--accent-bg);
  }

  /* ========== 搜索无结果 ========== */
  .no-results {
    text-align: center;
    padding: 40px 20px;
    background: var(--bg-card);
    border-radius: 14px;
    border: 1px solid var(--border-light);
    margin: 8px auto;
    max-width: 1100px;
  }

  .no-results-icon {
    font-size: 40px;
    display: block;
    margin-bottom: 12px;
  }

  .no-results h3 {
    font-size: 16px;
    color: var(--text-secondary);
    margin-bottom: 6px;
  }

  .no-results p {
    color: var(--text-muted);
    font-size: 14px;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 14px;
    font-weight: 500;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .link-btn:hover {
    color: var(--accent-hover);
  }

  /* ========== 卡片网格 ========== */
  .card-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 16px;
    padding: 12px 32px 32px;
    max-width: 1100px;
    margin: 0 auto;
    animation: fadeIn 0.4s ease;
  }

  /* ========== 空状态 ========== */
  .empty-state {
    text-align: center;
    padding: 64px 20px;
    background: var(--bg-card);
    border-radius: 16px;
    border: 1px solid var(--border-light);
  }

  .empty-illustration {
    width: 80px;
    height: 80px;
    background: var(--border-light);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    margin: 0 auto 16px;
  }

  .empty-icon {
    font-size: 36px;
  }

  .empty-state h3 {
    font-size: 18px;
    color: var(--text-secondary);
    margin-bottom: 6px;
  }

  .empty-state p {
    color: var(--text-muted);
    font-size: 14px;
  }
</style>
