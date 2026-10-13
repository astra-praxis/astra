/**
 * 全局工作空间状态管理
 *
 * 工作空间页面和聊天页面共享同一份项目数据，
 * 任一页面切换工作空间后，另一页面也会同步。
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

// ===== 类型 =====
export interface ProjectItem {
  name: string;
  path: string;
  has_readme?: boolean;
}

export interface WorkspaceConfig {
  name: string;
  path: string;
}

export interface EditorSetting {
  name: string;
  command: string;
}

interface AppSettings {
  editor: EditorSetting;
  workspaces: WorkspaceConfig[];
  active_workspace: string | null;
}

interface WorkspaceState {
  projects: ProjectItem[];
  workspaces: WorkspaceConfig[];
  activePath: string;
  activeName: string;
  editor: EditorSetting;
  loading: boolean;
  loadFromSettings: () => Promise<void>;
  scanProjects: (path: string) => Promise<void>;
  switchWorkspace: (path: string) => Promise<void>;
  findProjectByName: (name: string) => ProjectItem | undefined;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  projects: [],
  workspaces: [],
  activePath: '',
  activeName: '',
  editor: { name: '', command: '' },
  loading: false,

  /** 从设置加载并自动扫描活动工作空间 */
  loadFromSettings: async () => {
    try {
      const settings = await invoke<AppSettings>('get_settings');
      set({
        workspaces: settings.workspaces ?? [],
        editor: settings.editor ?? { name: '', command: '' },
      });

      if (settings.active_workspace) {
        const ws = (settings.workspaces ?? []).find((w) => w.path === settings.active_workspace);
        set({ activePath: settings.active_workspace, activeName: ws?.name ?? '' });
        await get().scanProjects(settings.active_workspace);
      }
    } catch (e) {
      console.error('加载工作空间失败:', e);
    }
  },

  /** 扫描指定路径下的项目 */
  scanProjects: async (path: string) => {
    set({ loading: true });
    try {
      const projects = await invoke<ProjectItem[]>('scan_workspace', { path });
      set({ projects });
    } catch (e) {
      console.error('扫描项目失败:', e);
      set({ projects: [] });
    } finally {
      set({ loading: false });
    }
  },

  /** 切换活动工作空间 */
  switchWorkspace: async (path: string) => {
    await invoke('set_active_workspace', { path });
    const ws = get().workspaces.find((w) => w.path === path);
    set({ activePath: path, activeName: ws?.name ?? '' });
    await get().scanProjects(path);
  },

  /** 按名称查找项目 */
  findProjectByName: (name: string) => {
    return get().projects.find((p) => p.name.toLowerCase() === name.toLowerCase());
  },
}));
