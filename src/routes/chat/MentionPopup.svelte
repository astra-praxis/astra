<script lang="ts">
    import type { ProjectItem } from "$lib/workspace.svelte";

    interface Props {
        query: string;
        projects: ProjectItem[];
        selectedIndex: number;
        onSelect: (project: ProjectItem, index: number) => void;
    }

    let { query, projects, selectedIndex, onSelect }: Props = $props();

    let listEl: HTMLElement | null = $state(null);

    // 选中项变化时滚动跟随（键盘导航/初始打开）
    $effect(() => {
        void selectedIndex;
        const selectedEl = listEl?.querySelector(".mention-item.selected");
        if (selectedEl) {
            selectedEl.scrollIntoView({
                block: "nearest",
                behavior: "smooth",
            });
        }
    });
</script>

<div class="mention-popup">
    <div class="mention-header">
        <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><path
                d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2v12z"
            /></svg
        >
        <span>选择项目</span>
        {#if query}
            <span class="mention-query">搜索: {query}</span>
        {/if}
    </div>
    <div class="mention-list" bind:this={listEl}>
        {#if projects.length === 0}
            <div class="mention-empty">没有匹配的项目</div>
        {:else}
            {#each projects as project, i (project.path)}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                    class="mention-item"
                    class:selected={i === selectedIndex}
                    onclick={() => onSelect(project, i)}
                    role="button"
                    tabindex="-1"
                >
                    <span class="mention-item-icon">📁</span>
                    <div class="mention-item-content">
                        <span class="mention-item-name">{project.name}</span>
                        <span
                            class="mention-item-path"
                            title={project.path}>{project.path}</span
                        >
                    </div>
                </div>
            {/each}
        {/if}
    </div>
    <div class="mention-hint">
        <kbd>↑↓</kbd> 选择 · <kbd>Enter</kbd> 确认 · <kbd>Esc</kbd> 关闭
    </div>
</div>

<style>
    .mention-popup {
        max-width: var(--dock-max, 760px);
        margin: 0 auto 8px;
        background: var(--bg-card);
        border: 1.5px solid var(--border);
        border-radius: 14px;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
        overflow: hidden;
        animation: slideUp 0.2s ease;
    }

    @keyframes slideUp {
        from {
            opacity: 0;
            transform: translateY(8px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    .mention-header {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 8px 14px;
        font-size: 12px;
        font-weight: 600;
        color: var(--text-secondary);
        border-bottom: 1px solid var(--border);
        background: var(--bg-subtle);
    }

    .mention-query {
        color: var(--accent);
        font-weight: 600;
    }

    .mention-list {
        max-height: 220px;
        overflow-y: auto;
        padding: 4px;
    }

    .mention-empty {
        padding: 20px 14px;
        text-align: center;
        color: var(--text-muted);
        font-size: 13px;
    }

    .mention-item {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 8px 10px;
        border-radius: 8px;
        cursor: pointer;
        transition: all 0.12s;
    }

    .mention-item:hover {
        background: var(--bg-subtle);
    }

    .mention-item.selected {
        background: var(--accent-bg);
    }

    .mention-item-icon {
        font-size: 16px;
        flex-shrink: 0;
    }

    .mention-item-content {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .mention-item-name {
        font-size: 13px;
        font-weight: 500;
        color: var(--text-primary);
    }

    .mention-item.selected .mention-item-name {
        color: var(--accent);
    }

    .mention-item-path {
        font-size: 11px;
        color: var(--text-muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .mention-hint {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 8px;
        padding: 6px 10px;
        font-size: 11px;
        color: var(--text-muted);
        border-top: 1px solid var(--border);
        background: var(--bg-subtle);
    }

    .mention-hint kbd {
        padding: 1px 5px;
        background: var(--bg-card);
        border: 1px solid rgba(255, 255, 255, 0.2);
        border-radius: 3px;
        font-size: 10px;
        font-family: ui-monospace, monospace;
        color: var(--text-secondary);
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
    }
</style>
