/**
 * 收藏的开发模式状态
 * 提供全局快捷入口
 */
import { invoke } from '@tauri-apps/api/core';

export interface DevModeFavorite {
  path: string;
  name: string;
}

class FavoriteStore {
  favorite = $state<DevModeFavorite | null>(null);
  loading = $state(false);

  async load() {
    if (this.favorite !== null) return; // 已加载
    this.loading = true;
    try {
      const settings = await invoke<{ favorite_dev_mode?: DevModeFavorite | null }>('get_settings');
      this.favorite = settings.favorite_dev_mode ?? null;
    } catch {
      this.favorite = null;
    } finally {
      this.loading = false;
    }
  }

  async set(path: string | null, name?: string) {
    try {
      const settings = await invoke<{ favorite_dev_mode?: DevModeFavorite | null }>('set_favorite_dev_mode', {
        path,
        name: name ?? null
      });
      this.favorite = settings.favorite_dev_mode ?? null;
    } catch (e) {
      console.error('设置收藏失败:', e);
    }
  }

  async clear() {
    await this.set(null);
  }

  isFavorite(path: string): boolean {
    return this.favorite?.path === path;
  }
}

export const favoriteStore = new FavoriteStore();