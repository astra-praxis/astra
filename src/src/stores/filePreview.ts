/**
 * Agent 页文件预览面板的全局状态。
 *
 * Agent 产出单文件产物（写文件 / 文档生成等）时，聊天内产物条目点击
 * 「预览」调用 openFile，在对话框右侧的文件预览面板（FilePreviewPanel）
 * 内打开：文本直出、Markdown 渲染、代码高亮、图片 / 视频 / PDF 内嵌，
 * 不支持的类型（docx 等）引导「使用默认应用打开」。
 *
 * 单窗口语义：全局只有一个预览面板实例，打开新文件即替换当前内容
 * （不并存多个面板）。
 */
import { create } from 'zustand';
import { useWorkspaceStore } from './workspace';

/** 面板宽度范围与默认值（拖拽分隔条时 clamp），与链接预览面板一致 */
export const FILE_PREVIEW_WIDTH = { min: 320, max: 920, default: 480 } as const;

/** 绝对路径判定：POSIX `/`、Windows 盘符 `C:\`、UNC `\\server`；file:// 等协议前缀不算 */
function isAbsolutePath(p: string): boolean {
  if (/^[A-Za-z]:[\\/]/.test(p) || p.startsWith('/') || p.startsWith('\\\\')) return true;
  return false;
}

/**
 * 相对路径解析：以当前活跃工作空间为基准 join 成绝对路径。
 * 解析不了（无基准）返回原值——预览面板读取失败时自会给出引导卡片，不在此静默丢路径。
 * 分隔符跟随基准目录（Windows 基准用 \ join），Rust 侧 std 对混用分隔符也能正确处理。
 */
function resolveAgainstWorkspace(path: string): string {
  if (isAbsolutePath(path) || path.includes('://')) return path;
  const base = useWorkspaceStore.getState().activePath;
  if (!base) return path;
  const sep = base.includes('\\') ? '\\' : '/';
  const trimmed = base.endsWith('/') || base.endsWith('\\') ? base.slice(0, -1) : base;
  return `${trimmed}${sep}${path}`;
}

interface FilePreviewState {
  open: boolean;
  /** 当前预览的文件绝对路径 */
  path: string;
  /** 每次打开 / 刷新自增；path 相同但内容被 Agent 重写时靠它触发重读 */
  version: number;
  /** 预览面板宽度（px），拖拽分隔条调整；开合面板后保持 */
  width: number;
  openFile: (path: string) => void;
  reload: () => void;
  close: () => void;
  setWidth: (w: number) => void;
}

export const useFilePreviewStore = create<FilePreviewState>((set) => ({
  open: false,
  path: '',
  version: 0,
  width: FILE_PREVIEW_WIDTH.default,

  openFile: (rawPath) => {
    // Agent 产物 location / 链接文本可能是相对路径：先解析成绝对路径再入 store
    const path = resolveAgainstWorkspace(rawPath);
    set((s) => ({ open: true, path, version: s.version + 1 }));
  },
  reload: () => set((s) => ({ version: s.version + 1 })),
  close: () => set({ open: false }),
  setWidth: (w) =>
    set(() => ({
      width: Math.max(
        FILE_PREVIEW_WIDTH.min,
        Math.min(FILE_PREVIEW_WIDTH.max, Math.round(w)),
      ),
    })),
}));
