/**
 * 日志页面
 * 移植自 src/routes/logs/+page.svelte
 */
import { useState } from 'react';
import { Link } from 'react-router';
import { ChevronLeft, FileText } from 'lucide-react';
import { useLogsStore, type LogEntry } from '@/stores/logs';
import { formatClockMs } from '@/lib/format';
import './LogsPage.scss';

type FilterLevel = 'all' | 'error' | 'warn' | 'info';

function formatDate(ts: number): string {
  const d = new Date(ts);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

function levelColor(level: LogEntry['level']): string {
  switch (level) {
    case 'error':
      return 'var(--error-text)';
    case 'warn':
      return 'var(--warning-text, #f59e0b)';
    default:
      return 'var(--text-secondary)';
  }
}

function levelLabel(level: LogEntry['level']): string {
  switch (level) {
    case 'error':
      return 'ERR';
    case 'warn':
      return 'WRN';
    default:
      return 'INF';
  }
}

export default function LogsPage() {
  const entries = useLogsStore((s) => s.entries);
  const clear = useLogsStore((s) => s.clear);

  const [filterLevel, setFilterLevel] = useState<FilterLevel>('all');
  const [filterSource, setFilterSource] = useState('');

  const filteredEntries = (() => {
    let result = entries;
    if (filterLevel !== 'all') {
      result = result.filter((e) => e.level === filterLevel);
    }
    if (filterSource.trim()) {
      const q = filterSource.trim().toLowerCase();
      result = result.filter(
        (e) => e.source.toLowerCase().includes(q) || e.message.toLowerCase().includes(q)
      );
    }
    return result;
  })();

  return (
    <div className="logs-page">
      <div className="logs-header">
        <Link to="/settings" className="back-btn" title="返回设置">
          <ChevronLeft size={14} strokeWidth={2.5} />
          返回设置
        </Link>
        <div className="logs-titles">
          <h1>日志</h1>
          <p className="logs-subtitle">应用运行日志，用于诊断问题</p>
        </div>
        <div className="logs-actions">
          <span className="logs-count">{filteredEntries.length} 条记录</span>
          <button
            className="clear-btn"
            onClick={() => clear()}
            disabled={entries.length === 0}
          >
            清空
          </button>
        </div>
      </div>

      <div className="logs-filters">
        <select
          value={filterLevel}
          onChange={(e) => setFilterLevel(e.target.value as FilterLevel)}
          className="filter-select"
        >
          <option value="all">全部级别</option>
          <option value="error">错误</option>
          <option value="warn">警告</option>
          <option value="info">信息</option>
        </select>
        <input
          type="text"
          className="filter-input"
          placeholder="搜索来源/内容..."
          value={filterSource}
          onChange={(e) => setFilterSource(e.target.value)}
        />
      </div>

      {filteredEntries.length === 0 ? (
        <div className="logs-empty">
          <FileText size={48} strokeWidth={1.5} />
          <p>暂无日志记录</p>
        </div>
      ) : (
        <div className="logs-list">
          {filteredEntries.map((entry) => (
            <div
              key={entry.timestamp}
              className={`log-entry${entry.level === 'error' ? ' log-error' : ''}${
                entry.level === 'warn' ? ' log-warn' : ''
              }`}
            >
              <div className="log-meta">
                <span className="log-level" style={{ color: levelColor(entry.level) }}>
                  [{levelLabel(entry.level)}]
                </span>
                <span className="log-source">{entry.source}</span>
                <span className="log-time">
                  {formatDate(entry.timestamp)} {formatClockMs(entry.timestamp)}
                </span>
              </div>
              <div className="log-message">{entry.message}</div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
