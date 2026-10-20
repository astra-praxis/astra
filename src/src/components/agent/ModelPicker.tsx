/**
 * Agent 对话框模型选择器（分组弹层式）
 *
 * 纯显示当前选择（本地模型 / Provider 模型），点击弹出分组列表：
 * 「本地模型」组（运行中的 llama-server）+ 每个 Provider 一组（打开时懒加载模型列表）。
 * 选择写入 chatToolbar store（sourceType / selectedModelPort / selectedProviderId /
 * overrideModelName），与 agent_start_run 的载荷语义对齐。
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Activity, ChevronDown, Cpu, History, Loader2, RefreshCw, Search, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useChatToolbarStore, type ModelInfo } from '@/stores/chatToolbar';
import './ModelPicker.scss';

/** Provider → 模型列表（弹层打开时逐个懒加载；'loading' 占位） */
type ProviderModelsState = Record<string, ModelInfo[] | 'loading'>;

/** 最近使用条目（localStorage 持久化，跨会话保留） */
type RecentEntry =
  | { t: 'model'; port: number; name: string }
  | { t: 'provider'; pid: string; mid: string; pname?: string };

const RECENT_KEY = 'astra-agent-mpick-recent';
const RECENT_MAX = 6;
/** 分组收起状态（localStorage 持久化，跨会话记忆） */
const COLLAPSED_KEY = 'astra-agent-mpick-collapsed';

function loadRecents(): RecentEntry[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const parsed = raw ? (JSON.parse(raw) as RecentEntry[]) : [];
    return Array.isArray(parsed) ? parsed.slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

function loadCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    const parsed = raw ? (JSON.parse(raw) as string[]) : [];
    return new Set(Array.isArray(parsed) ? parsed : []);
  } catch {
    return new Set();
  }
}

function pushRecent(entry: RecentEntry) {
  const rest = loadRecents().filter((r) => {
    if (r.t !== entry.t) return true;
    if (r.t === 'model' && entry.t === 'model') return r.port !== entry.port;
    if (r.t === 'provider' && entry.t === 'provider') {
      return !(r.pid === entry.pid && r.mid === entry.mid);
    }
    return true;
  });
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([entry, ...rest].slice(0, RECENT_MAX)));
  } catch {
    /* 忽略持久化失败：最近使用只是加速，不影响选择 */
  }
}

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
  // 最近使用（打开弹层时读取一次；选择后更新）
  const [recents, setRecents] = useState<RecentEntry[]>([]);
  // 分组收起状态（localStorage 持久化；key：provider id / 'local'）
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  // 键盘指向：可见 .mpick__item 序列中的下标（命令式高亮，避免随渲染重建）
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const kbdIdxRef = useRef(-1);

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
    setRecents(loadRecents());
    setCollapsed(loadCollapsed());
    // 等弹层渲染完成后聚焦，直接可打字搜索
    requestAnimationFrame(() => searchRef.current?.focus());
    void useChatToolbarStore.getState().refreshRunningModels();
    for (const p of providers) {
      setProviderModels((prev) => ({ ...prev, [p.id]: 'loading' }));
      invoke<ModelInfo[]>('fetch_provider_models', {
        baseUrl: p.base_url,
        apiKey: p.api_key,
        kind: p.kind,
      })
        .then((models) => setProviderModels((prev) => ({ ...prev, [p.id]: models })))
        .catch(() => setProviderModels((prev) => ({ ...prev, [p.id]: [] })));
    }
  }, [open, providers]);

  /** 展开/收起分组；状态持久化到 localStorage */
  function toggleGroup(id: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      try {
        localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
      } catch {
        /* 忽略持久化失败 */
      }
      return next;
    });
  }

  function pickLocal(port: number, name?: string) {
    const st = useChatToolbarStore.getState();
    st.setSourceType('model');
    st.setSelectedModelPort(port);
    pushRecent({ t: 'model', port, name: name ?? `本地模型 :${port}` });
    setOpen(false);
  }

  function pickProviderModel(providerId: string, modelId: string) {
    const st = useChatToolbarStore.getState();
    // selectProvider 会重置 overrideModelName 并触发后台拉取；随后显式指定所选模型
    st.setSourceType('provider');
    st.selectProvider(providerId);
    st.setOverrideModelName(modelId);
    pushRecent({
      t: 'provider',
      pid: providerId,
      mid: modelId,
      pname: providers.find((p) => p.id === providerId)?.name,
    });
    setOpen(false);
  }

  // ===== 键盘导航：上下在可见项间移动，回车选中，Esc 关闭 =====
  function visibleItems(): HTMLButtonElement[] {
    return Array.from(
      scrollRef.current?.querySelectorAll<HTMLButtonElement>('.mpick__item') ?? [],
    );
  }

  function clearPointed() {
    scrollRef.current
      ?.querySelectorAll('.mpick__item.is-pointed')
      .forEach((el) => el.classList.remove('is-pointed'));
    kbdIdxRef.current = -1;
  }

  function pointAt(items: HTMLButtonElement[], idx: number) {
    scrollRef.current
      ?.querySelectorAll('.mpick__item.is-pointed')
      .forEach((el) => el.classList.remove('is-pointed'));
    const el = items[idx];
    if (el) {
      el.classList.add('is-pointed');
      el.scrollIntoView({ block: 'nearest' });
    }
    kbdIdxRef.current = idx;
  }

  function onSearchKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const items = visibleItems();
      if (items.length === 0) return;
      const prev = kbdIdxRef.current;
      const next =
        e.key === 'ArrowDown'
          ? prev + 1 >= items.length
            ? 0
            : prev + 1
          : prev - 1 < 0
            ? items.length - 1
            : prev - 1;
      pointAt(items, next);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const items = visibleItems();
      if (items.length === 0) return;
      const idx = kbdIdxRef.current >= 0 && kbdIdxRef.current < items.length ? kbdIdxRef.current : 0;
      items[idx].scrollIntoView({ block: 'nearest' });
      items[idx].click();
    } else if (e.key === 'Escape') {
      if (query) setQuery('');
      else setOpen(false);
    }
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
              placeholder="搜索模型 / Provider，↑↓ 选择，回车确认"
              onChange={(e) => {
                setQuery(e.target.value);
                clearPointed();
              }}
              onKeyDown={onSearchKeyDown}
            />
            {query && (
              <button type="button" className="mpick__search-clear" onClick={() => setQuery('')} aria-label="清空搜索">
                <X size={11} strokeWidth={2.2} />
              </button>
            )}
          </div>

          <div className="mpick__scroll" ref={scrollRef}>
          {/* 最近使用：置顶直达，仅无搜索词时展示；失效条目（provider 已删 /
              本地模型已停）自动隐藏 */}
          {!q && recents.length > 0 && (
            <div className="mpick__group">
              <div className="mpick__group-name">最近使用</div>
              {recents.map((r) => {
                if (r.t === 'model') {
                  const stillRunning = runningModels.some((m) => m.port === r.port);
                  if (!stillRunning) return null;
                  const active = sourceType === 'model' && selectedModelPort === r.port;
                  return (
                    <button
                      key={`recent-model-${r.port}`}
                      type="button"
                      role="option"
                      aria-selected={active}
                      className={`mpick__item${active ? ' is-active' : ''}`}
                      onClick={() => pickLocal(r.port, r.name)}
                    >
                      <History size={13} strokeWidth={1.8} />
                      <span className="mpick__item-label">{r.name}</span>
                    </button>
                  );
                }
                const p = providers.find((x) => x.id === r.pid);
                if (!p) return null;
                const active =
                  sourceType === 'provider' && selectedProviderId === r.pid
                    ? (overrideModelName ?? p.active_model) === r.mid
                    : false;
                return (
                  <button
                    key={`recent-${r.pid}-${r.mid}`}
                    type="button"
                    role="option"
                    aria-selected={active}
                    className={`mpick__item${active ? ' is-active' : ''}`}
                    onClick={() => pickProviderModel(r.pid, r.mid)}
                  >
                    <History size={13} strokeWidth={1.8} />
                    <span className="mpick__item-label mono">
                      {r.pname ? `${r.pname} · ` : ''}
                      {r.mid}
                    </span>
                  </button>
                );
              })}
            </div>
          )}

          {hitLocal.length > 0 && (
            <div className="mpick__group">
              <button
                type="button"
                className="mpick__group-name"
                onClick={() => toggleGroup('local')}
                aria-expanded={!collapsed.has('local')}
              >
                <ChevronDown
                  size={12}
                  strokeWidth={2.2}
                  className={`mpick__group-chev${collapsed.has('local') ? '' : ' is-open'}`}
                />
                本地模型
                <span className="mpick__group-count">{hitLocal.length}</span>
                <span
                  role="button"
                  tabIndex={0}
                  className="mpick__refresh"
                  onClick={(e) => {
                    e.stopPropagation();
                    void useChatToolbarStore.getState().refreshRunningModels();
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      e.stopPropagation();
                      void useChatToolbarStore.getState().refreshRunningModels();
                    }
                  }}
                  title="刷新运行中的本地模型"
                  aria-label="刷新本地模型"
                >
                  <RefreshCw size={11} strokeWidth={2.2} />
                </span>
              </button>
              {!collapsed.has('local') &&
                hitLocal.map((m) => {
                const active = sourceType === 'model' && selectedModelPort === m.port;
                return (
                  <button
                    key={m.port}
                    type="button"
                    role="option"
                    aria-selected={active}
                    className={`mpick__item${active ? ' is-active' : ''}`}
                    onClick={() => pickLocal(m.port, `${m.name} :${m.port}`)}
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
            const isCollapsed = collapsed.has(p.id) && q === '';
            const modelList = Array.isArray(models) ? models : [];
            return (
              <div className="mpick__group" key={p.id}>
                <button
                  type="button"
                  className="mpick__group-name"
                  onClick={() => toggleGroup(p.id)}
                  aria-expanded={!isCollapsed}
                >
                  <ChevronDown
                    size={12}
                    strokeWidth={2.2}
                    className={`mpick__group-chev${isCollapsed ? '' : ' is-open'}`}
                  />
                  {p.name}
                  {modelList.length > 0 && (
                    <span className="mpick__group-count">{modelList.length}</span>
                  )}
                </button>
                {!isCollapsed &&
                  (models === undefined || models === 'loading' ? (
                    <div className="mpick__hint">
                      <Loader2 size={11} strokeWidth={2.2} className="mpick__spin" />
                      加载中…
                    </div>
                  ) : (
                    modelList.map((m) => {
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
                  ))}
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
