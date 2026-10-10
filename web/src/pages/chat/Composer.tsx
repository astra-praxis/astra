/**
 * 输入框 + @ 提及检测
 * 移植自 src/routes/chat/Composer.svelte。
 * bind:value → value + onValueChange 受控模式。
 * @ 提及正则原样保留：
 * - 检测 /(?:^|\s)@([\w\u4e00-\u9fff.-]*)$/
 * - 解析 /@([\w\u4e00-\u9fff][\w\u4e00-\u9fff.-]*)/g
 */
import { useRef, useState } from 'react';
import { ArrowRight } from 'lucide-react';
import { useWorkspaceStore, type ProjectItem } from '@/stores/workspace';
import MentionPopup from './MentionPopup';
import './Composer.scss';

interface Props {
  value: string;
  onValueChange: (v: string) => void;
  disabled: boolean;
  canSend: boolean;
  onSend: () => void;
  onStop: () => void;
}

export default function Composer({ value, onValueChange, disabled, canSend, onSend, onStop }: Props) {
  // ===== @ 提及状态 =====
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const [mentionActive, setMentionActive] = useState(false);
  const [mentionQuery, setMentionQuery] = useState('');
  const [mentionStartIndex, setMentionStartIndex] = useState(0); // @ 符号的起始位置
  const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0); // 当前选中的项目索引（键盘导航）

  const projects = useWorkspaceStore((s) => s.projects);

  // 无搜索词时显示前10个；有搜索词时搜索全部项目，最多显示20个匹配结果
  const q = mentionQuery.toLowerCase();
  const filteredProjects: ProjectItem[] = q
    ? projects.filter((p) => p.name.toLowerCase().includes(q)).slice(0, 20)
    : projects.slice(0, 10);

  function handleKeydown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // 当 @ 提及弹出时，处理键盘导航
    if (mentionActive) {
      // 如果列表为空，不处理方向键
      if (
        filteredProjects.length === 0 &&
        (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === 'Tab')
      ) {
        e.preventDefault();
        return;
      }

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        // 边界检查：确保 selectedIndex 不超过列表长度
        setMentionSelectedIndex((i) => Math.min(i + 1, filteredProjects.length - 1));
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setMentionSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        if (filteredProjects.length > 0 && mentionSelectedIndex >= 0) {
          selectMentionProject(filteredProjects[mentionSelectedIndex]);
        }
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setMentionActive(false);
        return;
      }
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSend();
    }
  }

  /** 监听 textarea 输入，检测 @ 提及 */
  function handleTextareaChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const el = e.currentTarget;
    onValueChange(el.value);

    const val = el.value;
    const cursorPos = el.selectionStart;

    if (!val || cursorPos === 0) {
      setMentionActive(false);
      return;
    }

    // 从光标位置向前查找最近的 @ 符号
    const beforeCursor = val.substring(0, cursorPos);
    // 匹配模式：@ 后面可以跟字母/数字/中文/点/连字符（停止在空格、换行等处）
    // \u4e00-\u9fff 匹配常用中文字符，\w 匹配字母数字下划线
    const atMatch = beforeCursor.match(/(?:^|\s)@([\w\u4e00-\u9fff.-]*)$/);

    if (atMatch) {
      setMentionActive(true);
      setMentionStartIndex(cursorPos - atMatch[0].length + (atMatch[0].startsWith('@') ? 0 : 1));
      setMentionQuery(atMatch[1]); // @ 后面的搜索词
      setMentionSelectedIndex(0);
    } else {
      setMentionActive(false);
    }
  }

  /** 选择项目，插入到输入框 */
  function selectMentionProject(project: ProjectItem) {
    const el = textareaRef.current;
    if (!el) return;

    const val = el.value;
    const cursorPos = el.selectionStart;

    // 替换 @xxx 为 @项目名 并在后面加空格
    const beforeMention = val.substring(0, mentionStartIndex);
    const afterCursor = val.substring(cursorPos);
    const insertion = `@${project.name} `;
    const newValue = beforeMention + insertion + afterCursor;

    onValueChange(newValue);
    setMentionActive(false);

    // 更新光标位置到插入文本之后（等待受控值渲染完成）
    requestAnimationFrame(() => {
      const textarea = textareaRef.current;
      if (textarea) {
        textarea.focus();
        const newCursorPos = beforeMention.length + insertion.length;
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      }
    });
  }

  /** 鼠标点击选择项目 */
  function clickMentionProject(project: ProjectItem, index: number) {
    setMentionSelectedIndex(index);
    selectMentionProject(project);
  }

  return (
    <>
      {/* 弹窗与输入框为兄弟节点（配合父级 .dock 布局） */}
      {mentionActive && projects.length > 0 && (
        <MentionPopup
          query={mentionQuery}
          projects={filteredProjects}
          selectedIndex={mentionSelectedIndex}
          onSelect={clickMentionProject}
        />
      )}

      <div className={`composer${disabled ? ' sending' : ''}`}>
        <textarea
          ref={textareaRef}
          value={value}
          onKeyDown={handleKeydown}
          onChange={handleTextareaChange}
          disabled={disabled}
          rows={2}
          placeholder={disabled ? '生成中…' : '问点什么，输入 @ 可提及项目'}
        />
        <div className="composer-side">
          <div className="composer-hint">
            <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd> 换行
          </div>
          {disabled ? (
            <button type="button" className="stop-btn" onClick={onStop}>
              <span className="stop-square" />
              停止
            </button>
          ) : (
            <button type="button" className="send-btn" onClick={onSend} disabled={!canSend} aria-label="发送">
              <ArrowRight size={16} strokeWidth={2.5} />
            </button>
          )}
        </div>
      </div>
    </>
  );
}
