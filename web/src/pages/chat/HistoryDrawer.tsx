/**
 * 右侧会话历史抽屉
 * 移植自 src/routes/chat/HistoryDrawer.svelte
 */
import { useState } from 'react';
import { Plus, Search, Trash2, X } from 'lucide-react';
import type { ProviderConfig } from '@/stores/chatToolbar';
import type { ChatSession, ChatSource } from './types';
import { formatDayTimeSec } from '@/lib/format';
import './HistoryDrawer.scss';

interface Props {
  open: boolean;
  sessions: ChatSession[];
  currentSessionId: string | null;
  isSending: boolean;
  providers: ProviderConfig[];
  onClose: () => void;
  onNewChat: () => void;
  onSelect: (s: ChatSession) => void;
  onDelete: (id: string) => void;
}

export default function HistoryDrawer({
  open,
  sessions,
  currentSessionId,
  isSending,
  providers,
  onClose,
  onNewChat,
  onSelect,
  onDelete,
}: Props) {
  // ===== 搜索 =====
  const [searchQuery, setSearchQuery] = useState('');

  const trimmed = searchQuery.trim().toLowerCase();
  const filteredSessions = trimmed
    ? sessions.filter((s) => {
        if (s.title.toLowerCase().includes(trimmed)) return true;
        if (s.messages && s.messages.length > 0) {
          for (const m of s.messages) {
            if (m.content.toLowerCase().includes(trimmed)) return true;
          }
        }
        return false;
      })
    : sessions;

  // ===== 展示辅助 =====
  function sourceIcon(s: ChatSource): string {
    if (s.type === 'model') return '🖥';
    return '⚡';
  }

  function sourceShortLabel(s: ChatSource): string {
    if (s.type === 'model') return `port ${s.port}`;
    const p = providers.find((x) => x.id === s.provider_id);
    return p?.name ?? 'provider';
  }

  function handleDelete(id: string, e: React.MouseEvent) {
    e.stopPropagation();
    onDelete(id);
  }

  return (
    <>
      {open && <div className="drawer-veil" role="presentation" />}
      <aside className={`history-drawer${open ? ' open' : ''}`}>
        <div className="hd-top">
          <span className="hd-title">对话历史</span>
          <button type="button" className="hd-close" onClick={onClose} aria-label="关闭">
            <X size={16} strokeWidth={2.5} />
          </button>
        </div>

        <button type="button" className="hd-new" onClick={onNewChat} disabled={isSending}>
          <Plus size={15} strokeWidth={2.5} />
          新建对话
        </button>

        <div className="hd-search">
          <Search size={13} strokeWidth={2.5} />
          <input
            type="text"
            placeholder="搜索对话历史…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button type="button" className="hd-search-clear" onClick={() => setSearchQuery('')} aria-label="清除搜索">
              ✕
            </button>
          )}
        </div>

        <div className="hd-list">
          {filteredSessions.length === 0 ? (
            <div className="hd-empty">
              <span className="hd-empty-icon">✦</span>
              <p>{searchQuery ? '没有匹配的对话' : '暂无历史记录'}</p>
            </div>
          ) : (
            filteredSessions.map((s) => (
              <div
                key={s.id}
                className={`hd-item${s.id === currentSessionId ? ' active' : ''}`}
                onClick={() => onSelect(s)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => e.key === 'Enter' && onSelect(s)}
              >
                <span className="hd-item-icon">{sourceIcon(s.source)}</span>
                <div className="hd-item-text">
                  <div className="hd-item-title">{s.title}</div>
                  <div className="hd-item-meta">
                    <span className="hd-item-source">{sourceShortLabel(s.source)}</span>
                    <span className="hd-item-dot">·</span>
                    <span className="hd-item-time">{formatDayTimeSec(s.updated_at)}</span>
                  </div>
                </div>
                <button
                  type="button"
                  className="hd-item-del"
                  onClick={(e) => handleDelete(s.id, e)}
                  aria-label="删除"
                  title="删除"
                >
                  <Trash2 size={12} strokeWidth={2.5} />
                </button>
              </div>
            ))
          )}
        </div>
      </aside>
    </>
  );
}
