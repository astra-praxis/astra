<script lang="ts">
    import { tick } from "svelte";
    import { workspaceStore, type ProjectItem } from "$lib/workspace.svelte";
    import MentionPopup from "./MentionPopup.svelte";

    interface Props {
        value: string;
        disabled: boolean;
        canSend: boolean;
        onSend: () => void;
        onStop: () => void;
    }

    let { value = $bindable(), disabled, canSend, onSend, onStop }: Props =
        $props();

    // ===== @ 提及状态 =====
    let textareaEl: HTMLTextAreaElement | null = $state(null);
    let mentionActive = $state(false);
    let mentionQuery = $state("");
    let mentionStartIndex = $state(0); // @ 符号的起始位置
    let mentionSelectedIndex = $state(0); // 当前选中的项目索引（键盘导航）

    const filteredProjects = $derived.by(() => {
        const q = mentionQuery.toLowerCase();
        if (!q) return workspaceStore.projects.slice(0, 10); // 无搜索词时显示前10个
        // 有搜索词时搜索全部项目，最多显示20个匹配结果
        return workspaceStore.projects
            .filter((p) => p.name.toLowerCase().includes(q))
            .slice(0, 20);
    });

    function handleKeydown(e: KeyboardEvent) {
        // 当 @ 提及弹出时，处理键盘导航
        if (mentionActive) {
            // 如果列表为空，不处理方向键
            if (
                filteredProjects.length === 0 &&
                (e.key === "ArrowDown" ||
                    e.key === "ArrowUp" ||
                    e.key === "Enter" ||
                    e.key === "Tab")
            ) {
                e.preventDefault();
                return;
            }

            if (e.key === "ArrowDown") {
                e.preventDefault();
                // 边界检查：确保 selectedIndex 不超过列表长度
                mentionSelectedIndex = Math.min(
                    mentionSelectedIndex + 1,
                    filteredProjects.length - 1,
                );
                return;
            }
            if (e.key === "ArrowUp") {
                e.preventDefault();
                mentionSelectedIndex = Math.max(mentionSelectedIndex - 1, 0);
                return;
            }
            if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                if (filteredProjects.length > 0 && mentionSelectedIndex >= 0) {
                    selectMentionProject(
                        filteredProjects[mentionSelectedIndex],
                    );
                }
                return;
            }
            if (e.key === "Escape") {
                e.preventDefault();
                mentionActive = false;
                return;
            }
        }

        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSend();
        }
    }

    /** 监听 textarea 输入，检测 @ 提及 */
    function handleTextareaInput() {
        if (!textareaEl) return;

        const val = textareaEl.value;
        const cursorPos = textareaEl.selectionStart;

        if (!val || cursorPos === 0) {
            mentionActive = false;
            return;
        }

        // 从光标位置向前查找最近的 @ 符号
        const beforeCursor = val.substring(0, cursorPos);
        // 匹配模式：@ 后面可以跟字母/数字/中文/点/连字符（停止在空格、换行等处）
        // \u4e00-\u9fff 匹配常用中文字符，\w 匹配字母数字下划线
        const atMatch = beforeCursor.match(/(?:^|\s)@([\w\u4e00-\u9fff.-]*)$/);

        if (atMatch) {
            mentionActive = true;
            mentionStartIndex =
                cursorPos -
                atMatch[0].length +
                (atMatch[0].startsWith("@") ? 0 : 1);
            mentionQuery = atMatch[1]; // @ 后面的搜索词
            mentionSelectedIndex = 0;
        } else {
            mentionActive = false;
        }
    }

    /** 选择项目，插入到输入框 */
    function selectMentionProject(project: ProjectItem) {
        if (!textareaEl) return;

        const val = textareaEl.value;
        const cursorPos = textareaEl.selectionStart;

        // 替换 @xxx 为 @项目名 并在后面加空格
        const beforeMention = val.substring(0, mentionStartIndex);
        const afterCursor = val.substring(cursorPos);
        const insertion = `@${project.name} `;
        const newValue = beforeMention + insertion + afterCursor;

        value = newValue;
        mentionActive = false;

        // 更新光标位置到插入文本之后
        tick().then(() => {
            if (textareaEl) {
                textareaEl.focus();
                const newCursorPos = beforeMention.length + insertion.length;
                textareaEl.setSelectionRange(newCursorPos, newCursorPos);
            }
        });
    }

    /** 鼠标点击选择项目 */
    function clickMentionProject(project: ProjectItem, index: number) {
        mentionSelectedIndex = index;
        selectMentionProject(project);
    }
</script>

<!-- 弹窗与输入框为兄弟节点（配合父级 .dock 布局） -->
{#if mentionActive && workspaceStore.projects.length > 0}
    <MentionPopup
        query={mentionQuery}
        projects={filteredProjects}
        selectedIndex={mentionSelectedIndex}
        onSelect={clickMentionProject}
    />
{/if}

<div class="composer" class:sending={disabled}>
    <textarea
        bind:this={textareaEl}
        bind:value
        onkeydown={handleKeydown}
        oninput={handleTextareaInput}
        disabled={disabled}
        rows="2"
        placeholder={disabled
            ? "生成中…"
            : "问点什么，输入 @ 可提及项目"}></textarea>
    <div class="composer-side">
        <div class="composer-hint">
            <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd> 换行
        </div>
        {#if disabled}
            <button class="stop-btn" onclick={onStop}>
                <span class="stop-square"></span>
                停止
            </button>
        {:else}
            <button
                class="send-btn"
                onclick={onSend}
                disabled={!canSend}
                aria-label="发送"
            >
                <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><line x1="5" y1="12" x2="19" y2="12" /><polyline
                        points="12 5 19 12 12 19"
                    /></svg
                >
            </button>
        {/if}
    </div>
</div>

<style>
    .composer {
        max-width: var(--dock-max, 760px);
        margin: 0 auto;
        background: var(--bg-card);
        border: 1.5px solid var(--border);
        border-radius: 18px;
        padding: 12px 14px 8px 16px;
        display: flex;
        flex-direction: column;
        gap: 8px;
        transition: all 0.2s;
        box-shadow: 0 4px 24px var(--shadow-md);
    }

    .composer:focus-within {
        border-color: var(--accent);
        box-shadow:
            0 0 0 4px var(--accent-ring),
            0 6px 28px var(--shadow-md);
    }

    .composer.sending {
        opacity: 0.92;
    }

    .composer textarea {
        width: 100%;
        border: none;
        background: var(--bg-card);
        color: var(--text-primary);
        font-size: 14.5px;
        font-family: inherit;
        line-height: 1.5;
        resize: none;
        outline: none;
        min-height: 24px;
        box-sizing: border-box;
        caret-color: var(--accent);
    }

    .composer textarea::placeholder {
        color: var(--text-placeholder);
        opacity: 1;
    }

    .composer-side {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }

    .composer-hint {
        font-size: 11px;
        color: var(--text-muted);
    }

    .composer-hint kbd {
        display: inline-block;
        padding: 1px 5px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 4px;
        font-size: 10px;
        font-family: ui-monospace, monospace;
        color: var(--text-secondary);
    }

    .send-btn {
        width: 34px;
        height: 34px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        color: white;
        cursor: pointer;
        transition: all 0.18s;
        flex-shrink: 0;
    }

    .send-btn:hover:not(:disabled) {
        background: var(--accent-hover);
        transform: scale(1.06);
    }

    .send-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
        transform: none;
    }

    .stop-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 7px 14px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 10px;
        color: var(--error-text);
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.15s;
        flex-shrink: 0;
    }

    .stop-btn:hover {
        background: var(--error-hover-bg);
    }

    .stop-square {
        width: 10px;
        height: 10px;
        background: var(--error-text);
        border-radius: 2px;
    }

    @media (max-width: 640px) {
        .composer-hint {
            display: none;
        }
    }
</style>
