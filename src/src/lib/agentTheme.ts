/**
 * Agent 对话页专属主题（仅作用于 .agent-root 作用域，不影响其他页面）
 *
 * - 通过 zustand 管理，持久化到 localStorage（key: agent-theme）
 * - AgentPage 读取 id 并在 .agent-root 上设置 data-agent-theme 属性，
 *   SCSS 覆盖层（styles/_agent-themes.scss）在该作用域内覆盖 CSS 变量
 * - 与全局 theme.ts 无关：这里的主题决定「对话区配色」，亮/暗两档由全局
 *   html[data-theme] 决定，二者叠加
 */
import { create } from 'zustand';

export type AgentThemeId = 'default' | 'starfield' | 'paper' | 'holo' | 'den';

export interface AgentThemeMeta {
  id: AgentThemeId;
  label: string;
  desc: string;
}

export const AGENT_THEMES: AgentThemeMeta[] = [
  { id: 'default', label: '默认', desc: '沿用全局靛蓝紫主题' },
  { id: 'starfield', label: '星轨', desc: '深空星野 · 星座连线' },
  { id: 'paper', label: '纸墨', desc: '信笺阅读 · 朱砂点缀' },
  { id: 'holo', label: '全息', desc: '霓虹双色 · 紫青辉光' },
  { id: 'den', label: '猫窝', desc: '暖夜陪伴 · 台灯光晕' },
];

interface AgentThemeState {
  id: AgentThemeId;
  setId: (id: AgentThemeId) => void;
}

function loadSaved(): AgentThemeId {
  const saved = localStorage.getItem('agent-theme') as AgentThemeId | null;
  return AGENT_THEMES.some((t) => t.id === saved) ? (saved as AgentThemeId) : 'default';
}

export const useAgentThemeStore = create<AgentThemeState>((set) => ({
  id: loadSaved(),
  setId: (id) => {
    set({ id });
    localStorage.setItem('agent-theme', id);
  },
}));
