/**
 * Agent 行为：放权模式 / 最大轮数 / 自动压缩 / 图片识别。
 * 全部经 agent store 即时生效（localStorage 持久化，放权模式同步后端），
 * 不走底部保存栏 —— 与主题同属「即时生效」类。
 */
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useAgentStore } from '@/stores/agent';

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
  const autoApprove = useAgentStore((s) => s.autoApprove);
  const setAutoApprove = useAgentStore((s) => s.setAutoApprove);
  const maxTurns = useAgentStore((s) => s.maxTurns);
  const setMaxTurns = useAgentStore((s) => s.setMaxTurns);
  const autoCompactTokens = useAgentStore((s) => s.autoCompactTokens);
  const setAutoCompactTokens = useAgentStore((s) => s.setAutoCompactTokens);
  const imageRecognition = useAgentStore((s) => s.imageRecognition);
  const setImageRecognition = useAgentStore((s) => s.setImageRecognition);

  return (
    <SettingsSection iconId="agent" title="Agent 行为" desc="控制 Agent 执行任务时的确认策略与能力边界，修改立即生效">
      <SettingsRow
        title="放权模式（自动批准所有操作）"
        desc="开启后，Agent 的所有确认类操作（写文件、编辑、Shell 命令、网络请求、构建文档等）自动批准，不再弹窗等待点击。未注册工具、提权命令、项目外路径逃逸仍会被硬规则拦截。"
      >
        <Switch
          on={autoApprove}
          onToggle={() => setAutoApprove(!autoApprove)}
          title={autoApprove ? '已开启：所有操作自动批准，点击关闭' : '已关闭：点击开启'}
        />
      </SettingsRow>

      <SettingsRow
        title="最大执行轮数"
        desc="Agent 单次任务内「模型请求 → 工具执行」的最大循环次数，防止无限工具循环。超出后任务报错结束，修改后新会话生效。"
      >
        <input
          type="number"
          min={1}
          max={200}
          value={maxTurns}
          className="agent-perm-input"
          onChange={(e) => {
            const n = Number(e.target.value);
            if (Number.isFinite(n) && n >= 1) setMaxTurns(n);
          }}
        />
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
