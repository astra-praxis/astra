import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { describeArtifact, type Artifact } from './artifact';
import { useFilePreviewStore } from '@/stores/filePreview';
import './ArtifactList.scss';

export type { Artifact, LegacyArtifact, SdkArtifact } from './artifact';

export interface ArtifactListProps {
  artifacts: readonly Artifact[];
}

type CopyStatus = 'idle' | 'pending' | 'success' | 'error';
type OpenStatus = 'idle' | 'pending' | 'error';

/** 生成资产（image/video）内联预览：data URL 经 media_read_asset 惰性加载 */
function MediaPreview({ assetId, mediaKind }: { assetId: string; mediaKind: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const previewId = useId();
  const [expanded, setExpanded] = useState(false);

  async function toggle() {
    const next = !expanded;
    setExpanded(next);
    if (!next || url || error) return;
    try {
      const payload = await invoke<{ data_url: string }>('media_read_asset', { assetId });
      setUrl(payload.data_url);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="artifact-list__media">
      <button
        type="button"
        className="artifact-list__toggle"
        aria-expanded={expanded}
        aria-controls={previewId}
        onClick={toggle}
      >
        {expanded ? '收起预览' : '展开预览'}
      </button>
      <div id={previewId} hidden={!expanded}>
        {url && mediaKind === 'image' ? (
          <img className="artifact-list__image" src={url} alt="生成图片预览" />
        ) : null}
        {url && mediaKind === 'video' ? (
          <video className="artifact-list__video" src={url} controls preload="metadata" />
        ) : null}
        {error ? <div className="artifact-list__media-error">预览加载失败：{error}</div> : null}
      </div>
    </div>
  );
}

function OpenAssetButton({ assetId }: { assetId: string }) {
  const [status, setStatus] = useState<OpenStatus>('idle');
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  async function open() {
    if (status === 'pending') return;
    setStatus('pending');
    try {
      await invoke('media_open_asset', { assetId });
      if (mounted.current) setStatus('idle');
    } catch {
      if (mounted.current) setStatus('error');
    }
  }
  return (
    <>
      <button
        type="button"
        className="artifact-list__copy"
        onClick={open}
        disabled={status === 'pending'}
      >
        {status === 'pending' ? '打开中…' : '打开'}
      </button>
      <span className="artifact-list__feedback" role="status" aria-live="polite">
        {status === 'error' ? '打开失败，请重试。' : null}
      </span>
    </>
  );
}

function ArtifactItem({ artifact }: { artifact: Artifact }) {
  const { location, label, mode, from, statistics, preview, assetId } = describeArtifact(artifact);
  const [expanded, setExpanded] = useState(false);
  const [copyStatus, setCopyStatus] = useState<CopyStatus>('idle');
  const previewId = useId();
  const statusId = useId();
  const mounted = useRef(false);
  const copying = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  async function copyLocation() {
    if (copying.current || !location) return;
    copying.current = true;
    setCopyStatus('pending');
    try {
      if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(location);
      if (mounted.current) setCopyStatus('success');
    } catch {
      if (mounted.current) setCopyStatus('error');
    } finally {
      copying.current = false;
    }
  }

  return (
    <li className="artifact-list__item">
      <div className="artifact-list__header">
        <span className="artifact-list__kind" title={artifact.kind}>{label}</span>
        {mode ? <span className="artifact-list__mode">{mode}</span> : null}
        {location ? (
          <button
            type="button"
            className="artifact-list__copy"
            onClick={() => useFilePreviewStore.getState().openFile(location)}
            aria-label={`预览文件：${location}`}
          >
            预览
          </button>
        ) : null}
        {assetId ? <OpenAssetButton assetId={assetId} /> : null}
        <button
          type="button"
          className="artifact-list__copy"
          onClick={copyLocation}
          disabled={!location || copyStatus === 'pending'}
          aria-label={`复制路径：${location}`}
          aria-describedby={statusId}
        >
          {copyStatus === 'pending' ? '复制中…' : '复制路径'}
        </button>
      </div>
      <code className="artifact-list__path">{location || '未提供路径'}</code>
      {from ? <div className="artifact-list__from">来源：<code>{from}</code></div> : null}
      {statistics.length > 0 ? (
        <div className="artifact-list__statistics">{statistics.join(' · ')}</div>
      ) : null}
      <span id={statusId} className="artifact-list__feedback" role="status" aria-live="polite">
        {copyStatus === 'success' ? '路径已复制' : null}
        {copyStatus === 'error' ? '复制失败，请选择上方路径手动复制。' : null}
      </span>
      {preview ? (
        <div className="artifact-list__preview">
          <button
            type="button"
            className="artifact-list__toggle"
            aria-expanded={expanded}
            aria-controls={previewId}
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? '收起预览' : '展开预览'}
          </button>
          <div id={previewId} hidden={!expanded}>
            {expanded ? <pre className="artifact-list__text" tabIndex={0} aria-label="产物纯文本预览">{preview}</pre> : null}
          </div>
        </div>
      ) : null}
      {assetId && (artifact.kind === 'image' || artifact.kind === 'video') ? (
        <MediaPreview assetId={assetId} mediaKind={artifact.kind} />
      ) : null}
    </li>
  );
}

/**
 * 产物条目：展示元数据，不直接读写文件。点击「预览」只负责把路径交给
 * 全局文件预览面板（FilePreviewPanel），读取与渲染都由面板经后端命令完成。
 */
export function ArtifactList({ artifacts }: ArtifactListProps) {
  if (artifacts.length === 0) return null;
  const occurrences = new Map<string, number>();
  return (
    <ul className="artifact-list" aria-label="工具产物">
      {artifacts.map((artifact) => {
        const identity = 'location' in artifact
          ? JSON.stringify(['sdk', artifact.id, artifact.location])
          : JSON.stringify(['legacy', artifact.kind, artifact.path]);
        const occurrence = occurrences.get(identity) ?? 0;
        occurrences.set(identity, occurrence + 1);
        return <ArtifactItem key={`${identity}:${occurrence}`} artifact={artifact} />;
      })}
    </ul>
  );
}
