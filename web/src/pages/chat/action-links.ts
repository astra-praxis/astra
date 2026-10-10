/**
 * action:// 链接执行
 * 移植自 src/routes/chat/action-links.ts。
 * 原版通过直接操作链接 DOM（textContent/class）反馈状态；
 * React 版改为纯函数：解析 href + 执行命令 + 返回结果文案，
 * 由 MarkdownMessage 中的 ActionLink 组件驱动 UI 状态。
 *
 * 已实现命令：
 * - open_project?path=xxx  用配置的编辑器打开项目
 * - run_command?cmd=xxx    执行 shell 命令
 */
import { invoke } from '@tauri-apps/api/core';
import { useWorkspaceStore } from '@/stores/workspace';

export interface ActionInfo {
  actionType: string;
  params: Record<string, string>;
}

/**
 * 解析 action:// 链接。
 * action://open_project?path=xxx → hostname 为命令类型。
 * 解析失败返回 null。
 */
export function parseActionHref(href: string): ActionInfo | null {
  try {
    const url = new URL(href);
    return { actionType: url.hostname, params: Object.fromEntries(url.searchParams) };
  } catch {
    return null;
  }
}

export interface ActionResult {
  ok: boolean;
  label: string;
}

function shortenError(err: unknown): string {
  return `❌ ${String(err).substring(0, 50)}`;
}

/** 执行 action 命令，返回结果与展示文案 */
export async function executeAction(info: ActionInfo): Promise<ActionResult> {
  const ws = useWorkspaceStore.getState();

  if (info.actionType === 'open_project' && info.params.path) {
    if (!ws.editor.command) {
      return { ok: false, label: '⚠ 请先在设置页配置编辑器' };
    }
    try {
      await invoke('open_in_editor', {
        path: info.params.path,
        editorCommand: ws.editor.command,
      });
      return { ok: true, label: `✓ 已用 ${ws.editor.name} 打开` };
    } catch (err) {
      return { ok: false, label: shortenError(err) };
    }
  }

  if (info.actionType === 'run_command' && info.params.cmd) {
    try {
      await invoke('run_command', {
        command: info.params.cmd,
        cwd: info.params.cwd || ws.activePath || undefined,
        timeoutSecs: 300,
      });
      return { ok: true, label: '✓ 执行完成' };
    } catch (err) {
      return { ok: false, label: shortenError(err) };
    }
  }

  return { ok: false, label: `❌ 未知命令: ${info.actionType}` };
}
