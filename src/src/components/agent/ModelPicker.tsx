/**
 * Agent 对话框模型选择器（分组弹层式）
 *
 * 纯显示当前选择（本地模型 / Provider 模型），点击弹出分组列表：
 * 「本地模型」组（运行中的 llama-server）+ 每个 Provider 一组（打开时懒加载模型列表）。
 * 选择写入 chatToolbar store（sourceType / selectedModelPort / selectedProviderId /
 * overrideModelName），与 agent_start_run 的载荷语义对齐。
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Activity, ChevronDown, Cpu, Loader2, RefreshCw, Search, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useChatToolbarStore, type ModelInfo } from '@/stores/chatToolbar';
import './ModelPicker.scss';

/** Provider → 模型列表（弹层打开时逐个懒加载；'loading' 占位） */
type ProviderModelsState = Record<string, ModelInfo[] | 'loading'>;

export default function ModelPicker() {
  const sourceType = useChatToolbarStore((s) => s.sourceType);
  const selectedProviderId = useChatToolbarStore((s) => s.selectedProviderId);
  const selectedModelPort = useChatToolbarStore((s) => s.selectedModelPort);
  const overrideModelName = useChatToolbarStore((s) => s.overrideModelName);
  const providers = useChatToolbarStore((s) => s.providers);
  const runningModels = useChatToolbarStore((s) => s.runningModels);
  const isSending = useChatToolbarStore((s) => s.isSending);

  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const [providerModels, setProviderModels] = useState<ProviderModelsState>({});
  // 弹层内搜索：过滤本地模型与各 Provider 的模型（标题/模型 id，大小写不敏感）
  const [query, setQuery] = useState('');

  const selectedProvider = providers.find((p) => p.id === selectedProviderId) ?? null;

  // 搜索命中的本地模型与 Provider 模型（组内无命中则整组隐藏）
  const q = query.trim().toLowerCase();
  const hitLocal = useMemo(
    () => runningModels.filter((m) => !q || `${m.name}:${m.port}`.toLowerCase().includes(q)),
    [runningModels, q],
  );
  const hitProviders = useMemo(
    () =>
      providers
        .map((p) => {
          const models = providerModels[p.id];
          if (!q) {
            // 无搜索词：空列表的组整组隐藏（加载中/未知保留，等加载完再定）
            return Array.isArray(models) && models.length === 0 ? null : { provider: p, models };
          }
          // Provider 名不命中且模型列表已知 → 直接过滤；加载中/未知 → 保留（等加载完再筛）
          if (models === undefined || models === 'loading') {
            return p.name.toLowerCase().includes(q) ? { provider: p, models } : null;
          }
          const hit = models.filter((m) => m.id.toLowerCase().includes(q));
          return hit.length > 0 ? { provider: p, models: hit } : null;
        })
        .filter((g): g is { provider: (typeof providers)[number]; models: ModelInfo[] | 'loading' } => g !== null),
    [providers, providerModels, q],
  );

  // ===== 显示态（纯展示，点击弹层选择） =====
  const label = useMemo(() => {
    if (sourceType === 'model') {
      const m = runningModels.find((x) => x.port === selectedModelPort);
      return m ? `${m.name} :${m.port}` : '本地模型';
    }
    const modelName = overrideModelName ?? selectedProvider?.active_model ?? null;
    return `${selectedProvider?.name ?? '未选择 Provider'}${modelName ? ` · ${modelName}` : ''}`;
  }, [sourceType, selectedModelPort, overrideModelName, selectedProvider, runningModels]);

  // ===== 点击外部关闭 =====
  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  // ===== 弹层打开时刷新数据（本地模型 + 逐 Provider 懒加载模型列表），并聚焦搜索框 =====
  useEffect(() => {
    if (!open) return;
    setQuery('');
    // 等弹层渲染完成后聚焦，直接可打字搜索
    requestAnimationFrame(() => searchRef.current?.focus());
    void useChatToolbarStore.getState().refreshRunningModels();
    for (const p of providers) {
      setProviderModels((prev) => ({ ...prev, [p.id]: 'loading' }));
      invoke<ModelInfo[]>('fetch_provider_models', { baseUrl: p.base_url, apiKey: p.api_key })
        .then((models) => setProviderModels((prev) => ({ ...prev, [p.id]: models })))
        .catch(() => setProviderModels((prev) => ({ ...prev, [p.id]: [] })));
    }
  }, [open, providers]);

  function pickLocal(port: number) {
    const st = useChatToolbarStore.getState();
    st.setSourceType('model');
    st.setSelectedModelPort(port);
    setOpen(false);
  }

  function pickProviderModel(providerId: string, modelId: string) {
    const st = useChatToolbarStore.getState();
    // selectProvider 会重置 overrideModelName 并触发后台拉取；随后显式指定所选模型
    st.setSourceType('provider');
    st.selectProvider(providerId);
    st.setOverrideModelName(modelId);
    setOpen(false);
  }

  return (
    <div className="mpick" ref={rootRef}>
      <button
        type="button"
        className="mpick__box"
        onClick={() => setOpen((o) => !o)}
        disabled={isSending}
        title="切换模型（本地模型 / API Provider）"
      >
        {sourceType === 'model' ? (
          <Cpu size={13} strokeWidth={2} />
        ) : (
          <Activity size={13} strokeWidth={2} />
        )}
        <span className="mpick__label">{label}</span>
        <ChevronDown size={12} strokeWidth={2.2} className={`mpick__chevron${open ? ' is-open' : ''}`} />
      </button>

      {open && (
        <div className="mpick__pop" role="listbox" aria-label="选择模型">
          <div className="mpick__search">
            <Search size={12} strokeWidth={2} />
            <input
              ref={searchRef}
              value={query}
              placeholder="搜索模型 / Provider…"
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  if (query) setQuery('');
                  else setOpen(false);
                }
              }}
            />
            {query && (
              <button type="button" className="mpick__search-clear" onClick={() => setQuery('')} aria-label="清空搜索">
                <X size={11} strokeWidth={2.2} />
              </button>
            )}
          </div>

          <div className="mpick__scroll">
          {hitLocal.length > 0 && (
            <div className="mpick__group">
              <div className="mpick__group-name">
                本地模型
                <button
                  type="button"
                  className="mpick__refresh"
                  onClick={() => void useChatToolbarStore.getState().refreshRunningModels()}
                  title="刷新运行中的本地模型"
                  aria-label="刷新本地模型"
                >
                  <RefreshCw size={11} strokeWidth={2.2} />
                </button>
              </div>
              {hitLocal.map((m) => {
                const active = sourceType === 'model' && selectedModelPort === m.port;
                return (
                  <button
                    key={m.port}
                    type="button"
                    role="option"
                    aria-selected={active}
                    className={`mpick__item${active ? ' is-active' : ''}`}
                    onClick={() => pickLocal(m.port)}
                  >
                    <Cpu size={13} strokeWidth={1.8} />
                    <span className="mpick__item-label">{m.name} :{m.port}</span>
                  </button>
                );
              })}
            </div>
          )}

          {hitProviders.map(({ provider: p, models }) => {
            const currentModel = overrideModelName ?? p.active_model;
            return (
              <div className="mpick__group" key={p.id}>
                <div className="mpick__group-name">{p.name}</div>
                {models === undefined || models === 'loading' ? (
                  <div className="mpick__hint">
                    <Loader2 size={11} strokeWidth={2.2} className="mpick__spin" />
                    加载中…
                  </div>
                ) : (
                  models.map((m) => {
                    const active =
                      sourceType === 'provider' && selectedProviderId === p.id && currentModel === m.id;
                    return (
                      <button
                        key={m.id}
                        type="button"
                        role="option"
                        aria-selected={active}
                        className={`mpick__item${active ? ' is-active' : ''}`}
                        onClick={() => pickProviderModel(p.id, m.id)}
                      >
                        <Activity size={13} strokeWidth={1.8} />
                        <span className="mpick__item-label mono">{m.id}</span>
                      </button>
                    );
                  })
                )}
              </div>
            );
          })}

          {providers.length === 0 && !q && (
            <div className="mpick__hint">尚未配置 Provider（可在「Providers」页添加）</div>
          )}
          {q && hitLocal.length === 0 && hitProviders.length === 0 && (
            <div className="mpick__empty">没有匹配「{query}」的模型</div>
          )}
          </div>
        </div>
      )}
    </div>
  );
}
