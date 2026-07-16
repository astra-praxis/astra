import { invoke } from '@tauri-apps/api/core';

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

// ===== 全局响应式状态 =====
export const nm = $state({
  scanning: false,
  cleaning: false,
  error: '',
  progress: '',
  activePath: null as string | null,
  cancelled: false, // 用户点击停止时标记
});

export const nmCache = new Map<string, NodeModulesInfo[]>();

// ===== 停止扫描 =====
export function stopScan() {
  nm.cancelled = true;
  nm.progress = '正在停止...';
}

// 导出函数获取 busy 状态（$derived 无法从模块直接导出）
export function getBusy() {
  return nm.scanning || nm.cleaning;
}

// ===== 扫描 =====
export async function startScan(workspacePath: string) {
  if (nm.scanning) return;
  nm.cancelled = false;
  nm.scanning = true;
  nm.error = '';
  nm.progress = '正在扫描 node_modules...';
  nm.activePath = workspacePath;
  try {
    const result = await invoke<NodeModulesInfo[]>('scan_node_modules', {
      workspacePath,
      maxDepth: 15,
    });
    if (nm.cancelled) {
      // 用户已取消，丢弃结果
      nm.progress = '已取消';
      return [];
    }
    nmCache.set(workspacePath, result);
    nm.progress = '';
    return result;
  } catch (e) {
    nm.error = `扫描失败: ${e}`;
    nm.progress = '';
    return [];
  } finally {
    nm.scanning = false;
    nm.cancelled = false;
    nm.activePath = null;
  }
}

// ===== 清理单个 =====
export async function cleanSingle(path: string) {
  nm.cleaning = true;
  nm.error = '';
  nm.progress = '正在删除...';
  try {
    const result = await invoke<CleanResult>('clean_node_modules', { paths: [path] });
    if (result.failed_paths.length > 0) {
      nm.error = `删除失败: ${result.failed_paths[0][1]}`;
    }
    return result.cleaned_paths;
  } catch (e) {
    nm.error = `删除失败: ${e}`;
    return [];
  } finally {
    nm.cleaning = false;
    nm.progress = '';
  }
}

// ===== 清理全部 =====
export async function cleanAll(paths: string[]) {
  if (paths.length === 0 || nm.cleaning) return;
  nm.cleaning = true;
  nm.error = '';
  nm.progress = `正在清理 ${paths.length} 个 node_modules...`;
  try {
    const result = await invoke<CleanResult>('clean_node_modules', { paths });
    if (result.failed_paths.length > 0) {
      nm.error = `${result.failed_paths.length} 个目录删除失败`;
    }
    nm.progress = '';
    return result.cleaned_paths;
  } catch (e) {
    nm.error = `清理失败: ${e}`;
    nm.progress = '';
    return [];
  } finally {
    nm.cleaning = false;
  }
}
