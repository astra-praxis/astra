<script lang="ts">
    interface Props {
        systemPrompt: string;
        temperature: number;
        maxTokens: number;
        disabled: boolean;
    }

    let { systemPrompt = $bindable(), temperature = $bindable(), maxTokens = $bindable(), disabled }: Props =
        $props();
</script>

<div class="settings-sheet">
    <div class="ss-field">
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label>系统提示</label>
        <textarea
            bind:value={systemPrompt}
            rows="2"
            disabled={disabled}
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
                disabled={disabled}
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
                disabled={disabled}
            />
        </div>
    </div>
</div>

<style>
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
</style>
