/**
 * Agent 对话右侧的链接预览面板。
 *
 * Agent 回复里的链接被拦截后（MarkdownMessage 的 linkPreview 模式），
 * 在此用 iframe 打开；工具条支持后退 / 前进 / 刷新 / 系统浏览器打开 / 关闭。
 *
 * 拖出新窗口：按住头部拖拽手柄，把指针拖出面板边界后松开，
 * 即用 WebviewWindow 弹出一个独立窗口展示当前页面；
 * 内嵌面板保持不动（复制语义，内嵌始终是默认形态）
 * （创建失败时回退到系统浏览器打开）。
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ChevronLeft, ChevronRight, RotateCw, ExternalLink, X, Globe, GripVertical, ShieldAlert } from 'lucide-react';
import { openInBrowser } from '@/lib/openService';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useLinkPreviewStore, PREVIEW_WIDTH } from '@/stores/linkPreview';
import './LinkPreviewPanel.scss';

/** 指针超出面板边界该距离后判定为「拖出」 */
const DRAG_OUT_THRESHOLD = 24;

export default function LinkPreviewPanel() {
  const url = useLinkPreviewStore((s) => s.stack[s.index] ?? '');
  const canBack = useLinkPreviewStore((s) => s.index > 0);
  const canForward = useLinkPreviewStore((s) => s.index < s.stack.length - 1);
  const back = useLinkPreviewStore((s) => s.back);
  const forward = useLinkPreviewStore((s) => s.forward);
  const close = useLinkPreviewStore((s) => s.close);
  const width = useLinkPreviewStore((s) => s.width);
  const setWidth = useLinkPreviewStore((s) => s.setWidth);

  const panelRef = useRef<HTMLElement | null>(null);
  // 拖出判定与弹窗去重：副作用不能放进 setState updater（StrictMode 会双调用）
  const dragOutRef = useRef(false);
  const poppingRef = useRef(false);
  // key 变化强制重建 iframe（刷新；后退/前进由 url 变化自然触发重建）
  const [reloadKey, setReloadKey] = useState(0);
  const [drag, setDrag] = useState<{ x: number; y: number; out: boolean } | null>(null);
  // 站点拒绝内嵌（X-Frame-Options / CSP frame-ancestors）时置 true，展示引导卡片
  const [blocked, setBlocked] = useState(false);
  const host = useMemo(() => {
    try {
      return new URL(url).hostname || url;
    } catch {
      return url;
    }
  }, [url]);

  // url 或刷新时重置拦截状态
  useEffect(() => {
    setBlocked(false);
  }, [url, reloadKey]);

  /**
   * 探测内嵌是否被站点拒绝：load 后跨域页面访问 contentWindow 会抛异常（正常加载）；
   * 能访问且仍是 about:blank，说明真实页面从未加载（被 XFO/CSP 拦下）。
   */
  function onFrameLoad(e: React.SyntheticEvent<HTMLIFrameElement>) {
    try {
      const href = e.currentTarget.contentWindow?.location.href;
      if (!href || href === 'about:blank') setBlocked(true);
    } catch {
      // 跨域访问异常 = 真实站点已加载
    }
  }

  // ===== 拖出新窗口 =====

  function onGripDown(e: React.PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    document.body.classList.add('is-link-preview-dragging');
    dragOutRef.current = false;
    setDrag({ x: e.clientX, y: e.clientY, out: false });
  }

  function onGripMove(e: React.PointerEvent) {
    const rect = panelRef.current?.getBoundingClientRect();
    const out = rect
      ? e.clientX < rect.left - DRAG_OUT_THRESHOLD ||
        e.clientX > rect.right + DRAG_OUT_THRESHOLD ||
        e.clientY < rect.top - DRAG_OUT_THRESHOLD ||
        e.clientY > rect.bottom + DRAG_OUT_THRESHOLD
      : false;
    dragOutRef.current = out;
    setDrag({ x: e.clientX, y: e.clientY, out });
  }

  function onGripUp() {
    if (!dragOutRef.current && !drag) return; // 无拖拽中的会话，忽略重复的 up/cancel
    document.body.classList.remove('is-link-preview-dragging');
    const shouldPop = dragOutRef.current;
    dragOutRef.current = false;
    setDrag(null);
    if (shouldPop) void popOut();
  }

  // ===== 拖拽分隔条调宽 =====

  const resizeRef = useRef<{ startX: number; startW: number } | null>(null);

  function onResizeDown(e: React.PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    resizeRef.current = { startX: e.clientX, startW: width };
    document.body.classList.add('is-link-preview-resizing');
  }

  function onResizeMove(e: React.PointerEvent) {
    const st = resizeRef.current;
    if (!st) return;
    // 向左拖加宽；上限同时受窗口宽度约束，给对话列保留至少 380px
    const max = Math.min(PREVIEW_WIDTH.max, window.innerWidth - 380);
    setWidth(Math.max(PREVIEW_WIDTH.min, Math.min(max, st.startW + (st.startX - e.clientX))));
  }

  function onResizeUp() {
    resizeRef.current = null;
    document.body.classList.remove('is-link-preview-resizing');
  }

  /** 弹出独立窗口展示当前页面；内嵌面板保持不动（复制语义）。失败回退系统浏览器 */
  async function popOut() {
    if (!url || poppingRef.current) return;
    poppingRef.current = true;
    try {
      const win = new WebviewWindow(`link-preview-${Date.now()}`, {
        url,
        title: host,
        width: 1080,
        height: 760,
      });
      await new Promise<void>((resolve, reject) => {
        win.once('tauri://created', () => resolve());
        win.once('tauri://error', () => reject(new Error('create webview window failed')));
      });
    } catch {
      void openInBrowser(url).catch(() => {});
    } finally {
      poppingRef.current = false;
    }
  }

  return (
    <aside className="link-preview" ref={panelRef} style={{ width }}>
      {/* 左缘拖拽分隔条：拖动调宽，双击重置默认宽度 */}
      <div
        className="link-preview__resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="拖动调整预览面板宽度"
        title="拖动调整宽度 · 双击重置"
        onPointerDown={onResizeDown}
        onPointerMove={onResizeMove}
        onPointerUp={onResizeUp}
        onPointerCancel={onResizeUp}
        onDoubleClick={() => setWidth(PREVIEW_WIDTH.default)}
      />
      <div className="link-preview__bar">
        <span
          className={`link-preview__grip${drag ? ' is-dragging' : ''}`}
          onPointerDown={onGripDown}
          onPointerMove={onGripMove}
          onPointerUp={onGripUp}
          onPointerCancel={onGripUp}
          title="按住拖动，拖出面板后松开即可在新窗口打开"
          aria-label="拖出新窗口"
        >
          <GripVertical size={13} strokeWidth={2} />
        </span>
        <div className="link-preview__addr" title={url}>
          <Globe size={12} strokeWidth={1.8} />
          <span className="link-preview__host">{host}</span>
        </div>
        <div className="link-preview__btns">
          <button
            type="button"
            disabled={!canBack}
            onClick={back}
            title="后退"
            aria-label="后退"
          >
            <ChevronLeft size={14} strokeWidth={2} />
          </button>
          <button
            type="button"
            disabled={!canForward}
            onClick={forward}
            title="前进"
            aria-label="前进"
          >
            <ChevronRight size={14} strokeWidth={2} />
          </button>
          <button
            type="button"
            onClick={() => setReloadKey((k) => k + 1)}
            title="刷新"
            aria-label="刷新"
          >
            <RotateCw size={13} strokeWidth={2} />
          </button>
          <button
            type="button"
            onClick={() => void openInBrowser(url).catch(() => {})}
            title="在系统浏览器打开"
            aria-label="在系统浏览器打开"
          >
            <ExternalLink size={13} strokeWidth={2} />
          </button>
          <button type="button" onClick={close} title="关闭预览" aria-label="关闭预览">
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      <iframe
        key={`${url}#${reloadKey}`}
        className="link-preview__frame"
        src={url}
        title={host}
        onLoad={onFrameLoad}
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
      />

      {blocked && (
        <div className="link-preview__blocked">
          <ShieldAlert size={22} strokeWidth={1.8} />
          <b>{host} 拒绝被内嵌展示</b>
          <span>
            站点通过 X-Frame-Options / CSP 禁止 iframe 嵌入（浏览器安全策略，无法绕过）
          </span>
          <div className="link-preview__blocked-actions">
            <button type="button" onClick={() => void popOut()}>
              <GripVertical size={13} strokeWidth={2} />
              新窗口打开
            </button>
            <button type="button" onClick={() => void openInBrowser(url).catch(() => {})}>
              <ExternalLink size={13} strokeWidth={2} />
              系统浏览器
            </button>
          </div>
        </div>
      )}

      <div className="link-preview__hint">
        <span>被拒或人机验证（Cloudflare）过不去？换顶级窗口即可：</span>
        <button type="button" onClick={() => void popOut()}>
          <ExternalLink size={11} strokeWidth={2} />
          新窗口打开
        </button>
      </div>

      {/* 拖拽 ghost（跟随指针）与拖出后的全屏投放提示；仅视觉反馈，事件仍由手柄捕获 */}
      {drag &&
        createPortal(
          <div className="link-preview-ghost" style={{ left: drag.x + 14, top: drag.y + 14 }}>
            <Globe size={12} strokeWidth={2} />
            {host}
          </div>,
          document.body,
        )}
      {drag?.out &&
        createPortal(
          <div className="link-preview-dropzone">
            <div className="link-preview-dropzone__card">
              <ExternalLink size={16} strokeWidth={2} />
              松开鼠标，在新窗口打开 {host}
            </div>
          </div>,
          document.body,
        )}
    </aside>
  );
}
