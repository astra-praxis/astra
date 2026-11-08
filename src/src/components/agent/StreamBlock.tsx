import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { createOutputScroll } from '@/hooks/outputScroll';
import { useStreamingText } from '@/hooks/useStreamingText';
import { DIFF_LANGUAGE } from './monacoLanguages';
import './StreamBlock.scss';

// Monaco 体积大，懒加载成独立 chunk；仅「内容已完成 + 声明使用 Monaco」时加载
const MonacoView = lazy(() => import('./MonacoView'));

/**
 * 可折叠流式内容块（思考过程 / 写文件内容等共用，参考 Zed 的 thinking 块）。
 *
 * - 流式期间（done 为 false）默认展开，正文随增量自动滚到底部
 * - 完成后自动收起；用户手动展开过则尊重用户选择
 * - 点击头部可随时展开/收起
 * - variant 仅影响样式（思考块 / 工具内容），交互一致
 * - 传入 monacoLang / monacoDiff 时，内容完成后（done）用 Monaco 渲染：
 *   写文件/编辑文件内容获得行号、语法高亮（VSCode 核心编辑器样式）；
 *   流式期间仍用 <pre>（Monaco 不适合逐 token 重建，完成态切换一次）
 */
export function StreamBlock({
  text,
  done,
  title,
  icon,
  variant,
  diff,
  monacoLang,
  monacoDiff,
}: {
  text: string;
  /** 内容是否已结束；缺省视为未结束（流式中） */
  done?: boolean;
  title: string;
  icon?: ReactNode;
  variant?: 'thinking' | 'tool';
  /** 按行渲染 diff（- 删 / + 增），配合流式文本逐行上色 */
  diff?: boolean;
  /** 内容完成后用 Monaco 渲染的语言（如 'typescript'、'rust'） */
  monacoLang?: string;
  /** 内容完成后用 Monaco 渲染为 unified diff 视图（edit_file 对照用） */
  monacoDiff?: boolean;
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

  // diff 模式：逐行拆分渲染，-/+ 行加专属类；空行保留占位
  const bodyNode = diff ? (
    renderedText.split('\n').map((line, i) => (
      <span
        key={i}
        className={
          line.startsWith('- ')
            ? 'stream-block__diff-line is-del'
            : line.startsWith('+ ')
              ? 'stream-block__diff-line is-add'
              : 'stream-block__diff-line'
        }
      >
        {line}
        {'\n'}
      </span>
    ))
  ) : (
    renderedText
  );

  // 内容已完成且声明使用 Monaco：切到编辑器渲染（流式期间保持 <pre>）
  const useMonaco = finished && (monacoLang !== undefined || monacoDiff === true) && text.length > 0;
  const monacoLanguage = monacoDiff ? DIFF_LANGUAGE : monacoLang ?? DIFF_LANGUAGE;

  return (
    <div className={`stream-block${variant ? ` stream-block--${variant}` : ''}${diff ? ' stream-block--diff' : ''}`}>
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
        <>
          {useMonaco ? (
            <div className="stream-block__monaco">
              <Suspense
                fallback={
                  <pre className="stream-block__body" tabIndex={0}>{bodyNode}</pre>
                }
              >
                <MonacoView content={text} language={monacoLanguage} />
              </Suspense>
            </div>
          ) : (
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
              {bodyNode}
            </pre>
          )}
        </>
      )}
    </div>
  );
}
