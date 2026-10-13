import { useEffect, useState } from 'react';
import { useNmStore, nmCache, type NodeModulesInfo } from '@/stores/nm';
import { formatBytes } from '@/lib/format';
import Modal from '@/components/ui/Modal';
import { Activity, Play, RefreshCw, Trash2 } from 'lucide-react';
import './NodeModulesPanel.scss';

interface Props {
  /** 当前展示的项目路径（变化时自动从缓存恢复或重置） */
  projectPath: string;
  editorName: string;
  onOpenEditor: (path: string) => void;
}

export default function NodeModulesPanel({
  projectPath,
  editorName,
  onOpenEditor,
}: Props) {
  const scanning = useNmStore((s) => s.scanning);
  const cleaning = useNmStore((s) => s.cleaning);
  const progress = useNmStore((s) => s.progress);
  const startScan = useNmStore((s) => s.startScan);
  const cleanSingle = useNmStore((s) => s.cleanSingle);
  const cleanAll = useNmStore((s) => s.cleanAll);

  const [nodeModulesList, setNodeModulesList] = useState<NodeModulesInfo[]>([]);
  const [nodeModulesScannedPath, setNodeModulesScannedPath] = useState<string | null>(null);
  const [nodeModulesError, setNodeModulesError] = useState('');
  const [deletingPath, setDeletingPath] = useState<string | null>(null);

  // 切换项目时：有缓存直接展示，无缓存清空等待手动扫描
  useEffect(() => {
    const cached = nmCache.get(projectPath);
    if (cached) {
      setNodeModulesList(cached);
      setNodeModulesScannedPath(projectPath);
    } else {
      setNodeModulesList([]);
      setNodeModulesScannedPath(null);
    }
    setNodeModulesError('');
  }, [projectPath]);

  const nmProjectCount = new Set(nodeModulesList.map((r) => r.project_name)).size;
  const totalNodeModulesSize = nodeModulesList.reduce(
    (sum, r) => sum + r.size_bytes,
    0,
  );

  async function scanNodeModules() {
    if (scanning) return;
    setNodeModulesList([]);
    setNodeModulesError('');
    const result = await startScan(projectPath);
    setNodeModulesList(result || []);
    setNodeModulesScannedPath(projectPath);
    const err = useNmStore.getState().error;
    if (err) setNodeModulesError(err);
  }

  async function doDeleteNodeModule() {
    if (!deletingPath) return;
    const target = deletingPath;
    const cleaned = await cleanSingle(target);
    setNodeModulesList((prev) => {
      const next = prev.filter((r) => !cleaned.includes(r.path));
      nmCache.set(projectPath, next);
      return next;
    });
    const err = useNmStore.getState().error;
    if (err) setNodeModulesError(err);
    setDeletingPath(null);
  }

  async function cleanAllNodeModules() {
    if (nodeModulesList.length === 0 || cleaning) return;
    const cleaned = await cleanAll(nodeModulesList.map((r) => r.path));
    if (cleaned) {
      setNodeModulesList((prev) => {
        const next = prev.filter((r) => !cleaned.includes(r.path));
        nmCache.set(projectPath, next);
        return next;
      });
    }
    const err = useNmStore.getState().error;
    if (err) setNodeModulesError(err);
  }

  return (
    <>
      <div className="nm-panel">
        <div className="nm-panel__section-header">
          <div className="nm-panel__section-title">
            <Activity size={16} strokeWidth={2} />
            <span>node_modules 清理</span>
          </div>
          <button
            className="nm-panel__scan-btn"
            onClick={scanNodeModules}
            disabled={scanning}
          >
            {scanning ? (
              <>
                <span className="nm-panel__btn-spinner" />
                扫描中...
              </>
            ) : (
              <>
                <RefreshCw size={14} strokeWidth={2} />
                {nodeModulesScannedPath === projectPath ? '重新扫描' : '扫描 node_modules'}
              </>
            )}
          </button>
        </div>

        {scanning && progress && (
          <div className="nm-panel__scan-progress">
            <div className="nm-panel__scan-progress-spinner" />
            <span className="nm-panel__scan-progress-text" title={progress}>
              {progress}
            </span>
          </div>
        )}

        {nodeModulesScannedPath === projectPath && !scanning && (
          <>
            {nodeModulesError && <div className="nm-panel__error">{nodeModulesError}</div>}

            {nodeModulesList.length === 0 ? (
              <div className="nm-panel__empty">
                <span>📦</span>
                <span>此项目中未找到 node_modules 文件夹</span>
              </div>
            ) : (
              <>
                <div className="nm-panel__summary">
                  <span className="nm-panel__stat">
                    <strong>{nmProjectCount}</strong> 个项目下共{' '}
                    <strong>{nodeModulesList.length}</strong> 个 node_modules
                  </span>
                  <span className="nm-panel__stat">
                    总计 <strong>{formatBytes(totalNodeModulesSize)}</strong>
                  </span>
                  <button
                    className="nm-panel__clean-btn"
                    onClick={cleanAllNodeModules}
                    disabled={cleaning}
                  >
                    {cleaning ? (
                      <>
                        <span className="nm-panel__btn-spinner" />
                        清理中...
                      </>
                    ) : (
                      <>
                        <Trash2 size={13} strokeWidth={2} />
                        一键清理全部
                      </>
                    )}
                  </button>
                </div>

                <div className="nm-panel__list">
                  {nodeModulesList.map((nmItem) => (
                    <div key={nmItem.path} className="nm-panel__item">
                      <div className="nm-panel__item-content">
                        <div className="nm-panel__item-path" title={nmItem.path}>
                          {nmItem.path}
                        </div>
                        <div className="nm-panel__item-right">
                          <button
                            className="nm-panel__item-open-btn"
                            onClick={() => onOpenEditor(nmItem.project_path)}
                            title={`在 ${editorName || '编辑器'} 中打开`}
                            aria-label="在编辑器中打开"
                          >
                            <Play size={13} strokeWidth={2} />
                          </button>
                          <span className="nm-panel__item-size">{nmItem.size_display}</span>
                        </div>
                      </div>
                      <button
                        className="nm-panel__item-delete-btn"
                        onClick={() => setDeletingPath(nmItem.path)}
                        title="删除此 node_modules"
                      >
                        <Trash2 size={12} strokeWidth={2} />
                      </button>
                    </div>
                  ))}
                </div>
              </>
            )}
          </>
        )}
      </div>

      {deletingPath && (
        <Modal
          open
          title="确认删除"
          variant="confirm"
          onClose={() => setDeletingPath(null)}
          closeOnOverlay={false}
          footer={
            <button
              className="nm-panel__confirm-delete-btn"
              onClick={doDeleteNodeModule}
              disabled={cleaning}
            >
              {cleaning ? (
                <>
                  <span className="nm-panel__btn-spinner" />
                  删除中...
                </>
              ) : (
                '确认删除'
              )}
            </button>
          }
        >
          <div className="nm-panel__confirm-body">
            <p>确定要删除以下 node_modules 吗？</p>
            <p className="nm-panel__confirm-path">{deletingPath.replace(projectPath, '.')}</p>
          </div>
        </Modal>
      )}
    </>
  );
}
