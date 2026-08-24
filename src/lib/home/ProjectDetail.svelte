<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { goto } from "$app/navigation";
    import { folderColor } from "./folder-utils";
    import type {
        EditorSetting,
        ProjectDetail as ProjectDetailData,
        SubDetail,
    } from "./types";
    import NodeModulesPanel from "./NodeModulesPanel.svelte";

    interface Props {
        project: ProjectDetailData;
        loading: boolean;
        editor: EditorSetting;
        onBack: () => void;
        onOpenEditor: (path: string) => void;
    }

    let { project, loading, editor, onBack, onOpenEditor }: Props = $props();

    // 检查项目是否有 casp-portal 子目录（用于显示「开发模式」按钮）
    // 通过 Tauri 命令扫描项目目录，不依赖 sub_items（git 仓库时 sub_items 为空）
    let hasCaspPortal = $state(false);
    let caspPortalPath = $state<string | null>(null);

    $effect(() => {
        void project.path;
        detectCaspPortal(project.path);
    });

    async function detectCaspPortal(projectPath: string) {
        hasCaspPortal = false;
        caspPortalPath = null;
        try {
            // 先检查项目本身是否以 casp-portal 开头
            const folderName = projectPath.split("/").pop() || "";
            if (folderName.startsWith("casp-portal")) {
                hasCaspPortal = true;
                caspPortalPath = projectPath;
                return;
            }
            // 调用后端扫描项目目录下的子目录，查找 casp-portal* 目录
            const found = await invoke<string | null>("find_casp_portal_dir", {
                path: projectPath,
            });
            if (found) {
                hasCaspPortal = true;
                caspPortalPath = found;
            }
        } catch {
            // 忽略错误
        }
    }

    function gotoDevMode() {
        const devPath = caspPortalPath ?? project.path;
        goto(
            `/dev-mode/${encodeURIComponent(devPath)}?from=${encodeURIComponent(project.path)}`,
        );
    }
</script>

<div class="detail-view">
    <div class="detail-nav">
        <button class="back-btn" onclick={onBack}>
            <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><polyline points="15 18 9 12 15 6" /></svg
            >
            返回
        </button>
    </div>
    {#if loading}
        <div class="detail-loading">
            <div class="spinner"></div>
            <span>加载详情中...</span>
        </div>
    {:else}
        <div class="detail-header">
            <div class="detail-header-left">
                <div
                    class="detail-avatar"
                    style="background: {folderColor(project.name)}22; color: {folderColor(project.name)}"
                    >{project.name.charAt(0).toUpperCase()}</div
                >
                <div>
                    <h2 class="detail-title">{project.name}</h2>
                    <div class="detail-path" title={project.path}>
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><path
                                d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                        /></svg>
                        <span>{project.path}</span>
                    </div>
                </div>
            </div>
            <button class="editor-open-btn" onclick={() => onOpenEditor(project.path)}>
                <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><polygon points="5 3 19 12 5 21 5 3" /></svg
                >
                在 {editor.name || "编辑器"} 中打开
            </button>
            {#if hasCaspPortal}
                <button class="dev-mode-btn" onclick={gotoDevMode}>
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
                    开发模式
                </button>
            {/if}
        </div>
        {#if project.readme_preview}
            <div class="detail-readme">
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
                        ><path
                            d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
                        /><polyline points="14 2 14 8 20 8" /></svg
                    ><span>README</span>
                </div>
                <pre class="readme-content">{project.readme_preview}</pre>
            </div>
        {/if}

        <!-- ===== node_modules 管理 ===== -->
        <NodeModulesPanel
            projectPath={project.path}
            editorName={editor.name}
            onOpenEditor={onOpenEditor}
        />

        {#snippet renderSubItem(item: SubDetail)}
            <div
                class="sub-detail-card"
                style="margin-left: {Math.min(item.depth, 5) * 8}px"
            >
                <div class="sub-detail-header">
                    <span class="sub-detail-icon">📁</span>
                    <span class="sub-detail-name">{item.name}</span>
                    <button
                        class="sub-open-btn"
                        onclick={() => onOpenEditor(item.path)}
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
                            ><polygon points="5 3 19 12 5 21 5 3" /></svg
                        >
                        打开
                    </button>
                </div>
                {#if item.readme_preview}
                    <div class="sub-readme">
                        <div class="sub-readme-header">📖 README</div>
                        <pre class="sub-readme-content">{item.readme_preview}</pre>
                    </div>
                {/if}
                {#if item.git_repo}
                    <div class="git-info">
                        <svg
                            width="14"
                            height="14"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            ><circle cx="18" cy="18" r="3" /><circle
                                cx="6"
                                cy="6"
                                r="3"
                            /><path d="M13 6h3a2 2 0 0 1 2 2v7" /><line
                                x1="6"
                                y1="9"
                                x2="6"
                                y2="21" /></svg
                        >
                        <a
                            class="git-url"
                            href={item.git_repo.remote_url || "#"}
                            target="_blank"
                            rel="noreferrer">{item.git_repo.remote_url}</a
                        >
                    </div>
                {/if}
                {#if item.children && item.children.length > 0}
                    {#each item.children as child}
                        {@render renderSubItem(child)}
                    {/each}
                {/if}
            </div>
        {/snippet}

        {#if project.sub_items.length > 0}
            <div class="detail-subs">
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
                        ><path
                            d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                    /></svg
                    ><span>子项目 ({project.sub_items.length})</span>
                </div>
                <div class="sub-detail-list">
                    {#each project.sub_items as item}
                        {@render renderSubItem(item)}
                    {/each}
                </div>
            </div>
        {:else}
            <div class="no-subs">
                <span class="no-subs-icon">📭</span>
                <p>该项目下没有子目录</p>
            </div>
        {/if}
    {/if}
</div>

<style>
    /* ========== 项目详情视图 ========== */
    .detail-view {
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

    .detail-nav {
        margin-bottom: 20px;
    }

    .back-btn {
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

    .back-btn:hover {
        background: var(--bg-subtle);
        border-color: var(--text-placeholder);
    }

    .detail-loading {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 12px;
        padding: 60px;
        color: var(--text-secondary);
    }

    .detail-loading .spinner {
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

    .detail-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 24px;
        padding: 20px 24px;
        background: var(--bg-card);
        border-radius: 14px;
        border: 1px solid var(--border-light);
        flex-wrap: wrap;
    }

    .detail-header-left {
        display: flex;
        align-items: center;
        gap: 16px;
        min-width: 0;
        flex: 1;
    }

    .detail-avatar {
        width: 48px;
        height: 48px;
        border-radius: 14px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 22px;
        font-weight: 700;
        flex-shrink: 0;
    }

    .detail-title {
        font-size: 22px;
        font-weight: 700;
        color: var(--text-primary);
        margin-bottom: 4px;
    }

    .detail-path {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--text-muted);
        font-family: ui-monospace, monospace;
        max-width: 500px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .detail-path svg {
        flex-shrink: 0;
    }

    .detail-path span {
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .editor-open-btn {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 20px;
        background: var(--accent-gradient);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s ease;
        white-space: nowrap;
        box-shadow: 0 2px 6px var(--accent-shadow);
    }

    .editor-open-btn:hover {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px var(--accent-shadow-hover);
    }

    .dev-mode-btn {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 20px;
        background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s ease;
        white-space: nowrap;
        box-shadow: 0 2px 6px rgba(102, 126, 234, 0.3);
        margin-left: 10px;
    }

    .dev-mode-btn:hover {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px rgba(102, 126, 234, 0.5);
    }

    /* 区域标题 */
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

    /* README 预览 */
    .detail-readme {
        margin-bottom: 24px;
        padding: 20px;
        background: var(--bg-card);
        border-radius: 14px;
        border: 1px solid var(--border-light);
    }

    .readme-content {
        font-size: 13px;
        color: var(--text-secondary);
        line-height: 1.6;
        white-space: pre-wrap;
        font-family: ui-monospace, monospace;
        overflow-x: auto;
    }

    /* 子项目列表 */
    .detail-subs {
        margin-bottom: 24px;
    }

    .sub-detail-list {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .sub-detail-card {
        padding: 18px 20px;
        background: var(--bg-card);
        border-radius: 14px;
        border: 1px solid var(--border-light);
        border-left: 4px solid var(--border);
        transition: all 0.2s ease;
    }

    .sub-detail-card:hover {
        box-shadow: 0 2px 8px var(--shadow-sm);
    }

    .sub-detail-header {
        display: flex;
        align-items: center;
        gap: 10px;
        margin-bottom: 10px;
    }

    .sub-detail-icon {
        font-size: 20px;
        flex-shrink: 0;
    }

    .sub-detail-name {
        font-size: 15px;
        font-weight: 600;
        color: var(--text-primary);
        font-family: ui-monospace, monospace;
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .git-info {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--text-secondary);
        padding: 8px 12px;
        background: var(--bg-subtle);
        border-radius: 8px;
        border: 1px solid var(--border-light);
        margin-bottom: 8px;
    }

    .git-info svg {
        flex-shrink: 0;
        color: var(--text-secondary);
    }

    .git-url {
        color: var(--link);
        text-decoration: none;
        font-family: ui-monospace, monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .git-url:hover {
        text-decoration: underline;
        color: var(--link-hover);
    }

    .sub-open-btn {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 6px 12px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 6px;
        font-size: 12px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s ease;
    }

    .sub-open-btn:hover {
        background: var(--bg-card-hover);
        color: var(--text-primary);
    }

    /* 子项目 README */
    .sub-readme {
        margin: 8px 0;
        padding: 10px 12px;
        background: var(--bg-subtle);
        border-radius: 8px;
        border: 1px solid var(--border-light);
    }

    .sub-readme-header {
        display: flex;
        align-items: center;
        gap: 5px;
        font-size: 11px;
        font-weight: 600;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.5px;
        margin-bottom: 6px;
    }

    .sub-readme-content {
        font-size: 12px;
        color: var(--text-secondary);
        line-height: 1.5;
        white-space: pre-wrap;
        font-family: ui-monospace, monospace;
        margin: 0;
    }

    .no-subs {
        text-align: center;
        padding: 60px 20px;
        background: var(--bg-card);
        border-radius: 14px;
        border: 1px solid var(--border-light);
    }

    .no-subs-icon {
        font-size: 40px;
        display: block;
        margin-bottom: 8px;
    }

    .no-subs p {
        color: var(--text-muted);
        font-size: 14px;
    }
</style>
