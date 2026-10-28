/**
 * Agent 状态栏小组件（从 AgentPage 拆出）：思考模式 SplitButton + 上下文用量指示
 * + 运行中子任务胶囊（执行时间 + 取消）。
 */
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Brain, Check, ChevronDown, Loader2, X } from 'lucide-react';
import { THINKING_EFFORTS, useAgentStore, type AgentUsage, type ThinkingEffort } from '@/stores/agent';

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

// ===== 运行中子任务胶囊（后台子进程 / 子 Agent / 生成 / MCP）=====

interface RunningTask {
  kind: string;
  id: string;
  label: string;
  elapsed_secs: number;
  /** 子 Agent：最新 RunStatus（serde 名） */
  detail?: string | null;
  /** 子 Agent：最近一次工具调用（"tool · args"） */
  last_tool?: string | null;
  /** 子 Agent：所属会话（关联发起委托的对话卡片） */
  session_id?: string | null;
  /** 子 Agent：父会话当前模型名（v1 子 Agent 与父 Run 同模型） */
  model?: string | null;
}

const KIND_LABEL: Record<string, string> = {
  process: '进程',
  subagent: '子 Agent',
  generation: '生成',
  mcp: 'MCP',
};

/**
 * 运行中子任务胶囊：Agent 拉起的子进程 / 子 Agent 委托 / 生成 / MCP 任务，
 * 每个「类型 + 取消」小按钮。执行耗时在对应工具卡片头部展示，这里不再重复。
 * 无运行中任务不渲染。
 */
export function RunningSubtasks() {
  const [tasks, setTasks] = useState<RunningTask[]>([]);

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const list = await invoke<RunningTask[]>('agent_tasks_running');
        if (!alive) return;
        setTasks(list);
      } catch {
        // 后端不可用（纯 web 调试）时静默，不打扰对话
      }
    };
    void poll();
    const poller = setInterval(poll, 3000);
    return () => {
      alive = false;
      clearInterval(poller);
    };
  }, []);

  if (tasks.length === 0) return null;

  const cancel = async (t: RunningTask) => {
    setTasks((list) => list.filter((x) => !(x.kind === t.kind && x.id === t.id)));
    try {
      await invoke('agent_task_cancel', { kind: t.kind, id: t.id });
    } catch (e) {
      useAgentStore.getState().setError(`取消失败：${String(e)}`);
      void invoke<RunningTask[]>('agent_tasks_running').then((l) => setTasks(l)).catch(() => {});
    }
  };

  return (
    <span className="agent-dock__subtasks">
      {tasks.map((t) => (
        <span key={`${t.kind}:${t.id}`} className="agent-dock__subtask-chip" title={t.last_tool ? `${t.label} · ${t.last_tool}` : t.label}>
          <Loader2 size={11} strokeWidth={2} className="agent-dock__subtask-spin" />
          <span className="agent-dock__subtask-kind">{KIND_LABEL[t.kind] ?? t.kind}</span>
          {t.model && <span className="agent-dock__subtask-model" title={t.model}>{t.model}</span>}
          <button
            type="button"
            className="agent-dock__subtask-stop"
            aria-label={`取消 ${KIND_LABEL[t.kind] ?? t.kind}任务`}
            title="取消该任务"
            onClick={() => void cancel(t)}
          >
            <X size={11} strokeWidth={2} />
          </button>
        </span>
      ))}
    </span>
  );
}
