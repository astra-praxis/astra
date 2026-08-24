<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onMount, onDestroy } from "svelte";
    import type { ModelConfig, RunningModelInfo } from "./types";
    import ModelFormModal from "./ModelFormModal.svelte";
    import ModelCard from "./ModelCard.svelte";
    import ScanPanel from "./ScanPanel.svelte";
    import Modal from "$lib/ui/Modal.svelte";

    // ===== 状态 =====
    let models = $state<ModelConfig[]>([]);
    let runningInfo = $state<RunningModelInfo[]>([]);
    let loading = $state(true);
    let error = $state("");
    /** null = 添加模式；非空 = 编辑该模型 */
    let formModel = $state<ModelConfig | null>(null);
    let showForm = $state(false);
    let deleteTarget = $state<ModelConfig | null>(null);
    let actionLoading = $state<string | null>(null); // track which model id is performing an action
    let pollInterval: ReturnType<typeof setInterval> | null = null;

    // ===== 加载 =====
    onMount(() => {
        loadModels();
        pollRunning();
        pollInterval = setInterval(pollRunning, 3000);
    });

    onDestroy(() => {
        if (pollInterval) clearInterval(pollInterval);
    });

    async function loadModels() {
        loading = true;
        error = "";
        try {
            models = await invoke<ModelConfig[]>("list_model_configs");
        } catch (e) {
            error = `加载失败: ${e}`;
        } finally {
            loading = false;
        }
    }

    async function pollRunning() {
        try {
            runningInfo = await invoke<RunningModelInfo[]>("list_running_servers");
        } catch {
            // 静默忽略轮询错误
        }
    }

    // ===== 状态查询 =====
    function isModelRunning(model: ModelConfig): boolean {
        const info = runningInfo.find((r) => r.id === model.id || r.port === model.port);
        return (info?.status ?? "stopped") === "running";
    }

    // ===== 启动/停止 =====
    async function startModel(model: ModelConfig) {
        actionLoading = model.id;
        try {
            await invoke("start_llama_server", {
                serverPath: model.server_path,
                modelPath: model.model_path,
                port: model.port,
                ngl: model.ngl,
                modelName: model.name,
            });
            await pollRunning();
        } catch (e) {
            error = `启动失败: ${e}`;
        } finally {
            actionLoading = null;
        }
    }

    async function stopModel(model: ModelConfig) {
        actionLoading = model.id;
        try {
            await invoke("stop_llama_server", { port: model.port });
            await pollRunning();
        } catch (e) {
            error = `停止失败: ${e}`;
        } finally {
            actionLoading = null;
        }
    }

    // ===== 删除 =====
    async function doDelete() {
        const target = deleteTarget;
        if (!target) return;
        try {
            await invoke("delete_model_config", { id: target.id });
            models = models.filter((m) => m.id !== target.id);
            deleteTarget = null;
        } catch (e) {
            error = `删除失败: ${e}`;
            deleteTarget = null;
        }
    }

    // ===== 弹窗编排 =====
    function openAddModal() {
        formModel = null;
        showForm = true;
    }

    function openEditModal(model: ModelConfig) {
        formModel = model;
        showForm = true;
    }
</script>

<div class="models-page">
    <div class="page-header">
        <div class="header-left">
            <a href="/settings" class="btn-back" title="返回设置">
                <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><polyline points="15 18 9 12 15 6" /></svg
                >
                返回设置
            </a>
            <h1>模型管理</h1>
            <p class="subtitle">管理 llama.cpp 模型服务</p>
        </div>
        <div class="header-actions">
            <button class="btn-refresh" onclick={loadModels} disabled={loading}>
                <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><polyline points="23 4 23 10 17 10" /><path
                        d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"
                    /></svg
                >
                刷新
            </button>
            <button class="btn-add" onclick={openAddModal}>
                <svg
                    width="16"
                    height="16"
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
                添加模型
            </button>
        </div>
    </div>

    <!-- 错误提示 -->
    {#if error}
        <div class="error-banner">
            <span>⚠️</span>
            <span>{error}</span>
            <button class="error-dismiss" onclick={() => (error = "")}>✕</button>
        </div>
    {/if}

    <!-- 加载 -->
    {#if loading}
        <div class="loading">
            <div class="spinner"></div>
            <span>正在加载模型配置...</span>
        </div>
    {/if}

    <!-- 扫描模型目录 -->
    <ScanPanel {models} onAdded={loadModels} onError={(msg) => (error = msg)} />

    <!-- 空状态 -->
    {#if !loading && models.length === 0 && !error}
        <div class="empty-state">
            <span class="empty-icon">🤖</span>
            <h3>还没有配置任何模型</h3>
            <p>点击上方"添加模型"按钮，配置第一个 llama.cpp 模型</p>
        </div>
    {/if}

    <!-- 模型卡片 -->
    {#if !loading && models.length > 0}
        <div class="models-count">{models.length} 个模型</div>
        <div class="models-grid">
            {#each models as model (model.id)}
                <ModelCard
                    {model}
                    isRunning={isModelRunning(model)}
                    isLoading={actionLoading === model.id}
                    onDelete={(m) => (deleteTarget = m)}
                    onStop={stopModel}
                    onStart={startModel}
                    onEdit={openEditModal}
                />
            {/each}
        </div>
    {/if}
</div>

<!-- 添加/编辑模型弹窗 -->
{#if showForm}
    <ModelFormModal
        model={formModel}
        onClose={() => (showForm = false)}
        onSaved={loadModels}
    />
{/if}

<!-- 删除确认弹窗 -->
{#if deleteTarget}
    <Modal
        title="确认删除"
        onClose={() => (deleteTarget = null)}
        variant="confirm"
    >
        <div class="modal-body">
            <div class="confirm-icon">⚠️</div>
            <p class="confirm-text">
                确定要删除模型 <strong>{deleteTarget.name}</strong> 吗？
            </p>
            <div class="confirm-path">{deleteTarget.name}</div>
        </div>
        <div class="modal-footer">
            <button class="btn-cancel" onclick={() => (deleteTarget = null)}
                >取消</button
            >
            <button class="btn-danger" onclick={doDelete}> 确认删除 </button>
        </div>
    </Modal>
{/if}

<style>
    .models-page {
        max-width: 1100px;
        margin: 0 auto;
        animation: fadeIn 0.3s ease;
    }

    @keyframes fadeIn {
        from {
            opacity: 0;
            transform: translateY(8px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    /* 页头 */
    .page-header {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        margin-bottom: 24px;
    }

    .page-header h1 {
        font-size: 26px;
        font-weight: 700;
        color: var(--text-primary);
        margin-bottom: 6px;
    }

    .subtitle {
        color: var(--text-muted);
        font-size: 14px;
    }

    .header-actions {
        display: flex;
        gap: 10px;
    }

    .btn-back {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        margin-bottom: 12px;
        padding: 5px 10px 5px 8px;
        font-size: 12px;
        font-weight: 600;
        color: var(--text-muted);
        background: var(--bg-subtle);
        border: 1px solid var(--border-light);
        border-radius: 8px;
        text-decoration: none;
        align-self: flex-start;
        transition:
            background 0.2s ease,
            color 0.2s ease;
    }

    .btn-back:hover {
        background: var(--bg-card-hover);
        color: var(--text-primary);
    }

    .btn-refresh {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 8px 16px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        font-size: 14px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-refresh:hover:not(:disabled) {
        background: var(--bg-subtle);
        border-color: var(--border-strong);
    }

    .btn-refresh:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .btn-add {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 8px 16px;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-add:hover {
        background: var(--accent-hover);
        transform: translateY(-1px);
        box-shadow: 0 4px 12px var(--accent-shadow);
    }

    /* 错误 */
    .error-banner {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 12px 16px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 12px;
        color: var(--error-text);
        margin-bottom: 20px;
        font-size: 14px;
    }

    .error-dismiss {
        margin-left: auto;
        background: none;
        border: none;
        color: var(--error-muted);
        cursor: pointer;
        font-size: 16px;
    }

    .error-dismiss:hover {
        color: var(--error-text);
    }

    /* 加载 */
    .loading {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 12px;
        padding: 60px;
        color: var(--text-secondary);
    }



    /* 空状态 */
    .empty-state {
        text-align: center;
        padding: 80px 20px;
        background: var(--bg-card);
        border-radius: 16px;
        border: 2px dashed var(--border);
    }

    .empty-icon {
        font-size: 56px;
        display: block;
        margin-bottom: 16px;
    }

    .empty-state h3 {
        font-size: 20px;
        color: var(--text-secondary);
        margin-bottom: 8px;
    }

    .empty-state p {
        color: var(--text-muted);
        font-size: 15px;
    }

    /* 计数 */
    .models-count {
        font-size: 14px;
        color: var(--text-muted);
        margin-bottom: 16px;
    }

    /* 卡片网格 */
    .models-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
        gap: 16px;
    }


</style>
