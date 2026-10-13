/**
 * 全局日志存储
 * 用于收集应用各处的日志记录（开发服务器输出、错误等）
 */
import { create } from 'zustand';

export interface LogEntry {
  timestamp: number;
  level: 'info' | 'error' | 'warn';
  source: string; // 例如 'dev-server', 'install', 'app'
  message: string;
}

interface LogsState {
  entries: LogEntry[];
  maxEntries: number;
  add: (entry: Omit<LogEntry, 'timestamp'>) => void;
  info: (source: string, message: string) => void;
  error: (source: string, message: string) => void;
  warn: (source: string, message: string) => void;
  clear: () => void;
}

export const useLogsStore = create<LogsState>((set, get) => ({
  entries: [],
  maxEntries: 500,
  add: (entry) => {
    const fullEntry: LogEntry = { ...entry, timestamp: Date.now() };
    set((state) => ({
      entries: [fullEntry, ...state.entries].slice(0, get().maxEntries),
    }));
  },
  info: (source, message) => get().add({ level: 'info', source, message }),
  error: (source, message) => get().add({ level: 'error', source, message }),
  warn: (source, message) => get().add({ level: 'warn', source, message }),
  clear: () => set({ entries: [] }),
}));

/** 命令式访问（非组件场景） */
export const logs = {
  info: (source: string, message: string) => useLogsStore.getState().info(source, message),
  error: (source: string, message: string) => useLogsStore.getState().error(source, message),
  warn: (source: string, message: string) => useLogsStore.getState().warn(source, message),
};
