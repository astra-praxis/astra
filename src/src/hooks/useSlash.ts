/**
 * / 技能调用共享逻辑（Agent 页输入框；与 useMention 同一套交互模式）
 *
 * - 检测正则 /(?:^|\s)\/([\w\u4e00-\u9fff.-]*)$/（@ 提及同款规则，符号换成 /）
 * - 无搜索词显示前 10 个；有搜索词过滤，最多 20 个
 * - 键盘导航 ↑↓ / Enter / Tab 选中 / Esc 关闭；列表为空时方向与 Enter/Tab 吞掉不冒泡
 * - 选中后在光标处把 /xxx 替换为 /技能名 并补空格
 *   （与 useMention 不同：键盘选中通过 onPickValue 回调回写状态，不丢返回值）
 */
import { useState, type RefObject } from 'react';

export interface UserSkillLite {
  name: string;
  description: string;
}

/** 检测用正则：/ 后跟字母/数字/中文/点/连字符（停止在空格、换行等处） */
export const SLASH_DETECT_RE = /(?:^|\s)\/([\w\u4e00-\u9fff.-]*)$/;

export interface UseSlashResult {
  active: boolean;
  query: string;
  filtered: UserSkillLite[];
  selectedIndex: number;
  /** onKeyDown 时调用：处理弹层导航；返回 true 表示事件已消费，调用方应直接 return */
  handleKeydown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => boolean;
  /** onChange 时调用：根据当前值与光标位置检测 / 调用 */
  detect: (value: string, cursorPos: number) => void;
  /** 选中技能：替换 /xxx 为 /技能名，返回新输入框值（焦点与光标由内部恢复） */
  pick: (skill: UserSkillLite, currentValue: string) => string;
  close: () => void;
}

export function useSlash(
  skills: UserSkillLite[],
  textareaRef: RefObject<HTMLTextAreaElement | null>,
  onPickValue: (v: string) => void,
): UseSlashResult {
  const [active, setActive] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);

  const q = query.toLowerCase();
  const filtered: UserSkillLite[] = q
    ? skills
        .filter(
          (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
        )
        .slice(0, 20)
    : skills.slice(0, 10);

  function detect(value: string, cursorPos: number) {
    if (!value || cursorPos === 0) {
      setActive(false);
      return;
    }
    const m = value.substring(0, cursorPos).match(SLASH_DETECT_RE);
    if (m) {
      setActive(true);
      setQuery(m[1]);
      setSelectedIndex(0);
    } else {
      setActive(false);
    }
  }

  /** 选中技能：光标处重新匹配 /query 起点（保留原空白），替换为 /技能名 并补空格 */
  function pick(skill: UserSkillLite, currentValue: string): string {
    const el = textareaRef.current;
    const cursorPos = el?.selectionStart ?? currentValue.length;
    const m = currentValue.substring(0, cursorPos).match(SLASH_DETECT_RE);
    const startIdx = m
      ? cursorPos - m[0].length + (m[0].startsWith('/') ? 0 : 1)
      : cursorPos;

    const insertion = `/${skill.name} `;
    const newValue =
      currentValue.substring(0, startIdx) + insertion + currentValue.substring(cursorPos);

    setActive(false);
    onPickValue(newValue);

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

  function handleKeydown(e: React.KeyboardEvent<HTMLTextAreaElement>): boolean {
    if (!active) return false;

    // 列表为空时吞掉方向键 / Enter / Tab（避免误发送），Esc 仍可关闭
    if (
      filtered.length === 0 &&
      (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === 'Tab')
    ) {
      e.preventDefault();
      return true;
    }

    if (e.key === 'ArrowDown') {
      e.preventDefault();
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
        pick(filtered[selectedIndex], e.currentTarget.value);
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
    handleKeydown,
    detect,
    pick,
    close: () => setActive(false),
  };
}

/**
 * 解析输入文本开头的 /技能名（发送时调用）。
 * 仅当 /xxx 是已知技能名时才算命中；返回技能名与去掉前缀后的任务文本。
 */
export function parseSlashCommand(
  text: string,
  knownSkills: UserSkillLite[],
): { skill: string; rest: string } | null {
  const m = text.match(/^\/([\w\u4e00-\u9fff.-]+)(?:\s+([\s\S]*))?$/);
  if (!m) return null;
  const name = m[1];
  if (!knownSkills.some((s) => s.name === name)) return null;
  return { skill: name, rest: (m[2] ?? '').trim() };
}
