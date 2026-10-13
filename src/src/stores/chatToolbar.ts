/**
 * 对话页工具栏共享状态
 * ChatToolbar（标题栏）与 Chat 页面之间共享模型/Provider 选择与回调。
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

// ===== 类型 =====
export interface RunningModelInfo {
  name: string;
  port: number;
  status: string;
  pid: number | null;
  model_name?: string;
  model_path?: string;
  started_at?: number;
}

export interface ProviderConfig {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  active_model: string | null;
}

export interface ModelInfo {
  id: string;
  name?: string;
}

export type ToolbarSourceType = 'provider' | 'model';

interface ChatToolbarState {
  sourceType: ToolbarSourceType;
  selectedProviderId: string | null;
  selectedModelPort: number | null;
  overrideModelName: string | null;
  isSending: boolean;
  showSettings: boolean;

  providers: ProviderConfig[];
  providerModels: ModelInfo[];
  runningModels: RunningModelInfo[];
  modelsLoading: boolean;
  sessionsCount: number;

  // 回调（由 chat 页面设置）
  onClear: () => void;
  onToggleSidebar: () => void;

  // actions
  setSourceType: (type: ToolbarSourceType) => void;
  selectProvider: (id: string) => void;
  setSelectedModelPort: (port: number | null) => void;
  setOverrideModelName: (name: string | null) => void;
  setIsSending: (v: boolean) => void;
  setShowSettings: (v: boolean) => void;
  setSessionsCount: (n: number) => void;
  setOnClear: (fn: () => void) => void;
  setOnToggleSidebar: (fn: () => void) => void;
  fetchProviderModels: () => Promise<void>;
  refreshRunningModels: () => Promise<void>;
  loadProviders: () => Promise<void>;
  loadInitial: () => Promise<void>;
}

export const useChatToolbarStore = create<ChatToolbarState>((set, get) => ({
  sourceType: 'provider',
  selectedProviderId: null,
  selectedModelPort: null,
  overrideModelName: null,
  isSending: false,
  showSettings: false,

  providers: [],
  providerModels: [],
  runningModels: [],
  modelsLoading: false,
  sessionsCount: 0,

  onClear: () => {},
  onToggleSidebar: () => {},

  setSourceType: (type) => set({ sourceType: type }),
  selectProvider: (id) => {
    set({ selectedProviderId: id, overrideModelName: null });
    // 切换 provider 后自动拉取模型并回退到该 provider 的 active_model
    get().fetchProviderModels();
  },
  setSelectedModelPort: (port) => set({ selectedModelPort: port }),
  setOverrideModelName: (name) => set({ overrideModelName: name }),
  setIsSending: (v) => set({ isSending: v }),
  setShowSettings: (v) => set({ showSettings: v }),
  setSessionsCount: (n) => set({ sessionsCount: n }),
  setOnClear: (fn) => set({ onClear: fn }),
  setOnToggleSidebar: (fn) => set({ onToggleSidebar: fn }),

  fetchProviderModels: async () => {
    const { providers, selectedProviderId } = get();
    const provider = providers.find((p) => p.id === selectedProviderId);
    if (!provider) return;
    set({ modelsLoading: true });
    try {
      const models = await invoke<ModelInfo[]>('fetch_provider_models', {
        baseUrl: provider.base_url,
        apiKey: provider.api_key,
      });
      const validIds = new Set(models.map((m) => m.id));
      const override = get().overrideModelName;
      // 如果 overrideModelName 不在新列表里，用 provider 的 active_model 或第一个模型回退
      if (!override || !validIds.has(override)) {
        set({
          providerModels: models,
          overrideModelName:
            provider.active_model && validIds.has(provider.active_model)
              ? provider.active_model
              : models[0]?.id ?? null,
        });
      } else {
        set({ providerModels: models });
      }
    } catch (e) {
      console.error('获取模型失败:', e);
    } finally {
      set({ modelsLoading: false });
    }
  },

  refreshRunningModels: async () => {
    try {
      set({ runningModels: await invoke<RunningModelInfo[]>('list_running_servers') });
    } catch (e) {
      console.error('刷新运行模型失败:', e);
    }
  },

  loadProviders: async () => {
    try {
      const providers = await invoke<ProviderConfig[]>('list_providers');
      set({ providers });
      if (providers.length > 0 && !get().selectedProviderId) {
        set({ selectedProviderId: providers[0].id });
        get().fetchProviderModels();
      }
    } catch (e) {
      console.error('加载 providers 失败:', e);
    }
  },

  /** 加载初始数据（providers + running models），应用启动时调用一次 */
  loadInitial: async () => {
    await Promise.all([get().loadProviders(), get().refreshRunningModels()]);

    // 有运行中的本地模型时优先切到 model 来源
    const { runningModels, selectedModelPort } = get();
    if (runningModels.length > 0) {
      set({
        sourceType: 'model',
        selectedModelPort: selectedModelPort ?? runningModels[0].port,
      });
    }
  },
}));
