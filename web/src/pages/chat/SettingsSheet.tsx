/**
 * 参数设置面板
 * 移植自 src/routes/chat/SettingsSheet.svelte。
 * $bindable → 受控 props + onChange 回调。
 */
import './SettingsSheet.scss';

interface Props {
  systemPrompt: string;
  temperature: number;
  maxTokens: number;
  disabled: boolean;
  onSystemPromptChange: (v: string) => void;
  onTemperatureChange: (v: number) => void;
  onMaxTokensChange: (v: number) => void;
}

function parseNumberInput(raw: string): number | null {
  if (raw === '') return null; // 输入中保持原值，避免被打断
  const n = Number(raw);
  return Number.isNaN(n) ? null : n;
}

export default function SettingsSheet({
  systemPrompt,
  temperature,
  maxTokens,
  disabled,
  onSystemPromptChange,
  onTemperatureChange,
  onMaxTokensChange,
}: Props) {
  return (
    <div className="settings-sheet">
      <div className="ss-field">
        <label>系统提示</label>
        <textarea
          value={systemPrompt}
          rows={2}
          disabled={disabled}
          placeholder="为对话设定角色和约束"
          onChange={(e) => onSystemPromptChange(e.target.value)}
        />
      </div>
      <div className="ss-row">
        <div className="ss-field-inline">
          <label>温度</label>
          <input
            type="number"
            step="0.1"
            min="0"
            max="2"
            value={temperature}
            disabled={disabled}
            onChange={(e) => {
              const n = parseNumberInput(e.target.value);
              if (n !== null) onTemperatureChange(n);
            }}
          />
        </div>
        <div className="ss-field-inline">
          <label>最大 Token</label>
          <input
            type="number"
            step="100"
            min="1"
            value={maxTokens}
            disabled={disabled}
            onChange={(e) => {
              const n = parseNumberInput(e.target.value);
              if (n !== null) onMaxTokensChange(n);
            }}
          />
        </div>
      </div>
    </div>
  );
}
