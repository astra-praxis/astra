import { useEffect, useRef, useState, type ChangeEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigate, useParams } from 'react-router';
import {
  ArrowLeft,
  Check,
  ChevronDown,
  Code,
  Copy,
  FileText,
  List,
  Pencil,
  RefreshCw,
  Trash2,
} from 'lucide-react';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import { formatTimestampSec } from '@/lib/format';
import Modal from '@/components/ui/Modal';
import './ProviderDetailPage.scss';

// ===== 类型 =====
type ProviderKind = 'openai' | 'anthropic';

interface ProviderConfig {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  active_model: string | null;
  kind: ProviderKind;
  created_at: number;
  updated_at: number;
}

const KIND_LABEL: Record<ProviderKind, string> = {
  openai: 'OpenAI 兼容 API',
  anthropic: 'Anthropic Messages API',
};

interface ModelInfo {
  id: string;
  object: string | null;
  created: number | null;
  owned_by: string | null;
}

type ExampleTab = 'curl' | 'python';

export default function ProviderDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  // ===== 状态 =====
  const [provider, setProvider] = useState<ProviderConfig | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [error, setError] = useState('');
  const [modelsError, setModelsError] = useState('');
  const [modelsLoaded, setModelsLoaded] = useState(false);
  const [selectedModelId, setSelectedModelId] = useState(''); // 下拉选择器选中的模型
  const [configCopied, setConfigCopied] = useState(false); // 配置文档复制状态

  const [showEditModal, setShowEditModal] = useState(false);
  const [editName, setEditName] = useState('');
  const [editBaseUrl, setEditBaseUrl] = useState('');
  const [editApiKey, setEditApiKey] = useState('');
  const [editKind, setEditKind] = useState<ProviderKind>('openai');
  const [editError, setEditError] = useState('');
  const [editSaving, setEditSaving] = useState(false);

  const [activeTab, setActiveTab] = useState<ExampleTab>('curl');
  const [copied, setCopied] = useState(false);

  const copyTimerRef = useRef<number | null>(null);
  const configTimerRef = useRef<number | null>(null);

  // ===== 派生：按协议类型生成调用示例 =====
  const curlExample = !provider
    ? ''
    : provider.kind === 'anthropic'
      ? `curl ${provider.base_url}/messages \\\n  -H "Content-Type: application/json" \\\n  -H "x-api-key: ${provider.api_key || 'YOUR_API_KEY'}" \\\n  -H "anthropic-version: 2023-06-01" \\\n  -d '{
    "model": "${provider.active_model ?? 'MODEL_NAME'}",
    "max_tokens": 1024,
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'`
      : `curl ${provider.base_url}/chat/completions \\\n  -H "Content-Type: application/json" \\\n  -H "Authorization: Bearer ${provider.api_key || 'YOUR_API_KEY'}" \\\n  -d '{
    "model": "${provider.active_model ?? 'MODEL_NAME'}",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'`;

  const pythonExample = !provider
    ? ''
    : provider.kind === 'anthropic'
      ? `from anthropic import Anthropic

  client = Anthropic(
      base_url="${provider.base_url}",
      api_key="${provider.api_key || 'YOUR_API_KEY'}",
  )

  message = client.messages.create(
      model="${provider.active_model ?? 'MODEL_NAME'}",
      max_tokens=1024,
      messages=[
          {"role": "user", "content": "Hello"}
      ],
  )

  print(message.content[0].text)`
      : `from openai import OpenAI

  client = OpenAI(
      base_url="${provider.base_url}",
      api_key="${provider.api_key || 'YOUR_API_KEY'}",
  )

  response = client.chat.completions.create(
      model="${provider.active_model ?? 'MODEL_NAME'}",
      messages=[
          {"role": "user", "content": "Hello"}
      ],
  )

  print(response.choices[0].message.content)`;

  // ===== 模型配置文档 =====
  const modelConfigJson =
    provider && models.length > 0
      ? JSON.stringify(
          {
            api_url: provider.base_url,
            available_models: models.map((m) => ({
              name: m.id,
              max_tokens: 200000,
              max_output_tokens: 32000,
              max_completion_tokens: 200000,
              capabilities: {
                tools: true,
                images: false,
                parallel_tool_calls: false,
                prompt_cache_key: false,
                chat_completions: true,
                interleaved_reasoning: false,
              },
            })),
          },
          null,
          2,
        )
      : '';

  // ===== 加载 =====
  useEffect(() => {
    void loadProvider();
    return () => {
      if (copyTimerRef.current !== null) clearTimeout(copyTimerRef.current);
      if (configTimerRef.current !== null) clearTimeout(configTimerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  async function loadProvider() {
    setLoading(true);
    setError('');
    try {
      const p = await invoke<ProviderConfig>('get_provider', { id });
      setProvider(p);
      setSelectedModelId(p.active_model ?? '');
      // 如果有 active_model，自动加载模型列表
      await fetchModelsFor(p);

      // 同步到全局 toolbar：当前查看的 provider 成为选中状态
      useChatToolbarStore.setState({
        sourceType: 'provider',
        selectedProviderId: p.id,
        overrideModelName: p.active_model,
      });
      await useChatToolbarStore.getState().fetchProviderModels();
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  async function fetchModels() {
    if (!provider) return;
    await fetchModelsFor(provider);
  }

  async function fetchModelsFor(p: ProviderConfig) {
    setModelsLoading(true);
    setModelsError('');
    try {
      setModels(
        await invoke<ModelInfo[]>('fetch_provider_models', {
          baseUrl: p.base_url,
          apiKey: p.api_key,
          kind: p.kind,
        }),
      );
      setModelsLoaded(true);
    } catch (e) {
      setModelsError(`获取模型失败: ${e}`);
      setModelsLoaded(true);
    } finally {
      setModelsLoading(false);
    }
  }

  // ===== 切换模型 =====
  async function selectModel(modelId: string) {
    if (!provider || modelId === provider.active_model) return;
    try {
      await invoke('set_active_model', {
        providerId: provider.id,
        modelId,
      });
      setProvider({ ...provider, active_model: modelId });
      setSelectedModelId(modelId);

      // 同步到全局 toolbar
      useChatToolbarStore.setState({
        selectedProviderId: provider.id,
        sourceType: 'provider',
        overrideModelName: modelId,
      });
      await useChatToolbarStore.getState().fetchProviderModels();
    } catch (e) {
      setError(`切换模型失败: ${e}`);
    }
  }

  // 下拉选择器变化时切换模型
  function onModelSelectChange(e: ChangeEvent<HTMLSelectElement>) {
    const modelId = e.target.value;
    if (modelId) {
      void selectModel(modelId);
    }
  }

  // ===== 编辑 =====
  function openEditModal() {
    if (!provider) return;
    setEditName(provider.name);
    setEditBaseUrl(provider.base_url);
    setEditApiKey(provider.api_key);
    setEditKind(provider.kind);
    setEditError('');
    setEditSaving(false);
    setShowEditModal(true);
  }

  async function saveEdit() {
    setEditError('');
    if (!provider) return;
    if (!editName.trim()) {
      setEditError('请输入名称');
      return;
    }
    if (!editBaseUrl.trim()) {
      setEditError('请输入 API 地址');
      return;
    }

    setEditSaving(true);
    try {
      await invoke('save_provider', {
        id: provider.id,
        name: editName.trim(),
        baseUrl: editBaseUrl.trim(),
        apiKey: editApiKey.trim(),
        activeModel: provider.active_model,
        kind: editKind,
      });
      setShowEditModal(false);
      await loadProvider();
      void useChatToolbarStore.getState().loadProviders(); // 同步全局 toolbar
    } catch (e) {
      setEditError(`保存失败: ${e}`);
    } finally {
      setEditSaving(false);
    }
  }

  // ===== 删除 =====
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  async function doDelete() {
    if (!provider) return;
    try {
      await invoke('delete_provider', { id: provider.id });
      void useChatToolbarStore.getState().loadProviders(); // 同步全局 toolbar
      navigate('/providers');
    } catch (e) {
      setError(`删除失败: ${e}`);
      setShowDeleteConfirm(false);
    }
  }

  // ===== 复制 =====
  async function copyCode(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      if (copyTimerRef.current !== null) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => {
        setCopied(false);
        copyTimerRef.current = null;
      }, 2000);
    } catch {
      // ignore
    }
  }

  // ===== 复制配置文档 =====
  async function copyConfig() {
    try {
      await navigator.clipboard.writeText(modelConfigJson);
      setConfigCopied(true);
      if (configTimerRef.current !== null) clearTimeout(configTimerRef.current);
      configTimerRef.current = window.setTimeout(() => {
        setConfigCopied(false);
        configTimerRef.current = null;
      }, 2000);
    } catch {
      // ignore
    }
  }

  // ===== 辅助 =====
  function maskKey(key: string): string {
    if (!key) return '未设置';
    if (key.length <= 8) return '••••';
    return key.slice(0, 4) + '••••••••' + key.slice(-4);
  }

  return (
    <div className="provider-detail">
      {/* 返回 */}
      <button className="provider-detail__back" onClick={() => navigate('/providers')}>
        <ArrowLeft size={18} strokeWidth={2.5} />
        返回列表
      </button>

      {loading && (
        <div className="provider-detail__loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      )}

      {error && !provider && <div className="provider-detail__error">{error}</div>}

      {provider && (
        <>
          {/* 头部信息卡片 */}
          <div className="provider-detail__info-card">
            <div className="provider-detail__header-row">
              <div className="provider-detail__header-left">
                <span className="provider-detail__icon">🔌</span>
                <div>
                  <h1 className="provider-detail__title">{provider.name}</h1>
                  <p className="provider-detail__subtitle">
                    {KIND_LABEL[provider.kind] ?? 'OpenAI 兼容 API'}
                  </p>
                </div>
              </div>
              <div className="provider-detail__header-actions">
                <button className="provider-detail__btn-edit" onClick={openEditModal}>
                  <Pencil size={15} strokeWidth={2.5} />
                  编辑
                </button>
                <button
                  className="provider-detail__btn-danger-sm"
                  onClick={() => setShowDeleteConfirm(true)}
                >
                  <Trash2 size={15} strokeWidth={2.5} />
                  删除
                </button>
              </div>
            </div>

            {/* 基本信息网格 */}
            <div className="provider-detail__info-grid">
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">API 地址</span>
                <span className="provider-detail__info-value provider-detail__info-value--mono">
                  {provider.base_url}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">API Key</span>
                <span className="provider-detail__info-value provider-detail__info-value--mono">
                  {maskKey(provider.api_key)}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">当前模型</span>
                <span className="provider-detail__info-value provider-detail__info-value--mono provider-detail__info-value--highlight">
                  {provider.active_model ?? '未选择'}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">服务商类型</span>
                <span className="provider-detail__info-value">
                  {KIND_LABEL[provider.kind] ?? 'OpenAI 兼容'}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">创建时间</span>
                <span className="provider-detail__info-value">
                  {formatTimestampSec(provider.created_at)}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">更新时间</span>
                <span className="provider-detail__info-value">
                  {formatTimestampSec(provider.updated_at)}
                </span>
              </div>
              <div className="provider-detail__info-item">
                <span className="provider-detail__info-label">模型数量</span>
                <span className="provider-detail__info-value">
                  {models.length > 0 ? models.length : '—'}
                </span>
              </div>
            </div>
          </div>

          {error && <div className="provider-detail__error">{error}</div>}

          {/* 模型列表 */}
          <div className="provider-detail__section-card">
            <div className="provider-detail__section-header">
              <div className="provider-detail__title-row">
                <h2 className="provider-detail__section-title">
                  <List size={20} strokeWidth={1.8} />
                  可用模型
                </h2>
                {modelsLoading ? (
                  <span className="provider-detail__badge-loading">
                    <span className="mini-spinner" />
                    获取中...
                  </span>
                ) : modelsLoaded && models.length > 0 ? (
                  <span className="provider-detail__badge-count">{models.length} 个</span>
                ) : null}
              </div>
              <button
                className="provider-detail__btn-refresh-models"
                onClick={() => void fetchModels()}
                disabled={modelsLoading}
              >
                <RefreshCw
                  size={15}
                  strokeWidth={2.5}
                  className={modelsLoading ? 'provider-detail__spinning' : ''}
                />
                刷新模型列表
              </button>
            </div>

            {modelsError && <div className="provider-detail__error">{modelsError}</div>}

            {!modelsLoading && !modelsLoaded && (
              <div className="provider-detail__empty-models">
                <span className="provider-detail__empty-icon-sm">📋</span>
                <p>点击"刷新模型列表"获取可用模型</p>
              </div>
            )}

            {modelsLoading && (
              <div className="provider-detail__loading-sm">
                <div className="mini-spinner" />
                <span>正在获取模型列表...</span>
              </div>
            )}

            {!modelsLoading && modelsLoaded && models.length === 0 && !modelsError && (
              <div className="provider-detail__empty-models">
                <span className="provider-detail__empty-icon-sm">📭</span>
                <p>未发现任何模型</p>
              </div>
            )}

            {!modelsLoading && models.length > 0 && (
              <>
                {/* 模型切换栏：下拉选择器 */}
                <div className="provider-detail__selector-bar">
                  <div className="provider-detail__select-wrapper">
                    <select
                      className="provider-detail__select"
                      value={selectedModelId}
                      onChange={onModelSelectChange}
                    >
                      {models.map((model) => (
                        <option key={model.id} value={model.id}>
                          {model.id}
                        </option>
                      ))}
                    </select>
                    <ChevronDown size={14} strokeWidth={2.5} className="provider-detail__select-chevron" />
                  </div>
                  <span className="provider-detail__active-badge">
                    当前: {provider.active_model}
                  </span>
                </div>

                {/* 全部模型列表（紧凑展示） */}
                <div className="provider-detail__chip-list">
                  {models.map((model) => {
                    const isActive = provider.active_model === model.id;
                    return (
                      <button
                        key={model.id}
                        className={`provider-detail__chip ${isActive ? 'provider-detail__chip--active' : ''}`}
                        onClick={() => void selectModel(model.id)}
                        title={model.id}
                      >
                        {model.id}
                      </button>
                    );
                  })}
                </div>
              </>
            )}
          </div>

          {/* 代码示例 */}
          <div className="provider-detail__section-card">
            <div className="provider-detail__section-header">
              <h2 className="provider-detail__section-title">
                <Code size={20} strokeWidth={1.8} />
                调用示例
              </h2>
              <button
                className="provider-detail__btn-copy"
                onClick={() => void copyCode(activeTab === 'curl' ? curlExample : pythonExample)}
              >
                {copied ? (
                  <>
                    <Check size={14} strokeWidth={2.5} />
                    已复制
                  </>
                ) : (
                  <>
                    <Copy size={14} strokeWidth={2.5} />
                    复制
                  </>
                )}
              </button>
            </div>

            <div className="provider-detail__tabs">
              <button
                className={`provider-detail__tab ${
                  activeTab === 'curl' ? 'provider-detail__tab--active' : ''
                }`}
                onClick={() => setActiveTab('curl')}
              >
                cURL
              </button>
              <button
                className={`provider-detail__tab ${
                  activeTab === 'python' ? 'provider-detail__tab--active' : ''
                }`}
                onClick={() => setActiveTab('python')}
              >
                Python
              </button>
            </div>

            <pre className="provider-detail__code-block">
              <code>{activeTab === 'curl' ? curlExample : pythonExample}</code>
            </pre>
          </div>

          {/* 模型配置文档 */}
          {modelConfigJson && (
            <div className="provider-detail__section-card">
              <div className="provider-detail__section-header">
                <h2 className="provider-detail__section-title">
                  <FileText size={20} strokeWidth={1.8} />
                  模型配置文档
                </h2>
                <button className="provider-detail__btn-copy" onClick={() => void copyConfig()}>
                  {configCopied ? (
                    <>
                      <Check size={14} strokeWidth={2.5} />
                      已复制
                    </>
                  ) : (
                    <>
                      <Copy size={14} strokeWidth={2.5} />
                      复制
                    </>
                  )}
                </button>
              </div>
              <pre className="provider-detail__code-block provider-detail__code-block--config">
                <code>{modelConfigJson}</code>
              </pre>
            </div>
          )}
        </>
      )}

      {/* 编辑弹窗 */}
      {showEditModal && (
        <Modal
          open
          title="编辑服务商"
          onClose={() => setShowEditModal(false)}
          closeOnOverlay={false}
          footer={
            <button className="btn-save" onClick={saveEdit} disabled={editSaving}>
              {editSaving ? '保存中...' : '保存'}
            </button>
          }
        >
          {editError && <div className="form-error">{editError}</div>}
          <div className="form-group">
            <label>服务商类型</label>
            <div className="kind-seg">
              <button
                type="button"
                className={`kind-seg__btn ${editKind === 'openai' ? 'kind-seg__btn--active' : ''}`}
                onClick={() => setEditKind('openai')}
              >
                OpenAI 兼容
              </button>
              <button
                type="button"
                className={`kind-seg__btn ${editKind === 'anthropic' ? 'kind-seg__btn--active' : ''}`}
                onClick={() => setEditKind('anthropic')}
              >
                Anthropic
              </button>
            </div>
            <p className="input-hint">
              {editKind === 'anthropic'
                ? 'Anthropic 原生 Messages API，走 x-api-key 认证'
                : '适用于 OpenAI 兼容网关与本地服务（默认）'}
            </p>
          </div>
          <div className="form-group">
            <label>名称</label>
            <input type="text" value={editName} onChange={(e) => setEditName(e.target.value)} />
          </div>
          <div className="form-group">
            <label>API 地址</label>
            <input
              type="text"
              value={editBaseUrl}
              onChange={(e) => setEditBaseUrl(e.target.value)}
              placeholder={editKind === 'anthropic' ? 'https://api.anthropic.com' : 'https://api.openai.com/v1'}
            />
          </div>
          <div className="form-group">
            <label>API Key</label>
            <input
              type="password"
              value={editApiKey}
              onChange={(e) => setEditApiKey(e.target.value)}
            />
          </div>
        </Modal>
      )}

      {/* 删除确认 */}
      {showDeleteConfirm && (
        <Modal
          open
          title="确认删除"
          onClose={() => setShowDeleteConfirm(false)}
          variant="confirm"
          footer={
            <button className="btn-danger" onClick={doDelete}>
              确认删除
            </button>
          }
        >
          <div className="confirm-icon">⚠️</div>
          <p className="confirm-text">
            确定要删除服务商 <strong>{provider?.name}</strong> 吗？此操作不可撤销。
          </p>
        </Modal>
      )}
    </div>
  );
}
