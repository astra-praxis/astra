/**
 * 设置中心状态层（zustand，模块级单例）
 *
 * 三类配置的保存方式（见 settings.types.ts）：
 * - 主题 / Agent 行为：即时生效（分别在 theme store 与 agent store，不经此 store）
 * - 工作空间：操作后立即执行（add/remove/set_active 均即时 invoke）
 * - 其余普通偏好：本 store 的草稿机制 —— 修改只写 draft，统一由保存栏提交
 *
 * 草稿驻留内存：切换分类、离开设置页都不丢失，返回后仍可继续编辑或放弃。
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type {
  AppSettings,
  EditorSetting,
  GenerationConfigState,
  ProviderInfo,
  SettingsValues,
  WorkspaceConfig,
} from '../settings.types';

interface SettingsState {
  loaded: boolean;
  loadError: string;
  /** 最近一次成功保存的值，作为脏检查基线 */
  baseline: SettingsValues | null;
  draft: SettingsValues | null;
  editors: EditorSetting[];
  providers: ProviderInfo[];
  workspaces: WorkspaceConfig[];
  activeWorkspace: string | null;
  saving: boolean;
  savedFlash: boolean;
  saveError: string;

  load: () => Promise<void>;
  update: (patch: Partial<SettingsValues>) => void;
  discard: () => void;
  save: () => Promise<void>;
  /** 工作空间即时操作返回新 AppSettings 后同步列表 */
  applyWorkspaceResult: (s: AppSettings) => void;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  loaded: false,
  loadError: '',
  baseline: null,
  draft: null,
  editors: [],
  providers: [],
  workspaces: [],
  activeWorkspace: null,
  saving: false,
  savedFlash: false,
  saveError: '',

  async load() {
    if (get().loaded) return;
    try {
      const [settings, presetEditors, gen, providerList] = await Promise.all([
        invoke<AppSettings>('get_settings'),
        invoke<EditorSetting[]>('get_preset_editors'),
        invoke<GenerationConfigState>('generation_get_config'),
        invoke<ProviderInfo[]>('list_providers').catch(() => [] as ProviderInfo[]),
      ]);

      // 当前编辑器是否为预设（否则视为自定义命令）
      const matched = presetEditors.find((e) => e.command === settings.editor.command);
      const values: SettingsValues = {
        editorName: matched ? matched.name : settings.editor.name || '自定义',
        editorCommand: settings.editor.command,
        useCustomEditor: !matched,
        scanDepth: settings.scan_depth || 3,
        // 旧设置只配了 search_endpoint 而未选引擎 → 实际走 SearXNG，如实回显
        searchEngine: settings.search_engine ?? (settings.search_endpoint ? 'searxng' : 'bing'),
        searchEndpoint: settings.search_endpoint ?? '',
        memoryAgentSummary: settings.memory_agent_summary ?? false,
        memorySummaryProviderId: settings.memory_summary_provider_id ?? null,
        genConfig: gen,
      };

      set({
        loaded: true,
        baseline: values,
        draft: { ...values },
        editors: presetEditors,
        providers: providerList,
        workspaces: settings.workspaces,
        activeWorkspace: settings.active_workspace,
      });
    } catch (e) {
      console.error('加载设置失败:', e);
      set({ loadError: String(e) });
    }
  },

  update(patch) {
    const draft = get().draft;
    if (!draft) return;
    set({ draft: { ...draft, ...patch }, saveError: '', savedFlash: false });
  },

  discard() {
    const baseline = get().baseline;
    if (!baseline) return;
    set({ draft: { ...baseline }, saveError: '', savedFlash: false });
  },

  async save() {
    const { draft, workspaces, activeWorkspace } = get();
    if (!draft || get().saving) return;

    const command = draft.editorCommand.trim();
    if (!command) {
      set({ saveError: '默认编辑器命令不能为空，请先在「默认编辑器」中选择或填写。' });
      return;
    }

    set({ saving: true, saveError: '' });
    try {
      await invoke('save_settings', {
        settings: {
          editor: {
            name: draft.useCustomEditor ? '自定义' : draft.editorName,
            command,
          },
          workspaces,
          active_workspace: activeWorkspace,
          scan_depth: draft.scanDepth,
          search_engine: draft.searchEngine,
          search_endpoint: draft.searchEndpoint.trim() || null,
          memory_agent_summary: draft.memoryAgentSummary,
          memory_summary_provider_id: draft.memorySummaryProviderId,
        },
      });
      await invoke('generation_set_config', { config: draft.genConfig });

      // 保存值即新基线（搜索地址按保存时的 trim 归一，避免基线与草稿永久不等）
      set({
        baseline: { ...draft, searchEndpoint: draft.searchEndpoint.trim() },
        saving: false,
        savedFlash: true,
      });
      window.setTimeout(() => {
        if (get().savedFlash) set({ savedFlash: false });
      }, 2000);
    } catch (e) {
      console.error('保存设置失败:', e);
      // 保留草稿，用户可重试或放弃
      set({ saving: false, saveError: `保存失败：${e}` });
    }
  },

  applyWorkspaceResult(result) {
    set({
      workspaces: result.workspaces,
      activeWorkspace: result.active_workspace ?? null,
    });
  },
}));

/** 脏检查：草稿与基线是否一致（搜索地址按 trim 归一，与保存逻辑一致） */
export function settingsEqual(a: SettingsValues, b: SettingsValues): boolean {
  return (
    a.editorName === b.editorName &&
    a.editorCommand === b.editorCommand &&
    a.useCustomEditor === b.useCustomEditor &&
    a.scanDepth === b.scanDepth &&
    a.searchEngine === b.searchEngine &&
    a.searchEndpoint.trim() === b.searchEndpoint.trim() &&
    a.memoryAgentSummary === b.memoryAgentSummary &&
    a.memorySummaryProviderId === b.memorySummaryProviderId &&
    JSON.stringify(a.genConfig) === JSON.stringify(b.genConfig)
  );
}
