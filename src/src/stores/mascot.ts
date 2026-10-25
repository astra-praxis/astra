/**
 * 吉祥物（猫猫）互动事件总线 + 动作调度器
 *
 * 独立于 agent.ts 的小型 zustand store：Mascot 组件订阅事件做出动画/气泡，
 * 输入框 / 其他入口（& 前缀、点击、Agent 状态联动）通过 interact() / say() 触发。
 *
 * 单独建 store 的原因：agent.ts 受 agent.test.mjs 的 vm mock 约束（只能 import
 * zustand / tauri invoke / listen / workspace 四个模块），互动事件不能塞进
 * agent.ts，否则测试会挂。
 *
 * 调度规则（让动作有触发条件、持续时间与优先级，而不是不停地乱动）：
 * - 优先级 critical > work > interact > ambient：关键状态（错误/取消）永远生效；
 *   氛围彩蛋（ambient）在 Agent 忙时只出气泡、不接管姿态，避免打工作演出
 * - 事件去重 + 冷却：同一 key 在冷却窗口内重复触发只响应一次，防止气泡刷屏
 * - 动作超时：每个短暂动作由 Mascot 侧 holdMs 定时结束，结束后按真实状态恢复
 */
import { create } from 'zustand';

/** 与 Mascot.tsx 的 Pose 保持一致（type-only 引用，运行时不产生循环依赖） */
export type { Pose as MascotPose } from '@/components/agent/Mascot';

/** 动作优先级：
 *  - critical 关键状态：错误、取消等必须明确反馈的事件
 *  - work     工作状态：发送冲刺、排队提示等随 Agent 状态走
 *  - interact 用户主动触发：& 命令、点击（Agent 运行中也允许演出）
 *  - ambient  氛围彩蛋：自动触发；Agent 忙时只出气泡，不抢姿态 */
export type MascotPriority = 'critical' | 'work' | 'interact' | 'ambient';

export interface MascotEvent {
  /** 单调递增序号：即使连续两次内容相同，Mascot 也按 seq 重新触发 */
  seq: number;
  /** null = 只出气泡，不接管姿态（排队提示、状态播报等场景） */
  pose: string | null;
  /** 一次性动画（摸头 / 跳跃） */
  fx?: 'pet' | 'jump';
  /** 气泡文字（短暂展示后消失） */
  bubble?: string;
  /** 姿态保持时长，默认 3000ms */
  holdMs?: number;
  priority: MascotPriority;
}

export interface MascotInteractOptions {
  fx?: 'pet' | 'jump';
  bubble?: string;
  holdMs?: number;
  /** 默认 interact */
  priority?: MascotPriority;
  /** 去重/冷却键：同一事件短时间重复发生只响应一次 */
  key?: string;
  /** 同 key 冷却窗口 ms（与 key 搭配使用；0/缺省 = 不冷却） */
  cooldownMs?: number;
}

interface MascotState {
  event: MascotEvent | null;
  /** 触发一次演出：pose 为 null 表示只出气泡；被冷却拦截时返回 false */
  interact: (pose: string | null, opts?: MascotInteractOptions) => boolean;
  /** 只出气泡不换姿态（状态播报 / 氛围搭话） */
  say: (bubble: string, opts?: Omit<MascotInteractOptions, 'fx' | 'bubble'>) => boolean;
}

let seq = 0;
/** key → 上次触发时间（模块级，组件重建也保留） */
const firedAt = new Map<string, number>();

export const useMascotStore = create<MascotState>((set, get) => ({
  event: null,
  interact: (pose, opts = {}) => {
    const { key, cooldownMs = 0, priority = 'interact' } = opts;
    if (key && cooldownMs > 0) {
      const now = Date.now();
      const last = firedAt.get(key) ?? 0;
      if (now - last < cooldownMs) return false;
      firedAt.set(key, now);
    }
    set({
      event: {
        seq: ++seq,
        pose,
        fx: opts.fx,
        bubble: opts.bubble,
        holdMs: opts.holdMs,
        priority,
      },
    });
    return true;
  },
  say: (bubble, opts = {}) => get().interact(null, { ...opts, bubble }),
}));
