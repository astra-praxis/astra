/**
 * Agent 行为：多级放权 / 自动压缩 / 图片识别。
 * 全部经 agent store 即时生效（localStorage 持久化，放权级别同步后端），
 * 不走底部保存栏 —— 与主题同属「即时生效」类。
 * 轮数 / 工具调用等防循环预算不在此暴露：后端内部兑底 9999，避免任务被预算报错打断。
 */
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useAgentStore, type AutoMode } from '@/stores/agent';

const AUTO_MODE_OPTIONS: { value: AutoMode; label: string; desc: string }[] = [
  {
    value: 'off',
    label: '关闭',
    desc: '所有确认类操作都会弹确认卡，等待你逐个批准。',
  },
  {
    value: 'project',
    label: '当前目录',
    desc: '当前项目目录内的文件读写、编辑等路径操作自动批准；其他操作仍弹卡。',
  },
  {
    value: 'workspace',
    label: '工作空间',
    desc: '已添加工作空间内的路径操作自动批准；其他操作仍弹卡。',
  },
  {
    value: 'all',
    label: '全部放行',
    desc: '所有确认类操作（含 Shell 命令）自动批准，不再弹窗。',
  },
];

function Switch({ on, onToggle, title }: { on: boolean; onToggle: () => void; title: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      className={`agent-perm-switch${on ? ' is-on' : ''}`}
      title={title}
      onClick={onToggle}
    >
      <span className="agent-perm-knob" />
    </button>
  );
}

export default function AgentSettings() {
  const autoMode = useAgentStore((s) => s.autoMode);
  const setAutoMode = useAgentStore((s) => s.setAutoMode);
  const autoCompactTokens = useAgentStore((s) => s.autoCompactTokens);
  const setAutoCompactTokens = useAgentStore((s) => s.setAutoCompactTokens);
  const imageRecognition = useAgentStore((s) => s.imageRecognition);
  const setImageRecognition = useAgentStore((s) => s.setImageRecognition);

  return (
    <SettingsSection iconId="agent" title="Agent 行为" desc="控制 Agent 执行任务时的确认策略与能力边界，修改立即生效">
      <SettingsRow
        title="放权模式"
        desc="控制哪些确认类操作自动批准。路径级放权只覆盖文件读写、编辑等明确落在边界内的操作；Shell 命令仅在「全部放行」时自动批准。未注册工具、提权命令、项目外路径逃逸始终被硬规则拦截。"
        stacked
      >
        <div className="agent-perm-stack">
          <div className="agent-perm-segments" role="radiogroup" aria-label="放权模式">
            {AUTO_MODE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                type="button"
                role="radio"
                aria-checked={autoMode === opt.value}
                className={`agent-perm-segment${autoMode === opt.value ? ' is-active' : ''}`}
                onClick={() => setAutoMode(opt.value)}
              >
                {opt.label}
              </button>
            ))}
          </div>
          <p className="agent-perm-hint">
            {AUTO_MODE_OPTIONS.find((o) => o.value === autoMode)?.desc}
          </p>
        </div>
      </SettingsRow>

      <SettingsRow
        title="上下文自动压缩"
        desc="长对话超过该估算 token 阈值时，Agent 自动把历史压缩成结构化摘要后继续任务（对齐 Zed 的 /compact），避免超出模型上下文上限。设为 0 关闭；修改后新会话生效。"
      >
        <input
          type="number"
          min={0}
          max={1000000}
          step={1000}
          value={autoCompactTokens}
          className="agent-perm-input"
          onChange={(e) => {
            const n = Number(e.target.value);
            if (Number.isFinite(n) && n >= 0) setAutoCompactTokens(n);
          }}
        />
      </SettingsRow>

      <SettingsRow
        title="图片识别"
        desc="开启表示你已确认所选模型及网关支持 vision / image_url，并同意将 read_file 读取的项目图片上传至所选模型。图片内容会写入本机 SQLite 会话历史（含 WAL、备份），删除源图片不会删除历史；不支持时会报错，不会静默降级。仅新会话生效；旧会话保持原设置，配置或模型来源不一致时拒绝续聊。支持 PNG、JPEG、WebP。"
      >
        <Switch
          on={imageRecognition}
          onToggle={() => setImageRecognition(!imageRecognition)}
          title={imageRecognition ? '已开启：新会话允许读取图片（需模型支持）' : '已关闭：点击开启'}
        />
      </SettingsRow>
    </SettingsSection>
  );
}
