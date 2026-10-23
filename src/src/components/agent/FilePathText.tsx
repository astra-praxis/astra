/**
 * 文件路径链接化：把文本中的绝对路径（POSIX `/Users/...`、Windows `C:\...`）
 * 变成可点击片段，点击后在 Agent 页右侧文件预览面板（FilePreviewPanel）打开。
 *
 * 用于对话记录场景：工具卡参数摘要（argsSummary）、工具结果文本、
 * 粘贴溢出提示等。空白与换行原样保留（可放在 <pre> 内使用）。
 *
 * 点击时 stopPropagation：工具卡头部整体是展开/收起按钮，路径点击不应触发折叠。
 * 相对路径不可靠（误报率高），只处理绝对路径。
 */
import type { MouseEvent } from 'react';
import { useFilePreviewStore } from '@/stores/filePreview';

// 绝对路径：POSIX 以 / 开头，Windows 盘符 C:\ 开头；排除 URL、空白与常见包裹符号
const PATH_RE = /(\/[^\s'"`[\]<>(){},;:=|?*\\]+|[A-Za-z]:\\[^\s'"`[\]<>(){},;:=|?*/]+)\s?/g;

/** 剥离句尾标点（中英文），路径本身以标点结尾的情形极罕见 */
function trimTrailingPunctuation(s: string): { path: string; tail: string } {
  const m = s.match(/[.,;:!?)\]】」』"']*[。，、；：！？）】」』"']*$/);
  const cut = m ? m[0].length : 0;
  return { path: cut ? s.slice(0, -cut) : s, tail: cut ? s.slice(-cut) : '' };
}

/** 是否是值得点击的绝对路径：长度足够、不止根、排除协议地址（https:// 等） */
export function isAbsoluteFilePath(s: string): boolean {
  if (s.length < 4) return false;
  if (/^[a-z][a-z\d+.-]*:/i.test(s)) return false; // scheme: http(s) / action: / mailto:
  if (s.startsWith('//')) return false; // 协议相对地址
  const slashes = s.includes('\\') ? (s.match(/\\/g)?.length ?? 0) : (s.match(/\//g)?.length ?? 0);
  return slashes >= 2; // 至少「/目录/文件」两级，排除孤立的 / 或 C:\
}

export function openFilePath(path: string) {
  useFilePreviewStore.getState().openFile(path);
}

interface FilePathTextProps {
  text: string;
}

export default function FilePathText({ text }: FilePathTextProps) {
  const segments: Array<{ type: 'text' | 'path'; value: string }> = [];
  let last = 0;
  for (const m of text.matchAll(PATH_RE)) {
    const raw = m[1];
    const start = m.index ?? 0;
    // 前一段是 URL 的一部分（前面紧跟 scheme:）时跳过，避免把 https:// 的路径段当文件
    const before = text.slice(Math.max(0, start - 8), start);
    const { path, tail } = trimTrailingPunctuation(raw);
    if (!isAbsoluteFilePath(path)) continue;
    if (before.endsWith('://')) continue;
    if (start > last) segments.push({ type: 'text', value: text.slice(last, start) });
    segments.push({ type: 'path', value: path });
    if (tail) segments.push({ type: 'text', value: tail });
    last = start + raw.length;
  }
  if (last < text.length) segments.push({ type: 'text', value: text.slice(last) });

  // 无路径：原样返回，零开销
  if (segments.length === 1 && segments[0].type === 'text') return <>{text}</>;

  function onPathClick(e: MouseEvent, path: string) {
    e.stopPropagation();
    e.preventDefault();
    openFilePath(path);
  }

  return (
    <>
      {segments.map((seg, i) =>
        seg.type === 'text' ? (
          <span key={i}>{seg.value}</span>
        ) : (
          <span
            key={i}
            className="file-path-link"
            title={`预览文件：${seg.value}`}
            onClick={(e) => onPathClick(e, seg.value)}
          >
            {seg.value}
          </span>
        ),
      )}
    </>
  );
}
