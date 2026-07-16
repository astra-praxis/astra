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

// ===== 工具栏共享状态 =====
export const toolbarState = $state({
  sourceType: 'provider' as 'provider' | 'model',
  selectedProviderId: null as string | null,
  selectedModelPort: null as number | null,
  overrideModelName: null as string | null,
  isSending: false,
  showSettings: false,

  providers: [] as ProviderConfig[],
  providerModels: [] as ModelInfo[],
  runningModels: [] as RunningModelInfo[],
  modelsLoading: false,
  sessionsCount: 0,

  // 回调（由 chat 页面设置）
  onClear: (() => {}) as () => void,
  onToggleSidebar: (() => {}) as () => void,
});

// ===== 动作 =====
export function onSwitchType(type: 'provider' | 'model') {
  toolbarState.sourceType = type;
}

export function onSelectProviderChange() {
  toolbarState.overrideModelName = null;
  // 切换 provider 后自动拉取模型并回退到该 provider 的 active_model
  handleFetchModels();
}

export async function handleFetchModels() {
  const provider = toolbarState.providers.find(p => p.id === toolbarState.selectedProviderId);
  if (!provider) return;
  toolbarState.modelsLoading = true;
  try {
    const models = await invoke<ModelInfo[]>('fetch_provider_models', {
      baseUrl: provider.base_url,
      apiKey: provider.api_key,
    });
    toolbarState.providerModels = models;

    const validIds = new Set(models.map(m => m.id));
    // 如果 overrideModelName 不在新列表里，用 provider 的 active_model 或第一个模型回退
    if (!toolbarState.overrideModelName || !validIds.has(toolbarState.overrideModelName)) {
      toolbarState.overrideModelName =
        (provider.active_model && validIds.has(provider.active_model))
          ? provider.active_model
          : models[0]?.id ?? null;
    }
  } catch (e) {
    console.error('获取模型失败:', e);
  } finally {
    toolbarState.modelsLoading = false;
  }
}

export async function refreshRunningModels() {
  try {
    toolbarState.runningModels = await invoke<RunningModelInfo[]>('list_running_servers');
  } catch (e) {
    console.error('刷新运行模型失败:', e);
  }
}

/** 加载 providers 列表到 toolbarState */
export async function loadProviders() {
  try {
    const providers = await invoke<ProviderConfig[]>('list_providers');
    toolbarState.providers = providers;
    if (providers.length > 0 && !toolbarState.selectedProviderId) {
      toolbarState.selectedProviderId = providers[0].id;
      handleFetchModels();
    }
  } catch (e) {
    console.error('加载 providers 失败:', e);
  }
}

/** 加载初始数据（providers + running models） */
export async function loadInitialToolbarData() {
  await Promise.all([
    loadProviders(),
    refreshRunningModels(),
  ]);

  // 有运行中的本地模型时优先切到 model 来源
  if (toolbarState.runningModels.length > 0) {
    toolbarState.sourceType = 'model';
    if (!toolbarState.selectedModelPort) {
      toolbarState.selectedModelPort = toolbarState.runningModels[0].port;
    }
  }
}
