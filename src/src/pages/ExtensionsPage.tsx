/**
 * 扩展管理页：查看 / 启停 / 编辑 / 删除已配置的 MCP Server 与 ACP Agent
 *
 * 配置本体在 astra.toml（[mcp.servers.*] / [acp.agents.*]），
 * 本页是它的可视化管理面；所有修改仅对新会话生效（工具集合建 Thread 时冻结）。
 */
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router';
import {
  Blocks,
  Pencil,
  Trash2,
  ExternalLink,
  RefreshCw,
  FileWarning,
  ToggleLeft,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import './ExtensionsPage.scss';

interface McpExtensionInfo {
  id: string;
  enabled: boolean;
  transport: string;
  command: string;
  args: string[];
  env: string[];
  required: boolean;
  allow: string[];
  deny: string[];
  callTimeoutSecs: number;
}

interface AcpExtensionInfo {
  id: string;
  enabled: boolean;
  command: string;
  args: string[];
  env: string[];
  required: boolean;
  permission: string;
  sessionPolicy: string;
}

interface ExtensionsList {
  mcp: McpExtensionInfo[];
  acp: AcpExtensionInfo[];
  configPath: string;
  parseError: string | null;
}

type Kind = 'mcp' | 'acp';

interface EditTarget {
  kind: Kind;
  id: string;
  command: string;
  argsText: string;
  envText: string;
  required: boolean;
}

interface DeleteTarget {
  kind: Kind;
  id: string;
}

interface McpRuntimeStatus {
  id: string;
  status: 'stopped' | 'starting' | 'ready' | 'failed';
  toolCount: number;
  tools: string[];
  error: string | null;
  attempts: number;
}

interface AcpRuntimeStatus {
  id: string;
  status: 'stopped' | 'starting' | 'ready' | 'failed';
  tool: string | null;
  error: string | null;
  attempts: number;
}

interface SessionRuntimeStatus {
  sessionId: string;
  mcp: McpRuntimeStatus[];
  acp: AcpRuntimeStatus[];
}

const STATUS_LABEL: Record<string, string> = {
  ready: '运行中',
  starting: '加载中',
  failed: '加载失败',
  stopped: '未加载',
};

function statusLabel(s: string): string {
  return STATUS_LABEL[s] ?? s;
}

function kindLabel(kind: Kind): string {
  return kind === 'mcp' ? 'MCP' : 'ACP';
}

export default function ExtensionsPage() {
  const [list, setList] = useState<ExtensionsList | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [edit, setEdit] = useState<EditTarget | null>(null);
  const [deleting, setDeleting] = useState<DeleteTarget | null>(null);
  const [runtime, setRuntime] = useState<SessionRuntimeStatus[] | null>(null);

  const refreshRuntime = useCallback(async () => {
    try {
      const r = await invoke<SessionRuntimeStatus[]>('extensions_runtime_status');
      setRuntime(r);
    } catch {
      // 状态查询失败不阻塞配置管理（旧版本后端可能没有该命令）
      setRuntime(null);
    }
  }, []);

  const refresh = useCallback(async () => {
    try {
      const r = await invoke<ExtensionsList>('extensions_list');
      setList(r);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    void refreshRuntime();
    // 会话扩展在建新会话时变化：停留页面时低频轮询，切回来时也能看到最新状态
    const timer = setInterval(() => void refreshRuntime(), 5000);
    return () => clearInterval(timer);
  }, [refresh, refreshRuntime]);

  const toggleEnabled = async (kind: Kind, id: string, enabled: boolean) => {
    const key = `${kind}/${id}`;
    setBusyId(key);
    try {
      await invoke('extensions_set_enabled', { kind, id, enabled });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  };

  const doRemove = async () => {
    if (!deleting) return;
    const key = `${deleting.kind}/${deleting.id}`;
    setBusyId(key);
    try {
      await invoke('extensions_remove', { kind: deleting.kind, id: deleting.id });
      setDeleting(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  };

  const doUpdate = async () => {
    if (!edit) return;
    const key = `${edit.kind}/${edit.id}`;
    setBusyId(key);
    try {
      const args = edit.argsText
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean);
      const env = edit.envText
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean);
      await invoke('extensions_update', {
        kind: edit.kind,
        id: edit.id,
        command: edit.command.trim(),
        args,
        env,
        required: edit.required,
      });
      setEdit(null);
      await refresh();
    } catch (e) {
      setError(String(e));
      // 保存失败时保留弹窗内容让用户改
    } finally {
      setBusyId(null);
    }
  };

  const openEdit = (kind: Kind, info: McpExtensionInfo | AcpExtensionInfo) => {
    setEdit({
      kind,
      id: info.id,
      command: info.command,
      argsText: info.args.join('\n'),
      envText: info.env.join('\n'),
      required: info.required,
    });
  };

  const renderRow = (kind: Kind, info: McpExtensionInfo | AcpExtensionInfo) => {
    const mcp = kind === 'mcp' ? (info as McpExtensionInfo) : null;
    const key = `${kind}/${info.id}`;
    const busy = busyId === key;
    return (
      <div key={key} className={`ext-card${info.enabled ? '' : ' disabled'}`}>
        <div className="ext-head">
          <span className={`kind-badge ${kind}`}>{kindLabel(kind)}</span>
          <span className="ext-id" title={info.id}>
            {info.id}
          </span>
          {info.required && <span className="badge required">必需</span>}
          {!info.enabled && <span className="badge off">已停用</span>}
          <div className="spacer" />
          <label className="switch" title={info.enabled ? '停用' : '启用'}>
            <input
              type="checkbox"
              checked={info.enabled}
              disabled={busy}
              onChange={(e) => void toggleEnabled(kind, info.id, e.target.checked)}
            />
            <span className="slider" />
          </label>
        </div>
        <code className="ext-cmd">
          {info.command} {info.args.join(' ')}
        </code>
        {info.env.length > 0 && (
          <div className="ext-env">
            环境变量：
            {info.env.map((n) => (
              <span key={n} className="env-chip">
                {n}
              </span>
            ))}
          </div>
        )}
        <div className="ext-meta">
          {mcp ? (
            <>
              <span>超时 {mcp.callTimeoutSecs}s</span>
              {mcp.allow.length > 0 && <span>允许 {mcp.allow.length} 个工具</span>}
              {mcp.deny.length > 0 && <span>禁止 {mcp.deny.length} 个工具</span>}
              {mcp.allow.length === 0 && mcp.deny.length === 0 && (
                <span>权限：默认确认</span>
              )}
            </>
          ) : (
            <>
              <span>权限：{(info as AcpExtensionInfo).permission}</span>
              <span>会话：{(info as AcpExtensionInfo).sessionPolicy}</span>
            </>
          )}
        </div>
        <div className="ext-actions">
          <button className="icon-btn" title="编辑" onClick={() => openEdit(kind, info)}>
            <Pencil size={15} />
          </button>
          <button
            className="icon-btn danger"
            title="删除"
            onClick={() => setDeleting({ kind, id: info.id })}
          >
            <Trash2 size={15} />
          </button>
        </div>
      </div>
    );
  };

  return (
    <div className="extensions-page">
      <div className="page-head">
        <div className="head-text">
          <h1>
            <Blocks size={22} /> 扩展管理
          </h1>
          <p>已配置的 MCP Server 与 ACP Agent。修改后对新建会话生效，运行中会话保持原工具集合。</p>
        </div>
        <div className="head-actions">
          <Link className="btn-secondary" to="/market/mcp">
            <ExternalLink size={15} />
            去市场安装
          </Link>
          <button className="btn-secondary" onClick={() => void refresh()}>
            <RefreshCw size={15} className={loading ? 'spin' : ''} />
            刷新
          </button>
        </div>
      </div>

      {/* 运行状态：配置里「启用」≠ 实际加载成功；这里反映当前活跃会话的真实状态 */}
      <div className="section">
        <div className="section-title">
          运行状态
          <span className="count">{runtime?.length ?? 0}</span>
          <span className="runtime-hint">活跃会话中实际加载的扩展（5s 自动刷新）</span>
        </div>
        {runtime === null ? (
          <div className="runtime-empty">状态不可用</div>
        ) : runtime.length === 0 ? (
          <div className="runtime-empty">
            暂无活跃会话 —— 扩展在新建会话时加载，启动一个会话后这里会显示加载状态
          </div>
        ) : (
          runtime.map((session) => (
            <div key={session.sessionId} className="runtime-session">
              <div className="runtime-session-id" title={session.sessionId}>
                {session.sessionId}
              </div>
              {session.mcp.length === 0 && session.acp.length === 0 && (
                <div className="runtime-empty">该会话未配置扩展</div>
              )}
              {session.mcp.map((m) => (
                <div key={m.id} className="runtime-row">
                  <span className="kind-badge mcp">MCP</span>
                  <span className="runtime-name">{m.id}</span>
                  <span className={`status-badge ${m.status}`}>{statusLabel(m.status)}</span>
                  {m.toolCount > 0 && <span className="runtime-meta">{m.toolCount} 个工具</span>}
                  {m.attempts > 1 && <span className="runtime-meta">重试 {m.attempts - 1} 次</span>}
                  {m.error && <span className="runtime-error" title={m.error}>{m.error}</span>}
                </div>
              ))}
              {session.acp.map((a) => (
                <div key={a.id} className="runtime-row">
                  <span className="kind-badge acp">ACP</span>
                  <span className="runtime-name">{a.id}</span>
                  <span className={`status-badge ${a.status}`}>{statusLabel(a.status)}</span>
                  {a.tool && <span className="runtime-meta">{a.tool}</span>}
                  {a.attempts > 1 && <span className="runtime-meta">重试 {a.attempts - 1} 次</span>}
                  {a.error && <span className="runtime-error" title={a.error}>{a.error}</span>}
                </div>
              ))}
            </div>
          ))
        )}
      </div>

      {error && (
        <div className="error-banner">
          <FileWarning size={16} />
          {error}
        </div>
      )}
      {list?.parseError && (
        <div className="error-banner">
          <FileWarning size={16} />
          配置文件解析失败（检查后仍可编辑其他段）：
          <code>{list.parseError}</code>
        </div>
      )}

      {loading ? (
        <div className="loading">加载中…</div>
      ) : (
        <>
          <div className="section">
            <div className="section-title">
              MCP Servers
              <span className="count">{list?.mcp.length ?? 0}</span>
            </div>
            {list && list.mcp.length > 0 ? (
              <div className="ext-list">{list.mcp.map((m) => renderRow('mcp', m))}</div>
            ) : (
              <div className="empty">
                <ToggleLeft size={28} />
                暂无 MCP 配置，可到
                <Link to="/market/mcp">扩展市场</Link>
                安装
              </div>
            )}
          </div>

          <div className="section">
            <div className="section-title">
              ACP Agents
              <span className="count">{list?.acp.length ?? 0}</span>
            </div>
            {list && list.acp.length > 0 ? (
              <div className="ext-list">{list.acp.map((a) => renderRow('acp', a))}</div>
            ) : (
              <div className="empty">
                <ToggleLeft size={28} />
                暂无 ACP Agent 配置
              </div>
            )}
          </div>

          {list?.configPath && <div className="config-path">配置文件：{list.configPath}</div>}
        </>
      )}

      {/* 编辑弹窗 */}
      <Modal
        open={edit !== null}
        onClose={() => setEdit(null)}
        title={edit ? `编辑 ${kindLabel(edit.kind)}：${edit.id}` : ''}
        closeOnOverlay={false}
        footer={
          <button className="ext-btn-primary" disabled={busyId !== null} onClick={() => void doUpdate()}>
            {busyId !== null ? '保存中…' : '保存'}
          </button>
        }
      >
        {edit && (
          <div className="ext-edit-form">
            <label>
              启动命令
              <input
                value={edit.command}
                onChange={(e) => setEdit({ ...edit, command: e.target.value })}
                placeholder="如 npx / claude"
              />
            </label>
            <label>
              参数（每行一个）
              <textarea
                rows={4}
                value={edit.argsText}
                onChange={(e) => setEdit({ ...edit, argsText: e.target.value })}
                placeholder={'-y\n@playwright/mcp'}
              />
            </label>
            <label>
              环境变量名（每行一个，值需在宿主环境导出）
              <textarea
                rows={3}
                value={edit.envText}
                onChange={(e) => setEdit({ ...edit, envText: e.target.value })}
                placeholder={'SMITHERY_API_KEY'}
              />
            </label>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={edit.required}
                onChange={(e) => setEdit({ ...edit, required: e.target.checked })}
              />
              必需扩展（启动失败时阻塞会话而非降级）
            </label>
          </div>
        )}
      </Modal>

      {/* 删除确认弹窗 */}
      <Modal
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        title="确认删除"
        variant="confirm"
        footer={
          <button className="btn-danger" disabled={busyId !== null} onClick={() => void doRemove()}>
            <Trash2 size={16} strokeWidth={2.5} />
            确认删除
          </button>
        }
      >
        {deleting && (
          <>
            <p className="confirm-text">
              确定要删除 {kindLabel(deleting.kind)} 扩展 <strong>{deleting.id}</strong> 的配置吗？
            </p>
            <p className="confirm-hint">仅删除 astra.toml 中的配置段，不会卸载外部程序本身。</p>
          </>
        )}
      </Modal>
    </div>
  );
}
