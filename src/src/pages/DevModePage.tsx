/**
 * 开发模式页面
 * 移植自 src/routes/dev-mode/[encoded]/+page.svelte
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ChevronLeft, Zap, Star, RefreshCw, Square } from 'lucide-react';
import { logs } from '@/stores/logs';
import { useFavoriteStore } from '@/stores/favorite';
import { formatClockSec } from '@/lib/format';
import Modal from '@/components/ui/Modal';
import './DevModePage.scss';

interface DevSubDir {
  label: string;
  key: string;
  work_dir: string;
  has_package_json: boolean;
}

interface DevCardInfo {
  display_name: string;
  folder_name: string;
  path: string;
  sub_dirs: DevSubDir[];
  category: string;
}

interface DevServerInfo {
  id: string;
  card_name: string;
  subdir: string;
  work_dir: string;
  command: string;
  started_at: number;
  status: string;
  pid: number | null;
  port: number | null;
}

interface DevHistoryEntry {
  work_dir: string;
  card_name: string;
  subdir_key: string;
  command: string;
  project_path: string;
  last_started_at: number;
}

interface DevConfig {
  cookie: string;
  proxy_target: string;
  port: number | null;
}

interface DevLogLine {
  stream: string;
  line: string;
}

// 从粘贴文本中提取 WISCPSID=xxx
function extractWiscpsid(input: string): string {
  const m = input.match(/WISCPSID=([^\s;]+)/i);
  if (m) return `WISCPSID=${m[1]}`;
  // 如果只粘贴了 UUID-like 值
  if (/^[a-f0-9-]{8,}$/i.test(input.trim())) return `WISCPSID=${input.trim()}`;
  return input.trim();
}

export default function DevModePage() {
  const navigate = useNavigate();
  const { encoded } = useParams<{ encoded: string }>();
  const [searchParams, setSearchParams] = useSearchParams();

  const projectPath = decodeURIComponent(encoded ?? '');
  const projectName = projectPath.split('/').pop() || '';

  const [cards, setCards] = useState<DevCardInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [refreshing, setRefreshing] = useState(false);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);

  const [runningServers, setRunningServers] = useState<Record<string, DevServerInfo>>({});
  const [failedServers, setFailedServers] = useState<Record<string, number>>({});
  const [failedInfo, setFailedInfo] = useState<Record<string, DevServerInfo>>({});
  const [logsMap, setLogsMap] = useState<Record<string, DevLogLine[]>>({});
  const [history, setHistory] = useState<DevHistoryEntry[]>([]);
  const [installing, setInstalling] = useState<Record<string, boolean>>({});
  const [cmdLogs, setCmdLogs] = useState<Record<string, string[]>>({});

  // 运行中服务器的实时镜像（事件监听器内部读取用，避免闭包过期）
  const runningServersRef = useRef<Record<string, DevServerInfo>>({});
  useEffect(() => {
    runningServersRef.current = runningServers;
  }, [runningServers]);

  const activeKeys = useMemo(
    () =>
      new Set(
        Object.values(runningServers)
          .filter((s) => s.status === 'running')
          .map((s) => `${s.card_name}:${s.subdir}`)
      ),
    [runningServers]
  );

  const mainCards = cards.filter((c) => c.category === 'main');
  const templateCards = cards.filter((c) => c.category === 'template');
  const cardList = cards.filter((c) => c.category === 'card');

  const runningList = Object.values(runningServers)
    .filter((s) => s.status === 'running')
    .sort((a, b) => a.started_at - b.started_at);
  const runningBase = runningList.filter((s) => {
    const c = cards.find((c) => c.folder_name === s.card_name);
    return c && (c.category === 'main' || c.category === 'template');
  });
  const runningCards = runningList.filter((s) => {
    const c = cards.find((c) => c.folder_name === s.card_name);
    return !c || c.category === 'card';
  });

  const selectedCard = cards.find((c) => c.folder_name === selectedFolder) ?? null;

  // 检查是否是从快捷入口进入（无返回按钮）
  const isQuickMode = searchParams.get('quick') === '1';

  // 收藏状态
  const favorite = useFavoriteStore((s) => s.favorite);
  const loadFavorite = useFavoriteStore((s) => s.load);
  const setFavorite = useFavoriteStore((s) => s.set);
  const clearFavorite = useFavoriteStore((s) => s.clear);
  const isFavorited = favorite?.path === projectPath;

  async function toggleFavorite() {
    if (isFavorited) {
      await clearFavorite();
    } else {
      await setFavorite(projectPath, projectName);
    }
  }

  // 根据卡片类别返回启动命令
  function devCommandForCard(card: DevCardInfo): string {
    return card.category === 'main' ? 'pnpm run dev' : 'pnpm run dev:shell';
  }

  // ===== 历史记录 =====
  async function loadHistory() {
    try {
      setHistory(await invoke<DevHistoryEntry[]>('list_dev_history', { projectPath }));
    } catch {
      /* ignore */
    }
  }

  async function clearHistory() {
    try {
      await invoke('clear_dev_history', { projectPath });
      setHistory([]);
    } catch (e) {
      setError(`清除历史失败: ${e}`);
    }
  }

  // ===== 初始化 + 事件监听 =====
  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    loadFavorite();

    async function init() {
      const unlog = await listen<{ server_id: string; stream: string; line: string }>(
        'dev-server-log',
        (e) => {
          const { server_id, stream, line } = e.payload;
          setLogsMap((prev) => ({
            ...prev,
            [server_id]: [...(prev[server_id] ?? []), { stream, line }],
          }));
        }
      );
      unlisteners.push(unlog);

      const uncmd = await listen<{ work_dir: string; command: string; stream: string; line: string }>(
        'dev-cmd-line',
        (e) => {
          const key = `${e.payload.work_dir}:${e.payload.command}`;
          setCmdLogs((prev) => ({
            ...prev,
            [key]: [...(prev[key] ?? []), `[${e.payload.stream}] ${e.payload.line}`],
          }));
        }
      );
      unlisteners.push(uncmd);

      const uncdone = await listen<{
        work_dir: string;
        command: string;
        exit_code: number;
        success: boolean;
      }>('dev-cmd-done', (e) => {
        const key = `${e.payload.work_dir}:${e.payload.command}`;
        setCmdLogs((prev) => ({
          ...prev,
          [key]: [...(prev[key] ?? []), `─── 完成 (退出码 ${e.payload.exit_code}) ───`],
        }));
        // 记录日志
        if (e.payload.command === 'install') {
          if (e.payload.success) {
            logs.info('install', `安装完成 (${e.payload.work_dir})`);
          } else {
            logs.error('install', `安装失败 (退出码 ${e.payload.exit_code}): ${e.payload.work_dir}`);
          }
        }
      });
      unlisteners.push(uncdone);

      const unst = await listen<{ server_id: string; exit_code: number; success: boolean }>(
        'dev-server-stopped',
        (e) => {
          const info = runningServersRef.current[e.payload.server_id];
          setRunningServers((prev) => {
            const ns = { ...prev };
            delete ns[e.payload.server_id];
            return ns;
          });
          // 内部记录失败信息（不在页面显示）
          if (!e.payload.success) {
            setFailedServers((prev) => ({ ...prev, [e.payload.server_id]: e.payload.exit_code }));
            if (info) {
              setFailedInfo((prev) => ({ ...prev, [e.payload.server_id]: info }));
            }
            // 记录错误日志
            logs.error(
              'dev-server',
              `${info?.card_name ?? e.payload.server_id} 启动失败 (退出码 ${e.payload.exit_code})`
            );
          } else {
            setLogsMap((prev) => {
              const ns = { ...prev };
              delete ns[e.payload.server_id];
              return ns;
            });
            logs.info('dev-server', `${info?.card_name ?? e.payload.server_id} 已停止`);
          }
        }
      );
      unlisteners.push(unst);

      // 监听一键停止全部
      const unall = await listen<{ count: number }>('dev-all-stopped', () => {
        setRunningServers({});
        setFailedServers({});
        setFailedInfo({});
        setLogsMap({});
      });
      unlisteners.push(unall);

      // 监听端口事件
      const unport = await listen<{ server_id: string; port: number }>('dev-server-port', (e) => {
        const { server_id, port } = e.payload;
        setRunningServers((prev) =>
          prev[server_id] ? { ...prev, [server_id]: { ...prev[server_id], port } } : prev
        );
      });
      unlisteners.push(unport);

      if (disposed) return;

      try {
        const result = await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath });
        if (disposed) return;
        setCards(result);
        const servers = await invoke<DevServerInfo[]>('list_dev_servers');
        if (disposed) return;
        const sMap: Record<string, DevServerInfo> = {};
        for (const s of servers) sMap[s.id] = s;
        setRunningServers(sMap);
        const cardCategories = result.filter((c) => c.category === 'card');
        if (cardCategories.length > 0) setSelectedFolder(cardCategories[0].folder_name);
        await loadHistory();
      } catch (e) {
        if (!disposed) setError(`加载失败: ${e}`);
      } finally {
        if (!disposed) setLoading(false);
      }
    }

    void init();

    return () => {
      disposed = true;
      for (const u of unlisteners) u();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectPath]);

  function isRunning(card: DevCardInfo, subDir: DevSubDir): boolean {
    return activeKeys.has(`${card.folder_name}:${subDir.key}`);
  }
  function cardHasRunning(card: DevCardInfo): boolean {
    return card.sub_dirs.some((sd) => isRunning(card, sd));
  }
  function serverIdFor(card: DevCardInfo, subDir: DevSubDir): string | undefined {
    return Object.values(runningServers).find(
      (s) => s.card_name === card.folder_name && s.subdir === subDir.key
    )?.id;
  }

  async function startDev(card: DevCardInfo, subDir: DevSubDir) {
    // 清除该卡片之前的失败记录
    for (const sid of Object.keys(failedServers)) {
      const info = failedInfo[sid];
      if (info && info.card_name === card.folder_name && info.subdir === subDir.key) {
        setFailedServers((prev) => {
          const fs = { ...prev };
          delete fs[sid];
          return fs;
        });
        setFailedInfo((prev) => {
          const fi = { ...prev };
          delete fi[sid];
          return fi;
        });
        setLogsMap((prev) => {
          const ns = { ...prev };
          delete ns[sid];
          return ns;
        });
      }
    }
    const command = devCommandForCard(card);
    logs.info('dev-server', `启动 ${card.display_name}/${subDir.label}`);
    try {
      const id = await invoke<string>('start_dev_server', {
        workDir: subDir.work_dir,
        cardName: card.folder_name,
        subdirKey: subDir.key,
        command,
        projectPath,
      });
      const info: DevServerInfo = {
        id,
        card_name: card.folder_name,
        subdir: subDir.key,
        work_dir: subDir.work_dir,
        command,
        started_at: Math.floor(Date.now() / 1000),
        status: 'running',
        pid: null,
        port: null,
      };
      setRunningServers((prev) => ({ ...prev, [id]: info }));
      await loadHistory();
    } catch (e) {
      setError(`启动失败: ${e}`);
      logs.error('dev-server', `启动失败: ${e}`);
    }
  }

  async function stopDev(serverId: string) {
    const info = runningServers[serverId];
    try {
      await invoke<boolean>('stop_dev_server', { serverId });
      setRunningServers((prev) => {
        const ns = { ...prev };
        delete ns[serverId];
        return ns;
      });
      logs.info('dev-server', `${info?.card_name ?? serverId} 已停止`);
    } catch (e) {
      setError(`停止失败: ${e}`);
      logs.error('dev-server', `停止失败: ${e}`);
    }
  }

  async function runInstall(card: DevCardInfo, subDir: DevSubDir) {
    const k = `${card.folder_name}:${subDir.key}`;
    setInstalling((prev) => ({ ...prev, [k]: true }));
    setCmdLogs((prev) => ({ ...prev, [`${subDir.work_dir}:install`]: [] }));
    logs.info('install', `${card.display_name}/${subDir.label} pnpm install`);
    try {
      await invoke<string>('run_card_command', {
        workDir: subDir.work_dir,
        subdirKey: subDir.key,
        commandName: 'install',
      });
    } catch (e) {
      setError(`安装失败: ${e}`);
      logs.error('install', `${card.display_name}/${subDir.label} 安装失败: ${e}`);
    }
  }

  const [switching, setSwitching] = useState(false);
  async function quickSwitch(card: DevCardInfo, subDir: DevSubDir) {
    if (isRunning(card, subDir)) return;
    setSwitching(true);
    try {
      for (const s of runningCards) {
        await invoke<boolean>('stop_dev_server', { serverId: s.id });
        setRunningServers((prev) => {
          const ns = { ...prev };
          delete ns[s.id];
          return ns;
        });
      }
      await startDev(card, subDir);
    } catch (e) {
      setError(`切换失败: ${e}`);
    } finally {
      setSwitching(false);
    }
  }

  async function refresh() {
    setRefreshing(true);
    try {
      setCards(await invoke<DevCardInfo[]>('scan_dev_dirs', { projectPath }));
      await loadHistory();
    } catch (e) {
      setError(`刷新失败: ${e}`);
    } finally {
      setRefreshing(false);
    }
  }

  function serverCard(server: DevServerInfo): DevCardInfo | undefined {
    return cards.find((c) => c.folder_name === server.card_name);
  }
  function serverSubDirLabel(server: DevServerInfo): string {
    const c = serverCard(server);
    return c?.sub_dirs.find((s) => s.key === server.subdir)?.label ?? server.subdir;
  }

  // 一键停止所有运行中的服务
  const [stoppingAll, setStoppingAll] = useState(false);
  async function stopAllServers() {
    setStoppingAll(true);
    try {
      await invoke<number>('stop_all_dev_servers');
      setRunningServers({});
      logs.info('dev-server', '已停止所有服务');
    } catch (e) {
      setError(`停止失败: ${e}`);
      logs.error('dev-server', `停止失败: ${e}`);
    } finally {
      setStoppingAll(false);
    }
  }

  // 从历史记录重启服务
  async function restartFromHistory(entry: DevHistoryEntry) {
    // 优先在已扫描的 cards 中匹配
    const card = cards.find((c) => c.sub_dirs.some((sd) => sd.work_dir === entry.work_dir));
    if (card) {
      const subDir = card.sub_dirs.find((sd) => sd.work_dir === entry.work_dir);
      if (subDir) {
        await startDev(card, subDir);
        return;
      }
    }
    // 卡片可能已改名/删除，但目录仍存在，直接启动
    try {
      const id = await invoke<string>('start_dev_server', {
        workDir: entry.work_dir,
        cardName: entry.card_name,
        subdirKey: entry.subdir_key,
        command: entry.command,
        projectPath,
      });
      const info: DevServerInfo = {
        id,
        card_name: entry.card_name,
        subdir: entry.subdir_key,
        work_dir: entry.work_dir,
        command: entry.command,
        started_at: Math.floor(Date.now() / 1000),
        status: 'running',
        pid: null,
        port: null,
      };
      setRunningServers((prev) => ({ ...prev, [id]: info }));
      await loadHistory();
      logs.info('dev-server', `启动 ${entry.card_name}/${entry.subdir_key}`);
    } catch (e) {
      setError(`启动失败: ${e}`);
      logs.error('dev-server', `启动失败: ${e}`);
    }
  }

  // 判断历史条目是否正在运行
  function historyIsRunning(entry: DevHistoryEntry): boolean {
    return Object.values(runningServers).some(
      (s) => s.work_dir === entry.work_dir && s.status === 'running'
    );
  }

  // 历史条目对应的显示名
  function historyDisplayName(entry: DevHistoryEntry): string {
    const card = cards.find((c) => c.folder_name === entry.card_name);
    if (card) return card.display_name;
    return entry.card_name;
  }

  // 历史条目对应的子目录标签
  function historySubDirLabel(entry: DevHistoryEntry): string {
    const card = cards.find((c) => c.folder_name === entry.card_name);
    if (card) {
      const sd = card.sub_dirs.find((s) => s.key === entry.subdir_key);
      if (sd) return sd.label;
    }
    return entry.subdir_key;
  }

  // 返回：优先返回到项目详情页（通过 ?from= 携带的项目路径）
  function goBack() {
    const from = searchParams.get('from');
    if (from) {
      navigate(`/?detail=${encodeURIComponent(from)}`);
    } else {
      navigate('/');
    }
  }

  // ===== 开发配置（Cookie / 代理地址） =====
  const [showConfig, setShowConfig] = useState(false);
  const [configLoading, setConfigLoading] = useState(false);
  const [configSaving, setConfigSaving] = useState(false);
  const [configError, setConfigError] = useState('');
  const [configData, setConfigData] = useState<DevConfig | null>(null);
  const [cookieInput, setCookieInput] = useState(''); // 用户粘贴的原始内容
  const [cookiePreview, setCookiePreview] = useState(''); // 提取后的 WISCPSID=xxx

  function onCookieInput(value: string) {
    setCookieInput(value);
    setCookiePreview(extractWiscpsid(value));
  }

  // 配置按钮按下的主卡片路径
  const [configMasterPath, setConfigMasterPath] = useState('');
  async function openConfig(masterPath: string) {
    setConfigMasterPath(masterPath);
    setShowConfig(true);
    setConfigLoading(true);
    setConfigError('');
    setCookieInput('');
    setCookiePreview('');
    try {
      const data = await invoke<DevConfig>('read_dev_config', { masterPath });
      setConfigData(data);
      setCookieInput(data.cookie);
      setCookiePreview(data.cookie);
    } catch (e) {
      setConfigError(`读取配置失败: ${e}`);
    } finally {
      setConfigLoading(false);
    }
  }

  async function saveConfig() {
    if (!cookiePreview) return;
    setConfigSaving(true);
    setConfigError('');
    try {
      await invoke('save_dev_cookie', { masterPath: configMasterPath, cookie: cookiePreview });
      setShowConfig(false);
    } catch (e) {
      setConfigError(`保存失败: ${e}`);
    } finally {
      setConfigSaving(false);
    }
  }

  async function openLogin() {
    if (!configData?.proxy_target) return;
    try {
      await invoke('open_login_url', { url: configData.proxy_target });
    } catch (e) {
      setConfigError(`打开浏览器失败: ${e}`);
    }
  }

  // ===== 渲染辅助 =====
  function renderBaseCard(card: DevCardInfo, isTemplate: boolean) {
    const cardRunning = cardHasRunning(card);
    return (
      <div key={card.folder_name} className={`base-card${cardRunning ? ' base-running' : ''}`}>
        <div className="base-card-head">
          <span className="base-card-name">{card.display_name}</span>
          {isTemplate ? (
            <span className="base-card-badge">模板</span>
          ) : (
            <button
              className="btn-config"
              onClick={() => openConfig(card.path)}
              title="配置 Cookie / 代理地址"
            >
              ⚙
            </button>
          )}
          {cardRunning && <span className="run-dot-sm" />}
        </div>
        <div className="base-card-folder">{card.folder_name}</div>
        <div className="base-card-actions">
          {card.sub_dirs.map((subDir) =>
            subDir.has_package_json ? (
              <div key={subDir.key} className="base-action-row">
                <span className="base-action-label">{subDir.label}</span>
                {isRunning(card, subDir) ? (
                  <button
                    className="btn-sm btn-stop"
                    onClick={() => {
                      const id = serverIdFor(card, subDir);
                      if (id) stopDev(id);
                    }}
                  >
                    停止
                  </button>
                ) : (
                  <button className="btn-sm btn-start" onClick={() => startDev(card, subDir)}>
                    启动
                  </button>
                )}
                <button
                  className="btn-sm btn-install"
                  onClick={() => runInstall(card, subDir)}
                  disabled={installing[`${card.folder_name}:${subDir.key}`]}
                >
                  {installing[`${card.folder_name}:${subDir.key}`] ? '...' : 'Install'}
                </button>
              </div>
            ) : null
          )}
        </div>
        {/* 运行中状态条 */}
        {card.sub_dirs.map((subDir) => {
          if (!isRunning(card, subDir)) return null;
          const sid = serverIdFor(card, subDir);
          if (!sid) return null;
          const srv = runningServers[sid];
          return (
            <div key={`bar-${subDir.key}`} className="running-status-bar">
              <span className="run-dot-sm" />
              <span className="running-status-label">{subDir.label} 运行中</span>
              {srv?.port && (
                <a
                  className="port-badge"
                  href={`http://localhost:${srv.port}`}
                  target="_blank"
                  rel="noreferrer"
                  title="点击打开"
                >
                  :{srv.port}
                </a>
              )}
              <span className="running-status-time">
                {formatClockSec(srv?.started_at ?? 0)}
              </span>
              <button className="log-stop-btn" onClick={() => stopDev(sid)} title="停止">
                <Square size={9} fill="currentColor" strokeWidth={0} />
              </button>
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div className="dev-mode-page">
      {/* 导航栏 */}
      <div className="dev-nav">
        {!isQuickMode && (
          <button className="back-btn" onClick={goBack}>
            <ChevronLeft size={18} strokeWidth={2.5} />
            返回
          </button>
        )}
        <div className="dev-nav-info">
          <Zap size={16} strokeWidth={2} />
          <span className="dev-nav-title">{projectName}</span>
          <span className="dev-nav-badge">开发模式</span>
        </div>
        <button
          className={`fav-btn${isFavorited ? ' fav-active' : ''}`}
          onClick={toggleFavorite}
          title={isFavorited ? '取消收藏' : '收藏为快捷入口'}
        >
          <Star size={16} fill={isFavorited ? 'currentColor' : 'none'} strokeWidth={isFavorited ? 1 : 1.5} />
        </button>
        {runningList.length > 0 && (
          <button
            className="stop-all-nav"
            onClick={stopAllServers}
            disabled={stoppingAll}
            title="一键停止所有服务"
          >
            {stoppingAll ? (
              <div className="btn-spinner-sm" />
            ) : (
              <Square size={14} fill="currentColor" strokeWidth={0} />
            )}
            <span>停止全部 ({runningList.length})</span>
          </button>
        )}
        <button
          className="refresh-btn"
          onClick={refresh}
          disabled={refreshing}
          title="刷新目录"
        >
          {refreshing ? <div className="btn-spinner-sm" /> : <RefreshCw size={16} strokeWidth={2} />}
        </button>
      </div>

      {error && (
        <div className="dev-error">
          {error}{' '}
          <button className="error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {loading ? (
        <div className="dev-loading">
          <div className="spinner" />
          <span>正在扫描卡片目录...</span>
        </div>
      ) : (
        <>
          {/* 两栏布局 */}
          <div className="two-col">
            {/* 左栏：基础服务 */}
            <div className="col-left">
              <div className="col-header">
                <span className="col-title">基础服务</span>
                <span className="col-hint">主站点 + 模板</span>
              </div>

              {mainCards.map((card) => renderBaseCard(card, false))}
              {templateCards.map((card) => renderBaseCard(card, true))}
            </div>

            {/* 右栏：卡片 */}
            <div className="col-right">
              <div className="col-header">
                <span className="col-title">卡片</span>
                <span className="col-hint">{cardList.length} 个</span>
                {runningList.length > 0 && (
                  <button
                    className="btn-sm btn-stop-all"
                    onClick={stopAllServers}
                    disabled={stoppingAll}
                  >
                    停止全部
                  </button>
                )}
              </div>

              {/* 卡片标签 */}
              <div className="card-tags">
                {cardList.map((card) => {
                  const cardRunning = cardHasRunning(card);
                  return (
                    <button
                      key={card.folder_name}
                      className={`card-tag${selectedFolder === card.folder_name ? ' tag-active' : ''}${
                        cardRunning ? ' tag-running' : ''
                      }`}
                      onClick={() => setSelectedFolder(card.folder_name)}
                    >
                      {cardRunning && <span className="tag-dot" />}
                      {card.display_name}
                    </button>
                  );
                })}
              </div>

              {/* 选中卡片操作 */}
              {selectedCard && (
                <div className="card-panel">
                  <div className="card-panel-head">
                    <span className="card-panel-name">{selectedCard.display_name}</span>
                    <span className="card-panel-folder">{selectedCard.folder_name}</span>
                  </div>
                  <div className="card-panel-actions">
                    {selectedCard.sub_dirs.map((subDir) =>
                      subDir.has_package_json ? (
                        <div key={subDir.key} className="card-action-row">
                          <span className="card-action-label">{subDir.label}</span>
                          <span className="card-action-path" title={subDir.work_dir}>
                            {subDir.work_dir.split('/').slice(-3).join('/')}
                          </span>
                          <div className="card-action-btns">
                            {isRunning(selectedCard, subDir) ? (
                              <button
                                className="btn-sm btn-stop"
                                onClick={() => {
                                  const id = serverIdFor(selectedCard, subDir);
                                  if (id) stopDev(id);
                                }}
                              >
                                停止
                              </button>
                            ) : (
                              <>
                                <button
                                  className="btn-sm btn-switch"
                                  onClick={() => quickSwitch(selectedCard, subDir)}
                                  disabled={switching}
                                >
                                  {switching ? '⏳' : '⇄'} 切换
                                </button>
                                <button
                                  className="btn-sm btn-start-sm"
                                  onClick={() => startDev(selectedCard, subDir)}
                                >
                                  启动
                                </button>
                              </>
                            )}
                            <button
                              className="btn-sm btn-install"
                              onClick={() => runInstall(selectedCard, subDir)}
                              disabled={installing[`${selectedCard.folder_name}:${subDir.key}`]}
                            >
                              {installing[`${selectedCard.folder_name}:${subDir.key}`]
                                ? '...'
                                : 'Install'}
                            </button>
                          </div>
                        </div>
                      ) : null
                    )}
                  </div>
                </div>
              )}

              {/* 运行中的卡片服务 */}
              {runningCards.length > 0 && (
                <div className="running-cards">
                  {runningCards.map((srv) => {
                    const sc = serverCard(srv);
                    return (
                      <div key={srv.id} className="run-card">
                        <div className="run-card-head">
                          <span className="run-dot" />
                          <span className="run-name">{sc?.display_name ?? srv.card_name}</span>
                          <span className="run-sub">{serverSubDirLabel(srv)}</span>
                          {srv.port && (
                            <a
                              className="port-badge"
                              href={`http://localhost:${srv.port}`}
                              target="_blank"
                              rel="noreferrer"
                              title="点击打开"
                            >
                              :{srv.port}
                            </a>
                          )}
                          <span className="run-time">{formatClockSec(srv.started_at)}</span>
                          <button className="log-stop-btn" onClick={() => stopDev(srv.id)} title="停止">
                            <Square size={9} fill="currentColor" strokeWidth={0} />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}

              {/* 最近启动历史 */}
              {history.length > 0 && (
                <div className="history-section">
                  <div className="history-header">
                    <span className="history-title">🕘 最近启动</span>
                    <span className="history-hint">{history.length} 条</span>
                    <button className="btn-history-clear" onClick={clearHistory} title="清除历史">
                      清除
                    </button>
                  </div>
                  <div className="history-list">
                    {history.map((entry) => (
                      <div
                        key={entry.work_dir}
                        className={`history-item${historyIsRunning(entry) ? ' history-running' : ''}`}
                      >
                        <div className="history-item-info">
                          <span className="history-item-name">{historyDisplayName(entry)}</span>
                          <span className="history-item-sub">{historySubDirLabel(entry)}</span>
                          <span className="history-item-time">
                            {formatClockSec(entry.last_started_at)}
                          </span>
                        </div>
                        {historyIsRunning(entry) ? (
                          <span className="run-dot-sm" />
                        ) : (
                          <button
                            className="btn-sm btn-start-sm"
                            onClick={() => restartFromHistory(entry)}
                            title="一键启动"
                          >
                            启动
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </div>

          {cards.length === 0 && (
            <div className="empty-state">
              <Zap size={48} strokeWidth={1.5} />
              <h3>未找到卡片目录</h3>
              <p>此项目下没有包含前端项目的子目录</p>
            </div>
          )}
        </>
      )}

      {/* 开发配置弹窗 */}
      <Modal
        open={showConfig}
        onClose={() => setShowConfig(false)}
        title="开发配置"
        closeDisabled={configSaving}
      >
        {configLoading ? (
          <div className="config-body">
            <div className="spinner" />
            <span>正在读取配置...</span>
          </div>
        ) : configData ? (
          <>
            <div className="config-body">
              {configError && (
                <div className="config-error">
                  {configError}{' '}
                  <button className="error-dismiss" onClick={() => setConfigError('')}>
                    ✕
                  </button>
                </div>
              )}

              {/* 后端代理地址 + 打开登录 */}
              <div className="config-section">
                <label className="config-label" htmlFor="proxy-target">
                  🌐 后端代理地址
                  {configData.port && <span className="config-port">: {configData.port}</span>}
                </label>
                <div className="proxy-row">
                  <input
                    id="proxy-target"
                    className="config-proxy-input"
                    value={configData.proxy_target}
                    readOnly
                  />
                  <button
                    className="btn-open-login"
                    onClick={openLogin}
                    disabled={!configData.proxy_target}
                    title="在浏览器中打开登录"
                  >
                    打开登录
                  </button>
                </div>
              </div>

              {/* Cookie 配置 */}
              <div className="config-section">
                <label className="config-label" htmlFor="cookie-input">
                  🍪 登录 Cookie
                </label>
                <p className="config-hint">
                  点「打开登录」→ 登录后端 → F12 复制 Cookie → 粘贴到下方
                </p>
                <textarea
                  id="cookie-input"
                  className="config-textarea"
                  value={cookieInput}
                  onChange={(e) => onCookieInput(e.target.value)}
                  placeholder="粘贴完整 Cookie（会自动提取 WISCPSID）..."
                  rows={3}
                />

                {cookiePreview && cookiePreview !== cookieInput.trim() ? (
                  <div className="cookie-preview">
                    <span className="cookie-preview-label">✅ 提取结果：</span>
                    <code>{cookiePreview}</code>
                  </div>
                ) : cookiePreview ? (
                  <div className="cookie-preview">
                    <span className="cookie-preview-label">当前 Cookie：</span>
                    <code>{cookiePreview}</code>
                  </div>
                ) : null}
              </div>
            </div>

            <div className="config-footer">
              <button
                className="btn-save"
                onClick={saveConfig}
                disabled={configSaving || !cookiePreview}
              >
                {configSaving ? '保存中...' : '保存 Cookie'}
              </button>
            </div>
          </>
        ) : null}
      </Modal>
    </div>
  );
}
