/**
 * 收藏的开发模式状态（全局快捷入口）
 */
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

export interface DevModeFavorite {
  path: string;
  name: string;
}

interface FavoriteState {
  favorite: DevModeFavorite | null;
  loading: boolean;
  load: () => Promise<void>;
  set: (path: string | null, name?: string) => Promise<void>;
  clear: () => Promise<void>;
  isFavorite: (path: string) => boolean;
}

export const useFavoriteStore = create<FavoriteState>((set, get) => ({
  favorite: null,
  loading: false,

  load: async () => {
    if (get().favorite !== null) return; // 已加载
    set({ loading: true });
    try {
      const settings = await invoke<{ favorite_dev_mode?: DevModeFavorite | null }>('get_settings');
      set({ favorite: settings.favorite_dev_mode ?? null });
    } catch {
      set({ favorite: null });
    } finally {
      set({ loading: false });
    }
  },

  set: async (path, name) => {
    try {
      const settings = await invoke<{ favorite_dev_mode?: DevModeFavorite | null }>('set_favorite_dev_mode', {
        path,
        name: name ?? null,
      });
      set({ favorite: settings.favorite_dev_mode ?? null });
    } catch (e) {
      console.error('设置收藏失败:', e);
    }
  },

  clear: async () => {
    await get().set(null);
  },

  isFavorite: (path: string) => get().favorite?.path === path,
}));
