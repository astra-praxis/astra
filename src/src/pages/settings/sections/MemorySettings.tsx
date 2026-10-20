/**
 * 记忆：Agent 总结开关（重启生效）+ 总结用模型来源（条件展示）
 */
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useSettingsStore } from '../hooks/useSettings';

export default function MemorySettings() {
  const draft = useSettingsStore((s) => s.draft);
  const providers = useSettingsStore((s) => s.providers);
  const update = useSettingsStore((s) => s.update);

  if (!draft) return null;

  return (
    <SettingsSection iconId="memory" title="记忆" desc="会话记忆的总结方式与模型来源">
      <SettingsRow
        title="记忆 Agent 总结"
        desc="开启后由模型做会话总结与决策提炼（更连贯，但每次压缩有一次额外调用）；关闭时走本地规则合并（零模型依赖）。总结失败自动降级回规则，不影响记忆可用性。重启应用后生效。"
      >
        <button
          type="button"
          role="switch"
          aria-checked={draft.memoryAgentSummary}
          className={`agent-perm-switch${draft.memoryAgentSummary ? ' is-on' : ''}`}
          title={draft.memoryAgentSummary ? '已开启：模型做会话总结' : '已关闭：规则合并'}
          onClick={() => update({ memoryAgentSummary: !draft.memoryAgentSummary })}
        >
          <span className="agent-perm-knob" />
        </button>
      </SettingsRow>

      {draft.memoryAgentSummary && (
        <SettingsRow
          title="总结用模型来源"
          desc="做会话总结的供应商与活动模型（在 API 接口页配置）。默认自动选第一个配置齐全的。"
        >
          <select
            value={draft.memorySummaryProviderId ?? ''}
            className="agent-perm-input agent-perm-input--select"
            onChange={(e) => update({ memorySummaryProviderId: e.target.value || null })}
          >
            <option value="">自动（第一个可用的）</option>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
                {p.active_model ? `（${p.active_model}）` : ''}
              </option>
            ))}
          </select>
        </SettingsRow>
      )}
    </SettingsSection>
  );
}
