/**
 * Agent 对话右侧的文件预览面板（产物预览）。
 *
 * Agent 产出单文件后，点击聊天内产物条目的「预览」在此打开：
 * - 文本类 → pre 直出；Markdown → MarkdownMessage 渲染；
 *   代码类 → md 代码块（highlight.js）高亮
 * - 图片 / 视频 → data URL 内嵌；PDF → blob URL + iframe 内嵌
 * - 不支持内嵌的（docx / xlsx / zip 等）或读取失败 / 超限 →
 *   居中引导卡片，提供「使用默认应用打开」（plugin-opener openPath）
 *
 * 结构对齐 LinkPreviewPanel：左缘分隔条拖拽调宽 + 工具条（刷新 /
 * 默认应用 / 关闭）。单窗口语义：打开新文件即替换当前内容。
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openPath } from '@tauri-apps/plugin-opener';
import {
  AppWindow,
  FileCode,
  FileImage,
  FileText,
  FileVideoCamera,
  RotateCw,
  TriangleAlert,
  X,
} from 'lucide-react';
import MarkdownMessage from '../chat/MarkdownMessage';
import { useFilePreviewStore, FILE_PREVIEW_WIDTH } from '@/stores/filePreview';
import './FilePreviewPanel.scss';

interface PreviewPayload {
  kind: 'text' | 'markdown' | 'code' | 'image' | 'video' | 'pdf' | 'unsupported';
  text: string | null;
  data_url: string | null;
  lang: string | null;
  mime: string;
  size: number;
}

/** data URL → blob URL（PDF 的 data URL 在 webview iframe 里渲染不稳，blob 化最可靠） */
function dataUrlToBlobUrl(dataUrl: string): string {
  const comma = dataUrl.indexOf(',');
  const meta = dataUrl.slice(0, comma);
  const b64 = dataUrl.slice(comma + 1);
  const mime = /data:([^;]+)/.exec(meta)?.[1] ?? 'application/octet-stream';
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return URL.createObjectURL(new Blob([bytes], { type: mime }));
}

function fmtSize(size: number): string {
  if (size >= 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  if (size >= 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${size} B`;
}

/** kind → 工具条/回退卡片图标 */
function kindIcon(kind: PreviewPayload['kind']) {
  if (kind === 'image') return FileImage;
  if (kind === 'video') return FileVideoCamera;
  if (kind === 'code') return FileCode;
  if (kind === 'unsupported') return TriangleAlert;
  return FileText;
}

export default function FilePreviewPanel() {
  const path = useFilePreviewStore((s) => s.path);
  const version = useFilePreviewStore((s) => s.version);
  const width = useFilePreviewStore((s) => s.width);
  const setWidth = useFilePreviewStore((s) => s.setWidth);
  const reload = useFilePreviewStore((s) => s.reload);
  const close = useFilePreviewStore((s) => s.close);

  const name = useMemo(() => path.split(/[\\/]/).pop() || path, [path]);

  const [state, setState] = useState<{
    loading: boolean;
    error: string | null;
    payload: PreviewPayload | null;
  }>({ loading: true, error: null, payload: null });

  // 路径 / 刷新变化时重读文件（cancelled 守卫防乱序覆盖）
  useEffect(() => {
    let cancelled = false;
    setState({ loading: true, error: null, payload: null });
    invoke<PreviewPayload>('preview_read_file', { path })
      .then((payload) => {
        if (!cancelled) setState({ loading: false, error: null, payload });
      })
      .catch((e) => {
        if (!cancelled) setState({ loading: false, error: String(e), payload: null });
      });
    return () => {
      cancelled = true;
    };
  }, [path, version]);

  // PDF：data URL → blob URL（每次 payload 更换时重建并 revoke 旧 URL）
  const [pdfUrl, setPdfUrl] = useState<string | null>(null);
  useEffect(() => {
    if (state.payload?.kind === 'pdf' && state.payload.data_url) {
      const url = dataUrlToBlobUrl(state.payload.data_url);
      setPdfUrl(url);
      return () => URL.revokeObjectURL(url);
    }
    setPdfUrl(null);
  }, [state.payload]);

  // 代码围栏：正文里出现更长反引号串时升级围栏长度，避免内容截断
  const fence = useMemo(() => {
    if (state.payload?.kind !== 'code' || !state.payload.text) return '```';
    let maxRun = 0;
    for (const m of state.payload.text.matchAll(/`+/g)) {
      if (m[0].length > maxRun) maxRun = m[0].length;
    }
    return '`'.repeat(Math.max(3, maxRun + 1));
  }, [state.payload]);

  // 拖拽分隔条调宽（与 LinkPreviewPanel 同款交互）
  const resizeRef = useRefOfResize(width, setWidth);

  const payload = state.payload;
  const Icon = kindIcon(payload?.kind ?? 'text');

  return (
    <aside className="file-preview" style={{ width }}>
      <div
        className="file-preview__resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="拖动调整预览面板宽度"
        title="拖动调整宽度 · 双击重置"
        onPointerDown={resizeRef.onDown}
        onPointerMove={resizeRef.onMove}
        onPointerUp={resizeRef.onUp}
        onPointerCancel={resizeRef.onUp}
        onDoubleClick={() => setWidth(FILE_PREVIEW_WIDTH.default)}
      />

      <div className="file-preview__bar">
        <div className="file-preview__addr" title={path}>
          <Icon size={12} strokeWidth={1.8} />
          <span className="file-preview__name">{name}</span>
          {payload ? (
            <span className="file-preview__size">{fmtSize(payload.size)}</span>
          ) : null}
        </div>
        <div className="file-preview__btns">
          <button type="button" onClick={reload} title="刷新" aria-label="刷新">
            <RotateCw size={13} strokeWidth={2} />
          </button>
          <button
            type="button"
            onClick={() => void openPath(path).catch(() => {})}
            title="使用默认应用打开"
            aria-label="使用默认应用打开"
          >
            <AppWindow size={13} strokeWidth={2} />
          </button>
          <button type="button" onClick={close} title="关闭预览" aria-label="关闭预览">
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      <div className="file-preview__body">
        {state.loading ? <div className="file-preview__loading">加载中…</div> : null}

        {!state.loading && state.error ? (
          <div className="file-preview__fallback">
            <TriangleAlert size={22} strokeWidth={1.8} />
            <b>无法内嵌预览</b>
            <span>{state.error}</span>
            <button type="button" onClick={() => void openPath(path).catch(() => {})}>
              <AppWindow size={13} strokeWidth={2} />
              使用默认应用打开
            </button>
          </div>
        ) : null}

        {!state.loading && !state.error && payload ? (
          <>
            {(payload.kind === 'text' || payload.kind === 'code') && payload.text ? (
              payload.kind === 'text' ? (
                <pre className="file-preview__text">{payload.text}</pre>
              ) : (
                <div className="file-preview__md">
                  <MarkdownMessage content={`${fence}${payload.lang ?? ''}\n${payload.text}\n${fence}`} />
                </div>
              )
            ) : null}

            {payload.kind === 'markdown' && payload.text ? (
              <div className="file-preview__md">
                <MarkdownMessage content={payload.text} />
              </div>
            ) : null}

            {payload.kind === 'image' && payload.data_url ? (
              <div className="file-preview__media">
                <img src={payload.data_url} alt={name} />
              </div>
            ) : null}

            {payload.kind === 'video' && payload.data_url ? (
              <div className="file-preview__media">
                <video src={payload.data_url} controls preload="metadata" />
              </div>
            ) : null}

            {payload.kind === 'pdf' ? (
              pdfUrl ? (
                <iframe className="file-preview__frame" src={pdfUrl} title={name} />
              ) : (
                <div className="file-preview__loading">加载中…</div>
              )
            ) : null}

            {payload.kind === 'unsupported' ? (
              <div className="file-preview__fallback">
                <TriangleAlert size={22} strokeWidth={1.8} />
                <b>暂不支持内嵌预览</b>
                <span>
                  该文件类型（{payload.mime}）无法在应用内直接展示
                </span>
                <button type="button" onClick={() => void openPath(path).catch(() => {})}>
                  <AppWindow size={13} strokeWidth={2} />
                  使用默认应用打开
                </button>
              </div>
            ) : null}
          </>
        ) : null}
      </div>

      {/* PDF 内嵌失败兜底：iframe 被拦截时给用户明确出路 */}
      {payload?.kind === 'pdf' && pdfUrl ? (
        <div className="file-preview__hint">
          <span>PDF 显示异常？</span>
          <button type="button" onClick={() => void openPath(path).catch(() => {})}>
            <AppWindow size={11} strokeWidth={2} />
            使用默认应用打开
          </button>
        </div>
      ) : null}
    </aside>
  );
}

// ============================================================
// 分隔条拖拽（独立 hook 形态内聚在文件底部，状态不参与渲染）
// ============================================================

function useRefOfResize(width: number, setWidth: (w: number) => void) {
  const st = useRef<{ startX: number; startW: number } | null>(null);
  function onDown(e: React.PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    st.current = { startX: e.clientX, startW: width };
    document.body.classList.add('is-file-preview-resizing');
  }
  function onMove(e: React.PointerEvent) {
    const cur = st.current;
    if (!cur) return;
    // 向左拖加宽；给对话列保留至少 380px
    const max = Math.min(FILE_PREVIEW_WIDTH.max, window.innerWidth - 380);
    setWidth(Math.max(FILE_PREVIEW_WIDTH.min, Math.min(max, cur.startW + (cur.startX - e.clientX))));
  }
  function onUp() {
    st.current = null;
    document.body.classList.remove('is-file-preview-resizing');
  }
  return { onDown, onMove, onUp };
}
