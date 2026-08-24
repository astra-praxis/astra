<script lang="ts">
    import { goto } from "$app/navigation";
    import type { ModelConfig } from "./types";

    interface Props {
        model: ModelConfig;
        isRunning: boolean;
        isLoading: boolean;
        onDelete: (model: ModelConfig) => void;
        onStop: (model: ModelConfig) => void;
        onStart: (model: ModelConfig) => void;
        onEdit: (model: ModelConfig) => void;
    }

    let { model, isRunning, isLoading, onDelete, onStop, onStart, onEdit }: Props =
        $props();
</script>

<div class="model-card">
    <div class="model-header">
        <span class="model-icon">🧩</span>
        <div class="model-info">
            <h3 class="model-name">{model.name}</h3>
            <div class="model-meta">
                <span class="meta-tag">端口 {model.port}</span>
                <span class="meta-tag">ngl {model.ngl}</span>
            </div>
        </div>
        <span
            class="status-badge {isRunning ? 'status-running' : 'status-stopped'}"
        >
            {isRunning ? "运行中" : "已停止"}
        </span>
    </div>

    <div class="model-paths">
        <div class="path-row">
            <span class="path-label">模型</span>
            <span class="path-value" title={model.model_path}>{model.model_path}</span
            >
        </div>
    </div>

    <div class="model-footer">
        <div class="footer-left">
            <button
                class="btn-delete"
                onclick={() => onDelete(model)}
                title="删除"
            >
                <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><polyline points="3 6 5 6 21 6" /><path
                        d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
                    /></svg
                >
            </button>
            {#if isLoading}
                <span class="action-loading">
                    <span class="mini-spinner"></span>
                    处理中...
                </span>
            {/if}
        </div>
        <div class="footer-actions">
            {#if isRunning}
                <button
                    class="btn-stop"
                    onclick={() => onStop(model)}
                    disabled={isLoading}
                >
                    <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><rect x="6" y="6" width="12" height="12" rx="1" /></svg
                    >
                    停止
                </button>
                <button class="btn-detail" onclick={() => goto(`/models/${model.id}`)}>
                    详情 →
                </button>
            {:else}
                <button
                    class="btn-start"
                    onclick={() => onStart(model)}
                    disabled={isLoading}
                >
                    <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><polygon points="5 3 19 12 5 21 5 3" /></svg
                    >
                    启动
                </button>
            {/if}
            <button class="btn-edit" onclick={() => onEdit(model)} title="编辑">
                <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path
                        d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"
                    /><path
                        d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"
                    /></svg
                >
            </button>
        </div>
    </div>
</div>

<style>
    .model-card {
        background: var(--bg-card);
        border-radius: 14px;
        padding: 20px;
        box-shadow: 0 1px 3px var(--shadow-sm);
        border: 1px solid var(--border-light);
        display: flex;
        flex-direction: column;
        gap: 14px;
        transition: all 0.2s ease;
    }

    .model-card:hover {
        box-shadow: 0 4px 12px var(--shadow-hover);
        border-color: var(--border);
    }

    .model-header {
        display: flex;
        align-items: flex-start;
        gap: 12px;
    }

    .model-icon {
        font-size: 24px;
        flex-shrink: 0;
        width: 44px;
        height: 44px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--bg-subtle);
        border-radius: 12px;
    }

    .model-info {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .model-name {
        font-size: 16px;
        font-weight: 600;
        color: var(--text-primary);
        font-family: ui-monospace, monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .model-meta {
        display: flex;
        gap: 8px;
        font-size: 12px;
    }

    .meta-tag {
        color: var(--text-secondary);
        background: var(--bg-subtle);
        padding: 2px 8px;
        border-radius: 6px;
    }

    /* 状态徽章 */
    .status-badge {
        font-size: 11px;
        font-weight: 600;
        padding: 4px 10px;
        border-radius: 8px;
        flex-shrink: 0;
        display: inline-flex;
        align-items: center;
    }

    .status-running {
        color: var(--success-text);
        background: var(--success-bg);
    }

    .status-running::before {
        content: "";
        display: inline-block;
        width: 6px;
        height: 6px;
        background: var(--success-text);
        border-radius: 50%;
        margin-right: 6px;
        animation: pulse 1.5s ease infinite;
    }

    @keyframes pulse {
        0%,
        100% {
            opacity: 1;
        }
        50% {
            opacity: 0.4;
        }
    }

    .status-stopped {
        color: var(--text-muted);
        background: var(--bg-subtle);
    }

    /* 路径 */
    .model-paths {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .path-row {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 12px;
    }

    .path-label {
        color: var(--text-muted);
        flex-shrink: 0;
        width: 36px;
    }

    .path-value {
        color: var(--text-secondary);
        font-family: ui-monospace, monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    /* 卡片底部 */
    .model-footer {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding-top: 10px;
        border-top: 1px solid var(--border-light);
    }

    .footer-left {
        flex: 1;
    }

    .footer-actions {
        display: flex;
        gap: 8px;
        align-items: center;
    }

    .action-loading {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--text-muted);
    }



    /* 按钮 */
    .btn-start {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 6px 14px;
        background: var(--accent);
        border: none;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-start:hover:not(:disabled) {
        background: var(--accent-hover);
        transform: translateY(-1px);
    }

    .btn-start:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .btn-stop {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 6px 14px;
        background: var(--error-text);
        border: none;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-stop:hover:not(:disabled) {
        background: #b91c1c;
        transform: translateY(-1px);
    }

    .btn-stop:disabled {
        opacity: 0.5;
    }

    .btn-detail {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 6px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 13px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-detail:hover {
        background: var(--bg-card-hover);
        color: var(--text-primary);
    }

    .btn-edit {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 6px 10px;
        background: none;
        border: 1px solid var(--border-light);
        border-radius: 8px;
        font-size: 13px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-edit:hover {
        color: var(--accent);
        background: var(--accent-light);
        border-color: var(--accent);
    }

    .btn-delete {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 6px 10px;
        background: none;
        border: 1px solid var(--border-light);
        border-radius: 8px;
        font-size: 13px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-delete:hover {
        color: var(--error-text);
        background: var(--error-bg);
        border-color: var(--error-border);
    }
</style>
