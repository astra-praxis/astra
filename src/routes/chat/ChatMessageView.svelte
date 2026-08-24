<script lang="ts">
    import MarkdownMessage from "./MarkdownMessage.svelte";
    import type { ChatMessage } from "./types";

    interface Props {
        msg: ChatMessage;
        isLast: boolean;
        isSending: boolean;
        workspacePath: string;
        onCopy: (msg: ChatMessage) => void;
        onToggleFavorite: (msg: ChatMessage) => void;
    }

    let {
        msg,
        isLast,
        isSending,
        workspacePath,
        onCopy,
        onToggleFavorite,
    }: Props = $props();
</script>

<article
    class="msg"
    class:user={msg.role === "user"}
    class:assistant={msg.role === "assistant"}
>
    <div class="msg-content" class:err={msg.error}>
        <!-- 推理过程（可折叠） -->
        {#if msg.reasoning}
            <div class="reasoning-wrap">
                <button
                    class="reasoning-toggle"
                    onclick={() => (msg.showReasoning = !msg.showReasoning)}
                >
                    <span class="reasoning-dot"></span>
                    <span>推理过程</span>
                    <svg
                        width="12"
                        height="12"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        class:rotated={msg.showReasoning}
                        ><polyline points="6 9 12 15 18 9" /></svg
                    >
                </button>
                {#if msg.showReasoning}
                    <pre class="reasoning-text">{msg.reasoning}</pre>
                {/if}
            </div>
        {/if}

        <!-- 主回复内容 -->
        {#if !msg.content && isSending && isLast}
            <div class="typing">
                <span></span><span></span><span></span>
            </div>
        {:else if msg.role === "assistant"}
            <div class="msg-body">
                <MarkdownMessage
                    content={msg.content}
                    {workspacePath}
                    isFresh={msg.isFresh || false}
                />
            </div>
        {:else}
            <pre class="msg-text">{msg.content}</pre>
        {/if}

        <!-- 底部操作按钮（仅 AI 输出） -->
        {#if msg.role === "assistant" && msg.content && !(isSending && isLast)}
            <div class="msg-footer">
                <button
                    class="mf-btn"
                    onclick={() => onCopy(msg)}
                    title="复制全文"
                    aria-label="复制全文"
                >
                    <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><rect
                            x="9"
                            y="9"
                            width="13"
                            height="13"
                            rx="2"
                            ry="2"
                        /><path
                            d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
                        /></svg
                    >
                </button>
                <button
                    class="mf-btn"
                    class:faved={msg.favorite}
                    onclick={() => onToggleFavorite(msg)}
                    title={msg.favorite ? "取消收藏" : "收藏"}
                    aria-label="收藏"
                >
                    {#if msg.favorite}
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="currentColor"
                            stroke="currentColor"
                            stroke-width="2"
                            ><polygon
                                points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"
                            /></svg
                        >
                    {:else}
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            ><polygon
                                points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"
                            /></svg
                        >
                    {/if}
                </button>
            </div>
        {/if}
    </div>
</article>

<style>
    /* ===== 单条消息 ===== */
    .msg {
        max-width: var(--msg-max, 760px);
        width: 100%;
        margin: 0 auto;
        padding: 4px 0;
        animation: msgEnter 0.3s ease;
    }

    .msg:has(.msg-footer) {
        padding-bottom: 0;
    }

    @keyframes msgEnter {
        from {
            opacity: 0;
            transform: translateY(6px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    /* 用户：占满整行 */
    .msg.user {
        display: flex;
        justify-content: stretch;
    }

    /* 用户消息：暗色卡片 + 内阴影 + 占满整行 */
    .msg.user .msg-content {
        width: 100%;
        padding: 10px 16px;
        background: var(--sidebar-bg);
        border: 1px solid var(--sidebar-border);
        border-radius: 10px;
        box-shadow: inset 0 1px 4px rgba(0, 0, 0, 0.25);
    }

    .msg.user .msg-text {
        color: var(--text-primary);
    }

    /* 助手：纯文本 */
    .msg.assistant {
        display: flex;
        justify-content: flex-start;
    }

    .msg.assistant .msg-content {
        padding: 4px 0;
        color: var(--text-primary);
    }

    .msg.assistant .msg-content.err {
        color: var(--error-text);
    }

    /* 消息容器基准 */
    .msg-content {
        position: relative;
        display: inline-block;
        max-width: 100%;
        min-width: 0;
        overflow-wrap: break-word;
        word-break: break-word;
    }

    .msg-body {
        margin-top: 4px;
        max-width: 100%;
        min-width: 0;
        overflow: hidden;
    }

    /* 隐形桥接区域：连接消息与右侧按钮，避免 hover 断开 */
    .msg:has(.msg-footer) .msg-content::after {
        content: "";
        position: absolute;
        top: 0;
        right: 0;
        bottom: 0;
        width: 12px;
        transform: translateX(24px);
    }

    .msg-text {
        margin: 0;
        font-family: inherit;
        font-size: 14.5px;
        line-height: 1.7;
        white-space: pre-wrap;
        word-break: break-word;
    }

    /* 推理部分（折叠式） */
    .reasoning-wrap {
        margin-bottom: 10px;
    }

    .reasoning-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: var(--accent);
        flex-shrink: 0;
        animation: pulse 1.4s ease infinite;
    }

    @keyframes pulse {
        0%,
        100% {
            opacity: 1;
            transform: scale(1);
        }
        50% {
            opacity: 0.4;
            transform: scale(0.85);
        }
    }

    .reasoning-toggle {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 5px 10px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 12px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s;
        font-family: inherit;
    }

    .reasoning-toggle:hover {
        color: var(--accent);
        border-color: var(--accent);
    }

    .reasoning-toggle svg.rotated {
        transform: rotate(180deg);
    }

    .reasoning-toggle svg {
        transition: transform 0.2s;
    }

    .reasoning-text {
        margin: 8px 0 0;
        padding: 10px 12px;
        background: var(--bg-subtle);
        border-radius: 8px;
        font-size: 12px;
        line-height: 1.55;
        color: var(--text-secondary);
        white-space: pre-wrap;
        word-break: break-word;
        font-family: ui-monospace, monospace;
        border: 1px dashed var(--border);
    }

    /* 打字指示器 */
    .typing {
        display: inline-flex;
        gap: 4px;
        align-items: center;
        padding: 4px 0;
    }

    .typing span {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--accent);
        animation: typingBounce 1.2s infinite;
    }

    .typing span:nth-child(2) {
        animation-delay: 0.15s;
    }
    .typing span:nth-child(3) {
        animation-delay: 0.3s;
    }

    @keyframes typingBounce {
        0%,
        60%,
        100% {
            transform: translateY(0);
            opacity: 0.4;
        }
        30% {
            transform: translateY(-4px);
            opacity: 1;
        }
    }

    /* 消息右侧操作（定位到消息框右侧外部，竖排，默认隐藏） */
    .msg-footer {
        position: absolute;
        right: -36px;
        bottom: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
        opacity: 0;
        transition: opacity 0.2s ease;
        transition-delay: 0.3s;
        pointer-events: none;
        z-index: 5;
    }

    .msg:hover .msg-footer,
    .msg:focus-within .msg-footer {
        opacity: 1;
        pointer-events: auto;
        transition-delay: 0s;
    }

    .mf-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.15s;
        padding: 0;
    }

    .msg.user .mf-btn {
        background: rgba(0, 0, 0, 0.2);
        border-color: rgba(255, 255, 255, 0.2);
        color: rgba(255, 255, 255, 0.85);
    }

    .mf-btn:hover {
        color: var(--accent);
        border-color: var(--accent);
        background: var(--accent-bg);
    }

    .msg.user .mf-btn:hover {
        background: rgba(255, 255, 255, 0.2);
        color: white;
        border-color: rgba(255, 255, 255, 0.4);
    }

    .mf-btn.faved {
        color: #f59e0b;
        border-color: #f59e0b;
        background: rgba(245, 158, 11, 0.1);
    }

    .mf-btn.faved:hover {
        color: #d97706;
        border-color: #d97706;
    }
</style>
