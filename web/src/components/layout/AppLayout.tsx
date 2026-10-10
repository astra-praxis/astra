/**
 * 应用布局：可拖拽标题栏 + 侧边栏 + 内容区
 * 移植自 src/routes/+layout.svelte
 */
import { useEffect, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import {
  FolderOpen,
  MessageSquare,
  Wrench,
  ScrollText,
  Settings,
  Activity,
  Square,
  X,
} from 'lucide-react';
import { useNmStore } from '@/stores/nm';
import { useFavoriteStore } from '@/stores/favorite';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import { useWorkspaceStore } from '@/stores/workspace';
import ChatToolbar from '@/components/chat/ChatToolbar';
import './layout.scss';

const menuItems = [
  { id: 'workspace', label: '工作空间', path: '/', icon: FolderOpen },
  { id: 'chat', label: '对话', path: '/chat', icon: MessageSquare },
  { id: 'tools', label: '工具箱', path: '/tools', icon: Wrench },
  { id: 'logs', label: '日志', path: '/logs', icon: ScrollText },
];

export default function AppLayout() {
  const navigate = useNavigate();
  const location = useLocation();

  const nmScanning = useNmStore((s) => s.scanning);
  const nmCleaning = useNmStore((s) => s.cleaning);
  const nmProgress = useNmStore((s) => s.progress);
  const stopScan = useNmStore((s) => s.stopScan);

  const favorite = useFavoriteStore((s) => s.favorite);
  const loadFavorite = useFavoriteStore((s) => s.load);

  const loadInitialToolbar = useChatToolbarStore((s) => s.loadInitial);
  const loadFromSettings = useWorkspaceStore((s) => s.loadFromSettings);

  const [sidebarCollapsed, setSidebarCollapsed] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(false);

  const titleBusy = nmScanning || nmCleaning;

  // 初始化：收藏开发模式 + 工具栏数据 + 工作空间
  useEffect(() => {
    loadFavorite();
    loadInitialToolbar();
    loadFromSettings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ===== 全局快捷键 =====
  useEffect(() => {
    function handleGlobalKeydown(e: KeyboardEvent) {
      const cmd = e.metaKey || e.ctrlKey;
      // Cmd+, → 打开设置页
      if (cmd && e.key === ',') {
        e.preventDefault();
        navigate('/settings');
      }
      // Cmd+W → 最小化到托盘（隐藏窗口）
      if (cmd && e.key === 'w' && !e.shiftKey) {
        e.preventDefault();
        invoke('minimize_to_tray');
      }
    }
    window.addEventListener('keydown', handleGlobalKeydown);
    return () => window.removeEventListener('keydown', handleGlobalKeydown);
  }, [navigate]);

  // ===== 窗口拖拽（点击交互元素时不触发） =====
  function onTitlebarMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    const target = e.target as Element;
    if (target.closest('button, select, input, textarea, a, [data-nodrag]')) return;
    invoke('drag_window');
  }

  const isActive = (path: string) =>
    location.pathname === path || location.pathname.startsWith(path + '/');

  return (
    <div className={`app-layout${sidebarCollapsed ? ' collapsed' : ''}`}>
      {/* 顶部标题栏（可拖拽）：左 chat 工具 ｜ 中 标题 ｜ 右 状态+快捷入口 */}
      <header className="title-bar" onMouseDown={onTitlebarMouseDown}>
        <div className="titlebar-left">
          <ChatToolbar side="left" />
        </div>

        <div className="titlebar-center">
          <span className="title-bar-title">星野</span>
        </div>

        <div className="titlebar-right">
          <ChatToolbar side="right" />
          {favorite && (
            <button
              className="titlebar-dev-entry"
              onClick={() => navigate(`/dev-mode/${encodeURIComponent(favorite.path)}?quick=1`)}
              title={`开发模式: ${favorite.name}`}
            >
              <Activity size={14} strokeWidth={1.8} />
              <span className="dev-entry-name">{favorite.name}</span>
            </button>
          )}
          <div className={`titlebar-loading${titleBusy ? ' titlebar-visible' : ''}`}>
            <div className={`titlebar-spinner${titleBusy ? ' spinner-visible' : ''}`} />
            <span className={`titlebar-progress${titleBusy ? ' progress-visible' : ''}`}>
              {nmProgress}
            </span>
            {nmScanning && (
              <button className="titlebar-stop-btn" onClick={stopScan} title="停止">
                <Square size={12} strokeWidth={3} />
              </button>
            )}
          </div>
        </div>
      </header>

      <div className="app-body">
        {/* 移动端遮罩 */}
        {sidebarOpen && (
          <div className="mobile-overlay" onClick={() => setSidebarOpen(false)} />
        )}

        {/* 汉堡按钮（移动端） */}
        <button className="hamburger" onClick={() => setSidebarOpen(!sidebarOpen)} aria-label="切换菜单">
          {sidebarOpen ? (
            <X size={22} strokeWidth={2.5} />
          ) : (
            <MenuIcon />
          )}
        </button>

        {/* 侧边栏 */}
        <aside className={`sidebar${sidebarOpen ? ' open' : ''}`}>
          <div className="sidebar-header">
            <button
              className="header-logo-btn"
              onClick={() => setSidebarCollapsed(!sidebarCollapsed)}
              title={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}
              aria-label={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}
            >
              <span className="header-logo">星</span>
            </button>
            <h2 className="header-title">星野</h2>
          </div>

          <nav className="sidebar-nav">
            {menuItems.map((item) => {
              const Icon = item.icon;
              return (
                <a
                  key={item.id}
                  href={item.path}
                  className={isActive(item.path) ? 'active' : ''}
                  onClick={(e) => {
                    e.preventDefault();
                    navigate(item.path);
                    setSidebarOpen(false);
                  }}
                  title={item.label}
                >
                  <span className="nav-icon">
                    <Icon size={18} strokeWidth={1.8} />
                  </span>
                  <span className="nav-label">{item.label}</span>
                </a>
              );
            })}
          </nav>

          {/* 底部设置入口 */}
          <div className="sidebar-footer">
            <a
              href="/settings"
              className={isActive('/settings') ? 'active' : ''}
              onClick={(e) => {
                e.preventDefault();
                navigate('/settings');
                setSidebarOpen(false);
              }}
              title="设置"
            >
              <Settings size={18} strokeWidth={1.8} className="settings-icon" />
              <span className="footer-label">设置</span>
            </a>
          </div>
        </aside>

        {/* 主内容区 */}
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function MenuIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <line x1="3" y1="6" x2="21" y2="6" />
      <line x1="3" y1="12" x2="21" y2="12" />
      <line x1="3" y1="18" x2="21" y2="18" />
    </svg>
  );
}
