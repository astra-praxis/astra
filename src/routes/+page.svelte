<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import {
    type NodeModulesInfo,
    nm,
    nmCache,
    startScan,
    cleanSingle,
    cleanAll,
  } from '$lib/nm-store.svelte';

  // ===== 类型 =====
  interface SubProject {
    name: string;
    path: string;
  }

  interface ProjectCard {
    name: string;
    path: string;
    has_readme: boolean;
    sub_projects: SubProject[];
  }

  interface EditorSetting {
    name: string;
    command: string;
  }

  interface AppSettings {
    editor: EditorSetting;
    workspaces: WorkspaceConfig[];
    active_workspace: string | null;
  }

  interface WorkspaceConfig {
    name: string;
    path: string;
  }

  interface NodeModulesInfo {
    path: string;
    project_path: string;
    size_bytes: number;
    size_display: string;
    project_name: string;
    has_pnpm_lock: boolean;
  }

  interface CleanResult {
    success: boolean;
    cleaned_paths: string[];
    failed_paths: [string, string][];
    total_freed_bytes: number;
    total_freed_display: string;
  }

  // ===== 状态 =====
  let workspacePath = $state('');
  let projects = $state<ProjectCard[]>([]);
  let loading = $state(false);
  let error = $state('');
  let searchQuery = $state('');
  let editorSetting = $state<EditorSetting>({ name: '', command: '' });
  let workspaces = $state<WorkspaceConfig[]>([]);

  // ===== node_modules 管理状态（使用全局 store） =====
  let nodeModulesList = $state<NodeModulesInfo[]>([]);
  let nodeModulesScannedPath = $state<string | null>(null);
  let nodeModulesError = $state('');
  let deletingPath = $state<string | null>(null);

  // 按项目名分组统计
  let nmProjectCount = $derived(new Set(nodeModulesList.map(r => r.project_name)).size);

  // ===== 页面加载时自动读取设置并扫描 =====
  onMount(async () => {
    await loadAndScan();
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

  // ===== 高亮搜索匹配 =====
  function highlight(text: string): string {
    if (!searchQuery.trim()) return text;
    const q = searchQuery.trim();
    const escaped = q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const regex = new RegExp(`(${escaped})`, 'gi');
    return text.replace(regex, '<mark>$1</mark>');
  }

  // ===== 文件夹名称提取 =====
  function folderName(path: string): string {
    const parts = path.replace(/\\/g, '/').split('/');
    return parts[parts.length - 1];
  }

  // ===== 获取文件夹图标颜色 =====
  function folderColor(name: string): string {
    const colors = [
      '#4fc3f7', '#ff7043', '#66bb6a', '#ab47bc',
      '#ffa726', '#26c6da', '#ec407a', '#7e57c2',
      '#8d6e63', '#78909c', '#29b6f6', '#f06292',
    ];
    let hash = 0;
    for (let i = 0; i < name.length; i++) {
      hash = name.charCodeAt(i) + ((hash << 5) - hash);
    }
    return colors[Math.abs(hash) % colors.length];
  }

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
    // Escape → 返回工作空间 / 关闭弹窗
    if (e.key === 'Escape') {
      if (selectedProject) {
        e.preventDefault();
        backToWorkspace();
      } else if (showCreateModal) {
        e.preventDefault();
        closeCreateModal();
      }
    }
  }

  // ===== 新建项目弹窗状态 =====
  let showCreateModal = $state(false);
  let newFolderName = $state('');
  let newProjectName = $state('');
  let creating = $state(false);

  // ===== 详情视图状态 =====
  let selectedProject = $state<ProjectDetail | null>(null);
  let detailLoading = $state(false);

  function openCreateModal() {
    newFolderName = '';
    newProjectName = '';
    showCreateModal = true;
  }

  function closeCreateModal() {
    if (creating) return;
    showCreateModal = false;
  }

  async function createNewProject() {
    const folder = newFolderName.trim();
    if (!folder) return;
    creating = true;
    error = '';
    try {
      const newProject = await invoke<ProjectCard>('create_project', {
        workspacePath: workspacePath,
        folderName: folder,
        projectName: newProjectName.trim()
      });
      projects = [...projects, newProject];
      projects.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
      showCreateModal = false;
    } catch (e) {
      error = `创建失败: ${e}`;
    } finally {
      creating = false;
    }
  }

  // ===== 项目详情 =====
  interface GitRepo {
    name: string;
    path: string;
    remote_url: string | null;
  }

  interface SubDetail {
    name: string;
    path: string;
    git_repo: GitRepo | null;
    children: SubDetail[];
    readme_preview: string;
    depth: number;
  }

  interface ProjectDetail {
    name: string;
    path: string;
    has_readme: boolean;
    readme_preview: string;
    sub_items: SubDetail[];
  }

  async function showDetail(project: ProjectCard) {
    // 滚动到顶部
    document.querySelector('.content')?.scrollTo(0, 0);
    detailLoading = true;
    error = '';
    nodeModulesError = '';
    // 从缓存读取扫描结果
    const cached = nmCache.get(project.path);
    try {
      const detail = await invoke<ProjectDetail>('get_project_detail', { path: project.path });
      selectedProject = detail;
      if (cached) {
        // 有缓存：直接展示上次的扫描结果
        nodeModulesList = cached;
        nodeModulesScannedPath = project.path;
      } else {
        // 无缓存：清空，等待用户手动扫描
        nodeModulesList = [];
        nodeModulesScannedPath = null;
      }
    } catch (e) {
      error = `加载详情失败: ${e}`;
    } finally {
      detailLoading = false;
    }
  }

  function backToWorkspace() {
    selectedProject = null;
    deletingPath = null;
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

  // ===== node_modules 扫描（当前项目） =====
  async function scanNodeModules() {
    if (!selectedProject || nm.scanning) return;
    nodeModulesList = [];
    nodeModulesError = '';
    const result = await startScan(selectedProject.path);
    nodeModulesList = result || [];
    nodeModulesScannedPath = selectedProject.path;
    if (nm.error) {
      nodeModulesError = nm.error;
    }
  }

  let totalNodeModulesSize = $derived(
    nodeModulesList.reduce((sum, r) => sum + r.size_bytes, 0)
  );

  function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + units[i];
  }

  // ===== 单项删除 =====
  function confirmDeleteNodeModule(path: string) {
    deletingPath = path;
  }

  function cancelDeleteNodeModule() {
    deletingPath = null;
  }

  async function doDeleteNodeModule() {
    if (!deletingPath) return;
    const path = deletingPath;
    const cleaned = await cleanSingle(path);
    nodeModulesList = nodeModulesList.filter(r => !cleaned.includes(r.path));
    // 更新缓存
    if (selectedProject) {
      nmCache.set(selectedProject.path, nodeModulesList);
    }
    if (nm.error) {
      nodeModulesError = nm.error;
    }
    deletingPath = null;
  }

  // ===== 一键清理全部 =====
  async function cleanAllNodeModules() {
    if (nodeModulesList.length === 0 || nm.cleaning) return;
    const cleaned = await cleanAll(nodeModulesList.map(r => r.path));
    if (cleaned) {
      nodeModulesList = nodeModulesList.filter(r => !cleaned.includes(r.path));
      // 更新缓存
      if (selectedProject) {
        nmCache.set(selectedProject.path, nodeModulesList);
      }
    }
    if (nm.error) {
      nodeModulesError = nm.error;
    }
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
  {#if selectedProject}
    <!-- 项目详情视图 -->
    <div class="detail-view">
      <div class="detail-nav">
        <button class="back-btn" onclick={backToWorkspace}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
          返回
        </button>
      </div>
      {#if detailLoading}
        <div class="detail-loading">
          <div class="spinner"></div>
          <span>加载详情中...</span>
        </div>
      {:else}
        <div class="detail-header">
          <div class="detail-header-left">
            <div class="detail-avatar" style="background: {folderColor(selectedProject.name)}22; color: {folderColor(selectedProject.name)}">{selectedProject.name.charAt(0).toUpperCase()}</div>
            <div>
              <h2 class="detail-title">{selectedProject.name}</h2>
              <div class="detail-path" title={selectedProject.path}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                <span>{selectedProject.path}</span>
              </div>
            </div>
          </div>
          <button class="editor-open-btn" onclick={function() { if (selectedProject) openEditorForPath(selectedProject.path); }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
            在 {editorSetting.name || '编辑器'} 中打开
          </button>
        </div>
        {#if selectedProject.readme_preview}
          <div class="detail-readme">
            <div class="section-title"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg><span>README</span></div>
            <pre class="readme-content">{selectedProject.readme_preview}</pre>
          </div>
        {/if}

        <!-- ===== node_modules 管理 ===== -->
        <div class="detail-nm">
          <div class="nm-section-header">
            <div class="section-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
              <span>node_modules 清理</span>
            </div>
            <button class="nm-scan-btn" onclick={scanNodeModules} disabled={nm.scanning}>
              {#if nm.scanning}
                <div class="btn-spinner"></div>
                扫描中...
              {:else}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9"/><polyline points="21 3 21 9 15 9"/></svg>
                {nodeModulesScannedPath === selectedProject?.path ? '重新扫描' : '扫描 node_modules'}
              {/if}
            </button>
          </div>

          {#if nm.scanning && nm.progress}
            <div class="nm-scan-progress">
              <div class="nm-scan-progress-spinner"></div>
              <span class="nm-scan-progress-text" title={nm.progress}>{nm.progress}</span>
            </div>
          {/if}

          {#if nodeModulesScannedPath === selectedProject?.path && !nm.scanning}
            {#if nodeModulesError}
              <div class="nm-error">{nodeModulesError}</div>
            {/if}

            {#if nodeModulesList.length === 0}
              <div class="nm-empty-inline">
                <span>📦</span>
                <span>此项目中未找到 node_modules 文件夹</span>
              </div>
            {:else}
              <div class="nm-summary-inline">
                <span class="nm-stat"><strong>{nmProjectCount}</strong> 个项目下共 <strong>{nodeModulesList.length}</strong> 个 node_modules</span>
                <span class="nm-stat">总计 <strong>{formatBytes(totalNodeModulesSize)}</strong></span>
                <button class="nm-clean-btn" onclick={cleanAllNodeModules} disabled={nm.cleaning}>
                  {#if nm.cleaning}
                    <div class="btn-spinner"></div>
                    清理中...
                  {:else}
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                    一键清理全部
                  {/if}
                </button>
              </div>

              <div class="nm-list-inline">
                {#each nodeModulesList as nm (nm.path)}
                  <div class="nm-item-inline">
                    <div class="nm-item-inline-content">
                      <div class="nm-item-inline-path" title={nm.path}>{nm.path}</div>
                      <div class="nm-item-inline-right">
                        <button
                          class="nm-item-inline-open-btn"
                          onclick={() => openEditorForPath(nm.project_path)}
                          title={`在 ${editorSetting.name || '编辑器'} 中打开`}
                          aria-label="在编辑器中打开"
                        >
                          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                        </button>
                        <span class="nm-item-inline-size">{nm.size_display}</span>
                      </div>
                    </div>
                    <button class="nm-item-delete-btn" onclick={() => confirmDeleteNodeModule(nm.path)} title="删除此 node_modules">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
        </div>

        <!-- 删除确认弹窗 -->
        {#if deletingPath}
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
          <div class="modal-overlay" onclick={cancelDeleteNodeModule} role="presentation">
            <div class="nm-confirm-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="确认删除" tabindex="-1">
              <div class="modal-header">
                <h2>确认删除</h2>
              </div>
              <div class="nm-confirm-body">
                <p>确定要删除以下 node_modules 吗？</p>
                <p class="nm-confirm-path">{deletingPath.replace(selectedProject?.path || '', '.')}</p>
              </div>
              <div class="modal-footer">
                <button class="btn-cancel" onclick={cancelDeleteNodeModule} disabled={nm.cleaning}>取消</button>
                <button class="nm-confirm-delete-btn" onclick={doDeleteNodeModule} disabled={nm.cleaning}>
                  {#if nm.cleaning}
                    <div class="btn-spinner"></div>
                    删除中...
                  {:else}
                    确认删除
                  {/if}
                </button>
              </div>
            </div>
          </div>
        {/if}

        {#snippet renderSubItem(item: SubDetail)}
          <div class="sub-detail-card" style="margin-left: {Math.min(item.depth, 5) * 8}px">
            <div class="sub-detail-header">
              <span class="sub-detail-icon">📁</span>
              <span class="sub-detail-name">{item.name}</span>
              <button class="sub-open-btn" onclick={() => openEditorForPath(item.path)}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                打开
              </button>
            </div>
            {#if item.readme_preview}
              <div class="sub-readme">
                <div class="sub-readme-header">📖 README</div>
                <pre class="sub-readme-content">{item.readme_preview}</pre>
              </div>
            {/if}
            {#if item.git_repo}
              <div class="git-info">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" y1="9" x2="6" y2="21"/></svg>
                <a class="git-url" href={item.git_repo.remote_url || '#'} target="_blank" rel="noreferrer">{item.git_repo.remote_url}</a>
              </div>
            {/if}
            {#if item.children && item.children.length > 0}
              {#each item.children as child}
                {@render renderSubItem(child)}
              {/each}
            {/if}
          </div>
        {/snippet}

        {#if selectedProject.sub_items.length > 0}
          <div class="detail-subs">
            <div class="section-title"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg><span>子项目 ({selectedProject.sub_items.length})</span></div>
            <div class="sub-detail-list">
              {#each selectedProject.sub_items as item}
                {@render renderSubItem(item)}
              {/each}
            </div>
          </div>
        {:else}
          <div class="no-subs"><span class="no-subs-icon">📭</span><p>该项目下没有子目录</p></div>
        {/if}
      {/if}
    </div>
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
          <div class="project-card" style="--card-accent: {folderColor(project.name)}" onclick={() => showDetail(project)} onkeydown={(e) => e.key === 'Enter' && showDetail(project)} role="button" tabindex="0" title="查看项目详情">
            <div class="card-accent-bar"></div>
            <div class="card-content">
              <div class="card-header">
                <div class="card-avatar" style="background: {folderColor(project.name)}22; color: {folderColor(project.name)}">{project.name.charAt(0).toUpperCase()}</div>
                <div class="card-header-text">
                  <h3 class="card-title">{@html highlight(project.name)}</h3>
                  <span class="card-folder">{@html highlight(folderName(project.path))}</span>
                </div>
              </div>
              <div class="card-path" title={project.path}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg><span>{project.path}</span></div>
              {#if project.sub_projects?.length > 0}
                <div class="sub-projects">
                  <div class="sub-label"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg><span>子项目 ({project.sub_projects.length})</span></div>
                  <div class="sub-list">
                    {#each project.sub_projects as sub}
                      <button class="sub-item" onclick={(e) => { e.stopPropagation(); openProject(sub.path); }} title="在 {editorSetting.name || '编辑器'} 中打开 {sub.name}"><span class="sub-item-name">{sub.name}</span></button>
                    {/each}
                  </div>
                </div>
              {/if}
              <div class="card-footer">
                <div class="footer-right">
                  <button class="open-editor-btn" onclick={(e) => { e.stopPropagation(); openProject(project.path); }} title="在 {editorSetting.name || '编辑器'} 中打开"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>打开</button>
                </div>
              </div>
            </div>
          </div>
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
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
      <div class="modal-overlay" onclick={closeCreateModal} role="presentation">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="新建项目" tabindex="-1">
          <div class="modal-header">
            <h2>新建项目</h2>
            <button class="modal-close" onclick={closeCreateModal} disabled={creating} aria-label="关闭">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </div>
          <div class="modal-body">
            <div class="form-group">
              <label for="folder-name">文件夹名称 <span class="required">*</span></label>
              <input
                id="folder-name"
                type="text"
                placeholder="例如：my-project"
                bind:value={newFolderName}
                disabled={creating}
                onkeydown={(e) => e.key === 'Enter' && createNewProject()}
              />
              <p class="form-hint">将在当前工作空间下创建此名称的子文件夹</p>
            </div>
            <div class="form-group">
              <label for="project-name">项目名称 <span class="required">*</span></label>
              <input
                id="project-name"
                type="text"
                placeholder="输入项目显示名称"
                bind:value={newProjectName}
                disabled={creating}
                onkeydown={(e) => e.key === 'Enter' && createNewProject()}
              />
              <p class="form-hint">项目 README.md 的标题</p>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-cancel" onclick={closeCreateModal} disabled={creating}>取消</button>
            <button
              class="btn-confirm"
              onclick={createNewProject}
              disabled={!newFolderName.trim() || !newProjectName.trim() || creating}
            >
              {#if creating}
                <div class="btn-spinner"></div>
                创建中...
              {:else}
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 14.66V20a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5.34"/><polygon points="18 2 22 6 12 16 8 16 8 12 18 2"/></svg>
                创建项目
              {/if}
            </button>
          </div>
        </div>
      </div>
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

  .project-card {
    background: var(--bg-card);
    border-radius: 14px;
    overflow: hidden;
    box-shadow: 0 1px 3px var(--shadow-sm);
    border: 1px solid var(--border-light);
    transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    display: flex;
    flex-direction: column;
    cursor: pointer;
    outline: none;
  }

  .project-card:hover {
    box-shadow: 0 8px 24px var(--shadow-hover);
    transform: translateY(-3px);
    border-color: transparent;
  }

  .project-card:focus-visible {
    box-shadow: 0 0 0 3px var(--accent-ring);
    transform: translateY(-2px);
  }

  .card-accent-bar {
    height: 4px;
    background: var(--card-accent, #667eea);
    flex-shrink: 0;
  }

  .card-content {
    padding: 20px;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 14px;
  }

  .card-avatar {
    width: 42px;
    height: 42px;
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    font-weight: 700;
    flex-shrink: 0;
  }

  .card-header-text {
    min-width: 0;
    flex: 1;
  }

  .card-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-title :global(mark) {
    background: var(--highlight-bg);
    color: var(--highlight-text);
    border-radius: 3px;
    padding: 0 2px;
  }

  .card-folder {
    font-size: 12px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
  }

  .card-folder :global(mark) {
    background: var(--highlight-bg);
    color: var(--highlight-text);
    border-radius: 3px;
    padding: 0 2px;
  }

  .card-path {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-muted);
    padding: 8px 10px;
    background: var(--bg-subtle);
    border-radius: 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--border-light);
  }

  .card-path svg {
    flex-shrink: 0;
  }

  .card-path span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ===== 子项目 ===== */
  .sub-projects {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sub-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .sub-label svg {
    flex-shrink: 0;
  }

  .sub-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .sub-item {
    display: inline-flex;
    align-items: center;
    padding: 4px 10px;
    background: var(--accent-bg);
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--link);
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: ui-monospace, monospace;
  }

  .sub-item:hover {
    background: var(--accent-bg-hover);
    border-color: var(--link);
    color: var(--link-hover);
  }

  .sub-item:active {
    transform: scale(0.97);
  }

  .card-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    margin-top: auto;
  }

  .footer-right {
    display: flex;
    align-items: center;
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

  /* ========== 卡片打开按钮 ========== */
  .open-editor-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
    opacity: 0;
  }

  .project-card:hover .open-editor-btn {
    opacity: 1;
  }

  .open-editor-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  /* ========== 项目详情视图 ========== */
  .detail-view {
    animation: fadeIn 0.3s ease;
  }

  .detail-nav {
    margin-bottom: 20px;
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .back-btn:hover {
    background: var(--bg-subtle);
    border-color: var(--text-placeholder);
  }

  .detail-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 60px;
    color: var(--text-secondary);
  }

  .detail-loading .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .detail-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 24px;
    padding: 20px 24px;
    background: var(--bg-card);
    border-radius: 14px;
    border: 1px solid var(--border-light);
    flex-wrap: wrap;
  }

  .detail-header-left {
    display: flex;
    align-items: center;
    gap: 16px;
    min-width: 0;
    flex: 1;
  }

  .detail-avatar {
    width: 48px;
    height: 48px;
    border-radius: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 22px;
    font-weight: 700;
    flex-shrink: 0;
  }

  .detail-title {
    font-size: 22px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 4px;
  }

  .detail-path {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
    max-width: 500px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail-path svg {
    flex-shrink: 0;
  }

  .detail-path span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .editor-open-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 20px;
    background: var(--accent-gradient);
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
    box-shadow: 0 2px 6px var(--accent-shadow);
  }

  .editor-open-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px var(--accent-shadow-hover);
  }

  /* 区域标题 */
  .section-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 14px;
  }

  .section-title svg {
    color: var(--text-muted);
    flex-shrink: 0;
  }

  /* README 预览 */
  .detail-readme {
    margin-bottom: 24px;
    padding: 20px;
    background: var(--bg-card);
    border-radius: 14px;
    border: 1px solid var(--border-light);
  }

  .readme-content {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.6;
    white-space: pre-wrap;
    font-family: ui-monospace, monospace;
    overflow-x: auto;
  }

  /* 子项目列表 */
  .detail-subs {
    margin-bottom: 24px;
  }

  .sub-detail-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .sub-detail-card {
    padding: 18px 20px;
    background: var(--bg-card);
    border-radius: 14px;
    border: 1px solid var(--border-light);
    border-left: 4px solid var(--border);
    transition: all 0.2s ease;
  }

  .sub-detail-card:hover {
    box-shadow: 0 2px 8px var(--shadow-sm);
  }

  .sub-detail-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 10px;
  }

  .sub-detail-icon {
    font-size: 20px;
    flex-shrink: 0;
  }

  .sub-detail-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: ui-monospace, monospace;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }



  .git-info {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
    padding: 8px 12px;
    background: var(--bg-subtle);
    border-radius: 8px;
    border: 1px solid var(--border-light);
    margin-bottom: 8px;
  }

  .git-info svg {
    flex-shrink: 0;
    color: var(--text-muted);
  color: var(--text-secondary);
}

  .git-url {
    color: var(--link);
    text-decoration: none;
    font-family: ui-monospace, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .git-url:hover {
    text-decoration: underline;
    color: var(--link-hover);
  }

  .sub-open-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .sub-open-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  /* 子项目 README */
  .sub-readme {
    margin: 8px 0;
    padding: 10px 12px;
    background: var(--bg-subtle);
    border-radius: 8px;
    border: 1px solid var(--border-light);
  }

  .sub-readme-header {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }

  .sub-readme-content {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.5;
    white-space: pre-wrap;
    font-family: ui-monospace, monospace;
    margin: 0;
  }

  .no-subs {
    text-align: center;
    padding: 60px 20px;
    background: var(--bg-card);
    border-radius: 14px;
    border: 1px solid var(--border-light);
  }

  .no-subs-icon {
    font-size: 40px;
    display: block;
    margin-bottom: 8px;
  }

  .no-subs p {
    color: var(--text-muted);
    font-size: 14px;
  }

  /* ========== 弹窗遮罩 ========== */
  .modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fadeInOverlay 0.2s ease;
  }

  @keyframes fadeInOverlay {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .modal {
    background: var(--bg-card);
    border-radius: 20px;
    padding: 0;
    width: 480px;
    max-width: 90vw;
    box-shadow: 0 24px 48px rgba(0, 0, 0, 0.4);
    border: 1px solid var(--border);
    animation: slideUp 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  }

  @keyframes slideUp {
    from { opacity: 0; transform: translateY(16px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 24px 0;
  }

  .modal-header h2 {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .modal-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 6px;
    border-radius: 8px;
    transition: all 0.2s;
  }

  .modal-close:hover:not(:disabled) {
    background: var(--border-light);
    color: var(--text-secondary);
  }

  .modal-close:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .modal-body {
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-group label {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .required {
    color: var(--error-text);
  }

  .form-group input {
    padding: 10px 14px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    color: var(--text-primary);
    outline: none;
    transition: all 0.2s;
  }

  .form-group input::placeholder {
    color: var(--text-placeholder);
  }

  .form-group input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-light);
  }

  .form-group input:disabled {
    background: var(--bg-subtle);
    cursor: not-allowed;
  }

  .form-hint {
    font-size: 12px;
    color: var(--text-muted);
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 24px 20px;
  }

  .btn-cancel {
    padding: 10px 20px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-cancel:hover:not(:disabled) {
    background: var(--border-light);
  }

  .btn-cancel:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-confirm {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 22px;
    background: var(--accent-gradient);
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 2px 6px var(--accent-shadow);
  }

  .btn-confirm:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px var(--accent-shadow-hover);
  }

  .btn-confirm:disabled {
    opacity: 0.6;
    cursor: not-allowed;
    box-shadow: none;
    transform: none;
  }

  .btn-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ========== node_modules 详情页内联面板 ========== */

  .detail-nm {
    margin-top: 20px;
    margin-bottom: 24px;
    background: var(--bg-card);
    border-radius: 12px;
    border: 1px solid var(--border);
    padding: 18px 22px;
  }

  .nm-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  .nm-scan-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    background: var(--accent-gradient);
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .nm-scan-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 2px 8px var(--accent-shadow);
  }

  .nm-scan-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .nm-scan-progress {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 10px 0;
    padding: 8px 14px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    min-width: 0;
  }

  .nm-scan-progress-spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  .nm-scan-progress-text {
    font-size: 12px;
    color: var(--text-secondary);
    font-family: var(--font-mono, 'SF Mono', Menlo, monospace);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .nm-error {
    background: rgba(255,59,48,0.1);
    color: var(--error-text);
    padding: 10px 14px;
    border-radius: 8px;
    font-size: 13px;
    margin-bottom: 12px;
  }

  .nm-empty-inline {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 0;
    color: var(--text-secondary);
    font-size: 14px;
  }

  .nm-summary-inline {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 16px;
    padding: 8px 0 12px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 8px;
  }

  .nm-stat {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .nm-stat strong {
    color: var(--text-primary);
  }

  .nm-clean-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    background: linear-gradient(135deg, #ff5252, #ff1744);
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
    margin-left: auto;
  }

  .nm-clean-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(255,23,68,0.3);
  }

  .nm-clean-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .nm-list-inline {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .nm-item-inline {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 11px 12px;
    border-radius: 8px;
    transition: background 0.15s;
  }

  .nm-item-inline:hover {
    background: var(--bg-input);
  }

  .nm-item-inline-content {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-width: 0;
  }

  .nm-item-inline-right {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    margin-left: 12px;
  }

  .nm-item-inline-path {
    font-size: 13px;
    color: var(--text-secondary);
    font-family: var(--font-mono, 'SF Mono', Menlo, monospace);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .nm-item-inline-open-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    flex-shrink: 0;
    opacity: 0;
    transition: background 0.15s, color 0.15s, opacity 0.15s;
  }

  .nm-item-inline:hover .nm-item-inline-open-btn {
    opacity: 1;
  }

  .nm-item-inline-open-btn:hover {
    background: var(--accent);
    color: white;
  }

  .nm-item-inline-size {
    font-size: 13px;
    color: var(--text-secondary);
    font-weight: 600;
    flex-shrink: 0;
    margin-left: 12px;
  }

  .nm-item-delete-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s;
    flex-shrink: 0;
    opacity: 0;
  }

  .nm-item-inline:hover .nm-item-delete-btn {
    opacity: 1;
  }

  .nm-item-delete-btn:hover {
    color: #ff1744;
    border-color: #ff1744;
    background: rgba(255,23,68,0.08);
  }

  /* ===== 删除确认弹窗 ===== */
  .nm-confirm-modal {
    background: var(--bg-card);
    border-radius: 16px;
    width: 90%;
    max-width: 440px;
    box-shadow: 0 12px 48px rgba(0,0,0,0.25);
    border: 1px solid var(--border);
    overflow: hidden;
  }

  .nm-confirm-body {
    padding: 8px 20px 16px;
    font-size: 14px;
    color: var(--text-secondary);
  }

  .nm-confirm-body p {
    margin: 4px 0;
  }

  .nm-confirm-path {
    font-family: monospace;
    font-size: 12px;
    color: var(--text-primary);
    background: var(--bg-input);
    padding: 8px 12px;
    border-radius: 8px;
    word-break: break-all;
    margin-top: 8px !important;
  }

  .nm-confirm-delete-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 22px;
    background: linear-gradient(135deg, #ff5252, #ff1744);
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .nm-confirm-delete-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(255,23,68,0.3);
  }

  .nm-confirm-delete-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
