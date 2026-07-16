import { invoke } from '@tauri-apps/api/core';

// ===== 类型 =====
export interface RunningModelInfo {
  name: string;
  port: number;
  status: string;
  pid: number;
  model_name: string;
  model_path: string;
  started_at: number;
}

export interface ProviderConfig {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
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
}

export async function handleFetchModels() {
  const provider = toolbarState.providers.find(p => p.id === toolbarState.selectedProviderId);
  if (!provider) return;
  toolbarState.modelsLoading = true;
  try {
    const models = await invoke<ModelInfo[]>('providers::fetch_provider_models', {
      baseUrl: provider.base_url,
      apiKey: provider.api_key,
    });
    toolbarState.providerModels = models;
    toolbarState.overrideModelName = null;
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
