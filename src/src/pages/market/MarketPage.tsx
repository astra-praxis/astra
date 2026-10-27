/**
 * 扩展/技能共享页面组件
 * - MCP：展示已安装（config.toml 已配置）的 MCP / ACP 扩展，
 *   支持启停 / 删除 / 运行状态标记；新扩展不在页面安装，
 *   由用户在 Agent 对话中让 AI 下载并写好配置
 * - Skill：展示本地技能库已安装的技能；新技能同样回 Agent 对话安装
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigate } from 'react-router';
import {
  RefreshCw,
  X,
  MessageSquarePlus,
  FileWarning,
  CheckCircle2,
  Trash2,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import './MarketPage.scss';

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

// 后端 UserSkillMeta 序列化为 snake_case
interface UserSkillMeta {
  name: string;
  display_name: string;
  description: string;
  path: string;
  file_count: number;
  /** 目录内存在 SKILL.md（/调用 与 read_skill 都依赖它） */
  available: boolean;
}

// 后端 market_status 序列化为 camelCase；路径为账户感知的实际路径
// （账户模式：accounts/<id>/astra-home/；legacy：~/.astra 与 ~/.config/astra）
interface MarketStatus {
  nodeAvailable: boolean;
  configPath: string;
  skillsDir: string;
}

type Kind = 'mcp' | 'acp';
type DeleteKind = Kind | 'skill';

interface DeleteTarget {
  kind: DeleteKind;
  id: string;
}

interface McpRuntimeStatus {
  id: string;
  status: 'stopped' | 'starting' | 'ready' | 'failed';
  toolCount: number;
  error: string | null;
  attempts: number;
}

interface AcpRuntimeStatus {
  id: string;
  status: 'stopped' | 'starting' | 'ready' | 'failed';
  error: string | null;
  attempts: number;
}

interface SessionRuntimeStatus {
  sessionId: string;
  mcp: McpRuntimeStatus[];
  acp: AcpRuntimeStatus[];
}

function kindLabel(kind: DeleteKind): string {
  if (kind === 'mcp') return 'MCP 扩展';
  if (kind === 'acp') return 'ACP Agent';
  return '技能';
}

// 会话间聚合：同一扩展在多个会话里状态可能不同，取"最好"的状态展示
// （任一会话 ready 即视为可用；全部 failed 才报失败并带出错误信息）
const STATUS_RANK: Record<string, number> = {
  stopped: 0,
  starting: 1,
  failed: 2,
  ready: 3,
};

export default function MarketPage({ kind }: { kind: 'mcp' | 'skill' }) {
  const navigate = useNavigate();
  const [status, setStatus] = useState<MarketStatus | null>(null);
  // MCP：已安装（config.toml 已配置）扩展列表
  const [installed, setInstalled] = useState<ExtensionsList | null>(null);
  // Skill：本地技能库已安装技能
  const [skills, setSkills] = useState<UserSkillMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [busyId, setBusyId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<DeleteTarget | null>(null);
  // 活跃会话中扩展的真实加载状态（5s 轮询）
  const [runtime, setRuntime] = useState<SessionRuntimeStatus[] | null>(null);
  // 开关/删除后"变更已应用 / 稍后生效"的提示
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      if (kind === 'mcp') {
        const [list, st] = await Promise.all([
          invoke<ExtensionsList>('extensions_list'),
          invoke<MarketStatus>('market_status'),
        ]);
        setInstalled(list);
        setStatus(st);
      } else {
        setSkills(await invoke<UserSkillMeta[]>('list_user_skills'));
      }
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [kind]);

  const refreshRuntime = useCallback(async () => {
    try {
      setRuntime(await invoke<SessionRuntimeStatus[]>('extensions_runtime_status'));
    } catch {
      // 状态查询失败不阻塞配置展示（旧版本后端可能没有该命令）
      setRuntime(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
    if (kind === 'mcp') {
      void refreshRuntime();
      // 会话扩展在新建会话时变化：低频轮询，切回来时也能看到最新状态
      const timer = setInterval(() => void refreshRuntime(), 5000);
      return () => clearInterval(timer);
    }
  }, [refresh, refreshRuntime, kind]);

  // 开关/删除后自动让变更生效：卸载所有空闲会话的内存 Thread，
  // 下次对话按新配置重新装配。运行中的会话拿不到锁 → 提示稍后生效。
  const applyToSessions = useCallback(async () => {
    try {
      const r = await invoke<{ unloaded: number; skipped: string[] }>(
        'agent_unload_all_sessions',
      );
      if (r.skipped.length > 0) {
        setNotice(
          `${r.skipped.length} 个会话未能立即重载（未落盘或被占用），其下次对话时生效`,
        );
      } else {
        setNotice(
          r.unloaded > 0 ? `已应用：${r.unloaded} 个会话将在下次对话时使用新配置` : null,
        );
      }
    } catch {
      // Agent 正在运行：拿不到 threads 锁，改为提示
      setNotice('当前有会话正在运行，变更将在其下次对话时自动生效');
    }
    void refreshRuntime();
  }, [refreshRuntime]);

  async function toggleEnabled(t: Kind, id: string, enabled: boolean) {
    const key = `${t}/${id}`;
    setBusyId(key);
    try {
      await invoke('extensions_set_enabled', { kind: t, id, enabled });
      await refresh();
      await applyToSessions();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function doRemove() {
    if (!deleting) return;
    const key = `${deleting.kind}/${deleting.id}`;
    setBusyId(key);
    try {
      if (deleting.kind === 'skill') {
        await invoke('delete_user_skill', { name: deleting.id });
      } else {
        await invoke('extensions_remove', { kind: deleting.kind, id: deleting.id });
      }
      setDeleting(null);
      await refresh();
      if (deleting.kind !== 'skill') await applyToSessions();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  }

  // 按扩展聚合运行状态：key = "mcp/<id>" | "acp/<id>"
  const runtimeMap = useMemo(() => {
    const map = new Map<string, { status: string; toolCount: number; error: string | null }>();
    for (const session of runtime ?? []) {
      for (const m of session.mcp) {
        const key = `mcp/${m.id}`;
        const prev = map.get(key);
        if (!prev || STATUS_RANK[m.status] > STATUS_RANK[prev.status]) {
          map.set(key, { status: m.status, toolCount: m.toolCount, error: m.error });
        }
      }
      for (const a of session.acp) {
        const key = `acp/${a.id}`;
        const prev = map.get(key);
        if (!prev || STATUS_RANK[a.status] > STATUS_RANK[prev.status]) {
          map.set(key, { status: a.status, toolCount: 0, error: a.error });
        }
      }
    }
    return map;
  }, [runtime]);

  const installedMcpList = installed?.mcp ?? [];
  const installedAcpList = installed?.acp ?? [];
  const installedTotal = installedMcpList.length + installedAcpList.length;

  // 跳到 Agent 对话安装新扩展：导航
  function goAgentInstall() {
    navigate('/agent');
  }

  // 卡片上的可用状态：小圆点（绿 = 可用，红 = 不可用，灰 = 未加载）。
  // 按需求不区分「加载失败」细节、不展示报错文本——用户只关心可用与否；
  // 具体错误仍可从 hover 原文与后端日志排查。
  function renderStatus(t: Kind, id: string) {
    const entry = runtimeMap.get(`${t}/${id}`);
    if (!entry) return null;
    const dotClass =
      entry.status === 'ready' || entry.status === 'starting' ? 'is-ok' : 'is-down';
    const tip =
      entry.status === 'ready'
        ? `可用${entry.toolCount > 0 ? ` · ${entry.toolCount} 个工具` : ''}`
        : entry.status === 'starting'
          ? '加载中'
          : '未加载（新会话自动重试）';
    return (
      <span className={`status-dot ${dotClass}`} title={tip} />
    );
  }

  return (
    <div className={`market-page market-${kind}`}>
      <div className="page-header">
        <div className="header-left">
          <h1>{kind === 'mcp' ? 'MCP 扩展' : 'Skill 技能'}</h1>
          <p className="subtitle">
            {kind === 'mcp'
              ? '已安装的 MCP / ACP 扩展；安装新扩展请到 Agent 对话中让 AI 下载并配置'
              : '已安装到本地技能库的技能，在 Agent 对话中用 /技能名 调用；安装新技能回 Agent 对话让 AI 下载'}
          </p>
        </div>
        <button className="btn-refresh" onClick={() => void refresh()} disabled={loading}>
          <RefreshCw size={15} strokeWidth={2.2} className={loading ? 'spinning' : ''} />
          刷新
        </button>
      </div>

      {error && (
        <div className="error-banner">
          <FileWarning size={15} />
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => setError('')}>
            <X size={14} />
          </button>
        </div>
      )}
      {notice && (
        <div className="info-banner">
          <CheckCircle2 size={15} />
          <span>{notice}</span>
          <button className="error-dismiss" onClick={() => setNotice(null)}>
            <X size={14} />
          </button>
        </div>
      )}

      {/* 依赖状态（仅 MCP 页） */}
      {kind === 'mcp' && (
        <div className="dep-status">
          <span className={status?.nodeAvailable ? 'dep-ok' : 'dep-missing'}>
            {status?.nodeAvailable ? '✓ Node.js 可用' : '✗ 未检测到 Node.js（MCP/ACP 运行必需）'}
          </span>
        </div>
      )}

      {loading ? (
        <div className="loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      ) : (
        <div
          className={`market-grid${
            kind === 'mcp'
              ? installedTotal === 0 && !installed?.parseError
                ? ' is-empty'
                : ''
              : skills.length === 0
                ? ' is-empty'
                : ''
          }`}
        >
          {kind === 'mcp' ? (
            <>
              {/* 安装引导：新扩展回 Agent 对话安装（空状态时隐藏，避免与空状态引导重复） */}
              {!(installedTotal === 0 && !installed?.parseError) && (
                <div className="mcp-guide">
                  <div className="mcp-guide-text">
                    <span className="mcp-guide-title">
                      <MessageSquarePlus size={15} strokeWidth={2.2} />
                      想安装新扩展？
                    </span>
                    <span className="mcp-guide-desc">
                      回到 Agent 对话直接说，例如：「帮我安装 Playwright 浏览器 MCP」。
                    </span>
                  </div>
                  <button className="btn-install" onClick={goAgentInstall}>
                    <MessageSquarePlus size={14} strokeWidth={2.5} />
                    去 Agent 对话安装
                  </button>
                </div>
              )}

              {/* 配置解析失败（坏配置也要露出，不能静默空白） */}
              {installed?.parseError && (
                <div className="error-banner">
                  <FileWarning size={15} />
                  <span>配置解析失败：{installed.parseError}</span>
                </div>
              )}

              {installedMcpList.map((e) => {
                const busy = busyId === `mcp/${e.id}`;
                return (
                  <div
                    key={`mcp:${e.id}`}
                    className={`market-card${e.enabled ? '' : ' is-disabled'}`}
                  >
                    <div className="card-head">
                      <h3 className="card-name">{e.id}</h3>
                      <span className="badge-kind kind-mcp">MCP</span>
                      {e.required && <span className="badge-kind kind-required">必需</span>}
                      <span className="spacer" />
                      <button
                        className="icon-btn danger"
                        title="删除"
                        disabled={busy}
                        onClick={() => setDeleting({ kind: 'mcp', id: e.id })}
                      >
                        <Trash2 size={14} />
                      </button>
                      <label className="switch" title={e.enabled ? '停用' : '启用'}>
                        <input
                          type="checkbox"
                          checked={e.enabled}
                          disabled={busy}
                          onChange={(ev) => void toggleEnabled('mcp', e.id, ev.target.checked)}
                        />
                        <span className="slider" />
                      </label>
                    </div>
                    <div className="card-meta">
                      <code className="cmd" title={`${e.command} ${e.args.join(' ')}`}>
                        {e.command} {e.args.join(' ')}
                      </code>
                    </div>
                    {e.env.length > 0 && (
                      <div className="env-hint">
                        需要环境变量: <code>{e.env.join(', ')}</code>
                      </div>
                    )}
                    {e.allow.length > 0 && (
                      <div className="env-hint" title={e.allow.join(', ')}>
                        已放行 {e.allow.length} 个工具
                      </div>
                    )}
                    {renderStatus('mcp', e.id)}
                  </div>
                );
              })}

              {installedAcpList.map((e) => {
                const busy = busyId === `acp/${e.id}`;
                return (
                  <div
                    key={`acp:${e.id}`}
                    className={`market-card${e.enabled ? '' : ' is-disabled'}`}
                  >
                    <div className="card-head">
                      <h3 className="card-name">{e.id}</h3>
                      <span className="badge-kind kind-acp">ACP</span>
                      {e.required && <span className="badge-kind kind-required">必需</span>}
                      <span className="spacer" />
                      <button
                        className="icon-btn danger"
                        title="删除"
                        disabled={busy}
                        onClick={() => setDeleting({ kind: 'acp', id: e.id })}
                      >
                        <Trash2 size={14} />
                      </button>
                      <label className="switch" title={e.enabled ? '停用' : '启用'}>
                        <input
                          type="checkbox"
                          checked={e.enabled}
                          disabled={busy}
                          onChange={(ev) => void toggleEnabled('acp', e.id, ev.target.checked)}
                        />
                        <span className="slider" />
                      </label>
                    </div>
                    <div className="card-meta">
                      <code className="cmd" title={`${e.command} ${e.args.join(' ')}`}>
                        {e.command} {e.args.join(' ')}
                      </code>
                    </div>
                    <div className="env-hint">
                      权限模式: {e.permission} · 会话策略: {e.sessionPolicy}
                    </div>
                    {renderStatus('acp', e.id)}
                  </div>
                );
              })}

              {installedTotal === 0 && !installed?.parseError && (
                <div className="empty-state">
                  <span className="empty-icon">🧩</span>
                  <h3>还没有已安装的扩展</h3>
                  <p>
                    回到 Agent 对话说「帮我安装 xxx MCP」,
                    AI 会下载依赖并写好 {installed?.configPath || 'config.toml'} 配置，装完这里就能看到。
                  </p>
                  <button className="btn-install" onClick={goAgentInstall}>
                    <MessageSquarePlus size={14} strokeWidth={2.5} />
                    去 Agent 对话安装
                  </button>
                </div>
              )}
            </>
          ) : (
            <>
              {/* 安装引导：新技能回 Agent 对话安装（空状态时隐藏，避免与空状态引导重复） */}
              {skills.length > 0 && (
                <div className="mcp-guide">
                  <div className="mcp-guide-text">
                    <span className="mcp-guide-title">
                      <MessageSquarePlus size={15} strokeWidth={2.2} />
                      想安装新技能？
                    </span>
                    <span className="mcp-guide-desc">
                      回到 Agent 对话直接说，例如：「帮我安装 docx 技能」。
                    </span>
                  </div>
                  <button className="btn-install" onClick={goAgentInstall}>
                    <MessageSquarePlus size={14} strokeWidth={2.5} />
                    去 Agent 对话安装
                  </button>
                </div>
              )}

              {skills.map((s) => {
                const busy = busyId === `skill/${s.name}`;
                const displayName = s.display_name;
                // 缺 SKILL.md 的技能不可用，也不进详情
                const clickable = s.available;
                return (
                  <div
                    key={s.name}
                    className={`market-card${clickable ? ' is-clickable' : ''}`}
                    onClick={clickable ? () => navigate(`/skills/${s.name}`) : undefined}
                    role={clickable ? 'button' : undefined}
                    tabIndex={clickable ? 0 : undefined}
                    onKeyDown={
                      clickable
                        ? (e) => e.key === 'Enter' && navigate(`/skills/${s.name}`)
                        : undefined
                    }
                  >
                    <div className="card-head">
                      <h3 className="card-name" title={s.name}>
                        /{s.name}
                      </h3>
                      {s.available ? (
                        <span className="badge-kind kind-on">可用</span>
                      ) : (
                        <span className="badge-kind kind-off">不可用</span>
                      )}
                      {displayName !== s.name && (
                        <span className="badge-kind kind-off" title={displayName}>
                          {displayName}
                        </span>
                      )}
                      <span className="spacer" />
                      <button
                        className="icon-btn danger"
                        title="删除"
                        disabled={busy}
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleting({ kind: 'skill', id: s.name });
                        }}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                    {s.description && <p className="card-desc">{s.description}</p>}
                    <div className="card-tags">
                      {!s.available && (
                        <span className="tag tag-warn">缺少 SKILL.md，/调用不可用</span>
                      )}
                      {clickable && <span className="tag">点击查看文档</span>}
                    </div>
                  </div>
                );
              })}

              {skills.length === 0 && (
                <div className="empty-state">
                  <span className="empty-icon">🎓</span>
                  <h3>还没有已安装的技能</h3>
                  <p>
                    回到 Agent 对话说「帮我安装 xxx 技能」,
                    AI 会下载并装进技能库目录（{status?.skillsDir || '见技能详情'}），装完这里就能看到。
                  </p>
                  <button className="btn-install" onClick={goAgentInstall}>
                    <MessageSquarePlus size={14} strokeWidth={2.5} />
                    去 Agent 对话安装
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* 删除确认弹窗（Modal 走 Portal 渲染在 body 下，确认样式须全局类） */}
      <Modal
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        title="确认删除"
        variant="confirm"
        footer={
          <button
            className="market-btn-danger"
            disabled={busyId !== null}
            onClick={() => void doRemove()}
          >
            <Trash2 size={15} strokeWidth={2.5} />
            确认删除
          </button>
        }
      >
        {deleting && (
          <>
            <p className="market-confirm-text">
              确定要删除 {kindLabel(deleting.kind)} <strong>{deleting.id}</strong>{' '}
              {deleting.kind === 'skill' ? '及其全部文件吗？' : '的配置吗？'}
            </p>
            <p className="market-confirm-hint">
              {deleting.kind === 'skill'
                ? `将删除技能库目录（${status?.skillsDir || '见技能详情'}）下的整个技能目录，不可恢复。`
                : `仅删除 ${installed?.configPath || 'config.toml'} 中的配置段，不会卸载外部程序本身。`}
            </p>
          </>
        )}
      </Modal>
    </div>
  );
}
