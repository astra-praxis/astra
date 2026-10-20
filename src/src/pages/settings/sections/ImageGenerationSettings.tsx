/**
 * 图片生成（generate_image）：启用开关 + 网关 / 模型 / API Key。
 * 配置纳入统一草稿，由底部保存栏提交（不再有独立保存按钮）；
 * 关闭开关不清空已填参数。API Key 用密码框并可切换明文。
 */
import { useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useSettingsStore } from '../hooks/useSettings';

export default function ImageGenerationSettings() {
  const draft = useSettingsStore((s) => s.draft);
  const update = useSettingsStore((s) => s.update);
  const [showKey, setShowKey] = useState(false);

  if (!draft) return null;

  const gen = draft.genConfig;
  const setGen = (patch: Partial<typeof gen>) =>
    update({ genConfig: { ...gen, ...patch } });

  return (
    <SettingsSection
      iconId="image-gen"
      title="图片生成"
      desc="generate_image 工具的网关与模型配置；填写后通过底部保存栏统一保存，新会话生效"
    >
      <SettingsRow
        title="启用图片生成"
        desc="开启后 Agent 获得 generate_image 工具，可提交图片生成任务（付费外部调用，每次提交都会弹出确认卡）。API Key 仅保存在本机，不上传。"
      >
        <button
          type="button"
          role="switch"
          aria-checked={gen.enabled}
          className={`agent-perm-switch${gen.enabled ? ' is-on' : ''}`}
          title={gen.enabled ? '已开启：新会话注册 generate_image 工具' : '已关闭：点击开启'}
          onClick={() => setGen({ enabled: !gen.enabled })}
        >
          <span className="agent-perm-knob" />
        </button>
      </SettingsRow>

      {gen.enabled && (
        <>
          <SettingsRow
            title="生成网关与模型"
            desc="地址以 /v1 结尾（如 https://api.openai.com/v1）；模型为供应商的图片生成型号。未经真实网关验证前请先用小图试跑。"
            stacked
          >
            <div className="gen-field-group">
              <input
                type="text"
                value={gen.base_url}
                placeholder="https://api.openai.com/v1"
                className="gen-field-input"
                aria-label="生成网关地址"
                onChange={(e) => setGen({ base_url: e.target.value })}
              />
              <input
                type="text"
                value={gen.model_id}
                placeholder="gpt-image-1"
                className="gen-field-input"
                aria-label="生成模型"
                onChange={(e) => setGen({ model_id: e.target.value })}
              />
              <div className="gen-key-row">
                <input
                  type={showKey ? 'text' : 'password'}
                  value={gen.api_key}
                  placeholder="API Key（仅存本机）"
                  className="gen-field-input"
                  aria-label="API Key"
                  autoComplete="off"
                  onChange={(e) => setGen({ api_key: e.target.value })}
                />
                <button
                  type="button"
                  className="gen-key-toggle"
                  onClick={() => setShowKey((v) => !v)}
                  title={showKey ? '隐藏 API Key' : '显示 API Key'}
                >
                  {showKey ? <EyeOff size={15} strokeWidth={2} /> : <Eye size={15} strokeWidth={2} />}
                </button>
              </div>
            </div>
          </SettingsRow>
        </>
      )}
    </SettingsSection>
  );
}
