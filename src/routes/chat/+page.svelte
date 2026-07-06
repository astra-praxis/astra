<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy, tick } from "svelte";
    import MarkdownMessage from "./MarkdownMessage.svelte";
    import { workspaceStore, type ProjectItem } from "$lib/workspace.svelte";

    // 标题栏 slot id（layout 里定义）
    const TITLEBAR_SLOT_ID = "titlebar-slot";

    // ===== 类型 =====
    interface RunningModelInfo {
        name: string;
        port: number;
        status: string;
        pid: number | null;
    }
    interface ProviderConfig {
        id: string;
        name: string;
        base_url: string;
        api_key: string;
        active_model: string | null;
    }
    interface ModelInfo {
        id: string;
        owned_by: string | null;
    }
    interface ChatMessage {
        role: "user" | "assistant";
        content: string;
        reasoning?: string;
        timestamp: number;
        error?: boolean;
        favorite?: boolean;
        showReasoning?: boolean;
    }
    type ChatSource =
        | { type: "model"; port: number; model_name: string }
        | { type: "provider"; provider_id: string; model: string | null };

    interface ChatSession {
        id: string;
        title: string;
        source: ChatSource;
        messages: Array<{
            role: string;
            content: string;
            reasoning?: string;
            timestamp?: number;
            error?: boolean;
        }>;
        created_at: number;
        updated_at: number;
    }

    // ===== 布局 =====
    let sidebarOpen = $state(false);

    // ===== @ 提及功能 =====
    let textareaEl: HTMLTextAreaElement | null = $state(null);
    let mentionActive = $state(false);
    let mentionQuery = $state("");
    let mentionStartIndex = $state(0); // @ 符号的起始位置
    let mentionSelectedIndex = $state(0); // 当前选中的项目索引（键盘导航）

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
    let toolbarEl: HTMLElement | null = $state(null);

    let sourceType = $state<"provider" | "model">("provider");
    let selectedProviderId = $state<string | null>(null);
    let selectedModelPort = $state<number | null>(null);
    let overrideModelName = $state<string | null>(null);

    // ===== 参数 =====
    let showSettings = $state(false);
    let systemPrompt = $state("你是一个有用的助手，请简洁准确地回答用户问题。");
    let temperature = $state(0.7);
    let maxTokens = $state(4000);

    // ===== 工作空间增强系统提示 =====
    function buildWorkspaceContext(): string {
        if (!workspaceStore.projects.length) return "";
        const projList = workspaceStore.projects
            .slice(0, 20)
            .map((p) => `- ${p.name} (路径: ${p.path})`)
            .join("\n");
        return `

            当前工作空间信息：
            - 名称: ${workspaceStore.activeName || "未设置"}
            - 路径: ${workspaceStore.activePath || "未设置"}
            项目列表：
            ${projList}

            当用户想要执行操作时，你可以使用特殊链接来触发命令。格式如下：
            [按钮文案](action://命令类型?参数名=参数值)

            除了 action 链接，你也可以直接输出 shell 命令代码块，
            用户可以通过代码块上的「执行」按钮一键运行。
            执行时会弹窗确认，用户允许后才会执行，请放心推荐。

            可用命令：
            - open_project: 打开项目，参数为 path（项目完整路径）

            例如：
            1. 使用 action 链接打开项目：
            好的，我来为你打开 astra 项目：[🚀 打开项目](action://open_project?path=/Users/workplace/astra)

            2. 使用可执行代码块运行命令：
            运行构建：
            \`\`\`bash
            cd /Users/workplace/astra && pnpm build
            \`\`\`

            注意：
            1. 链接文案应该简洁明了，包含 emoji 更醒目
            2. path 参数必须是完整的项目路径，从上面的项目列表中获取
            3. 一次只生成一个 action 链接，避免生成多个以免用户困惑
            `;
    }

    // ===== 解析用户消息中的 @ 提及 =====
    function parseMentions(text: string): ProjectItem[] {
        // 支持中文、字母、数字、点、连字符
        const mentionRegex = /@([\w\u4e00-\u9fff][\w\u4e00-\u9fff.-]*)/g;
        const mentions: ProjectItem[] = [];
        let match;
        while ((match = mentionRegex.exec(text)) !== null) {
            const name = match[1];
            const project = workspaceStore.projects.find(
                (p) => p.name.toLowerCase() === name.toLowerCase(),
            );
            if (project && !mentions.includes(project)) {
                mentions.push(project);
            }
        }
        return mentions;
    }

    let sessions = $state<ChatSession[]>([]);

    let searchQuery = $state("");

    const filteredSessions = $derived.by(() => {
        if (!searchQuery.trim()) return sessions;
        const q = searchQuery.toLowerCase();
        return sessions.filter((s) => {
            if (s.title.toLowerCase().includes(q)) return true;
            if (s.messages && s.messages.length > 0) {
                for (const m of s.messages) {
                    if (m.content.toLowerCase().includes(q)) return true;
                }
            }
            return false;
        });
    });

    let unlisteners: Array<() => void> = [];

    // ===== 生命周期 =====
    onMount(async () => {
        await Promise.all([
            loadSessions(),
            loadRunningModels(),
            loadProviders(),
            workspaceStore.loadFromSettings(),
        ]);

        if (providers.length > 0) {
            selectedProviderId = providers[0].id;
            handleFetchModels();
        }

        if (runningModels.length > 0) {
            sourceType = "model";
            selectedModelPort = runningModels[0].port;
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
        // 清理 portal 残留
        const target = document.getElementById(TITLEBAR_SLOT_ID);
        if (target) {
            target.innerHTML = "";
            target.classList.remove("has-toolbar");
        }
        unlisteners.forEach((fn) => fn());
    });

    // ===== 派生 =====
    const selectedProvider = $derived(
        providers.find((p) => p.id === selectedProviderId) ?? null,
    );
    const selectedModel = $derived(
        runningModels.find((m) => m.port === selectedModelPort) ?? null,
    );

    const currentModelName = $derived.by(() => {
        if (sourceType === "model") return selectedModel?.name ?? "local";
        return overrideModelName ?? selectedProvider?.active_model ?? null;
    });

    const currentSourceLabel = $derived.by(() => {
        if (sourceType === "model") return selectedModel?.name ?? "本地模型";
        return selectedProvider?.name ?? "未选择";
    });

    const canSend = $derived.by(() => {
        if (isSending || input.trim().length === 0) return false;
        if (sourceType === "model") return !!selectedModelPort;
        return !!selectedProviderId && !!currentModelName;
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
        if (sourceType === t || isSending) return;
        sourceType = t;
        error = "";
        if (t === "model" && runningModels.length > 0 && !selectedModelPort) {
            selectedModelPort = runningModels[0].port;
        }
        if (t === "provider" && providers.length > 0 && !selectedProviderId) {
            selectedProviderId = providers[0].id;
            handleFetchModels();
        }
    }

    function onSelectProviderChange() {
        overrideModelName = null;
        providerModels = [];
        if (selectedProviderId) handleFetchModels();
    }

    function newChat() {
        if (isSending) return;
        currentSessionId = null;
        messages = [];
        error = "";
        input = "";
        overrideModelName = null;
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
        }));

        if (s.source.type === "provider") {
            sourceType = "provider";
            const sid = s.source.provider_id;
            selectedProviderId = sid;
            const provider = providers.find((p) => p.id === sid);
            overrideModelName =
                s.source.model ?? provider?.active_model ?? null;
            if (provider) handleFetchModels();
        } else if (s.source.type === "model") {
            sourceType = "model";
            selectedModelPort = s.source.port;
        }

        sidebarOpen = false;
        await scrollToBottom();
    }

    function deleteSession(id: string, e: Event) {
        e.stopPropagation();
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
        if (sourceType === "model") {
            return {
                type: "model",
                port: selectedModelPort!,
                model_name: selectedModel?.name ?? "local",
            };
        }
        return {
            type: "provider",
            provider_id: selectedProviderId!,
            model: overrideModelName,
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

    async function handleSend() {
        const text = input.trim();
        if (!text || isSending) return;

        if (sourceType === "model" && !selectedModelPort) {
            error = "请先选择一个运行中的本地模型";
            return;
        }
        if (sourceType === "provider") {
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
        if (sourceType === "model") {
            source = buildSource();
        } else {
            source = {
                type: "provider",
                provider_id: selectedProviderId!,
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

    function handleKeydown(e: KeyboardEvent) {
        // 当 @ 提及弹出时，处理键盘导航
        if (mentionActive) {
            const filteredProjects = getFilteredProjects();

            // 如果列表为空，不处理方向键
            if (
                filteredProjects.length === 0 &&
                (e.key === "ArrowDown" ||
                    e.key === "ArrowUp" ||
                    e.key === "Enter" ||
                    e.key === "Tab")
            ) {
                e.preventDefault();
                return;
            }

            if (e.key === "ArrowDown") {
                e.preventDefault();
                // 边界检查：确保 selectedIndex 不超过列表长度
                mentionSelectedIndex = Math.min(
                    mentionSelectedIndex + 1,
                    filteredProjects.length - 1,
                );
                scrollToSelected();
                return;
            }
            if (e.key === "ArrowUp") {
                e.preventDefault();
                mentionSelectedIndex = Math.max(mentionSelectedIndex - 1, 0);
                scrollToSelected();
                return;
            }
            if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                if (filteredProjects.length > 0 && mentionSelectedIndex >= 0) {
                    selectMentionProject(
                        filteredProjects[mentionSelectedIndex],
                    );
                }
                return;
            }
            if (e.key === "Escape") {
                e.preventDefault();
                closeMention();
                return;
            }
        }

        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    }

    /** 滚动到选中的项目 */
    function scrollToSelected() {
        if (!mentionListEl) return;
        tick().then(() => {
            const selectedEl = mentionListEl?.querySelector(
                ".mention-item.selected",
            );
            if (selectedEl) {
                selectedEl.scrollIntoView({
                    block: "nearest",
                    behavior: "smooth",
                });
            }
        });
    }

    async function scrollToBottom() {
        await tick();
        if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    // ===== @ 提及功能 =====
    let mentionListEl: HTMLElement | null = $state(null);

    function getFilteredProjects(): ProjectItem[] {
        const q = mentionQuery.toLowerCase();
        if (!q) return workspaceStore.projects.slice(0, 10); // 无搜索词时显示前10个
        // 有搜索词时搜索全部项目，最多显示20个匹配结果
        return workspaceStore.projects
            .filter((p) => p.name.toLowerCase().includes(q))
            .slice(0, 20);
    }

    /** 监听 textarea 输入，检测 @ 提及 */
    function handleTextareaInput() {
        if (!textareaEl) return;

        const val = textareaEl.value;
        const cursorPos = textareaEl.selectionStart;

        if (!val || cursorPos === 0) {
            mentionActive = false;
            return;
        }

        // 从光标位置向前查找最近的 @ 符号
        const beforeCursor = val.substring(0, cursorPos);
        // 匹配模式：@ 后面可以跟字母/数字/中文/点/连字符（停止在空格、换行等处）
        // \u4e00-\u9fff 匹配常用中文字符，\w 匹配字母数字下划线
        const atMatch = beforeCursor.match(/(?:^|\s)@([\w\u4e00-\u9fff.-]*)$/);

        if (atMatch) {
            mentionActive = true;
            mentionStartIndex =
                cursorPos -
                atMatch[0].length +
                (atMatch[0].startsWith("@") ? 0 : 1);
            mentionQuery = atMatch[1]; // @ 后面的搜索词
            mentionSelectedIndex = 0;
        } else {
            mentionActive = false;
        }
    }

    /** 选择项目，插入到输入框 */
    function selectMentionProject(project: ProjectItem) {
        if (!textareaEl) return;

        const val = textareaEl.value;
        const cursorPos = textareaEl.selectionStart;

        // 替换 @xxx 为 @项目名 并在后面加空格
        const beforeMention = val.substring(0, mentionStartIndex);
        const afterCursor = val.substring(cursorPos);
        const insertion = `@${project.name} `;
        const newValue = beforeMention + insertion + afterCursor;

        input = newValue;
        mentionActive = false;

        // 更新光标位置到插入文本之后
        tick().then(() => {
            if (textareaEl) {
                textareaEl.focus();
                const newCursorPos = beforeMention.length + insertion.length;
                textareaEl.setSelectionRange(newCursorPos, newCursorPos);
            }
        });
    }

    /** 关闭提及弹出 */
    function closeMention() {
        mentionActive = false;
    }

    /** 鼠标点击选择项目 */
    function clickMentionProject(project: ProjectItem, index: number) {
        mentionSelectedIndex = index;
        selectMentionProject(project);
    }

    // ===== Action 链接点击处理 =====
    async function handleActionClick(e: MouseEvent) {
        const target = e.target as HTMLElement;
        const link = target.closest('a[href^="action://"]');
        if (!link) return;

        // 防止重复点击
        if (
            link.classList.contains("action-executed") ||
            link.classList.contains("action-done") ||
            link.classList.contains("action-error")
        ) {
            return;
        }

        e.preventDefault();
        const href = link.getAttribute("href") || "";
        const url = new URL(href);
        const actionType = url.hostname; // action://open_project?path=xxx -> hostname is 'open_project'
        const params = Object.fromEntries(url.searchParams);

        if (actionType === "open_project" && params.path) {
            if (!workspaceStore.editor.command) {
                link.textContent = "⚠ 请先在设置页配置编辑器";
                link.classList.add("action-error");
                return;
            }

            // 标记为正在执行
            link.textContent = "⏳ 正在打开...";
            link.classList.add("action-executed");

            try {
                await invoke("open_in_editor", {
                    path: params.path,
                    editorCommand: workspaceStore.editor.command,
                });
                link.textContent = `✓ 已用 ${workspaceStore.editor.name} 打开`;
                link.classList.add("action-done");
            } catch (err) {
                link.textContent = `❌ ${String(err).substring(0, 50)}`;
                link.classList.add("action-error");
            }
        } else if (actionType === "run_command" && params.cmd) {
            // 通过 action 链接执行命令
            link.textContent = "⏳ 正在执行...";
            link.classList.add("action-executed");
            try {
                await invoke("run_command", {
                    command: params.cmd,
                    cwd: params.cwd || workspaceStore.activePath || undefined,
                    timeoutSecs: 300,
                });
                link.textContent = "✓ 执行完成";
                link.classList.add("action-done");
            } catch (err) {
                link.textContent = `❌ ${String(err).substring(0, 50)}`;
                link.classList.add("action-error");
            }
        }
    }

    function formatTime(ts: number): string {
        if (!ts) return "";
        const d = new Date(ts * 1000);
        const now = new Date();
        const isToday = d.toDateString() === now.toDateString();
        const time = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
        if (isToday) return time;
        return `${d.getMonth() + 1}/${d.getDate()} ${time}`;
    }

    function sourceIcon(s: ChatSource): string {
        if (s.type === "model") return "🖥";
        return "⚡";
    }

    function sourceShortLabel(s: ChatSource): string {
        if (s.type === "model") return `port ${s.port}`;
        const p = providers.find((x) => x.id === s.provider_id);
        return p?.name ?? "provider";
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

    // 工具栏 DOM 传送到全局标题栏
    $effect(() => {
        if (!toolbarEl) return;
        const target = document.getElementById(TITLEBAR_SLOT_ID);
        if (target && toolbarEl.parentElement !== target) {
            target.appendChild(toolbarEl);
            target.classList.add("has-toolbar");
        }
    });
</script>

<div class="chat-root" class:with-sidebar={sidebarOpen}>
    <!-- ===== 历史抽屉（右侧滑入） ===== -->
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    {#if sidebarOpen}
        <div
            class="drawer-veil"
            onclick={() => (sidebarOpen = false)}
            role="presentation"
        ></div>
    {/if}
    <aside class="history-drawer" class:open={sidebarOpen}>
        <div class="hd-top">
            <span class="hd-title">对话历史</span>
            <button
                class="hd-close"
                onclick={() => (sidebarOpen = false)}
                aria-label="关闭"
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
                    ><line x1="18" y1="6" x2="6" y2="18" /><line
                        x1="6"
                        y1="6"
                        x2="18"
                        y2="18"
                    /></svg
                >
            </button>
        </div>

        <button class="hd-new" onclick={newChat} disabled={isSending}>
            <svg
                width="15"
                height="15"
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
            新建对话
        </button>

        <div class="hd-search">
            <svg
                width="13"
                height="13"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><circle cx="11" cy="11" r="8" /><line
                    x1="21"
                    y1="21"
                    x2="16.65"
                    y2="16.65"
                /></svg
            >
            <input
                type="text"
                placeholder="搜索对话历史…"
                bind:value={searchQuery}
            />
            {#if searchQuery}
                <button
                    class="hd-search-clear"
                    onclick={() => (searchQuery = "")}
                    aria-label="清除搜索">✕</button
                >
            {/if}
        </div>

        <div class="hd-list">
            {#if filteredSessions.length === 0}
                <div class="hd-empty">
                    <span class="hd-empty-icon">✦</span>
                    <p>{searchQuery ? "没有匹配的对话" : "暂无历史记录"}</p>
                </div>
            {:else}
                {#each filteredSessions as s (s.id)}
                    <div
                        class="hd-item"
                        class:active={s.id === currentSessionId}
                        onclick={() => selectSession(s)}
                        role="button"
                        tabindex="0"
                        onkeydown={(e) => e.key === "Enter" && selectSession(s)}
                    >
                        <span class="hd-item-icon">{sourceIcon(s.source)}</span>
                        <div class="hd-item-text">
                            <div class="hd-item-title">{s.title}</div>
                            <div class="hd-item-meta">
                                <span class="hd-item-source"
                                    >{sourceShortLabel(s.source)}</span
                                >
                                <span class="hd-item-dot">·</span>
                                <span class="hd-item-time"
                                    >{formatTime(s.updated_at)}</span
                                >
                            </div>
                        </div>
                        <button
                            class="hd-item-del"
                            onclick={(e) => deleteSession(s.id, e)}
                            aria-label="删除"
                            title="删除"
                        >
                            <svg
                                width="12"
                                height="12"
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
                {/each}
            {/if}
        </div>
    </aside>

    <!-- ===== 主对话区 ===== -->
    <section class="chat-main">
        <!-- 工具栏（会被挂载到全局顶部标题栏） -->
        <div class="toolbar-row" bind:this={toolbarEl}>
            <!-- 左侧：模式切换 + 选择器 -->
            <div class="tb-left">
                <div class="seg-control">
                    <button
                        class:active={sourceType === "provider"}
                        onclick={() => onSwitchType("provider")}
                        title="API 提供者"
                    >
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><path d="M22 12h-4l-3 9L9 3l-3 9H2" /></svg
                        >
                    </button>
                    <button
                        class:active={sourceType === "model"}
                        onclick={() => onSwitchType("model")}
                        title="本地模型"
                    >
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><rect
                                x="2"
                                y="3"
                                width="20"
                                height="14"
                                rx="2"
                                ry="2"
                            /><line x1="8" y1="21" x2="16" y2="21" /><line
                                x1="12"
                                y1="17"
                                x2="12"
                                y2="21"
                            /></svg
                        >
                    </button>
                </div>

                {#if sourceType === "provider"}
                    <select
                        class="picker"
                        bind:value={selectedProviderId}
                        onchange={onSelectProviderChange}
                        disabled={isSending}
                        title="选择 Provider"
                    >
                        {#if providers.length === 0}
                            <option value={null}>尚未配置</option>
                        {/if}
                        {#each providers as p (p.id)}
                            <option value={p.id}>{p.name}</option>
                        {/each}
                    </select>

                    <div class="picker-group">
                        <select
                            class="picker mono"
                            value={currentModelName}
                            onchange={(e) =>
                                (overrideModelName =
                                    (e.target as HTMLSelectElement).value ||
                                    null)}
                            disabled={isSending || modelsLoading}
                            title="选择模型"
                        >
                            {#if !currentModelName}
                                <option value="">未选模型</option>
                            {/if}
                            {#if modelsLoading}
                                <option value="">加载中…</option>
                            {/if}
                            {#if currentModelName && providerModels.findIndex((m) => m.id === currentModelName) < 0}
                                <option value={currentModelName}
                                    >{currentModelName}</option
                                >
                            {/if}
                            {#each providerModels as m (m.id)}
                                <option value={m.id}>{m.id}</option>
                            {/each}
                        </select>
                        <button
                            class="icon-btn sm"
                            onclick={handleFetchModels}
                            disabled={isSending || modelsLoading}
                            title="刷新模型列表"
                            aria-label="刷新模型"
                        >
                            <svg
                                width="13"
                                height="13"
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
                        </button>
                    </div>
                {:else}
                    <select
                        class="picker mono"
                        bind:value={selectedModelPort}
                        disabled={isSending}
                        title="选择本地模型"
                    >
                        {#if runningModels.length === 0}
                            <option value={null}>无运行模型</option>
                        {/if}
                        {#each runningModels as m (m.port)}
                            <option value={m.port}>{m.name} :{m.port}</option>
                        {/each}
                    </select>
                    <button
                        class="icon-btn sm"
                        onclick={loadRunningModels}
                        disabled={isSending}
                        title="刷新"
                        aria-label="刷新"
                    >
                        <svg
                            width="13"
                            height="13"
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
                    </button>
                {/if}
            </div>

            <!-- 右侧：功能按钮 -->
            <div class="tb-right">
                <button
                    class="icon-btn"
                    class:active={showSettings}
                    onclick={() => (showSettings = !showSettings)}
                    title="参数设置"
                    aria-label="参数设置"
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
                        ><circle cx="12" cy="12" r="3" /><path
                            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"
                        /></svg
                    >
                </button>
                <button
                    class="icon-btn"
                    onclick={handleClear}
                    disabled={isSending || messages.length === 0}
                    title="清空对话"
                    aria-label="清空对话"
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
                        ><polyline points="3 6 5 6 21 6" /><path
                            d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
                        /></svg
                    >
                </button>
                <button
                    class="icon-btn"
                    onclick={() => (sidebarOpen = !sidebarOpen)}
                    title="历史记录"
                    aria-label="历史记录"
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
                        ><line x1="3" y1="6" x2="21" y2="6" /><line
                            x1="3"
                            y1="12"
                            x2="21"
                            y2="12"
                        /><line x1="3" y1="18" x2="21" y2="18" /></svg
                    >
                    {#if sessions.length > 0}
                        <span class="tb-badge">{sessions.length}</span>
                    {/if}
                </button>
            </div>
        </div>

        {#if showSettings}
            <div class="settings-sheet">
                <div class="ss-field">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>系统提示</label>
                    <textarea
                        bind:value={systemPrompt}
                        rows="2"
                        disabled={isSending}
                        placeholder="为对话设定角色和约束"></textarea>
                </div>
                <div class="ss-row">
                    <div class="ss-field-inline">
                        <!-- svelte-ignore a11y_label_has_associated_control -->
                        <label>温度</label>
                        <input
                            type="number"
                            step="0.1"
                            min="0"
                            max="2"
                            bind:value={temperature}
                            disabled={isSending}
                        />
                    </div>
                    <div class="ss-field-inline">
                        <!-- svelte-ignore a11y_label_has_associated_control -->
                        <label>最大 Token</label>
                        <input
                            type="number"
                            step="100"
                            min="1"
                            bind:value={maxTokens}
                            disabled={isSending}
                        />
                    </div>
                </div>
            </div>
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
                <!-- 空状态 - 居中欢迎 -->
                <div class="welcome">
                    <div class="welcome-orb"></div>
                    <h1 class="welcome-greet">有什么可以帮你？</h1>
                    <p class="welcome-sub">
                        连接到 <strong>{currentSourceLabel}</strong>
                        {#if currentModelName}<span class="welcome-model"
                                >{currentModelName}</span
                            >{/if}
                        开始对话
                    </p>
                    <div class="welcome-examples">
                        <button
                            onclick={() =>
                                (input = "帮我解释一下什么是 Transformer 架构")}
                        >
                            <span>💡</span> 解释 Transformer 架构
                        </button>
                        <button
                            onclick={() =>
                                (input = "用 Python 实现一个快速排序")}
                        >
                            <span>⚡</span> Python 快速排序
                        </button>
                        <button
                            onclick={() =>
                                (input =
                                    "帮我把这段话翻译成英文：今天天气很好")}
                        >
                            <span>🌐</span> 中英翻译
                        </button>
                        <button
                            onclick={() =>
                                (input = "给我讲一个关于程序员的冷笑话")}
                        >
                            <span>😄</span> 来个冷笑话
                        </button>
                    </div>
                </div>
            {/if}

            {#each messages as msg, i (i)}
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
                                    onclick={() =>
                                        (msg.showReasoning =
                                            !msg.showReasoning)}
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
                                        ><polyline
                                            points="6 9 12 15 18 9"
                                        /></svg
                                    >
                                </button>
                                {#if msg.showReasoning}
                                    <pre
                                        class="reasoning-text">{msg.reasoning}</pre>
                                {/if}
                            </div>
                        {/if}

                        <!-- 主回复内容 -->
                        {#if !msg.content && isSending && i === messages.length - 1}
                            <div class="typing">
                                <span></span><span></span><span></span>
                            </div>
                        {:else if msg.role === "assistant"}
                            <div class="msg-body">
                                <MarkdownMessage
                                    content={msg.content}
                                    workspacePath={workspaceStore.activePath}
                                />
                            </div>
                        {:else}
                            <pre class="msg-text">{msg.content}</pre>
                        {/if}

                        <!-- 底部操作按钮（仅 AI 输出） -->
                        {#if msg.role === "assistant" && msg.content && !(isSending && i === messages.length - 1)}
                            <div class="msg-footer">
                                <button
                                    class="mf-btn"
                                    onclick={() => copyMessage(msg)}
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
                                    onclick={() => toggleFavorite(msg)}
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
            {/each}
        </div>

        <!-- 输入区 -->
        <div class="dock">
            <!-- @ 提及弹出 -->
            {#if mentionActive && workspaceStore.projects.length > 0}
                <div class="mention-popup">
                    <div class="mention-header">
                        <svg
                            width="12"
                            height="12"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><path
                                d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                            /></svg
                        >
                        <span>选择项目</span>
                        {#if mentionQuery}
                            <span class="mention-query"
                                >搜索: {mentionQuery}</span
                            >
                        {/if}
                    </div>
                    <div class="mention-list" bind:this={mentionListEl}>
                        {#if getFilteredProjects().length === 0}
                            <div class="mention-empty">没有匹配的项目</div>
                        {:else}
                            {#each getFilteredProjects() as project, i (project.path)}
                                <!-- svelte-ignore a11y_click_events_have_key_events -->
                                <div
                                    class="mention-item"
                                    class:selected={i === mentionSelectedIndex}
                                    onclick={() =>
                                        clickMentionProject(project, i)}
                                    role="button"
                                    tabindex="-1"
                                >
                                    <span class="mention-item-icon">📁</span>
                                    <div class="mention-item-content">
                                        <span class="mention-item-name"
                                            >{project.name}</span
                                        >
                                        <span
                                            class="mention-item-path"
                                            title={project.path}
                                            >{project.path}</span
                                        >
                                    </div>
                                </div>
                            {/each}
                        {/if}
                    </div>
                    <div class="mention-hint">
                        <kbd>↑↓</kbd> 选择 · <kbd>Enter</kbd> 确认 ·
                        <kbd>Esc</kbd> 关闭
                    </div>
                </div>
            {/if}

            <div class="composer" class:sending={isSending}>
                <textarea
                    bind:this={textareaEl}
                    bind:value={input}
                    onkeydown={handleKeydown}
                    oninput={handleTextareaInput}
                    disabled={isSending}
                    rows="2"
                    placeholder={isSending
                        ? "生成中…"
                        : "问点什么，输入 @ 可提及项目"}></textarea>
                <div class="composer-side">
                    <div class="composer-hint">
                        <kbd>Enter</kbd> 发送 · <kbd>Shift</kbd>+<kbd>Enter</kbd
                        > 换行
                    </div>
                    {#if isSending}
                        <button class="stop-btn" onclick={handleStop}>
                            <span class="stop-square"></span>
                            停止
                        </button>
                    {:else}
                        <button
                            class="send-btn"
                            onclick={handleSend}
                            disabled={!canSend}
                            aria-label="发送"
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
                                ><line
                                    x1="5"
                                    y1="12"
                                    x2="19"
                                    y2="12"
                                /><polyline points="12 5 19 12 12 19" /></svg
                            >
                        </button>
                    {/if}
                </div>
            </div>
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

    /* ===== 历史抽屉 ===== */
    .drawer-veil {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.45);
        z-index: 40;
        animation: fadeIn 0.2s ease;
    }

    .history-drawer {
        position: fixed;
        right: 0;
        top: 0;
        bottom: 0;
        z-index: 41;
        width: 280px;
        max-width: 86vw;
        background: var(--bg-card);
        border-left: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        transform: translateX(100%);
        transition: transform 0.28s cubic-bezier(0.32, 0.72, 0, 1);
        box-shadow: -4px 0 24px rgba(0, 0, 0, 0.08);
    }

    .history-drawer.open {
        transform: translateX(0);
        box-shadow: -4px 0 32px rgba(0, 0, 0, 0.15);
    }

    .hd-search {
        display: flex;
        align-items: center;
        gap: 6px;
        margin: 0 12px 8px;
        padding: 6px 10px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 8px;
        transition: border-color 0.15s;
    }

    .hd-search:focus-within {
        border-color: var(--accent);
    }

    .hd-search svg {
        flex-shrink: 0;
        color: var(--text-muted);
    }

    .hd-search input {
        flex: 1;
        border: none;
        background: transparent;
        color: var(--text-primary);
        font-size: 13px;
        font-family: inherit;
        outline: none;
        min-width: 0;
    }

    .hd-search input::placeholder {
        color: var(--text-placeholder);
    }

    .hd-search-clear {
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        font-size: 12px;
        padding: 0 2px;
    }

    .hd-search-clear:hover {
        color: var(--text-primary);
    }

    .hd-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 16px 16px 12px;
    }

    .hd-title {
        font-size: 13px;
        font-weight: 700;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--text-muted);
    }

    .hd-close {
        width: 26px;
        height: 26px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--bg-subtle);
        border: none;
        border-radius: 6px;
        color: var(--text-muted);
        cursor: pointer;
        transition: all 0.15s;
    }

    .hd-close:hover {
        color: var(--accent);
        background: var(--accent-bg);
    }

    .hd-new {
        margin: 0 12px 8px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        padding: 9px 12px;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        font-size: 13px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s;
    }

    .hd-new:hover:not(:disabled) {
        background: var(--accent-hover);
        box-shadow: 0 6px 16px var(--accent-shadow);
    }

    .hd-new:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .hd-list {
        flex: 1;
        overflow-y: auto;
        padding: 4px 8px 12px;
    }

    .hd-empty {
        padding: 60px 16px;
        text-align: center;
        color: var(--text-muted);
    }

    .hd-empty-icon {
        font-size: 28px;
        color: var(--accent);
        display: block;
        margin-bottom: 8px;
    }

    .hd-item {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        padding: 9px 8px;
        border-radius: 8px;
        cursor: pointer;
        transition: background 0.15s;
        position: relative;
    }

    .hd-item:hover {
        background: var(--bg-subtle);
    }

    .hd-item.active {
        background: var(--accent-bg);
    }

    .hd-item.active::before {
        content: "";
        position: absolute;
        right: 0;
        top: 50%;
        transform: translateY(-50%);
        width: 3px;
        height: 60%;
        background: var(--accent);
        border-radius: 3px 0 0 3px;
    }

    .hd-item-icon {
        font-size: 13px;
        margin-top: 1px;
        opacity: 0.8;
    }

    .hd-item-text {
        flex: 1;
        min-width: 0;
    }

    .hd-item-title {
        font-size: 13px;
        font-weight: 500;
        color: var(--text-primary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        margin-bottom: 3px;
    }

    .hd-item-meta {
        display: flex;
        gap: 4px;
        font-size: 11px;
        color: var(--text-muted);
        align-items: center;
    }

    .hd-item-source {
        font-family: ui-monospace, monospace;
        max-width: 130px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .hd-item-del {
        width: 22px;
        height: 22px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: transparent;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        border-radius: 5px;
        opacity: 0;
        transition: all 0.15s;
        flex-shrink: 0;
        margin-top: -2px;
    }

    .hd-item:hover .hd-item-del,
    .hd-item.active .hd-item-del {
        opacity: 0.7;
    }

    .hd-item-del:hover {
        opacity: 1 !important;
        color: var(--error-text);
        background: var(--error-bg);
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

    /* ===== 工具栏（单行） ===== */
    .toolbar-row {
        width: 100%;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        padding: 0 20px;
        position: relative;
        flex-shrink: 0;
        white-space: nowrap;
    }

    .tb-left {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-shrink: 0;
        min-width: 0;
    }

    .tb-right {
        display: flex;
        align-items: center;
        gap: 2px;
        flex-shrink: 0;
    }

    .icon-btn {
        position: relative;
        width: 28px;
        height: 28px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: transparent;
        border: none;
        border-radius: 6px;
        color: var(--sidebar-text);
        cursor: pointer;
        transition: all 0.15s;
        flex-shrink: 0;
    }

    .icon-btn:hover:not(:disabled) {
        background: var(--sidebar-hover-bg);
        color: var(--sidebar-text-hover);
    }

    .icon-btn.active {
        background: var(--accent-bg);
        color: var(--accent);
    }

    .icon-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }

    .icon-btn.sm {
        width: 26px;
        height: 26px;
    }

    .tb-badge {
        position: absolute;
        top: 3px;
        right: 3px;
        min-width: 16px;
        height: 16px;
        padding: 0 4px;
        background: var(--accent);
        color: white;
        font-size: 10px;
        font-weight: 700;
        border-radius: 8px;
        display: flex;
        align-items: center;
        justify-content: center;
        line-height: 1;
    }

    .seg-control {
        display: inline-flex;
        background: rgba(255, 255, 255, 0.08);
        border-radius: 7px;
        padding: 2px;
        gap: 2px;
    }

    .seg-control button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 26px;
        height: 22px;
        background: transparent;
        border: none;
        border-radius: 5px;
        color: var(--sidebar-text);
        cursor: pointer;
        transition: all 0.12s;
    }

    .seg-control button:hover {
        color: var(--sidebar-text-hover);
    }

    .seg-control button.active {
        background: var(--sidebar-active-bg);
        color: var(--sidebar-active-text);
    }

    .picker {
        padding: 3px 8px;
        background: rgba(255, 255, 255, 0.06);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        font-size: 12px;
        color: var(--sidebar-text);
        cursor: pointer;
        outline: none;
        transition: all 0.12s;
        max-width: 160px;
        font-family: inherit;
    }

    .picker:hover {
        color: var(--sidebar-text-hover);
        border-color: rgba(255, 255, 255, 0.2);
    }

    .picker.mono {
        font-family: ui-monospace, monospace;
    }

    .picker:focus {
        color: var(--sidebar-accent);
        border-color: var(--sidebar-accent);
    }

    .picker:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .picker-group {
        display: inline-flex;
        align-items: center;
        gap: 3px;
        min-width: 0;
    }

    .spinning {
        animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }

    /* 设置抽屉 */
    .settings-sheet {
        margin: 0 24px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 14px;
        display: flex;
        flex-direction: column;
        gap: 12px;
        animation: slideDown 0.2s ease;
    }

    @keyframes slideDown {
        from {
            opacity: 0;
            transform: translateY(-6px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    .ss-field {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .ss-field label,
    .ss-field-inline label {
        font-size: 11px;
        font-weight: 600;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }

    .ss-field textarea {
        width: 100%;
        padding: 8px 12px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 13px;
        color: var(--text-primary);
        outline: none;
        font-family: inherit;
        resize: vertical;
        transition: border-color 0.15s;
        box-sizing: border-box;
    }

    .ss-field textarea:focus {
        border-color: var(--accent);
    }

    .ss-row {
        display: flex;
        gap: 14px;
    }

    .ss-field-inline {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .ss-field-inline input {
        padding: 7px 12px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-size: 13px;
        color: var(--text-primary);
        outline: none;
        transition: border-color 0.15s;
        box-sizing: border-box;
    }

    .ss-field-inline input:focus {
        border-color: var(--accent);
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

    /* ===== 空状态 ===== */
    .welcome {
        margin: auto;
        text-align: center;
        padding: 40px 20px;
        max-width: 600px;
    }

    .welcome-orb {
        width: 80px;
        height: 80px;
        border-radius: 50%;
        margin: 0 auto 24px;
        background: var(--accent-gradient);
        position: relative;
        box-shadow: 0 12px 40px var(--accent-shadow);
        animation: orbFloat 4s ease-in-out infinite;
    }

    .welcome-orb::before {
        content: "";
        position: absolute;
        inset: -8px;
        border-radius: 50%;
        background: var(--accent-gradient);
        opacity: 0.25;
        filter: blur(16px);
        animation: orbPulse 4s ease-in-out infinite;
    }

    @keyframes orbFloat {
        0%,
        100% {
            transform: translateY(0);
        }
        50% {
            transform: translateY(-8px);
        }
    }

    @keyframes orbPulse {
        0%,
        100% {
            transform: scale(1);
            opacity: 0.25;
        }
        50% {
            transform: scale(1.15);
            opacity: 0.4;
        }
    }

    .welcome-greet {
        font-size: 28px;
        font-weight: 700;
        color: var(--text-primary);
        margin-bottom: 8px;
        letter-spacing: -0.02em;
    }

    .welcome-sub {
        font-size: 14px;
        color: var(--text-muted);
        margin-bottom: 32px;
    }

    .welcome-sub strong {
        color: var(--text-secondary);
        font-weight: 600;
    }

    .welcome-model {
        display: inline-block;
        margin-left: 6px;
        padding: 2px 8px;
        background: var(--accent-bg);
        color: var(--accent);
        border-radius: 6px;
        font-family: ui-monospace, monospace;
        font-size: 12px;
    }

    .welcome-examples {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 10px;
        text-align: left;
    }

    .welcome-examples button {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 16px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        font-size: 13px;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.2s;
        text-align: left;
        font-family: inherit;
    }

    .welcome-examples button:hover {
        border-color: var(--accent);
        color: var(--accent);
        transform: translateY(-2px);
        box-shadow: 0 6px 16px var(--shadow-md);
    }

    .welcome-examples button span {
        font-size: 18px;
    }

    /* ===== 单条消息 ===== */
    .msg {
        max-width: var(--msg-max);
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

    .msg-body {
        margin-top: 4px;
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

    /* ===== 输入停靠 ===== */
    .dock {
        padding: 12px 24px 20px;
        flex-shrink: 0;
    }

    .composer {
        max-width: var(--dock-max);
        margin: 0 auto;
        background: var(--bg-card);
        border: 1.5px solid var(--border);
        border-radius: 18px;
        padding: 12px 14px 8px 16px;
        display: flex;
        flex-direction: column;
        gap: 8px;
        transition: all 0.2s;
        box-shadow: 0 4px 24px var(--shadow-md);
    }

    .composer:focus-within {
        border-color: var(--accent);
        box-shadow:
            0 0 0 4px var(--accent-ring),
            0 6px 28px var(--shadow-md);
    }

    .composer.sending {
        opacity: 0.92;
    }

    .composer textarea {
        width: 100%;
        border: none;
        background: var(--bg-card);
        color: var(--text-primary);
        font-size: 14.5px;
        font-family: inherit;
        line-height: 1.5;
        resize: none;
        outline: none;
        min-height: 24px;
        box-sizing: border-box;
        caret-color: var(--accent);
    }

    .composer textarea::placeholder {
        color: var(--text-placeholder);
        opacity: 1;
    }

    .composer-side {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }

    .composer-hint {
        font-size: 11px;
        color: var(--text-muted);
    }

    .composer-hint kbd {
        display: inline-block;
        padding: 1px 5px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 4px;
        font-size: 10px;
        font-family: ui-monospace, monospace;
        color: var(--text-secondary);
    }

    .send-btn {
        width: 34px;
        height: 34px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--accent);
        border: none;
        border-radius: 10px;
        color: white;
        cursor: pointer;
        transition: all 0.18s;
        flex-shrink: 0;
    }

    .send-btn:hover:not(:disabled) {
        background: var(--accent-hover);
        transform: scale(1.06);
    }

    .send-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
        transform: none;
    }

    .stop-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 7px 14px;
        background: var(--error-bg);
        border: 1px solid var(--error-border);
        border-radius: 10px;
        color: var(--error-text);
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.15s;
        flex-shrink: 0;
    }

    .stop-btn:hover {
        background: var(--error-hover-bg);
    }

    .stop-square {
        width: 10px;
        height: 10px;
        background: var(--error-text);
        border-radius: 2px;
    }

    /* ===== @ 提及弹出框 ===== */
    .mention-popup {
        max-width: var(--dock-max);
        margin: 0 auto 8px;
        background: var(--bg-card);
        border: 1.5px solid var(--border);
        border-radius: 14px;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
        overflow: hidden;
        animation: slideUp 0.2s ease;
    }

    @keyframes slideUp {
        from {
            opacity: 0;
            transform: translateY(8px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    .mention-header {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 8px 14px;
        font-size: 12px;
        font-weight: 600;
        color: var(--text-secondary);
        border-bottom: 1px solid var(--border);
        background: var(--bg-subtle);
    }

    .mention-query {
        color: var(--accent);
        font-weight: 600;
    }

    .mention-list {
        max-height: 220px;
        overflow-y: auto;
        padding: 4px;
    }

    .mention-empty {
        padding: 20px 14px;
        text-align: center;
        color: var(--text-muted);
        font-size: 13px;
    }

    .mention-item {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 8px 10px;
        border-radius: 8px;
        cursor: pointer;
        transition: all 0.12s;
    }

    .mention-item:hover {
        background: var(--bg-subtle);
    }

    .mention-item.selected {
        background: var(--accent-bg);
    }

    .mention-item-icon {
        font-size: 16px;
        flex-shrink: 0;
    }

    .mention-item-content {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .mention-item-name {
        font-size: 13px;
        font-weight: 500;
        color: var(--text-primary);
    }

    .mention-item.selected .mention-item-name {
        color: var(--accent);
    }

    .mention-item-path {
        font-size: 11px;
        color: var(--text-muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .mention-hint {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 8px;
        padding: 6px 10px;
        font-size: 11px;
        color: var(--text-muted);
        border-top: 1px solid var(--border);
        background: var(--bg-subtle);
    }

    .mention-hint kbd {
        padding: 1px 5px;
        background: var(--bg-card);
        border: 1px solid rgba(255, 255, 255, 0.2);
        border-radius: 3px;
        font-size: 10px;
        font-family: ui-monospace, monospace;
        color: var(--text-secondary);
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
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

    @keyframes fadeIn {
        from {
            opacity: 0;
        }
        to {
            opacity: 1;
        }
    }

    /* 响应式 */
    @media (max-width: 640px) {
        .welcome-examples {
            grid-template-columns: 1fr;
        }

        .messages {
            padding: 16px 16px 4px;
        }

        .dock {
            padding: 8px 16px 16px;
        }

        .toolbar-row {
            padding: 8px 16px;
        }

        .composer-hint {
            display: none;
        }
    }
</style>
