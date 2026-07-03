<script lang="ts">
  import { page } from '$app/stores';
  import '../styles/global.css';
  import { initTheme } from '$lib/theme.svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { invoke } from '@tauri-apps/api/core';
  let { children } = $props();

  // 标题栏插槽 id，暴露给 chat 页面 portal 使用
  const titlebarSlotId = 'titlebar-slot';

  initTheme();

  const menuItems = [
    { id: 'workspace', label: '工作空间', path: '/', icon: '📁' },
    { id: 'models', label: '模型管理', path: '/models', icon: '🤖' },
    { id: 'providers', label: 'API接口', path: '/providers', icon: '🔌' },
    { id: 'chat', label: '对话', path: '/chat', icon: '💬' },
    { id: 'skills', label: 'Skills管理', path: '/skills', icon: '🧩' }
  ];

  let sidebarOpen = $state(false);
  let sidebarCollapsed = $state(true);

  function toggleMobileMenu() {
    sidebarOpen = !sidebarOpen;
  }

  function closeMobileMenu() {
    sidebarOpen = false;
  }

  function toggleCollapse() {
    sidebarCollapsed = !sidebarCollapsed;
  }

  // ===== 窗口拖拽 =====
  function windowDrag(node: HTMLElement) {
    function onMousedown(e: MouseEvent) {
      if (e.button !== 0) return;
      // 点击交互元素时不触发窗口拖拽
      const target = e.target;
      if (
        target instanceof Element &&
        target.closest('button, select, input, textarea, a, [data-nodrag]')
      ) {
        return;
      }
      invoke('drag_window');
    }
    node.addEventListener('mousedown', onMousedown);
    return {
      destroy() {
        node.removeEventListener('mousedown', onMousedown);
      }
    };
  }

  // ===== 全局快捷键 =====
  onMount(() => {
    function handleGlobalKeydown(e: KeyboardEvent) {
      const cmd = e.metaKey || e.ctrlKey;

      // Cmd+, → 打开设置页
      if (cmd && e.key === ',') {
        e.preventDefault();
        goto('/settings');
      }

      // Cmd+W → 最小化到托盘（隐藏窗口）
      if (cmd && e.key === 'w' && !e.shiftKey) {
        e.preventDefault();
        invoke('minimize_to_tray');
      }
    }

    window.addEventListener('keydown', handleGlobalKeydown);
    return () => window.removeEventListener('keydown', handleGlobalKeydown);
  });
</script>

<div class="app-layout" class:collapsed={sidebarCollapsed}>
  <!-- 顶部标题栏（可拖拽） -->
  <header use:windowDrag class="title-bar">
    <span class="title-bar-title">星野</span>
    <div id="titlebar-slot" class="title-bar-slot"></div>
  </header>

  <div class="app-body">
    <!-- 移动端遮罩 -->
    {#if sidebarOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
      <div class="mobile-overlay" onclick={closeMobileMenu} role="presentation"></div>
    {/if}

    <!-- 汉堡按钮（移动端） -->
    <button class="hamburger" onclick={toggleMobileMenu} aria-label="切换菜单">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        {#if sidebarOpen}
          <line x1="18" y1="6" x2="6" y2="18"/>
          <line x1="6" y1="6" x2="18" y2="18"/>
        {:else}
          <line x1="3" y1="6" x2="21" y2="6"/>
          <line x1="3" y1="12" x2="21" y2="12"/>
          <line x1="3" y1="18" x2="21" y2="18"/>
        {/if}
      </svg>
    </button>

    <!-- 侧边栏 -->
    <aside class="sidebar" class:open={sidebarOpen}>
      <div class="sidebar-header">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <button class="header-logo-btn" onclick={toggleCollapse} title={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'} aria-label={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}>
          <span class="header-logo">星</span>
        </button>
        <h2 class="header-title">星野</h2>
      </div>
      <nav class="sidebar-nav">
        {#each menuItems as item}
          <a
            href={item.path}
            class:active={$page.url.pathname === item.path}
            onclick={closeMobileMenu}
            title={item.label}
          >
            <span class="nav-icon">{item.icon}</span>
            <span class="nav-label">{item.label}</span>
          </a>
        {/each}
      </nav>

      <!-- 底部设置入口 -->
      <div class="sidebar-footer">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <a
          href="/settings"
          class:active={$page.url.pathname === '/settings'}
          onclick={closeMobileMenu}
          title="设置"
        >
          <svg class="settings-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
          </svg>
          <span class="footer-label">设置</span>
        </a>
      </div>
    </aside>

    <!-- 主内容区 -->
    <main class="content">
      {@render children()}
    </main>
  </div>
</div>

<style>
  * {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  .app-layout {
    display: flex;
    flex-direction: column;
    height: 100vh;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
  }

  .app-body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  /* 汉堡按钮（默认隐藏） */
  .hamburger {
    display: none;
    position: fixed;
    top: 12px;
    left: 12px;
    z-index: 1001;
    background: var(--sidebar-bg);
    color: var(--text-primary);
    border: none;
    border-radius: 10px;
    padding: 10px;
    cursor: pointer;
    box-shadow: 0 2px 8px var(--shadow-md);
    transition: all 0.2s;
  }

  .hamburger:hover {
    background: var(--bg-card-hover);
  }

  /* 移动端遮罩 */
  .mobile-overlay {
    display: none;
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0,0,0,0.4);
    z-index: 999;
  }

  /* ===== 顶部标题栏（可拖拽） ===== */
  .title-bar {
    height: 38px;
    background: var(--sidebar-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    user-select: none;
    -webkit-user-select: none;
    border-bottom: 1px solid var(--sidebar-border);
  }

  .title-bar-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--sidebar-text);
    letter-spacing: 0.5px;
    user-select: none;
    -webkit-user-select: none;
    flex-shrink: 0;
  }

  .title-bar-slot {
    display: none;
    flex: 1;
    height: 100%;
    align-items: center;
    min-width: 0;
  }

  /* 有工具栏时隐藏标题，slot 占满整个 title bar */
  .title-bar:has(.title-bar-slot:not(:empty)) {
    justify-content: flex-start;
    padding-left: 80px;
    padding-right: 24px;
  }

  .title-bar:has(.title-bar-slot:not(:empty)) .title-bar-title {
    display: none;
  }

  .title-bar:has(.title-bar-slot:not(:empty)) .title-bar-slot {
    display: flex;
    justify-content: stretch;
  }

  .sidebar {
    width: 220px;
    background: var(--sidebar-bg);
    color: var(--text-primary);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    transition: width 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
  }

  /* 收起状态：VS Code 风格，只保留图标宽度 */
  .collapsed .sidebar {
    width: 48px;
  }

  /* ===== 侧边栏头部 ===== */
  .sidebar-header {
    display: flex;
    align-items: center;
    padding: 8px 12px;
    border-bottom: 1px solid var(--sidebar-border);
    gap: 8px;
    min-height: 48px;
  }

  /* 收起时 header 只显示居中的 logo 按钮 */
  .collapsed .sidebar-header {
    padding: 8px 6px;
    justify-content: center;
  }

  .header-logo-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    border-radius: 8px;
    transition: all 0.2s;
  }

  .header-logo-btn:hover .header-logo {
    background: var(--sidebar-accent);
    filter: brightness(1.1);
  }

  .header-logo {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    background: var(--sidebar-active-bg);
    color: var(--sidebar-active-text);
    font-weight: 700;
    font-size: 15px;
    border-radius: 8px;
    transition: background 0.2s;
  }

  .header-title {
    flex: 1;
    font-size: 18px;
    font-weight: 700;
    color: var(--sidebar-accent);
    letter-spacing: 1px;
    white-space: nowrap;
    overflow: hidden;
  }

  /* 收起时隐藏标题文字 */
  .collapsed .header-title {
    display: none;
  }

  /* ===== 导航 ===== */
  .sidebar-nav {
    flex: 1;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .sidebar-nav a {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    color: var(--sidebar-text);
    text-decoration: none;
    font-size: 14px;
    border-radius: 8px;
    transition: background 0.2s ease, color 0.2s ease;
    white-space: nowrap;
    overflow: hidden;
  }

  .sidebar-nav a:hover {
    background: var(--sidebar-hover-bg);
    color: var(--sidebar-text-hover);
  }

  .sidebar-nav a.active {
    background: var(--sidebar-active-bg);
    color: var(--sidebar-active-text);
    font-weight: 600;
  }

  .nav-icon {
    font-size: 18px;
    flex-shrink: 0;
    width: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .nav-label {
    overflow: hidden;
  }

  /* 收起时隐藏文字标签，图标居中 */
  .collapsed .nav-label {
    display: none;
  }

  .collapsed .sidebar-nav a {
    justify-content: center;
    padding: 10px 0;
  }

  /* ===== 底部 ===== */
  .sidebar-footer {
    padding: 8px;
    border-top: 1px solid var(--sidebar-border);
  }

  .sidebar-footer a {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    color: var(--sidebar-text);
    text-decoration: none;
    font-size: 14px;
    border-radius: 8px;
    transition: background 0.2s ease, color 0.2s ease;
    white-space: nowrap;
    overflow: hidden;
  }

  .sidebar-footer a:hover {
    background: var(--sidebar-hover-bg);
    color: var(--sidebar-text-hover);
  }

  .sidebar-footer a.active {
    background: var(--sidebar-active-bg);
    color: var(--sidebar-active-text);
    font-weight: 600;
  }

  .settings-icon {
    flex-shrink: 0;
  }

  .footer-label {
    overflow: hidden;
  }

  /* 收起时隐藏底部文字 */
  .collapsed .footer-label {
    display: none;
  }

  .collapsed .sidebar-footer a {
    justify-content: center;
    padding: 10px 0;
  }

  /* ===== 主内容区 ===== */
  .content {
    flex: 1;
    overflow-y: auto;
    background: var(--bg-app);
    padding: 32px;
    transition: padding 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  }

  /* ===== 响应式：小屏幕 ===== */
  @media (max-width: 768px) {
    .hamburger {
      display: flex;
      align-items: center;
      justify-content: center;
    }

    .mobile-overlay {
      display: block;
    }

    .sidebar {
      position: fixed;
      top: 0;
      left: 0;
      bottom: 0;
      z-index: 1000;
      width: 220px;
      transform: translateX(-100%);
      transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    }

    .sidebar.open {
      transform: translateX(0);
    }

    /* 移动端始终展开，不受 collapse 影响 */
    .collapsed .sidebar {
      width: 220px;
    }

    .collapsed .header-title,
    .collapsed .nav-label,
    .collapsed .footer-label {
      display: block;
    }

    .collapsed .sidebar-nav a,
    .collapsed .sidebar-footer a {
      justify-content: flex-start;
      padding: 10px 12px;
    }

    .content {
      padding: 20px;
      padding-top: 60px;
    }
  }
</style>
