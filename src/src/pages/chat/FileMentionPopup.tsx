/**
 * @ 文件引用弹层（Agent 页输入框）
 * 复用 mention-popup 的 SCSS 类（同 SlashPopup 的做法），条目为项目内文件相对路径。
 */
import { useEffect, useRef } from 'react';
import { FileText } from 'lucide-react';
import './MentionPopup.scss';

interface Props {
  query: string;
  files: string[];
  selectedIndex: number;
  onSelect: (file: string, index: number) => void;
}

export default function FileMentionPopup({ query, files, selectedIndex, onSelect }: Props) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // 选中项变化时滚动跟随（键盘导航/初始打开）
  useEffect(() => {
    const selectedEl = listRef.current?.querySelector('.mention-item.selected');
    if (selectedEl) {
      selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex, files]);

  return (
    <div className="mention-popup">
      <div className="mention-header">
        <FileText size={12} strokeWidth={2.5} />
        <span>引用项目文件</span>
        {query && <span className="mention-query">搜索: {query}</span>}
      </div>
      <div className="mention-list" ref={listRef}>
        {files.length === 0 ? (
          <div className="mention-empty">没有匹配的文件</div>
        ) : (
          files.map((file, i) => {
            const slashIdx = file.lastIndexOf('/');
            return (
              <div
                key={file}
                className={`mention-item${i === selectedIndex ? ' selected' : ''}`}
                onClick={() => onSelect(file, i)}
                role="button"
                tabIndex={-1}
              >
                <span className="mention-item-icon">
                  <FileText size={14} strokeWidth={1.8} />
                </span>
                <div className="mention-item-content">
                  <span className="mention-item-name">
                    {slashIdx >= 0 ? file.slice(slashIdx + 1) : file}
                  </span>
                  {slashIdx >= 0 && (
                    <span className="mention-item-path" title={file}>
                      {file.slice(0, slashIdx)}
                    </span>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
      <div className="mention-hint">
        <kbd>↑↓</kbd> 选择 · <kbd>Enter</kbd> 确认 · <kbd>Esc</kbd> 关闭
      </div>
    </div>
  );
}
