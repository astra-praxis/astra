import { folderColor, folderName, highlight } from './folder-utils';
import type { ProjectCard as ProjectCardData } from './types';
import { Bot, Folder, Play } from 'lucide-react';
import './ProjectCard.scss';

interface Props {
  project: ProjectCardData;
  searchQuery: string;
  editorName: string;
  /** 点击卡片主体 → 查看详情 */
  onOpen: (project: ProjectCardData) => void;
  /** 用编辑器打开项目/子项目 */
  onOpenProject: (path: string) => void;
  /** 在 Agent 页打开该项目 */
  onOpenAgent: (path: string) => void;
}

export default function ProjectCardView({
  project,
  searchQuery,
  editorName,
  onOpen,
  onOpenProject,
  onOpenAgent,
}: Props) {
  const accent = folderColor(project.name);

  return (
    <div
      className="project-card"
      style={{ ['--card-accent' as string]: accent }}
      onClick={() => onOpen(project)}
      onKeyDown={(e) => e.key === 'Enter' && onOpen(project)}
      role="button"
      tabIndex={0}
      title="查看项目详情"
    >
      <div className="project-card__accent-bar" />
      <div className="project-card__content">
        <div className="project-card__header">
          <div
            className="project-card__avatar"
            style={{ background: `${accent}22`, color: accent }}
          >
            {project.name.charAt(0).toUpperCase()}
          </div>
          <div className="project-card__header-text">
            <h3
              className="project-card__title"
              dangerouslySetInnerHTML={{ __html: highlight(project.name, searchQuery) }}
            />
            <span
              className="project-card__folder"
              dangerouslySetInnerHTML={{ __html: highlight(folderName(project.path), searchQuery) }}
            />
          </div>
        </div>
        <div className="project-card__path" title={project.path}>
          <Folder size={13} strokeWidth={2} />
          <span>{project.path}</span>
        </div>
        {project.sub_projects.length > 0 && (
          <div className="project-card__sub-projects">
            <div className="project-card__sub-label">
              <Folder size={12} strokeWidth={2} />
              <span>子项目 ({project.sub_projects.length})</span>
            </div>
            <div className="project-card__sub-list">
              {project.sub_projects.map((sub) => (
                <button
                  key={sub.path}
                  className="project-card__sub-item"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenProject(sub.path);
                  }}
                  title={`在 ${editorName || '编辑器'} 中打开 ${sub.name}`}
                >
                  <span className="project-card__sub-item-name">{sub.name}</span>
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="project-card__footer">
          <div className="project-card__footer-right">
            <button
              className="project-card__open-btn"
              onClick={(e) => {
                e.stopPropagation();
                onOpenAgent(project.path);
              }}
              title="在 Agent 页打开该项目"
            >
              <Bot size={13} strokeWidth={2} />
              Agent
            </button>
            <button
              className="project-card__open-btn"
              onClick={(e) => {
                e.stopPropagation();
                onOpenProject(project.path);
              }}
              title={`在 ${editorName || '编辑器'} 中打开`}
            >
              <Play size={13} strokeWidth={2} />
              打开
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
