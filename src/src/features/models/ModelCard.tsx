import { useNavigate } from 'react-router';
import { Pencil, Play, Square, Trash2 } from 'lucide-react';
import type { ModelConfig } from './types';
import './ModelCard.scss';

interface ModelCardProps {
  model: ModelConfig;
  isRunning: boolean;
  isLoading: boolean;
  onDelete: (model: ModelConfig) => void;
  onStop: (model: ModelConfig) => void;
  onStart: (model: ModelConfig) => void;
  onEdit: (model: ModelConfig) => void;
}

export default function ModelCard({
  model,
  isRunning,
  isLoading,
  onDelete,
  onStop,
  onStart,
  onEdit,
}: ModelCardProps) {
  const navigate = useNavigate();

  return (
    <div className="model-card">
      <div className="model-card__header">
        <span className="model-card__icon">🧩</span>
        <div className="model-card__info">
          <h3 className="model-card__name">{model.name}</h3>
          <div className="model-card__meta">
            <span className="model-card__meta-tag">端口 {model.port}</span>
            <span className="model-card__meta-tag">ngl {model.ngl}</span>
          </div>
        </div>
        <span
          className={`model-card__status ${
            isRunning ? 'model-card__status--running' : 'model-card__status--stopped'
          }`}
        >
          {isRunning ? '运行中' : '已停止'}
        </span>
      </div>

      <div className="model-card__paths">
        <div className="model-card__path-row">
          <span className="model-card__path-label">模型</span>
          <span className="model-card__path-value" title={model.model_path}>
            {model.model_path}
          </span>
        </div>
      </div>

      <div className="model-card__footer">
        <div className="model-card__footer-left">
          <button
            className="model-card__btn-delete"
            onClick={() => onDelete(model)}
            title="删除"
          >
            <Trash2 size={14} strokeWidth={1.8} />
          </button>
          {isLoading && (
            <span className="model-card__action-loading">
              <span className="mini-spinner" />
              处理中...
            </span>
          )}
        </div>
        <div className="model-card__actions">
          {isRunning ? (
            <button
              className="model-card__btn-stop"
              onClick={() => onStop(model)}
              disabled={isLoading}
            >
              <Square size={14} strokeWidth={1.8} />
              停止
            </button>
          ) : (
            <button
              className="model-card__btn-start"
              onClick={() => onStart(model)}
              disabled={isLoading}
            >
              <Play size={14} strokeWidth={1.8} />
              启动
            </button>
          )}
          <button
            className="model-card__btn-detail"
            onClick={() => navigate(`/models/${model.id}`)}
          >
            详情 →
          </button>
          <button className="model-card__btn-edit" onClick={() => onEdit(model)} title="编辑">
            <Pencil size={14} strokeWidth={1.8} />
          </button>
        </div>
      </div>
    </div>
  );
}
