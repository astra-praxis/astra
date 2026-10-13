import { useEffect, useId, useRef, useState } from 'react';
import { describeArtifact, type Artifact } from './artifact';
import './ArtifactList.scss';

export type { Artifact, LegacyArtifact, SdkArtifact } from './artifact';

export interface ArtifactListProps {
  artifacts: readonly Artifact[];
}

type CopyStatus = 'idle' | 'pending' | 'success' | 'error';

function ArtifactItem({ artifact }: { artifact: Artifact }) {
  const { location, label, mode, from, statistics, preview } = describeArtifact(artifact);
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
    </li>
  );
}

/** Presentation only: never reads, opens, or executes artifact locations. */
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
