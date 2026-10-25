/**
 * Astra 吉祥物（黑猫）：蹲在 Agent 输入框右上角，由对话内容驱动形态。
 *
 * 形态推导分两层：
 *   1. basePose（语义形态，优先级高 → 低）：
 *      error                 → state-network-error（网络异常）
 *      双击跳跃中            → inter-jump
 *      running（运行中）:
 *        权限等待确认         → expr-question（疑问，配合 inter-beckon 轮换）
 *        工具被拒绝           → expr-angry（生气）
 *        工具失败             → inter-shake（摇头）
 *        写文件类工具         → work-typing（打字）
 *        搜索类工具           → work-search（搜索）
 *        读文件类工具         → work-reading（看书）
 *        命令/终端类工具      → work-tools（使用工具）
 *        记忆写入             → work-backpack（背包装进记忆）
 *        规划/思考            → expr-thinking（思考）
 *        其他工具             → expr-focused（专注）
 *      正在思考（thinking）  → expr-thinking
 *      刚完成(4s)            → 按回复关键词：完成→work-done / 搜索→work-search /
 *                             提问→expr-question / 出错→expr-wronged / 睡觉→expr-sleeping /
 *                             再见→inter-wave / 默认→expr-happy
 *      hover                 → expr-happy
 *      空闲 > 90s            → expr-sleeping
 *      默认                  → basic-sit
 *   2. 活跃期（running 或刚完成 4s 内）叠加「相似图轮换」：每 2.5s tick 一次，
 *      从该形态的相似图组里轮换一张（如打字时 work-typing ↔ work-laptop ↔ expr-focused），
 *      让对话过程中的猫"活"起来。轮换只在活跃期生效，非活跃态固定展示。
 *
 * 交互：
 *   单击 = 摸头（弹跳 + 爱心，纯动画）
 *   双击 = 跳起来 + 气泡提示「输入 & 跟我互动」（不再预填输入框文本）
 *   悬停 = 开心 + 「& 跟我玩」按钮，点击只聚焦输入框
 *   外部互动（输入框 & 命令）经 useMascotStore.interact() 短暂接管姿态 + 气泡
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { useAgentStore, type Entry } from '@/stores/agent';
import { useMascotStore } from '@/stores/mascot';
import './Mascot.scss';

type Pose =
  | 'curious' | 'focused' | 'thinking' | 'happy' | 'surprised'
  | 'sleeping' | 'typing' | 'search' | 'reading' | 'tools'
  | 'done' | 'wronged' | 'error' | 'wave' | 'jump' | 'question'
  | 'laptop' | 'idea' | 'wink' | 'tilt' | 'waiting' | 'loading'
  | 'backpack' | 'notice' | 'leave' | 'angry' | 'shake' | 'beckon'
  | 'play' | 'crash' | 'test'
  | 'walk' | 'run' | 'stretch' | 'lie-down' | 'stand' | 'tail'
  | 'crouch' | 'lie-flat' | 'nod' | 'fade' | 'vanish';

export type { Pose };

/** 全部 42 张切图，按语义映射到 Pose */
const IMG: Record<Pose, string> = {
  curious: '/mascot/actions/basic-sit.png',
  stand: '/mascot/actions/basic-stand.png',
  walk: '/mascot/actions/basic-walk.png',
  run: '/mascot/actions/basic-run.png',
  stretch: '/mascot/actions/basic-stretch.png',
  'lie-down': '/mascot/actions/basic-lie-down.png',
  tail: '/mascot/actions/basic-tail-swish.png',
  focused: '/mascot/actions/expr-focused.png',
  thinking: '/mascot/actions/expr-thinking.png',
  happy: '/mascot/actions/expr-happy.png',
  surprised: '/mascot/actions/expr-surprised.png',
  sleeping: '/mascot/actions/expr-sleeping.png',
  typing: '/mascot/actions/work-typing.png',
  search: '/mascot/actions/work-search.png',
  reading: '/mascot/actions/work-reading.png',
  tools: '/mascot/actions/work-tools.png',
  done: '/mascot/actions/work-done.png',
  wronged: '/mascot/actions/expr-wronged.png',
  error: '/mascot/actions/state-network-error.png',
  wave: '/mascot/actions/inter-wave.png',
  jump: '/mascot/actions/inter-jump.png',
  question: '/mascot/actions/expr-question.png',
  laptop: '/mascot/actions/work-laptop.png',
  idea: '/mascot/actions/work-idea.png',
  wink: '/mascot/actions/expr-wink.png',
  tilt: '/mascot/actions/inter-tilt.png',
  waiting: '/mascot/actions/state-waiting.png',
  loading: '/mascot/actions/state-loading.png',
  backpack: '/mascot/actions/work-backpack.png',
  notice: '/mascot/actions/state-notice.png',
  leave: '/mascot/actions/state-leave.png',
  angry: '/mascot/actions/expr-angry.png',
  shake: '/mascot/actions/inter-shake.png',
  beckon: '/mascot/actions/inter-beckon.png',
  crouch: '/mascot/actions/inter-crouch.png',
  'lie-flat': '/mascot/actions/inter-lie-flat.png',
  nod: '/mascot/actions/inter-nod.png',
  play: '/mascot/actions/inter-play.png',
  crash: '/mascot/actions/state-crash.png',
  test: '/mascot/actions/state-test.png',
  fade: '/mascot/actions/state-fade.png',
  vanish: '/mascot/actions/state-vanish.png',
};

const TIP: Record<Pose, string> = {
  curious: 'Astra 在看着你（单击摸头，双击唤起对话）',
  stand: 'Astra 站起来了',
  walk: 'Astra 在散步…',
  run: 'Astra 跑起来了！',
  stretch: 'Astra 伸了个懒腰',
  'lie-down': 'Astra 趴着休息…',
  tail: 'Astra 甩了甩尾巴',
  focused: 'Astra 正在专注工作…',
  thinking: 'Astra 正在思考…',
  happy: 'Astra 很开心！',
  surprised: 'Astra 被吓到了…',
  sleeping: 'Astra 打瞌睡中（悬停唤醒）',
  typing: 'Astra 正在敲键盘…',
  search: 'Astra 正在搜索…',
  reading: 'Astra 正在读文件…',
  tools: 'Astra 正在使用工具…',
  done: 'Astra 完成任务啦！',
  wronged: 'Astra 遇到问题了…',
  error: 'Astra 遇到网络异常…',
  wave: 'Astra 跟你挥手～',
  jump: 'Astra 跳起来了！',
  question: 'Astra 在等你确认…',
  laptop: 'Astra 在看电脑…',
  idea: 'Astra 灵光一闪！',
  wink: 'Astra 眨眨眼～',
  tilt: 'Astra 歪着头看…',
  waiting: 'Astra 在等待…',
  loading: 'Astra 加载中…',
  backpack: 'Astra 把东西装进背包…',
  notice: 'Astra 要告诉你一件事',
  leave: 'Astra 要走了…',
  angry: 'Astra 有点生气…',
  shake: 'Astra 摇摇头…',
  beckon: 'Astra 在招手叫你…',
  crouch: 'Astra 蹲着观察…',
  'lie-flat': 'Astra 趴平了…',
  nod: 'Astra 点了点头',
  play: 'Astra 想玩耍～',
  crash: 'Astra 好像崩了…',
  test: 'Astra 正在测试…',
  fade: 'Astra 变透明了…',
  vanish: 'Astra 消失了…',
};

/**
 * 活跃期（running / 刚完成）的相似图轮换组：
 * 每个 basePose → 一组语义相近的图，按 tick 轮换。
 */
const VARIATIONS: Partial<Record<Pose, Pose[]>> = {
  typing: ['typing', 'laptop', 'focused', 'walk'],
  laptop: ['laptop', 'typing', 'focused'],
  search: ['search', 'thinking', 'reading', 'walk'],
  reading: ['reading', 'thinking', 'curious', 'nod'],
  tools: ['tools', 'idea', 'focused', 'run'],
  focused: ['focused', 'laptop', 'thinking', 'stretch'],
  thinking: ['thinking', 'focused', 'idea', 'walk'],
  question: ['question', 'beckon', 'tilt'],
  waiting: ['waiting', 'loading', 'crouch', 'lie-flat'],
  loading: ['loading', 'waiting', 'test', 'run'],
  backpack: ['backpack', 'tools', 'done'],
  done: ['done', 'happy', 'wink', 'tail'],
  happy: ['happy', 'wink', 'tail', 'stand'],
  notice: ['notice', 'wave', 'nod', 'happy'],
  test: ['test', 'loading', 'focused'],
  error: ['error', 'crash', 'wronged', 'fade'],
  wronged: ['wronged', 'shake', 'fade', 'vanish'],
  sleeping: ['sleeping', 'lie-down'],
  curious: ['curious', 'stand', 'tail', 'nod'],
};

/** 工具名 → 形态（精确匹配优先，keyword 兜底覆盖 MCP 动态工具） */
const TOOL_POSE: Record<string, Pose> = {
  // 写文件 / 编辑
  write_file: 'typing', edit_file: 'typing', apply_patch: 'typing',
  create_file: 'typing', append_file: 'typing', write_user_skill: 'typing',
  write_user_skill_file: 'typing', update_skill: 'typing',
  // 文件系统变更（复制/移动/建目录）
  copy_path: 'tools', move_path: 'tools', create_directory: 'tools',
  // 搜索
  search_web: 'search', search_similar: 'search', grep: 'search',
  find_path: 'search', find: 'search', memory_search: 'search',
  embed_text: 'search', search_chunks: 'search',
  // 读取 / 查看
  read_file: 'reading', read_directory: 'reading', open_file: 'reading',
  open_in_editor: 'reading', extract_document: 'reading',
  list_workspaces: 'reading', list_user_skill_files: 'reading',
  read_user_skill_file: 'reading', scan_skills: 'reading',
  // 命令 / 终端 / 任务
  shell_execute: 'tools', shell_background: 'tools', shell_kill: 'tools',
  shell_status: 'tools', task_status: 'tools', task_result: 'tools',
  task_cancel: 'tools', run_command: 'tools', terminal: 'tools',
  execute: 'tools', build_docx: 'tools',
  // 记忆
  memory_remember: 'backpack', memory_forget: 'backpack', memory_status: 'backpack',
  // 规划 / 思考
  update_plan: 'thinking', plan: 'thinking', think: 'thinking',
  // 模型 / 环境
  list_local_models: 'laptop', run_llama_server: 'laptop',
  mcp_config_read: 'laptop', mcp_config_write: 'tools',
  extensions_list: 'laptop', extensions_set_enabled: 'tools',
  extensions_remove: 'tools', extensions_update: 'tools',
  checkpoint_list: 'laptop', checkpoint_create: 'done', checkpoint_restore: 'test',
  generate_image: 'idea', fetch: 'search', download_file: 'loading',
  ask_user: 'question',
};

/** 关键词兜底：覆盖 MCP 工具等动态名（如 mcp__xxx_read） */
function toolPose(name: string): Pose {
  const exact = TOOL_POSE[name];
  if (exact) return exact;
  const n = name.toLowerCase();
  if (n.includes('search') || n.includes('query') || n.includes('find') || n.includes('grep')) return 'search';
  if (n.includes('write') || n.includes('edit') || n.includes('patch') || n.includes('append') || n.includes('create')) return 'typing';
  if (n.includes('read') || n.includes('list') || n.includes('open') || n.includes('show') || n.includes('get')) return 'reading';
  if (n.includes('shell') || n.includes('exec') || n.includes('command') || n.includes('run') || n.includes('task')) return 'tools';
  if (n.includes('memory') || n.includes('remember') || n.includes('store')) return 'backpack';
  if (n.includes('plan') || n.includes('think')) return 'thinking';
  if (n.includes('download')) return 'loading';
  if (n.includes('generate') || n.includes('image') || n.includes('draw')) return 'idea';
  return 'focused';
}

/** 回复内容关键词 → 形态（完成后展示） */
function analyzeText(text: string): Pose {
  const t = text.toLowerCase();
  if (/错误|失败|出错|异常|崩溃|无法|error|failed|cannot|timeout/i.test(t)) return 'wronged';
  if (/完成|搞定|成功|已保存|做好了|done|saved|finished|success|恭喜/i.test(t)) return 'done';
  if (/搜索|查找|检索|查询|search|find|lookup/i.test(t)) return 'search';
  if (/思考|分析|考虑|think|analyze|研究/i.test(t)) return 'thinking';
  if (/晚安|睡觉|休息|sleep|rest|zzz/i.test(t)) return 'sleeping';
  if (/再见|拜拜|离开|bye|farewell|走了/i.test(t)) return 'wave';
  if (/谢谢|感谢|很棒|厉害|辛苦|thanks|great|awesome|赞/i.test(t)) return 'happy';
  if (/\?|？|是否|可以吗|为什么|怎么|how|why|what|吗$/i.test(t)) return 'question';
  return 'happy';
}

const IDLE_SLEEP_MS = 90_000;
const HAPPY_MS = 4000;
const FLASH_MS = 900;
const VARIATION_MS = 2500;
const INTERACT_HOLD_MS = 3000;
/** 输入感知：持续输入 notice→typing 的升级时间 / 停止输入后退出关注态的时间 */
const TYPING_TYPE_MS = 2500;
const TYPING_STOP_MS = 4000;

/** 夜间时段（23 点～次日 6 点）：部分气泡台词切换为夜间版 */
function isNight(): boolean {
  const h = new Date().getHours();
  return h >= 23 || h < 6;
}

interface Props {
  /** 只聚焦输入框（不预填任何文本）；互动内容统一由用户以 & 前缀输入 */
  onFocusInput?: () => void;
}

export function Mascot({ onFocusInput }: Props) {
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const error = useAgentStore((s) => s.error);
  const entries = useAgentStore((s) => s.entries);
  const input = useAgentStore((s) => s.input);
  const queueRaw = useAgentStore((s) => s.queue);
  const sessionId = useAgentStore((s) => s.sessionId);
  /** 当前会话排队条数（排队提示气泡用） */
  const queuedCount = useMemo(
    () => queueRaw.filter((q) => q.sessionId === sessionId).length,
    [queueRaw, sessionId],
  );

  const [hover, setHover] = useState(false);
  const [justFinished, setJustFinished] = useState(false);
  const [idle, setIdle] = useState(false);
  // 用户输入感知：off → notice（抬头看你）→ typing（跟着敲键盘）；运行中不打扰
  const [typingPhase, setTypingPhase] = useState<'off' | 'notice' | 'typing'>('off');
  // 当前待回答的提问卡 id（无则 null）
  const [pendingQuestionId, setPendingQuestionId] = useState<string | null>(null);
  // 隐藏彩蛋：vanish 演出后藏起来，可从占位按钮叫回
  const [hidden, setHidden] = useState(false);
  const [vanishing, setVanishing] = useState(false);
  const vanishTimerRef = useRef<number | null>(null);
  // 一次性动画（单击摸头 / 双击跳跃）；jump 同时把形态切成跳跃姿态
  const [fx, setFx] = useState<'pet' | 'jump' | null>(null);
  // 双击惊吓闪态：短暂切成惊讶表情
  const [flash, setFlash] = useState(false);
  // 气泡文字（问候/对话提示）
  const [bubble, setBubble] = useState<string | null>(null);
  // 活跃期相似图轮换 tick
  const [tick, setTick] = useState(0);
  // 外部互动（& 猫猫命令等）：短暂接管姿态 + 一次性动画 + 气泡
  const event = useMascotStore((s) => s.event);
  const [interactPose, setInteractPose] = useState<Pose | null>(null);
  const interactTimerRef = useRef<number | null>(null);

  const prevRunningRef = useRef(running);
  const lastActiveRef = useRef(Date.now());
  const bubbleTimerRef = useRef<number | null>(null);
  // 事件处理读最新 running，避免把它放进 deps 重复处理同一事件
  const runningRef = useRef(running);
  runningRef.current = running;
  // 连点计数（连点生气彩蛋）
  const pokeTimesRef = useRef<number[]>([]);
  // 工具失败 → 后续工具重跑的检测标记
  const sawFailedToolRef = useRef(false);

  // 完成 → 刚完成态：running 由 true 变 false 且无错误时，按内容展示 4 秒
  useEffect(() => {
    const wasRunning = prevRunningRef.current;
    prevRunningRef.current = running;
    if (wasRunning && !running && !error) {
      lastActiveRef.current = Date.now();
      setJustFinished(true);
      const t = window.setTimeout(() => setJustFinished(false), HAPPY_MS);
      return () => window.clearTimeout(t);
    }
    if (running) lastActiveRef.current = Date.now();
  }, [running, error]);

  // 空闲检测：非运行状态下超过阈值则打瞌睡（低频轮询，避免常驻定时器抖动）
  useEffect(() => {
    const timer = window.setInterval(() => {
      setIdle(!running && Date.now() - lastActiveRef.current > IDLE_SLEEP_MS);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [running]);

  // 输入感知：Agent 空闲且有输入时先关注后陪打；停笔或清空后回到常规推导。
  // 每次按键重置 toOff（停笔 4s 才退出），但升级计时器独立于按键（见下个 effect）
  useEffect(() => {
    if (running || input.trim().length === 0) {
      setTypingPhase('off');
      return;
    }
    lastActiveRef.current = Date.now();
    setTypingPhase((p) => (p === 'off' ? 'notice' : p));
    const toOff = window.setTimeout(() => setTypingPhase('off'), TYPING_STOP_MS);
    return () => window.clearTimeout(toOff);
  }, [input, running]);

  // 持续输入 TYPING_TYPE_MS 后升级为「一起打字」（计时器不因后续按键重置）
  useEffect(() => {
    if (typingPhase !== 'notice') return;
    const t = window.setTimeout(() => setTypingPhase('typing'), TYPING_TYPE_MS);
    return () => window.clearTimeout(t);
  }, [typingPhase]);

  // 进入关注态时轻声搭一句（冷却防打扰；运行中不会进入该态；夜间换台词）
  useEffect(() => {
    if (typingPhase !== 'notice') return;
    useMascotStore.getState().say(isNight() ? '喵…这么晚还在想什么呢，早点睡哦' : '喵？在想什么呢～', {
      priority: 'ambient',
      key: 'typing-notice',
      cooldownMs: 30_000,
    });
  }, [typingPhase]);

  // 排队提示：新消息入队时播报一次（Agent 忙时只出气泡，空闲时切 waiting 姿态）
  const prevQueueCountRef = useRef(queuedCount);
  useEffect(() => {
    const prev = prevQueueCountRef.current;
    prevQueueCountRef.current = queuedCount;
    if (queuedCount <= prev) return;
    useMascotStore.getState().interact(runningRef.current ? null : 'waiting', {
      priority: 'work',
      key: 'queue-notice',
      cooldownMs: 15_000,
      bubble: `有 ${queuedCount} 条消息在排队喵`,
      holdMs: 3500,
    });
  }, [queuedCount]);

  // 工具失败后又有工具开跑：播报「再试一次」（工作级，只出气泡不抢姿态）
  useEffect(() => {
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.kind !== 'tool') continue;
      if (e.status === 'running') {
        if (sawFailedToolRef.current) {
          sawFailedToolRef.current = false;
          useMascotStore.getState().say('喵！再试一次！', { priority: 'work', key: 'tool-retry', cooldownMs: 8000 });
        }
      } else if (e.status === 'failed') {
        sawFailedToolRef.current = true;
      } else {
        sawFailedToolRef.current = false;
      }
      return; // 只看最新的工具卡
    }
  }, [entries]);

  // ask_user 提问卡：追踪当前待回答的提问 id
  useEffect(() => {
    const pending = entries.find(
      (e): e is Extract<Entry, { kind: 'question' }> =>
        e.kind === 'question' && e.answer === undefined && !e.declined,
    );
    setPendingQuestionId(pending?.questionId ?? null);
  }, [entries]);

  // 提问出现：猫猫招手把话筒递给用户；15s 未答轻催一次
  useEffect(() => {
    if (!pendingQuestionId) return;
    useMascotStore.getState().interact('beckon', {
      priority: 'work',
      key: 'ask-user',
      bubble: '轮到你啦喵！',
      holdMs: 3000,
    });
    const t = window.setTimeout(() => {
      const still = useAgentStore
        .getState()
        .entries.some(
          (e) =>
            e.kind === 'question' &&
            e.questionId === pendingQuestionId &&
            e.answer === undefined &&
            !e.declined,
        );
      if (still) {
        useMascotStore.getState().interact('beckon', {
          priority: 'work',
          key: 'ask-user-nudge',
          bubble: '喵…还在等你的回答哦',
          holdMs: 2500,
        });
      }
    }, 15_000);
    return () => window.clearTimeout(t);
  }, [pendingQuestionId]);

  // 目光跟随：切回窗口时抬头欢迎一次（冷却防打扰）；切走保持当前姿势，不反复呼唤
  useEffect(() => {
    const onFocus = () => {
      lastActiveRef.current = Date.now();
      if (runningRef.current) return; // 工作演出不打扰
      useMascotStore.getState().interact('notice', {
        priority: 'ambient',
        key: 'window-focus',
        cooldownMs: 90_000,
        bubble: isNight() ? '这么晚还在忙喵…早点睡哦' : '喵呜～回来啦',
        holdMs: 2500,
      });
    };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, []);

  // 活跃期（运行中 / 刚完成）相似图轮换：每 VARIATION_MS 前进一格
  useEffect(() => {
    if (!running && !justFinished) return;
    const timer = window.setInterval(() => setTick((k) => k + 1), VARIATION_MS);
    return () => window.clearInterval(timer);
  }, [running, justFinished]);

  // 气泡自动消失
  useEffect(() => {
    if (bubble) {
      bubbleTimerRef.current = window.setTimeout(() => setBubble(null), 2600);
    }
    return () => {
      if (bubbleTimerRef.current !== null) window.clearTimeout(bubbleTimerRef.current);
    };
  }, [bubble]);

  // 外部互动事件：按优先级演出。ambient（氛围彩蛋）在 Agent 忙时不接管姿态、
  // 只出气泡；其余优先级接管姿态 + fx + 气泡，holdMs 后恢复常规推导（读真实状态）
  const lastEventSeqRef = useRef(0);
  useEffect(() => {
    if (!event || event.seq === lastEventSeqRef.current) return;
    lastEventSeqRef.current = event.seq;
    lastActiveRef.current = Date.now();
    if (event.pose && !(event.priority === 'ambient' && runningRef.current)) {
      if (interactTimerRef.current !== null) window.clearTimeout(interactTimerRef.current);
      setInteractPose(event.pose as Pose);
      if (event.fx) setFx(event.fx);
      interactTimerRef.current = window.setTimeout(() => {
        setInteractPose(null);
        setFx(null);
      }, event.holdMs ?? INTERACT_HOLD_MS);
    }
    if (event.bubble) {
      const text = event.bubble;
      setBubble(null); // 重触发定时器
      requestAnimationFrame(() => setBubble(text));
    }
    return () => {
      if (interactTimerRef.current !== null) window.clearTimeout(interactTimerRef.current);
    };
  }, [event]);

  // ===== 基础形态推导 =====
  const base = useMemo<Pose>(() => {
    if (interactPose) return interactPose;
    if (error) return 'error';
    if (fx === 'jump') return 'jump';
    if (flash) return 'surprised';
    if (running) {
      // 从最新条目往回找：思考未结束 → 思考；工具卡 → 按工具类型；默认专注
      for (let i = entries.length - 1; i >= 0; i--) {
        const e = entries[i];
        if (e.kind === 'thinking' && !e.done) return 'thinking';
        if (e.kind === 'tool') {
          if (e.perm?.state === 'pending') return 'question';
          if (e.status === 'denied') return 'angry';
          if (e.status === 'failed') return 'shake';
          if (e.status === 'running') return toolPose(e.toolName);
        }
      }
      return 'focused';
    }
    // 用户输入感知：优先级在 hover / idle 之上，也盖过刚完成的庆祝窗口
    if (typingPhase !== 'off') return typingPhase === 'typing' ? 'typing' : 'notice';
    if (justFinished) {
      // 最后一条助手/终态文本 → 关键词形态
      for (let i = entries.length - 1; i >= 0; i--) {
        const e = entries[i];
        if (e.kind === 'assistant') return analyzeText(e.text);
        if (e.kind === 'terminal') return e.outcome === 'failed' ? 'wronged' : 'done';
      }
      return 'done';
    }
    if (hover) return 'happy';
    if (idle) return 'sleeping';
    return 'curious';
  }, [entries, running, error, justFinished, hover, idle, fx, flash, interactPose, typingPhase]);

  // ===== 活跃期相似图轮换 =====
  const active = running || justFinished;
  const pose = useMemo<Pose>(() => {
    if (!active) return base;
    const set = VARIATIONS[base];
    if (!set || set.length === 0) return base;
    return set[tick % set.length];
  }, [active, base, tick]);

  // ===== 交互 =====
  const clearFx = () => setFx(null);

  const showBubble = (text: string) => {
    setBubble(null); // 重触发定时器
    requestAnimationFrame(() => setBubble(text));
  };

  /** 提示互动方式：不预填文本，用户自己在输入框输入 & 命令 */
  const talk = () => {
    showBubble('喵～在输入框输入 & 跟我互动');
    onFocusInput?.();
  };

  const handleClick = () => {
    lastActiveRef.current = Date.now();
    // 连点彩蛋：2.5s 内戳 4 次猫猫会生气（冷却防止一直气呼呼）
    const now = Date.now();
    const pokes = pokeTimesRef.current.filter((t) => now - t < 2500);
    pokes.push(now);
    pokeTimesRef.current = pokes;
    if (pokes.length >= 4) {
      pokeTimesRef.current = [];
      useMascotStore.getState().interact('angry', {
        priority: 'interact',
        key: 'poke-angry',
        cooldownMs: 8000,
        holdMs: 2000,
        bubble: '别戳啦！！喵！',
      });
      return;
    }
    setFx('pet'); // 单击 = 摸头动画（纯展示）
  };

  const handleDoubleClick = () => {
    lastActiveRef.current = Date.now();
    setFx('jump');
    setFlash(true);
    window.setTimeout(() => setFlash(false), FLASH_MS);
    talk(); // 双击 = 唤起对话
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      lastActiveRef.current = Date.now();
      setFx('jump');
      setFlash(true);
      window.setTimeout(() => setFlash(false), FLASH_MS);
      talk();
    }
  };

  /** 隐藏彩蛋：vanish 演出后真正藏起，占位按钮可叫回 */
  const handleHide = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (vanishTimerRef.current !== null) window.clearTimeout(vanishTimerRef.current);
    setVanishing(true);
    useMascotStore.getState().interact('vanish', {
      priority: 'interact',
      bubble: '我先去休息啦，想我了就点我的位置喵',
      holdMs: 900,
    });
    vanishTimerRef.current = window.setTimeout(() => {
      setVanishing(false);
      setHidden(true);
    }, 900);
  };

  const handleRestore = (e: React.MouseEvent) => {
    e.stopPropagation();
    setHidden(false);
    useMascotStore.getState().interact('wave', {
      priority: 'interact',
      bubble: '喵～我回来啦',
      holdMs: 2000,
    });
  };

  // 卸载时清掉隐藏演出定时器
  useEffect(
    () => () => {
      if (vanishTimerRef.current !== null) window.clearTimeout(vanishTimerRef.current);
    },
    [],
  );

  // 隐藏态：只留一个「叫回来」的小占位
  if (hidden) {
    return (
      <button
        type="button"
        className="mascot mascot--restore"
        onClick={handleRestore}
        title="把猫猫叫回来"
        aria-label="把猫猫叫回来"
      >
        🐱
      </button>
    );
  }

  return (
    <div
      className={`mascot is-${pose}${fx ? ` is-${fx}` : ''}${bubble ? ' has-bubble' : ''}${vanishing ? ' mascot--vanishing' : ''}`}
      role="button"
      tabIndex={0}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onKeyDown={handleKeyDown}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => {
        setHover(false);
        setBubble(null);
      }}
      title={TIP[pose]}
      aria-label={TIP[pose]}
    >
      <img
        className="mascot__img"
        src={IMG[pose]}
        alt=""
        draggable={false}
        onAnimationEnd={clearFx}
      />
      {pose === 'sleeping' && <span className="mascot__zzz">z</span>}
      {fx === 'pet' && <span className="mascot__heart">♥</span>}
      {bubble && <span className="mascot__bubble">{bubble}</span>}
      {/* 悬停出现的「说句话」按钮：明确的唤起对话入口 */}
      {hover && !bubble && !running && (
        <button
          type="button"
          className="mascot__talk"
          onClick={(e) => {
            e.stopPropagation();
            talk();
          }}
          aria-label="用 & 跟猫猫互动"
        >
          & 跟我玩
        </button>
      )}
      {/* 悬停出现的「去休息」按钮：vanish 演出后藏起，可从占位叫回 */}
      {hover && !vanishing && (
        <button
          type="button"
          className="mascot__hide"
          onClick={handleHide}
          aria-label="让猫猫去休息"
          title="让猫猫去休息"
        >
          ×
        </button>
      )}
    </div>
  );
}
