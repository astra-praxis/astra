/**
 * @ 项目提及弹出框
 * 移植自 src/routes/chat/MentionPopup.svelte
 */
import { useEffect, useRef } from 'react';
import { Folder } from 'lucide-react';
import type { ProjectItem } from '@/stores/workspace';
import './MentionPopup.scss';

interface Props {
  query: string;
  projects: ProjectItem[];
  selectedIndex: number;
  onSelect: (project: ProjectItem, index: number) => void;
}

export default function MentionPopup({ query, projects, selectedIndex, onSelect }: Props) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // 选中项变化时滚动跟随（键盘导航/初始打开）
  useEffect(() => {
    const selectedEl = listRef.current?.querySelector('.mention-item.selected');
    if (selectedEl) {
      selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex, projects]);

  return (
    <div className="mention-popup">
      <div className="mention-header">
        <Folder size={12} strokeWidth={2.5} />
        <span>选择项目</span>
        {query && <span className="mention-query">搜索: {query}</span>}
      </div>
      <div className="mention-list" ref={listRef}>
        {projects.length === 0 ? (
          <div className="mention-empty">没有匹配的项目</div>
        ) : (
          projects.map((project, i) => (
            <div
              key={project.path}
              className={`mention-item${i === selectedIndex ? ' selected' : ''}`}
              onClick={() => onSelect(project, i)}
              role="button"
              tabIndex={-1}
            >
              <span className="mention-item-icon">📁</span>
              <div className="mention-item-content">
                <span className="mention-item-name">{project.name}</span>
                <span className="mention-item-path" title={project.path}>
                  {project.path}
                </span>
              </div>
            </div>
          ))
        )}
      </div>
      <div className="mention-hint">
        <kbd>↑↓</kbd> 选择 · <kbd>Enter</kbd> 确认 · <kbd>Esc</kbd> 关闭
      </div>
    </div>
  );
}
