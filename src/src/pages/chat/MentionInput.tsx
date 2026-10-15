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
import FileMentionPopup from './FileMentionPopup';
import SlashPopup from './SlashPopup';
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
  /** 组件内部的弹层导航 / 换行 / 删 chip 处理完后调用；页面在此处理发送与队列 */
  onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => void;
}

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

function isChipEl(n: Node | null): n is HTMLElement {
  return !!n && n.nodeType === Node.ELEMENT_NODE && (n as HTMLElement).classList.contains(CHIP_CLASS);
}

/** 节点的纯文本长度（chip 记作 token 长度、br 记 1，保证与序列化值一一对应） */
function nodeLen(n: Node): number {
  if (n.nodeType === Node.TEXT_NODE) return n.textContent?.length ?? 0;
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
  if (n.nodeType === Node.TEXT_NODE) return n.textContent ?? '';
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
    return sum + offset;
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
      const len = n.textContent?.length ?? 0;
      if (acc + len >= target) {
        found = { node: n, offset: target - acc };
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
  for (const seg of tokenize(value, skills, fileSet)) {
    if (seg.chip) {
      root.appendChild(buildChip(seg.chip, seg.text));
    } else {
      root.appendChild(document.createTextNode(seg.text));
    }
  }
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
  { value, onChange, files, skills, placeholder, onKeyDown },
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
  const closePops = useCallback(() => {
    setMentionPop((p) => (p.active ? { ...p, active: false } : p));
    setSlashPop((p) => (p.active ? { ...p, active: false } : p));
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

  /** @/ 检测：光标前缀命中哪种 token 就开哪个弹层（两者符号不同，天然互斥） */
  const runDetect = useCallback((value: string, offset: number) => {
    const before = value.slice(0, offset);
    const mm = before.match(MENTION_DETECT_RE);
    const sm = before.match(SLASH_DETECT_RE);
    if (mm) {
      setSlashPop((p) => (p.active ? { ...p, active: false } : p));
      setMentionPop({ active: true, query: mm[1], sel: 0 });
    } else if (sm) {
      setMentionPop((p) => (p.active ? { ...p, active: false } : p));
      setSlashPop({ active: true, query: sm[1], sel: 0 });
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
      if (mentionPop.active || slashPop.active) {
        const isMention = mentionPop.active;
        const list = isMention ? fileHits : skillHits;
        const sel = isMention ? mentionPop.sel : slashPop.sel;
        const setSel = (f: (i: number) => number) =>
          isMention
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
            pickToken(isMention ? `@${item} ` : `/${(item as UserSkillLite).name} `);
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

      // 3) Backspace 删除紧贴光标前的 chip（原子删除）
      if (e.key === 'Backspace') {
        const s = window.getSelection();
        if (s && s.rangeCount > 0 && s.isCollapsed) {
          const range = s.getRangeAt(0);
          let prev: Node | null = null;
          if (range.startContainer === el && range.startOffset > 0) {
            prev = el.childNodes[range.startOffset - 1] ?? null;
          } else if (range.startContainer.nodeType === Node.TEXT_NODE && range.startOffset === 0) {
            prev = range.startContainer.previousSibling;
          }
          if (isChipEl(prev)) {
            e.preventDefault();
            prev.remove();
            onChangeRef.current(serializeEl(el));
            return;
          }
        }
      }

      // 4) 页面逻辑（发送 / 排队 / 打断 / 队列拉回）
      onKeyDown(e);
    },
    [closePops, fileHits, mentionPop, onKeyDown, pickToken, skillHits, slashPop],
  );

  // chip × 点击删除（事件委托：chip 由 DOM 直建，无法逐个挂 React 事件）
  const handleEditorClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest(`.${CHIP_CLASS}-x`);
    const el = editorRef.current;
    if (!target || !el) return;
    e.preventDefault();
    e.stopPropagation();
    target.closest(`.${CHIP_CLASS}`)?.remove();
    onChangeRef.current(serializeEl(el));
  }, []);

  const handlePaste = useCallback((e: React.ClipboardEvent<HTMLDivElement>) => {
    e.preventDefault();
    const text = e.clipboardData.getData('text/plain');
    if (!text) return;
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
