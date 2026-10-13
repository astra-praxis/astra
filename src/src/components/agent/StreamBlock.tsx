import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { createOutputScroll } from '@/hooks/outputScroll';
import { useStreamingText } from '@/hooks/useStreamingText';
import './StreamBlock.scss';

/**
 * 可折叠流式内容块（思考过程 / 写文件内容等共用，参考 Zed 的 thinking 块）。
 *
 * - 流式期间（done 为 false）默认展开，正文随增量自动滚到底部
 * - 完成后自动收起；用户手动展开过则尊重用户选择
 * - 点击头部可随时展开/收起
 * - variant 仅影响样式（思考块 / 工具内容），交互一致
 */
export function StreamBlock({
  text,
  done,
  title,
  icon,
  variant,
}: {
  text: string;
  /** 内容是否已结束；缺省视为未结束（流式中） */
  done?: boolean;
  title: string;
  icon?: ReactNode;
  variant?: 'thinking' | 'tool';
}) {
  const finished = done ?? false;
  const [open, setOpen] = useState(!finished);
  const [userToggled, setUserToggled] = useState(false);
  const bodyRef = useRef<HTMLPreElement | null>(null);
  const renderedText = useStreamingText(text, !finished);
  const [scroll] = useState(() => createOutputScroll(
    () => bodyRef.current,
    (callback) => requestAnimationFrame(callback),
    (id) => cancelAnimationFrame(id),
  ));
  useEffect(() => () => scroll.dispose(), [scroll]);
  useEffect(() => {
    if (open) scroll.resume();
    else scroll.dispose();
  }, [open, scroll]);

  // 完成后自动收起（用户手动展开过则尊重用户选择）
  useEffect(() => {
    if (finished && !userToggled) setOpen(false);
  }, [finished, userToggled]);

  // 展开且仍在流式输出时，正文滚动条保持在最底部
  useEffect(() => {
    if (open && !finished) scroll.schedule();
  }, [open, renderedText, finished, scroll]);

  return (
    <div className={`stream-block${variant ? ` stream-block--${variant}` : ''}`}>
      <button
        type="button"
        className="stream-block__head"
        onClick={() => {
          setUserToggled(true);
          setOpen((v) => !v);
        }}
      >
        {icon}
        <span>{title}</span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`stream-block__chev${open ? ' rotated' : ''}`}
        />
      </button>
      {open && (
        <pre ref={bodyRef} className="stream-block__body" tabIndex={0}
          onScroll={() => scroll.onScroll()}
          onWheel={(event) => {
            event.stopPropagation();
            if (event.deltaY < 0) scroll.pause();
          }}
          onTouchMove={(event) => { event.stopPropagation(); scroll.pause(); }}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (['ArrowUp', 'PageUp', 'Home'].includes(event.key)) scroll.pause();
          }}
        >
          {renderedText}
        </pre>
      )}
    </div>
  );
}
