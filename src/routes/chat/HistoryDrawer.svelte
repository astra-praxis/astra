<script lang="ts">
    import type { ProviderConfig } from "$lib/chat-state.svelte";
    import type { ChatSession, ChatSource } from "./types";
    import { formatDayTimeSec } from "$lib/format";

    interface Props {
        open: boolean;
        sessions: ChatSession[];
        currentSessionId: string | null;
        isSending: boolean;
        providers: ProviderConfig[];
        onClose: () => void;
        onNewChat: () => void;
        onSelect: (s: ChatSession) => void;
        onDelete: (id: string) => void;
    }

    let {
        open,
        sessions,
        currentSessionId,
        isSending,
        providers,
        onClose,
        onNewChat,
        onSelect,
        onDelete,
    }: Props = $props();

    // ===== 搜索 =====
    let searchQuery = $state("");

    const filteredSessions = $derived.by(() => {
        if (!searchQuery.trim()) return sessions;
        const q = searchQuery.toLowerCase();
        return sessions.filter((s) => {
            if (s.title.toLowerCase().includes(q)) return true;
            if (s.messages && s.messages.length > 0) {
                for (const m of s.messages) {
                    if (m.content.toLowerCase().includes(q)) return true;
                }
            }
            return false;
        });
    });

    // ===== 展示辅助 =====
    function sourceIcon(s: ChatSource): string {
        if (s.type === "model") return "🖥";
        return "⚡";
    }

    function sourceShortLabel(s: ChatSource): string {
        if (s.type === "model") return `port ${s.port}`;
        const p = providers.find((x) => x.id === s.provider_id);
        return p?.name ?? "provider";
    }

    function handleDelete(id: string, e: Event) {
        e.stopPropagation();
        onDelete(id);
    }
</script>

{#if open}
    <div
        class="drawer-veil"
        onclick={onClose}
        role="presentation"
    ></div>
{/if}
<aside class="history-drawer" class:open={open}>
    <div class="hd-top">
        <span class="hd-title">对话历史</span>
        <button
            class="hd-close"
            onclick={onClose}
            aria-label="关闭"
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
                ><line x1="18" y1="6" x2="6" y2="18" /><line
                    x1="6"
                    y1="6"
                    x2="18"
                    y2="18"
                /></svg
            >
        </button>
    </div>

    <button class="hd-new" onclick={onNewChat} disabled={isSending}>
        <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><line x1="12" y1="5" x2="12" y2="19" /><line
                x1="5"
                y1="12"
                x2="19"
                y2="12"
            /></svg
        >
        新建对话
    </button>

    <div class="hd-search">
        <svg
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><circle cx="11" cy="11" r="8" /><line
                x1="21"
                y1="21"
                x2="16.65"
                y2="16.65"
            /></svg
        >
        <input
            type="text"
            placeholder="搜索对话历史…"
            bind:value={searchQuery}
        />
        {#if searchQuery}
            <button
                class="hd-search-clear"
                onclick={() => (searchQuery = "")}
                aria-label="清除搜索">✕</button
            >
        {/if}
    </div>

    <div class="hd-list">
        {#if filteredSessions.length === 0}
            <div class="hd-empty">
                <span class="hd-empty-icon">✦</span>
                <p>{searchQuery ? "没有匹配的对话" : "暂无历史记录"}</p>
            </div>
        {:else}
            {#each filteredSessions as s (s.id)}
                <div
                    class="hd-item"
                    class:active={s.id === currentSessionId}
                    onclick={() => onSelect(s)}
                    role="button"
                    tabindex="0"
                    onkeydown={(e) => e.key === "Enter" && onSelect(s)}
                >
                    <span class="hd-item-icon">{sourceIcon(s.source)}</span>
                    <div class="hd-item-text">
                        <div class="hd-item-title">{s.title}</div>
                        <div class="hd-item-meta">
                            <span class="hd-item-source"
                                >{sourceShortLabel(s.source)}</span
                            >
                            <span class="hd-item-dot">·</span>
                            <span class="hd-item-time"
                                >{formatDayTimeSec(s.updated_at)}</span
                            >
                        </div>
                    </div>
                    <button
                        class="hd-item-del"
                        onclick={(e) => handleDelete(s.id, e)}
                        aria-label="删除"
                        title="删除"
                    >
                        <svg
                            width="12"
                            height="12"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><polyline points="3 6 5 6 21 6" /><path
                                d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
                        /></svg>
                    </button>
                </div>
            {/each}
        {/if}
    </div>
</aside>

<style>
    .drawer-veil {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.45);
        z-index: 40;
        animation: fadeIn 0.2s ease;
    }

    .history-drawer {
        position: fixed;
        right: 0;
        top: 0;
        bottom: 0;
        z-index: 41;
        width: 280px;
        max-width: 86vw;
        background: var(--bg-card);
        border-left: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        transform: translateX(100%);
        transition: transform 0.28s cubic-bezier(0.32, 0.72, 0, 1);
        box-shadow: -4px 0 24px rgba(0, 0, 0, 0.08);
    }

    .history-drawer.open {
        transform: translateX(0);
        box-shadow: -4px 0 32px rgba(0, 0, 0, 0.15);
    }

    .hd-search {
        display: flex;
        align-items: center;
        gap: 6px;
        margin: 0 12px 8px;
        padding: 6px 10px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 8px;
        transition: border-color 0.15s;
    }

    .hd-search:focus-within {
        border-color: var(--accent);
    }

    .hd-search svg {
        flex-shrink: 0;
        color: var(--text-muted);
    }

    .hd-search input {
        flex: 1;
        border: none;
        background: transparent;
        color: var(--text-primary);
        font-size: 13px;
        font-family: inherit;
        outline: none;
        min-width: 0;
    }

    .hd-search input::placeholder {
        color: var(--text-placeholder);
    }

    .hd-search-clear {
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        font-size: 12px;
        padding: 0 2px;
    }

    .hd-search-clear:hover {
        color: var(--text-primary);
    }

    .hd-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 16px 16px 12px;
    }

    .hd-title {
        font-size: 13px;
        font-weight: 700;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--text-muted);
    }

    .hd-close {
        width: 26px;
        height: 26px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--bg-subtle);
        border: none;
        border-radius: 6px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.15s;
    }

    .hd-close:hover {
        color: var(--accent);
        background: var(--accent-bg);
    }

    .hd-new {
        margin: 0 12px 8px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        padding: 9px 12px;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .hd-new:hover:not(:disabled) {
        background: var(--accent-hover);
        box-shadow: 0 6px 16px var(--accent-shadow);
    }

    .hd-new:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .hd-list {
        flex: 1;
        overflow-y: auto;
        padding: 4px 8px 12px;
    }

    .hd-empty {
        padding: 60px 16px;
        text-align: center;
        color: var(--text-muted);
    }

    .hd-empty-icon {
        font-size: 28px;
        color: var(--accent);
        display: block;
        margin-bottom: 8px;
    }

    .hd-item {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        padding: 9px 8px;
        border-radius: 8px;
        cursor: pointer;
        transition: background 0.15s;
        position: relative;
    }

    .hd-item:hover {
        background: var(--bg-subtle);
    }

    .hd-item.active {
        background: var(--accent-bg);
    }

    .hd-item.active::before {
        content: "";
        position: absolute;
        right: 0;
        top: 50%;
        transform: translateY(-50%);
        width: 3px;
        height: 60%;
        background: var(--accent);
        border-radius: 3px 0 0 3px;
    }

    .hd-item-icon {
        font-size: 13px;
        margin-top: 1px;
        opacity: 0.8;
    }

    .hd-item-text {
        flex: 1;
        min-width: 0;
    }

    .hd-item-title {
        font-size: 13px;
        font-weight: 500;
        color: var(--text-primary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        margin-bottom: 3px;
    }

    .hd-item-meta {
        display: flex;
        gap: 4px;
        font-size: 11px;
        color: var(--text-muted);
        align-items: center;
    }

    .hd-item-source {
        font-family: ui-monospace, monospace;
        max-width: 130px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .hd-item-del {
        width: 22px;
        height: 22px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: transparent;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        border-radius: 5px;
        opacity: 0;
        transition: all 0.15s;
        flex-shrink: 0;
        margin-top: -2px;
    }

    .hd-item:hover .hd-item-del,
    .hd-item.active .hd-item-del {
        opacity: 0.7;
    }

    .hd-item-del:hover {
        opacity: 1 !important;
        color: var(--error-text);
        background: var(--error-bg);
    }

    @keyframes fadeIn {
        from {
            opacity: 0;
        }
        to {
            opacity: 1;
        }
    }
</style>
