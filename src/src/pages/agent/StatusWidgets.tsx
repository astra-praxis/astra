/**
 * Agent 状态栏小组件（从 AgentPage 拆出）：思考模式 SplitButton + 上下文用量指示。
 */
import { useEffect, useRef, useState } from 'react';
import { Brain, Check, ChevronDown } from 'lucide-react';
import { THINKING_EFFORTS, type AgentUsage, type ThinkingEffort } from '@/stores/agent';

const EFFORT_LABEL: Record<ThinkingEffort, string> = {
  minimal: '最省',
  low: '低',
  medium: '中',
  high: '高',
};

/**
 * 思考模式 SplitButton（Zed 式）：主按钮切换开关（开 = accent 高亮），
 * 右侧小 chevron 展开力度菜单。菜单复用现有 popup 样式模式（绝对定位 + 外点关闭）。
 */
export function ThinkingButton(props: {
  enabled: boolean;
  effort: ThinkingEffort;
  onToggle: () => void;
  onEffortChange: (e: ThinkingEffort) => void;
}) {
  const { enabled, effort, onToggle, onEffortChange } = props;
  const [menuOpen, setMenuOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // 菜单打开时监听外点关闭（capture 不需要，pointerdown 先于 click 即可）
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [menuOpen]);

  return (
    <div ref={rootRef} className={`agent-dock__thinking${enabled ? ' is-on' : ''}`}>
      <button
        type="button"
        className="agent-dock__thinking-main"
        onClick={onToggle}
        aria-pressed={enabled}
        title={`思考模式：${enabled ? '开' : '关'} · 力度 ${EFFORT_LABEL[effort]}（模型/网关需支持思考，不支持时请求报错）`}
      >
        <Brain size={14} strokeWidth={1.8} />
      </button>
      <button
        type="button"
        className="agent-dock__thinking-chev"
        onClick={() => setMenuOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label="选择思考力度"
        title={`思考力度：${EFFORT_LABEL[effort]}`}
      >
        <ChevronDown size={11} strokeWidth={2.4} className={menuOpen ? 'agent-dock__thinking-chev-up' : ''} />
      </button>
      {menuOpen && (
        <div className="agent-dock__effort-menu" role="menu">
          {THINKING_EFFORTS.map((e) => (
            <button
              key={e}
              type="button"
              role="menuitemradio"
              aria-checked={effort === e}
              onClick={() => {
                onEffortChange(e);
                setMenuOpen(false);
              }}
            >
              <span>{EFFORT_LABEL[e]}</span>
              {effort === e && <Check size={13} strokeWidth={2.5} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** token 数缩写（1234 → 1.2k，1500000 → 1.5M），用于无上限时的用量展示 */
function humanizeTokens(n: number): string {
  if (n >= 1_000_000) {
    const v = n / 1_000_000;
    return `${Number.isInteger(v) ? v : v.toFixed(1)}M`;
  }
  if (n >= 1_000) {
    const v = n / 1_000;
    return `${Number.isInteger(v) ? v : v.toFixed(1)}k`;
  }
  return String(n);
}

/**
 * 上下文用量指示：有 max 显示环形进度 + 百分比（≥85% 警示色）；
 * 无 max 有 total 显示缩写；usage 为 null 不渲染。tooltip 给 input/output/total 明细。
 */
export function ContextUsage({ usage }: { usage: AgentUsage | null }) {
  if (!usage) return null;
  const { input, output, total, max } = usage;
  const used = total ?? (input != null && output != null ? input + output : undefined);
  const pct = max != null && max > 0 && used != null ? Math.min(used / max, 1) : null;
  if (pct == null && total == null) return null;
  const detail = `输入 ${input ?? '—'} · 输出 ${output ?? '—'} · 合计 ${total ?? '—'}${max != null ? ` · 上限 ${max}` : ''}`;
  const hot = pct != null && pct >= 0.85;
  const radius = 6;
  const circumference = 2 * Math.PI * radius;
  return (
    <span className={`agent-dock__usage${hot ? ' is-hot' : ''}`} title={detail}>
      {pct != null ? (
        <>
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <circle className="agent-dock__usage-track" cx="8" cy="8" r={radius} />
            <circle
              className="agent-dock__usage-arc"
              cx="8"
              cy="8"
              r={radius}
              strokeDasharray={`${circumference * pct} ${circumference}`}
            />
          </svg>
          <span className="agent-dock__usage-pct">{Math.round(pct * 100)}%</span>
        </>
      ) : (
        <span className="agent-dock__usage-total">{humanizeTokens(total!)}</span>
      )}
    </span>
  );
}
