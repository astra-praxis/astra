<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import type { ModelConfig, PathCheckResult } from "./types";
    import Modal from "$lib/ui/Modal.svelte";

    interface Props {
        /** null = 添加模式；传入模型 = 编辑模式 */
        model?: ModelConfig | null;
        onClose: () => void;
        /** 保存成功后由页面刷新列表 */
        onSaved: () => void;
    }

    let { model = null, onClose, onSaved }: Props = $props();

    // 仅捕获初值：弹窗每次打开都全新挂载（svelte-ignore state_referenced_locally）
    // svelte-ignore state_referenced_locally
    const isEdit = model !== null;
    // svelte-ignore state_referenced_locally
    const initial = model ?? {
        name: "",
        server_path: "",
        model_path: "",
        port: 8080,
        ngl: 999,
    };

    let name = $state(initial.name);
    let serverPath = $state(initial.server_path);
    let modelPath = $state(initial.model_path);
    let port = $state(initial.port);
    let ngl = $state(initial.ngl);
    let formError = $state("");
    let formSaving = $state(false);
    let pathChecking = $state(false);
    let pathResult = $state<PathCheckResult | null>(null);

    async function save() {
        formError = "";
        if (!name.trim()) {
            formError = "请输入模型名称";
            return;
        }
        if (!modelPath.trim()) {
            formError = "请输入模型文件路径";
            return;
        }
        if (!port || port < 1 || port > 65535) {
            formError = "端口号范围 1-65535";
            return;
        }
        if (ngl < 0 || !Number.isInteger(ngl)) {
            formError = "ngl 必须为非负整数";
            return;
        }

        formSaving = true;
        try {
            const config: ModelConfig = {
                // 编辑保持原 ID；添加按名称+端口生成
                id: model ? model.id : `model-${name.trim().toLowerCase().replace(/\s+/g, "-").trim()}-${port}`,
                name: name.trim(),
                model_path: modelPath.trim(),
                server_path: serverPath.trim() || "llama", // 空则默认使用 'llama'
                port,
                ngl,
            };
            await invoke("save_model_config", { model: config });
            onClose();
            onSaved();
        } catch (e) {
            formError = `保存失败: ${e}`;
        } finally {
            formSaving = false;
        }
    }

    async function checkPaths() {
        pathChecking = true;
        pathResult = null;
        try {
            const [modelValid, modelError, serverValid, serverError] =
                await invoke<[boolean, string, boolean, string]>(
                    "check_model_paths",
                    { modelPath, serverPath },
                );
            pathResult = { modelValid, modelError, serverValid, serverError };
        } catch (e) {
            pathResult = {
                modelValid: false,
                modelError: `校验失败: ${e}`,
                serverValid: false,
                serverError: "",
            };
        } finally {
            pathChecking = false;
        }
    }
</script>

<Modal title={isEdit ? "编辑模型" : "添加模型"} onClose={onClose} closeOnOverlay={false}>
    <div class="modal-body">
        {#if formError}
            <div class="form-error">{formError}</div>
        {/if}
        <div class="form-group">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>模型名称</label>
            <input type="text" bind:value={name} placeholder="如: qwen2.5-7b" />
        </div>
        <div class="form-group">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label>llama 路径 <span class="optional-tag">(可选)</span></label>
            <input
                type="text"
                bind:value={serverPath}
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
                bind:value={modelPath}
                placeholder="/path/to/model.gguf"
            />
        </div>
        <div class="form-row">
            <div class="form-group">
                <!-- svelte-ignore a11y_label_has_associated_control -->
                <label>端口</label>
                <input type="number" bind:value={port} min="1" max="65535" />
            </div>
            <div class="form-group">
                <!-- svelte-ignore a11y_label_has_associated_control -->
                <label>ngl (GPU 层数)</label>
                <input type="number" bind:value={ngl} min="0" />
            </div>
        </div>

        <!-- 路径校验区域 -->
        <div class="path-check-section">
            <button class="btn-check" onclick={checkPaths} disabled={pathChecking}>
                {#if pathChecking}
                    <span class="btn-spinner"></span>
                    校验中...
                {:else}
                    🔍 校验路径
                {/if}
            </button>
            {#if pathResult}
                <div class="path-result">
                    <div
                        class="path-result-item {pathResult.modelValid
                            ? 'path-ok'
                            : 'path-fail'}"
                    >
                        {pathResult.modelValid ? "✅" : "❌"} 模型文件：{pathResult
                            .modelValid
                            ? "有效"
                            : pathResult.modelError}
                    </div>
                    <div
                        class="path-result-item {pathResult.serverValid
                            ? 'path-ok'
                            : 'path-fail'}"
                    >
                        {pathResult.serverValid ? "✅" : "❌"} llama 路径：{pathResult
                            .serverValid
                            ? "有效"
                            : pathResult.serverError}
                    </div>
                </div>
            {/if}
        </div>
    </div>
    <div class="modal-footer">
        <button class="btn-cancel" onclick={onClose}>取消</button>
        <button class="btn-save" onclick={save} disabled={formSaving}>
            {formSaving ? "保存中..." : "保存"}
        </button>
    </div>
</Modal>

