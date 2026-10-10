import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router';
import { ChevronLeft, Plus, RefreshCw } from 'lucide-react';
import type { ModelConfig, RunningModelInfo } from '@/features/models/types';
import ModelFormModal from '@/features/models/ModelFormModal';
import ModelCard from '@/features/models/ModelCard';
import ScanPanel from '@/features/models/ScanPanel';
import Modal from '@/components/ui/Modal';
import './ModelsPage.scss';

export default function ModelsPage() {
  // ===== 状态 =====
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [runningInfo, setRunningInfo] = useState<RunningModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  /** null = 添加模式；非空 = 编辑该模型 */
  const [formModel, setFormModel] = useState<ModelConfig | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ModelConfig | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null); // track which model id is performing an action

  // ===== 加载 =====
  useEffect(() => {
    void loadModels();
    void pollRunning();
    const interval = setInterval(() => {
      void pollRunning();
    }, 3000);
    return () => clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function loadModels() {
    setLoading(true);
    setError('');
    try {
      setModels(await invoke<ModelConfig[]>('list_model_configs'));
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  async function pollRunning() {
    try {
      setRunningInfo(await invoke<RunningModelInfo[]>('list_running_servers'));
    } catch {
      // 静默忽略轮询错误
    }
  }

  // ===== 状态查询 =====
  function isModelRunning(model: ModelConfig): boolean {
    const info = runningInfo.find((r) => r.id === model.id || r.port === model.port);
    return (info?.status ?? 'stopped') === 'running';
  }

  // ===== 启动/停止 =====
  async function startModel(model: ModelConfig) {
    setActionLoading(model.id);
    try {
      await invoke('start_llama_server', {
        serverPath: model.server_path,
        modelPath: model.model_path,
        port: model.port,
        ngl: model.ngl,
        modelName: model.name,
      });
      await pollRunning();
    } catch (e) {
      setError(`启动失败: ${e}`);
    } finally {
      setActionLoading(null);
    }
  }

  async function stopModel(model: ModelConfig) {
    setActionLoading(model.id);
    try {
      await invoke('stop_llama_server', { port: model.port });
      await pollRunning();
    } catch (e) {
      setError(`停止失败: ${e}`);
    } finally {
      setActionLoading(null);
    }
  }

  // ===== 删除 =====
  async function doDelete() {
    const target = deleteTarget;
    if (!target) return;
    try {
      await invoke('delete_model_config', { id: target.id });
      setModels((prev) => prev.filter((m) => m.id !== target.id));
      setDeleteTarget(null);
    } catch (e) {
      setError(`删除失败: ${e}`);
      setDeleteTarget(null);
    }
  }

  // ===== 弹窗编排 =====
  function openAddModal() {
    setFormModel(null);
    setShowForm(true);
  }

  function openEditModal(model: ModelConfig) {
    setFormModel(model);
    setShowForm(true);
  }

  return (
    <div className="models-page">
      <div className="models-page__header">
        <div className="models-page__header-left">
          <Link to="/settings" className="models-page__back" title="返回设置">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回设置
          </Link>
          <h1 className="models-page__title">模型管理</h1>
          <p className="models-page__subtitle">管理 llama.cpp 模型服务</p>
        </div>
        <div className="models-page__actions">
          <button className="models-page__refresh" onClick={() => void loadModels()} disabled={loading}>
            <RefreshCw size={16} strokeWidth={2.5} />
            刷新
          </button>
          <button className="btn-add" onClick={openAddModal}>
            <Plus size={16} strokeWidth={2.5} />
            添加模型
          </button>
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div className="models-page__error">
          <span>⚠️</span>
          <span>{error}</span>
          <button className="models-page__error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {/* 加载 */}
      {loading && (
        <div className="models-page__loading">
          <div className="spinner" />
          <span>正在加载模型配置...</span>
        </div>
      )}

      {/* 扫描模型目录 */}
      <ScanPanel models={models} onAdded={loadModels} onError={(msg) => setError(msg)} />

      {/* 空状态 */}
      {!loading && models.length === 0 && !error && (
        <div className="models-page__empty">
          <span className="models-page__empty-icon">🤖</span>
          <h3>还没有配置任何模型</h3>
          <p>点击上方"添加模型"按钮，配置第一个 llama.cpp 模型</p>
        </div>
      )}

      {/* 模型卡片 */}
      {!loading && models.length > 0 && (
        <>
          <div className="models-page__count">{models.length} 个模型</div>
          <div className="models-page__grid">
            {models.map((model) => (
              <ModelCard
                key={model.id}
                model={model}
                isRunning={isModelRunning(model)}
                isLoading={actionLoading === model.id}
                onDelete={(m) => setDeleteTarget(m)}
                onStop={stopModel}
                onStart={startModel}
                onEdit={openEditModal}
              />
            ))}
          </div>
        </>
      )}

      {/* 添加/编辑模型弹窗 */}
      {showForm && (
        <ModelFormModal model={formModel} onClose={() => setShowForm(false)} onSaved={loadModels} />
      )}

      {/* 删除确认弹窗 */}
      {deleteTarget && (
        <Modal
          open
          title="确认删除"
          onClose={() => setDeleteTarget(null)}
          variant="confirm"
          footer={
            <button className="btn-danger" onClick={doDelete}>
              确认删除
            </button>
          }
        >
          <div className="confirm-icon">⚠️</div>
          <p className="confirm-text">
            确定要删除模型 <strong>{deleteTarget.name}</strong> 吗？
          </p>
          <div className="confirm-path">{deleteTarget.name}</div>
        </Modal>
      )}
    </div>
  );
}
