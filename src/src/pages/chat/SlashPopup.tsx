/**
 * / 技能调用弹出框（复用 mention-popup 的样式类，与 @ 提及弹层同视觉）
 */
import { useEffect, useRef } from 'react';
import { Sparkles } from 'lucide-react';
import type { UserSkillLite } from '@/hooks/useSlash';
import './MentionPopup.scss';

interface Props {
  query: string;
  skills: UserSkillLite[];
  selectedIndex: number;
  onSelect: (skill: UserSkillLite, index: number) => void;
}

export default function SlashPopup({ query, skills, selectedIndex, onSelect }: Props) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // 选中项变化时滚动跟随（键盘导航/初始打开）
  useEffect(() => {
    const selectedEl = listRef.current?.querySelector('.mention-item.selected');
    if (selectedEl) {
      selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex, skills]);

  return (
    <div className="mention-popup">
      <div className="mention-header">
        <Sparkles size={12} strokeWidth={2.5} />
        <span>选择技能</span>
        {query && <span className="mention-query">搜索: {query}</span>}
      </div>
      <div className="mention-list" ref={listRef}>
        {skills.length === 0 ? (
          <div className="mention-empty">没有匹配的技能</div>
        ) : (
          skills.map((skill, i) => (
            <div
              key={skill.name}
              className={`mention-item${i === selectedIndex ? ' selected' : ''}`}
              onClick={() => onSelect(skill, i)}
              role="button"
              tabIndex={-1}
            >
              <span className="mention-item-icon">⚡</span>
              <div className="mention-item-content">
                <span className="mention-item-name">/{skill.name}</span>
                {skill.description && (
                  <span className="mention-item-path" title={skill.description}>
                    {skill.description}
                  </span>
                )}
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
