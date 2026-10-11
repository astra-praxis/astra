import { useEffect, useRef, useState } from 'react';
import { Brain, ChevronDown } from 'lucide-react';
import './ThinkingBlock.scss';

/**
 * 思考过程块（reasoning，参考 Zed 的 thinking 折叠块）。
 *
 * - 流式期间默认展开，正文随增量自动滚到底部
 * - 思考结束（done）后自动收起；用户手动展开过则尊重用户选择
 * - 点击头部可随时展开/收起
 */
export function ThinkingBlock({ text, done }: { text: string; done?: boolean }) {
  const finished = done ?? false;
  const [open, setOpen] = useState(!finished);
  const [userToggled, setUserToggled] = useState(false);
  const bodyRef = useRef<HTMLPreElement | null>(null);

  // 思考完成后自动收起（用户手动展开过则尊重用户选择）
  useEffect(() => {
    if (finished && !userToggled) setOpen(false);
  }, [finished, userToggled]);

  // 展开且仍在流式输出时，正文滚动条保持在最底部
  useEffect(() => {
    const el = bodyRef.current;
    if (open && el) el.scrollTop = el.scrollHeight;
  }, [open, text]);

  return (
    <div className="thinking-block">
      <button
        type="button"
        className="thinking-block__head"
        onClick={() => {
          setUserToggled(true);
          setOpen((v) => !v);
        }}
      >
        <Brain size={13} strokeWidth={1.8} />
        <span>思考过程</span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`thinking-block__chev${open ? ' rotated' : ''}`}
        />
      </button>
      {open && (
        <pre ref={bodyRef} className="thinking-block__body">
          {text}
        </pre>
      )}
    </div>
  );
}
