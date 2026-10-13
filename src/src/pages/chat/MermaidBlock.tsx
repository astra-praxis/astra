/**
 * Mermaid 图渲染块（markdown ```mermaid 代码块）
 *
 * - 动态 import('mermaid')：体积大（~1MB），只在消息里真的出现 mermaid 块时才加载，
 *   Vite 会拆成独立异步 chunk，不拖慢首屏
 * - 渲染失败（流式输出中语法不完整 / 真实语法错误）回退为源码展示 + 错误提示，
 *   代码补全后自动重渲染成图
 * - 跟随 <html data-theme> 亮/暗主题切换重渲染
 * - 缩放浏览：工具条 + Ctrl/⌘滚轮（兼容触控板捏合）缩放、拖拽平移、双击复位、全屏查看
 */
import { memo, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { createPortal } from 'react-dom';
import { Copy, Maximize2, X, ZoomIn, ZoomOut } from 'lucide-react';

type MermaidAPI = (typeof import('mermaid'))['default'];

let mermaidPromise: Promise<MermaidAPI> | null = null;

/** 全局渲染序号：ID 必须跨实例唯一（每实例计数会在多图同帧渲染时撞 ID） */
let renderSeq = 0;

/** 渲染串行队列：mermaid.render 官方不支持并发调用（共享临时 DOM 与内部状态），
 *  多图同帧渲染（如历史会话回放）会互相干扰报错，必须排队逐个执行 */
let renderChain: Promise<void> = Promise.resolve();

function enqueueRender(task: () => Promise<void>): void {
  // task 内部自捕错，链不会 reject；前一个失败不阻塞后续
  renderChain = renderChain.then(task, task);
}

/** 懒加载 mermaid 模块（只加载一次；主题在每次渲染前 initialize） */
function loadMermaid(): Promise<MermaidAPI> {
  if (!mermaidPromise) {
    mermaidPromise = import('mermaid').then((m) => {
      m.default.initialize({ startOnLoad: false, securityLevel: 'strict' });
      return m.default;
    });
  }
  return mermaidPromise;
}

function currentTheme(): 'dark' | 'default' {
  return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'default';
}

/** 按主题微调 mermaid 变量：default/dark 主题的连线都偏浅，加深保证可读性 */
function themeVariables(theme: 'dark' | 'default'): Record<string, string> {
  return theme === 'dark'
    ? {
        lineColor: '#9aa4b8',
        actorLineColor: '#9aa4b8',
        textColor: '#d5dbe6',
        edgeLabelBackground: '#1b1e35',
      }
    : {
        lineColor: '#475569',
        actorLineColor: '#475569',
        textColor: '#1e293b',
        edgeLabelBackground: '#f8fafc',
      };
}

interface MermaidBlockProps {
  code: string;
  /** 流式输出中：渲染失败时不显示错误提示（语法不完整是常态） */
  streaming?: boolean;
}

const MermaidBlock = memo(function MermaidBlock({ code, streaming = false }: MermaidBlockProps) {
  const [theme, setTheme] = useState(currentTheme);
  const [svg, setSvg] = useState('');
  const [error, setError] = useState('');
  const [fullscreen, setFullscreen] = useState(false);

  // 跟随主题切换重渲染
  useEffect(() => {
    const ob = new MutationObserver(() => setTheme(currentTheme()));
    ob.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => ob.disconnect();
  }, []);

  // 渲染（防抖 300ms：流式输出时代码频繁变化，避免每个 chunk 都跑一次渲染；
  // 实际 render 经全局串行队列执行，见 enqueueRender）
  useEffect(() => {
    let cancelled = false;
    setSvg('');
    setError('');
    const timer = setTimeout(() => {
      const id = `astra-mmd-${++renderSeq}`;
      enqueueRender(async () => {
        if (cancelled) return;
        try {
          const mermaid = await loadMermaid();
          mermaid.initialize({
            startOnLoad: false,
            securityLevel: 'strict',
            theme,
            themeVariables: themeVariables(theme),
          });
          const { svg: out } = await mermaid.render(id, code.trim());
          if (!cancelled) setSvg(out);
        } catch (e: unknown) {
          if (!cancelled) setError(e instanceof Error ? e.message : String(e));
        }
      });
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [code, theme]);

  const [copied, setCopied] = useState(false);
  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // 复制失败忽略
    }
  }

  if (svg) {
    return (
      <>
        <ZoomableSvg svg={svg} onOpenFullscreen={() => setFullscreen(true)} />
        {fullscreen &&
          createPortal(
            <div
              className="md-mermaid-overlay"
              onClick={(e) => {
                if (e.target === e.currentTarget) setFullscreen(false);
              }}
            >
              <ZoomableSvg svg={svg} fullscreen onClose={() => setFullscreen(false)} />
            </div>,
            document.body,
          )}
      </>
    );
  }

  // 渲染中 / 失败：回退为源码展示
  return (
    <>
      <pre className="md-mermaid-src">
        <button type="button" className="code-x-btn code-copy-btn" onClick={() => void handleCopy()}>
          <Copy size={12} strokeWidth={2.2} /> {copied ? '已复制' : '复制'}
        </button>
        <code>{code}</code>
      </pre>
      {error && !streaming && <div className="md-mermaid-error">Mermaid 渲染失败：{error}</div>}
    </>
  );
});

// ============================================================
// 缩放查看器（内联与全屏共用）
// ============================================================

const MIN_ZOOM = 0.4;
const MAX_ZOOM = 4;
const ZOOM_STEP = 1.2;

function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

interface ZoomableSvgProps {
  svg: string;
  fullscreen?: boolean;
  onClose?: () => void;
  onOpenFullscreen?: () => void;
}

function ZoomableSvg({ svg, fullscreen = false, onClose, onOpenFullscreen }: ZoomableSvgProps) {
  const clipRef = useRef<HTMLDivElement | null>(null);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef<{ px: number; py: number; ox: number; oy: number } | null>(null);
  const zoomed = zoom !== 1;

  // Ctrl/⌘ + 滚轮（触控板捏合同样带 ctrlKey）缩放；
  // React 合成 wheel 事件是 passive 的无法 preventDefault，须挂原生监听
  useEffect(() => {
    const el = clipRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      setZoom((z) => clampZoom(z * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, []);

  // 全屏时 Esc 关闭
  useEffect(() => {
    if (!fullscreen || !onClose) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [fullscreen, onClose]);

  function zoomBy(factor: number) {
    setZoom((z) => clampZoom(z * factor));
  }

  function reset() {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  }

  function onPointerDown(e: ReactPointerEvent<HTMLDivElement>) {
    if (!zoomed) return;
    dragRef.current = { px: e.clientX, py: e.clientY, ox: pan.x, oy: pan.y };
    setDragging(true);
    e.currentTarget.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: ReactPointerEvent<HTMLDivElement>) {
    const d = dragRef.current;
    if (!d) return;
    setPan({ x: d.ox + (e.clientX - d.px), y: d.oy + (e.clientY - d.py) });
  }

  function onPointerEnd() {
    dragRef.current = null;
    setDragging(false);
  }

  return (
    <div className={`md-mermaid${fullscreen ? ' is-full' : ''}`}>
      <div className="md-mermaid-toolbar">
        <button type="button" className="md-mermaid-tbtn" title="缩小" onClick={() => zoomBy(1 / ZOOM_STEP)}>
          <ZoomOut size={13} strokeWidth={2.2} />
        </button>
        <button type="button" className="md-mermaid-zoom" title="重置缩放（双击图也可）" onClick={reset}>
          {Math.round(zoom * 100)}%
        </button>
        <button type="button" className="md-mermaid-tbtn" title="放大" onClick={() => zoomBy(ZOOM_STEP)}>
          <ZoomIn size={13} strokeWidth={2.2} />
        </button>
        {fullscreen ? (
          <button type="button" className="md-mermaid-tbtn" title="关闭（Esc）" onClick={onClose}>
            <X size={13} strokeWidth={2.2} />
          </button>
        ) : (
          <button type="button" className="md-mermaid-tbtn" title="全屏查看" onClick={onOpenFullscreen}>
            <Maximize2 size={13} strokeWidth={2.2} />
          </button>
        )}
      </div>
      <div
        ref={clipRef}
        className="md-mermaid-clip"
        style={{ overflow: zoomed ? 'hidden' : 'auto' }}
      >
        <div
          className={`md-mermaid-canvas${zoomed ? (dragging ? ' is-dragging' : ' is-pannable') : ''}`}
          style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerEnd}
          onPointerCancel={onPointerEnd}
          onDoubleClick={reset}
        >
          {/* mermaid.render 产物已按 securityLevel: strict 消毒 */}
          <div className="md-mermaid-svg" dangerouslySetInnerHTML={{ __html: svg }} />
        </div>
      </div>
    </div>
  );
}

export default MermaidBlock;
