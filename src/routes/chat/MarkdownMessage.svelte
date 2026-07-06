<script lang="ts">
    import { marked } from "marked";
    import hljs from "highlight.js/lib/core";
    import "highlight.js/styles/github-dark.css";
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy, tick } from "svelte";
    import { workspaceStore } from "$lib/workspace.svelte";
    import javascript from "highlight.js/lib/languages/javascript";
    import typescript from "highlight.js/lib/languages/typescript";
    import python from "highlight.js/lib/languages/python";
    import rust from "highlight.js/lib/languages/rust";
    import css from "highlight.js/lib/languages/css";
    import xml from "highlight.js/lib/languages/xml";
    import bash from "highlight.js/lib/languages/bash";
    import json from "highlight.js/lib/languages/json";
    import sql from "highlight.js/lib/languages/sql";
    import plaintext from "highlight.js/lib/languages/plaintext";

    hljs.registerLanguage("javascript", javascript);
    hljs.registerLanguage("typescript", typescript);
    hljs.registerLanguage("python", python);
    hljs.registerLanguage("rust", rust);
    hljs.registerLanguage("css", css);
    hljs.registerLanguage("html", xml);
    hljs.registerLanguage("xml", xml);
    hljs.registerLanguage("bash", bash);
    hljs.registerLanguage("sh", bash);
    hljs.registerLanguage("json", json);
    hljs.registerLanguage("sql", sql);
    hljs.registerLanguage("text", plaintext);
    hljs.registerLanguage("plaintext", plaintext);

    // 配置 marked
    marked.setOptions({
        breaks: true,
        gfm: true,
    });

    let { content = "", workspacePath = "", isFresh = false } = $props();
    let containerEl: HTMLElement | null = null;
    let rendered = $state("");

    // ===== 命令执行状态 =====
    interface CommandExecution {
        id: string;
        preElement: HTMLPreElement;
        outputElement: HTMLElement;
        status: "idle" | "confirming" | "running" | "done" | "error";
        output: string[];
    }
    // 命令执行状态（非响应式，DOM 操作手动处理）
    let executions: CommandExecution[] = [];
    let unlisteners: Array<() => void> = [];

    // 渲染 markdown
    $effect(() => {
        if (!content) {
            rendered = "";
            return;
        }
        rendered = marked.parse(content) as string;
    });

    // 渲染后处理
    $effect(() => {
        if (!rendered || !containerEl) return;
        tick().then(() => {
            if (!containerEl) return;
            // 高亮代码块 + 按钮处理
            containerEl.querySelectorAll("pre").forEach((pre) => {
                const codeEl = pre.querySelector("code") as HTMLElement | null;
                if (!codeEl) return;
                const lang =
                    codeEl.className.match(/language-(\w+)/)?.[1] || "";
                const isShell = ["bash", "sh", "shell"].includes(lang);

                // 跳过已处理的
                if (pre.querySelector(".code-x-btn")) return;

                hljs.highlightElement(codeEl);

                // 复制按钮
                const copyBtn = document.createElement("button");
                copyBtn.className = "code-x-btn code-copy-btn";
                copyBtn.innerHTML =
                    '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg> 复制';
                copyBtn.addEventListener("click", async () => {
                    const code = codeEl.textContent ?? "";
                    try {
                        await navigator.clipboard.writeText(code);
                        copyBtn.textContent = "✓ 已复制";
                        copyBtn.classList.add("copied");
                        setTimeout(() => {
                            copyBtn.innerHTML =
                                '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg> 复制';
                            copyBtn.classList.remove("copied");
                        }, 2000);
                    } catch {
                        copyBtn.textContent = "复制失败";
                    }
                });
                pre.prepend(copyBtn);

                // execute 按钮（仅 shell 代码块）
                if (isShell) {
                    const execBtn = document.createElement("button");
                    execBtn.className = "code-x-btn code-exec-btn";
                    execBtn.innerHTML =
                        '<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg> 执行';
                    execBtn.title = "执行该命令";
                    execBtn.addEventListener("click", () =>
                        handleExecClick(pre, codeEl.textContent ?? ""),
                    );
                    pre.prepend(execBtn);
                }
            });

            // 自动执行 action 链接
            autoExecuteActionLinks();
        });
    });

    // ===== 自动执行 action 链接 =====
    function autoExecuteActionLinks() {
        if (!containerEl || !isFresh) return; //仅对新消息自动执行
        containerEl.querySelectorAll('a[href^="action://"]').forEach((link) => {
            const actionLink = link as HTMLAnchorElement;
            // 跳过已自动执行过的链接
            if (actionLink.dataset.autoExecuted) return;
            actionLink.dataset.autoExecuted = "1";

            const href = actionLink.getAttribute("href") || "";
            const url = new URL(href);
            const actionType = url.hostname; // 'open_project', 'run_command'
            const params = Object.fromEntries(url.searchParams);

            if (actionType === "open_project" && params.path) {
                // 检查编辑器是否配置
                if (!workspaceStore.editor.command) {
                    actionLink.textContent = "⚠ 请先在设置页配置编辑器";
                    actionLink.classList.add("action-error");
                    return;
                }

                // 标记正在执行
                actionLink.textContent = "⏳ 正在打开...";
                actionLink.classList.add("action-executed");

                // 自动打开项目
                invoke("open_in_editor", {
                    path: params.path,
                    editorCommand: workspaceStore.editor.command,
                })
                    .then(() => {
                        actionLink.textContent = `✓ 已用 ${workspaceStore.editor.name} 打开`;
                        actionLink.classList.remove("action-executed");
                        actionLink.classList.add("action-done");
                    })
                    .catch((err: any) => {
                        actionLink.textContent = `❌ ${String(err).substring(0, 50)}`;
                        actionLink.classList.remove("action-executed");
                        actionLink.classList.add("action-error");
                    });
            } else if (actionType === "run_command" && params.cmd) {
                // 自动执行命令（可选：这里也可以自动执行 run_command）
                // 根据需求决定是否自动执行命令
                // actionLink.textContent = '⏳ 正在执行...';
                // actionLink.classList.add('action-executed');
                // invoke('run_command', {
                //     command: params.cmd,
                //     cwd: params.cwd || workspacePath || undefined,
                //     timeoutSecs: 300,
                // }).then(() => {
                //     actionLink.textContent = '✓ 执行完成';
                //     actionLink.classList.remove('action-executed');
                //     actionLink.classList.add('action-done');
                // }).catch((err: any) => {
                //     actionLink.textContent = `❌ ${String(err).substring(0, 50)}`;
                //     actionLink.classList.remove('action-executed');
                //     actionLink.classList.add('action-error');
                // });
            }
        });
    }

    // ===== 命令执行处理 =====
    async function handleExecClick(pre: HTMLPreElement, command: string) {
        const existingExec = executions.find((e) => e.preElement === pre);
        if (
            existingExec &&
            (existingExec.status === "running" ||
                existingExec.status === "confirming")
        ) {
            return;
        }

        // 清理旧的输出元素
        const oldOutput = pre.nextElementSibling;
        if (oldOutput?.classList.contains("command-output")) {
            oldOutput.remove();
        }
        if (existingExec) {
            const idx = executions.indexOf(existingExec);
            if (idx !== -1) executions.splice(idx, 1);
        }

        // 创建输出元素
        const outputElement = document.createElement("div");
        outputElement.className = "command-output";
        outputElement.innerHTML =
            '<div class="output-header"><span class="output-title">命令输出</span><button class="output-close">×</button></div><div class="output-body"><span class="loading">⏳</span></div>';
        pre.after(outputElement);

        const closeBtn = outputElement.querySelector(
            ".output-close",
        ) as HTMLButtonElement;
        closeBtn?.addEventListener("click", () => {
            outputElement.remove();
            const idx = executions.findIndex(
                (e) => e.outputElement === outputElement,
            );
            if (idx !== -1) executions.splice(idx, 1);
        });

        // 创建执行上下文
        const execId = crypto.randomUUID();
        const exec: CommandExecution = {
            id: execId,
            preElement: pre,
            outputElement,
            status: "confirming",
            output: [],
        };
        executions.push(exec);

        // 显示确认框
        showConfirmDialog(outputElement, command, exec);
    }

    function showConfirmDialog(
        outputEl: HTMLElement,
        command: string,
        exec: CommandExecution,
    ) {
        const body = outputEl.querySelector(".output-body") as HTMLElement;
        if (!body) return;

        body.innerHTML = `
      <div class="confirm-dialog">
        <div class="confirm-title">⚠️ 确认执行命令？</div>
        <pre class="confirm-preview">${escapeHtml(command)}</pre>
        <div class="confirm-actions">
          <button class="btn-cancel">取消</button>
          <button class="btn-confirm">允许执行</button>
        </div>
      </div>
    `;

        const btnConfirm = body.querySelector(
            ".btn-confirm",
        ) as HTMLButtonElement;
        const btnCancel = body.querySelector(
            ".btn-cancel",
        ) as HTMLButtonElement;

        btnCancel.addEventListener("click", () => {
            outputEl.remove();
            const idx = executions.indexOf(exec);
            if (idx !== -1) executions.splice(idx, 1);
        });

        btnConfirm.addEventListener("click", () => {
            executeCommand(exec, command);
        });
    }

    async function executeCommand(exec: CommandExecution, command: string) {
        exec.status = "running";
        exec.output = [];

        const body = exec.outputElement.querySelector(
            ".output-body",
        ) as HTMLElement;
        if (!body) return;

        body.innerHTML =
            '<div class="output-status running">正在执行...</div><div class="output-lines"></div>';
        const linesEl = body.querySelector(".output-lines") as HTMLElement;

        try {
            await invoke("run_command", {
                command,
                cwd: workspacePath || undefined,
                timeoutSecs: 300,
            });
        } catch (err) {
            finishExecution(exec, false, String(err));
            return;
        }
    }

    function finishExecution(
        exec: CommandExecution,
        success: boolean,
        errorMsg?: string,
    ) {
        exec.status = success ? "done" : "error";

        const body = exec.outputElement.querySelector(
            ".output-body",
        ) as HTMLElement;
        if (!body) return;

        const linesEl = body.querySelector(".output-lines") as HTMLElement;
        if (linesEl) {
            if (errorMsg) {
                const line = document.createElement("div");
                line.className = "output-line error";
                line.textContent = `❌ ${errorMsg}`;
                linesEl.appendChild(line);
            }
        }

        const statusEl = body.querySelector(".output-status") as HTMLElement;
        if (statusEl) {
            statusEl.className = `output-status ${exec.status}`;
            statusEl.textContent = success ? "✓ 执行完成" : "✗ 执行失败";
        }
    }

    function appendOutput(
        exec: CommandExecution,
        line: string,
        type: "stdout" | "stderr" | "error",
    ) {
        const linesEl = exec.outputElement.querySelector(
            ".output-lines",
        ) as HTMLElement;
        if (!linesEl) return;

        const lineEl = document.createElement("div");
        lineEl.className = `output-line ${type}`;
        lineEl.textContent = line;
        linesEl.appendChild(lineEl);
        linesEl.scrollTop = linesEl.scrollHeight;
    }

    function escapeHtml(text: string): string {
        return text
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;");
    }

    // ===== 事件监听 =====
    onMount(() => {
        const unStdout = listen<string>("command-stdout", (e) => {
            if (executions.length === 0) return;
            // 广播给所有正在执行的任务（简化，可改成按 id 匹配）
            executions.forEach((exec) => {
                if (exec.status === "running") {
                    appendOutput(exec, e.payload, "stdout");
                }
            });
        });

        const unStderr = listen<string>("command-stderr", (e) => {
            if (executions.length === 0) return;
            executions.forEach((exec) => {
                if (exec.status === "running") {
                    appendOutput(exec, e.payload, "stderr");
                }
            });
        });

        const unDone = listen("command-done", (): void => {
            if (executions.length === 0) return;
            // 标记所有正在执行的任务成功
            executions.forEach((exec) => {
                if (exec.status === "running") {
                    finishExecution(exec, true);
                }
            });
        });

        const unError = listen<string>("command-error", (e) => {
            if (executions.length === 0) return;
            executions.forEach((exec) => {
                if (exec.status === "running") {
                    finishExecution(exec, false, e.payload);
                }
            });
        });

        Promise.all([unStdout, unStderr, unDone, unError]).then((unsubs) => {
            unlisteners = [...unsubs];
        });
    });

    onDestroy(() => {
        unlisteners.forEach((fn) => fn());
    });
</script>

<div class="md-content" bind:this={containerEl}>
    {#if rendered}
        {@html rendered}
    {:else}
        <span class="md-empty">{content}</span>
    {/if}
</div>

<style>
    .md-content {
        line-height: 1.7;
        font-size: 14.5px;
        min-width: 0;
        max-width: 100%;
        overflow-wrap: break-word;
    }

    .md-content :global(p) {
        margin: 0 0 8px;
    }

    .md-content :global(p:last-child) {
        margin-bottom: 0;
    }

    .md-content :global(h1),
    .md-content :global(h2),
    .md-content :global(h3),
    .md-content :global(h4) {
        margin: 16px 0 8px;
        font-weight: 600;
        color: var(--text-primary);
    }

    .md-content :global(h1) {
        font-size: 1.3em;
    }
    .md-content :global(h2) {
        font-size: 1.15em;
    }
    .md-content :global(h3) {
        font-size: 1.05em;
    }

    .md-content :global(ul),
    .md-content :global(ol) {
        margin: 4px 0 8px;
        padding-left: 20px;
    }

    .md-content :global(li) {
        margin-bottom: 2px;
    }

    .md-content :global(blockquote) {
        margin: 8px 0;
        padding: 4px 12px;
        border-left: 3px solid var(--accent);
        color: var(--text-secondary);
        background: var(--bg-subtle);
        border-radius: 0 6px 6px 0;
    }

    .md-content :global(code) {
        font-family: ui-monospace, "SF Mono", Monaco, monospace;
        font-size: 0.88em;
        padding: 1px 5px;
        background: var(--bg-subtle);
        border-radius: 4px;
        color: var(--accent);
    }

    .md-content :global(pre) {
        margin: 10px 0;
        border-radius: 10px;
        overflow: hidden;
        background: #0f0f1a;
        position: relative;
        max-width: 100%;
    }

    .md-content :global(pre code) {
        display: block;
        padding: 14px 16px;
        background: transparent;
        font-size: 13px;
        line-height: 1.55;
        overflow-x: auto;
        max-width: 100%;
        color: #e2e8f0;
        font-family: ui-monospace, "SF Mono", Monaco, monospace;
        white-space: pre;
        tab-size: 2;
    }

    /* 按钮样式（统一 .code-x-btn 前缀） */
    .md-content :global(table) {
        border-collapse: collapse;
        width: 100%;
        margin: 8px 0;
        font-size: 13px;
    }

    .md-content :global(th),
    .md-content :global(td) {
        border: 1px solid var(--border);
        padding: 6px 10px;
        text-align: left;
    }

    .md-content :global(th) {
        background: var(--bg-subtle);
        font-weight: 600;
    }

    .md-content :global(hr) {
        border: none;
        border-top: 1px solid var(--border);
        margin: 16px 0;
    }

    .md-content :global(a) {
        color: var(--link);
        text-decoration: underline;
    }

    .md-content :global(a:hover) {
        color: var(--link-hover);
    }

    .md-content :global(img) {
        max-width: 100%;
        border-radius: 8px;
        margin: 8px 0;
    }

    .md-empty {
        white-space: pre-wrap;
        word-break: break-word;
    }

    /* ===== 执行按钮区域样式 ===== */
    .md-content :global(.code-x-btn) {
        position: absolute;
        top: 6px;
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 3px 9px;
        background: rgba(255, 255, 255, 0.08);
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 6px;
        font-size: 11px;
        font-weight: 500;
        color: rgba(255, 255, 255, 0.7);
        cursor: pointer;
        transition: all 0.15s;
        opacity: 0;
        font-family: inherit;
        z-index: 2;
    }

    .md-content :global(.code-exec-btn) {
        right: 68px;
        color: #4ade80;
        border-color: rgba(74, 222, 128, 0.3);
    }

    .md-content :global(.code-exec-btn:hover) {
        background: rgba(74, 222, 128, 0.2);
        color: #6ee7b0;
        border-color: rgba(74, 222, 128, 0.5);
    }

    .md-content :global(.code-copy-btn) {
        right: 6px;
    }

    .md-content :global(pre:hover .code-x-btn) {
        opacity: 1;
    }

    .md-content :global(.code-x-btn:hover) {
        background: rgba(255, 255, 255, 0.15);
        color: white;
    }

    .md-content :global(.code-x-btn.copied) {
        background: rgba(74, 222, 128, 0.15);
        color: #4ade80;
        border-color: rgba(74, 222, 128, 0.3);
    }

    /* ===== 命令输出区域 ===== */
    .md-content :global(.command-output) {
        margin: -4px 0 10px;
        border-radius: 0 0 10px 10px;
        overflow: hidden;
        background: #0b0b18;
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-top: none;
        font-family: ui-monospace, "SF Mono", Monaco, monospace;
        font-size: 13px;
        animation: slideDown 0.2s ease-out;
    }

    .md-content :global(.output-header) {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 12px;
        background: rgba(255, 255, 255, 0.04);
        border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    }

    .md-content :global(.output-title) {
        color: rgba(255, 255, 255, 0.5);
        font-size: 11px;
        font-weight: 500;
        letter-spacing: 0.5px;
        text-transform: uppercase;
    }

    .md-content :global(.output-close) {
        background: none;
        border: none;
        color: rgba(255, 255, 255, 0.3);
        cursor: pointer;
        font-size: 16px;
        padding: 0 4px;
        line-height: 1;
        transition: color 0.15s;
    }

    .md-content :global(.output-close:hover) {
        color: rgba(255, 255, 255, 0.8);
    }

    .md-content :global(.output-body) {
        padding: 8px 12px;
        max-height: 400px;
        overflow-y: auto;
    }

    .md-content :global(.output-status) {
        font-size: 11px;
        margin-bottom: 6px;
        letter-spacing: 0.3px;
    }

    .md-content :global(.output-status.running) {
        color: #60a5fa;
    }

    .md-content :global(.output-status.done) {
        color: #4ade80;
    }

    .md-content :global(.output-status.error) {
        color: #f87171;
    }

    .md-content :global(.output-lines) {
        max-height: 300px;
        overflow-y: auto;
        font-size: 12px;
        line-height: 1.5;
    }

    .md-content :global(.output-line) {
        padding: 1px 0;
        white-space: pre-wrap;
        word-break: break-all;
        color: #e2e8f0;
    }

    .md-content :global(.output-line.stderr) {
        color: #fca5a5;
    }

    .md-content :global(.output-line.error) {
        color: #f87171;
        padding: 4px 0;
    }

    .md-content :global(.loading) {
        display: inline-block;
        animation: pulse 1.2s ease-in-out infinite;
        font-size: 16px;
    }

    /* ===== 确认对话框 ===== */
    .md-content :global(.confirm-dialog) {
        padding: 4px 0;
    }

    .md-content :global(.confirm-title) {
        font-size: 13px;
        font-weight: 600;
        color: #fbbf24;
        margin-bottom: 8px;
    }

    .md-content :global(.confirm-preview) {
        background: rgba(0, 0, 0, 0.3);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 6px;
        padding: 8px 10px;
        font-family: ui-monospace, "SF Mono", Monaco, monospace;
        font-size: 12px;
        color: #e2e8f0;
        white-space: pre-wrap;
        word-break: break-all;
        margin-bottom: 10px;
        max-height: 160px;
        overflow-y: auto;
    }

    .md-content :global(.confirm-actions) {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
    }

    .md-content :global(.confirm-actions button) {
        padding: 6px 16px;
        border-radius: 6px;
        border: none;
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        transition: all 0.15s;
        font-family: inherit;
    }

    .md-content :global(.btn-cancel) {
        background: rgba(255, 255, 255, 0.08);
        color: rgba(255, 255, 255, 0.7);
        border: 1px solid rgba(255, 255, 255, 0.12);
    }

    .md-content :global(.btn-cancel:hover) {
        background: rgba(255, 255, 255, 0.15);
        color: white;
    }

    .md-content :global(.btn-confirm) {
        background: #4ade80;
        color: #0b0b18;
        font-weight: 600;
    }

    .md-content :global(.btn-confirm:hover) {
        background: #60ef90;
        transform: translateY(-1px);
        box-shadow: 0 2px 8px rgba(74, 222, 128, 0.3);
    }

    @keyframes slideDown {
        from {
            opacity: 0;
            transform: translateY(-8px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    @keyframes pulse {
        0%,
        100% {
            opacity: 0.4;
        }
        50% {
            opacity: 1;
        }
    }
</style>
