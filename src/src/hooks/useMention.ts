/**
 * @ 提及共享交互逻辑（chat 页提及项目 / Agent 页引用项目文件，共用一套交互）
 *
 * 从两处重复实现提取，行为等价：
 * - 检测正则 /(?:^|\s)@([\w\u4e00-\u9fff./-]*)$/（\w 不含中文，必须显式加 \u4e00-\u9fff；
 *   斜杠用于文件相对路径引用，项目名不含 / 不受影响）
 * - 无搜索词显示前 10 个；有搜索词过滤，最多 20 个
 * - 键盘导航 ↑↓ / Enter / Tab 选中 / Esc 关闭；列表为空时方向与 Enter/Tab 一律吞掉不冒泡
 * - 选中后在光标处把 @xxx 替换为 @引用文本 并补空格，rAF 恢复焦点与光标
 *
 * 条目类型泛型：字符串项（文件相对路径）取本身作引用文本；
 * 对象项默认取 name 字段（ProjectItem 等），点击路径由调用方 setInput(pick(...)) 处理。
 */
import { useState, type RefObject } from 'react';

/** 检测用正则：@ 后跟字母/数字/中文/点/连字符/斜杠（停止在空格、换行等处） */
const MENTION_DETECT_RE = /(?:^|\s)@([\w\u4e00-\u9fff./-]*)$/;

/**
 * 是否为「发送」Enter。
 * 中文输入法确认时 macOS WKWebView 会发 keyCode 229 的 Enter
 * （isComposing 可能已是 false），两个条件都要拦，否则选字回车会直接发送。
 */
export function isSendEnter(e: React.KeyboardEvent<HTMLTextAreaElement>): boolean {
  return e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing && e.keyCode !== 229;
}

/** 取选中项插入文本：字符串项即本身；对象项默认取 name 字段 */
function labelOf<T>(item: T): string {
  if (typeof item === 'string') return item;
  if (item && typeof item === 'object' && 'name' in item) {
    return String((item as { name: unknown }).name);
  }
  return String(item);
}

export interface UseMentionResult<T> {
  /** 弹层是否激活 */
  active: boolean;
  /** @ 后的搜索词 */
  query: string;
  /** 按搜索词过滤后的条目 */
  filtered: T[];
  /** 键盘导航选中索引 */
  selectedIndex: number;
  setSelectedIndex: (i: number) => void;
  /** onChange 时调用：根据当前值与光标位置检测 @ 提及 */
  detect: (value: string, cursorPos: number) => void;
  /** onKeyDown 时调用：处理提及导航；返回 true 表示事件已消费，调用方应直接 return */
  handleKeydown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => boolean;
  /** 选中条目：替换 @xxx 为 @引用文本，返回新输入框值（焦点与光标由内部恢复） */
  pick: (item: T, currentValue: string) => string;
  /** 手动触发（底栏「添加上下文」按钮）：光标处插入 @ 并检测，等价用户键入 @；返回新输入框值 */
  insertAt: () => string;
  /** 关闭弹层 */
  close: () => void;
}

export function useMention<T>(
  items: T[],
  textareaRef: RefObject<HTMLTextAreaElement | null>,
  onPick?: (item: T) => void,
  /** 键盘选中时回写输入框值（点击路径由调用方 setInput(pick(...)) 处理） */
  onPickValue?: (v: string) => void,
): UseMentionResult<T> {
  const [active, setActive] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);

  // 无搜索词时显示前10个；有搜索词时过滤，最多显示20个匹配结果
  const q = query.toLowerCase();
  const filtered: T[] = q
    ? items.filter((it) => labelOf(it).toLowerCase().includes(q)).slice(0, 20)
    : items.slice(0, 10);

  function detect(value: string, cursorPos: number) {
    if (!value || cursorPos === 0) {
      setActive(false);
      return;
    }
    const atMatch = value.substring(0, cursorPos).match(MENTION_DETECT_RE);
    if (atMatch) {
      setActive(true);
      setQuery(atMatch[1]); // @ 后面的搜索词
      setSelectedIndex(0);
    } else {
      setActive(false);
    }
  }

  /** 选中条目：光标处重新匹配 @query 起点（弹出期间光标不动，与记录 start index 等价） */
  function pick(item: T, currentValue: string): string {
    const el = textareaRef.current;
    const cursorPos = el?.selectionStart ?? currentValue.length;
    const atMatch = currentValue.substring(0, cursorPos).match(MENTION_DETECT_RE);
    // 匹配可能带前导空白（(?:^|\s)），起点要落在 @ 上，保留原空白
    const startIdx = atMatch
      ? cursorPos - atMatch[0].length + (atMatch[0].startsWith('@') ? 0 : 1)
      : cursorPos;

    const insertion = `@${labelOf(item)} `;
    const newValue =
      currentValue.substring(0, startIdx) + insertion + currentValue.substring(cursorPos);

    setActive(false);
    onPick?.(item);

    // 更新光标位置到插入文本之后（等待受控值渲染完成）
    requestAnimationFrame(() => {
      const t = textareaRef.current;
      if (t) {
        t.focus();
        const pos = startIdx + insertion.length;
        t.setSelectionRange(pos, pos);
      }
    });
    return newValue;
  }

  /**
   * 手动触发 @ 提及（底栏按钮）：在光标处插入 @，按检测规则打开弹层。
   * 与键入 @ 行为一致——前置字符非空白/行首时正则不命中，弹层不打开。
   * 返回新输入框值，由调用方回写受控状态（textarea 是受控组件，不能直接改 DOM value）。
   */
  function insertAt(): string {
    const el = textareaRef.current;
    const value = el?.value ?? '';
    const start = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? start;
    const next = value.slice(0, start) + '@' + value.slice(end);
    const pos = start + 1;
    detect(next, pos);
    // 受控值渲染完成后恢复焦点与光标（与 pick 同款 rAF 模式）
    requestAnimationFrame(() => {
      const t = textareaRef.current;
      if (t) {
        t.focus();
        t.setSelectionRange(pos, pos);
      }
    });
    return next;
  }

  function handleKeydown(e: React.KeyboardEvent<HTMLTextAreaElement>): boolean {
    if (!active) return false;

    // 如果列表为空，不处理方向键 / Enter / Tab（吞掉，避免误发送）
    if (
      filtered.length === 0 &&
      (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === 'Tab')
    ) {
      e.preventDefault();
      return true;
    }

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      // 边界检查：确保 selectedIndex 不超过列表长度
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
      return true;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
      return true;
    }
    if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault();
      if (filtered.length > 0 && selectedIndex >= 0) {
        const newValue = pick(filtered[selectedIndex], e.currentTarget.value);
        // pick 只计算新值，键盘路径必须显式回写，否则输入框停留在 @query
        onPickValue?.(newValue);
      }
      return true;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      setActive(false);
      return true;
    }
    return false;
  }

  return {
    active,
    query,
    filtered,
    selectedIndex,
    setSelectedIndex,
    detect,
    handleKeydown,
    pick,
    insertAt,
    close: () => setActive(false),
  };
}
