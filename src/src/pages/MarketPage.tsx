/**
 * 扩展市场共享页面组件（MCP / Skill 两种形态）
 * 搜索内置目录 → 卡片展示（推荐置顶）→ 一键安装
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Search, Download, Check, Star, RefreshCw, X, ExternalLink } from 'lucide-react';
import './MarketPage.scss';

interface McpMarketEntry {
  id: string;
  kind: 'mcp' | 'acp';
  name: string;
  description: string;
  tags: string[];
  official: boolean;
  recommended: boolean;
  recommendReason: string;
  command: string;
  args: string[];
  requires: string[];
  npmPackage: string | null;
  suggestedAllow: string[];
  requiresEnv: string[];
}

interface SkillMarketEntry {
  id: string;
  name: string;
  description: string;
  installs: number;
  source: string;
  skillId: string;
  pageUrl: string;
  official: boolean;
  recommended: boolean;
  recommendReason: string;
  installed: boolean;
}

interface MarketStatus {
  configuredIds: string[];
  nodeAvailable: boolean;
  gitAvailable: boolean;
}

interface InstallResult {
  ok: boolean;
  message: string;
}

type InstallState = 'idle' | 'installing' | 'installed' | 'error';

function formatInstalls(n: number): string {
  if (n >= 10000) return `${(n / 10000).toFixed(1)} 万`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

const RECO_BANNERS = {
  mcp: [
    { scene: '浏览器自动化 / 网页抓取', pick: 'Playwright' },
    { scene: '测试 Agent 链路', pick: 'Everything' },
    { scene: '把外部编码 Agent 当工具调用', pick: 'Claude Code (ACP)' },
  ],
  skill: [
    { scene: '文档交付（Word 报告/标书）', pick: 'docx' },
    { scene: '前端 / UI 任务', pick: 'frontend-design' },
    { scene: '幻灯片制作', pick: 'pptx' },
  ],
} as const;

export default function MarketPage({ kind }: { kind: 'mcp' | 'skill' }) {
  const [query, setQuery] = useState('');
  const [mcpEntries, setMcpEntries] = useState<McpMarketEntry[]>([]);
  const [skillEntries, setSkillEntries] = useState<SkillMarketEntry[]>([]);
  const [status, setStatus] = useState<MarketStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [installStates, setInstallStates] = useState<Record<string, InstallState>>({});
  const [installMessages, setInstallMessages] = useState<Record<string, string>>({});
  // skills.sh 远程条目描述懒加载缓存
  const [descMap, setDescMap] = useState<Record<string, string>>({});

  async function refresh() {
    setLoading(true);
    setError('');
    try {
      if (kind === 'mcp') {
        const [entries, st] = await Promise.all([
          invoke<McpMarketEntry[]>('market_mcp_search', { query: '' }),
          invoke<MarketStatus>('market_status'),
        ]);
        setMcpEntries(entries);
        setStatus(st);
      } else {
        const [entries, st] = await Promise.all([
          invoke<SkillMarketEntry[]>('market_skill_search', { query: '' }),
          invoke<MarketStatus>('market_status'),
        ]);
        setSkillEntries(entries);
        setStatus(st);
      }
    } catch (e) {
      setError(`加载市场目录失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 搜索（Rust 侧空格分词 AND 匹配）
  const searched = useMemo(() => {
    return async () => {
      try {
        if (kind === 'mcp') {
          setMcpEntries(
            await invoke<McpMarketEntry[]>('market_mcp_search', { query })
          );
        } else {
          setSkillEntries(
            await invoke<SkillMarketEntry[]>('market_skill_search', { query })
          );
        }
      } catch (e) {
        setError(`搜索失败: ${e}`);
      }
    };
  }, [kind, query]);

  useEffect(() => {
    const t = setTimeout(() => void searched(), 200);
    return () => clearTimeout(t);
  }, [searched]);

  // 远程条目描述懒加载：批量拉 SKILL.md frontmatter（失败静默，不占卡片）
  useEffect(() => {
    if (kind !== 'skill') return;
    const missing = skillEntries.filter((e) => !e.description && !descMap[e.id]);
    if (missing.length === 0) return;
    let cancelled = false;
    void (async () => {
      const results = await Promise.allSettled(
        missing.map((e) => invoke<string>('market_skill_detail', { id: e.id }))
      );
      if (cancelled) return;
      setDescMap((prev) => {
        const next = { ...prev };
        missing.forEach((e, i) => {
          const r = results[i];
          if (r.status === 'fulfilled' && r.value) next[e.id] = r.value;
        });
        return next;
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [skillEntries, kind, descMap]);

  async function doInstallMcp(entry: McpMarketEntry) {
    setInstallStates((p) => ({ ...p, [entry.id]: 'installing' }));
    setInstallMessages((p) => ({ ...p, [entry.id]: '' }));
    try {
      const r = await invoke<InstallResult>('market_mcp_install', { id: entry.id });
      setInstallStates((p) => ({ ...p, [entry.id]: r.ok ? 'installed' : 'error' }));
      setInstallMessages((p) => ({ ...p, [entry.id]: r.message }));
      if (r.ok) setStatus(await invoke<MarketStatus>('market_status'));
    } catch (e) {
      setInstallStates((p) => ({ ...p, [entry.id]: 'error' }));
      setInstallMessages((p) => ({ ...p, [entry.id]: String(e) }));
    }
  }

  async function doInstallSkill(entry: SkillMarketEntry) {
    setInstallStates((p) => ({ ...p, [entry.id]: 'installing' }));
    setInstallMessages((p) => ({ ...p, [entry.id]: '' }));
    try {
      const r = await invoke<InstallResult>('market_skill_install', { id: entry.id });
      setInstallStates((p) => ({ ...p, [entry.id]: r.ok ? 'installed' : 'error' }));
      setInstallMessages((p) => ({ ...p, [entry.id]: r.message }));
      if (r.ok) {
        // 重新搜索以刷新 installed 标记
        try {
          setSkillEntries(
            await invoke<SkillMarketEntry[]>('market_skill_search', { query })
          );
        } catch {
          /* 刷新失败不影响安装结果展示 */
        }
      }
    } catch (e) {
      setInstallStates((p) => ({ ...p, [entry.id]: 'error' }));
      setInstallMessages((p) => ({ ...p, [entry.id]: String(e) }));
    }
  }

  const configuredSet = new Set(status?.configuredIds ?? []);
  const banners = RECO_BANNERS[kind];
  const isMcp = kind === 'mcp';

  return (
    <div className={`market-page market-${kind}`}>
      <div className="page-header">
        <div className="header-left">
          <h1>{isMcp ? 'MCP 扩展市场' : 'Skill 技能市场'}</h1>
          <p className="subtitle">
            {isMcp
              ? '搜索并一键安装 MCP / ACP 扩展，安装后写入配置，新会话生效'
              : '从 Anthropic Skills 仓库安装技能到本地技能库，安装后通过 /技能名 调用'}
          </p>
        </div>
        <button className="btn-refresh" onClick={() => void refresh()} disabled={loading}>
          <RefreshCw size={15} strokeWidth={2.2} className={loading ? 'spinning' : ''} />
          刷新
        </button>
      </div>

      {error && (
        <div className="error-banner">
          <span>⚠️</span>
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => setError('')}>
            <X size={14} />
          </button>
        </div>
      )}

      {/* 推荐横幅 */}
      <div className="reco-banner">
        <span className="reco-title">
          <Star size={14} strokeWidth={2.2} />
          怎么选
        </span>
        {banners.map((b) => (
          <span key={b.pick} className="reco-item">
            {b.scene} → <strong>{b.pick}</strong>
          </span>
        ))}
      </div>

      {/* 依赖状态 */}
      <div className="dep-status">
        {isMcp && (
          <span className={status?.nodeAvailable ? 'dep-ok' : 'dep-missing'}>
            {status?.nodeAvailable ? '✓ Node.js 可用' : '✗ 未检测到 Node.js（MCP/ACP 运行必需）'}
          </span>
        )}
        {!isMcp && (
          <span className={status?.gitAvailable ? 'dep-ok' : 'dep-missing'}>
            {status?.gitAvailable ? '✓ git 可用' : '✗ 未检测到 git（技能安装必需）'}
          </span>
        )}
      </div>

      {/* 搜索框 */}
      <div className="market-search">
        <Search size={16} strokeWidth={2.2} />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={isMcp ? '搜索扩展，如 浏览器 github' : '搜索技能，如 文档 前端'}
        />
      </div>

      {loading ? (
        <div className="loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      ) : (
        <div className="market-grid">
          {isMcp &&
            mcpEntries.map((e) => {
              const configured = configuredSet.has(e.id);
              const st = installStates[e.id] ?? 'idle';
              return (
                <div key={e.id} className={`market-card${e.recommended ? ' recommended' : ''}`}>
                  <div className="card-head">
                    <h3 className="card-name">{e.name}</h3>
                    {e.recommended && <span className="badge-reco">推荐</span>}
                    {e.official && <span className="badge-official">官方</span>}
                    <span className={`badge-kind kind-${e.kind}`}>{e.kind.toUpperCase()}</span>
                  </div>
                  <p className="card-desc">{e.description}</p>
                  <div className="card-tags">
                    {e.tags.map((t) => (
                      <span key={t} className="tag">
                        {t}
                      </span>
                    ))}
                  </div>
                  {e.recommended && (
                    <div className="reco-reason" title={e.recommendReason}>
                      💡 {e.recommendReason}
                    </div>
                  )}
                  <div className="card-meta">
                    <code className="cmd" title={`${e.command} ${e.args.join(' ')}`}>
                      {e.command} {e.args.join(' ')}
                    </code>
                  </div>
                  {e.requiresEnv.length > 0 && (
                    <div className="env-hint">
                      需要预先导出环境变量: <code>{e.requiresEnv.join(', ')}</code>
                    </div>
                  )}
                  {e.requires.length > 0 && (
                    <div className="env-hint">运行依赖: {e.requires.join(' / ')}</div>
                  )}
                  {configured ? (
                    <div className="install-done">✓ 已配置</div>
                  ) : (
                    <button
                      className={`btn-install${st === 'error' ? ' has-error' : ''}`}
                      disabled={st === 'installing'}
                      onClick={() => void doInstallMcp(e)}
                    >
                      {st === 'installing' ? (
                        <>
                          <div className="spinner-small" />
                          安装中...
                        </>
                      ) : st === 'installed' ? (
                        <>
                          <Check size={14} strokeWidth={2.5} />
                          已安装
                        </>
                      ) : (
                        <>
                          <Download size={14} strokeWidth={2.5} />
                          安装
                        </>
                      )}
                    </button>
                  )}
                  {installMessages[e.id] && st !== 'installing' && (
                    <div className={`install-msg${st === 'error' ? ' is-error' : ''}`}>
                      {installMessages[e.id]}
                    </div>
                  )}
                </div>
              );
            })}

          {!isMcp &&
            skillEntries.map((e) => {
              const st = installStates[e.id] ?? 'idle';
              const desc = e.description || descMap[e.id] || '';
              const installed = e.installed || st === 'installed';
              return (
                <div key={e.id} className={`market-card${e.recommended ? ' recommended' : ''}`}>
                  <div className="card-head">
                    <h3 className="card-name">/{e.name}</h3>
                    {e.recommended && <span className="badge-reco">推荐</span>}
                    {e.official && <span className="badge-official">官方</span>}
                  </div>
                  <p className={`card-desc${desc ? '' : ' is-loading'}`}>
                    {desc || '描述加载中…'}
                  </p>
                  <div className="card-tags">
                    <span className="tag" title={`安装量 ${e.installs}`}>
                      ⬇ {formatInstalls(e.installs)}
                    </span>
                    <span className="tag">{e.source}</span>
                  </div>
                  {e.recommended && (
                    <div className="reco-reason" title={e.recommendReason}>
                      💡 {e.recommendReason}
                    </div>
                  )}
                  <div className="card-meta">
                    <code className="cmd" title={`安装后用 /${e.skillId} 调用`}>
                      /{e.skillId}
                    </code>
                  </div>
                  <div className="card-footer">
                    {installed ? (
                      <div className="install-done">✓ 已安装</div>
                    ) : (
                      <button
                        className={`btn-install${st === 'error' ? ' has-error' : ''}`}
                        disabled={st === 'installing'}
                        onClick={() => void doInstallSkill(e)}
                      >
                        {st === 'installing' ? (
                          <>
                            <div className="spinner-small" />
                            安装中...
                          </>
                        ) : (
                          <>
                            <Download size={14} strokeWidth={2.5} />
                            安装
                          </>
                        )}
                      </button>
                    )}
                    <button
                      className="btn-page"
                      onClick={() => void openUrl(e.pageUrl)}
                      title="在 skills.sh 查看详情"
                    >
                      <ExternalLink size={13} strokeWidth={2} />
                      详情页
                    </button>
                  </div>
                  {installMessages[e.id] && st !== 'installing' && (
                    <div className={`install-msg${st === 'error' ? ' is-error' : ''}`}>
                      {installMessages[e.id]}
                    </div>
                  )}
                </div>
              );
            })}

          {!isMcp && skillEntries.length === 0 && (
            <div className="empty-state">
              <span className="empty-icon">🔍</span>
              <h3>没有匹配的技能</h3>
              <p>换个关键词试试</p>
            </div>
          )}
          {isMcp && mcpEntries.length === 0 && (
            <div className="empty-state">
              <span className="empty-icon">🔍</span>
              <h3>没有匹配的扩展</h3>
              <p>换个关键词试试</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
