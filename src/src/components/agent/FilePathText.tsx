/**
 * 文件路径链接化：把文本中的路径片段变成可点击链接，点击后在 Agent 页
 * 右侧文件预览面板（FilePreviewPanel）打开。
 *
 * 候选识别（正则，宽匹配）：
 * - 绝对路径：POSIX `/Users/...`、Windows `C:\...`
 * - 相对路径：`src/main.rs`、`./docs/a.md`、`../README.md` 等（含分隔符才候选）
 *
 * 可点击性（磁盘存在性验证，严判定）：候选先经 Rust `path_exists` 校验，
 * 存在才升级为可点击片段，否则保持普通文本——正则的误报由存在性检查兜底。
 * 校验结果按「基准目录 + 路径」缓存（模块级 Promise Map），跨实例去重。
 *
 * 用于对话记录场景：工具卡参数摘要（argsSummary）、工具结果文本、
 * 粘贴溢出提示等。空白与换行原样保留（可放在 <pre> 内使用）。
 *
 * 点击时 stopPropagation：工具卡头部整体是展开/收起按钮，路径点击不应触发折叠。
 */
import { useEffect, useState, type MouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useFilePreviewStore } from '@/stores/filePreview';
import { useWorkspaceStore } from '@/stores/workspace';

// 绝对路径：POSIX 以 / 开头，Windows 盘符 C:\ 开头；排除 URL、空白与常见包裹符号
const PATH_RE = /(\/[^\s'"`[\]<>(){},;:=|?*\\]+|[A-Za-z]:\\[^\s'"`[\]<>(){},;:=|?*/]+)\s?/g;

// 相对路径候选：至少「目录/文件」两级（含 / 或 \ 分隔符），支持 ./ 与 ../ 前缀；
// 宽匹配，误报（and/or、日期等）由存在性验证过滤
const REL_PATH_RE =
  /(?:\.{1,2}[\\/])?(?:[\w\u4e00-\u9fff@.%+-]+[\\/])+[\w\u4e00-\u9fff@.%+-]+/g;

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

/** 存在性验证缓存：key = baseDir\0candidate，跨实例去重避免重复 IPC */
const existsCache = new Map<string, Promise<boolean>>();
const CACHE_LIMIT = 500;
/** 单段文本的候选上限，防病态长文本产生大量 IPC */
const CANDIDATE_LIMIT = 50;

/**
 * 路径存在性验证（文件或目录均可）。相对路径以当前工作空间为基准解析；
 * 解析失败或不存在返回 false。结果缓存，同一「基准 + 路径」只发一次 IPC。
 */
export function verifyPathExists(path: string): Promise<boolean> {
  const baseDir = useWorkspaceStore.getState().activePath || null;
  const key = `${baseDir ?? ''}\u0000${path}`;
  let p = existsCache.get(key);
  if (!p) {
    if (existsCache.size >= CACHE_LIMIT) existsCache.clear();
    p = invoke<boolean>('path_exists', { path, baseDir }).catch(() => false);
    existsCache.set(key, p);
  }
  return p;
}

interface Segment {
  type: 'text' | 'path';
  value: string;
}

interface PathMatch {
  start: number;
  end: number;
  path: string;
  tail: string;
}

/** 提取路径候选段：绝对路径优先，相对路径不与绝对路径重叠且不被更长路径包含 */
function extractSegments(text: string): Segment[] {
  const matches: PathMatch[] = [];

  // 第一遍：绝对路径（沿用原判定）
  const absRanges: Array<[number, number]> = [];
  for (const m of text.matchAll(PATH_RE)) {
    const raw = m[1];
    const start = m.index ?? 0;
    // 前一段是 URL 的一部分（前面紧跟 scheme:）时跳过，避免把 https:// 的路径段当文件
    const before = text.slice(Math.max(0, start - 8), start);
    const { path, tail } = trimTrailingPunctuation(raw);
    if (!isAbsoluteFilePath(path)) continue;
    if (before.endsWith('://')) continue;
    absRanges.push([start, start + raw.length]);
    matches.push({ start, end: start + raw.length, path, tail });
  }

  // 第二遍：相对路径候选——与绝对路径区间重叠、紧随分隔符（长路径中段）或
  // 紧跟 scheme: 的跳过；候选本身再走存在性验证
  for (const m of text.matchAll(REL_PATH_RE)) {
    const raw = m[0];
    const start = m.index ?? 0;
    const end = start + raw.length;
    if (absRanges.some(([s, e]) => start < e && end > s)) continue;
    if (start > 0 && /[\\/]/.test(text[start - 1])) continue; // 更长路径的中段
    const before = text.slice(Math.max(0, start - 8), start);
    if (before.endsWith('://')) continue; // URL 的路径部分
    const { path, tail } = trimTrailingPunctuation(raw);
    matches.push({ start, end, path, tail });
  }

  // 按出现位置排序（稳定排序：同位置绝对路径先生成先保留），丢弃重叠者
  matches.sort((a, b) => a.start - b.start);
  const kept: PathMatch[] = [];
  let lastEnd = -1;
  for (const m of matches) {
    if (m.start < lastEnd) continue;
    kept.push(m);
    lastEnd = m.end;
  }

  const segments: Segment[] = [];
  let last = 0;
  for (const m of kept) {
    if (m.start > last) segments.push({ type: 'text', value: text.slice(last, m.start) });
    segments.push({ type: 'path', value: m.path });
    if (m.tail) segments.push({ type: 'text', value: m.tail });
    last = m.end;
  }
  if (last < text.length) segments.push({ type: 'text', value: text.slice(last) });
  return segments;
}

interface FilePathTextProps {
  text: string;
}

export default function FilePathText({ text }: FilePathTextProps) {
  const segments = extractSegments(text);
  const [verified, setVerified] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let cancelled = false;
    const candidates = [...new Set(segments.filter((s) => s.type === 'path').map((s) => s.value))]
      .filter((p) => !(p in verified))
      .slice(0, CANDIDATE_LIMIT);
    for (const p of candidates) {
      verifyPathExists(p).then((ok) => {
        if (cancelled) return;
        // 不存在的也记入，避免同文本重复校验；存在的升级为可点击
        setVerified((v) => (p in v ? v : { ...v, [p]: ok }));
      });
    }
    return () => {
      cancelled = true;
    };
    // verified 不进依赖：校验完成触发重渲染后无需重跑
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

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
        seg.type === 'text' || !verified[seg.value] ? (
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
