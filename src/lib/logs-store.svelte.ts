/**
 * 全局日志存储
 * 用于收集应用各处的日志记录（开发服务器输出、错误等）
 */

export interface LogEntry {
  timestamp: number;
  level: 'info' | 'error' | 'warn';
  source: string; // 例如 'dev-server', 'install', 'app'
  message: string;
}

class LogsStore {
  entries = $state<LogEntry[]>([]);
  maxEntries = 500;

  add(entry: Omit<LogEntry, 'timestamp'>) {
    const fullEntry: LogEntry = {
      ...entry,
      timestamp: Date.now()
    };
    this.entries = [fullEntry, ...this.entries].slice(0, this.maxEntries);
  }

  info(source: string, message: string) {
    this.add({ level: 'info', source, message });
  }

  error(source: string, message: string) {
    this.add({ level: 'error', source, message });
  }

  warn(source: string, message: string) {
    this.add({ level: 'warn', source, message });
  }

  clear() {
    this.entries = [];
  }
}

export const logs = new LogsStore();