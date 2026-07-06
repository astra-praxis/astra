<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onMount, onDestroy } from "svelte";
    import { goto } from "$app/navigation";

    // ===== 类型 =====
    interface ModelConfig {
        id: string;
        name: string;
        model_path: string;
        server_path: string;
        port: number;
        ngl: number;
    }

    interface RunningModelInfo extends ModelConfig {
        status: string;
        pid: number | null;
        started_at: number;
    }

    // ===== 状态 =====
    let models = $state<ModelConfig[]>([]);
    let runningInfo = $state<RunningModelInfo[]>([]);
    let loading = $state(true);
    let error = $state("");
    let showAddModal = $state(false);
    let deleteTarget = $state<ModelConfig | null>(null);
    let actionLoading = $state<string | null>(null); // track which model id is performing an action
    let pollInterval: ReturnType<typeof setInterval> | null = null;

    // ===== 新模型表单 =====
    let newName = $state("");
    let newServerPath = $state("");
    let newModelPath = $state("");
    let newPort = $state(8080);
    let newNgl = $state(999);
    let formError = $state("");
    let formSaving = $state(false);

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
            runningInfo = await invoke<RunningModelInfo[]>(
                "list_running_servers",
            );
        } catch {
            // 静默忽略轮询错误
        }
    }

    // ===== 状态查询 =====
    function getModelStatus(model: ModelConfig): string {
        const info = runningInfo.find(
            (r) => r.id === model.id || r.port === model.port,
        );
        return info?.status ?? "stopped";
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

    // ===== 添加模型 =====
    function openAddModal() {
        newName = "";
        newServerPath = "";
        newModelPath = "";
        newPort = 8080;
        newNgl = 999;
        formError = "";
        formSaving = false;
        showAddModal = true;
    }

    async function saveNewModel() {
        formError = "";
        if (!newName.trim()) {
            formError = "请输入模型名称";
            return;
        }
        if (!newModelPath.trim()) {
            formError = "请输入模型文件路径";
            return;
        }
        if (!newPort || newPort < 1 || newPort > 65535) {
            formError = "端口号范围 1-65535";
            return;
        }
        if (newNgl < 0 || !Number.isInteger(newNgl)) {
            formError = "ngl 必须为非负整数";
            return;
        }

        formSaving = true;
        try {
            const modelId = `model-${newName.trim().toLowerCase().replace(/\s+/g, "-").trim()}-${newPort}`;
            const newConfig: ModelConfig = {
                id: modelId,
                name: newName.trim(),
                model_path: newModelPath.trim(),
                server_path: newServerPath.trim() || "llama", // 空则默认使用 'llama'
                port: newPort,
                ngl: newNgl,
            };
            await invoke("save_model_config", { model: newConfig });
            showAddModal = false;
            await loadModels();
        } catch (e) {
            formError = `保存失败: ${e}`;
        } finally {
            formSaving = false;
        }
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
            <button class="error-dismiss" onclick={() => (error = "")}>✕</button
            >
        </div>
    {/if}

    <!-- 加载 -->
    {#if loading}
        <div class="loading">
            <div class="spinner"></div>
            <span>正在加载模型配置...</span>
        </div>
    {/if}

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
                {@const status = getModelStatus(model)}
                {@const isRunning = status === "running"}
                {@const isLoading = actionLoading === model.id}
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
                            class="status-badge {isRunning
                                ? 'status-running'
                                : 'status-stopped'}"
                        >
                            {isRunning ? "运行中" : "已停止"}
                        </span>
                    </div>

                    <div class="model-paths">
                        <div class="path-row">
                            <span class="path-label">模型</span>
                            <span class="path-value" title={model.model_path}
                                >{model.model_path}</span
                            >
                        </div>
                    </div>

                    <div class="model-footer">
                        <div class="footer-left">
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
                                    onclick={() => stopModel(model)}
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
                                        ><rect
                                            x="6"
                                            y="6"
                                            width="12"
                                            height="12"
                                            rx="1"
                                        /></svg
                                    >
                                    停止
                                </button>
                                <button
                                    class="btn-detail"
                                    onclick={() => goto(`/models/${model.id}`)}
                                >
                                    详情 →
                                </button>
                            {:else}
                                <button
                                    class="btn-start"
                                    onclick={() => startModel(model)}
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
                                        ><polygon
                                            points="5 3 19 12 5 21 5 3"
                                        /></svg
                                    >
                                    启动
                                </button>
                            {/if}
                            <button
                                class="btn-delete"
                                onclick={() => (deleteTarget = model)}
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
                        </div>
                    </div>
                </div>
            {/each}
        </div>
    {/if}
</div>

<!-- 添加模型弹窗 -->
{#if showAddModal}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div class="modal-overlay" role="presentation">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div class="modal" role="dialog" aria-label="添加模型" tabindex="-1">
            <div class="modal-header">
                <h2>添加模型</h2>
                <button
                    class="modal-close"
                    onclick={() => (showAddModal = false)}
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
            <div class="modal-body">
                {#if formError}
                    <div class="form-error">{formError}</div>
                {/if}
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>模型名称</label>
                    <input
                        type="text"
                        bind:value={newName}
                        placeholder="如: qwen2.5-7b"
                    />
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label
                        >llama 路径 <span class="optional-tag">(可选)</span
                        ></label
                    >
                    <input
                        type="text"
                        bind:value={newServerPath}
                        placeholder="留空使用系统 PATH 中的 llama"
                    />
                    <p class="input-hint">
                        若已配置环境变量，可留空直接使用 <code>llama</code> 命令
                    </p>
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>模型文件路径 (.gguf)</label>
                    <input
                        type="text"
                        bind:value={newModelPath}
                        placeholder="/path/to/model.gguf"
                    />
                </div>
                <div class="form-row">
                    <div class="form-group">
                        <!-- svelte-ignore a11y_label_has_associated_control -->
                        <label>端口</label>
                        <input
                            type="number"
                            bind:value={newPort}
                            min="1"
                            max="65535"
                        />
                    </div>
                    <div class="form-group">
                        <!-- svelte-ignore a11y_label_has_associated_control -->
                        <label>ngl (GPU 层数)</label>
                        <input type="number" bind:value={newNgl} min="0" />
                    </div>
                </div>
            </div>
            <div class="modal-footer">
                <button
                    class="btn-cancel"
                    onclick={() => (showAddModal = false)}>取消</button
                >
                <button
                    class="btn-save"
                    onclick={saveNewModel}
                    disabled={formSaving}
                >
                    {formSaving ? "保存中..." : "保存"}
                </button>
            </div>
        </div>
    </div>
{/if}

<!-- 删除确认弹窗 -->
{#if deleteTarget}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div
        class="modal-overlay"
        onclick={() => (deleteTarget = null)}
        role="presentation"
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div
            class="modal"
            onclick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="确认删除"
            tabindex="-1"
        >
            <div class="modal-header">
                <h2>确认删除</h2>
                <button
                    class="modal-close"
                    onclick={() => (deleteTarget = null)}
                    aria-label="取消"
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
                <button class="btn-danger" onclick={doDelete}>
                    确认删除
                </button>
            </div>
        </div>
    </div>
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

    .spinner {
        width: 24px;
        height: 24px;
        border: 3px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
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

    .mini-spinner {
        width: 12px;
        height: 12px;
        border: 2px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
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

    /* 弹窗 */
    .modal-overlay {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(0, 0, 0, 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 1000;
        animation: fadeInOverlay 0.2s ease;
    }

    @keyframes fadeInOverlay {
        from {
            opacity: 0;
        }
        to {
            opacity: 1;
        }
    }

    .modal {
        background: var(--bg-card);
        border-radius: 20px;
        width: 480px;
        max-width: 90vw;
        max-height: 85vh;
        overflow-y: auto;
        box-shadow: 0 24px 48px rgba(0, 0, 0, 0.3);
        border: 1px solid var(--border);
        animation: slideUp 0.25s ease;
    }

    @keyframes slideUp {
        from {
            opacity: 0;
            transform: translateY(16px) scale(0.97);
        }
        to {
            opacity: 1;
            transform: translateY(0) scale(1);
        }
    }

    .modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 20px 24px 0;
    }

    .modal-header h2 {
        font-size: 18px;
        font-weight: 700;
        color: var(--text-primary);
    }

    .modal-close {
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        padding: 4px;
        border-radius: 6px;
        transition: all 0.2s;
    }

    .modal-close:hover {
        background: var(--bg-subtle);
        color: var(--text-secondary);
    }

    .modal-body {
        padding: 20px 24px;
    }

    /* 表单 */
    .form-group {
        margin-bottom: 16px;
    }

    .form-group label {
        display: block;
        font-size: 13px;
        font-weight: 500;
        color: var(--text-secondary);
        margin-bottom: 6px;
    }

    .form-group input {
        width: 100%;
        padding: 10px 12px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 10px;
        font-size: 14px;
        color: var(--text-primary);
        outline: none;
        transition: all 0.2s;
        box-sizing: border-box;
    }

    .form-group input:focus {
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-ring);
    }

    .form-row {
        display: flex;
        gap: 12px;
    }

    .form-row .form-group {
        flex: 1;
    }

    .optional-tag {
        font-size: 12px;
        font-weight: 400;
        color: var(--text-muted);
    }

    .input-hint {
        margin-top: 6px;
        font-size: 12px;
        color: var(--text-muted);
        line-height: 1.4;
    }

    .input-hint code {
        background: var(--bg-subtle);
        padding: 1px 5px;
        border-radius: 4px;
        font-size: 11px;
        color: var(--accent);
    }

    .form-error {
        padding: 10px 14px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 8px;
        color: var(--error-muted);
        margin-bottom: 16px;
        color: var(--error-text);
        font-size: 13px;
    }

    .modal-footer {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        padding: 16px 24px 20px;
    }

    .btn-cancel {
        padding: 10px 20px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 10px;
        font-size: 14px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-cancel:hover {
        background: var(--bg-card-hover);
    }

    .btn-save {
        padding: 10px 24px;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-save:hover:not(:disabled) {
        background: var(--accent-hover);
    }

    .btn-save:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    /* 删除确认弹窗 */
    .confirm-icon {
        font-size: 40px;
        margin-bottom: 12px;
    }

    .confirm-text {
        font-size: 15px;
        color: var(--text-secondary);
        margin-bottom: 8px;
    }

    .confirm-path {
        font-size: 12px;
        color: var(--text-muted);
        font-family: ui-monospace, monospace;
        padding: 8px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border-light);
        border-radius: 8px;
    }

    .btn-danger {
        padding: 10px 20px;
        background: var(--error-text);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-danger:hover {
        background: #b91c1c;
        transform: translateY(-1px);
        box-shadow: 0 4px 12px rgba(220, 38, 38, 0.3);
    }
</style>
