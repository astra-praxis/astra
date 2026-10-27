/**
 * 应用布局：可拖拽标题栏 + 侧边栏 + 内容区
 * 移植自 src/routes/+layout.svelte
 */
import { useEffect, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import {
  Bot,
  FolderOpen,
  Blocks,
  GraduationCap,
  Settings,
  Activity,
  Square,
  X,
} from 'lucide-react';
import { useNmStore } from '@/stores/nm';
import { useFavoriteStore } from '@/stores/favorite';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import { useWorkspaceStore } from '@/stores/workspace';
import AccountMenu from '@/components/account/AccountMenu';
import './layout.scss';

const menuItems = [
  { id: 'agent', label: 'Agent', path: '/agent', icon: Bot },
  { id: 'workspace', label: '工作空间', path: '/workspace', icon: FolderOpen },
  { id: 'market-mcp', label: '扩展管理', path: '/market/mcp', icon: Blocks },
  { id: 'market-skills', label: '技能管理', path: '/market/skills', icon: GraduationCap },
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

  // Agent 页：模型切换收进输入框、顶部功能移入标题栏（页面通过 portal 渲染到挂载点）
  const isAgent = location.pathname.startsWith('/agent');
  const pageTitlebarId = isAgent ? 'agent' : null;

  // macOS 用 Overlay 标题栏（原生栏透明融入、红绿灯叠在自定义标题栏上，
  // 自定义栏必须保留：拖拽区 + 红绿灯留白）；Windows/Linux 下 titleBarStyle
  // 不生效、原生标题栏照常显示，自定义栏只会在其上方多出一条重复 header
  // → 隐藏自定义栏，拖拽与窗口控制全部交给原生栏
  const isMac = navigator.userAgent.includes('Macintosh');

  return (
    <div className={`app-layout${sidebarCollapsed ? ' collapsed' : ''}`}>
      {/* 顶部标题栏（可拖拽）：左 chat 工具 ｜ 中 标题 ｜ 右 状态+快捷入口 */}
      {isMac && (
      <header className="title-bar" onMouseDown={onTitlebarMouseDown}>
        {pageTitlebarId ? (
          // 左侧 padding 留出 macOS 红绿灯区域
          <div
            className="titlebar-left agent-titlebar-left"
            id={`${pageTitlebarId}-titlebar-left`}
          />
        ) : (
          // 其他页面：模型切换等已收进 chat/agent 页输入框，标题栏左侧留空
          <div className="titlebar-left" />
        )}

        <div className="titlebar-center">
          <img className="titlebar-logo" src="/logo.png" alt="Astra logo" />
          <span className="title-bar-title">Astra</span>
        </div>

        <div className="titlebar-right">
          {pageTitlebarId && (
            <div className="agent-titlebar-right" id={`${pageTitlebarId}-titlebar-right`} />
          )}
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
      )}

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
              <img className="header-logo" src="/logo.png" alt="Astra logo" />
            </button>
            <h2 className="header-title">Astra</h2>
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

          {/* 底部：用户入口（praxis 式，legacy 模式不渲染） + 设置入口 */}
          <div className="sidebar-footer">
            <div className="sidebar-footer__account">
              <AccountMenu collapsed={sidebarCollapsed} />
            </div>
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
