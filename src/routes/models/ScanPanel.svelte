<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import type { ModelConfig, ModelFileInfo } from "./types";

    interface Props {
        /** 当前已配置模型（用于端口分配与已添加判断） */
        models: ModelConfig[];
        /** 添加成功后由页面刷新列表 */
        onAdded: () => Promise<void> | void;
        /** 添加失败时上报页面错误横幅 */
        onError: (msg: string) => void;
    }

    let { models, onAdded, onError }: Props = $props();

    let showScanSection = $state(false);
    let scanDir = $state("");
    let scanning = $state(false);
    let scanError = $state("");
    let scannedModels = $state<ModelFileInfo[]>([]);
    let addingIds = $state<Set<string>>(new Set());

    async function scanDirectory() {
        if (!scanDir.trim()) {
            scanError = "请输入目录路径";
            return;
        }
        scanning = true;
        scanError = "";
        scannedModels = [];
        try {
            const result = await invoke<ModelFileInfo[]>("scan_model_directory", {
                dir: scanDir.trim(),
            });
            scannedModels = result;
            if (result.length === 0) {
                scanError = "该目录下未找到 .gguf 模型文件";
            }
        } catch (e) {
            scanError = `扫描失败: ${e}`;
        } finally {
            scanning = false;
        }
    }

    async function addScannedModel(file: ModelFileInfo) {
        const name = file.filename.replace(/\.gguf$/i, "");
        const port = 8080 + models.length;
        addingIds = new Set([...addingIds, file.path]);
        try {
            const newConfig: ModelConfig = {
                id: `model-${name.toLowerCase().replace(/\s+/g, "-").trim()}-${port}`,
                name,
                model_path: file.path,
                server_path: "llama",
                port,
                ngl: 999,
            };
            await invoke("save_model_config", { model: newConfig });
            await onAdded();
            scannedModels = scannedModels.filter((m) => m.path !== file.path);
        } catch (e) {
            onError(`添加失败: ${e}`);
        } finally {
            const next = new Set(addingIds);
            next.delete(file.path);
            addingIds = next;
        }
    }

    async function addAllScannedModels() {
        for (const file of scannedModels) {
            await addScannedModel(file);
        }
    }

    function isAlreadyAdded(file: ModelFileInfo): boolean {
        return models.some((m) => m.model_path === file.path);
    }
</script>

<div class="scan-section">
    <button
        class="scan-toggle"
        onclick={() => (showScanSection = !showScanSection)}
        aria-expanded={showScanSection}
    >
        <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
        >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        扫描模型目录
        <svg
            class="toggle-chevron {showScanSection ? 'open' : ''}"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
        >
            <polyline points="6 9 12 15 18 9" />
        </svg>
    </button>

    {#if showScanSection}
        <div class="scan-body">
            <div class="scan-input-row">
                <input
                    type="text"
                    bind:value={scanDir}
                    placeholder="输入模型目录路径，如 ~/models"
                    disabled={scanning}
                />
                <button
                    class="btn-scan"
                    onclick={scanDirectory}
                    disabled={scanning}
                >
                    {#if scanning}
                        <span class="mini-spinner"></span>
                        扫描中...
                    {:else}
                        扫描
                    {/if}
                </button>
            </div>
            {#if scanError}
                <div class="scan-error">{scanError}</div>
            {/if}
            {#if scannedModels.length > 0}
                <div class="scanned-results">
                    <div class="results-header">
                        <span>发现 {scannedModels.length} 个模型文件</span>
                        <button class="btn-add-all" onclick={addAllScannedModels}>
                            全部添加
                        </button>
                    </div>
                    <div class="scanned-list">
                        {#each scannedModels as file (file.path)}
                            {@const alreadyAdded = isAlreadyAdded(file)}
                            <div
                                class="scanned-item"
                                class:already-added={alreadyAdded}
                            >
                                <span class="item-icon">🧩</span>
                                <div class="item-info">
                                    <span class="item-name">{file.filename}</span>
                                    <span class="item-size">{file.size_display}</span>
                                </div>
                                {#if alreadyAdded}
                                    <span class="item-added">已添加</span>
                                {:else}
                                    <button
                                        class="btn-add-item"
                                        onclick={() => addScannedModel(file)}
                                        disabled={addingIds.has(file.path)}
                                    >
                                        {#if addingIds.has(file.path)}
                                            添加中...
                                        {:else}
                                            + 添加
                                        {/if}
                                    </button>
                                {/if}
                            </div>
                        {/each}
                    </div>
                </div>
            {/if}
        </div>
    {/if}
</div>

<style>
    .scan-section {
        margin-bottom: 20px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        overflow: hidden;
    }

    .scan-toggle {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 12px 16px;
        background: transparent;
        border: none;
        font-size: 14px;
        font-weight: 600;
        color: var(--text-primary);
        cursor: pointer;
        transition: background 0.2s;
    }

    .scan-toggle:hover {
        background: var(--bg-card-hover);
    }

    .toggle-chevron {
        margin-left: auto;
        transition: transform 0.2s;
        color: var(--text-muted);
    }

    .toggle-chevron.open {
        transform: rotate(180deg);
    }

    .scan-body {
        padding: 0 16px 16px;
    }

    .scan-input-row {
        display: flex;
        gap: 8px;
    }

    .scan-input-row input {
        flex: 1;
        padding: 10px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 14px;
        color: var(--text-primary);
        outline: none;
        transition: all 0.2s;
        box-sizing: border-box;
    }

    .scan-input-row input:focus {
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-ring);
    }

    .scan-input-row input:disabled {
        opacity: 0.6;
    }

    .btn-scan {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 10px 20px;
        background: var(--accent);
        border: 1px solid transparent;
        border-radius: 8px;
        font-size: 14px;
        font-weight: 500;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
        white-space: nowrap;
    }

    .btn-scan:hover:not(:disabled) {
        background: var(--accent-hover);
    }

    .btn-scan:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }



    .scan-error {
        margin-top: 10px;
        padding: 8px 12px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 8px;
        font-size: 13px;
        color: var(--error-text);
    }

    .scanned-results {
        margin-top: 12px;
    }

    .results-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 8px;
        font-size: 13px;
        color: var(--text-secondary);
        font-weight: 500;
    }

    .btn-add-all {
        padding: 4px 12px;
        background: var(--accent-bg);
        border: 1px solid var(--accent);
        border-radius: 6px;
        font-size: 12px;
        font-weight: 500;
        color: var(--accent);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-add-all:hover {
        background: var(--accent);
        color: white;
    }

    .scanned-list {
        display: flex;
        flex-direction: column;
        gap: 6px;
        max-height: 300px;
        overflow-y: auto;
    }

    .scanned-item {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 10px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        transition: all 0.2s;
    }

    .scanned-item:hover {
        border-color: var(--border-strong);
    }

    .scanned-item.already-added {
        opacity: 0.5;
    }

    .item-icon {
        font-size: 18px;
        flex-shrink: 0;
    }

    .item-info {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
    }

    .item-name {
        font-size: 14px;
        font-weight: 500;
        color: var(--text-primary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .item-size {
        font-size: 12px;
        color: var(--text-muted);
    }

    .item-added {
        font-size: 12px;
        color: #16a34a;
        font-weight: 500;
        flex-shrink: 0;
    }

    .btn-add-item {
        padding: 4px 12px;
        background: var(--accent-bg);
        border: 1px solid var(--border-light);
        border-radius: 6px;
        font-size: 12px;
        font-weight: 500;
        color: var(--accent);
        cursor: pointer;
        transition: all 0.2s;
        flex-shrink: 0;
    }

    .btn-add-item:hover:not(:disabled) {
        background: var(--accent);
        color: white;
        border-color: var(--accent);
    }

    .btn-add-item:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    @media (max-width: 640px) {
        .scan-input-row {
            flex-direction: column;
        }
    }
</style>
