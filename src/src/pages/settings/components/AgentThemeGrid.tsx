/**
 * Agent 对话主题选择器（设置页「常规」分区使用）
 * 选中即写 localStorage（agent-theme）并即时生效，无需点保存。
 */
import { Check } from 'lucide-react';
import { AGENT_THEMES, useAgentThemeStore } from '@/lib/agentTheme';
import AgentThemePreview from '@/components/agent/AgentThemePreview';

export default function AgentThemeGrid() {
  const theme = useAgentThemeStore((s) => s.id);
  const setId = useAgentThemeStore((s) => s.setId);

  return (
    <div className="theme-grid" role="radiogroup" aria-label="Agent 对话主题">
      {AGENT_THEMES.map((t) => (
        <button
          key={t.id}
          role="radio"
          aria-checked={theme === t.id}
          className={`theme-option agent-theme-option${theme === t.id ? ' selected' : ''}`}
          onClick={() => setId(t.id)}
        >
          <AgentThemePreview theme={t.id} className="agent-theme-preview" />
          <span className="theme-meta">
            <span className="theme-name">{t.label}</span>
            {theme === t.id && <Check size={15} strokeWidth={2.5} className="theme-check" />}
          </span>
        </button>
      ))}
    </div>
  );
}
