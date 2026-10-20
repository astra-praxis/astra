/**
 * 工作空间设置：操作后立即执行的例外路径。
 * 列表与活跃项由 useSettingsStore 托管（save_settings 需要一并提交），
 * 本 hook 只封装三个即时 Tauri 操作。
 */
import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore } from './useSettings';
import type { AppSettings } from '../settings.types';

export function useWorkspaceSettings() {
  const workspaces = useSettingsStore((s) => s.workspaces);
  const activeWorkspace = useSettingsStore((s) => s.activeWorkspace);
  const applyWorkspaceResult = useSettingsStore((s) => s.applyWorkspaceResult);

  const addWorkspace = useCallback(async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择工作空间目录',
    });
    if (!selected) return;

    // 提取文件夹名作为默认名称
    const name = selected.split('/').filter(Boolean).pop() || selected;
    const result = await invoke<AppSettings>('add_workspace', { name, path: selected });
    applyWorkspaceResult(result);
  }, [applyWorkspaceResult]);

  const removeWorkspace = useCallback(
    async (path: string) => {
      const result = await invoke<AppSettings>('remove_workspace', { path });
      applyWorkspaceResult(result);
    },
    [applyWorkspaceResult],
  );

  const setActiveWorkspace = useCallback(
    async (path: string) => {
      const result = await invoke<AppSettings>('set_active_workspace', { path });
      applyWorkspaceResult(result);
    },
    [applyWorkspaceResult],
  );

  return { workspaces, activeWorkspace, addWorkspace, removeWorkspace, setActiveWorkspace };
}
