/**
 * node_modules 清理页面
 * 移植自 src/routes/tools/nm/+page.svelte
 * 扫描/清理逻辑使用全局 @/stores/nm（startScan/cleanSingle），
 * 逐项清理日志、跳过 pnpm 项目、停止等 UI 逻辑在本页实现。
 */
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { ChevronLeft, Folder, Search, Trash2, Play } from 'lucide-react';
import { useNmStore, type NodeModulesInfo } from '@/stores/nm';
import { formatBytes } from '@/lib/format';
import './NmPage.scss';

interface AppSettings {
  editor: { name: string; command: string };
  workspaces: { name: string; path: string }[];
  active_workspace: string | null;
  scan_depth: number;
}

interface CleanLogItem {
  project_name: string;
  path: string;
  success: boolean;
  size: string;
  message: string;
}

export default function NmPage() {
  const navigate = useNavigate();

  // 全局 nm store：扫描状态 / 进度 / 清理
  const scanning = useNmStore((s) => s.scanning);
  const storeError = useNmStore((s) => s.error);
  const storeProgress = useNmStore((s) => s.progress);
  const startScan = useNmStore((s) => s.startScan);
  const cleanSingle = useNmStore((s) => s.cleanSingle);

  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  // 扫描结果
  const [nmList, setNmList] = useState<NodeModulesInfo[]>([]);
  const [nmScanned, setNmScanned] = useState(false);

  // 编辑器
  const [editorCommand, setEditorCommand] = useState('');
  const [editorName, setEditorName] = useState('');

  // 清理状态
  const [cleaning, setCleaning] = useState(false);
  const [cleanLogs, setCleanLogs] = useState<CleanLogItem[]>([]);
  const [cleanIndex, setCleanIndex] = useState(0);
  const [done, setDone] = useState(false);
  const cleanStoppedRef = useRef(false);
  const logRef = useRef<HTMLDivElement | null>(null);

  // ===== 计算 =====
  const nmTotalSize = nmList.reduce((s, r) => s + r.size_bytes, 0);
  const nmProjectCount = new Set(nmList.map((r) => r.project_name)).size;
  const pnpmCount = nmList.filter((n) => n.has_pnpm_lock).length;
  const nmCleanCount = cleanLogs.filter((l) => l.success && !l.message.includes('⏭')).length;
  const nmSkippedCount = cleanLogs.filter((l) => l.message.includes('⏭')).length;
  const nmFailedCount = cleanLogs.filter((l) => !l.success).length;
  const nmCleanedSize = cleanLogs
    .filter((l) => l.success)
    .reduce((s, l) => {
      const item = nmList.find((n) => n.path === l.path);
      return s + (item?.size_bytes || 0);
    }, 0);

  // ===== 初始化：加载工作空间 =====
  useEffect(() => {
    let disposed = false;

    (async () => {
      try {
        const settings = await invoke<AppSettings>('get_settings');
        if (disposed) return;
        setActiveWorkspace(settings.active_workspace);
        setEditorCommand(settings.editor.command);
        setEditorName(settings.editor.name);
      } catch (e) {
        console.error('加载设置失败:', e);
      } finally {
        if (!disposed) setIsLoading(false);
      }
    })();

    return () => {
      disposed = true;
    };
  }, []);

  // ===== 扫描 =====
  async function scanNodeModules() {
    if (!activeWorkspace || scanning || cleaning) return;
    setNmList([]);
    setNmScanned(false);
    setCleanLogs([]);
    setCleanIndex(0);
    cleanStoppedRef.current = false;
    setDone(false);

    const result = await startScan(activeWorkspace);
    if (result) {
      setNmList(result);
      setNmScanned(true);
    }
  }

  // ===== 逐项清理 =====
  async function cleanAll() {
    if (nmList.length === 0 || cleaning) return;
    setCleaning(true);
    setCleanLogs([]);
    setCleanIndex(0);
    cleanStoppedRef.current = false;
    setDone(false);

    for (let i = 0; i < nmList.length; i++) {
      if (cleanStoppedRef.current) break;
      setCleanIndex(i + 1);
      const item = nmList[i];

      if (item.has_pnpm_lock) {
        setCleanLogs((prev) => [
          ...prev,
          {
            project_name: item.project_name,
            path: item.path,
            success: true,
            size: item.size_display,
            message: `⏭ ${item.project_name} — 已跳过 (pnpm 项目)`,
          },
        ]);
      } else {
        const before = useNmStore.getState().error;
        const cleaned = await cleanSingle(item.path);
        const after = useNmStore.getState().error;
        const success = cleaned.includes(item.path);
        const failReason = after && after !== before ? after.replace(/^删除失败: /, '') : '失败';
        setCleanLogs((prev) => [
          ...prev,
          {
            project_name: item.project_name,
            path: item.path,
            success,
            size: item.size_display,
            message: success
              ? `✓ ${item.project_name} — ${item.size_display}`
              : `✗ ${item.project_name} — 失败: ${failReason}`,
          },
        ]);
      }

      // 滚动到日志底部
      const logEl = logRef.current;
      if (logEl) {
        await new Promise((r) => setTimeout(r, 10));
        logEl.scrollTop = logEl.scrollHeight;
      }
    }

    setCleaning(false);
    if (!cleanStoppedRef.current) setDone(true);
  }

  function stopCleaning() {
    cleanStoppedRef.current = true;
  }

  // ===== 用编辑器打开所在项目目录 =====
  async function openInEditor(path: string) {
    if (!editorCommand) return;
    try {
      await invoke('open_in_editor', { path, editorCommand });
    } catch (e) {
      console.error('打开编辑器失败:', e);
    }
  }

  return (
    <div className="nm-page">
      {/* 页头 */}
      <div className="page-header">
        <div className="header-left">
          <button className="btn-back" onClick={() => navigate('/tools')} title="返回工具箱">
            <ChevronLeft size={14} strokeWidth={2.5} />
          </button>
          <h1>node_modules 清理</h1>
        </div>
      </div>

      {/* 工作空间信息 */}
      <div className="ws-info">
        {isLoading ? (
          <span className="ws-loading">加载中...</span>
        ) : activeWorkspace ? (
          <>
            <Folder size={16} strokeWidth={2} />
            <span className="ws-path" title={activeWorkspace}>
              {activeWorkspace}
            </span>
          </>
        ) : (
          <span className="ws-empty">
            尚未设置工作空间，请前往
            <Link to="/settings" className="ws-link">
              设置
            </Link>
            页面配置
          </span>
        )}
      </div>

      {/* 扫描按钮 */}
      <div className="scan-section">
        <button
          className="scan-btn"
          onClick={scanNodeModules}
          disabled={!activeWorkspace || scanning || cleaning}
        >
          {scanning ? (
            <>
              <div className="spinner" />
              正在扫描...
            </>
          ) : (
            <>
              <Search size={16} strokeWidth={2} />
              扫描 node_modules
            </>
          )}
        </button>
        {nmScanned && nmList.length > 0 && (
          <>
            <button className="clean-all-btn" onClick={cleanAll} disabled={cleaning || done}>
              {cleaning ? (
                <>
                  <div className="spinner" />
                  清理中 {cleanIndex}/{nmList.length} ...
                </>
              ) : done ? (
                '清理完成'
              ) : (
                <>
                  <Trash2 size={14} strokeWidth={2} />
                  一键清理全部 ({nmList.length})
                </>
              )}
            </button>
            {cleaning && (
              <button className="stop-btn" onClick={stopCleaning}>
                停止
              </button>
            )}
          </>
        )}
      </div>

      {/* 扫描进度 */}
      {scanning && storeProgress && (
        <div className="scan-progress">
          <div className="scan-progress-spinner" />
          <span className="scan-progress-text" title={storeProgress}>
            {storeProgress}
          </span>
        </div>
      )}

      {storeError && <div className="nm-store-error">{storeError}</div>}

      {/* 扫描结果 */}
      {nmScanned &&
        (nmList.length === 0 ? (
          <div className="nm-empty">
            <span className="nm-empty-icon">📦</span>
            <p>当前工作空间中未找到 node_modules 文件夹</p>
          </div>
        ) : (
          <>
            <div className="nm-stats">
              <span className="stat">
                <strong>{nmProjectCount}</strong> 个项目
              </span>
              <span className="stat-divider">·</span>
              <span className="stat">
                <strong>{nmList.length}</strong> 个 node_modules
              </span>
              <span className="stat-divider">·</span>
              <span className="stat">
                总计 <strong>{formatBytes(nmTotalSize)}</strong>
              </span>
              {pnpmCount > 0 && (
                <>
                  <span className="stat-divider">·</span>
                  <span className="stat">
                    <strong>{pnpmCount}</strong> 个 pnpm
                  </span>
                </>
              )}
              {nmCleanCount > 0 && (
                <>
                  <span className="stat-divider">·</span>
                  <span className="stat stat-cleaned">
                    已清理 <strong>{formatBytes(nmCleanedSize)}</strong>
                  </span>
                </>
              )}
            </div>

            {/* 进度条 */}
            {(cleaning || done) && (
              <>
                <div className="progress-bar-track">
                  <div
                    className="progress-bar-fill"
                    style={{ width: `${nmList.length > 0 ? (cleanIndex / nmList.length) * 100 : 0}%` }}
                  />
                  <span className="progress-text">
                    {cleanIndex}/{nmList.length}
                  </span>
                </div>
                {nmFailedCount > 0 && (
                  <div className="progress-errors">{nmFailedCount} 个删除失败</div>
                )}
              </>
            )}

            {/* 日志 */}
            {cleanLogs.length > 0 && (
              <div className="nm-log has-content" ref={logRef}>
                {cleanLogs.map((log, idx) => (
                  <div
                    key={`${log.path}-${idx}`}
                    className={`nm-log-item${log.success ? ' nm-log-success' : ''}${
                      !log.success ? ' nm-log-error' : ''
                    }`}
                  >
                    <span className="nm-log-message">{log.message}</span>
                  </div>
                ))}
                {done && nmFailedCount === 0 && nmSkippedCount === 0 && (
                  <div className="nm-log-item nm-log-done">
                    <span className="nm-log-message">
                      🎉 全部清理完成，共释放 {formatBytes(nmCleanedSize)}
                    </span>
                  </div>
                )}
                {done && nmFailedCount === 0 && nmSkippedCount > 0 && (
                  <div className="nm-log-item nm-log-done">
                    <span className="nm-log-message">
                      🎉 清理完成，{nmSkippedCount} 个 pnpm 项目已跳过，释放{' '}
                      {formatBytes(nmCleanedSize)}
                    </span>
                  </div>
                )}
                {done && nmFailedCount > 0 && (
                  <div className="nm-log-item nm-log-error">
                    <span className="nm-log-message">
                      ⚠️ 清理完成，{nmFailedCount} 个失败，{nmSkippedCount} 个跳过
                    </span>
                  </div>
                )}
              </div>
            )}

            {/* 列表 */}
            <div className="nm-list">
              <div className="nm-list-header">
                <span>路径</span>
                <span>大小</span>
              </div>
              {nmList.map((nm) => {
                const cleaned = cleanLogs.find((l) => l.path === nm.path && l.success);
                return (
                  <div
                    key={nm.path}
                    className={`nm-item${cleaned ? ' nm-item-cleaned' : ''}${
                      nm.has_pnpm_lock ? ' nm-item-pnpm' : ''
                    }`}
                  >
                    <div className="nm-item-left" title={nm.path}>
                      <span className="nm-item-path">{nm.path}</span>
                      <button
                        className="nm-item-open-btn"
                        onClick={() => openInEditor(nm.project_path)}
                        title={`在 ${editorName || '编辑器'} 中打开 ${nm.project_name}`}
                        aria-label={`在 ${editorName || '编辑器'} 中打开 ${nm.project_name}`}
                      >
                        <Play size={13} strokeWidth={2} />
                      </button>
                    </div>
                    <div className="nm-item-right">
                      <span className="nm-item-size">{nm.size_display}</span>
                      {cleaned ? (
                        <span className="nm-item-badge">已删除</span>
                      ) : nm.has_pnpm_lock ? (
                        <span className="nm-badge-pnpm">pnpm</span>
                      ) : null}
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        ))}
    </div>
  );
}
