/**
 * node_modules 扫描/清理全局状态
 * 标题栏、工具箱、工作空间详情共用。
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// ===== 类型 =====
export interface NodeModulesInfo {
  path: string;
  project_path: string;
  size_bytes: number;
  size_display: string;
  project_name: string;
  has_pnpm_lock: boolean;
}

export interface CleanResult {
  success: boolean;
  cleaned_paths: string[];
  failed_paths: [string, string][];
  total_freed_bytes: number;
  total_freed_display: string;
}

interface NmState {
  scanning: boolean;
  cleaning: boolean;
  error: string;
  progress: string;
  activePath: string | null;
  cancelled: boolean; // 用户点击停止时标记
  stopScan: () => void;
  startScan: (workspacePath: string) => Promise<NodeModulesInfo[] | undefined>;
  cleanSingle: (path: string) => Promise<string[]>;
  cleanAll: (paths: string[]) => Promise<string[] | undefined>;
}

/** 路径 → 扫描结果缓存（非响应式） */
export const nmCache = new Map<string, NodeModulesInfo[]>();

export const useNmStore = create<NmState>((set, get) => ({
  scanning: false,
  cleaning: false,
  error: '',
  progress: '',
  activePath: null,
  cancelled: false,

  /** 停止扫描 */
  stopScan: () => {
    set({ cancelled: true, progress: '正在停止...' });
  },

  /** 扫描指定工作空间下的 node_modules */
  startScan: async (workspacePath: string) => {
    if (get().scanning) return;
    set({ cancelled: false, scanning: true, error: '', progress: '正在扫描 node_modules...', activePath: workspacePath });
    // 监听后端实时推送的扫描进度（当前文件夹名）
    const unlisten = await listen<string>('nm-scan-progress', (e) => {
      if (!useNmStore.getState().cancelled) {
        set({ progress: `正在扫描: ${e.payload}` });
      }
    });
    try {
      const result = await invoke<NodeModulesInfo[]>('scan_node_modules', {
        workspacePath,
        maxDepth: 15,
      });
      if (get().cancelled) {
        set({ progress: '已取消' });
        return [];
      }
      nmCache.set(workspacePath, result);
      set({ progress: '' });
      return result;
    } catch (e) {
      set({ error: `扫描失败: ${e}`, progress: '' });
      return [];
    } finally {
      unlisten();
      set({ scanning: false, cancelled: false, activePath: null });
    }
  },

  /** 清理单个 */
  cleanSingle: async (path: string) => {
    set({ cleaning: true, error: '', progress: '正在删除...' });
    try {
      const result = await invoke<CleanResult>('clean_node_modules', { paths: [path] });
      if (result.failed_paths.length > 0) {
        set({ error: `删除失败: ${result.failed_paths[0][1]}` });
      }
      return result.cleaned_paths;
    } catch (e) {
      set({ error: `删除失败: ${e}` });
      return [];
    } finally {
      set({ cleaning: false, progress: '' });
    }
  },

  /** 清理全部 */
  cleanAll: async (paths: string[]) => {
    if (paths.length === 0 || get().cleaning) return;
    set({ cleaning: true, error: '', progress: `正在清理 ${paths.length} 个 node_modules...` });
    try {
      const result = await invoke<CleanResult>('clean_node_modules', { paths });
      if (result.failed_paths.length > 0) {
        set({ error: `${result.failed_paths.length} 个目录删除失败` });
      }
      set({ progress: '' });
      return result.cleaned_paths;
    } catch (e) {
      set({ error: `清理失败: ${e}`, progress: '' });
      return [];
    } finally {
      set({ cleaning: false });
    }
  },
}));

/** busy 状态便捷读取（非组件场景） */
export function getNmBusy(): boolean {
  return useNmStore.getState().scanning || useNmStore.getState().cleaning;
}
