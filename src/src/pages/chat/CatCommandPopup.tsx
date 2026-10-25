/**
 * & 猫猫互动弹出框（复用 mention-popup 的样式类，与 @提及 / /技能 弹层同视觉）
 * 选中命令即把 &命令名 写入输入框，发送时由 AgentPage 拦截为本地猫猫互动。
 */
import { useEffect, useRef } from 'react';
import { Cat } from 'lucide-react';
import type { CatCommand } from '@/hooks/useCatCommand';
import './MentionPopup.scss';

interface Props {
  query: string;
  commands: CatCommand[];
  selectedIndex: number;
  onSelect: (command: CatCommand, index: number) => void;
}

export default function CatCommandPopup({ query, commands, selectedIndex, onSelect }: Props) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // 选中项变化时滚动跟随（键盘导航/初始打开）
  useEffect(() => {
    const selectedEl = listRef.current?.querySelector('.mention-item.selected');
    if (selectedEl) {
      selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex, commands]);

  return (
    <div className="mention-popup">
      <div className="mention-header">
        <Cat size={12} strokeWidth={2.5} />
        <span>和猫猫互动</span>
        {query && <span className="mention-query">搜索: {query}</span>}
      </div>
      <div className="mention-list" ref={listRef}>
        {commands.length === 0 ? (
          <div className="mention-empty">没有这个互动动作，试试 摸头 / 打招呼 / 玩耍…</div>
        ) : (
          commands.map((c, i) => (
            <div
              key={c.id}
              className={`mention-item${i === selectedIndex ? ' selected' : ''}`}
              onClick={() => onSelect(c, i)}
              role="button"
              tabIndex={-1}
            >
              <span className="mention-item-icon">{c.emoji}</span>
              <div className="mention-item-content">
                <span className="mention-item-name">&{c.label}</span>
                <span className="mention-item-path" title={c.desc}>
                  {c.desc}
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
