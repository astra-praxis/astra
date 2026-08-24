<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy, tick } from "svelte";
    import { workspaceStore } from "$lib/workspace.svelte";
    import { toolbarState, type ProviderConfig, type ModelInfo, type RunningModelInfo } from "$lib/chat-state.svelte";
    import type { ChatMessage, ChatSession, ChatSource } from "./types";
    import { buildWorkspaceContext, parseMentions } from "./workspace-context";
    import { handleActionClick } from "./action-links";
    import HistoryDrawer from "./HistoryDrawer.svelte";
    import SettingsSheet from "./SettingsSheet.svelte";
    import Welcome from "./Welcome.svelte";
    import ChatMessageView from "./ChatMessageView.svelte";
    import Composer from "./Composer.svelte";

    // ===== 布局 =====
    let sidebarOpen = $state(false);

    // ===== 来源数据 =====
    let runningModels = $state<RunningModelInfo[]>([]);
    let providers = $state<ProviderConfig[]>([]);
    let providerModels = $state<ModelInfo[]>([]);
    let modelsLoading = $state(false);
    let modelsError = $state("");

    // ===== 当前会话 =====
    let currentSessionId = $state<string | null>(null);
    let messages = $state<ChatMessage[]>([]);
    let input = $state("");
    let isSending = $state(false);
    let error = $state("");
    let messagesEl: HTMLElement | null = null;

    // 选择状态直接使用 toolbarState（与全局 ChatToolbar 同步）

    // ===== 参数 =====
    let showSettings = $state(false);
    let systemPrompt = $state("你是一个有用的助手，请简洁准确地回答用户问题。");
    let temperature = $state(0.7);
    let maxTokens = $state(4000);

    let sessions = $state<ChatSession[]>([]);

    let unlisteners: Array<() => void> = [];

    // ===== 生命周期 =====
    onMount(async () => {
        await Promise.all([
            loadSessions(),
            loadRunningModels(),
            loadProviders(),
            workspaceStore.loadFromSettings(),
        ]);

        // 优先复用 toolbarState 的已选值（用户在 toolbar 上可能已切换过）
        if (toolbarState.selectedProviderId && providers.some(p => p.id === toolbarState.selectedProviderId)) {
            // keep existing selection
        } else if (providers.length > 0) {
            toolbarState.selectedProviderId = providers[0].id;
        }
        if (toolbarState.selectedProviderId) handleFetchModels();

        if (toolbarState.selectedModelPort && runningModels.some(m => m.port === toolbarState.selectedModelPort)) {
            // keep existing selection
        } else if (runningModels.length > 0) {
            toolbarState.selectedModelPort = runningModels[0].port;
        }

        const unChunk = await listen<string>("chat-chunk", (e) => {
            const last = messages[messages.length - 1];
            if (last && last.role === "assistant") {
                last.content += e.payload;
                messages = [...messages];
                scrollToBottom();
            }
        });

        const unReasoning = await listen<string>(
            "chat-chunk-reasoning",
            (e) => {
                const last = messages[messages.length - 1];
                if (last && last.role === "assistant") {
                    last.reasoning = (last.reasoning ?? "") + e.payload;
                    messages = [...messages];
                    scrollToBottom();
                }
            },
        );

        const unDone = await listen("chat-done", () => {
            isSending = false;
            saveCurrentSession();
        });

        const unError = await listen<string>("chat-error", (e) => {
            isSending = false;
            const last = messages[messages.length - 1];
            if (last && last.role === "assistant" && !last.content) {
                last.content = e.payload;
                last.error = true;
                messages = [...messages];
            } else {
                error = e.payload;
            }
        });

        unlisteners.push(unChunk, unReasoning, unDone, unError);
    });

    onDestroy(() => {
        unlisteners.forEach((fn) => fn());
    });

    // ===== 派生 =====
    const selectedProvider = $derived(
        providers.find((p) => p.id === toolbarState.selectedProviderId) ?? null,
    );

    const selectedModel = $derived(
        runningModels.find((m) => m.port === toolbarState.selectedModelPort) ?? null,
    );

    const currentModelName = $derived.by(() => {
        if (toolbarState.sourceType === "model") return selectedModel?.name ?? "local";
        return toolbarState.overrideModelName ?? selectedProvider?.active_model ?? null;
    });

    const currentSourceLabel = $derived.by(() => {
        if (toolbarState.sourceType === "model") return selectedModel?.name ?? "本地模型";
        return selectedProvider?.name ?? "未选择";
    });

    const canSend = $derived.by(() => {
        if (isSending || input.trim().length === 0) return false;
        if (toolbarState.sourceType === "model") return !!toolbarState.selectedModelPort;
        return !!toolbarState.selectedProviderId && !!currentModelName;
    });

    // ===== 加载 =====
    async function loadSessions() {
        try {
            sessions = await invoke<ChatSession[]>("list_chat_sessions");
        } catch (e) {
            console.error("加载聊天记录失败", e);
        }
    }

    async function loadRunningModels() {
        try {
            const all = await invoke<RunningModelInfo[]>(
                "list_running_servers",
            );
            runningModels = all.filter((m) => m.status === "running");
        } catch (e) {
            console.error("加载运行中的模型失败", e);
        }
    }

    async function loadProviders() {
        try {
            providers = await invoke<ProviderConfig[]>("list_providers");
        } catch (e) {
            console.error("加载 providers 失败", e);
        }
    }

    async function handleFetchModels() {
        if (!selectedProvider) {
            modelsError = "请先选择一个 Provider";
            return;
        }
        modelsLoading = true;
        modelsError = "";
        try {
            providerModels = await invoke<ModelInfo[]>(
                "fetch_provider_models",
                {
                    baseUrl: selectedProvider.base_url,
                    apiKey: selectedProvider.api_key,
                },
            );
        } catch (e) {
            modelsError = String(e);
            providerModels = [];
        } finally {
            modelsLoading = false;
        }
    }

    function onSwitchType(t: "provider" | "model") {
        if (toolbarState.sourceType === t || isSending) return;
        toolbarState.sourceType = t;
        error = "";
        if (t === "model" && runningModels.length > 0 && !toolbarState.selectedModelPort) {
            toolbarState.selectedModelPort = runningModels[0].port;
        }
        if (t === "provider" && providers.length > 0 && !toolbarState.selectedProviderId) {
            toolbarState.selectedProviderId = providers[0].id;
            handleFetchModels();
        }
    }

    function onSelectProviderChange() {
        toolbarState.overrideModelName = null;
        providerModels = [];
        if (toolbarState.selectedProviderId) handleFetchModels();
    }

    // ===== 会话管理 =====
    function newChat() {
        if (isSending) return;
        currentSessionId = null;
        messages = [];
        error = "";
        input = "";
        toolbarState.overrideModelName = null;
        sidebarOpen = false;
    }

    async function selectSession(s: ChatSession) {
        if (isSending) return;
        currentSessionId = s.id;
        error = "";
        input = "";

        messages = (s.messages as ChatMessage[]).map((m) => ({
            role: m.role as "user" | "assistant",
            content: m.content,
            reasoning: m.reasoning,
            timestamp: m.timestamp ?? 0,
            error: m.error,
            favorite: m.favorite,
            showReasoning: false,
            isFresh: false, // 历史消息不自动执行 action
        }));

        if (s.source.type === "provider") {
            toolbarState.sourceType = "provider";
            const sid = s.source.provider_id;
            toolbarState.selectedProviderId = sid;
            const provider = providers.find((p) => p.id === sid);
            toolbarState.overrideModelName =
                s.source.model ?? provider?.active_model ?? null;
            if (provider) handleFetchModels();
        } else if (s.source.type === "model") {
            toolbarState.sourceType = "model";
            toolbarState.selectedModelPort = s.source.port;
        }

        sidebarOpen = false;
        await scrollToBottom();
    }

    function deleteSession(id: string) {
        if (isSending) return;
        invoke("delete_chat_session", { id })
            .then(() => {
                sessions = sessions.filter((s) => s.id !== id);
                if (currentSessionId === id) newChat();
            })
            .catch((err) => {
                error = `删除失败: ${err}`;
            });
    }

    function genTitle(): string {
        const firstUser = messages.find((m) => m.role === "user");
        if (firstUser) {
            const t = firstUser.content.trim().slice(0, 24);
            return t + (firstUser.content.length > 24 ? "…" : "");
        }
        return "新对话";
    }

    function buildSource(): ChatSource {
        if (toolbarState.sourceType === "model") {
            return {
                type: "model",
                port: toolbarState.selectedModelPort!,
                model_name: selectedModel?.name ?? "local",
            };
        }
        return {
            type: "provider",
            provider_id: toolbarState.selectedProviderId!,
            model: toolbarState.overrideModelName,
        };
    }

    async function saveCurrentSession() {
        if (messages.length === 0) return;

        const source = buildSource();
        const payloadMessages = messages.map((m) => ({
            role: m.role,
            content: m.content,
            reasoning: m.reasoning,
            timestamp: m.timestamp,
            error: m.error,
            favorite: m.favorite,
        }));

        try {
            const saved = await invoke<ChatSession>("save_chat_session", {
                id: currentSessionId,
                title: genTitle(),
                source,
                messages: payloadMessages,
            });
            currentSessionId = saved.id;

            const idx = sessions.findIndex((s) => s.id === saved.id);
            if (idx >= 0) {
                sessions[idx] = saved;
                sessions = [...sessions].sort(
                    (a, b) => b.updated_at - a.updated_at,
                );
            } else {
                sessions = [saved, ...sessions];
            }
        } catch (e) {
            console.error("保存会话失败", e);
        }
    }

    // ===== 发送 =====
    async function handleSend() {
        const text = input.trim();
        if (!text || isSending) return;

        if (toolbarState.sourceType === "model" && !toolbarState.selectedModelPort) {
            error = "请先选择一个运行中的本地模型";
            return;
        }
        if (toolbarState.sourceType === "provider") {
            if (!selectedProvider) {
                error = "请先选择一个 API 提供者";
                return;
            }
            if (!currentModelName) {
                error = `请先点击"刷新模型列表"并选择一个模型`;
                return;
            }
        }

        input = "";
        error = "";

        const userMsg: ChatMessage = {
            role: "user",
            content: text,
            timestamp: Date.now(),
        };
        const placeholder: ChatMessage = {
            role: "assistant",
            content: "",
            timestamp: Date.now(),
            isFresh: true, // 标记为新消息
        };
        messages = [...messages, userMsg, placeholder];
        await scrollToBottom();

        isSending = true;

        const context: Array<{ role: string; content: string }> = [];

        // 构建增强的系统提示（包含工作空间信息）
        const enhancedSystemPrompt =
            systemPrompt.trim() + buildWorkspaceContext();
        if (enhancedSystemPrompt) {
            context.push({ role: "system", content: enhancedSystemPrompt });
        }

        // 解析用户消息中的 @ 提及，如果有提及则补充项目信息
        const mentions = parseMentions(text);
        let enhancedUserContent = text;
        if (mentions.length > 0) {
            const mentionInfo = mentions
                .map((p) => `【引用项目】${p.name}: ${p.path}`)
                .join("\n");
            enhancedUserContent = `${text}\n\n${mentionInfo}`;
        }

        for (let i = 0; i < messages.length - 1; i++) {
            if (messages[i].error) continue;
            if (i === messages.length - 2 && messages[i].role === "user") {
                // 最后一条用户消息使用增强版本
                context.push({ role: "user", content: enhancedUserContent });
            } else {
                context.push({
                    role: messages[i].role,
                    content: messages[i].content,
                });
            }
        }

        let source: ChatSource;
        if (toolbarState.sourceType === "model") {
            source = buildSource();
        } else {
            source = {
                type: "provider",
                provider_id: toolbarState.selectedProviderId!,
                model: currentModelName,
            };
        }

        try {
            await invoke("send_chat", {
                req: {
                    source,
                    messages: context,
                    max_tokens: maxTokens,
                    temperature,
                },
            });
        } catch (e) {
            isSending = false;
            const last = messages[messages.length - 1];
            if (last && !last.content) {
                last.content = `❌ 调用失败: ${e}`;
                last.error = true;
                messages = [...messages];
            } else {
                error = String(e);
            }
        }
    }

    async function handleStop() {
        try {
            await invoke("stop_chat");
        } catch {
            // ignore
        }
        isSending = false;
    }

    function handleClear() {
        if (isSending) return;
        messages = [];
        error = "";
        currentSessionId = null;
    }

    async function scrollToBottom() {
        await tick();
        if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    // ===== 消息操作 =====
    async function copyMessage(msg: ChatMessage) {
        try {
            await navigator.clipboard.writeText(msg.content);
        } catch {
            // ignore
        }
    }

    function toggleFavorite(msg: ChatMessage) {
        msg.favorite = !msg.favorite;
        messages = [...messages];
        // 自动保存
        saveCurrentSession();
    }

    // 同步状态到工具栏 store + 设置回调
    $effect(() => {
        toolbarState.isSending = isSending;
        toolbarState.showSettings = showSettings;
        toolbarState.sessionsCount = sessions.length;
    });

    // 同步 Provider / 模型数据到 toolbarState（全局 ChatToolbar 的数据源）
    $effect(() => {
        toolbarState.providers = providers;
    });
    $effect(() => {
        toolbarState.runningModels = runningModels;
    });
    $effect(() => {
        toolbarState.providerModels = providerModels;
        toolbarState.modelsLoading = modelsLoading;
    });

    onMount(() => {
        toolbarState.onClear = handleClear;
        toolbarState.onToggleSidebar = () => (sidebarOpen = !sidebarOpen);
    });
</script>

<div class="chat-root" class:with-sidebar={sidebarOpen}>
    <HistoryDrawer
        open={sidebarOpen}
        sessions={sessions}
        currentSessionId={currentSessionId}
        isSending={isSending}
        providers={providers}
        onClose={() => (sidebarOpen = false)}
        onNewChat={newChat}
        onSelect={selectSession}
        onDelete={deleteSession}
    />

    <!-- ===== 主对话区 ===== -->
    <section class="chat-main">
        {#if showSettings}
            <SettingsSheet
                bind:systemPrompt
                bind:temperature
                bind:maxTokens
                disabled={isSending}
            />
        {/if}

        {#if modelsError}
            <div class="floating-warn">⚡ {modelsError}</div>
        {/if}
        {#if error}
            <div class="floating-warn">
                ⚠️ {error}<button onclick={() => (error = "")}>✕</button>
            </div>
        {/if}

        <!-- 消息流 -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
            class="messages"
            bind:this={messagesEl}
            onclick={handleActionClick}
            role="presentation"
        >
            {#if messages.length === 0}
                <Welcome
                    sourceLabel={currentSourceLabel}
                    modelName={currentModelName}
                    onPick={(text) => (input = text)}
                />
            {/if}

            {#each messages as msg, i (i)}
                <ChatMessageView
                    msg={msg}
                    isLast={i === messages.length - 1}
                    isSending={isSending}
                    workspacePath={workspaceStore.activePath}
                    onCopy={copyMessage}
                    onToggleFavorite={toggleFavorite}
                />
            {/each}
        </div>

        <!-- 输入区（含 @ 提及弹窗） -->
        <div class="dock">
            <Composer
                bind:value={input}
                disabled={isSending}
                {canSend}
                onSend={handleSend}
                onStop={handleStop}
            />
        </div>
    </section>
</div>

<style>
    /* 仅在 chat 页面让 content 全屏，不影响其他页面 */
    :global(body .content:has(.chat-root)) {
        padding: 0 !important;
        overflow: hidden !important;
    }

    .chat-root {
        --chat-gap: 16px;
        --dock-max: 760px;
        --msg-max: 760px;

        height: 100%;
        overflow: hidden;
        display: flex;
        background:
            radial-gradient(
                ellipse 80% 60% at 50% -10%,
                var(--accent-light),
                transparent 70%
            ),
            var(--bg-app);
    }

    /* ===== 主区 ===== */
    .chat-main {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        position: relative;
        height: 100%;
        min-height: 0;
    }

    /* 警告条 */
    .floating-warn {
        margin: 10px 24px 0;
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 9px 14px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 10px;
        color: var(--error-text);
        font-size: 13px;
    }

    .floating-warn button {
        margin-left: auto;
        background: transparent;
        border: none;
        color: var(--error-muted);
        cursor: pointer;
        font-size: 14px;
        padding: 0 4px;
    }

    /* ===== 消息流 ===== */
    .messages {
        flex: 1;
        overflow-y: auto;
        padding: 24px 24px 8px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        scroll-behavior: smooth;
    }

    /* ===== 输入停靠 ===== */
    .dock {
        padding: 12px 24px 20px;
        flex-shrink: 0;
    }

    /* ===== Action 链接样式 ===== */
    :global(a[href^="action://"]) {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 5px 12px;
        background: var(--accent-bg);
        border: 1px solid rgba(255, 255, 255, 0.2);
        border-radius: 8px;
        color: var(--accent);
        cursor: pointer;
        transition: all 0.15s;
        font-size: 13px;
        font-weight: 500;
        text-decoration: none;
    }

    :global(a[href^="action://"]:hover) {
        background: var(--accent);
        color: white;
        border-color: var(--accent);
        transform: translateY(-1px);
        box-shadow: 0 4px 12px var(--accent-shadow);
    }

    :global(.action-done) {
        background: rgba(74, 222, 128, 0.15);
        color: #4ade80;
        border-color: rgba(74, 222, 128, 0.3);
        cursor: default;
        pointer-events: none;
    }

    :global(.action-error) {
        background: var(--error-bg);
        color: var(--error-text);
        border-color: var(--error-border);
        cursor: default;
        pointer-events: none;
    }

    :global(.action-executed) {
        background: rgba(255, 255, 255, 0.1);
        color: var(--text-secondary);
        cursor: wait;
        opacity: 0.8;
    }

    /* 响应式 */
    @media (max-width: 640px) {
        .messages {
            padding: 16px 16px 4px;
        }

        .dock {
            padding: 8px 16px 16px;
        }
    }
</style>
