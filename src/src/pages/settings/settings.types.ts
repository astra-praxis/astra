/**
 * 设置中心类型定义（与后端 Tauri 命令对齐）
 */

/** 生成供应商配置（与后端 agent_generation.rs GenerationConfig 对齐） */
export interface GenerationConfigState {
  enabled: boolean;
  base_url: string;
  api_key: string;
  model_id: string;
}

export interface EditorSetting {
  name: string;
  command: string;
}

export interface WorkspaceConfig {
  name: string;
  path: string;
}

/** get_settings / save_settings 载荷（与后端 AppSettings 对齐） */
export interface AppSettings {
  editor: EditorSetting;
  workspaces: WorkspaceConfig[];
  active_workspace: string | null;
  scan_depth: number;
  search_engine?: string | null;
  search_endpoint?: string | null;
  memory_agent_summary?: boolean;
  memory_summary_provider_id?: string | null;
}

/** 供应商配置（list_providers 返回，与后端 ProviderConfig 对齐） */
export interface ProviderInfo {
  id: string;
  name: string;
  base_url: string;
  active_model: string | null;
}

/**
 * 统一草稿保存的可编辑设置：
 * save_settings（编辑器、扫描深度、搜索、记忆）+ generation_set_config（图片生成）。
 * 主题与 Agent 行为（localStorage 即时生效）不在此列。
 */
export interface SettingsValues {
  editorName: string;
  editorCommand: string;
  useCustomEditor: boolean;
  scanDepth: number;
  searchEngine: string;
  searchEndpoint: string;
  memoryAgentSummary: boolean;
  memorySummaryProviderId: string | null;
  genConfig: GenerationConfigState;
}
