<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { getThemeMode, setThemeMode, type ThemeMode } from '$lib/theme.svelte';

  // ===== 类型 =====
  interface EditorSetting {
    name: string;
    command: string;
  }

  interface AppSettings {
    editor: EditorSetting;
    workspaces: WorkspaceConfig[];
    active_workspace: string | null;
    scan_depth: number;
  }

  interface WorkspaceConfig {
    name: string;
    path: string;
  }

  // ===== 状态 =====
  let editors = $state<EditorSetting[]>([]);
  let selectedEditor = $state<EditorSetting | null>(null);
  let customCommand = $state('');
  let useCustom = $state(false);
  let saving = $state(false);
  let saved = $state(false);
  let testResult = $state<'idle' | 'success' | 'error'>('idle');
  let testMessage = $state('');

  // 工作空间状态
  let workspaces = $state<WorkspaceConfig[]>([]);
  let activeWorkspace = $state<string | null>(null);

  // 主题状态
  let currentTheme = $state<ThemeMode>(getThemeMode());

  // 扫描深度状态
  let scanDepth = $state(3);
  const themeOptions: { value: ThemeMode; label: string; icon: string }[] = [
    { value: 'light', label: '浅色', icon: '☀️' },
    { value: 'dark', label: '深色', icon: '🌙' },
    { value: 'system', label: '跟随系统', icon: '🖥️' },
  ];

  function changeTheme(theme: ThemeMode) {
    currentTheme = theme;
    setThemeMode(theme);
  }

  // ===== 加载设置 =====
  onMount(async () => {
    try {
      const [settings, presetEditors] = await Promise.all([
        invoke<AppSettings>('get_settings'),
        invoke<EditorSetting[]>('get_preset_editors')
      ]);

      editors = presetEditors;
      workspaces = settings.workspaces;
      activeWorkspace = settings.active_workspace;

      scanDepth = settings.scan_depth || 3;

      // 检查当前设置是否在预设中
      const match = presetEditors.find(e => e.command === settings.editor.command);
      if (match) {
        selectedEditor = match;
        useCustom = false;
      } else {
        // 自定义编辑器
        useCustom = true;
        customCommand = settings.editor.command;
        selectedEditor = settings.editor;
      }
    } catch (e) {
      console.error('加载设置失败:', e);
    }
  });

  // ===== 选择预设编辑器 =====
  function selectEditor(editor: EditorSetting) {
    selectedEditor = editor;
    useCustom = false;
    saved = false;
    testResult = 'idle';
  }

  // ===== 切换自定义 =====
  function enableCustom() {
    useCustom = true;
    customCommand = '';
    selectedEditor = { name: '自定义', command: '' };
    saved = false;
    testResult = 'idle';
  }

  // ===== 保存设置 =====
  async function saveSettings() {
    saving = true;
    saved = false;

    try {
      const editor = useCustom
        ? { name: '自定义', command: customCommand.trim() }
        : selectedEditor!;

      if (!editor.command.trim()) {
        return;
      }

      await invoke('save_settings', {
        settings: {
          editor,
          workspaces,
          active_workspace: activeWorkspace,
          scan_depth: scanDepth
        }
      });

      saved = true;
      setTimeout(() => saved = false, 2000);
    } catch (e) {
      console.error('保存设置失败:', e);
    } finally {
      saving = false;
    }
  }

  // ===== 测试编辑器 =====
  async function testEditor() {
    testResult = 'idle';
    testMessage = '';

    const command = useCustom ? customCommand.trim() : selectedEditor?.command || '';
    if (!command) return;

    try {
      // 打开临时目录测试（当前用户目录）
      await invoke('open_in_editor', {
        path: '~',
        editorCommand: command
      });
      testResult = 'success';
      testMessage = `${command} 启动成功！`;
      setTimeout(() => testResult = 'idle', 3000);
    } catch (e) {
      testResult = 'error';
      testMessage = `${e}`;
    }
  }

  // ===== 表单是否有效 =====
  let isValid = $derived(
    useCustom ? customCommand.trim().length > 0 : selectedEditor !== null
  );

  let currentEditorName = $derived(
    useCustom ? customCommand.trim() || '未设置' : selectedEditor?.name || '未设置'
  );

  // ===== 选择工作空间目录 =====
  async function addWorkspace() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择工作空间目录'
      });

      if (selected) {
        // 提取文件夹名作为默认名称
        const name = selected.split('/').filter(Boolean).pop() || selected;
        const result = await invoke<AppSettings>('add_workspace', { name, path: selected });
        workspaces = result.workspaces;
        activeWorkspace = result.active_workspace ?? null;
        saved = false;
      }
    } catch (e) {
      console.error('添加工作空间失败:', e);
    }
  }

  // ===== 删除工作空间 =====
  async function removeWorkspace(path: string) {
    try {
      const result = await invoke<AppSettings>('remove_workspace', { path });
      workspaces = result.workspaces;
      activeWorkspace = result.active_workspace ?? null;
      saved = false;
    } catch (e) {
      console.error('删除工作空间失败:', e);
    }
  }

  // ===== 设为活跃 =====
  async function setActiveWs(path: string) {
    try {
      const result = await invoke<AppSettings>('set_active_workspace', { path });
      activeWorkspace = result.active_workspace ?? null;
      saved = false;
    } catch (e) {
      console.error('设置活跃工作空间失败:', e);
    }
  }
</script>

<div class="settings-page">
  <div class="page-header">
    <div class="header-left">
      <h1>设置</h1>
      <p class="subtitle">配置你的开发工具偏好</p>
    </div>
  </div>

  <!-- 导航入口卡片 -->
  <div class="settings-card nav-card">
    <div class="card-section-header">
      <div class="section-icon">🧭</div>
      <div class="section-text">
        <h3>高级管理</h3>
        <p>以下功能已从侧边栏移入设置，点击进入对应页面</p>
      </div>
    </div>

    <div class="nav-grid">
      <a href="/models" class="nav-entry">
        <span class="nav-entry-icon">🤖</span>
        <span class="nav-entry-body">
          <span class="nav-entry-title">模型管理</span>
          <span class="nav-entry-desc">管理本地与服务端模型</span>
        </span>
        <span class="nav-entry-arrow">›</span>
      </a>
      <a href="/providers" class="nav-entry">
        <span class="nav-entry-icon">🔌</span>
        <span class="nav-entry-body">
          <span class="nav-entry-title">API 接口</span>
          <span class="nav-entry-desc">配置 AI 服务提供商与密钥</span>
        </span>
        <span class="nav-entry-arrow">›</span>
      </a>
      <a href="/skills" class="nav-entry">
        <span class="nav-entry-icon">🧩</span>
        <span class="nav-entry-body">
          <span class="nav-entry-title">Skills 管理</span>
          <span class="nav-entry-desc">管理可用的技能模块</span>
        </span>
        <span class="nav-entry-arrow">›</span>
      </a>
    </div>
  </div>

  <!-- 主题设置卡片 -->
  <div class="settings-card theme-card">
    <div class="card-section-header">
      <div class="section-icon">🎨</div>
      <div class="section-text">
        <h3>外观主题</h3>
        <p>选择浅色、深色或跟随系统主题</p>
      </div>
    </div>

    <div class="theme-grid">
      {#each themeOptions as opt}
        <button
          class="theme-option"
          class:selected={currentTheme === opt.value}
          onclick={() => changeTheme(opt.value)}
        >
          <span class="theme-icon">{opt.icon}</span>
          <span class="theme-name">{opt.label}</span>
          {#if currentTheme === opt.value}
            <span class="check-mark">✓</span>
          {/if}
        </button>
      {/each}
    </div>
  </div>

  <!-- 扫描深度设置卡片 -->
  <div class="settings-card scan-depth-card">
    <div class="card-section-header">
      <div class="section-icon">🔍</div>
      <div class="section-text">
        <h3>扫描深度</h3>
        <p>点击项目卡片查看详情时，递归遍历子目录的最大层级数</p>
      </div>
    </div>

    <div class="custom-input-group">
      <label for="scan-depth">递归深度</label>
      <input
        id="scan-depth"
        type="number"
        min="1"
        max="5"
        bind:value={scanDepth}
      />
      <p class="input-hint">范围 1–5，数值越大遍历越深（默认 3）</p>
    </div>
  </div>

  <!-- 编辑器设置卡片 -->
  <div class="settings-card">
    <div class="card-section-header">
      <div class="section-icon">⚡</div>
      <div class="section-text">
        <h3>默认编辑器</h3>
        <p>点击工作空间的项目卡片时，使用此编辑器打开项目目录</p>
      </div>
    </div>

    <!-- 当前选择状态 -->
    <div class="current-selection">
      <span class="selection-label">当前选择：</span>
      <span class="selection-value">{currentEditorName}</span>
    </div>

    <!-- 预设编辑器列表 -->
    <div class="editor-grid">
      {#each editors as editor}
        <button
          class="editor-option"
          class:selected={!useCustom && selectedEditor?.command === editor.command}
          onclick={() => selectEditor(editor)}
        >
          <span class="editor-icon">
            {#if editor.name === 'VS Code'}
              <svg width="24" height="24" viewBox="0 0 24 24"><path fill="#007ACC" d="M23.15 2.587L18.21.21a1.49 1.49 0 0 0-1.705.29l-9.46 8.63-4.12-3.128a.999.999 0 0 0-1.144.06L.805 7.347a1 1 0 0 0 .002 1.518L4.368 12 .805 15.136a1 1 0 0 0-.002 1.518l1.004.785a.999.999 0 0 0 1.144.06l4.12-3.128 9.46 8.63a1.49 1.49 0 0 0 1.704.29l4.942-2.377A1.5 1.5 0 0 0 24 19.414V4.586a1.5 1.5 0 0 0-.85-1.999z"/></svg>
            {:else if editor.name === 'Zed'}
              <svg width="24" height="24" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="#084B33"/><text x="12" y="17" text-anchor="middle" fill="white" font-size="14" font-weight="bold" font-family="sans-serif">Z</text></svg>
            {:else if editor.name === 'Cursor'}
              <span class="emoji-icon">🖱️</span>
            {:else if editor.name === 'WebStorm'}
              <svg width="24" height="24" viewBox="0 0 24 24"><circle cx="12" cy="12" r="12" fill="#07C3F2"/><text x="12" y="17" text-anchor="middle" fill="white" font-size="12" font-weight="bold" font-family="sans-serif">WS</text></svg>
            {:else if editor.name === 'IntelliJ IDEA'}
              <svg width="24" height="24" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="#FC3752"/><text x="12" y="17" text-anchor="middle" fill="white" font-size="10" font-weight="bold" font-family="sans-serif">IJ</text></svg>
            {:else if editor.name === 'Sublime Text'}
              <span class="emoji-icon">📝</span>
            {:else if editor.name === 'Atom'}
              <span class="emoji-icon">⚛️</span>
            {:else if editor.name === 'Windsurf'}
              <span class="emoji-icon">🏄</span>
            {:else}
              <span class="emoji-icon">🔧</span>
            {/if}
          </span>
          <span class="editor-name">{editor.name}</span>
          <span class="editor-command">({editor.command})</span>
          {#if !useCustom && selectedEditor?.command === editor.command}
            <span class="check-mark">✓</span>
          {/if}
        </button>
      {/each}

      <!-- 自定义选项 -->
      <button
        class="editor-option custom-option"
        class:selected={useCustom}
        onclick={enableCustom}
      >
        <span class="editor-icon">
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/></svg>
        </span>
        <span class="editor-name">自定义</span>
        <span class="editor-command">(输入命令)</span>
        {#if useCustom}
          <span class="check-mark">✓</span>
        {/if}
      </button>
    </div>

    <!-- 自定义命令输入 -->
    {#if useCustom}
      <div class="custom-input-group">
        <label for="custom-command">命令行工具名称或路径</label>
        <input
          id="custom-command"
          type="text"
          placeholder="如：code、zed、/usr/local/bin/code ..."
          bind:value={customCommand}
          oninput={() => saved = false}
        />
        <p class="input-hint">输入编辑器在终端中的启动命令，或者完整路径</p>
      </div>
    {/if}

    <!-- 测试按钮（仅测试，不保存） -->
    <div class="test-bar">
      <button
        class="btn-test"
        onclick={testEditor}
        disabled={!isValid}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
        测试启动编辑器
      </button>

      {#if testResult !== 'idle'}
        <span class="test-hint" class:test-success={testResult === 'success'} class:test-error={testResult === 'error'}>
          {#if testResult === 'success'}✅{:else}❌{/if}
          {testMessage}
        </span>
      {/if}
    </div>
  </div>

  <!-- ===== 工作空间设置 ===== -->
  <div class="settings-card workspace-card">
    <div class="card-section-header">
      <div class="section-icon">📁</div>
      <div class="section-text">
        <h3>工作空间管理</h3>
        <p>添加多个工作空间目录，在主页可一键切换</p>
      </div>
    </div>

    {#if workspaces.length > 0}
      <div class="ws-list">
        {#each workspaces as ws}
          <div class="ws-item" class:active={ws.path === activeWorkspace}>
            <div class="ws-item-left">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
              <div class="ws-item-info">
                <span class="ws-item-name">{ws.name}</span>
                <span class="ws-item-path">{ws.path}</span>
              </div>
            </div>
            <div class="ws-item-actions">
              {#if ws.path !== activeWorkspace}
                <button class="ws-btn-activate" onclick={() => setActiveWs(ws.path)} title="切换为此工作空间">切换</button>
              {:else}
                <span class="ws-badge-active">当前</span>
              {/if}
              <button class="ws-btn-remove" onclick={() => removeWorkspace(ws.path)} title="移除">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="ws-empty">
        <span>还没有添加任何工作空间</span>
      </div>
    {/if}

    <button class="btn-select-folder" onclick={addWorkspace}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
      添加工作空间
    </button>
  </div>

  <!-- ===== 全局保存栏 ===== -->
  <div class="save-bar">
    <div class="save-bar-info">
      <span class="save-dot"></span>
      <span>修改了编辑器和工作空间配置后，点击保存生效</span>
    </div>
    <button
      class="btn-save"
      onclick={saveSettings}
      disabled={saving}
    >
      {#if saved}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        已保存
      {:else if saving}
        保存中...
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/></svg>
        保存设置
      {/if}
    </button>
  </div>
</div>

<style>
  .settings-page {
    max-width: 780px;
    margin: 0 auto;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .page-header {
    margin-bottom: 28px;
  }

  .page-header h1 {
    font-size: 26px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 6px;
  }

  .subtitle {
    color: var(--text-muted);
    font-size: 14px;
  }

  /* ===== 设置卡片 ===== */
  .settings-card {
    background: var(--bg-card);
    border-radius: 16px;
    padding: 28px;
    box-shadow: 0 1px 3px var(--shadow-sm);
    border: 1px solid var(--border-light);
  }

  .card-section-header {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    margin-bottom: 24px;
    padding-bottom: 20px;
    border-bottom: 1px solid var(--border-light);
  }

  .section-icon {
    font-size: 28px;
    flex-shrink: 0;
  }

  .section-text h3 {
    font-size: 18px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 4px;
  }

  .section-text p {
    font-size: 13px;
    color: var(--text-muted);
    line-height: 1.5;
  }

  /* 当前选择 */
  .current-selection {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
    padding: 10px 14px;
    background: var(--bg-subtle);
    border-radius: 10px;
    border: 1px solid var(--border);
  }

  .selection-label {
    font-size: 13px;
    color: var(--text-muted);
  }

  .selection-value {
    font-size: 14px;
    font-weight: 600;
    color: var(--accent);
  }

  /* ===== 主题选择器 ===== */
  .theme-card {
    margin-bottom: 24px;
  }

  .nav-card {
    margin-bottom: 24px;
  }

  .nav-grid {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .nav-entry {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    background: var(--bg-subtle);
    border: 1px solid var(--border-light);
    border-radius: 12px;
    text-decoration: none;
    color: var(--text-primary);
    transition: background 0.2s ease, border-color 0.2s ease, transform 0.15s ease;
  }

  .nav-entry:hover {
    background: var(--bg-card-hover);
    border-color: var(--accent);
    transform: translateY(-1px);
  }

  .nav-entry-icon {
    font-size: 22px;
    flex-shrink: 0;
  }

  .nav-entry-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .nav-entry-title {
    font-size: 14px;
    font-weight: 600;
  }

  .nav-entry-desc {
    font-size: 12px;
    color: var(--text-muted);
  }

  .nav-entry-arrow {
    flex-shrink: 0;
    font-size: 20px;
    color: var(--text-muted);
    line-height: 1;
  }

  .scan-depth-card {
    margin-bottom: 24px;
  }

  .scan-depth-card .custom-input-group input {
    width: 120px;
  }

  .theme-grid {
    display: flex;
    gap: 10px;
  }

  .theme-option {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 20px 12px;
    background: var(--bg-subtle);
    border: 2px solid var(--border);
    border-radius: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
    position: relative;
  }

  .theme-option:hover {
    border-color: var(--border-strong);
    background: var(--bg-card-hover);
  }

  .theme-option.selected {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .theme-icon {
    font-size: 28px;
  }

  .theme-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  /* ===== 编辑器网格 ===== */
  .editor-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 10px;
    margin-bottom: 20px;
  }

  .editor-option {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 16px 12px;
    background: var(--bg-subtle);
    border: 2px solid var(--border);
    border-radius: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
    position: relative;
  }

  .editor-option:hover {
    border-color: var(--border-strong);
    background: var(--border-light);
  }

  .editor-option.selected {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .editor-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .emoji-icon {
    font-size: 22px;
  }

  .editor-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .editor-command {
    font-size: 11px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
  }

  .check-mark {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 20px;
    height: 20px;
    background: var(--accent);
    color: var(--bg-card);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: bold;
  }

  .custom-option {
    border-style: dashed;
  }

  /* ===== 自定义命令输入 ===== */
  .custom-input-group {
    margin-bottom: 20px;
  }

  .custom-input-group label {
    display: block;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .custom-input-group input {
    width: 100%;
    padding: 10px 14px;
    background: var(--bg-input);
    font-size: 14px;
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: 10px;
    outline: none;
    transition: all 0.2s;
    font-family: ui-monospace, monospace;
  }

  .custom-input-group input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-light);
  }

  .custom-input-group input::placeholder {
    color: var(--text-placeholder);
  }

  .input-hint {
    font-size: 12px;
    color: var(--text-muted);
    margin-top: 6px;
  }

  /* ===== 测试按钮 ===== */
  .test-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .btn-test {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 10px 18px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-test:hover:not(:disabled) {
    background: var(--bg-subtle);
    border-color: var(--border-strong);
  }

  .btn-test:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .test-hint {
    font-size: 13px;
    padding: 6px 12px;
    border-radius: 8px;
    animation: slideDown 0.3s ease;
  }

  .test-success {
    background: var(--success-bg);
    color: var(--success-text);
  }

  .test-error {
    background: var(--error-bg);
    color: var(--error-text);
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ===== 工作空间设置 ===== */
  .workspace-card {
    margin-top: 24px;
  }

  .ws-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: 14px;
    background: var(--bg-subtle);
    border-radius: 10px;
    margin-bottom: 16px;
  }

  .ws-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 16px;
  }

  .ws-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    transition: all 0.2s;
  }

  .ws-item.active {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .ws-item-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }

  .ws-item-left svg {
    flex-shrink: 0;
    color: var(--text-muted);
  }

  .ws-item-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .ws-item-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .ws-item-path {
    font-size: 11px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ws-item-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .ws-btn-activate {
    padding: 4px 10px;
    background: var(--accent-bg);
    border: 1px solid #c7d2fe;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--accent);
    cursor: pointer;
    transition: all 0.2s;
  }

  .ws-btn-activate:hover {
    background: var(--accent-bg-hover);
  }

  .ws-badge-active {
    font-size: 11px;
    font-weight: 600;
    color: var(--success-text);
    background: var(--success-bg);
    padding: 3px 8px;
    border-radius: 6px;
  }

  .ws-btn-remove {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: 6px;
    transition: all 0.2s;
  }

  .ws-btn-remove:hover {
    color: var(--error-text);
    background: var(--error-bg);
  }

  .btn-select-folder {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 12px 20px;
    background: var(--bg-subtle);
    border: 1px dashed var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    width: 100%;
    justify-content: center;
  }

  .btn-select-folder:hover {
    background: var(--bg-card-hover);
    border-color: var(--accent);
    color: var(--accent);
  }

  /* ===== 全局保存栏 ===== */
  .save-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-top: 28px;
    padding: 16px 24px;
    background: var(--bg-card);
    border-radius: 14px;
    box-shadow: 0 1px 3px var(--shadow-sm), 0 4px 12px var(--shadow-md);
    border: 1px solid var(--border);
    position: sticky;
    bottom: 0;
  }

  .save-bar-info {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--text-muted);
  }

  .save-dot {
    width: 8px;
    height: 8px;
    background: var(--accent);
    border-radius: 50%;
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .btn-save {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 12px 28px;
    background: var(--accent-gradient);
    border: none;
    border-radius: 12px;
    font-size: 15px;
    font-weight: 600;
    color: var(--bg-card);
    cursor: pointer;
    transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
    white-space: nowrap;
    box-shadow: 0 2px 8px var(--accent-shadow);
  }

  .btn-save:hover:not(:disabled) {
    transform: translateY(-2px);
    box-shadow: 0 6px 16px var(--accent-shadow-hover);
  }

  .btn-save:active:not(:disabled) {
    transform: translateY(0);
  }

  .btn-save:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
