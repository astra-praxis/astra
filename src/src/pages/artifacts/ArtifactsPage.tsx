/**
 * 产物库页面：浏览 Agent 会话产出的文件产物（ARTIFACT_SPEC）。
 *
 * 数据来自 agent_artifact_list（SqliteArtifactRegistry，location-based
 * 元数据登记）。记录只证明「写入时产物存在于 location」，文件可能已被
 * 覆盖或删除——页面不做存活判断，仅提供打开/复制入口。
 *
 * 版本链在前端按 artifact_key 分组（registry 无分页，spec §4）。
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import {
  ChevronLeft,
  Copy,
  Check,
  ExternalLink,
  FileText,
  History,
} from 'lucide-react';
import { describeArtifact, type Artifact } from '@/components/agent/artifact';
import { useWorkspaceStore } from '@/stores/workspace';
import './ArtifactsPage.scss';

/** agent.rs AgentArtifactInfo 的前端镜像（camelCase 由后端直接序列化） */
interface AgentArtifactInfo {
  id: string;
  threadId: string;
  artifactKey: string;
  kind: string;
  location: string;
  preview: string | null;
  metadata: unknown;
  version: number;
  previousId: string | null;
  createdAt: number;
  status: string;
}

/** agent_sessions.rs AgentSession 的前端镜像（只取列表展示字段） */
interface AgentSessionBrief {
  id: string;
  title: string;
  projectName: string;
  updatedAt: number;
}

interface ArtifactGroup {
  key: string;
  latest: AgentArtifactInfo;
  history: AgentArtifactInfo[];
}

function groupArtifacts(records: AgentArtifactInfo[]): ArtifactGroup[] {
  const byKey = new Map<string, ArtifactGroup>();
  // list 返回 created_at 升序；升序遍历 + 覆盖 latest 即得每组最新版本
  for (const record of records) {
    const group = byKey.get(record.artifactKey) ?? {
      key: record.artifactKey,
      latest: record,
      history: [],
    };
    group.history.push(record);
    group.latest = record;
    byKey.set(record.artifactKey, group);
  }
  return [...byKey.values()].sort(
    (a, b) => b.latest.createdAt - a.latest.createdAt
  );
}

function describeRecord(record: AgentArtifactInfo) {
  // 复用聊天内 ArtifactList 的种类/模式文案（形状兼容 SDK artifact）
  const sdk: Artifact = {
    id: record.id,
    kind: record.kind,
    location: record.location,
    metadata: record.metadata,
    preview: record.preview,
  };
  return describeArtifact(sdk);
}

function formatTime(ms: number): string {
  if (!ms) return '—';
  return new Date(ms).toLocaleString();
}

export default function ArtifactsPage() {
  const [sessions, setSessions] = useState<AgentSessionBrief[]>([]);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [records, setRecords] = useState<AgentArtifactInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [copiedId, setCopiedId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<AgentSessionBrief[]>('list_agent_sessions')
      .then((list) => {
        if (!cancelled) setSessions(list);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const loadArtifacts = useCallback(async (id: string) => {
    setLoading(true);
    setError(null);
    setExpanded(new Set());
    try {
      const list = await invoke<AgentArtifactInfo[]>('agent_artifact_list', {
        sessionId: id,
      });
      setRecords(list);
    } catch (e) {
      setError(String(e));
      setRecords([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const selectSession = useCallback(
    (id: string) => {
      setSessionId(id);
      void loadArtifacts(id);
    },
    [loadArtifacts]
  );

  const groups = useMemo(() => groupArtifacts(records), [records]);

  const toggleHistory = useCallback((key: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const copyLocation = useCallback(async (record: AgentArtifactInfo) => {
    try {
      await navigator.clipboard.writeText(record.location);
      setCopiedId(record.id);
      setTimeout(() => setCopiedId((cur) => (cur === record.id ? null : cur)), 1500);
    } catch {
      /* 剪贴板不可用时静默（与聊天内 ArtifactList 一致） */
    }
  }, []);

  const openInEditor = useCallback(async (record: AgentArtifactInfo) => {
    // 读取当前配置的默认编辑器与活跃工作空间（原先 editorCommand 传空必然启动失败且被静默吞掉）
    const ws = useWorkspaceStore.getState();
    if (!ws.editor.command) return;
    try {
      await invoke('open_in_editor', {
        path: record.location,
        editorCommand: ws.editor.command,
        baseDir: ws.activePath || undefined,
      });
    } catch {
      /* 打开失败静默；location 可能已失效（见页面顶部说明） */
    }
  }, []);

  const activeSession = sessions.find((s) => s.id === sessionId);

  return (
    <div className="artifacts-page">
      <div className="artifacts-header">
        <div className="artifacts-title-row">
          <Link to="/settings" className="btn-back" title="返回设置">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回设置
          </Link>
          <div className="artifacts-titles">
            <h1>产物库</h1>
            <p className="artifacts-subtitle">
              Agent 会话产出的文件登记（路径为写入时刻的记录，不保证文件当前仍在）
            </p>
          </div>
        </div>
      </div>

      <div className="artifacts-body">
        <aside className="session-list">
          {sessions.length === 0 && !error ? (
            <div className="session-empty">暂无 Agent 会话</div>
          ) : (
            sessions.map((s) => (
              <button
                key={s.id}
                type="button"
                className={`session-item${s.id === sessionId ? ' is-active' : ''}`}
                onClick={() => selectSession(s.id)}
              >
                <span className="session-title">{s.title || '未命名会话'}</span>
                <span className="session-meta">
                  {s.projectName || '纯聊天'} · {formatTime(s.updatedAt)}
                </span>
              </button>
            ))
          )}
        </aside>

        <main className="artifact-detail">
          {error ? <div className="artifacts-error">{error}</div> : null}

          {!sessionId && !error ? (
            <div className="artifact-placeholder">
              <FileText size={28} strokeWidth={1.6} />
              <p>从左侧选择一个会话查看产物</p>
            </div>
          ) : null}

          {sessionId && loading ? (
            <div className="artifact-placeholder">加载中…</div>
          ) : null}

          {sessionId && !loading && groups.length === 0 && !error ? (
            <div className="artifact-placeholder">
              <FileText size={28} strokeWidth={1.6} />
              <p>该会话暂无登记的产物</p>
              <p className="artifact-hint">
                产物在工具成功产出文件时登记（写文件 / 编辑 / 文档转换等），
                接线之前的旧会话不会有记录
              </p>
            </div>
          ) : null}

          {sessionId && !loading && groups.length > 0 ? (
            <>
              <div className="artifact-context">
                {activeSession
                  ? `${activeSession.title || '未命名会话'} · ${groups.length} 个产物`
                  : `${groups.length} 个产物`}
              </div>
              <ul className="artifact-groups">
                {groups.map((group) => {
                  const described = describeRecord(group.latest);
                  return (
                    <li key={group.key} className="artifact-group">
                      <div className="artifact-group__header">
                        <span className="artifact-kind">{described.label}</span>
                        <code className="artifact-path">{group.latest.location}</code>
                        <span className="artifact-version">v{group.latest.version}</span>
                      </div>
                      <div className="artifact-group__actions">
                        <button
                          type="button"
                          onClick={() => copyLocation(group.latest)}
                          title="复制路径"
                        >
                          {copiedId === group.latest.id ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                          {copiedId === group.latest.id ? '已复制' : '复制路径'}
                        </button>
                        <button type="button" onClick={() => openInEditor(group.latest)}>
                          <ExternalLink size={14} />
                          编辑器打开
                        </button>
                        {group.history.length > 1 ? (
                          <button
                            type="button"
                            className="artifact-history-toggle"
                            onClick={() => toggleHistory(group.key)}
                          >
                            <History size={14} />
                            {expanded.has(group.key)
                              ? '收起历史'
                              : `历史 ${group.history.length} 版`}
                          </button>
                        ) : null}
                      </div>
                      {group.latest.preview ? (
                        <details className="artifact-preview">
                          <summary>预览</summary>
                          <pre>{group.latest.preview}</pre>
                        </details>
                      ) : null}
                      {expanded.has(group.key) ? (
                        <ul className="artifact-history">
                          {[...group.history].reverse().map((record) => {
                            const info = describeRecord(record);
                            return (
                              <li key={record.id} className="artifact-history__item">
                                <span className="artifact-kind">{info.label}</span>
                                <code>{record.location}</code>
                                <span>v{record.version}</span>
                                <span className="artifact-time">
                                  {formatTime(record.createdAt)}
                                </span>
                              </li>
                            );
                          })}
                        </ul>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            </>
          ) : null}
        </main>
      </div>
    </div>
  );
}
