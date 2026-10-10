import { useEffect, useMemo, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { useWorkspaceStore } from '@/stores/workspace';
import type { EditorSetting } from '@/stores/workspace';
import ProjectCardView from '@/features/home/ProjectCard';
import ProjectDetailView from '@/features/home/ProjectDetail';
import CreateProjectModal from '@/features/home/CreateProjectModal';
import type { ProjectCard, ProjectDetail as ProjectDetailData } from '@/features/home/types';
import { Folder, Plus, Search } from 'lucide-react';
import './WorkspacePage.scss';

const EMPTY_DETAIL: ProjectDetailData = {
  name: '',
  path: '',
  has_readme: false,
  readme_preview: '',
  sub_items: [],
};

export default function WorkspacePage() {
  const navigate = useNavigate();
  const location = useLocation();

  const projects = useWorkspaceStore((s) => s.projects);
  const workspaces = useWorkspaceStore((s) => s.workspaces);
  const activePath = useWorkspaceStore((s) => s.activePath);
  const editor = useWorkspaceStore((s) => s.editor);
  const loading = useWorkspaceStore((s) => s.loading);
  const loadFromSettings = useWorkspaceStore((s) => s.loadFromSettings);
  const switchWorkspace = useWorkspaceStore((s) => s.switchWorkspace);

  const [error, setError] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [selectedProject, setSelectedProject] = useState<ProjectDetailData | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  // store 的 ProjectItem 是窄化类型，实际返回值含 sub_projects（见后端 scan_workspace）
  const cards = useMemo<ProjectCard[]>(
    () =>
      projects.map((p) => {
        const subs = (p as Partial<ProjectCard>).sub_projects;
        return { ...p, has_readme: p.has_readme ?? false, sub_projects: subs ?? [] };
      }),
    [projects],
  );

  const filteredProjects = useMemo(() => {
    let list = cards;
    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      list = list.filter((p) => p.name.toLowerCase().includes(q) || p.path.toLowerCase().includes(q));
    }
    return list;
  }, [cards, searchQuery]);

  // ===== 页面加载时自动读取设置并扫描 =====
  useEffect(() => {
    void loadFromSettings();
    // 如果带有 ?detail= 参数（如从开发模式页面返回），自动打开项目详情
    const searchParams = new URLSearchParams(location.search);
    const detailPath = searchParams.get('detail');
    if (detailPath) {
      void showDetail(detailPath);
      // 清除查询参数，避免刷新时重复打开
      navigate(location.pathname, { replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ===== 全局快捷键：Cmd/Ctrl+K → 聚焦搜索 =====
  useEffect(() => {
    function handleKeydown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        document.querySelector<HTMLElement>('.workspace-page__hero-search-input')?.focus();
      }
    }
    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  }, []);

  // ===== 切换工作空间 =====
  async function handleSwitchWorkspace(path: string) {
    try {
      await switchWorkspace(path);
      setSearchQuery('');
    } catch (e) {
      setError(`切换工作空间失败: ${e}`);
    }
  }

  // ===== 点击卡片打开编辑器 =====
  async function openProject(path: string) {
    if (!editor.command) {
      setError('请先在「设置」页面配置默认编辑器');
      return;
    }
    try {
      await invoke('open_in_editor', { path, editorCommand: editor.command });
    } catch (e) {
      setError(`打开编辑器失败: ${e}`);
    }
  }

  function openEditorForPath(path: string) {
    if (!editor.command) {
      setError('请先在「设置」页面配置默认编辑器');
      return;
    }
    invoke('open_in_editor', { path, editorCommand: editor.command }).catch((e) => {
      setError(`打开编辑器失败: ${e}`);
    });
  }

  // ===== 项目详情 =====
  async function showDetail(projectPath: string) {
    // 滚动到顶部
    document.querySelector('.content')?.scrollTo(0, 0);
    setDetailLoading(true);
    setError('');
    try {
      const detail = await invoke<ProjectDetailData>('get_project_detail', { path: projectPath });
      setSelectedProject(detail);
    } catch (e) {
      setError(`加载详情失败: ${e}`);
    } finally {
      setDetailLoading(false);
    }
  }

  function backToWorkspace() {
    setSelectedProject(null);
    document.querySelector('.content')?.scrollTo(0, 0);
  }

  // ===== 新建项目 =====
  function handleProjectCreated(newProject: ProjectCard) {
    const next = [...useWorkspaceStore.getState().projects, newProject].sort((a, b) =>
      a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
    );
    useWorkspaceStore.setState({ projects: next });
    setShowCreateModal(false);
    setError('');
  }

  const editorForDetail: EditorSetting = editor;

  return (
    <div className="workspace-page">
      {/* 未设置工作空间 */}
      {!activePath && !loading && (
        <div className="workspace-page__no-workspace">
          <div className="workspace-page__no-workspace-icon">📂</div>
          <h3>尚未设置工作空间</h3>
          <p>前往设置页面选择一个目录作为工作空间</p>
          <Link to="/settings" className="workspace-page__settings-btn">
            <Folder size={16} strokeWidth={2.5} />
            前往设置
          </Link>
        </div>
      )}

      {/* 错误提示 */}
      {error && (
        <div className="workspace-page__error-banner">
          <div className="workspace-page__error-icon">⚠️</div>
          <span>{error}</span>
          <button className="workspace-page__error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {/* 加载状态 */}
      {loading && (
        <div className="workspace-page__loading-container">
          <div className="workspace-page__loading-card">
            <div className="workspace-page__spinner-ring" />
            <div className="workspace-page__loading-text">
              <span className="workspace-page__loading-title">正在扫描工作空间</span>
              <span className="workspace-page__loading-desc">遍历目录并检查 README.md...</span>
            </div>
          </div>
        </div>
      )}

      {/* 工作空间内容区 */}
      {selectedProject || detailLoading ? (
        <ProjectDetailView
          project={selectedProject ?? EMPTY_DETAIL}
          loading={detailLoading}
          editor={editorForDetail}
          onBack={backToWorkspace}
          onOpenEditor={openEditorForPath}
        />
      ) : (
        !loading &&
        activePath && (
          <>
            {/* 居中搜索区 */}
            <div className="workspace-page__hero-search">
              <div className="workspace-page__hero-search-inner">
                <div className="workspace-page__hero-search-icon">
                  <Search size={28} strokeWidth={2} />
                </div>
                <input
                  type="text"
                  className="workspace-page__hero-search-input"
                  placeholder="搜索项目名称..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  autoFocus
                />
                {searchQuery && (
                  <button
                    className="workspace-page__hero-search-clear"
                    onClick={() => setSearchQuery('')}
                  >
                    ✕
                  </button>
                )}
              </div>
              <div className="workspace-page__hero-meta">
                {/* 工作空间选择器 */}
                <div className="workspace-page__ws-selector">
                  <Folder size={14} strokeWidth={2} />
                  <select
                    className="workspace-page__ws-select"
                    value={activePath}
                    onChange={(e) => void handleSwitchWorkspace(e.target.value)}
                  >
                    {workspaces.map((ws) => (
                      <option key={ws.path} value={ws.path}>
                        {ws.name}
                      </option>
                    ))}
                  </select>
                </div>
                <span className="workspace-page__hero-dot">·</span>
                <span className="workspace-page__hero-count">{projects.length} 个项目</span>
                <span className="workspace-page__hero-dot">·</span>
                <Link to="/settings" className="workspace-page__hero-link">
                  管理工作空间
                </Link>
                <span className="workspace-page__hero-dot">·</span>
                <button
                  className="workspace-page__hero-create"
                  onClick={() => setShowCreateModal(true)}
                >
                  <Plus size={14} strokeWidth={2.5} />
                  新建项目
                </button>
              </div>
            </div>

            {/* 搜索无结果 */}
            {filteredProjects.length === 0 && projects.length > 0 && (
              <div className="workspace-page__no-results">
                <span className="workspace-page__no-results-icon">🔍</span>
                <h3>未找到匹配的项目</h3>
                <p>
                  尝试使用不同的关键词搜索，或{' '}
                  <button className="workspace-page__link-btn" onClick={() => setSearchQuery('')}>
                    清除搜索
                  </button>
                </p>
              </div>
            )}

            {/* 项目卡片网格 */}
            {filteredProjects.length > 0 && (
              <div className="workspace-page__card-grid">
                {filteredProjects.map((project) => (
                  <ProjectCardView
                    key={project.path}
                    project={project}
                    searchQuery={searchQuery}
                    editorName={editor.name}
                    onOpen={(p) => void showDetail(p.path)}
                    onOpenProject={(path) => void openProject(path)}
                  />
                ))}
              </div>
            )}

            {/* 空工作空间 */}
            {projects.length === 0 && (
              <div className="workspace-page__empty-state">
                <div className="workspace-page__empty-illustration">
                  <span className="workspace-page__empty-icon">📭</span>
                </div>
                <h3>这里还没有项目</h3>
                <p>该目录下没有找到任何子文件夹</p>
              </div>
            )}
          </>
        )
      )}

      {/* ===== 新建项目弹窗 ===== */}
      {showCreateModal && (
        <CreateProjectModal
          workspacePath={activePath}
          onClose={() => {
            setShowCreateModal(false);
            setError('');
          }}
          onCreated={handleProjectCreated}
          onError={(msg) => setError(msg)}
        />
      )}
    </div>
  );
}
