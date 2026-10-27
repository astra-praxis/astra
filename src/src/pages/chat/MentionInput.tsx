/**
 * Agent 输入框（contenteditable chip 编辑器）
 *
 * 替代「textarea + 高亮镜像层」方案：已选中的 /技能 与 @文件 渲染为真正的
 * 内联 tag（图标 + 彩色文字 + 删除按钮），Zed/Slack 同款交互。
 *
 * 边界契约：对外只说「带 @path / /技能 token 的纯文本」——
 * - chip 序列化时还原为 token 文本，发送 / 队列 / 气泡解析（parseUserRefs）零改动
 * - DOM 优先：用户输入直接改 DOM 后序列化上报；仅当外部值与序列化结果不一致时
 *   才重建 DOM（editQueued 回填 / 选中 chip 回声），保光标、保 IME
 * - IME 组合期间跳过同步与 @/ 检测，避免打断选字；发送 Enter 拦截由页面
 *   isSendEnter 负责（keyCode 229 / isComposing 双保险）
 */
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { MENTION_DETECT_RE } from '@/hooks/useMention';
import { SLASH_DETECT_RE, type UserSkillLite } from '@/hooks/useSlash';
import { CAT_DETECT_RE, filterCatCommands, type CatCommand } from '@/hooks/useCatCommand';
import FileMentionPopup from './FileMentionPopup';
import SlashPopup from './SlashPopup';
import CatCommandPopup from './CatCommandPopup';
import './MentionInput.scss';

export interface MentionInputHandle {
  /** 聚焦并把光标移到末尾（editQueued 回填用） */
  focusEnd: () => void;
  /** 光标处插入文本（底栏 @ 按钮），触发 input 事件走统一的 onChange/检测链路 */
  insertText: (t: string) => void;
}

interface Props {
  value: string;
  onChange: (v: string) => void;
  /** @ 可引用的项目文件相对路径 */
  files: string[];
  /** / 可调用的技能 */
  skills: UserSkillLite[];
  placeholder: string;
  /** 页面处理发送与队列 */
  onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => void;
  /** 超长粘贴拦截（SpillPaste）：返回 true 表示内容已落盘、不必插入编辑器；
   * false / 未传时照常内联插入。阈值 SPILL_PASTE_CHARS */
  onSpillPaste?: (text: string) => Promise<boolean>;
  /** 系统文件粘贴导入：剪贴板含文件（Finder/资源管理器复制的真实文件）时调用，
   * 由页面复制到 .astra/pastes/ 并返回实际导入列表；空列表表示无可导入文件
   * （截图等图片数据 / 读取失败），降级为普通文本粘贴 */
  onPasteFiles?: () => Promise<{ path: string; name: string; size: number }[]>;
}

/** 粘贴落盘阈值：超过该字符数的粘贴内容改写为文件，避免撑爆输入框与上下文 */
export const SPILL_PASTE_CHARS = 2_000;

// ===== token 解析（与气泡侧 parseUserRefs 同规则） =====

interface Token {
  text: string;
  chip: 'skill' | 'file' | null;
}

/** /技能名 仅前缀且须命中已知技能；@文件 任意位置且须命中项目文件，未命中原样保留 */
function tokenize(value: string, skills: UserSkillLite[], fileSet: Set<string>): Token[] {
  const tokens: Token[] = [];
  let plain = '';
  const flush = () => {
    if (plain) {
      tokens.push({ text: plain, chip: null });
      plain = '';
    }
  };

  let rest = value;
  const sm = rest.match(/^\/([\w\u4e00-\u9fff.-]+)/);
  if (sm && skills.some((s) => s.name === sm[1])) {
    tokens.push({ text: sm[0], chip: 'skill' });
    rest = rest.slice(sm[0].length);
  }

  while (rest) {
    const m = rest.match(/@([\w\u4e00-\u9fff./-]+)/);
    if (!m) {
      plain += rest;
      break;
    }
    plain += rest.slice(0, m.index);
    const name = m[1];
    const bare = name.replace(/\.+$/, '');
    const hit = fileSet.has(name) ? name : fileSet.has(bare) ? bare : null;
    if (hit) {
      flush();
      tokens.push({ text: `@${hit}`, chip: 'file' });
      rest = rest.slice((m.index ?? 0) + 1 + hit.length);
    } else {
      plain += m[0];
      rest = rest.slice((m.index ?? 0) + m[0].length);
    }
  }
  flush();
  return tokens;
}

// ===== DOM <-> 纯文本互转 =====

const CHIP_CLASS = 'minput__chip';



// 零宽空格光标锚点：chip 是 contenteditable=false，紧贴其后没有真实文本时
// WebKit 无法可靠落点/删除（光标被归一化、原生删除拒绝作用于不可编辑节点）。
// 锚点在序列化与偏移换算中长度记 0，对值与光标计算无影响。
const ZWSP = '\u200B';
const ZWSP_RE = /\u200B/g;

function isZwspText(n: Node | null): n is Text {
  return !!n && n.nodeType === Node.TEXT_NODE && n.textContent === ZWSP;
}

function isChipEl(n: Node | null): n is HTMLElement {
  return !!n && n.nodeType === Node.ELEMENT_NODE && (n as HTMLElement).classList.contains(CHIP_CLASS);
}

/** 节点的纯文本长度（chip 记作 token 长度、br 记 1，保证与序列化值一一对应） */

function nodeLen(n: Node): number {

  if (n.nodeType === Node.TEXT_NODE) return (n.textContent ?? '').replace(ZWSP_RE, '').length;
  if (n.nodeType !== Node.ELEMENT_NODE) return 0;
  const el = n as Element;
  if (el.tagName === 'BR') return 1;
  if (isChipEl(el)) return (el.dataset.token ?? '').length;
  let sum = 0;
  el.childNodes.forEach((c) => {
    sum += nodeLen(c);
  });
  return sum;
}

function serializeEl(root: HTMLElement): string {
  let out = '';
  root.childNodes.forEach((n) => {
    out += nodeToText(n);
  });
  return out;
}

function nodeToText(n: Node): string {

  if (n.nodeType === Node.TEXT_NODE) return (n.textContent ?? '').replace(ZWSP_RE, '');
  if (n.nodeType !== Node.ELEMENT_NODE) return '';
  const el = n as Element;
  if (el.tagName === 'BR') return '\n';
  if (isChipEl(el)) return el.dataset.token ?? '';
  let out = '';
  el.childNodes.forEach((c) => {
    out += nodeToText(c);
  });
  return out;
}

/** 光标位置 -> 纯文本偏移量（@/ 检测与 pick 的公共前提） */
function offsetIn(root: HTMLElement, container: Node, offset: number): number {
  if (container === root) {
    let sum = 0;
    for (let i = 0; i < offset && i < root.childNodes.length; i++) sum += nodeLen(root.childNodes[i]);
    return sum;
  }
  if (container.nodeType === Node.TEXT_NODE) {

    let sum = 0;

    let cur: Node | null = container;

    while (cur && cur !== root) {

      let sib = cur.previousSibling;

      while (sib) {

        sum += nodeLen(sib);

        sib = sib.previousSibling;

      }

      cur = cur.parentNode;

    }

    // 文本内偏移按「光标前有效字符数」计（剔除 ZWSP，锚点不占值空间）
    return sum + (container.textContent ?? '').slice(0, offset).replace(ZWSP_RE, '').length;

  }
  const parent = container.parentNode;
  if (!parent) return 0;
  return offsetIn(root, parent, Array.prototype.indexOf.call(parent.childNodes, container) + offset);
}

function computeCaret(root: HTMLElement): number {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return 0;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.startContainer)) return 0;
  return offsetIn(root, range.startContainer, range.startOffset);
}

/** 纯文本偏移量 -> 光标位置（target 落在 chip 区间内时放到 chip 之后） */
function placeCaret(root: HTMLElement, target: number): void {
  const sel = window.getSelection();
  if (!sel) return;
  let acc = 0;
  let found: { node: Node; offset: number } | null = null;
  const walk = (n: Node): void => {
    if (found) return;
    if (n.nodeType === Node.TEXT_NODE) {
      const raw = n.textContent ?? '';
      const len = raw.replace(ZWSP_RE, '').length;
      // 零宽锚点（有效长度 0）：target 恰在边界时落进锚点内部，让光标
      // 始终处于真实文本节点中，避开 WebKit 对贴着不可编辑元素的光标归一化
      if (raw === ZWSP && acc >= target) {
        found = { node: n, offset: 0 };
        return;
      }
      if (acc + len >= target) {
        // 找第 target-acc 个非 ZWSP 字符的原始偏移
        let k = target - acc;
        for (let i = 0; i < raw.length; i++) {
          if (raw[i] !== ZWSP) {
            if (k === 0) {
              found = { node: n, offset: i };
              return;
            }
            k--;
          }
        }
        found = { node: n, offset: raw.length };
        return;
      }
      acc += len;
      return;
    }
    if (n.nodeType !== Node.ELEMENT_NODE) return;
    const el = n as Element;
    if (el.tagName === 'BR') {
      acc += 1;
      return;
    }
    if (isChipEl(el)) {
      const len = (el.dataset.token ?? '').length;
      if (target < acc + len && el.parentNode) {
        found = {
          node: el.parentNode,
          offset: Array.prototype.indexOf.call(el.parentNode.childNodes, el) + 1,
        };
        return;
      }
      acc += len;
      return;
    }
    el.childNodes.forEach((c) => walk(c));
  };
  walk(root);
  const pos = found ?? { node: root, offset: root.childNodes.length };
  const range = document.createRange();
  range.setStart(pos.node, pos.offset);
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

/** 由值字符串重建 DOM（chip-ify 已命中的 token），外部值同步的唯一入口 */
function rebuildDOM(root: HTMLElement, value: string, skills: UserSkillLite[], fileSet: Set<string>): void {
  root.replaceChildren();
  const segs = tokenize(value, skills, fileSet);
  segs.forEach((seg, i) => {
    if (seg.chip) {
      root.appendChild(buildChip(seg.chip, seg.text));
      // chip 后没有真实文本段时补零宽锚点，保证光标有可落的文本节点
      const next = segs[i + 1];
      if (!next || next.chip) root.appendChild(document.createTextNode(ZWSP));
    } else {
      root.appendChild(document.createTextNode(seg.text));
    }
  });
}

const CHIP_ICONS: Record<'skill' | 'file', string> = {
  // lucide sparkles / file-text 的 path（DOM 直建无法走 React 组件，内联同形 SVG）
  skill:
    '<svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/><path d="M4 17v2"/><path d="M5 18H3"/></svg>',
  file:
    '<svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/></svg>',
};

const CHIP_X_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';

function buildChip(kind: 'skill' | 'file', token: string): HTMLElement {
  const chip = document.createElement('span');
  chip.className = `${CHIP_CLASS} ${CHIP_CLASS}--${kind}`;
  chip.contentEditable = 'false';
  chip.dataset.token = token;
  const label = token.replace(/^[@/]/, '');
  chip.innerHTML = `${CHIP_ICONS[kind]}<span class="${CHIP_CLASS}-label">${label}</span><span class="${CHIP_CLASS}-x" role="button" aria-label="移除">${CHIP_X_SVG}</span>`;
  return chip;
}

// ===== 组件 =====

const MentionInput = forwardRef<MentionInputHandle, Props>(function MentionInput(
  { value, onChange, files, skills, placeholder, onKeyDown, onSpillPaste, onPasteFiles },
  ref,
) {
  const editorRef = useRef<HTMLDivElement | null>(null);
  // 最近一次编辑器内光标的纯文本偏移量（弹层点击会使 window.getSelection 移出编辑器，
  // pick 必须用缓存值而不是实时读取）
  const caretRef = useRef(0);
  const composingRef = useRef(false);
  /** pick 路径：onChange 回声触发重建时，光标要落到的位置（一次有效） */
  const pendingCaretRef = useRef<number | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const skillsRef = useRef(skills);
  skillsRef.current = skills;
  const fileSet = useMemo(() => new Set(files), [files]);
  const fileSetRef = useRef(fileSet);
  fileSetRef.current = fileSet;

  const [mentionPop, setMentionPop] = useState({ active: false, query: '', sel: 0 });
  const [slashPop, setSlashPop] = useState({ active: false, query: '', sel: 0 });
  const [catPop, setCatPop] = useState({ active: false, query: '', sel: 0 });
  const closePops = useCallback(() => {
    setMentionPop((p) => (p.active ? { ...p, active: false } : p));
    setSlashPop((p) => (p.active ? { ...p, active: false } : p));
    setCatPop((p) => (p.active ? { ...p, active: false } : p));
  }, []);

  // 弹层候选列表（与原 useMention/useSlash 过滤规则一致）
  const mq = mentionPop.query.toLowerCase();
  const fileHits = useMemo(
    () => (mq ? files.filter((f) => f.toLowerCase().includes(mq)).slice(0, 20) : files.slice(0, 10)),
    [files, mq],
  );
  const sq = slashPop.query.toLowerCase();
  const skillHits = useMemo(
    () =>
      sq
        ? skills
            .filter((s) => s.name.toLowerCase().includes(sq) || s.description.toLowerCase().includes(sq))
            .slice(0, 20)
        : skills.slice(0, 10),
    [skills, sq],
  );
  // & 猫猫命令候选（& 空串时展示全部动作）
  const cq = catPop.query;
  const catHits = useMemo(() => filterCatCommands(cq), [cq]);

  /** @/&/ 检测：光标前缀命中哪种 token 就开哪个弹层（三者符号不同，互斥，每分支显式关另两个） */
  const runDetect = useCallback((value: string, offset: number) => {
    const before = value.slice(0, offset);
    const mm = before.match(MENTION_DETECT_RE);
    const sm = before.match(SLASH_DETECT_RE);
    const cm = before.match(CAT_DETECT_RE);
    if (mm) {
      setSlashPop((p) => (p.active ? { ...p, active: false } : p));
      setCatPop((p) => (p.active ? { ...p, active: false } : p));
      setMentionPop({ active: true, query: mm[1], sel: 0 });
    } else if (sm) {
      setMentionPop((p) => (p.active ? { ...p, active: false } : p));
      setCatPop((p) => (p.active ? { ...p, active: false } : p));
      setSlashPop({ active: true, query: sm[1], sel: 0 });
    } else if (cm) {
      setMentionPop((p) => (p.active ? { ...p, active: false } : p));
      setSlashPop((p) => (p.active ? { ...p, active: false } : p));
      setCatPop({ active: true, query: cm[1], sel: 0 });
    } else {
      closePops();
    }
  }, [closePops]);

  /** 通用 pick：把光标前的 @query / /query 文本替换为 token 并补空格，经重建落地为 chip */
  const pickToken = useCallback(
    (token: string) => {
      const el = editorRef.current;
      if (!el) return;
      const val = serializeEl(el);
      const offset = caretRef.current;
      const before = val.slice(0, offset);
      const m = before.match(token.startsWith('@') ? MENTION_DETECT_RE : SLASH_DETECT_RE);
      const startIdx = m
        ? offset - m[0].length + (m[0].trimStart().startsWith('@') || m[0].trimStart().startsWith('/') ? 0 : 1)
        : offset;
      const next = val.slice(0, startIdx) + token + val.slice(offset);
      pendingCaretRef.current = startIdx + token.length;
      closePops();
      onChangeRef.current(next);
    },
    [closePops],
  );

  /** & 命令 pick：把光标前的 &query 替换为 &命令id 纯文本并补空格（非 chip，发送时由页面解析拦截） */
  const pickCatCommand = useCallback(
    (command: CatCommand) => {
      const el = editorRef.current;
      if (!el) return;
      const val = serializeEl(el);
      const offset = caretRef.current;
      const before = val.slice(0, offset);
      const m = before.match(CAT_DETECT_RE);
      // 匹配串可能带前导空白（(?:^|\s)&），定位 & 的真实起点
      const ampIdx = m ? offset - m[0].length + m[0].indexOf('&') : offset;
      const token = `&${command.id} `;
      const next = val.slice(0, ampIdx) + token + val.slice(offset);
      pendingCaretRef.current = ampIdx + token.length;
      closePops();
      onChangeRef.current(next);
    },
    [closePops],
  );

  // 外部值同步：仅在值与 DOM 序列化结果不一致时重建（自己 onChange 的回声直接跳过）
  useLayoutEffect(() => {
    const el = editorRef.current;
    if (!el || composingRef.current) return;
    if (serializeEl(el) === value) return;
    rebuildDOM(el, value, skillsRef.current, fileSetRef.current);
    const pending = pendingCaretRef.current;
    pendingCaretRef.current = null;
    if (pending !== null) {
      // pick 路径：点击弹层使编辑器失焦，重建后收回焦点再放光标
      el.focus();
      placeCaret(el, pending);
    } else if (document.activeElement === el) {
      // 发送清空等场景：焦点仍在编辑器内，保住光标位置
      placeCaret(el, value.length);
    }
  }, [value]);

  // 光标移动跟随检测（方向键/点击；仅在选区位于编辑器内时更新缓存并重检测）
  useEffect(() => {
    const onSelChange = () => {
      const el = editorRef.current;
      if (!el || composingRef.current) return;
      const sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || !el.contains(sel.anchorNode)) return;
      const offset = computeCaret(el);
      caretRef.current = offset;
      runDetect(serializeEl(el), offset);
    };
    document.addEventListener('selectionchange', onSelChange);
    return () => document.removeEventListener('selectionchange', onSelChange);
  }, [runDetect]);

  const handleInput = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    const val = serializeEl(el);
    // 空内容归一化（清掉残留 <br>，让 :empty 生效显示 placeholder）
    if (!composingRef.current && val === '' && el.childNodes.length > 0) {
      el.replaceChildren();
    }
    onChangeRef.current(val);
    if (!composingRef.current) {
      const offset = computeCaret(el);
      caretRef.current = offset;
      runDetect(val, offset);
    }
  }, [runDetect]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const el = editorRef.current;
      if (!el) return;

      // IME 组合期间（含 macOS WKWebView keyCode 229）：候选确认的 Enter、
      // 选字的 ↑↓ 都不能被弹层导航/换行/删 chip 拦截，直接交给页面（isSendEnter 自会忽略）
      if (e.nativeEvent.isComposing || e.keyCode === 229) {
        onKeyDown(e);
        return;
      }
      caretRef.current = computeCaret(el);

      // 1) 弹层导航（↑↓ / Enter·Tab 选中 / Esc 关闭；列表为空时吞掉避免误发送）
      if (mentionPop.active || slashPop.active || catPop.active) {
        const isCat = catPop.active;
        const isMention = !isCat && mentionPop.active;
        const list = isCat ? catHits : isMention ? fileHits : skillHits;
        const sel = isCat ? catPop.sel : isMention ? mentionPop.sel : slashPop.sel;
        const setSel = (f: (i: number) => number) =>
          isCat
            ? setCatPop((p) => ({ ...p, sel: f(p.sel) }))
            : isMention
              ? setMentionPop((p) => ({ ...p, sel: f(p.sel) }))
              : setSlashPop((p) => ({ ...p, sel: f(p.sel) }));
        if (list.length === 0 && ['ArrowDown', 'ArrowUp', 'Enter', 'Tab'].includes(e.key)) {
          e.preventDefault();
          return;
        }
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          setSel((i) => Math.min(i + 1, list.length - 1));
          return;
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          setSel((i) => Math.max(i - 1, 0));
          return;
        }
        if (e.key === 'Enter' || e.key === 'Tab') {
          e.preventDefault();
          if (list.length > 0 && sel >= 0) {
            const item = list[sel];
            if (isCat) {
              pickCatCommand(item as CatCommand);
            } else {
              pickToken(isMention ? `@${item} ` : `/${(item as UserSkillLite).name} `);
            }
          }
          return;
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          closePops();
          return;
        }
      }

      // 2) Shift+Enter 换行（insertLineBreak 产生 <br>，序列化为 \n）
      if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        if (!document.execCommand('insertLineBreak') && el) {
          // 兜底：光标处手工插 \n 文本节点（pre-wrap 正常渲染）
          const sel = window.getSelection();
          if (sel && sel.rangeCount > 0) {
            const range = sel.getRangeAt(0);
            range.deleteContents();
            range.insertNode(document.createTextNode('\n'));
            range.collapse(false);
          }
        }
        return;
      }

      // 3) Backspace / Forward-Delete 原子删 chip。WebKit 会把贴着
      //    contenteditable=false 元素的光标归一化到不可靠位置，原生删除
      //    也经常拒绝对不可编辑节点生效，因此不依赖单一几何判断：向删除
      //    方向找邻居（可跨越 ZWSP 锚点与纯空白分隔符），命中 chip 则连同
      //    分隔空格一并移除（Slack 式原子删除）。
      if (e.key === 'Backspace' || e.key === 'Delete') {
        const s = window.getSelection();
        if (s && s.rangeCount > 0 && !s.isCollapsed) {
          // WebKit 首次 Backspace 会把不可编辑 chip 变为「选中」而非删除：
          // 选区恰好覆盖顶层 chip 时直接原子移除
          const range = s.getRangeAt(0);
          if (range.startContainer === el && range.endContainer === el) {
            for (let i = range.startOffset; i < range.endOffset; i++) {
              const n = el.childNodes[i];
              if (isChipEl(n)) {
                e.preventDefault();
                const nx = n.nextSibling;
                if (isZwspText(nx)) nx.remove();
                n.remove();
                onChangeRef.current(serializeEl(el));
                return;
              }
            }
          }
        } else if (s && s.rangeCount > 0 && s.isCollapsed) {
          const range = s.getRangeAt(0);
          const { startContainer, startOffset } = range;
          const back = e.key === 'Backspace';
          let node: Node | null = null;
          let anchor: Text | null = null;
          if (startContainer === el) {
            node = el.childNodes[startOffset + (back ? -1 : 0)] ?? null;
          } else if (startContainer.nodeType === Node.TEXT_NODE) {
            const t = startContainer as Text;
            const raw = t.textContent ?? '';
            const nearRaw = back ? raw.slice(0, startOffset) : raw.slice(startOffset);
            const near = nearRaw.replace(ZWSP_RE, '');
            if (back && startOffset === 0) {
              node = t.previousSibling;
            } else if (!back && startOffset === raw.length) {
              node = t.nextSibling;
            } else if (near.length === 0 && nearRaw.length > 0) {
              // 光标紧贴零宽锚点：跨过它找 chip，删除时顺带清掉这段 ZWSP
              anchor = t;
              node = back ? t.previousSibling : t.nextSibling;
            } else if (near.length > 0 && !/\S/.test(near)) {
              // 光标与 chip 之间只有空白（pickToken 的分隔空格）：原子删除
              anchor = t;
              node = back ? t.previousSibling : t.nextSibling;
            }
          }
          if (node && isZwspText(node)) {
            anchor = node;
            node = back ? node.previousSibling : node.nextSibling;
          }
          if (isChipEl(node)) {
            e.preventDefault();
            // chip 在值空间的起点（删除后光标回到这里）
            let chipStart = 0;
            for (const c of Array.from(el.childNodes)) {
              if (c === node) break;
              chipStart += nodeLen(c);
            }
            if (anchor) {
              if (anchor.textContent === ZWSP) {
                anchor.remove();
              } else if (anchor === startContainer) {
                // 只剥掉光标到 chip 之间的空白/ZWSP，保留真实文本
                anchor.textContent = back
                  ? (anchor.textContent ?? '').slice(startOffset)
                  : (anchor.textContent ?? '').slice(0, startOffset);
              }
            }
            // 清掉 chip 自身尾随的分隔空格，避免删除后出现双空格
            const next = node.nextSibling;
            if (
              next &&
              next !== anchor &&
              next.nodeType === Node.TEXT_NODE &&
              (next.textContent ?? '').startsWith(' ')
            ) {
              (next as Text).textContent = (next.textContent ?? '').slice(1);
            }
            node.remove();
            caretRef.current = chipStart;
            onChangeRef.current(serializeEl(el));
            placeCaret(el, chipStart);
            return;
          }
        }
      }

      // 4) 页面逻辑（发送 / 排队 / 打断 / 队列拉回）
      onKeyDown(e);
    },
    [closePops, fileHits, mentionPop, onKeyDown, pickCatCommand, pickToken, skillHits, slashPop, catHits, catPop],
  );

  // chip × 点击删除（事件委托：chip 由 DOM 直建，无法逐个挂 React 事件）
  const handleEditorClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest(`.${CHIP_CLASS}-x`);
    const el = editorRef.current;
    if (!target || !el) return;
    e.preventDefault();
    e.stopPropagation();
    const chip = target.closest(`.${CHIP_CLASS}`);
    if (!chip) return;
    // 同步清掉 chip 的尾随分隔（锚点 / 单个空格），避免删除后双空格
    const nx = chip.nextSibling;
    if (isZwspText(nx)) nx.remove();
    else if (nx && nx.nodeType === Node.TEXT_NODE && (nx.textContent ?? '').startsWith(' ')) {
      (nx as Text).textContent = (nx.textContent ?? '').slice(1);
    }
    chip.remove();
    onChangeRef.current(serializeEl(el));
  }, []);

  /** 纯文本插入（粘贴 / 落盘失败降级的公共尾路径） */
  const insertPlainText = useCallback((text: string) => {
    if (!document.execCommand('insertText', false, text)) {
      const sel = window.getSelection();
      if (sel && sel.rangeCount > 0) {
        const range = sel.getRangeAt(0);
        range.deleteContents();
        range.insertNode(document.createTextNode(text));
        range.collapse(false);
      }
    }
  }, []);

  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLDivElement>) => {
      e.preventDefault();
      const text = e.clipboardData.getData('text/plain');
      // 系统文件粘贴（Finder/资源管理器复制的文件）优先导入；webview 可能不把
      // 文件 URL 暴露给 clipboardData.files，故无文本时也尝试导入一次
      if (onPasteFiles && (e.clipboardData.files.length > 0 || !text)) {
        void onPasteFiles().then((imported) => {
          // 空结果 = 剪贴板没有可导入文件（截图等），降级回文本粘贴
          if (imported.length === 0 && text) {
            editorRef.current?.focus();
            insertPlainText(text);
          }
        });
        return;
      }
      if (!text) return;
      if (onSpillPaste && text.length >= SPILL_PASTE_CHARS) {
        // 落盘失败（无项目 / 写入报错）时降级回内联插入
        void onSpillPaste(text).then((spilled) => {
          if (!spilled) {
            editorRef.current?.focus();
            insertPlainText(text);
          }
        });
        return;
      }
      insertPlainText(text);
    },
    [onSpillPaste, onPasteFiles, insertPlainText],
  );

  useImperativeHandle(ref, () => ({
    focusEnd: () => {
      const el = editorRef.current;
      if (!el) return;
      el.focus();
      placeCaret(el, serializeEl(el).length);
    },
    insertText: (t: string) => {
      const el = editorRef.current;
      if (!el) return;
      el.focus();
      // 编辑器内无选区时先把光标放到末尾，避免插到开头
      const sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || !el.contains(sel.anchorNode)) {
        placeCaret(el, serializeEl(el).length);
      }
      document.execCommand('insertText', false, t);
    },
  }));

  return (
    <div className="minput">
      <div className="minput__popups">
        {mentionPop.active && files.length > 0 && (
          <FileMentionPopup
            query={mentionPop.query}
            files={fileHits}
            selectedIndex={mentionPop.sel}
            onSelect={(f) => pickToken(`@${f} `)}
          />
        )}
        {slashPop.active && skills.length > 0 && !mentionPop.active && (
          <SlashPopup
            query={slashPop.query}
            skills={skillHits}
            selectedIndex={slashPop.sel}
            onSelect={(s) => pickToken(`/${s.name} `)}
          />
        )}
        {catPop.active && !mentionPop.active && !slashPop.active && (
          <CatCommandPopup
            query={catPop.query}
            commands={catHits}
            selectedIndex={catPop.sel}
            onSelect={(c) => pickCatCommand(c)}
          />
        )}
      </div>
      <div
        ref={editorRef}
        className="minput__editor"
        contentEditable
        role="textbox"
        aria-multiline="true"
        aria-label={placeholder}
        data-placeholder={placeholder}
        suppressContentEditableWarning
        onInput={handleInput}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        onClick={handleEditorClick}
        onCompositionStart={() => {
          composingRef.current = true;
        }}
        onCompositionEnd={() => {
          composingRef.current = false;
          handleInput();
        }}
      />
    </div>
  );
});

export default MentionInput;
