/**
 * 统一打开服务（Open Service）——前端唯一的"打开目标"入口。
 *
 * 组件不再直接 import plugin-opener 或各自拼 open_in_editor 调用，
 * 只表达打开意图；路径解析语义在 Rust 侧 open_in_editor 的
 * resolve_open_path 统一（base_dir join + 词法规范化 + 无基准报错），
 * 本层只做目标类型分发：
 *
 * - openWithDefaultApp(path)  系统默认应用打开文件/目录（opener openPath）
 * - openWithEditor(path, ...) 用配置的编辑器打开（自定义 Rust 命令，
 *                             base_dir 必须由调用方给明确基准——通常是
 *                             workspaceStore.activePath）
 * - openInBrowser(url)        默认浏览器打开 URL（opener openUrl）
 *
 * 错误不在此吞：由调用方决定如何展示（面板内提示 / error state 等）。
 */
import { invoke } from '@tauri-apps/api/core';
import { openPath, openUrl } from '@tauri-apps/plugin-opener';

/** 用系统默认应用打开文件或目录（Finder / Explorer / 关联应用） */
export function openWithDefaultApp(path: string): Promise<void> {
  return openPath(path);
}

/** 用配置的编辑器打开路径；相对路径以 baseDir 为基准在 Rust 侧解析 */
export function openWithEditor(
  path: string,
  editorCommand: string,
  baseDir?: string,
): Promise<void> {
  return invoke('open_in_editor', {
    path,
    editorCommand,
    baseDir: baseDir || undefined,
  });
}

/** 用系统默认浏览器打开 URL（http/mailto 等非 file 协议） */
export function openInBrowser(url: string): Promise<void> {
  return openUrl(url);
}
