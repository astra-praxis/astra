// 失败重试状态机的**纯决策层**：AgentPage 的 effect 只负责执行动作，
// 所有「该不该重试 / 算不算成功 / 是否卡死」的判断收敛在这里。
//
// 为什么单独抽出来：这套重试逻辑历史上反复回归（无限循环、卡在第一次、
// 报错被提前清掉、长任务被误判），几个判断条件互相耦合，改一处崩另一处。
// 抽成纯函数后可以用 `node --test` 直接覆盖这些场景。
//
// 语义回顾（一次 Run 失败 = 一个「失败事件」）：
//   - 一个失败事件内最多自动重试 MAX_AUTO_RETRIES 次；
//   - Run 正常结束（成功）或用户新发消息才重置配额，手动重试不重置；
//   - 「成功」必须等 Run 真正起跑（started），否则调度后、起跑前的空闲瞬间
//     会被误判成成功 —— 旧版的「报错清了又出现」假循环就是这个。

/** 一个「失败事件」内最多自动重试的次数 */
export const MAX_AUTO_RETRIES = 3;

/** 两次尝试之间的固定等待（ms）。对齐 Zed：瞬时网络/流错误走
 *  FixedDelay { delay: BASE_RETRY_DELAY = 5s, max_attempts: 3 }。
 *  退避期间横幅显示「自动重试中（n/3）」，真实 attempt 开跑后横幅隐藏。 */
export const RETRY_DELAY_MS = 5000;

export interface RetryView {
  /** 自动重试进行中的第 N 次（null = 未在序列中） */
  autoRetry: number | null;
  /** 本次尝试的 Run 是否已真正起跑（观察到 running / settling 翻转） */
  started: boolean;
  /** 本失败事件内已用掉的自动重试次数 */
  budget: number;
  /** 是否还有可重发的请求（lastRunRef） */
  hasLastRun: boolean;
  /** 失败横幅的错误原文（null = 当前没有失败） */
  runError: string | null;
  /** 本会话 Run 正在跑 */
  running: boolean;
  /** 结算屏障中（含 sessionBusy；此时状态还不可信） */
  settling: boolean;
}

/** 判定：是否可以（继续）发起一次自动重试。返回下一次序号；null = 本轮不动。 */
export function nextAutoAttempt(v: RetryView): number | null {
  if (v.autoRetry !== null) return null; // 已在序列中，由 finishAction 推进
  if (!v.runError || v.running || v.settling) return null; // 失败且已结算才调度
  if (v.budget >= MAX_AUTO_RETRIES) return null; // 配额用尽，等用户手动
  if (!v.hasLastRun) return null;
  return v.budget + 1;
}

export type FinishAction = 'success' | 'exhausted' | 'next';

/** 收尾：一次**已起跑**的尝试结算后该怎么走。null = 还不到推进的时候。 */
export function finishAction(v: RetryView): FinishAction | null {
  if (v.autoRetry === null) return null; // 不在自动重试序列中
  if (!v.started) return null; // 还没起跑：绝不当成成功（防假循环）
  if (v.running || v.settling) return null; // 仍在跑 / 等持久化屏障
  if (!v.runError) return 'success';
  return v.budget >= MAX_AUTO_RETRIES ? 'exhausted' : 'next';
}

/** 死 Run 判定：已起跑、仍在 running、且一个事件都没到（后端没接上）。
 *  必须要求 `lastSequence === 0`：长任务（>30s 仍在流式输出）绝不能误伤。 */
export function isDeadRun(v: RetryView, lastSequence: number): boolean {
  return v.autoRetry !== null && v.started && v.running && lastSequence === 0;
}

/** 横幅展示模型：Run 正在流式产出时一律隐藏（其自身就是状态信号，避免
 *  「Agent 在上方作答、下面还挂着错误 + 重试」）；否则重试中显示进度，
 *  再否则显示常驻错误。 */
export type BannerView =
  | { kind: 'hidden' }
  | { kind: 'retrying'; attempt: number; message: string }
  | { kind: 'failed'; message: string; actions: true };

export function bannerView(
  autoRetry: number | null,
  runError: string | null,
  retryError: string | null,
  streaming: boolean,
): BannerView {
  if (streaming) return { kind: 'hidden' };
  if (autoRetry !== null) {
    return { kind: 'retrying', attempt: autoRetry, message: retryError ?? '' };
  }
  if (runError !== null) return { kind: 'failed', message: runError, actions: true };
  return { kind: 'hidden' };
}
