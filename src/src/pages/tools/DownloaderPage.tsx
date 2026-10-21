/**
 * 下载管理页面
 * 移植自 src/routes/tools/downloader/+page.svelte
 */
import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ArrowLeft, Trash2, RefreshCw, Plus, Pause, Play, Folder, File } from 'lucide-react';
import Modal from '@/components/ui/Modal';
import { formatSize, formatSpeed, formatRelativeSec } from '@/lib/format';
import './DownloaderPage.scss';

interface DownloadTask {
  id: string;
  url: string;
  filename: string;
  save_path: string;
  file_path: string;
  total_size: number;
  downloaded_size: number;
  status: string;
  error: string | null;
  created_at: number;
  started_at: number | null;
  completed_at: number | null;
  speed: number;
}

interface ProgressEvent {
  id: string;
  downloaded: number;
  total: number;
  speed: number;
  progress: number;
}

// 最终下载事件（下载结束：完成/失败，含时间戳）
interface FinalEvent {
  id: string;
  status: string;
  filename: string;
  file_path: string;
  finished_at: number | null;
  total: number;
  error: string | null;
}

interface FinalNotice {
  type: 'success' | 'error';
  message: string;
}

function getStatusInfo(status: string): { text: string; className: string } {
  switch (status) {
    case 'Pending':
      return { text: '等待中', className: 's-pending' };
    case 'Downloading':
      return { text: '下载中', className: 's-downloading' };
    case 'Paused':
      return { text: '已暂停', className: 's-paused' };
    case 'Completed':
      return { text: '已完成', className: 's-completed' };
    case 'Failed':
      return { text: '失败', className: 's-failed' };
    case 'Cancelled':
      return { text: '已取消', className: 's-cancelled' };
    default:
      return { text: status, className: '' };
  }
}

export default function DownloaderPage() {
  const navigate = useNavigate();

  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  // 新建下载弹窗
  const [showAddModal, setShowAddModal] = useState(false);
  const [newUrl, setNewUrl] = useState('');
  const [newFilename, setNewFilename] = useState('');
  const [newPath, setNewPath] = useState('');
  const [formError, setFormError] = useState('');
  const [formSaving, setFormSaving] = useState(false);

  // 进度跟踪
  const [progressMap, setProgressMap] = useState<Record<string, ProgressEvent>>({});

  // 最终下载事件提示（完成/失败，带时间）
  const [finalNotice, setFinalNotice] = useState<FinalNotice | null>(null);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [confirmClear, setConfirmClear] = useState(false);

  function showFinalNotice(type: 'success' | 'error', message: string) {
    setFinalNotice({ type, message });
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setFinalNotice(null), 5000);
  }

  async function loadTasks() {
    setLoading(true);
    setError('');
    try {
      setTasks(await invoke<DownloadTask[]>('list_downloads'));
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  function getProgress(taskId: string): ProgressEvent | undefined {
    return progressMap[taskId];
  }

  // ===== 初始化 + 事件监听 =====
  useEffect(() => {
    const unlistens: UnlistenFn[] = [];
    let disposed = false;

    (async () => {
      await loadTasks();

      // 监听下载进度
      unlistens.push(
        await listen<ProgressEvent>('download-progress', (event) => {
          setProgressMap((prev) => ({ ...prev, [event.payload.id]: event.payload }));
        })
      );

      // 监听下载完成
      unlistens.push(
        await listen<{ id: string }>('download-complete', () => {
          loadTasks();
        })
      );

      // 监听最终下载事件（完成/失败，含时间戳）
      unlistens.push(
        await listen<FinalEvent>('download-final', (event) => {
          const p = event.payload;
          const time = p.finished_at ? formatRelativeSec(p.finished_at) : '';
          if (p.status === 'Completed') {
            showFinalNotice('success', `下载完成「${p.filename}」${time}`);
          } else if (p.status === 'Failed') {
            showFinalNotice(
              'error',
              `下载失败「${p.filename}」${time}${p.error ? `：${p.error}` : ''}`
            );
          }
          loadTasks();
        })
      );
    })();

    return () => {
      disposed = true;
      for (const u of unlistens) u();
      if (noticeTimer.current) clearTimeout(noticeTimer.current);
    };
  }, []);

  // ===== 新增下载 =====
  function openAddModal() {
    setNewUrl('');
    setNewFilename('');
    setNewPath('');
    setFormError('');
    setFormSaving(false);
    setShowAddModal(true);
  }

  async function addDownload() {
    setFormError('');
    if (!newUrl.trim()) {
      setFormError('请输入下载链接');
      return;
    }

    setFormSaving(true);
    try {
      await invoke('add_download', {
        url: newUrl.trim(),
        filename: newFilename.trim() || null,
        savePath: newPath.trim() || null,
      });
      setShowAddModal(false);
      await loadTasks();
    } catch (e) {
      setFormError(`添加失败: ${e}`);
    } finally {
      setFormSaving(false);
    }
  }

  // ===== 操作 =====
  async function pauseTask(taskId: string) {
    try {
      await invoke('pause_download', { taskId });
      await loadTasks();
    } catch (e) {
      setError(`暂停失败: ${e}`);
    }
  }

  async function resumeTask(taskId: string) {
    try {
      await invoke('start_download', { taskId });
      await loadTasks();
    } catch (e) {
      setError(`继续失败: ${e}`);
    }
  }

  async function retryTask(taskId: string) {
    try {
      await invoke('retry_download', { taskId });
      await loadTasks();
    } catch (e) {
      setError(`重试失败: ${e}`);
    }
  }

  async function deleteTask(taskId: string, deleteFile = false) {
    try {
      await invoke('delete_download', { taskId, deleteFile });
      setProgressMap((prev) => {
        const next = { ...prev };
        delete next[taskId];
        return next;
      });
      await loadTasks();
    } catch (e) {
      setError(`删除失败: ${e}`);
    }
  }

  async function openFolder(path: string) {
    try {
      await invoke('open_download_folder', { path });
    } catch (e) {
      setError(`打开文件夹失败: ${e}`);
    }
  }

  async function openFile(path: string) {
    try {
      await invoke('open_download_file', { path });
    } catch (e) {
      setError(`打开文件失败: ${e}`);
    }
  }

  // 清空记录（二次确认，防止误操作）
  async function clearHistory() {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
    try {
      await invoke('clear_download_history');
      setProgressMap({});
      await loadTasks();
    } catch (e) {
      setError(`清空失败: ${e}`);
    }
  }

  function getProgressBar(task: DownloadTask): number {
    const progress = getProgress(task.id);
    if (progress) return progress.progress;
    if (task.total_size > 0) return Math.round((task.downloaded_size / task.total_size) * 100);
    return 0;
  }

  return (
    <div className="downloader-page">
      {/* 页头 */}
      <div className="page-header">
        <div className="header-left">
          <button className="btn-back" onClick={() => navigate('/tools')}>
            <ArrowLeft size={18} strokeWidth={2.5} />
            返回
          </button>
          <h1>下载记录</h1>
        </div>
        <div className="header-actions">
          <button
            className={`btn-clear${confirmClear ? ' confirming' : ''}`}
            onClick={clearHistory}
          >
            <Trash2 size={16} strokeWidth={2.5} />
            {confirmClear ? '确认清空？' : '清空记录'}
          </button>
          <button className="btn-refresh" onClick={loadTasks} disabled={loading}>
            <RefreshCw size={16} strokeWidth={2.5} />
            刷新
          </button>
          <button className="btn-add" onClick={openAddModal}>
            <Plus size={16} strokeWidth={2.5} />
            新建下载
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>⚠️</span>
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {/* 最终下载事件提示（完成/失败，带时间） */}
      {finalNotice && (
        <div className={`final-notice ${finalNotice.type}`}>
          <span>{finalNotice.type === 'success' ? '✅' : '⚠️'}</span>
          <span>{finalNotice.message}</span>
          <button className="error-dismiss" onClick={() => setFinalNotice(null)}>
            ✕
          </button>
        </div>
      )}

      {/* 加载 */}
      {loading && (
        <div className="loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      )}

      {/* 空状态 */}
      {!loading && tasks.length === 0 && !error && (
        <div className="empty-state">
          <div className="empty-icon-big">📥</div>
          <h3>暂无下载记录</h3>
          <p>点击「新建下载」按钮开始第一个下载</p>
        </div>
      )}

      {/* 下载列表 */}
      {!loading && tasks.length > 0 && (
        <>
          <div className="tasks-count">{tasks.length} 个任务</div>
          <div className="tasks-list">
            {tasks.map((task) => {
              const statusInfo = getStatusInfo(task.status);
              const progressValue = getProgressBar(task);
              const prog = getProgress(task.id);
              return (
                <div key={task.id} className="task-card">
                  <div className="task-main">
                    <div className="task-info">
                      <div className="task-header-row">
                        <h3 className="task-filename">{task.filename}</h3>
                        <span className={`task-status ${statusInfo.className}`}>
                          {statusInfo.text}
                        </span>
                      </div>
                      <div className="task-url" title={task.url}>
                        {task.url}
                      </div>
                      <div className="task-meta">
                        {task.status === 'Downloading' && prog ? (
                          <>
                            <span className="meta-speed">{formatSpeed(prog.speed)}</span>
                            <span className="meta-divider">·</span>
                            <span className="meta-size">
                              {formatSize(prog.downloaded)} / {formatSize(prog.total)}
                            </span>
                          </>
                        ) : task.total_size > 0 ? (
                          <span className="meta-size">
                            {formatSize(task.downloaded_size)} / {formatSize(task.total_size)}
                          </span>
                        ) : task.status === 'Downloading' ? (
                          <span className="meta-size">{formatSize(task.downloaded_size)}</span>
                        ) : null}
                        <span className="meta-divider">·</span>
                        <span className="meta-time">{formatRelativeSec(task.created_at)}</span>
                        {task.status === 'Completed' && task.completed_at && (
                          <>
                            <span className="meta-divider">·</span>
                            <span className="meta-time">
                              完成于 {formatRelativeSec(task.completed_at)}
                            </span>
                          </>
                        )}
                      </div>
                    </div>

                    {/* 进度条 */}
                    {(task.status === 'Downloading' ||
                      task.status === 'Paused' ||
                      task.status === 'Pending') && (
                      <div className="progress-bar-wrapper">
                        <div className="progress-bar">
                          <div
                            className={`progress-fill${task.status === 'Paused' ? ' paused' : ''}`}
                            style={{ width: `${progressValue}%` }}
                          />
                        </div>
                        <span className="progress-text">{progressValue}%</span>
                      </div>
                    )}

                    {/* 错误信息 */}
                    {task.error && (
                      <div className="task-error">
                        <span>⚠️</span>
                        <span>{task.error}</span>
                      </div>
                    )}
                  </div>

                  <div className="task-actions">
                    {/* 下载中 → 暂停 */}
                    {task.status === 'Downloading' && (
                      <button
                        className="btn-action btn-pause"
                        onClick={() => pauseTask(task.id)}
                        title="暂停"
                      >
                        <Pause size={14} strokeWidth={2.5} />
                        暂停
                      </button>
                    )}

                    {/* 暂停 → 继续 */}
                    {task.status === 'Paused' && (
                      <button
                        className="btn-action btn-resume"
                        onClick={() => resumeTask(task.id)}
                        title="继续"
                      >
                        <Play size={14} strokeWidth={2.5} />
                        继续
                      </button>
                    )}

                    {/* 失败/取消 → 重试 */}
                    {(task.status === 'Failed' || task.status === 'Cancelled') && (
                      <button
                        className="btn-action btn-retry"
                        onClick={() => retryTask(task.id)}
                        title="重试"
                      >
                        <RefreshCw size={14} strokeWidth={2.5} />
                        重试
                      </button>
                    )}

                    {/* 已完成 → 打开文件/文件夹 */}
                    {task.status === 'Completed' && (
                      <>
                        <button
                          className="btn-action btn-file"
                          onClick={() => openFile(task.file_path)}
                          title="打开文件"
                        >
                          <File size={14} strokeWidth={2.5} />
                          打开文件
                        </button>
                        <button
                          className="btn-action btn-folder"
                          onClick={() => openFolder(task.save_path)}
                          title="打开文件夹"
                        >
                          <Folder size={14} strokeWidth={2.5} />
                        </button>
                      </>
                    )}

                    {/* 删除（带确认） */}
                    <div className="btn-group">
                      <button
                        className="btn-action btn-delete-sm"
                        onClick={() => deleteTask(task.id, false)}
                        title="删除任务"
                      >
                        <Trash2 size={14} strokeWidth={2.5} />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* 新建下载弹窗 */}
      <Modal
        open={showAddModal}
        onClose={() => setShowAddModal(false)}
        title="新建下载"
        closeOnOverlay={false}
        footer={
          <button className="btn-save" onClick={addDownload} disabled={formSaving}>
            {formSaving ? '添加中...' : '开始下载'}
          </button>
        }
      >
        {formError && <div className="form-error">{formError}</div>}
        <div className="form-group">
          <label>
            下载链接 <span className="required">*</span>
          </label>
          <input
            type="text"
            value={newUrl}
            onChange={(e) => setNewUrl(e.target.value)}
            placeholder="https://example.com/file.zip"
          />
        </div>
        <div className="form-group">
          <label>
            文件名 <span className="optional-tag">(可选)</span>
          </label>
          <input
            type="text"
            value={newFilename}
            onChange={(e) => setNewFilename(e.target.value)}
            placeholder="留空自动从 URL 提取"
          />
        </div>
        <div className="form-group">
          <label>
            保存路径 <span className="optional-tag">(可选)</span>
          </label>
          <input
            type="text"
            value={newPath}
            onChange={(e) => setNewPath(e.target.value)}
            placeholder="留空使用系统下载目录"
          />
        </div>
      </Modal>
    </div>
  );
}
