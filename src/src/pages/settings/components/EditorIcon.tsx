/**
 * 编辑器图标映射（原 renderEditorIcon 移出页面组件）。
 * lucide-react 不收录品牌 logo：VS Code 用内联 SVG，JetBrains 系用品牌色
 * 字母色块，其余用 lucide 通用图标替代 emoji。
 */
import { Atom, MousePointer2, PenLine, Wind, Wrench } from 'lucide-react';
import type { EditorSetting } from '../settings.types';

export default function EditorIcon({ editor }: { editor: EditorSetting }) {
  if (editor.name === 'VS Code') {
    return (
      <svg width="24" height="24" viewBox="0 0 24 24">
        <path
          fill="#007ACC"
          d="M23.15 2.587L18.21.21a1.49 1.49 0 0 0-1.705.29l-9.46 8.63-4.12-3.128a.999.999 0 0 0-1.144.06L.805 7.347a1 1 0 0 0 .002 1.518L4.368 12 .805 15.136a1 1 0 0 0-.002 1.518l1.004.785a.999.999 0 0 0 1.144.06l4.12-3.128 9.46 8.63a1.49 1.49 0 0 0 1.704.29l4.942-2.377A1.5 1.5 0 0 0 24 19.414V4.586a1.5 1.5 0 0 0-.85-1.999z"
        />
      </svg>
    );
  }
  if (editor.name === 'Zed') {
    return (
      <svg width="24" height="24" viewBox="0 0 24 24">
        <rect width="24" height="24" rx="4" fill="#084B33" />
        <text
          x="12"
          y="17"
          textAnchor="middle"
          fill="white"
          fontSize="14"
          fontWeight="bold"
          fontFamily="sans-serif"
        >
          Z
        </text>
      </svg>
    );
  }
  if (editor.name === 'Cursor') return <MousePointer2 size={22} strokeWidth={1.8} />;
  if (editor.name === 'WebStorm') {
    return (
      <svg width="24" height="24" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="12" fill="#07C3F2" />
        <text
          x="12"
          y="17"
          textAnchor="middle"
          fill="white"
          fontSize="12"
          fontWeight="bold"
          fontFamily="sans-serif"
        >
          WS
        </text>
      </svg>
    );
  }
  if (editor.name === 'IntelliJ IDEA') {
    return (
      <svg width="24" height="24" viewBox="0 0 24 24">
        <rect width="24" height="24" rx="4" fill="#FC3752" />
        <text
          x="12"
          y="17"
          textAnchor="middle"
          fill="white"
          fontSize="10"
          fontWeight="bold"
          fontFamily="sans-serif"
        >
          IJ
        </text>
      </svg>
    );
  }
  if (editor.name === 'Sublime Text') return <PenLine size={22} strokeWidth={1.8} />;
  if (editor.name === 'Atom') return <Atom size={22} strokeWidth={1.8} />;
  if (editor.name === 'Windsurf') return <Wind size={22} strokeWidth={1.8} />;
  return <Wrench size={22} strokeWidth={1.8} />;
}
