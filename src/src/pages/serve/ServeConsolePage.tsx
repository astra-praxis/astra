/**
 * Agent 服务页（Serve 控制台入口）。
 *
 * 桌面端是 astra-serve 的 Client：本页只负责
 * - 探测本机 Serve 运行状态（serve_console_status）
 * - 拉起独立常驻的 astra-serve-host 进程（serve_console_start，可选指定程序路径）
 * - 打开独立窗口进入 Serve 自带的 Entity 管理页（serve_console_open）
 *
 * Serve 进程不随桌面端退出；窗口关闭也不影响后台 Entity 继续执行。
 */
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, Check, ExternalLink, Play, RefreshCw, Server } from 'lucide-react';
import './ServeConsolePage.scss';

interface ServeStatus {
  running: boolean;
  port: number;
}

interface ServeStartInfo {
  started: boolean;
  port: number;
  token: string;
  binPath: string;
  dataDir: string;
}

export default function ServeConsolePage({ webMode = false }: { webMode?: boolean }) {
  if (webMode) return <WebConsole />;
  const [status, setStatus] = useState<ServeStatus | null>(null);
  const [info, setInfo] = useState<ServeStartInfo | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [binPath, setBinPath] = useState('');
  const [tokenCopied, setTokenCopied] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<ServeStatus>('serve_console_status'));
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const start = async () => {
    setBusy(true);
    setError('');
    try {
      const result = await invoke<ServeStartInfo>('serve_console_start', {
        binPath: binPath.trim() || null,
        port: null,
        token: null,
      });
      setInfo(result);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const open = async () => {
    setError('');
    try {
      await invoke('serve_console_open', { port: null });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const copyToken = async () => {
    const token = info?.token;
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      setTokenCopied(true);
      window.setTimeout(() => setTokenCopied(false), 1500);
    } catch {
      // 剪贴板不可用时忽略
    }
  };

  const running = status?.running ?? false;

  return (
    <div className="serve-console">
      <header className="serve-console__head">
        <h2>
          <Server size={18} /> Agent 服务
        </h2>
        <p>
          Astra Serve 在本机后台托管多个 Entity（Agent），每个 Entity 有独立的对话、任务和记忆。
          服务独立运行：关闭本窗口或退出 Astra，已在执行的 Entity 不会中断。
        </p>
      </header>

      <section className="serve-console__card">
        <div className="serve-console__row">
          <span className={`serve-console__dot${running ? ' is-on' : ''}`} />
          <span className="serve-console__state">
            {running ? `运行中 · 端口 ${status?.port}` : '未运行'}
          </span>
          <button type="button" className="serve-console__icon-btn" onClick={() => void refresh()} title="刷新">
            <RefreshCw size={14} />
          </button>
        </div>

        <div className="serve-console__actions">
          {!running && (
            <button type="button" className="serve-console__btn primary" disabled={busy} onClick={() => void start()}>
              <Play size={14} /> {busy ? '启动中…' : '启动服务'}
            </button>
          )}
          <button type="button" className="serve-console__btn" disabled={!running} onClick={() => void open()}>
            <ExternalLink size={14} /> 打开控制台
          </button>
        </div>

        {!running && (
          <div className="serve-console__field">
            <label htmlFor="serve-bin-path">Serve 程序路径（可选，留空自动查找）</label>
            <input
              id="serve-bin-path"
              value={binPath}
              placeholder="如 /Users/you/workplace/astra-serve/target/debug/astra-serve-host"
              onChange={(e) => setBinPath(e.target.value)}
            />
            <p className="serve-console__hint">
              自动查找顺序：~/.astra/bin/astra-serve-host → 应用数据目录/bin → PATH
            </p>
          </div>
        )}
      </section>

      {info && (
        <section className="serve-console__card">
          <h3>本次启动信息</h3>
          <dl className="serve-console__meta">
            <div>
              <dt>端口</dt>
              <dd>{info.port}</dd>
            </div>
            <div>
              <dt>数据目录</dt>
              <dd>{info.dataDir}</dd>
            </div>
            <div>
              <dt>访问 token</dt>
              <dd>
                <code>{info.token}</code>
                <button type="button" className="serve-console__icon-btn" onClick={() => void copyToken()} title="复制 token">
                  {tokenCopied ? <Check size={13} /> : <Copy size={13} />}
                </button>
              </dd>
            </div>
          </dl>
          <p className="serve-console__hint">
            打开控制台后首次会要求输入 token（只需输一次，窗口会记住）。之后 Entity 创建、对话、审批都在控制台窗口里完成。
          </p>
        </section>
      )}
    </div>
  );
}

// ===== Web 模式（宿主内嵌控制台窗口）：无 Tauri IPC，直连宿主 HTTP API =====

interface WebEntity {
  entity_id: string;
  name: string;
  runtime?: { state?: string } | null;
}

const WEB_TOKEN_KEY = 'astra-serve-console-token';

/** 宿主控制台：token 解锁（一次，localStorage 记住）+ Entity 列表轮询 */
function WebConsole() {
  const port = Number(window.location.port) || 8800;
  const base = `http://127.0.0.1:${port}`;
  const [token, setToken] = useState(() => localStorage.getItem(WEB_TOKEN_KEY) ?? '');
  const [tokenInput, setTokenInput] = useState('');
  const [entities, setEntities] = useState<WebEntity[]>([]);
  const [error, setError] = useState('');

  const load = useCallback(
    async (t: string) => {
      try {
        const resp = await fetch(`${base}/api/entities`, {
          headers: { Authorization: `Bearer ${t}` },
        });
        if (resp.status === 401) {
          localStorage.removeItem(WEB_TOKEN_KEY);
          setToken('');
          setError('token 无效，请重新输入');
          return;
        }
        if (!resp.ok) {
          setError(`宿主返回 ${resp.status}`);
          return;
        }
        const data = (await resp.json()) as { entities?: WebEntity[] };
        setEntities(Array.isArray(data.entities) ? data.entities : []);
        setError('');
      } catch (e) {
        setError(`连不上宿主（${base}）：${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [base],
  );

  useEffect(() => {
    if (!token) return;
    void load(token);
    const timer = window.setInterval(() => {
      const t = localStorage.getItem(WEB_TOKEN_KEY);
      if (t) void load(t);
    }, 6000);
    return () => window.clearInterval(timer);
  }, [token, load]);

  if (!token) {
    return (
      <div className="serve-console">
        <header className="serve-console__head">
          <h2>
            <Server size={18} /> Agent 服务控制台
          </h2>
          <p>首次访问需要输入访问 token（在桌面端「Agent 服务」页可查看/复制）。</p>
        </header>
        {error && (
          <div className="serve-console__error" role="alert">
            {error}
          </div>
        )}
        <section className="serve-console__card">
          <div className="serve-console__field">
            <label htmlFor="web-console-token">访问 token</label>
            <input
              id="web-console-token"
              value={tokenInput}
              onChange={(e) => setTokenInput(e.target.value)}
              placeholder="32 位 hex"
              autoFocus
            />
          </div>
          <div className="serve-console__actions">
            <button
              type="button"
              className="serve-console__btn primary"
              disabled={!tokenInput.trim()}
              onClick={() => {
                const t = tokenInput.trim();
                localStorage.setItem(WEB_TOKEN_KEY, t);
                setToken(t);
                setError('');
              }}
            >
              <Play size={14} /> 进入控制台
            </button>
          </div>
        </section>
      </div>
    );
  }

  return (
    <div className="serve-console">
      <header className="serve-console__head">
        <h2>
          <Server size={18} /> Agent 服务控制台
        </h2>
        <p>
          宿主运行于端口 {port}。Entity 的对话与任务在桌面端 Agent 页进行
          （bot 的提问/审批会主动推送过去）；这里展示宿主托管的 Entity 概览。
        </p>
      </header>
      {error && (
        <div className="serve-console__error" role="alert">
          {error}
        </div>
      )}
      <section className="serve-console__card">
        <div className="serve-console__row">
          <span className="serve-console__dot is-on" />
          <span className="serve-console__state">已连接 · 端口 {port}</span>
          <button
            type="button"
            className="serve-console__icon-btn"
            onClick={() => void load(token)}
            title="刷新"
          >
            <RefreshCw size={14} />
          </button>
        </div>
        {entities.length === 0 ? (
          <p className="serve-console__hint">宿主还没有托管任何 Entity。在桌面端对话里让 AI 创建 bot，或用 serve_bot 工具创建。</p>
        ) : (
          <ul className="serve-console__entities">
            {entities.map((e) => (
              <li key={e.entity_id} className="serve-console__entity">
                <span className="serve-console__entity-name">{e.name || e.entity_id}</span>
                <span className="serve-console__entity-state">{e.runtime?.state || 'unknown'}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
