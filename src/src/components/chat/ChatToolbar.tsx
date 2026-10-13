/**
 * 对话页标题栏工具栏
 * 移植自 src/lib/ChatToolbar.svelte。
 * 左侧：来源切换 + Provider/模型选择；右侧：参数设置 / 清空 / 历史。
 * default export，props { side: 'left' | 'right' }，AppLayout 按此调用。
 */
import { Activity, Menu, Monitor, RefreshCw, Settings, Trash2 } from 'lucide-react';
import { useChatToolbarStore, type ToolbarSourceType } from '@/stores/chatToolbar';
import './ChatToolbar.scss';

interface Props {
  side: 'left' | 'right';
}

export default function ChatToolbar({ side }: Props) {
  const sourceType = useChatToolbarStore((s) => s.sourceType);
  const selectedProviderId = useChatToolbarStore((s) => s.selectedProviderId);
  const selectedModelPort = useChatToolbarStore((s) => s.selectedModelPort);
  const overrideModelName = useChatToolbarStore((s) => s.overrideModelName);
  const isSending = useChatToolbarStore((s) => s.isSending);
  const showSettings = useChatToolbarStore((s) => s.showSettings);
  const providers = useChatToolbarStore((s) => s.providers);
  const providerModels = useChatToolbarStore((s) => s.providerModels);
  const runningModels = useChatToolbarStore((s) => s.runningModels);
  const modelsLoading = useChatToolbarStore((s) => s.modelsLoading);

  /** 来源切换（与原版 onSwitchType 一致：发送中禁用 + 默认选中第一项） */
  function handleSwitchType(t: ToolbarSourceType) {
    const st = useChatToolbarStore.getState();
    if (st.sourceType === t || st.isSending) return;
    st.setSourceType(t);
    if (t === 'model' && st.runningModels.length > 0 && !st.selectedModelPort) {
      st.setSelectedModelPort(st.runningModels[0].port);
    }
    if (t === 'provider' && st.providers.length > 0 && !st.selectedProviderId) {
      st.selectProvider(st.providers[0].id);
    }
  }

  function handleProviderChange(e: React.ChangeEvent<HTMLSelectElement>) {
    const id = e.target.value;
    if (id) useChatToolbarStore.getState().selectProvider(id);
  }

  function handleModelPortChange(e: React.ChangeEvent<HTMLSelectElement>) {
    const port = Number(e.target.value);
    if (!Number.isNaN(port)) useChatToolbarStore.getState().setSelectedModelPort(port);
  }

  function handleOverrideModelChange(e: React.ChangeEvent<HTMLSelectElement>) {
    useChatToolbarStore.getState().setOverrideModelName(e.target.value || null);
  }

  if (side === 'left') {
    return (
      <div className="tb-left">
        <div className="seg-control">
          <button
            type="button"
            className={sourceType === 'provider' ? 'active' : ''}
            onClick={() => handleSwitchType('provider')}
            title="API 提供者"
          >
            <Activity size={14} strokeWidth={2.2} />
          </button>
          <button
            type="button"
            className={sourceType === 'model' ? 'active' : ''}
            onClick={() => handleSwitchType('model')}
            title="本地模型"
          >
            <Monitor size={14} strokeWidth={2.2} />
          </button>
        </div>

        {sourceType === 'provider' ? (
          <>
            <select
              className="picker"
              value={selectedProviderId ?? ''}
              onChange={handleProviderChange}
              disabled={isSending}
              title="选择 Provider"
            >
              {providers.length === 0 && <option value="">尚未配置</option>}
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>

            <div className="picker-group">
              <select
                className="picker mono"
                value={overrideModelName ?? ''}
                onChange={handleOverrideModelChange}
                disabled={isSending || modelsLoading}
                title="选择模型"
              >
                {!overrideModelName && <option value="">未选模型</option>}
                {modelsLoading && <option value="">加载中…</option>}
                {providerModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="icon-btn sm"
                onClick={() => void useChatToolbarStore.getState().fetchProviderModels()}
                disabled={isSending || modelsLoading}
                title="刷新模型列表"
                aria-label="刷新模型"
              >
                <RefreshCw size={13} strokeWidth={2.5} className={modelsLoading ? 'spinning' : ''} />
              </button>
            </div>
          </>
        ) : (
          <>
            <select
              className="picker mono"
              value={selectedModelPort ?? ''}
              onChange={handleModelPortChange}
              disabled={isSending}
              title="选择本地模型"
            >
              {runningModels.length === 0 && <option value="">无运行模型</option>}
              {runningModels.map((m) => (
                <option key={m.port} value={m.port}>
                  {m.name} :{m.port}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="icon-btn sm"
              onClick={() => void useChatToolbarStore.getState().refreshRunningModels()}
              disabled={isSending}
              title="刷新"
              aria-label="刷新"
            >
              <RefreshCw size={13} strokeWidth={2.5} />
            </button>
          </>
        )}
      </div>
    );
  }

  // ===== 右侧 =====
  return (
    <div className="tb-right">
      <button
        type="button"
        className={`icon-btn${showSettings ? ' active' : ''}`}
        onClick={() => useChatToolbarStore.getState().setShowSettings(!showSettings)}
        title="参数设置"
        aria-label="参数设置"
      >
        <Settings size={14} strokeWidth={2} />
      </button>
      <button
        type="button"
        className="icon-btn"
        onClick={() => useChatToolbarStore.getState().onClear()}
        disabled={isSending}
        title="清空对话"
        aria-label="清空对话"
      >
        <Trash2 size={14} strokeWidth={2} />
      </button>
      <button
        type="button"
        className="icon-btn"
        onClick={() => useChatToolbarStore.getState().onToggleSidebar()}
        title="历史记录"
        aria-label="历史记录"
      >
        <Menu size={14} strokeWidth={2} />
      </button>
    </div>
  );
}
