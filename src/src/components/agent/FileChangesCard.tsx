/**
 * 文件变更卡片（FileChanges）
 *
 * 展示一次 Agent Run 实际修改了哪些文件。数据边界：
 * - 卡片条目由 store 在 Run 终态时从工具 artifacts 聚合（宿主事实，
 *   不经模型自述），本组件只负责呈现；可恢复性（restorable）在挂载后
 *   从宿主命令 agent_change_sets 补全，未确认前隐藏恢复入口。
 * - 「查看差异 / 恢复修改」按需调用宿主命令（agent_change_sets /
 *   agent_change_diff / agent_change_restore），不在消息里内联 diff 数据。
 * - restorable=false 的文件（无快照，如目录/超大文件）隐藏恢复入口。
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleCheck,
  Diff,
  FileEdit,
  FilePlus,
  FileMinus,
  LoaderCircle,
  Undo2,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import { useAgentStore } from '@/stores/agent';
import type { FileChangeItem } from '@/stores/agent';
import './FileChangesCard.scss';

const KIND_META: Record<FileChangeItem['kind'], { label: string; icon: typeof FileEdit }> = {
  created: { label: '新增', icon: FilePlus },
  modified: { label: '修改', icon: FileEdit },
  deleted: { label: '删除', icon: FileMinus },
  moved: { label: '移动', icon: FileEdit },
};

interface DiffState {
  path: string;
  diff: string;
  loading: boolean;
  error?: string;
}

interface FileChangesCardProps {
  runId: string;
  files: FileChangeItem[];
}

export function FileChangesCard({ runId, files }: FileChangesCardProps) {
  const sessionId = useAgentStore((s) => s.sessionId);
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const [expanded, setExpanded] = useState(files.length <= 6);
  // 宿主确认的可恢复路径集合；null 表示尚未拉取，此时不显示恢复入口（安全降级）
  const [restorableSet, setRestorableSet] = useState<Set<string> | null>(null);
  const [diff, setDiff] = useState<DiffState | null>(null);
  const [confirm, setConfirm] = useState<FileChangeItem[] | null>(null);
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ ok: boolean; text: string } | null>(null);

  const totals = useMemo(
    () =>
      files.reduce(
        (acc, f) => ({ add: acc.add + f.insertions, del: acc.del + f.deletions }),
        { add: 0, del: 0 },
      ),
    [files],
  );
  useEffect(() => {
    let cancelled = false;
    invoke<{ run_id: string; files: { path: string; restorable: boolean }[] }[]>(
      'agent_change_sets',
      { sessionId },
    )
      .then((sets) => {
        if (cancelled) return;
        const mine = sets.find((s) => s.run_id === runId);
        setRestorableSet(new Set((mine?.files ?? []).filter((f) => f.restorable).map((f) => f.path)));
      })
      .catch(() => {
        if (!cancelled) setRestorableSet(new Set());
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, runId]);

  const restorable = useMemo(
    () => (restorableSet ? files.filter((f) => restorableSet.has(f.path)) : []),
    [files, restorableSet],
  );

  async function openDiff(path: string) {
    setDiff({ path, diff: '', loading: true });
    try {
      const res = await invoke<{ path: string; diff: string; insertions: number; deletions: number }>(
        'agent_change_diff',
        { sessionId, runId, path },
      );
      setDiff({
        path,
        diff: res.diff || '（内容与修改前一致，无差异）',
        loading: false,
      });
    } catch (e) {
      setDiff({ path, diff: '', loading: false, error: String(e) });
    }
  }

  async function doRestore(targets: FileChangeItem[]) {
    setRestoreBusy(true);
    setFeedback(null);
    try {
      const res = await invoke<{
        restored: { path: string; action: string }[];
        conflicts: { path: string; reason: string }[];
      }>('agent_change_restore', {
        sessionId,
        runId,
        paths: targets.map((t) => t.path),
        force: false,
      });
      const parts: string[] = [];
      if (res.restored.length > 0) {
        parts.push(`已恢复 ${res.restored.length} 个文件`);
      }
      if (res.conflicts.length > 0) {
        parts.push(`${res.conflicts.length} 个被拒绝：${res.conflicts[0].path} ${res.conflicts[0].reason}`);
      }
      setFeedback({ ok: res.conflicts.length === 0, text: parts.join('；') || '没有需要恢复的文件' });
      setConfirm(null);
    } catch (e) {
      setFeedback({ ok: false, text: String(e) });
      setConfirm(null);
    } finally {
      setRestoreBusy(false);
    }
  }

  return (
    <div className="fcard" data-run={runId}>
      <button
        type="button"
        className="fcard__head"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        {expanded ? <ChevronDown size={14} strokeWidth={1.8} /> : <ChevronRight size={14} strokeWidth={1.8} />}
        <FileEdit size={14} strokeWidth={1.8} />
        <span className="fcard__title">本次修改了 {files.length} 个文件</span>
        <span className="fcard__state">
          <CircleCheck size={12} strokeWidth={2} />
          已完成
        </span>
        <span className="fcard__stats">
          <span className="fcard__add">+{totals.add}</span>
          <span className="fcard__del">−{totals.del}</span>
        </span>
      </button>
      {expanded && (
        <ul className="fcard__list">
          {files.map((f) => {
            const meta = KIND_META[f.kind];
            const Icon = meta.icon;
            return (
              <li key={f.path} className="fcard__row">
                <Icon size={13} strokeWidth={1.8} className={`fcard__kind is-${f.kind}`} />
                <code className="fcard__path" title={f.from ? `${f.from} → ${f.path}` : f.path}>
                  {f.path}
                </code>
                <span className={`fcard__badge is-${f.kind}`}>{meta.label}</span>
                {f.insertions > 0 && <span className="fcard__add">+{f.insertions}</span>}
                {f.deletions > 0 && <span className="fcard__del">−{f.deletions}</span>}
                <span className="fcard__actions">
                  <button
                    type="button"
                    className="fcard__act"
                    disabled={diff?.loading}
                    onClick={() => void openDiff(f.path)}
                  >
                    <Diff size={12} strokeWidth={1.8} />
                    查看差异
                  </button>
                  {restorableSet?.has(f.path) && (
                    <button
                      type="button"
                      className="fcard__act"
                      disabled={running || restoreBusy}
                      title={running ? '会话运行中，暂不能恢复' : undefined}
                      onClick={() => setConfirm([f])}
                    >
                      <Undo2 size={12} strokeWidth={1.8} />
                      恢复修改
                    </button>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {expanded && restorable.length > 1 && (
        <div className="fcard__foot">
          {feedback && (
            <span className={`fcard__feedback is-${feedback.ok ? 'ok' : 'err'}`}>{feedback.text}</span>
          )}
          <button
            type="button"
            className="fcard__restore-all"
            disabled={running || restoreBusy}
            onClick={() => setConfirm(restorable)}
          >
            {restoreBusy ? (
              <LoaderCircle size={12} strokeWidth={2} className="is-spin" />
            ) : (
              <CheckCheck size={12} strokeWidth={1.8} />
            )}
            恢复全部修改
          </button>
        </div>
      )}
      {expanded && feedback && restorable.length <= 1 && (
        <div className="fcard__foot">
          <span className={`fcard__feedback is-${feedback.ok ? 'ok' : 'err'}`}>{feedback.text}</span>
        </div>
      )}

      <Modal
        open={diff !== null}
        onClose={() => setDiff(null)}
        title={diff ? `差异 · ${diff.path}` : '查看差异'}
        variant="default"
      >
        {diff?.loading && (
          <div className="fcard__diff-loading">
            <LoaderCircle size={16} strokeWidth={2} className="is-spin" />
            正在读取差异…
          </div>
        )}
        {diff && !diff.loading && diff.error && <div className="fcard__diff-error">{diff.error}</div>}
        {diff && !diff.loading && !diff.error && <pre className="fcard__diff">{diff.diff}</pre>}
      </Modal>

      <Modal
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title="恢复修改"
        variant="confirm"
        footer={
          <>
            <button type="button" className="fcard__btn" onClick={() => setConfirm(null)}>
              取消
            </button>
            <button
              type="button"
              className="fcard__btn is-primary"
              disabled={restoreBusy}
              onClick={() => confirm && void doRestore(confirm)}
            >
              确认恢复
            </button>
          </>
        }
      >
        <div className="fcard__confirm">
          将把以下 {confirm?.length ?? 0} 个文件恢复到本次修改前的状态：
          <ul>
            {confirm?.map((f) => (
              <li key={f.path}>
                <code>{f.path}</code>
              </li>
            ))}
          </ul>
          若文件在此之后又被修改过，恢复会被拒绝（不会静默覆盖）。
        </div>
      </Modal>
    </div>
  );
}
