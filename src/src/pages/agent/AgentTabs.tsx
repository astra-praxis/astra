/**
 * Agent 多会话标签栏（SESSION_RUNTIME_SPEC S0）
 *
 * tab 即 AgentSession 视图：切换时当前会话驻留内存（零重建），运行中的会话持续吸收事件。
 * tab 状态（openTabs / activeTabId）放在 agent store 内（测试 mock 约束，不拆独立 store），
 * 所以本组件直接订阅整个 store——tab 数量 ≤ MAX_RETAINED_TABS(8)，整订渲染开销可忽略。
 */
import { Loader2, MessageSquare, Plus, X } from 'lucide-react';
import { hasPendingDecision, runningSessionId, useAgentStore } from '../../stores/agent';
import type { AgentSessionMeta } from './AgentHistoryPanel';
import './AgentTabs.scss';

type AgentStoreState = ReturnType<typeof useAgentStore.getState>;

interface TabInfo {
  title: string;
  running: boolean;
  /** 有待用户决策的卡片（权限 / 提问） */
  waiting: boolean;
  draft: boolean;
}

/** 从条目流推导标题（与 saveSession 的标题规则一致） */
function titleOfEntries(entries: { kind: string; text?: string }[]): string {
  const firstUser = entries.find((e) => e.kind === 'user');
  return firstUser?.text ? firstUser.text.slice(0, 30) : 'Agent 会话';
}

function tabInfo(id: string, st: AgentStoreState, sessions: AgentSessionMeta[]): TabInfo {
  if (id === st.sessionId) {
    return {
      title: st.isDraft ? '新会话' : titleOfEntries(st.entries),
      running: st.running,
      waiting: hasPendingDecision(st.entries),
      draft: st.isDraft,
    };
  }
  const p = st.parked[id];
  if (p) {
    return {
      title: p.isDraft ? '新会话' : titleOfEntries(p.entries),
      running: p.running,
      waiting: hasPendingDecision(p.entries),
      draft: p.isDraft,
    };
  }
  const meta = sessions.find((s) => s.id === id);
  return { title: meta?.title ?? '会话', running: false, waiting: false, draft: false };
}

export default function AgentTabs() {
  const st = useAgentStore();
  const sessions = st.sessions;

  return (
    <div className="atabs" role="tablist" aria-label="Agent 会话标签">
      <div className="atabs__list">
        {st.openTabs.map((id) => {
          const info = tabInfo(id, st, sessions);
          const active = id === st.activeTabId;
          return (
            <div
              key={id}
              role="tab"
              aria-selected={active}
              tabIndex={0}
              className={`atabs__tab${active ? ' is-active' : ''}${info.running ? ' is-running' : ''}`}
              onClick={() => st.setTabActive(id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !active) st.setTabActive(id);
              }}
              title={info.title}
            >
              {info.running ? (
                <Loader2 size={11} strokeWidth={2.2} className="atabs__spin" />
              ) : (
                <MessageSquare size={11} strokeWidth={2} />
              )}
              <span className="atabs__title">{info.title}</span>
              {info.waiting && <span className="atabs__dot" title="有待处理的确认 / 提问" />}
              <button
                type="button"
                className="atabs__close"
                onClick={(e) => {
                  e.stopPropagation();
                  if (!st.closeTab(id)) st.setError('该会话正在运行，请先停止后再关闭');
                }}
                title="关闭会话标签"
                aria-label="关闭会话标签"
              >
                <X size={10} strokeWidth={2.4} />
              </button>
            </div>
          );
        })}
      </div>
      <button
        type="button"
        className="atabs__new"
        onClick={() => st.newTab()}
        title="新建会话标签（当前会话驻留内存，可随时切回）"
        aria-label="新建会话标签"
      >
        <Plus size={13} strokeWidth={2.2} />
      </button>
      {(() => {
        const busy = runningSessionId(st);
        // 忙碌会话正是当前激活 tab 时已有状态栏，不重复提示
        return busy && busy !== st.activeTabId ? (
          <span className="atabs__running-hint" title="另一个会话正在运行：可自由切换查看，但发送需等它结束">
            <Loader2 size={11} strokeWidth={2.2} className="atabs__spin" />
            运行中
          </span>
        ) : null;
      })()}
    </div>
  );
}
