<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { goto } from "$app/navigation";
    import { page } from "$app/stores";

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

    interface ModelInfo {
        id: string;
        object: string | null;
        created: number | null;
        owned_by: string | null;
    }

    // ===== 状态 =====
    let provider = $state<ProviderConfig | null>(null);
    let models = $state<ModelInfo[]>([]);
    let loading = $state(true);
    let modelsLoading = $state(false);
    let error = $state<string>("");
    let modelsError = $state<string>("");
    let modelsLoaded = $state(false);
    let selectedModelId = $state<string>(""); // 下拉选择器选中的模型
    let configCopied = $state(false); // 配置文档复制状态

    let showEditModal = $state(false);
    let editName = $state("");
    let editBaseUrl = $state("");
    let editApiKey = $state("");
    let editError = $state("");
    let editSaving = $state(false);

    let activeTab = $state<"curl" | "python">("curl");
    let copied = $state(false);

    const providerId = $derived($page.params.id);

    // ===== 派生 =====
    const curlExample = $derived(
        provider
            ? `curl ${provider.base_url}/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer ${provider.api_key || "YOUR_API_KEY"}" \\
  -d '{
    "model": "${provider.active_model ?? "MODEL_NAME"}",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'`
            : "",
    );

    const pythonExample = $derived(
        provider
            ? `from openai import OpenAI

  client = OpenAI(
      base_url="${provider.base_url}",
      api_key="${provider.api_key || "YOUR_API_KEY"}",
  )

  response = client.chat.completions.create(
      model="${provider.active_model ?? "MODEL_NAME"}",
      messages=[
          {"role": "user", "content": "Hello"}
      ],
  )

  print(response.choices[0].message.content)`
            : "",
    );

    // ===== 模型配置文档 =====
    const modelConfigJson = $derived(
        provider && models.length > 0
            ? JSON.stringify(
                  {
                      api_url: provider.base_url,
                      available_models: models.map((m) => ({
                          name: m.id,
                          max_tokens: 200000,
                          max_output_tokens: 32000,
                          max_completion_tokens: 200000,
                          capabilities: {
                              tools: true,
                              images: false,
                              parallel_tool_calls: false,
                              prompt_cache_key: false,
                              chat_completions: true,
                              interleaved_reasoning: false,
                          },
                      })),
                  },
                  null,
                  2,
              )
            : "",
    );

    // ===== 加载 =====
    onMount(async () => {
        await loadProvider();
    });

    async function loadProvider() {
        loading = true;
        error = "";
        try {
            provider = await invoke<ProviderConfig>("get_provider", {
                id: providerId,
            });
            // 如果有 active_model，自动加载模型列表
            await fetchModels();
        } catch (e) {
            error = `加载失败: ${e}`;
        } finally {
            loading = false;
        }
    }

    async function fetchModels() {
        if (!provider) return;
        modelsLoading = true;
        modelsError = "";
        try {
            models = await invoke<ModelInfo[]>("fetch_provider_models", {
                baseUrl: provider.base_url,
                apiKey: provider.api_key,
            });
            modelsLoaded = true;
        } catch (e) {
            modelsError = `获取模型失败: ${e}`;
            modelsLoaded = true;
        } finally {
            modelsLoading = false;
        }
    }

    // ===== 切换模型 =====
    async function selectModel(modelId: string) {
        if (!provider || modelId === provider.active_model) return;
        try {
            await invoke("set_active_model", {
                providerId: provider.id,
                modelId,
            });
            provider.active_model = modelId;
            selectedModelId = modelId;
        } catch (e) {
            error = `切换模型失败: ${e}`;
        }
    }

    // 下拉选择器变化时切换模型
    function onModelSelectChange(e: Event) {
        const target = e.target as HTMLSelectElement;
        const modelId = target.value;
        if (modelId) {
            selectModel(modelId);
        }
    }

    // ===== 编辑 =====
    function openEditModal() {
        if (!provider) return;
        editName = provider.name;
        editBaseUrl = provider.base_url;
        editApiKey = provider.api_key;
        editError = "";
        editSaving = false;
        showEditModal = true;
    }

    async function saveEdit() {
        editError = "";
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
                id: provider!.id,
                name: editName.trim(),
                baseUrl: editBaseUrl.trim(),
                apiKey: editApiKey.trim(),
                activeModel: provider!.active_model,
            });
            showEditModal = false;
            await loadProvider();
        } catch (e) {
            editError = `保存失败: ${e}`;
        } finally {
            editSaving = false;
        }
    }

    // ===== 删除 =====
    let showDeleteConfirm = $state(false);

    async function doDelete() {
        if (!provider) return;
        try {
            await invoke("delete_provider", { id: provider.id });
            goto("/providers");
        } catch (e) {
            error = `删除失败: ${e}`;
            showDeleteConfirm = false;
        }
    }

    // ===== 复制 =====
    async function copyCode(text: string) {
        try {
            await navigator.clipboard.writeText(text);
            copied = true;
            setTimeout(() => (copied = false), 2000);
        } catch {
            // ignore
        }
    }

    // ===== 复制配置文档 =====
    async function copyConfig() {
        try {
            await navigator.clipboard.writeText(modelConfigJson);
            configCopied = true;
            setTimeout(() => (configCopied = false), 2000);
        } catch {
            // ignore
        }
    }

    // ===== 辅助 =====
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

<div class="detail-page">
    <!-- 返回 -->
    <button class="back-btn" onclick={() => goto("/providers")}>
        <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><line x1="19" y1="12" x2="5" y2="12" /><polyline
                points="12 19 5 12 12 5"
            /></svg
        >
        返回列表
    </button>

    {#if loading}
        <div class="loading">
            <div class="spinner"></div>
            <span>正在加载...</span>
        </div>
    {/if}

    {#if error && !provider}
        <div class="error-banner">{error}</div>
    {/if}

    {#if provider}
        <!-- 头部信息卡片 -->
        <div class="info-card">
            <div class="card-header-row">
                <div class="card-header-left">
                    <span class="card-icon">🔌</span>
                    <div>
                        <h1 class="card-title">{provider.name}</h1>
                        <p class="card-subtitle">OpenAI 兼容 API</p>
                    </div>
                </div>
                <div class="card-header-actions">
                    <button class="btn-edit" onclick={openEditModal}>
                        <svg
                            width="15"
                            height="15"
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
                        class="btn-danger-sm"
                        onclick={() => (showDeleteConfirm = true)}
                    >
                        <svg
                            width="15"
                            height="15"
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
                        删除
                    </button>
                </div>
            </div>

            <!-- 基本信息网格 -->
            <div class="info-grid">
                <div class="info-item">
                    <span class="info-label">API 地址</span>
                    <span class="info-value mono">{provider.base_url}</span>
                </div>
                <div class="info-item">
                    <span class="info-label">API Key</span>
                    <span class="info-value mono"
                        >{maskKey(provider.api_key)}</span
                    >
                </div>
                <div class="info-item">
                    <span class="info-label">当前模型</span>
                    <span class="info-value mono highlight"
                        >{provider.active_model ?? "未选择"}</span
                    >
                </div>
                <div class="info-item">
                    <span class="info-label">创建时间</span>
                    <span class="info-value"
                        >{formatTime(provider.created_at)}</span
                    >
                </div>
                <div class="info-item">
                    <span class="info-label">更新时间</span>
                    <span class="info-value"
                        >{formatTime(provider.updated_at)}</span
                    >
                </div>
                <div class="info-item">
                    <span class="info-label">模型数量</span>
                    <span class="info-value"
                        >{models.length > 0 ? models.length : "—"}</span
                    >
                </div>
            </div>
        </div>

        {#if error}
            <div class="error-banner">{error}</div>
        {/if}

        <!-- 模型列表 -->
        <div class="section-card">
            <div class="section-header">
                <div class="section-title-row">
                    <h2 class="section-title">
                        <svg
                            width="20"
                            height="20"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><rect
                                x="3"
                                y="3"
                                width="18"
                                height="18"
                                rx="2"
                                ry="2"
                            /><line x1="9" y1="3" x2="9" y2="21" /></svg
                        >
                        可用模型
                    </h2>
                    {#if modelsLoading}
                        <span class="badge-loading">
                            <span class="mini-spinner"></span>
                            获取中...
                        </span>
                    {:else if modelsLoaded && models.length > 0}
                        <span class="badge-count">{models.length} 个</span>
                    {/if}
                </div>
                <button
                    class="btn-refresh-models"
                    onclick={fetchModels}
                    disabled={modelsLoading}
                >
                    <svg
                        width="15"
                        height="15"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        class:spinning={modelsLoading}
                        ><polyline points="23 4 23 10 17 10" /><path
                            d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"
                        /></svg
                    >
                    刷新模型列表
                </button>
            </div>

            {#if modelsError}
                <div class="error-banner">{modelsError}</div>
            {/if}

            {#if !modelsLoading && !modelsLoaded}
                <div class="empty-models">
                    <span class="empty-icon-sm">📋</span>
                    <p>点击"刷新模型列表"获取可用模型</p>
                </div>
            {/if}

            {#if modelsLoading}
                <div class="loading-sm">
                    <div class="mini-spinner"></div>
                    <span>正在获取模型列表...</span>
                </div>
            {/if}

            {#if !modelsLoading && modelsLoaded && models.length === 0 && !modelsError}
                <div class="empty-models">
                    <span class="empty-icon-sm">📭</span>
                    <p>未发现任何模型</p>
                </div>
            {/if}

            {#if !modelsLoading && models.length > 0}
                <!-- 模型切换栏：下拉选择器 + 搜索过滤 -->
                <div class="model-selector-bar">
                    <div class="model-select-wrapper">
                        <select
                            class="model-select"
                            onchange={onModelSelectChange}
                            bind:value={selectedModelId}
                        >
                            {#each models as model (model.id)}
                                <option
                                    value={model.id}
                                    selected={model.id ===
                                        provider.active_model}
                                    >{model.id}</option
                                >
                            {/each}
                        </select>
                        <svg
                            class="select-chevron"
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><polyline points="6 9 12 15 18 9" /></svg
                        >
                    </div>
                    <span class="active-model-badge"
                        >当前: {provider.active_model}</span
                    >
                </div>

                <!-- 全部模型列表（紧凑展示 + 搜索） -->
                <div class="model-compact-list">
                    {#each models as model (model.id)}
                        {@const isActive = provider.active_model === model.id}
                        <button
                            class="model-chip"
                            class:active={isActive}
                            onclick={() => selectModel(model.id)}
                            title={model.id}
                        >
                            {model.id}
                        </button>
                    {/each}
                </div>
            {/if}
        </div>

        <!-- 代码示例 -->
        <div class="section-card">
            <div class="section-header">
                <h2 class="section-title">
                    <svg
                        width="20"
                        height="20"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><polyline points="16 18 22 12 16 6" /><polyline
                            points="8 6 2 12 8 18"
                        /></svg
                    >
                    调用示例
                </h2>
                <button
                    class="btn-copy"
                    onclick={() =>
                        copyCode(
                            activeTab === "curl" ? curlExample : pythonExample,
                        )}
                >
                    {#if copied}
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><polyline points="20 6 9 17 4 12" /></svg
                        >
                        已复制
                    {:else}
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
                        复制
                    {/if}
                </button>
            </div>

            <div class="tabs">
                <button
                    class="tab"
                    class:active={activeTab === "curl"}
                    onclick={() => (activeTab = "curl")}>cURL</button
                >
                <button
                    class="tab"
                    class:active={activeTab === "python"}
                    onclick={() => (activeTab = "python")}>Python</button
                >
            </div>

            <pre class="code-block"><code
                    >{activeTab === "curl" ? curlExample : pythonExample}</code
                ></pre>
        </div>

        <!-- 模型配置文档 -->
        {#if modelConfigJson}
            <div class="section-card">
                <div class="section-header">
                    <h2 class="section-title">
                        <svg
                            width="20"
                            height="20"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><path
                                d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
                            /><polyline points="14 2 14 8 20 8" /><line
                                x1="16"
                                y1="13"
                                x2="8"
                                y2="13"
                            /><line x1="16" y1="17" x2="8" y2="17" /><polyline
                                points="10 9 9 9 8 9"
                            /></svg
                        >
                        模型配置文档
                    </h2>
                    <button class="btn-copy" onclick={copyConfig}>
                        {#if configCopied}
                            <svg
                                width="14"
                                height="14"
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2.5"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                ><polyline points="20 6 9 17 4 12" /></svg
                            >
                            已复制
                        {:else}
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
                            复制
                        {/if}
                    </button>
                </div>
                <pre class="code-block config-json"><code
                        >{modelConfigJson}</code
                    ></pre>
            </div>
        {/if}
    {/if}
</div>

<!-- 编辑弹窗 -->
{#if showEditModal}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div
        class="modal-overlay"
        onclick={() => (showEditModal = false)}
        role="presentation"
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div
            class="modal"
            onclick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="编辑接口"
            tabindex="-1"
        >
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
                    <input type="text" bind:value={editName} />
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>API 地址</label>
                    <input type="text" bind:value={editBaseUrl} />
                </div>
                <div class="form-group">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>API Key</label>
                    <input type="password" bind:value={editApiKey} />
                </div>
            </div>
            <div class="modal-footer">
                <button
                    class="btn-cancel"
                    onclick={() => (showEditModal = false)}>取消</button
                >
                <button
                    class="btn-save"
                    onclick={saveEdit}
                    disabled={editSaving}
                >
                    {editSaving ? "保存中..." : "保存"}
                </button>
            </div>
        </div>
    </div>
{/if}

<!-- 删除确认 -->
{#if showDeleteConfirm}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div
        class="modal-overlay"
        onclick={() => (showDeleteConfirm = false)}
        role="presentation"
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
        <div
            class="modal modal-sm"
            onclick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="确认删除"
            tabindex="-1"
        >
            <div class="modal-header">
                <h2>确认删除</h2>
                <button
                    class="modal-close"
                    onclick={() => (showDeleteConfirm = false)}
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
                    确定要删除接口 <strong>{provider?.name}</strong> 吗？此操作不可撤销。
                </p>
            </div>
            <div class="modal-footer">
                <button
                    class="btn-cancel"
                    onclick={() => (showDeleteConfirm = false)}>取消</button
                >
                <button class="btn-danger" onclick={doDelete}>确认删除</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .detail-page {
        max-width: 900px;
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

    .back-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 6px 12px;
        background: none;
        border: none;
        color: var(--text-secondary);
        font-size: 14px;
        cursor: pointer;
        margin-bottom: 20px;
        transition: color 0.2s;
    }

    .back-btn:hover {
        color: var(--accent);
    }

    /* 信息卡片 */
    .info-card {
        background: var(--bg-card);
        border-radius: 16px;
        padding: 24px;
        box-shadow: 0 1px 3px var(--shadow-sm);
        border: 1px solid var(--border-light);
        margin-bottom: 20px;
    }

    .card-header-row {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        margin-bottom: 20px;
    }

    .card-header-left {
        display: flex;
        align-items: center;
        gap: 14px;
    }

    .card-icon {
        font-size: 32px;
        width: 56px;
        height: 56px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--bg-subtle);
        border-radius: 14px;
        flex-shrink: 0;
    }

    .card-title {
        font-size: 22px;
        font-weight: 700;
        color: var(--text-primary);
        margin-bottom: 4px;
    }

    .card-subtitle {
        font-size: 13px;
        color: var(--text-muted);
    }

    .card-header-actions {
        display: flex;
        gap: 8px;
    }

    .btn-edit {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 7px 14px;
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

    .btn-danger-sm {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 7px 14px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 13px;
        font-weight: 500;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-danger-sm:hover {
        background: var(--error-bg);
        color: var(--error-text);
        border-color: var(--error-border);
    }

    /* 信息网格 */
    .info-grid {
        display: grid;
        grid-template-columns: repeat(2, 1fr);
        gap: 16px;
    }

    .info-item {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .info-label {
        font-size: 12px;
        color: var(--text-muted);
        font-weight: 500;
    }

    .info-value {
        font-size: 14px;
        color: var(--text-primary);
    }

    .info-value.mono {
        font-family: ui-monospace, monospace;
        font-size: 13px;
        word-break: break-all;
    }

    .info-value.highlight {
        color: var(--accent);
        font-weight: 600;
    }

    @media (max-width: 600px) {
        .info-grid {
            grid-template-columns: 1fr;
        }
    }

    /* 区块卡片 */
    .section-card {
        background: var(--bg-card);
        border-radius: 16px;
        padding: 24px;
        box-shadow: 0 1px 3px var(--shadow-sm);
        border: 1px solid var(--border-light);
        margin-bottom: 20px;
    }

    .section-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 16px;
        flex-wrap: wrap;
        gap: 10px;
    }

    .section-title-row {
        display: flex;
        align-items: center;
        gap: 10px;
    }

    .section-title {
        font-size: 17px;
        font-weight: 700;
        color: var(--text-primary);
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .badge-count {
        font-size: 12px;
        color: var(--accent);
        background: var(--accent-bg);
        padding: 2px 10px;
        border-radius: 10px;
        font-weight: 600;
    }

    .badge-loading {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--text-muted);
    }

    .btn-refresh-models {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 7px 14px;
        background: var(--accent);
        border: none;
        border-radius: 8px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .btn-refresh-models:hover:not(:disabled) {
        background: var(--accent-hover);
        transform: translateY(-1px);
        box-shadow: 0 4px 12px var(--accent-shadow);
    }

    .btn-refresh-models:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .spinning {
        animation: spin 0.8s linear infinite;
    }

    /* 模型切换栏：下拉选择器 */
    .model-selector-bar {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 14px;
        flex-wrap: wrap;
    }

    .model-select-wrapper {
        position: relative;
        flex: 1;
        min-width: 200px;
        max-width: 400px;
    }

    .model-select {
        width: 100%;
        padding: 9px 36px 9px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 10px;
        font-size: 13px;
        font-family: ui-monospace, monospace;
        color: var(--text-primary);
        cursor: pointer;
        appearance: none;
        -webkit-appearance: none;
        transition: border-color 0.15s;
        outline: none;
    }

    .model-select:focus {
        border-color: var(--accent);
    }

    .select-chevron {
        position: absolute;
        right: 12px;
        top: 50%;
        transform: translateY(-50%);
        pointer-events: none;
        color: var(--text-muted);
    }

    .active-model-badge {
        font-size: 12px;
        font-weight: 600;
        color: var(--accent);
        background: var(--accent-bg);
        padding: 5px 12px;
        border-radius: 8px;
        white-space: nowrap;
    }

    /* 紧凑模型芯片列表 */
    .model-compact-list {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        max-height: 180px;
        overflow-y: auto;
        padding: 2px 0;
    }

    .model-chip {
        display: inline-flex;
        align-items: center;
        padding: 4px 10px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 11px;
        font-family: ui-monospace, monospace;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s;
        max-width: 220px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .model-chip:hover {
        border-color: var(--accent);
        color: var(--accent);
        background: var(--accent-bg);
    }

    .model-chip.active {
        background: var(--accent);
        color: white;
        border-color: var(--accent);
        font-weight: 600;
    }

    .empty-models {
        text-align: center;
        padding: 40px 20px;
        color: var(--text-muted);
    }

    .empty-icon-sm {
        font-size: 36px;
        display: block;
        margin-bottom: 10px;
    }

    .loading-sm {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 8px;
        padding: 40px;
        color: var(--text-secondary);
        font-size: 14px;
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

    .mini-spinner {
        width: 14px;
        height: 14px;
        border: 2px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }

    /* 代码 */
    .btn-copy {
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

    .btn-copy:hover {
        background: var(--accent-bg);
        color: var(--accent);
        border-color: var(--accent);
    }

    .tabs {
        display: flex;
        gap: 4px;
        margin-bottom: 12px;
        border-bottom: 1px solid var(--border-light);
    }

    .tab {
        padding: 8px 16px;
        background: none;
        border: none;
        border-bottom: 2px solid transparent;
        font-size: 14px;
        font-weight: 500;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.2s;
    }

    .tab:hover {
        color: var(--text-primary);
    }

    .tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }

    .code-block {
        background: #0f0f1a;
        color: #e2e8f0;
        border-radius: 10px;
        padding: 16px;
        overflow-x: auto;
        font-family: ui-monospace, "SF Mono", Monaco, monospace;
        font-size: 13px;
        line-height: 1.6;
        margin: 0;
    }

    .config-json {
        max-height: 500px;
        overflow-y: auto;
        font-size: 12px;
        line-height: 1.5;
    }

    /* 弹窗 */
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

    .modal-sm {
        width: 400px;
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
