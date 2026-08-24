<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import Modal from "$lib/ui/Modal.svelte";
    import type { ProjectCard } from "./types";

    interface Props {
        workspacePath: string;
        onClose: () => void;
        onCreated: (project: ProjectCard) => void;
        onError: (message: string) => void;
    }

    let { workspacePath, onClose, onCreated, onError }: Props = $props();

    let newFolderName = $state("");
    let newProjectName = $state("");
    let creating = $state(false);

    function requestClose() {
        if (creating) return;
        onClose();
    }

    async function createNewProject() {
        const folder = newFolderName.trim();
        if (!folder) return;
        creating = true;
        try {
            const newProject = await invoke<ProjectCard>("create_project", {
                workspacePath,
                folderName: folder,
                projectName: newProjectName.trim(),
            });
            onCreated(newProject);
        } catch (e) {
            onError(`创建失败: ${e}`);
        } finally {
            creating = false;
        }
    }
</script>

<Modal
    title="新建项目"
    onClose={requestClose}
    closeDisabled={creating}
>
    <div class="modal-body">
        <div class="form-group">
            <label for="folder-name"
                >文件夹名称 <span class="required">*</span></label
            >
            <input
                id="folder-name"
                type="text"
                placeholder="例如：my-project"
                bind:value={newFolderName}
                disabled={creating}
                onkeydown={(e) => e.key === "Enter" && createNewProject()}
            />
            <p class="form-hint">将在当前工作空间下创建此名称的子文件夹</p>
        </div>
        <div class="form-group">
            <label for="project-name"
                >项目名称 <span class="required">*</span></label
            >
            <input
                id="project-name"
                type="text"
                placeholder="输入项目显示名称"
                bind:value={newProjectName}
                disabled={creating}
                onkeydown={(e) => e.key === "Enter" && createNewProject()}
            />
            <p class="form-hint">项目 README.md 的标题</p>
        </div>
    </div>
    <div class="modal-footer">
        <button class="btn-cancel" onclick={requestClose} disabled={creating}
            >取消</button
        >
        <button
            class="btn-confirm"
            onclick={createNewProject}
            disabled={!newFolderName.trim() || !newProjectName.trim() || creating}
        >
            {#if creating}
                <div class="btn-spinner"></div>
                创建中...
            {:else}
                <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path d="M20 14.66V20a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5.34" /><polygon
                        points="18 2 22 6 12 16 8 16 8 12 18 2"/></svg
                >
                创建项目
            {/if}
        </button>
    </div>
</Modal>

<style>
    .form-group {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .form-group label {
        font-size: 14px;
        font-weight: 600;
        color: var(--text-secondary);
    }

    .required {
        color: var(--error-text);
    }

    .form-group input {
        padding: 10px 14px;
        background: var(--bg-input);
        border: 1px solid var(--border);
        border-radius: 10px;
        font-size: 14px;
        color: var(--text-primary);
        outline: none;
        transition: all 0.2s;
    }

    .form-group input::placeholder {
        color: var(--text-placeholder);
    }

    .form-group input:focus {
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-light);
    }

    .form-group input:disabled {
        background: var(--bg-subtle);
        cursor: not-allowed;
    }

    .form-hint {
        font-size: 12px;
        color: var(--text-muted);
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

    .btn-cancel:hover:not(:disabled) {
        background: var(--border-light);
    }

    .btn-cancel:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    .btn-confirm {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 22px;
        background: var(--accent-gradient);
        border: none;
        border-radius: 10px;
        font-size: 14px;
        font-weight: 600;
        color: white;
        cursor: pointer;
        transition: all 0.2s ease;
        box-shadow: 0 2px 6px var(--accent-shadow);
    }

    .btn-confirm:hover:not(:disabled) {
        transform: translateY(-1px);
        box-shadow: 0 4px 12px var(--accent-shadow-hover);
    }

    .btn-confirm:disabled {
        opacity: 0.6;
        cursor: not-allowed;
        box-shadow: none;
        transform: none;
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
