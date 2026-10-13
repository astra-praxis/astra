/**
 * 参数设置面板
 * 移植自 src/routes/chat/SettingsSheet.svelte。
 * $bindable → 受控 props + onChange 回调。
 * 温度 / 最大 Token 已移除：agent-core 后端暂不支持这两个采样参数，避免「看起来能调其实无效」。
 * 图片识别开关在全局「设置 → Agent 权限」页（仅新会话生效的偏好放一处管理）。
 */
import './SettingsSheet.scss';

interface Props {
  systemPrompt: string;
  disabled: boolean;
  onSystemPromptChange: (v: string) => void;
}

export default function SettingsSheet({
  systemPrompt,
  disabled,
  onSystemPromptChange,
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
    </div>
  );
}
