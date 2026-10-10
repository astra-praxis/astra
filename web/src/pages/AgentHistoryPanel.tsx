/**
 * Agent 会话历史面板（常驻右侧）
 *
 * - 按 project_path 分组：绑定项目的会话按项目名归组，纯聊天归入「纯聊天」组
 * - 组内按 updated_at 倒序（后端已排好，保持顺序）；组间按组内最新会话时间倒序
 * - 点击条目 → onSelect 回放条目流；悬停出现删除按钮
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Bot, History, MessageSquare, Search, Trash2, X } from 'lucide-react';
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
  running: boolean;
  onSelect: (s: AgentSessionMeta) => void;
  onDelete: (id: string) => void;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  if (sameDay) return `${hh}:${mm}`;
  return `${d.getMonth() + 1}/${d.getDate()} ${hh}:${mm}`;
}

export default function AgentHistoryPanel({
  sessions,
  currentSessionId,
  running,
  onSelect,
  onDelete,
}: Props) {
  // 搜索：默认隐藏，点放大镜展开；按标题 / 项目名过滤（大小写不敏感）
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState('');
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus();
  }, [searchOpen]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter(
      (s) =>
        s.title.toLowerCase().includes(q) || s.project_name.toLowerCase().includes(q),
    );
  }, [sessions, query]);

  function closeSearch() {
    setSearchOpen(false);
    setQuery('');
  }

  // 按项目分组；组间按组内最新 updated_at 倒序，组内保持后端排好的倒序
  const groups = useMemo(() => {
    const map = new Map<string, AgentSessionMeta[]>();
    for (const s of filtered) {
      const key = s.project_path || '__chat__';
      const list = map.get(key) ?? [];
      list.push(s);
      map.set(key, list);
    }
    return [...map.entries()]
      .map(([key, list]) => ({
        key,
        name: key === '__chat__' ? '纯聊天' : list[0].project_name || key.split('/').pop() || key,
        latest: Math.max(...list.map((s) => s.updated_at)),
        sessions: list,
      }))
      .sort((a, b) => b.latest - a.latest);
  }, [filtered]);

  return (
    <aside className="ahist">
      <div className="ahist__head">
        <History size={14} strokeWidth={1.8} />
        <span>对话历史</span>
        <button
          type="button"
          className={`ahist__search-btn${searchOpen ? ' is-active' : ''}`}
          onClick={() => (searchOpen ? closeSearch() : setSearchOpen(true))}
          title={searchOpen ? '关闭搜索' : '搜索会话'}
          aria-label="搜索会话"
        >
          <Search size={13} strokeWidth={2} />
        </button>
        <span className="ahist__count">{filtered.length}</span>
      </div>

      {searchOpen && (
        <div className="ahist__search">
          <Search size={12} strokeWidth={2} />
          <input
            ref={searchInputRef}
            value={query}
            placeholder="搜索标题 / 项目名…"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                if (query) setQuery('');
                else closeSearch();
              }
            }}
          />
          {query && (
            <button type="button" className="ahist__search-clear" onClick={() => setQuery('')} aria-label="清空搜索">
              <X size={11} strokeWidth={2.2} />
            </button>
          )}
        </div>
      )}

      <div className="ahist__scroll">
        {filtered.length === 0 && (
          <div className="ahist__empty">
            <Bot size={22} strokeWidth={1.5} />
            <p>{query ? '没有匹配的会话' : '还没有会话记录'}</p>
          </div>
        )}

        {groups.map((g) => (
          <div key={g.key} className="ahist__group">
            <div className="ahist__group-name" title={g.key === '__chat__' ? undefined : g.key}>
              <MessageSquare size={11} strokeWidth={2} />
              {g.name}
            </div>
            {g.sessions.map((s) => (
              <div
                key={s.id}
                className={`ahist__item${s.id === currentSessionId ? ' is-current' : ''}`}
                onClick={() => !running && s.id !== currentSessionId && onSelect(s)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !running && s.id !== currentSessionId) onSelect(s);
                }}
                title={running ? '运行中不能切换会话' : s.title}
              >
                <div className="ahist__item-top">
                  <span className="ahist__item-title">{s.title || 'Agent 会话'}</span>
                  <span className="ahist__item-time">{formatTime(s.updated_at)}</span>
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
            ))}
          </div>
        ))}
      </div>
    </aside>
  );
}
