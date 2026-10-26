/**
 * Agent 对话主题迷你预览（设置页 + 底栏切换器共用）
 *
 * 用固定色值（不随当前主题变化）绘制每套主题的「对话区缩略图」，
 * 让用户在选择时就能看到目标主题的实际观感。
 * 每套预览 = 背景色 + 专属装饰（星轨星座 / 纸墨信纸 / 全息网格 / 猫窝灯晕）+ 气泡示意。
 */
import { useId } from 'react';
import type { AgentThemeId } from '@/lib/agentTheme';

interface Props {
  theme: AgentThemeId;
  className?: string;
  /** 渲染宽度（px），高度按 16:10 等比 */
  width?: number;
}

interface ThemeArt {
  bg: string;
  /** 装饰层（可选）：星座 / 信纸 / 网格 / 灯晕 */
  decor: 'starfield' | 'paper' | 'holo' | 'den' | 'none';
  /** 用户气泡（深色卡） */
  bubble: string;
  bubbleBorder: string;
  /** 助手文本行 */
  line: string;
  /** 品牌点缀 */
  accent: string;
  accentSoft: string;
  /** 工具卡造型：圆角 / 是否左侧竖线 / 是否辉光 */
  toolRadius: number;
  toolLeftBar?: string;
  toolGlow?: boolean;
  toolBorder?: string;
}

const ARTS: Record<AgentThemeId, ThemeArt> = {
  default: {
    bg: '#f3f4f8',
    decor: 'none',
    bubble: '#0d1120',
    bubbleBorder: 'rgba(255,255,255,0.08)',
    line: '#dfe3ee',
    accent: '#6366f1',
    accentSoft: 'rgba(99,102,241,0.12)',
    toolRadius: 12,
  },
  starfield: {
    bg: '#0b0e1f',
    decor: 'starfield',
    bubble: '#10152c',
    bubbleBorder: 'rgba(152,162,242,0.35)',
    line: '#333b63',
    accent: '#a5abee',
    accentSoft: 'rgba(152,162,242,0.14)',
    toolRadius: 8,
    toolGlow: true,
  },
  paper: {
    bg: '#f6f5f1',
    decor: 'paper',
    bubble: '#26303f',
    bubbleBorder: 'rgba(38,48,63,0.45)',
    line: '#c9ced6',
    accent: '#3d5a80',
    accentSoft: 'rgba(61,90,128,0.12)',
    toolRadius: 6,
    toolLeftBar: '#c0392b',
    toolBorder: 'dashed',
  },
  holo: {
    bg: '#080a14',
    decor: 'holo',
    bubble: '#171b31',
    bubbleBorder: 'rgba(34,211,238,0.4)',
    line: '#2e3350',
    accent: '#bda9f8',
    accentSoft: 'rgba(167,139,250,0.16)',
    toolRadius: 14,
    toolGlow: true,
  },
  den: {
    bg: '#1a1612',
    decor: 'den',
    bubble: '#33291c',
    bubbleBorder: 'rgba(232,162,92,0.35)',
    line: '#4a3e2c',
    accent: '#e8a25c',
    accentSoft: 'rgba(232,162,92,0.16)',
    toolRadius: 16,
  },
};

function Decor({ kind, accent, accentSoft }: { kind: ThemeArt['decor']; accent: string; accentSoft: string }) {
  switch (kind) {
    case 'starfield':
      return (
        <>
          <g fill="none" stroke={accent} strokeOpacity="0.5">
            <path d="M20 30 L55 18 L85 38 L120 24" />
            <path d="M150 70 L170 62" />
            <path d="M30 92 L65 84 L95 100" />
          </g>
          <g fill="#ffd9a0" fillOpacity="0.8">
            <circle cx="20" cy="30" r="1.6" />
            <circle cx="55" cy="18" r="2" />
            <circle cx="85" cy="38" r="1.4" />
            <circle cx="120" cy="24" r="1.8" />
            <circle cx="150" cy="70" r="1.6" />
            <circle cx="170" cy="62" r="2" />
            <circle cx="30" cy="92" r="1.6" />
            <circle cx="65" cy="84" r="1.8" />
            <circle cx="95" cy="100" r="1.4" />
          </g>
        </>
      );
    case 'paper':
      return (
        <g stroke="#c4cad4" strokeOpacity="0.55">
          <line x1="8" y1="46" x2="172" y2="46" />
          <line x1="8" y1="60" x2="172" y2="60" />
          <line x1="8" y1="74" x2="172" y2="74" />
          <line x1="8" y1="88" x2="172" y2="88" />
          <line x1="18" y1="10" x2="18" y2="110" stroke="#c0392b" strokeOpacity="0.45" />
        </g>
      );
    case 'holo':
      return (
        <>
          <g stroke={accent} strokeOpacity="0.45">
            <path d="M90 0 L20 110" />
            <path d="M90 0 L60 110" />
            <path d="M90 0 L90 110" />
            <path d="M90 0 L120 110" />
            <path d="M90 0 L160 110" />
          </g>
          <g stroke="#22d3ee" strokeOpacity="0.3">
            <line x1="0" y1="40" x2="180" y2="40" />
            <line x1="0" y1="75" x2="180" y2="75" />
          </g>
          <circle cx="90" cy="55" r="34" fill={accentSoft} />
        </>
      );
    case 'den':
    case 'none':
      return null;
  }
}

export default function AgentThemePreview({ theme, className = '', width = 160 }: Props) {
  const art = ARTS[theme];
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const glowId = `denGlow-${uid}`;
  const height = Math.round((width * 10) / 16);
  return (
    <svg
      className={className}
      width={width}
      height={height}
      viewBox="0 0 180 112"
      role="img"
      aria-label={`${theme} 主题预览`}
    >
      {/* 背景 + 装饰 */}
      <rect width="180" height="112" rx="8" fill={art.bg} />
      <Decor kind={art.decor} accent={art.accent} accentSoft={art.accentSoft} />
      {art.decor === 'den' && (
        <>
          <defs>
            <radialGradient id={glowId} cx="50%" cy="0%" r="85%">
              <stop offset="0%" stopColor="#f5b46a" stopOpacity="0.5" />
              <stop offset="70%" stopColor="#f5b46a" stopOpacity="0.1" />
              <stop offset="100%" stopColor="#f5b46a" stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width="180" height="112" rx="8" fill={`url(#${glowId})`} />
        </>
      )}

      {/* 顶部「标签栏」示意 */}
      <rect x="10" y="10" width="160" height="12" rx="4" fill={art.accentSoft} />

      {/* 用户气泡（右侧）+ 助手文本行（左侧） */}
      <rect x="96" y="30" width="74" height="26" rx="7" fill={art.bubble} stroke={art.bubbleBorder} />
      <rect x="16" y="62" width="96" height="5" rx="2.5" fill={art.line} />
      <rect x="16" y="74" width="72" height="5" rx="2.5" fill={art.line} />
      <rect x="16" y="86" width="84" height="5" rx="2.5" fill={art.line} />

      {/* 迷你工具卡：圆角 / 朱砂线 / 辉光随主题不同 */}
      <g>
        {art.toolGlow && (
          <rect x="16" y="32" width="64" height="14" rx={art.toolRadius} fill={art.accentSoft} />
        )}
        {art.toolLeftBar && <rect x="16" y="32" width="3" height="14" rx="1.5" fill={art.toolLeftBar} />}
        <rect
          x="16"
          y="32"
          width="64"
          height="14"
          rx={art.toolRadius}
          fill={art.bubble}
          stroke={art.bubbleBorder}
          strokeDasharray={art.toolBorder === 'dashed' ? '3 2' : undefined}
        />
        <rect x="23" y="37" width="22" height="4" rx="2" fill={art.accent} />
        <circle cx="20" cy="39" r="1.6" fill={art.accent} />
      </g>

      {/* 品牌点缀（发送按钮） */}
      <rect x="156" y="98" width="14" height="14" rx="4" fill={art.accent} />
    </svg>
  );
}
