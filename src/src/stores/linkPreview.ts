/**
 * Agent 对话链接预览面板的全局状态。
 *
 * Agent 回复中的 http(s) 链接被 MarkdownMessage 拦截后调用 openLink，
 * 在对话框右侧的 iframe 面板（LinkPreviewPanel）内打开，而不是让
 * Tauri webview 整页跳转。
 *
 * stack + index 构成简单的浏览历史，支持面板内的后退 / 前进。
 */
import { create } from 'zustand';

interface LinkPreviewState {
  open: boolean;
  /** 浏览历史栈；当前展示的地址为 stack[index] */
  stack: string[];
  index: number;
  /** 预览面板宽度（px），拖拽分隔条调整；开合面板后保持 */
  width: number;
  openLink: (url: string) => void;
  back: () => void;
  forward: () => void;
  close: () => void;
  setWidth: (w: number) => void;
}

/** 面板宽度范围与默认值（拖拽分隔条时 clamp） */
export const PREVIEW_WIDTH = { min: 320, max: 920, default: 480 } as const;

/** 补全裸域名（无 scheme）为 https://，非法地址返回 null */
export function normalizePreviewUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const candidate = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  try {
    const url = new URL(candidate);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    return url.href;
  } catch {
    return null;
  }
}

export const useLinkPreviewStore = create<LinkPreviewState>((set) => ({
  open: false,
  stack: [],
  index: -1,
  width: PREVIEW_WIDTH.default,

  openLink: (url) => {
    const normalized = normalizePreviewUrl(url);
    if (!normalized) return;
    set((s) => {
      // 与当前地址相同：只确保面板打开
      if (s.stack[s.index] === normalized) return { open: true };
      // 历史中已存在同地址：复用并截断其后的前进分支
      const existed = s.stack.lastIndexOf(normalized, s.index);
      if (existed !== -1) return { open: true, index: existed, stack: s.stack.slice(0, s.index + 1) };
      return { open: true, stack: [...s.stack.slice(0, s.index + 1), normalized], index: s.index + 1 };
    });
  },

  back: () => set((s) => ({ index: Math.max(0, s.index - 1) })),
  forward: () => set((s) => ({ index: Math.min(s.stack.length - 1, s.index + 1) })),
  close: () => set({ open: false }),
  setWidth: (w) =>
    set(() => ({
      width: Math.max(PREVIEW_WIDTH.min, Math.min(PREVIEW_WIDTH.max, Math.round(w))),
    })),
}));
