import { useEffect, useState, type CSSProperties, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { folderColor } from './folder-utils';
import type { EditorSetting } from '@/stores/workspace';
import type { ProjectDetail as ProjectDetailData, SubDetail } from './types';
import NodeModulesPanel from './NodeModulesPanel';
import {
  Activity,
  Bot,
  ChevronLeft,
  FileText,
  Folder,
  GitBranch,
  Play,
} from 'lucide-react';
import './ProjectDetail.scss';

interface Props {
  project: ProjectDetailData;
  loading: boolean;
  editor: EditorSetting;
  onBack: () => void;
  onOpenEditor: (path: string) => void;
  /** 在 Agent 页打开该项目 */
  onOpenAgent: (path: string) => void;
}

export default function ProjectDetailView({
  project,
  loading,
  editor,
  onBack,
  onOpenEditor,
  onOpenAgent,
}: Props) {
  const navigate = useNavigate();

  // 检查项目是否有 casp-portal 子目录（用于显示「开发模式」按钮）
  // 通过 Tauri 命令扫描项目目录，不依赖 sub_items（git 仓库时 sub_items 为空）
  const [hasCaspPortal, setHasCaspPortal] = useState(false);
  const [caspPortalPath, setCaspPortalPath] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    detectCaspPortal(project.path);
    return () => {
      cancelled = true;
    };

    async function detectCaspPortal(projectPath: string) {
      setHasCaspPortal(false);
      setCaspPortalPath(null);
      if (!projectPath) return;
      try {
        // 先检查项目本身是否以 casp-portal 开头
        const folder = projectPath.split('/').pop() || '';
        if (folder.startsWith('casp-portal')) {
          if (!cancelled) {
            setHasCaspPortal(true);
            setCaspPortalPath(projectPath);
          }
          return;
        }
        // 调用后端扫描项目目录下的子目录，查找 casp-portal* 目录
        const found = await invoke<string | null>('find_casp_portal_dir', {
          path: projectPath,
        });
        if (found && !cancelled) {
          setHasCaspPortal(true);
          setCaspPortalPath(found);
        }
      } catch {
        // 忽略错误
      }
    }
  }, [project.path]);

  function gotoDevMode() {
    const devPath = caspPortalPath ?? project.path;
    navigate(
      `/dev-mode/${encodeURIComponent(devPath)}?from=${encodeURIComponent(project.path)}`,
    );
  }

  /** 递归渲染子项目（无内部状态，纯函数即可） */
  function renderSubItem(item: SubDetail): ReactNode {
    const indentStyle: CSSProperties = {
      marginLeft: Math.min(item.depth, 5) * 8,
    };
    return (
      <div key={item.path} className="project-detail__sub-card" style={indentStyle}>
        <div className="project-detail__sub-header">
          <span className="project-detail__sub-icon">📁</span>
          <span className="project-detail__sub-name">{item.name}</span>
          <button
            className="project-detail__sub-open-btn"
            onClick={() => onOpenEditor(item.path)}
          >
            <Play size={12} strokeWidth={2.5} />
            打开
          </button>
        </div>
        {item.readme_preview && (
          <div className="project-detail__sub-readme">
            <div className="project-detail__sub-readme-header">📖 README</div>
            <pre className="project-detail__sub-readme-content">
              {item.readme_preview}
            </pre>
          </div>
        )}
        {item.git_repo && (
          <div className="project-detail__git-info">
            <GitBranch size={14} strokeWidth={2} />
            <a
              className="project-detail__git-url"
              href={item.git_repo.remote_url || '#'}
              target="_blank"
              rel="noreferrer"
            >
              {item.git_repo.remote_url}
            </a>
          </div>
        )}
        {item.children && item.children.length > 0 && (
          <>{item.children.map((child) => renderSubItem(child))}</>
        )}
      </div>
    );
  }

  const accent = folderColor(project.name);

  return (
    <div className="project-detail">
      <div className="project-detail__nav">
        <button className="project-detail__back-btn" onClick={onBack}>
          <ChevronLeft size={18} strokeWidth={2.5} />
          返回
        </button>
      </div>
      {loading ? (
        <div className="project-detail__loading">
          <div className="spinner" />
          <span>加载详情中...</span>
        </div>
      ) : (
        <>
          <div className="project-detail__header">
            <div className="project-detail__header-left">
              <div
                className="project-detail__avatar"
                style={{ background: `${accent}22`, color: accent }}
              >
                {project.name.charAt(0).toUpperCase()}
              </div>
              <div>
                <h2 className="project-detail__title">{project.name}</h2>
                <div className="project-detail__path" title={project.path}>
                  <Folder size={14} strokeWidth={2} />
                  <span>{project.path}</span>
                </div>
              </div>
            </div>
            <button
              className="project-detail__open-btn"
              onClick={() => onOpenEditor(project.path)}
            >
              <Play size={16} strokeWidth={2.5} />
              在 {editor.name || '编辑器'} 中打开
            </button>
            <button
              className="project-detail__agent-btn"
              onClick={() => onOpenAgent(project.path)}
              title="在 Agent 页打开该项目"
            >
              <Bot size={16} strokeWidth={2} />
              Agent 打开
            </button>
            {hasCaspPortal && (
              <button className="project-detail__dev-btn" onClick={gotoDevMode}>
                <Activity size={16} strokeWidth={2} />
                开发模式
              </button>
            )}
          </div>

          {project.readme_preview && (
            <div className="project-detail__readme">
              <div className="project-detail__section-title">
                <FileText size={16} strokeWidth={2} />
                <span>README</span>
              </div>
              <pre className="project-detail__readme-content">
                {project.readme_preview}
              </pre>
            </div>
          )}

          {/* ===== node_modules 管理 ===== */}
          <NodeModulesPanel
            projectPath={project.path}
            editorName={editor.name}
            onOpenEditor={onOpenEditor}
          />

          {project.sub_items.length > 0 ? (
            <div className="project-detail__subs">
              <div className="project-detail__section-title">
                <Folder size={16} strokeWidth={2} />
                <span>子项目 ({project.sub_items.length})</span>
              </div>
              <div className="project-detail__sub-list">
                {project.sub_items.map((item) => renderSubItem(item))}
              </div>
            </div>
          ) : (
            <div className="project-detail__no-subs">
              <span className="project-detail__no-subs-icon">📭</span>
              <p>该项目下没有子目录</p>
            </div>
          )}
        </>
      )}
    </div>
  );
}
