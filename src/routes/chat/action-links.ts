import { invoke } from "@tauri-apps/api/core";
import { workspaceStore } from "$lib/workspace.svelte";

/**
 * 拦截 AI 回复中的 action:// 链接点击，执行对应的 Tauri 命令。
 * 通过直接操作链接元素的 class/textContent 反馈执行状态：
 * action-executed（执行中）→ action-done / action-error。
 *
 * 已实现命令：
 * - open_project?path=xxx  用配置的编辑器打开项目
 * - run_command?cmd=xxx    执行 shell 命令（需确认）
 */
export async function handleActionClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const link = target.closest('a[href^="action://"]');
    if (!link) return;

    // 防止重复点击
    if (
        link.classList.contains("action-executed") ||
        link.classList.contains("action-done") ||
        link.classList.contains("action-error")
    ) {
        return;
    }

    e.preventDefault();
    const href = link.getAttribute("href") || "";
    const url = new URL(href);
    const actionType = url.hostname; // action://open_project?path=xxx -> hostname is 'open_project'
    const params = Object.fromEntries(url.searchParams);

    if (actionType === "open_project" && params.path) {
        if (!workspaceStore.editor.command) {
            link.textContent = "⚠ 请先在设置页配置编辑器";
            link.classList.add("action-error");
            return;
        }

        // 标记为正在执行
        link.textContent = "⏳ 正在打开...";
        link.classList.add("action-executed");

        try {
            await invoke("open_in_editor", {
                path: params.path,
                editorCommand: workspaceStore.editor.command,
            });
            link.textContent = `✓ 已用 ${workspaceStore.editor.name} 打开`;
            link.classList.add("action-done");
        } catch (err) {
            link.textContent = `❌ ${String(err).substring(0, 50)}`;
            link.classList.add("action-error");
        }
    } else if (actionType === "run_command" && params.cmd) {
        // 通过 action 链接执行命令
        link.textContent = "⏳ 正在执行...";
        link.classList.add("action-executed");
        try {
            await invoke("run_command", {
                command: params.cmd,
                cwd: params.cwd || workspaceStore.activePath || undefined,
                timeoutSecs: 300,
            });
            link.textContent = "✓ 执行完成";
            link.classList.add("action-done");
        } catch (err) {
            link.textContent = `❌ ${String(err).substring(0, 50)}`;
            link.classList.add("action-error");
        }
    }
}
