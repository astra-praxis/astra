/**
 * 主题管理：亮/暗/跟随系统
 * - 状态用 zustand 管理，持久化到 localStorage（key: theme-mode，与旧版兼容）
 * - initTheme() 必须在 createRoot().render() 之前调用，否则刷新闪白
 */
import { create } from 'zustand';

export type ThemeMode = 'light' | 'dark' | 'system';
export type EffectiveTheme = 'light' | 'dark';

interface ThemeState {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
}

export const useThemeStore = create<ThemeState>((set, get) => ({
  mode: 'system',
  setMode: (mode) => {
    set({ mode });
    localStorage.setItem('theme-mode', mode);
    applyTheme(getEffectiveTheme());
  },
}));

function getSystemTheme(): EffectiveTheme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/** 当前生效的主题（light 或 dark） */
export function getEffectiveTheme(): EffectiveTheme {
  const mode = useThemeStore.getState().mode;
  return mode === 'system' ? getSystemTheme() : mode;
}

function applyTheme(theme: EffectiveTheme) {
  document.documentElement.setAttribute('data-theme', theme);
}

/** 初始化主题：读取 localStorage、监听系统主题变化（渲染前调用一次） */
export function initTheme() {
  const saved = localStorage.getItem('theme-mode') as ThemeMode | null;
  if (saved && ['light', 'dark', 'system'].includes(saved)) {
    useThemeStore.setState({ mode: saved });
  }
  applyTheme(getEffectiveTheme());

  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (useThemeStore.getState().mode === 'system') {
      applyTheme(getEffectiveTheme());
    }
  });
}
