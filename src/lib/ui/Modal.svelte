<script lang="ts">
    import type { Snippet } from "svelte";

    interface Props {
        title: string;
        onClose: () => void;
        /** 提交中时禁用关闭（遮罩/Esc/关闭按钮） */
        closeDisabled?: boolean;
        /** 点击遮罩是否关闭（表单弹窗建议 false 防误触丢输入） */
        closeOnOverlay?: boolean;
        /** confirm 变体：更紧凑的确认类弹窗 */
        variant?: "default" | "confirm";
        /** 弹窗内容（modal-body + modal-footer 由调用方通过 children 提供） */
        children: Snippet;
    }

    let {
        title,
        onClose,
        closeDisabled = false,
        closeOnOverlay = true,
        variant = "default",
        children,
    }: Props = $props();

    function requestClose() {
        if (!closeDisabled) onClose();
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") requestClose();
    }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
    class="modal-overlay"
    onclick={closeOnOverlay ? requestClose : undefined}
    role="presentation"
>
    <div
        class="modal"
        class:compact={variant === "confirm"}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabindex="-1"
    >
        <div class="modal-header">
            <h2>{title}</h2>
            <button
                class="modal-close"
                onclick={requestClose}
                disabled={closeDisabled}
                aria-label="关闭"
            >
                <svg
                    width="18"
                    height="18"
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
        {@render children()}
    </div>
</div>
