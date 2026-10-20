import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link, useNavigate } from 'react-router';
import { ChevronLeft, Pencil, Plus, RefreshCw, Trash2, Wifi } from 'lucide-react';
import { useChatToolbarStore } from '@/stores/chatToolbar';
import { formatTimestampSec } from '@/lib/format';
import Modal from '@/components/ui/Modal';
import './ProvidersPage.scss';

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
  openai: 'OpenAI 兼容',
  anthropic: 'Anthropic',
};

export default function ProvidersPage() {
  const navigate = useNavigate();
  const syncToolbarProviders = useChatToolbarStore((s) => s.loadProviders);

  // ===== 状态 =====
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [showAddModal, setShowAddModal] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ProviderConfig | null>(null);

  // ===== 新表单 =====
  const [newName, setNewName] = useState('');
  const [newBaseUrl, setNewBaseUrl] = useState('');
  const [newApiKey, setNewApiKey] = useState('');
  const [newKind, setNewKind] = useState<ProviderKind>('openai');
  const [formError, setFormError] = useState('');
  const [formSaving, setFormSaving] = useState(false);
  const [testing, setTesting] = useState<ProviderConfig | null>(null);

  // ===== 编辑表单 =====
  const [showEditModal, setShowEditModal] = useState(false);
  const [editTarget, setEditTarget] = useState<ProviderConfig | null>(null);
  const [editName, setEditName] = useState('');
  const [editBaseUrl, setEditBaseUrl] = useState('');
  const [editApiKey, setEditApiKey] = useState('');
  const [editKind, setEditKind] = useState<ProviderKind>('openai');
  const [editError, setEditError] = useState('');
  const [editSaving, setEditSaving] = useState(false);

  // ===== 加载 =====
  useEffect(() => {
    void loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function loadProviders() {
    setLoading(true);
    setError('');
    try {
      setProviders(await invoke<ProviderConfig[]>('list_providers'));
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  // ===== 添加 =====
  function openAddModal() {
    setNewName('');
    setNewBaseUrl('');
    setNewApiKey('');
    setNewKind('openai');
    setFormError('');
    setFormSaving(false);
    setShowAddModal(true);
  }

  async function saveNewProvider() {
    setFormError('');
    if (!newName.trim()) {
      setFormError('请输入名称');
      return;
    }
    if (!newBaseUrl.trim()) {
      setFormError('请输入 API 地址');
      return;
    }

    setFormSaving(true);
    try {
      await invoke('save_provider', {
        name: newName.trim(),
        baseUrl: newBaseUrl.trim(),
        apiKey: newApiKey.trim(),
        activeModel: null,
        kind: newKind,
      });
      setShowAddModal(false);
      await loadProviders();
      void syncToolbarProviders(); // 同步全局 toolbar
    } catch (e) {
      setFormError(`保存失败: ${e}`);
    } finally {
      setFormSaving(false);
    }
  }

  // ===== 编辑 =====
  function openEditModal(provider: ProviderConfig) {
    setEditTarget(provider);
    setEditName(provider.name);
    setEditBaseUrl(provider.base_url);
    setEditApiKey(provider.api_key);
    setEditKind(provider.kind);
    setEditError('');
    setEditSaving(false);
    setShowEditModal(true);
  }

  async function saveEditProvider() {
    setEditError('');
    if (!editTarget) return;
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
        id: editTarget.id,
        name: editName.trim(),
        baseUrl: editBaseUrl.trim(),
        apiKey: editApiKey.trim(),
        activeModel: editTarget.active_model,
        kind: editKind,
      });
      setShowEditModal(false);
      setEditTarget(null);
      await loadProviders();
      void syncToolbarProviders(); // 同步全局 toolbar
    } catch (e) {
      setEditError(`保存失败: ${e}`);
    } finally {
      setEditSaving(false);
    }
  }

  // ===== 删除 =====
  async function doDelete() {
    const target = deleteTarget;
    if (!target) return;
    try {
      await invoke('delete_provider', { id: target.id });
      setProviders((prev) => prev.filter((p) => p.id !== target.id));
      void syncToolbarProviders(); // 同步全局 toolbar
      setDeleteTarget(null);
    } catch (e) {
      setError(`删除失败: ${e}`);
      setDeleteTarget(null);
    }
  }

  // ===== 快速测试连接 =====
  async function testConnection(provider: ProviderConfig) {
    setTesting(provider);
    try {
      const models = await invoke<unknown[]>('fetch_provider_models', {
        baseUrl: provider.base_url,
        apiKey: provider.api_key,
        kind: provider.kind,
      });
      if (models.length > 0) {
        alert(`连接成功！发现 ${models.length} 个可用模型`);
      } else {
        alert('连接成功，但未发现任何模型');
      }
    } catch (e) {
      alert(`连接失败: ${e}`);
    } finally {
      setTesting(null);
    }
  }

  function maskKey(key: string): string {
    if (!key) return '未设置';
    if (key.length <= 8) return '••••';
    return key.slice(0, 4) + '••••••••' + key.slice(-4);
  }

  return (
    <div className="providers-page">
      <div className="providers-page__header">
        <div className="providers-page__header-left">
          <Link to="/settings" className="providers-page__back" title="返回设置">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回设置
          </Link>
          <h1 className="providers-page__title">AI 服务商</h1>
          <p className="providers-page__subtitle">配置 OpenAI 兼容 / Anthropic 服务商与 API 密钥</p>
        </div>
        <div className="providers-page__actions">
          <button
            className="providers-page__refresh"
            onClick={() => void loadProviders()}
            disabled={loading}
          >
            <RefreshCw size={16} strokeWidth={2.5} />
            刷新
          </button>
          <button className="btn-add" onClick={openAddModal}>
            <Plus size={16} strokeWidth={2.5} />
            添加接口
          </button>
        </div>
      </div>

      {error && (
        <div className="providers-page__error">
          <span>⚠️</span>
          <span>{error}</span>
          <button className="providers-page__error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {loading && (
        <div className="providers-page__loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      )}

      {!loading && providers.length === 0 && !error && (
        <div className="providers-page__empty">
          <span className="providers-page__empty-icon">🔌</span>
          <h3>还没有配置任何 API 接口</h3>
          <p>点击上方"添加接口"按钮，配置第一个 OpenAI 兼容 API</p>
        </div>
      )}

      {!loading && providers.length > 0 && (
        <>
          <div className="providers-page__count">{providers.length} 个接口</div>
          <div className="providers-page__grid">
            {providers.map((provider) => {
              const isTesting = testing?.id === provider.id;
              return (
                <div key={provider.id} className="providers-page__card">
                  <div className="providers-page__card-header">
                    <span className="providers-page__icon">🔌</span>
                    <div className="providers-page__info">
                      <h3 className="providers-page__name">{provider.name}</h3>
                      <div className="providers-page__meta">
                        <span className="providers-page__meta-tag">{provider.base_url}</span>
                        <span className="providers-page__meta-tag providers-page__meta-tag--kind">
                          {KIND_LABEL[provider.kind] ?? 'OpenAI 兼容'}
                        </span>
                      </div>
                    </div>
                  </div>

                  <div className="providers-page__details">
                    <div className="providers-page__detail-row">
                      <span className="providers-page__detail-label">当前模型</span>
                      <span
                        className="providers-page__detail-value"
                        title={provider.active_model ?? ''}
                      >
                        {provider.active_model ?? '未选择'}
                      </span>
                    </div>
                    <div className="providers-page__detail-row">
                      <span className="providers-page__detail-label">API Key</span>
                      <span className="providers-page__detail-value providers-page__detail-value--mono">
                        {maskKey(provider.api_key)}
                      </span>
                    </div>
                    <div className="providers-page__detail-row">
                      <span className="providers-page__detail-label">创建时间</span>
                      <span className="providers-page__detail-value">
                        {formatTimestampSec(provider.created_at)}
                      </span>
                    </div>
                  </div>

                  <div className="providers-page__footer">
                    <div className="providers-page__footer-left">
                      <button
                        className="providers-page__btn-delete"
                        onClick={() => setDeleteTarget(provider)}
                        title="删除"
                      >
                        <Trash2 size={14} strokeWidth={2.5} />
                      </button>
                      {isTesting && (
                        <span className="providers-page__action-loading">
                          <span className="mini-spinner" />
                          测试中...
                        </span>
                      )}
                    </div>
                    <div className="providers-page__footer-actions">
                      <button
                        className="providers-page__btn-test"
                        onClick={() => testConnection(provider)}
                        disabled={isTesting}
                        title="测试连接"
                      >
                        <Wifi size={14} strokeWidth={2.5} />
                        测试
                      </button>
                      <button
                        className="providers-page__btn-edit"
                        onClick={() => openEditModal(provider)}
                        title="编辑"
                      >
                        <Pencil size={14} strokeWidth={2.5} />
                        编辑
                      </button>
                      <button
                        className="providers-page__btn-detail"
                        onClick={() => navigate(`/providers/${provider.id}`)}
                      >
                        详情 →
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* 添加弹窗 */}
      {showAddModal && (
        <Modal
          open
          title="添加 API 接口"
          onClose={() => setShowAddModal(false)}
          closeOnOverlay={false}
          footer={
            <button className="btn-save" onClick={saveNewProvider} disabled={formSaving}>
              {formSaving ? '保存中...' : '保存'}
            </button>
          }
        >
          {formError && <div className="form-error">{formError}</div>}
          <div className="form-group">
            <label>接口类型</label>
            <div className="kind-seg">
              <button
                type="button"
                className={`kind-seg__btn ${newKind === 'openai' ? 'kind-seg__btn--active' : ''}`}
                onClick={() => setNewKind('openai')}
              >
                OpenAI 兼容
              </button>
              <button
                type="button"
                className={`kind-seg__btn ${newKind === 'anthropic' ? 'kind-seg__btn--active' : ''}`}
                onClick={() => setNewKind('anthropic')}
              >
                Anthropic
              </button>
            </div>
            <p className="input-hint">
              {newKind === 'anthropic'
                ? 'Anthropic 原生 Messages API，走 x-api-key 认证'
                : '适用于 OpenAI 兼容网关与本地服务（默认）'}
            </p>
          </div>
          <div className="form-group">
            <label>名称</label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="如: 本地 Ollama"
            />
          </div>
          <div className="form-group">
            <label>API 地址</label>
            <input
              type="text"
              value={newBaseUrl}
              onChange={(e) => setNewBaseUrl(e.target.value)}
              placeholder={newKind === 'anthropic' ? 'https://api.anthropic.com' : 'http://gateway.internal:8002/v1'}
            />
{newKind === 'anthropic' ? (
            <p className="input-hint">
              与 Claude Code 的 <code>ANTHROPIC_BASE_URL</code> 一致，可不带 /v1
            </p>
            ) : (
            <p className="input-hint">
              OpenAI 兼容 API 地址，通常以 <code>/v1</code> 结尾
            </p>
            )}
          </div>
          <div className="form-group">
            <label>
              API Key <span className="optional-tag">(可选)</span>
            </label>
            <input
              type="password"
              value={newApiKey}
              onChange={(e) => setNewApiKey(e.target.value)}
              placeholder="sk-..."
            />
            <p className="input-hint">某些本地服务可能不需要 Key，可留空</p>
          </div>
        </Modal>
      )}

      {/* 编辑接口弹窗 */}
      {showEditModal && (
        <Modal
          open
          title="编辑接口"
          onClose={() => setShowEditModal(false)}
          closeOnOverlay={false}
          footer={
            <button className="btn-save" onClick={saveEditProvider} disabled={editSaving}>
              {editSaving ? '保存中...' : '保存'}
            </button>
          }
        >
          {editError && <div className="form-error">{editError}</div>}
          <div className="form-group">
            <label>接口类型</label>
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
            <input
              type="text"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              placeholder="如: 本地 Ollama"
            />
          </div>
          <div className="form-group">
            <label>API 地址</label>
            <input
              type="text"
              value={editBaseUrl}
              onChange={(e) => setEditBaseUrl(e.target.value)}
              placeholder={editKind === 'anthropic' ? 'https://api.anthropic.com' : 'http://gateway.internal:8002/v1'}
            />
{editKind === 'anthropic' ? (
            <p className="input-hint">
              与 Claude Code 的 <code>ANTHROPIC_BASE_URL</code> 一致，可不带 /v1
            </p>
            ) : (
            <p className="input-hint">
              OpenAI 兼容 API 地址，通常以 <code>/v1</code> 结尾
            </p>
            )}
          </div>
          <div className="form-group">
            <label>
              API Key <span className="optional-tag">(可选)</span>
            </label>
            <input
              type="password"
              value={editApiKey}
              onChange={(e) => setEditApiKey(e.target.value)}
              placeholder="sk-..."
            />
            <p className="input-hint">某些本地服务可能不需要 Key，可留空</p>
          </div>
        </Modal>
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
          <div className="confirm-text">
            确定要删除接口 <strong>{deleteTarget.name}</strong> 吗？
          </div>
        </Modal>
      )}
    </div>
  );
}
