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
 *   双击 = 跳起来 + 预填一句问候语并聚焦输入框（唤起 agent 对话）
 *   悬停 = 开心 + 出现「说句话」按钮，点击同双击（预填问候 + 聚焦）
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { useAgentStore } from '@/stores/agent';
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
  typing: ['typing', 'laptop', 'focused'],
  laptop: ['laptop', 'typing', 'focused'],
  search: ['search', 'thinking', 'reading'],
  reading: ['reading', 'thinking', 'curious'],
  tools: ['tools', 'idea', 'focused'],
  focused: ['focused', 'laptop', 'thinking'],
  thinking: ['thinking', 'focused', 'idea'],
  question: ['question', 'beckon', 'tilt'],
  waiting: ['waiting', 'loading', 'curious'],
  loading: ['loading', 'waiting', 'test'],
  backpack: ['backpack', 'tools', 'done'],
  done: ['done', 'happy', 'wink'],
  happy: ['happy', 'wink', 'curious'],
  notice: ['notice', 'wave', 'happy'],
  test: ['test', 'loading', 'focused'],
  error: ['error', 'crash', 'wronged'],
  wronged: ['wronged', 'shake', 'error'],
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

/** 点击「说句话」时预填的问候语（唤起对话） */
const GREETINGS = [
  '喵，想聊点什么？',
  '嘿，需要我帮忙吗？',
  '今天想让我做点什么？',
  '我在呢，有事尽管说～',
];

const IDLE_SLEEP_MS = 90_000;
const HAPPY_MS = 4000;
const FLASH_MS = 900;
const VARIATION_MS = 2500;

interface Props {
  /** 唤起对话：预填问候语到输入框并聚焦（由 AgentPage 注入 setInput + focusEnd） */
  onTalk?: (greeting: string) => void;
}

export function Mascot({ onTalk }: Props) {
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const error = useAgentStore((s) => s.error);
  const entries = useAgentStore((s) => s.entries);

  const [hover, setHover] = useState(false);
  const [justFinished, setJustFinished] = useState(false);
  const [idle, setIdle] = useState(false);
  // 一次性动画（单击摸头 / 双击跳跃）；jump 同时把形态切成跳跃姿态
  const [fx, setFx] = useState<'pet' | 'jump' | null>(null);
  // 双击惊吓闪态：短暂切成惊讶表情
  const [flash, setFlash] = useState(false);
  // 气泡文字（问候/对话提示）
  const [bubble, setBubble] = useState<string | null>(null);
  // 活跃期相似图轮换 tick
  const [tick, setTick] = useState(0);

  const prevRunningRef = useRef(running);
  const lastActiveRef = useRef(Date.now());
  const bubbleTimerRef = useRef<number | null>(null);

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

  // ===== 基础形态推导 =====
  const base = useMemo<Pose>(() => {
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
  }, [entries, running, error, justFinished, hover, idle, fx, flash]);

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

  /** 唤起对话：预填随机问候 + 聚焦输入框 + 气泡提示 */
  const talk = () => {
    if (!onTalk) return;
    const greeting = GREETINGS[Math.floor(Math.random() * GREETINGS.length)];
    onTalk(greeting);
    showBubble('想聊就按回车发送喵～');
  };

  const handleClick = () => {
    lastActiveRef.current = Date.now();
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

  return (
    <div
      className={`mascot is-${pose}${fx ? ` is-${fx}` : ''}${bubble ? ' has-bubble' : ''}`}
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
          aria-label="说句话"
        >
          💬 说句话
        </button>
      )}
    </div>
  );
}
