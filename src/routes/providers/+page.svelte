<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { loadProviders as syncToolbarProviders } from "$lib/chat-state.svelte";
    import { goto } from "$app/navigation";

    // ===== 类型 =====
    interface ProviderConfig {
        id: string;
        name: string;
        base_url: string;
        api_key: string;
        active_model: string | null;
        created_at: number;
        updated_at: number;
    }

    // ===== 状态 =====
    let providers = $state<ProviderConfig[]>([]);
    let loading = $state(true);
    let error = $state("");
    let showAddModal = $state(false);
    let deleteTarget = $state<ProviderConfig | null>(null);

    // ===== 新表单 =====
    let newName = $state("");
    let newBaseUrl = $state("");
    let newApiKey = $state("");
    let formError = $state("");
    let formSaving = $state(false);
    let testing = $state<ProviderConfig | null>(null);

    // ===== 编辑表单 =====
    let showEditModal = $state(false);
    let editTarget = $state<ProviderConfig | null>(null);
    let editName = $state("");
    let editBaseUrl = $state("");
    let editApiKey = $state("");
    let editError = $state("");
    let editSaving = $state(false);

    // ===== 加载 =====
    onMount(() => {
        loadProviders();
    });

    async function loadProviders() {
        loading = true;
        error = "";
        try {
            providers = await invoke<ProviderConfig[]>("list_providers");
        } catch (e) {
            error = `加载失败: ${e}`;
        } finally {
            loading = false;
        }
    }

    // ===== 添加 =====
    function openAddModal() {
        newName = "";
        newBaseUrl = "";
        newApiKey = "";
        formError = "";
        formSaving = false;
        showAddModal = true;
    }

    async function saveNewProvider() {
        formError = "";
        if (!newName.trim()) {
            formError = "请输入名称";
            return;
        }
        if (!newBaseUrl.trim()) {
            formError = "请输入 API 地址";
            return;
        }

        formSaving = true;
        try {
            await invoke("save_provider", {
                name: newName.trim(),
                baseUrl: newBaseUrl.trim(),
                apiKey: newApiKey.trim(),
                activeModel: null,
            });
            showAddModal = false;
            await loadProviders();
            syncToolbarProviders(); // 同步全局 toolbarState
        } catch (e) {
            formError = `保存失败: ${e}`;
        } finally {
            formSaving = false;
        }
    }

    // ===== 编辑 =====
    function openEditModal(provider: ProviderConfig) {
        editTarget = provider;
        editName = provider.name;
        editBaseUrl = provider.base_url;
        editApiKey = provider.api_key;
        editError = "";
        editSaving = false;
        showEditModal = true;
    }

    async function saveEditProvider() {
        editError = "";
        if (!editTarget) return;
        if (!editName.trim()) {
            editError = "请输入名称";
            return;
        }
        if (!editBaseUrl.trim()) {
            editError = "请输入 API 地址";
            return;
        }

        editSaving = true;
        try {
            await invoke("save_provider", {
                id: editTarget.id,
                name: editName.trim(),
                baseUrl: editBaseUrl.trim(),
                apiKey: editApiKey.trim(),
                activeModel: editTarget.active_model,
            });
            showEditModal = false;
            editTarget = null;
            await loadProviders();
            syncToolbarProviders(); // 同步全局 toolbarState
        } catch (e) {
            editError = `保存失败: ${e}`;
        } finally {
            editSaving = false;
        }
    }

    // ===== 删除 =====
    async function doDelete() {
        const target = deleteTarget;
        if (!target) return;
        try {
            await invoke("delete_provider", { id: target.id });
            providers = providers.filter((p) => p.id !== target.id);
            syncToolbarProviders(); // 同步全局 toolbarState
            deleteTarget = null;
        } catch (e) {
            error = `删除失败: ${e}`;
            deleteTarget = null;
        }
    }

    // ===== 快速测试连接 =====
    async function testConnection(provider: ProviderConfig) {
        testing = provider;
        try {
            const models = await invoke<unknown[]>("fetch_provider_models", {
                baseUrl: provider.base_url,
                apiKey: provider.api_key,
            });
            if (models.length > 0) {
                alert(`连接成功！发现 ${models.length} 个可用模型`);
            } else {
                alert("连接成功，但未发现任何模型");
            }
        } catch (e) {
            alert(`连接失败: ${e}`);
        } finally {
            testing = null;
        }
    }

    function formatTime(ts: number): string {
        if (!ts) return "";
        const d = new Date(ts * 1000);
        return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    }

    function maskKey(key: string): string {
        if (!key) return "未设置";
        if (key.length <= 8) return "••••";
        return key.slice(0, 4) + "••••••••" + key.slice(-4);
    }
</script>

<div class="providers-page">
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
            <h1>API 接口管理</h1>
            <p class="subtitle">配置 OpenAI 兼容的 API 接口</p>
        </div>
        <div class="header-actions">
            <button
                class="btn-refresh"
                onclick={loadProviders}
                disabled={loading}
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
                添加接口
            </button>
        </div>
    </div>

    {#if error}
        <div class="error-banner">
            <span>⚠️</span>
            <span>{error}</span>
            <button class="error-dismiss" onclick={() => (error = "")}>✕</button
            >
        </div>
    {/if}

    {#if loading}
        <div class="loading">
            <div class="spinner"></div>
            <span>正在加载...</span>
        </div>
    {/if}

    {#if !loading && providers.length === 0 && !error}
        <div class="empty-state">
            <span class="empty-icon">🔌</span>
            <h3>还没有配置任何 API 接口</h3>
            <p>点击上方"添加接口"按钮，配置第一个 OpenAI 兼容 API</p>
        </div>
    {/if}

    {#if !loading && providers.length > 0}
        <div class="providers-count">{providers.length} 个接口</div>
        <div class="providers-grid">
            {#each providers as provider (provider.id)}
                {@const isTesting = testing?.id === provider.id}
                <div class="provider-card">
                    <div class="provider-header">
                        <span class="provider-icon">🔌</span>
                        <div class="provider-info">
                            <h3 class="provider-name">{provider.name}</h3>
                            <div class="provider-meta">
                                <span class="meta-tag">{provider.base_url}</span
                                >
                            </div>
                        </div>
                    </div>

                    <div class="provider-details">
                        <div class="detail-row">
                            <span class="detail-label">当前模型</span>
                            <span
                                class="detail-value"
                                title={provider.active_model ?? ""}
                            >
                                {provider.active_model ?? "未选择"}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">API Key</span>
                            <span class="detail-value mono"
                                >{maskKey(provider.api_key)}</span
                            >
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">创建时间</span>
                            <span class="detail-value"
                                >{formatTime(provider.created_at)}</span
                            >
                        </div>
                    </div>

                    <div class="provider-footer">
                        <div class="footer-left">
                            <button
                                class="btn-delete"
                                onclick={() => (deleteTarget = provider)}
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
                            {#if isTesting}
                                <span class="action-loading">
                                    <span class="mini-spinner"></span>
                                    测试中...
                                </span>
                            {/if}
                        </div>
                        <div class="footer-actions">
                            <button
                                class="btn-test"
                                onclick={() => testConnection(provider)}
                                disabled={isTesting}
                                title="测试连接"
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
                                    ><path
                                        d="M5 12.55a11 11 0 0 1 14.08 0"
                                    /><path
                                        d="M1.42 9a16 16 0 0 1 21.16 0"
                                    /><path
                                        d="M8.53 16.11a6 6 0 0 1 6.95 0"
                                    /><line
                                        x1="12"
                                        y1="20"
                                        x2="12.01"
                                        y2="20"
                                    /></svg
                                >
                                测试
                            </button>
                            <button
                                class="btn-edit"
                                onclick={() => openEditModal(provider)}
                                title="编辑"
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
                                    ><path
                                        d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"
                                    /><path
                                        d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"
                                    /></svg
                                >
                                编辑
                            </button>
                            <button
                                class="btn-detail"
                                onclick={() =>
                                    goto(`/providers/${provider.id}`)}
                            >
                                详情 →
                            </button>
                        </div>
                    </div>
                </div>
            {/each}
        </div>
    {/if}
</div>

<!-- 添加弹窗 -->
{#if showAddModal}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div class="modal-overlay" role="presentation">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div class="modal" role="dialog" aria-label="添加接口" tabindex="-1">
            <div class="modal-header">
                <h2>添加 API 接口</h2>
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
                    <label>名称</label>
                    <input
                        type="text"
                        bind:value={newName}
                        placeholder="如: 本地 Ollama"
                    />
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>API 地址</label>
                    <input
                        type="text"
                        bind:value={newBaseUrl}
                        placeholder="http://gateway.internal:8002/v1"
                    />
                    <p class="input-hint">
                        OpenAI 兼容的 API 地址，通常以 <code>/v1</code> 结尾
                    </p>
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label
                        >API Key <span class="optional-tag">(可选)</span></label
                    >
                    <input
                        type="password"
                        bind:value={newApiKey}
                        placeholder="sk-..."
                    />
                    <p class="input-hint">某些本地服务可能不需要 Key，可留空</p>
                </div>
            </div>
            <div class="modal-footer">
                <button
                    class="btn-cancel"
                    onclick={() => (showAddModal = false)}>取消</button
                >
                <button
                    class="btn-save"
                    onclick={saveNewProvider}
                    disabled={formSaving}
                >
                    {formSaving ? "保存中..." : "保存"}
                </button>
            </div>
        </div>
    </div>
{/if}

<!-- 编辑接口弹窗 -->
{#if showEditModal}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div class="modal-overlay" role="presentation">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div class="modal" role="dialog" aria-label="编辑接口" tabindex="-1">
            <div class="modal-header">
                <h2>编辑接口</h2>
                <button
                    class="modal-close"
                    onclick={() => (showEditModal = false)}
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
                {#if editError}
                    <div class="form-error">{editError}</div>
                {/if}
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>名称</label>
                    <input
                        type="text"
                        bind:value={editName}
                        placeholder="如: 本地 Ollama"
                    />
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>API 地址</label>
                    <input
                        type="text"
                        bind:value={editBaseUrl}
                        placeholder="http://gateway.internal:8002/v1"
                    />
                    <p class="input-hint">
                        OpenAI 兼容的 API 地址，通常以 <code>/v1</code> 结尾
                    </p>
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label
                        >API Key <span class="optional-tag">(可选)</span></label
                    >
                    <input
                        type="password"
                        bind:value={editApiKey}
                        placeholder="sk-..."
                    />
                    <p class="input-hint">某些本地服务可能不需要 Key，可留空</p>
                </div>
            </div>
            <div class="modal-footer">
                <button
                    class="btn-cancel"
                    onclick={() => (showEditModal = false)}>取消</button
                >
                <button
                    class="btn-save"
                    onclick={saveEditProvider}
                    disabled={editSaving}
                >
                    {editSaving ? "保存中..." : "保存"}
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
                    确定要删除接口 <strong>{deleteTarget.name}</strong> 吗？
                </p>
            </div>
            <div class="modal-footer">
                <button class="btn-cancel" onclick={() => (deleteTarget = null)}
                    >取消</button
                >
                <button class="btn-danger" onclick={doDelete}>确认删除</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .providers-page {
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

    .providers-count {
        font-size: 14px;
        color: var(--text-muted);
        margin-bottom: 16px;
    }

    .providers-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
        gap: 16px;
    }

    .provider-card {
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

    .provider-card:hover {
        box-shadow: 0 4px 12px var(--shadow-hover);
        border-color: var(--border);
    }

    .provider-header {
        display: flex;
        align-items: flex-start;
        gap: 12px;
    }

    .provider-icon {
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

    .provider-info {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .provider-name {
        font-size: 16px;
        font-weight: 600;
        color: var(--text-primary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .provider-meta {
        display: flex;
        gap: 8px;
        font-size: 12px;
        flex-wrap: wrap;
    }

    .meta-tag {
        color: var(--text-secondary);
        background: var(--bg-subtle);
        padding: 2px 8px;
        border-radius: 6px;
        font-family: ui-monospace, monospace;
        max-width: 300px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .provider-details {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .detail-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        font-size: 12px;
        gap: 8px;
    }

    .detail-label {
        color: var(--text-muted);
        flex-shrink: 0;
    }

    .detail-value {
        color: var(--text-secondary);
        font-family: ui-monospace, monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        text-align: right;
    }

    .detail-value.mono {
        font-family: ui-monospace, monospace;
    }

    .provider-footer {
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

    .btn-test {
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

    .btn-test:hover:not(:disabled) {
        background: var(--accent-bg);
        color: var(--accent);
        border-color: var(--accent);
    }

    .btn-test:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .btn-detail {
        display: inline-flex;
        align-items: center;
        padding: 6px 12px;
        background: var(--accent-bg);
        border: 1px solid transparent;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 500;
        color: var(--accent);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-detail:hover {
        background: var(--accent);
        color: white;
    }

    .btn-edit {
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

    .btn-edit:hover {
        background: var(--accent-bg);
        color: var(--accent);
        border-color: var(--accent);
    }

    .btn-delete {
        display: inline-flex;
        align-items: center;
        padding: 6px 8px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-delete:hover {
        background: var(--error-bg);
        color: var(--error-text);
        border-color: var(--error-border);
    }

    .modal-overlay {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(0, 0, 0, 0.45);
        z-index: 1000;
        display: flex;
        align-items: center;
        justify-content: center;
        animation: fadeIn 0.2s ease;
    }

    .modal {
        background: var(--bg-card);
        border-radius: 16px;
        width: 480px;
        max-width: 90vw;
        max-height: 90vh;
        overflow-y: auto;
        box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
        animation: modalIn 0.25s cubic-bezier(0.34, 1.56, 0.64, 1);
    }

    @keyframes modalIn {
        from {
            opacity: 0;
            transform: scale(0.92) translateY(20px);
        }
        to {
            opacity: 1;
            transform: scale(1) translateY(0);
        }
    }

    .modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 20px 24px;
        border-bottom: 1px solid var(--border-light);
    }

    .modal-header h2 {
        font-size: 18px;
        font-weight: 700;
        color: var(--text-primary);
    }

    .modal-close {
        background: var(--bg-subtle);
        border: none;
        border-radius: 8px;
        width: 30px;
        height: 30px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        color: var(--text-muted);
        transition: all 0.2s;
    }

    .modal-close:hover {
        background: var(--error-bg);
        color: var(--error-text);
    }

    .modal-body {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 16px;
    }

    .modal-footer {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        padding: 20px 24px;
        border-top: 1px solid var(--border-light);
    }

    .form-error {
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        color: var(--error-text);
        padding: 10px 14px;
        border-radius: 8px;
        font-size: 13px;
    }

    .form-group {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .form-group label {
        font-size: 13px;
        font-weight: 600;
        color: var(--text-secondary);
    }

    .form-group input {
        padding: 10px 12px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 14px;
        color: var(--text-primary);
        outline: none;
        transition: border-color 0.2s;
        font-family: inherit;
    }

    .form-group input:focus {
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-ring);
    }

    .optional-tag {
        font-weight: 400;
        font-size: 12px;
        color: var(--text-muted);
    }

    .input-hint {
        font-size: 12px;
        color: var(--text-muted);
    }

    .input-hint code {
        background: var(--bg-subtle);
        padding: 1px 5px;
        border-radius: 4px;
        font-size: 11px;
    }

    .btn-cancel {
        padding: 8px 18px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
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
        padding: 8px 18px;
        background: var(--accent);
        border: none;
        border-radius: 8px;
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

    .btn-danger {
        padding: 8px 18px;
        background: var(--error-text);
        border: none;
        border-radius: 8px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-danger:hover {
        opacity: 0.9;
    }

    .confirm-icon {
        text-align: center;
        font-size: 48px;
        margin-bottom: 12px;
    }

    .confirm-text {
        text-align: center;
        color: var(--text-secondary);
        font-size: 15px;
        line-height: 1.6;
    }

    .confirm-text strong {
        color: var(--text-primary);
    }
</style>
