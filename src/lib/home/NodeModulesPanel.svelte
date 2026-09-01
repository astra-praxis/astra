<script lang="ts">
    import {
        nm,
        nmCache,
        startScan,
        cleanSingle,
        cleanAll,
        type NodeModulesInfo,
    } from "$lib/nm-store.svelte";
    import { formatBytes } from "$lib/format";
    import Modal from "$lib/ui/Modal.svelte";

    interface Props {
        /** 当前展示的项目路径（变化时自动从缓存恢复或重置） */
        projectPath: string;
        editorName: string;
        onOpenEditor: (path: string) => void;
    }

    let { projectPath, editorName, onOpenEditor }: Props = $props();

    let nodeModulesList = $state<NodeModulesInfo[]>([]);
    let nodeModulesScannedPath = $state<string | null>(null);
    let nodeModulesError = $state("");
    let deletingPath = $state<string | null>(null);

    // 切换项目时：有缓存直接展示，无缓存清空等待手动扫描
    $effect(() => {
        void projectPath;
        const cached = nmCache.get(projectPath);
        if (cached) {
            nodeModulesList = cached;
            nodeModulesScannedPath = projectPath;
        } else {
            nodeModulesList = [];
            nodeModulesScannedPath = null;
        }
        nodeModulesError = "";
    });

    const nmProjectCount = $derived(
        new Set(nodeModulesList.map((r) => r.project_name)).size,
    );

    const totalNodeModulesSize = $derived(
        nodeModulesList.reduce((sum, r) => sum + r.size_bytes, 0),
    );

    async function scanNodeModules() {
        if (nm.scanning) return;
        nodeModulesList = [];
        nodeModulesError = "";
        const result = await startScan(projectPath);
        nodeModulesList = result || [];
        nodeModulesScannedPath = projectPath;
        if (nm.error) {
            nodeModulesError = nm.error;
        }
    }

    function confirmDeleteNodeModule(path: string) {
        deletingPath = path;
    }

    function cancelDeleteNodeModule() {
        deletingPath = null;
    }

    async function doDeleteNodeModule() {
        if (!deletingPath) return;
        const path = deletingPath;
        const cleaned = await cleanSingle(path);
        nodeModulesList = nodeModulesList.filter(
            (r) => !cleaned.includes(r.path),
        );
        nmCache.set(projectPath, nodeModulesList);
        if (nm.error) {
            nodeModulesError = nm.error;
        }
        deletingPath = null;
    }

    async function cleanAllNodeModules() {
        if (nodeModulesList.length === 0 || nm.cleaning) return;
        const cleaned = await cleanAll(nodeModulesList.map((r) => r.path));
        if (cleaned) {
            nodeModulesList = nodeModulesList.filter(
                (r) => !cleaned.includes(r.path),
            );
            nmCache.set(projectPath, nodeModulesList);
        }
        if (nm.error) {
            nodeModulesError = nm.error;
        }
    }
</script>

<div class="detail-nm">
    <div class="nm-section-header">
        <div class="section-title">
            <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"
            /></svg>
            <span>node_modules 清理</span>
        </div>
        <button
            class="nm-scan-btn"
            onclick={scanNodeModules}
            disabled={nm.scanning}
        >
            {#if nm.scanning}
                <div class="btn-spinner"></div>
                扫描中...
            {:else}
                <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path d="M21 12a9 9 0 1 1-9-9" /><polyline
                        points="21 3 21 9 15 9"
                    /></svg
                >
                {nodeModulesScannedPath === projectPath
                    ? "重新扫描"
                    : "扫描 node_modules"}
            {/if}
        </button>
    </div>

    {#if nm.scanning && nm.progress}
        <div class="nm-scan-progress">
            <div class="nm-scan-progress-spinner"></div>
            <span class="nm-scan-progress-text" title={nm.progress}
                >{nm.progress}</span
            >
        </div>
    {/if}

    {#if nodeModulesScannedPath === projectPath && !nm.scanning}
        {#if nodeModulesError}
            <div class="nm-error">{nodeModulesError}</div>
        {/if}

        {#if nodeModulesList.length === 0}
            <div class="nm-empty-inline">
                <span>📦</span>
                <span>此项目中未找到 node_modules 文件夹</span>
            </div>
        {:else}
            <div class="nm-summary-inline">
                <span class="nm-stat"
                    ><strong>{nmProjectCount}</strong> 个项目下共
                    <strong>{nodeModulesList.length}</strong> 个
                    node_modules</span
                >
                <span class="nm-stat"
                    >总计 <strong>{formatBytes(totalNodeModulesSize)}</strong
                    ></span
                >
                <button
                    class="nm-clean-btn"
                    onclick={cleanAllNodeModules}
                    disabled={nm.cleaning}
                >
                    {#if nm.cleaning}
                        <div class="btn-spinner"></div>
                        清理中...
                    {:else}
                        <svg
                            width="13"
                            height="13"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><polyline points="3 6 5 6 21 6" /><path
                                d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
                        /></svg>
                        一键清理全部
                    {/if}
                </button>
            </div>

            <div class="nm-list-inline">
                {#each nodeModulesList as nmItem (nmItem.path)}
                    <div class="nm-item-inline">
                        <div class="nm-item-inline-content">
                            <div class="nm-item-inline-path" title={nmItem.path}
                                >{nmItem.path}</div
                            >
                            <div class="nm-item-inline-right">
                                <button
                                    class="nm-item-inline-open-btn"
                                    onclick={() =>
                                        onOpenEditor(nmItem.project_path)}
                                    title={`在 ${editorName || "编辑器"} 中打开`}
                                    aria-label="在编辑器中打开"
                                >
                                    <svg
                                        width="13"
                                        height="13"
                                        viewBox="0 0 24 24"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        ><polygon
                                            points="5 3 19 12 5 21 5 3"
                                    /></svg>
                                </button>
                                <span class="nm-item-inline-size"
                                    >{nmItem.size_display}</span
                                >
                            </div>
                        </div>
                        <button
                            class="nm-item-delete-btn"
                            onclick={() => confirmDeleteNodeModule(nmItem.path)}
                            title="删除此 node_modules"
                        >
                            <svg
                                width="12"
                                height="12"
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                ><polyline points="3 6 5 6 21 6" /><path
                                    d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
                            /></svg>
                        </button>
                    </div>
                {/each}
            </div>
        {/if}
    {/if}
</div>

{#if deletingPath}
    <Modal
        title="确认删除"
        variant="confirm"
        onClose={cancelDeleteNodeModule}
    >
        <div class="nm-confirm-body">
            <p>确定要删除以下 node_modules 吗？</p>
            <p class="nm-confirm-path"
                >{deletingPath.replace(projectPath, ".")}</p
            >
        </div>
        <div class="modal-footer">

            <button
                class="nm-confirm-delete-btn"
                onclick={doDeleteNodeModule}
                disabled={nm.cleaning}
            >
                {#if nm.cleaning}
                    <div class="btn-spinner"></div>
                    删除中...
                {:else}
                    确认删除
                {/if}
            </button>
        </div>
    </Modal>
{/if}

<style>
    .detail-nm {
        margin-top: 20px;
        margin-bottom: 24px;
        background: var(--bg-card);
        border-radius: 12px;
        border: 1px solid var(--border);
        padding: 18px 22px;
    }

    .section-title {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 16px;
        font-weight: 600;
        color: var(--text-primary);
        margin-bottom: 14px;
    }

    .section-title svg {
        color: var(--text-muted);
        flex-shrink: 0;
    }

    .nm-section-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 12px;
    }

    .nm-section-header .section-title {
        margin-bottom: 0;
    }

    .nm-scan-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 6px 14px;
        background: var(--accent-gradient);
        border: none;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .nm-scan-btn:hover:not(:disabled) {
        transform: translateY(-1px);
        box-shadow: 0 2px 8px var(--accent-shadow);
    }

    .nm-scan-btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .nm-scan-progress {
        display: flex;
        align-items: center;
        gap: 10px;
        margin: 10px 0;
        padding: 8px 14px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        min-width: 0;
    }

    .nm-scan-progress-spinner {
        width: 13px;
        height: 13px;
        border: 2px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.7s linear infinite;
        flex-shrink: 0;
    }

    .nm-scan-progress-text {
        font-size: 12px;
        color: var(--text-secondary);
        font-family: var(--font-mono, "SF Mono", Menlo, monospace);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
    }

    .nm-error {
        background: rgba(255, 59, 48, 0.1);
        color: var(--error-text);
        padding: 10px 14px;
        border-radius: 8px;
        font-size: 13px;
        margin-bottom: 12px;
    }

    .nm-empty-inline {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 12px 0;
        color: var(--text-secondary);
        font-size: 14px;
    }

    .nm-summary-inline {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 8px 16px;
        padding: 8px 0 12px;
        border-bottom: 1px solid var(--border);
        margin-bottom: 8px;
    }

    .nm-stat {
        font-size: 13px;
        color: var(--text-secondary);
    }

    .nm-stat strong {
        color: var(--text-primary);
    }

    .nm-clean-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 6px 14px;
        background: linear-gradient(135deg, #ff5252, #ff1744);
        border: none;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
        margin-left: auto;
    }

    .nm-clean-btn:hover:not(:disabled) {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px rgba(255, 23, 68, 0.3);
    }

    .nm-clean-btn:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .nm-list-inline {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .nm-item-inline {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 11px 12px;
        border-radius: 8px;
        transition: background 0.15s;
    }

    .nm-item-inline:hover {
        background: var(--bg-input);
    }

    .nm-item-inline-content {
        display: flex;
        align-items: center;
        justify-content: space-between;
        width: 100%;
        min-width: 0;
    }

    .nm-item-inline-right {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-shrink: 0;
        margin-left: 12px;
    }

    .nm-item-inline-path {
        font-size: 13px;
        color: var(--text-secondary);
        font-family: var(--font-mono, "SF Mono", Menlo, monospace);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
    }

    .nm-item-inline-open-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 24px;
        height: 24px;
        border: none;
        border-radius: 6px;
        background: transparent;
        color: var(--text-muted);
        cursor: pointer;
        flex-shrink: 0;
        opacity: 0;
        transition:
            background 0.15s,
            color 0.15s,
            opacity 0.15s;
    }

    .nm-item-inline:hover .nm-item-inline-open-btn {
        opacity: 1;
    }

    .nm-item-inline-open-btn:hover {
        background: var(--accent);
        color: white;
    }

    .nm-item-inline-size {
        font-size: 13px;
        color: var(--text-secondary);
        font-weight: 600;
        flex-shrink: 0;
        margin-left: 12px;
    }

    .nm-item-delete-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 26px;
        height: 26px;
        background: none;
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s;
        flex-shrink: 0;
        opacity: 0;
    }

    .nm-item-inline:hover .nm-item-delete-btn {
        opacity: 1;
    }

    .nm-item-delete-btn:hover {
        color: #ff1744;
        border-color: #ff1744;
        background: rgba(255, 23, 68, 0.08);
    }

    /* ===== 删除确认弹窗内容 ===== */
    .nm-confirm-body {
        padding: 8px 20px 16px;
        font-size: 14px;
        color: var(--text-secondary);
    }

    .nm-confirm-body p {
        margin: 4px 0;
    }

    .nm-confirm-path {
        font-family: monospace;
        font-size: 12px;
        color: var(--text-primary);
        background: var(--bg-input);
        padding: 8px 12px;
        border-radius: 8px;
        word-break: break-all;
        margin-top: 8px !important;
    }

    .modal-footer {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        padding: 16px 24px 20px;
    }


    .nm-confirm-delete-btn {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 22px;
        background: linear-gradient(135deg, #ff5252, #ff1744);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .nm-confirm-delete-btn:hover:not(:disabled) {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px rgba(255, 23, 68, 0.3);
    }

    .nm-confirm-delete-btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .btn-spinner {
        width: 16px;
        height: 16px;
        border: 2px solid rgba(255, 255, 255, 0.3);
        border-top-color: white;
        border-radius: 50%;
        animation: spin 0.7s linear infinite;
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }
</style>
