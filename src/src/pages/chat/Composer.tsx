/**
 * 输入框 + @ 提及检测
 * 移植自 src/routes/chat/Composer.svelte。
 * bind:value → value + onValueChange 受控模式。
 * @ 提及检测/导航/插入逻辑在 useMention（与 Agent 页共用）。
 */
import { useRef } from 'react';
import { ArrowRight } from 'lucide-react';
import { useWorkspaceStore } from '@/stores/workspace';
import { isSendEnter, useMention } from '@/hooks/useMention';
import MentionPopup from './MentionPopup';
import './Composer.scss';

interface Props {
  value: string;
  onValueChange: (v: string) => void;
  disabled: boolean;
  canSend: boolean;
  onSend: () => void;
  onStop: () => void;
  /** 底栏左侧扩展（模型切换等），渲染在提示文字之前 */
  toolbarSlot?: React.ReactNode;
  /** 底栏右侧扩展（设置/清空等），渲染在发送按钮之前 */
  actionSlot?: React.ReactNode;
}

export default function Composer({
  value,
  onValueChange,
  disabled,
  canSend,
  onSend,
  onStop,
  toolbarSlot,
  actionSlot,
}: Props) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const projects = useWorkspaceStore((s) => s.projects);

  const mention = useMention(projects, textareaRef, undefined, onValueChange);

  function handleKeydown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // 当 @ 提及弹出时，处理键盘导航；处理过的按键不再走发送
    if (mention.handleKeydown(e)) return;

    if (isSendEnter(e)) {
      e.preventDefault();
      onSend();
    }
  }

  /** 监听 textarea 输入，检测 @ 提及 */
  function handleTextareaChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const el = e.currentTarget;
    onValueChange(el.value);
    mention.detect(el.value, el.selectionStart);
  }

  /** 鼠标点击选择项目 */
  function clickMentionProject(project: Parameters<typeof mention.pick>[0], index: number) {
    mention.setSelectedIndex(index);
    onValueChange(mention.pick(project, textareaRef.current?.value ?? value));
  }

  return (
    <>
      {/* 弹窗与输入框为兄弟节点（配合父级 .dock 布局） */}
      {mention.active && projects.length > 0 && (
        <MentionPopup
          query={mention.query}
          projects={mention.filtered}
          selectedIndex={mention.selectedIndex}
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
          {toolbarSlot}
          <span className="composer-spacer" />
          <div className="composer-hint">
            <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd> 换行
          </div>
          {actionSlot}
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
