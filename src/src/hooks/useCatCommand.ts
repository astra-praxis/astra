/**
 * & 猫猫互动命令
 *
 * 输入框里以 & 开头唤起猫猫互动（如 &打招呼 / &wave），与 @提及、/技能
 * 同一套交互模式。选中命令后替换为 &命令名 并补空格，发送时由 AgentPage
 * 拦截为本地互动（不经过 agent 后端）：猫猫播放动画 + 在会话里以「猫猫」
 * 前缀回一句脚本台词，会话内可区分「猫猫在对话」还是「agent 在对话」。
 */
import type { MascotPose } from '@/stores/mascot';

/** 检测用正则：& 后跟字母/数字/中文/连字符（停止在空格、换行等处） */
export const CAT_DETECT_RE = /(?:^|\s)&([\w\u4e00-\u9fff-]*)$/;

export interface CatCommand {
  /** 稳定标识：发送时输入框替换为 &{id} */
  id: string;
  /** 展示名称（& 后的主关键字） */
  label: string;
  /** 可命中关键字：中文 + 英文（过滤/发送解析都用） */
  aliases: string[];
  /** 弹层图标 */
  emoji: string;
  /** 一句话说明 */
  desc: string;
  /** 触发姿态 */
  pose: MascotPose;
  /** 一次性动画 */
  fx?: 'pet' | 'jump';
  /** 猫猫在会话里回的脚本台词（以「猫猫」前缀展示） */
  reply: string;
}

export const CAT_COMMANDS: CatCommand[] = [
  {
    id: 'pet',
    label: '摸头',
    aliases: ['pet', 'rua', '摸摸', '摸头', '摸'],
    emoji: '🤚',
    desc: '摸摸猫猫的头',
    pose: 'happy',
    fx: 'pet',
    reply: '喵～好舒服，主人最好了！',
  },
  {
    id: 'wave',
    label: '打招呼',
    aliases: ['wave', 'hi', 'hello', '嗨', '你好', '打招呼', '挥手'],
    emoji: '👋',
    desc: '猫猫向你挥手',
    pose: 'wave',
    reply: '喵！你好呀主人～',
  },
  {
    id: 'jump',
    label: '跳跃',
    aliases: ['jump', '跳', '跳跃'],
    emoji: '🦘',
    desc: '猫猫跳起来',
    pose: 'jump',
    fx: 'jump',
    reply: '喵呜～飞起来啦！',
  },
  {
    id: 'sleep',
    label: '睡觉',
    aliases: ['sleep', 'zzz', '睡', '睡觉'],
    emoji: '😴',
    desc: '让猫猫打个盹',
    pose: 'sleeping',
    reply: '喵…晚安，主人也早点休息哦～',
  },
  {
    id: 'think',
    label: '思考',
    aliases: ['think', '想', '思考', '想想'],
    emoji: '🤔',
    desc: '猫猫陷入沉思',
    pose: 'thinking',
    reply: '喵？让我想想…',
  },
  {
    id: 'surprise',
    label: '惊讶',
    aliases: ['surprise', 'surprised', '吓', '惊讶'],
    emoji: '😲',
    desc: '吓猫猫一跳',
    pose: 'surprised',
    reply: '喵喵喵？！吓我一跳！',
  },
  {
    id: 'search',
    label: '搜索',
    aliases: ['search', '找', '搜索', '查'],
    emoji: '🔍',
    desc: '猫猫帮你找东西',
    pose: 'search',
    reply: '喵…让我找找看～',
  },
  {
    id: 'type',
    label: '打字',
    aliases: ['type', 'typing', '码', '打字'],
    emoji: '⌨️',
    desc: '猫猫敲键盘',
    pose: 'typing',
    reply: '喵喵喵（键盘敲得飞起）',
  },
  {
    id: 'celebrate',
    label: '庆祝',
    aliases: ['celebrate', 'done', '耶', '棒', '庆祝'],
    emoji: '🎉',
    desc: '和猫猫一起庆祝',
    pose: 'done',
    reply: '喵呜～太好了，我们做到了！',
  },
  {
    id: 'bye',
    label: '拜拜',
    aliases: ['bye', '再见', '拜拜', '走了'],
    emoji: '🚪',
    desc: '和猫猫道别',
    pose: 'leave',
    reply: '喵…主人要走了吗？拜拜～',
  },
  {
    id: 'angry',
    label: '生气',
    aliases: ['angry', '哼', '生气'],
    emoji: '😾',
    desc: '把猫猫惹生气了',
    pose: 'angry',
    reply: '喵！(╬￣皿￣) 主人别惹我！',
  },
  {
    id: 'play',
    label: '玩耍',
    aliases: ['play', '玩', '玩耍', '逗'],
    emoji: '🎾',
    desc: '和猫猫玩耍',
    pose: 'play',
    reply: '喵～来玩来玩！逗猫棒呢？',
  },
  // ===== 第二批：语义已被 TOOL_POSE / TIP 锚定的姿态 =====
  {
    id: 'stretch',
    label: '伸懒腰',
    aliases: ['stretch', '伸懒腰', '伸展', '懒腰'],
    emoji: '😽',
    desc: '猫猫伸个懒腰',
    pose: 'stretch',
    reply: '喵呜～伸个懒腰，精神多了！',
  },
  {
    id: 'read',
    label: '看书',
    aliases: ['read', 'book', '看书', '读书'],
    emoji: '📖',
    desc: '猫猫安静看书',
    pose: 'reading',
    reply: '喵…让我看看这本书讲了什么～',
  },
  {
    id: 'laptop',
    label: '敲电脑',
    aliases: ['laptop', '电脑', '敲电脑', '办公'],
    emoji: '💻',
    desc: '猫猫用电脑办公',
    pose: 'laptop',
    reply: '喵哒喵哒（认真敲代码中）',
  },
  {
    id: 'tail',
    label: '摇尾巴',
    aliases: ['tail', '摇尾巴', '尾巴'],
    emoji: '〰️',
    desc: '猫猫开心摇尾巴',
    pose: 'tail',
    reply: '喵呜～开心的时候尾巴会摇哦',
  },
  {
    id: 'wink',
    label: '眨眼',
    aliases: ['wink', '眨眼', '抛媚眼'],
    emoji: '😉',
    desc: '猫猫对你眨眨眼',
    pose: 'wink',
    reply: '喵～(･ω<)☆ 看到了吗！',
  },
  {
    id: 'tilt',
    label: '歪头',
    aliases: ['tilt', '歪头', '疑惑'],
    emoji: '🤨',
    desc: '猫猫歪头看你',
    pose: 'tilt',
    reply: '喵？什么意思呀？',
  },
  {
    id: 'lie-flat',
    label: '躺平',
    aliases: ['lie-flat', 'lieflat', '躺平', '摆烂'],
    emoji: '🛌',
    desc: '猫猫彻底躺平',
    pose: 'lie-flat',
    reply: '喵…今天什么都不想干，躺平了',
  },
  {
    id: 'backpack',
    label: '背书包',
    aliases: ['backpack', '书包', '背书包', '上学'],
    emoji: '🎒',
    desc: '猫猫背起书包出发',
    pose: 'backpack',
    reply: '喵！背好书包，出发咯！',
  },
  {
    id: 'crash',
    label: '累趴',
    aliases: ['crash', '累趴', '累'],
    emoji: '😵',
    desc: '猫猫累趴下了',
    pose: 'crash',
    reply: '喵…说了太多话，累趴了…',
  },
  {
    id: 'vanish',
    label: '消失',
    aliases: ['vanish', '消失', '隐身'],
    emoji: '💨',
    desc: '猫猫原地消失',
    pose: 'vanish',
    reply: '喵～我要藏起来咯！找不到我啦！',
  },
];

/** 按查询词过滤命令：命中 label / aliases / id 任意一项 */
export function filterCatCommands(query: string): CatCommand[] {
  const q = query.trim().toLowerCase();
  if (!q) return CAT_COMMANDS;
  return CAT_COMMANDS.filter(
    (c) =>
      c.id.toLowerCase().includes(q) ||
      c.label.includes(q) ||
      c.aliases.some((a) => a.toLowerCase().includes(q)),
  );
}

/** 精确匹配一个命令（别名 / id / label 全等） */
function exactCatCommand(token: string): CatCommand | null {
  const t = token.trim().toLowerCase();
  if (!t) return null;
  return (
    CAT_COMMANDS.find(
      (c) =>
        c.id.toLowerCase() === t ||
        c.label.toLowerCase() === t ||
        c.aliases.some((a) => a.toLowerCase() === t),
    ) ?? null
  );
}

/**
 * 发送时解析 & 猫猫互动：仅当整条消息以 &命令 开头（命令为已知命令）时命中。
 * 返回命中的命令与剩余文本；不命中返回 null（交给 agent 正常处理）。
 */
export function parseCatCommandInput(text: string): { command: CatCommand; rest: string } | null {
  const m = text.match(/^&([\w\u4e00-\u9fff-]+)(?:\s+([\s\S]*))?$/);
  if (!m) return null;
  const command = exactCatCommand(m[1]);
  if (!command) return null;
  return { command, rest: (m[2] ?? '').trim() };
}
