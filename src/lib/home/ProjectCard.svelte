<script lang="ts">
    import { folderColor, folderName, highlight } from "./folder-utils";
    import type { ProjectCard } from "./types";

    interface Props {
        project: ProjectCard;
        searchQuery: string;
        editorName: string;
        /** 点击卡片主体 → 查看详情 */
        onOpen: (project: ProjectCard) => void;
        /** 用编辑器打开项目/子项目 */
        onOpenProject: (path: string) => void;
    }

    let { project, searchQuery, editorName, onOpen, onOpenProject }: Props =
        $props();
</script>

<div
    class="project-card"
    style="--card-accent: {folderColor(project.name)}"
    onclick={() => onOpen(project)}
    onkeydown={(e) => e.key === "Enter" && onOpen(project)}
    role="button"
    tabindex="0"
    title="查看项目详情"
>
    <div class="card-accent-bar"></div>
    <div class="card-content">
        <div class="card-header">
            <div
                class="card-avatar"
                style="background: {folderColor(project.name)}22; color: {folderColor(project.name)}"
                >{project.name.charAt(0).toUpperCase()}</div
            >
            <div class="card-header-text">
                <h3 class="card-title"
                    >{@html highlight(project.name, searchQuery)}</h3
                >
                <span class="card-folder"
                    >{@html highlight(folderName(project.path), searchQuery)}</span
                >
            </div>
        </div>
        <div class="card-path" title={project.path}>
            <svg
                width="13"
                height="13"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><path
                    d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
            /></svg><span>{project.path}</span>
        </div>
        {#if project.sub_projects?.length > 0}
            <div class="sub-projects">
                <div class="sub-label">
                    <svg
                        width="12"
                        height="12"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><path
                            d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                    /></svg
                    ><span>子项目 ({project.sub_projects.length})</span>
                </div>
                <div class="sub-list">
                    {#each project.sub_projects as sub}
                        <button
                            class="sub-item"
                            onclick={(e) => {
                                e.stopPropagation();
                                onOpenProject(sub.path);
                            }}
                            title="在 {editorName || "编辑器"} 中打开 {sub.name}"
                            ><span class="sub-item-name">{sub.name}</span></button
                        >
                    {/each}
                </div>
            </div>
        {/if}
        <div class="card-footer">
            <div class="footer-right">
                <button
                    class="open-editor-btn"
                    onclick={(e) => {
                        e.stopPropagation();
                        onOpenProject(project.path);
                    }}
                    title="在 {editorName || "编辑器"} 中打开"
                    ><svg
                        width="13"
                        height="13"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><polygon points="5 3 19 12 5 21 5 3"
                    /></svg
                    >打开</button
                >
            </div>
        </div>
    </div>
</div>

<style>
    .project-card {
        background: var(--bg-card);
        border-radius: 14px;
        overflow: hidden;
        box-shadow: 0 1px 3px var(--shadow-sm);
        border: 1px solid var(--border-light);
        transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
        display: flex;
        flex-direction: column;
        cursor: pointer;
        outline: none;
    }

    .project-card:hover {
        box-shadow: 0 8px 24px var(--shadow-hover);
        transform: translateY(-3px);
        border-color: transparent;
    }

    .project-card:focus-visible {
        box-shadow: 0 0 0 3px var(--accent-ring);
        transform: translateY(-2px);
    }

    .card-accent-bar {
        height: 4px;
        background: var(--card-accent, #667eea);
        flex-shrink: 0;
    }

    .card-content {
        padding: 20px;
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 14px;
    }

    .card-header {
        display: flex;
        align-items: center;
        gap: 14px;
    }

    .card-avatar {
        width: 42px;
        height: 42px;
        border-radius: 12px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 18px;
        font-weight: 700;
        flex-shrink: 0;
    }

    .card-header-text {
        min-width: 0;
        flex: 1;
    }

    .card-title {
        font-size: 16px;
        font-weight: 600;
        color: var(--text-primary);
        margin: 0;
        line-height: 1.3;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .card-title :global(mark) {
        background: var(--highlight-bg);
        color: var(--highlight-text);
        border-radius: 3px;
        padding: 0 2px;
    }

    .card-folder {
        font-size: 12px;
        color: var(--text-muted);
        font-family: ui-monospace, monospace;
    }

    .card-folder :global(mark) {
        background: var(--highlight-bg);
        color: var(--highlight-text);
        border-radius: 3px;
        padding: 0 2px;
    }

    .card-path {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 12px;
        color: var(--text-muted);
        padding: 8px 10px;
        background: var(--bg-subtle);
        border-radius: 8px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        border: 1px solid var(--border-light);
    }

    .card-path svg {
        flex-shrink: 0;
    }

    .card-path span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    /* ===== 子项目 ===== */
    .sub-projects {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .sub-label {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 11px;
        color: var(--text-muted);
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
    }

    .sub-label svg {
        flex-shrink: 0;
    }

    .sub-list {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }

    .sub-item {
        display: inline-flex;
        align-items: center;
        padding: 4px 10px;
        background: var(--accent-bg);
        border: 1px solid var(--border-strong);
        border-radius: 6px;
        font-size: 12px;
        font-weight: 500;
        color: var(--link);
        cursor: pointer;
        transition: all 0.15s ease;
        font-family: ui-monospace, monospace;
    }

    .sub-item:hover {
        background: var(--accent-bg-hover);
        border-color: var(--link);
        color: var(--link-hover);
    }

    .sub-item:active {
        transform: scale(0.97);
    }

    .card-footer {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        margin-top: auto;
    }

    .footer-right {
        display: flex;
        align-items: center;
    }

    /* ========== 卡片打开按钮 ========== */
    .open-editor-btn {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 4px 10px;
        background: var(--bg-subtle);
        border: 1px solid var(--border);
        border-radius: 6px;
        font-size: 12px;
        font-weight: 500;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s ease;
        opacity: 0;
    }

    .project-card:hover .open-editor-btn {
        opacity: 1;
    }

    .open-editor-btn:hover {
        background: var(--bg-card-hover);
        color: var(--text-primary);
    }
</style>
