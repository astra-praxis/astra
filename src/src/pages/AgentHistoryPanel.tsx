/**
 * Agent 会话历史面板（全高左侧栏，Zed 式「历史即导航」）
 *
 * - 全局平铺：不按项目分组（项目只是会话的属性，D2），按 updated_at 倒序（后端已排好）
 * - 时间分桶：今天 / 昨天 / 更早
 * - 搜索框常驻；↑/↓ 键盘导航 + Enter 打开（对齐 Zed history 的键盘操作）
 * - 面板打开时自动滚动定位到当前会话
 * - 每条目带项目徽标（纯聊天会话显示「纯聊天」）与 运行中 / 待处理 / 打开中 标识
 * - 宽度可拖拽调节（持久化 localStorage）
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Bot, Loader2, Plus, Search, Trash2, X } from 'lucide-react';
import { hasPendingDecision, useAgentStore } from '@/stores/agent';
import './AgentHistoryPanel.scss';

export interface AgentSessionMeta {
  id: string;
  project_path: string;
  project_name: string;
  title: string;
  entries: unknown;
  created_at: number;
  updated_at: number;
}

interface Props {
  sessions: AgentSessionMeta[];
  currentSessionId: string;
  /** 当前绑定的项目路径；非空时只显示该项目的会话（Zed 式按项目隔离），空 = 纯聊天模式显示全部 */
  projectPath: string;
  onSelect: (s: AgentSessionMeta) => void;
  onDelete: (id: string) => void;
  onNewChat: () => void;
}

type Bucket = '今天' | '昨天' | '更早';

function bucketOf(ts: number): Bucket {
  const d = new Date(ts * 1000);
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (d.getTime() >= startOfToday) return '今天';
  if (d.getTime() >= startOfToday - 86400_000) return '昨天';
  return '更早';
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  if (bucketOf(ts) === '今天') return `${hh}:${mm}`;
  return `${d.getMonth() + 1}/${d.getDate()} ${hh}:${mm}`;
}

const BUCKET_ORDER: Bucket[] = ['今天', '昨天', '更早'];

// ===== 面板宽度（拖拽调节 + 持久化） =====
const WIDTH_KEY = 'astra.history.width';
const WIDTH_MIN = 200;
const WIDTH_MAX = 420;
const WIDTH_DEFAULT = 240;

function loadWidth(): number {
  const v = Number(localStorage.getItem(WIDTH_KEY));
  return Number.isFinite(v) && v >= WIDTH_MIN && v <= WIDTH_MAX ? v : WIDTH_DEFAULT;
}

export default function AgentHistoryPanel({ sessions, currentSessionId, projectPath, onSelect, onDelete, onNewChat }: Props) {
  const st = useAgentStore();
  // 搜索框常驻（Zed 式）；↑/↓ 导航游标基于过滤后的平铺顺序
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState(0);
  const [width, setWidth] = useState(loadWidth);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // 面板打开（挂载）即聚焦搜索框，并定位到当前会话
  useEffect(() => {
    searchInputRef.current?.focus();
    scrollRef.current
      ?.querySelector('.ahist__item.is-current')
      ?.scrollIntoView({ block: 'nearest' });
  }, []);

  const filtered = useMemo(() => {
    // 选中项目 → 只看该项目的会话（Zed 式项目隔离）；未选项目（纯聊天）→ 全部
    const byProject = projectPath ? sessions.filter((s) => s.project_path === projectPath) : sessions;
    const q = query.trim().toLowerCase();
    if (!q) return byProject;
    return byProject.filter(
      (s) =>
        s.title.toLowerCase().includes(q) || s.project_name.toLowerCase().includes(q),
    );
  }, [sessions, projectPath, query]);

  // 搜索词变化后游标回到第一项
  useEffect(() => {
    setCursor(0);
  }, [query]);

  // 时间分桶（桶间按固定顺序，桶内保持后端倒序）
  const buckets = useMemo(() => {
    const map = new Map<Bucket, AgentSessionMeta[]>();
    for (const s of filtered) {
      const key = bucketOf(s.updated_at);
      const list = map.get(key) ?? [];
      list.push(s);
      map.set(key, list);
    }
    return BUCKET_ORDER.filter((b) => map.has(b)).map((b) => ({ name: b, sessions: map.get(b)! }));
  }, [filtered]);

  // 键盘导航用的平铺顺序（与渲染顺序一致）
  const flat = useMemo(() => buckets.flatMap((b) => b.sessions), [buckets]);

  // 游标变化时保证可见
  useEffect(() => {
    scrollRef.current
      ?.querySelector(`[data-idx="${cursor}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [cursor]);

  function onListKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, flat.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const s = flat[cursor];
      if (s && s.id !== currentSessionId) onSelect(s);
    }
  }

  // 拖拽调宽：左缘固定，宽度 = 鼠标 x - 面板左缘
  function startResize(e: React.MouseEvent) {
    e.preventDefault();
    const panelLeft = e.currentTarget.parentElement!.getBoundingClientRect().left;
    let cur = width;
    function onMove(ev: MouseEvent) {
      cur = Math.min(WIDTH_MAX, Math.max(WIDTH_MIN, ev.clientX - panelLeft));
      setWidth(cur);
    }
    function onUp() {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      localStorage.setItem(WIDTH_KEY, String(cur));
    }
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }

  let idx = -1;
  return (
    <aside className="ahist" style={{ width }}>
      <div className="ahist__head">
        <Search size={12} strokeWidth={2} />
        <input
          ref={searchInputRef}
          value={query}
          placeholder="搜索对话历史…"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              if (query) setQuery('');
            } else {
              onListKeyDown(e);
            }
          }}
        />
        {query && (
          <button type="button" className="ahist__search-clear" onClick={() => setQuery('')} aria-label="清空搜索">
            <X size={11} strokeWidth={2.2} />
          </button>
        )}
        <span className="ahist__count">{filtered.length}</span>
        <button
          type="button"
          className="ahist__new"
          onClick={onNewChat}
          title="新建会话（当前会话驻留内存，可随时切回）"
          aria-label="新建会话"
        >
          <Plus size={14} strokeWidth={2} />
        </button>
      </div>

      <div className="ahist__scroll" ref={scrollRef} tabIndex={0} onKeyDown={onListKeyDown}>
        {filtered.length === 0 && (
          <div className="ahist__empty">
            <Bot size={22} strokeWidth={1.5} />
            <p>
              {query
                ? '没有匹配的会话'
                : projectPath
                  ? '当前项目还没有会话记录'
                  : '还没有会话记录'}
            </p>
          </div>
        )}

        {buckets.map((b) => (
          <div key={b.name} className="ahist__group">
            <div className="ahist__group-name">{b.name}</div>
            {b.sessions.map((s) => {
              idx += 1;
              const myIdx = idx;
              // 状态标识：运行中 > 待决策 > 打开中（当前 tab 或驻留中）
              const p = st.parked[s.id];
              const isFlat = s.id === st.sessionId;
              const running = isFlat ? st.running : (p?.running ?? false);
              const waiting = hasPendingDecision(isFlat ? st.entries : (p?.entries ?? []));
              const open = isFlat || st.openTabs.includes(s.id);
              return (
                <div
                  key={s.id}
                  data-idx={myIdx}
                  className={`ahist__item${s.id === currentSessionId ? ' is-current' : ''}${running ? ' is-running' : ''}${myIdx === cursor ? ' is-cursor' : ''}`}
                  onClick={() => {
                    setCursor(myIdx);
                    if (s.id !== currentSessionId) onSelect(s);
                  }}
                  onMouseEnter={() => setCursor(myIdx)}
                  role="button"
                  tabIndex={-1}
                  title={s.title}
                >
                  <div className="ahist__item-top">
                    <span className="ahist__item-title">{s.title || 'Agent 会话'}</span>
                    <span className="ahist__item-time">{formatTime(s.updated_at)}</span>
                  </div>
                  <div className="ahist__item-sub">
                    <span
                      className={`ahist__item-proj${s.project_path ? '' : ' is-chat'}`}
                      title={s.project_path || '未绑定项目的纯聊天会话'}
                    >
                      {s.project_name || (s.project_path ? s.project_path.split('/').pop() : '') || '纯聊天'}
                    </span>
                    {running ? (
                      <span className="ahist__item-state is-running">
                        <Loader2 size={10} strokeWidth={2.4} className="ahist__spin" />
                        运行中
                      </span>
                    ) : waiting ? (
                      <span className="ahist__item-state is-waiting">待处理</span>
                    ) : open ? (
                      <span className="ahist__item-state is-open">打开中</span>
                    ) : null}
                  </div>
                  <button
                    type="button"
                    className="ahist__item-del"
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(s.id);
                    }}
                    title="删除会话"
                  >
                    <Trash2 size={12} strokeWidth={1.8} />
                  </button>
                </div>
              );
            })}
          </div>
        ))}
      </div>

      {/* 右缘拖拽手柄 */}
      <div
        className="ahist__resize"
        onMouseDown={startResize}
        role="separator"
        aria-orientation="vertical"
      />
    </aside>
  );
}
