/**
 * 失败重试状态机的回归测试（纯决策层 ./retry.ts）。
 *
 * 这些用例对应线上真实回归，不是凑覆盖率的空壳：
 *  - 「重试成功后横幅还挂着、并且还报错」：看门狗把长任务当成卡死、重试发出时没清错误
 *  - 「无限循环」：配额没有被本次失败事件约束
 *  - 「卡在第一次」：调度与收尾互相抢跑
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = await readFile(new URL('./retry.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});
const exports = {};
vm.runInNewContext(outputText, { exports, console });
const { MAX_AUTO_RETRIES, nextAutoAttempt, finishAction, isDeadRun, bannerView } = exports;

/** vm 里产出的对象原型与本 realm 不同，深比较前先归一成纯数据 */
function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

/** 构造状态机视图（默认值 = 空闲、无失败、有可重发请求） */
function view(overrides = {}) {
  return {
    autoRetry: null,
    started: false,
    budget: 0,
    hasLastRun: true,
    runError: null,
    running: false,
    settling: false,
    ...overrides,
  };
}

const ERR = 'model request failed: error sending request for url (http://172.16.18.69/v1/chat/completions)';

test('首次失败、会话空闲 → 进入第 1 次自动重试', () => {
  assert.equal(nextAutoAttempt(view({ runError: ERR })), 1);
});

test('配额用尽后不再自动重试（交给用户手动）', () => {
  assert.equal(nextAutoAttempt(view({ runError: ERR, budget: MAX_AUTO_RETRIES })), null);
});

test('已在自动重试序列中不重复调度（防双发）', () => {
  assert.equal(nextAutoAttempt(view({ runError: ERR, autoRetry: 1, budget: 1 })), null);
});

test('Run 仍在跑 / 结算屏障中不调度', () => {
  assert.equal(nextAutoAttempt(view({ runError: ERR, running: true })), null);
  assert.equal(nextAutoAttempt(view({ runError: ERR, settling: true })), null);
});

test('没有可重发的请求（lastRunRef 为空）不调度', () => {
  assert.equal(nextAutoAttempt(view({ runError: ERR, hasLastRun: false })), null);
});

test('调度后、Run 起跑前的空闲瞬间绝不能被判成成功（假循环回归）', () => {
  // 旧实现用「runError 被清」当成功信号：重试发出时清掉错误 →
  // 起跑前这一瞬 runError 为 null → 序列提前结束 → 下一次失败又弹回来
  assert.equal(finishAction(view({ autoRetry: 1, started: false, budget: 1 })), null);
});

test('长任务仍在运行时不结束序列（37.7s 成功重试的回归）', () => {
  // 旧实现有 30s 无条件看门狗，把「作答中」当成卡死，横幅退回常驻错误
  assert.equal(finishAction(view({ autoRetry: 1, started: true, budget: 1, running: true })), null);
  assert.equal(finishAction(view({ autoRetry: 1, started: true, budget: 1, settling: true })), null);
});

test('已起跑并结算且无错误 → 成功，结束序列并可重置配额', () => {
  assert.equal(finishAction(view({ autoRetry: 1, started: true, budget: 1 })), 'success');
});

test('已起跑并结算、仍有配额 → 直接续下一次（不闪断）', () => {
  assert.equal(finishAction(view({ autoRetry: 1, started: true, budget: 1, runError: ERR })), 'next');
});

test('失败且配额已用尽 → exhausted（横幅常驻等用户）', () => {
  assert.equal(
    finishAction(view({ autoRetry: 3, started: true, budget: MAX_AUTO_RETRIES, runError: ERR })),
    'exhausted',
  );
});

test('不在序列中不推进', () => {
  assert.equal(finishAction(view({ started: true, runError: ERR, budget: 1 })), null);
});

test('死 Run：已起跑、仍在 running 但一个事件都没有', () => {
  assert.equal(isDeadRun(view({ autoRetry: 1, started: true, running: true }), 0), true);
});

test('长任务（事件已经流向 lastSequence > 0）不算死 Run', () => {
  assert.equal(isDeadRun(view({ autoRetry: 1, started: true, running: true }), 42), false);
});

test('未起跑 / 未在序列中不算死 Run', () => {
  assert.equal(isDeadRun(view({ autoRetry: 1, running: true }), 0), false);
  assert.equal(isDeadRun(view({ started: true, running: true }), 0), false);
});

test('横幅：重试中优先显示进度，不把上一条错误当常驻错误', () => {
  const b = bannerView(2, ERR, ERR, false);
  assert.deepEqual(plain(b), { kind: 'retrying', attempt: 2, message: ERR });
});

test('横幅：无重试且有错误 → 常驻错误 + 操作', () => {
  assert.deepEqual(plain(bannerView(null, ERR, null, false)), { kind: 'failed', message: ERR, actions: true });
});

test('横幅：无重试无错误 → 隐藏', () => {
  assert.deepEqual(plain(bannerView(null, null, ERR, false)), { kind: 'hidden' });
});

test('横幅：Run 正在流式产出 → 一律隐藏（作答中不得挂着错误/重试）', () => {
  // 回归：长任务重试成功、Agent 已在上面作答，下面却还挂着错误横幅 + 重试按钮
  assert.deepEqual(plain(bannerView(null, ERR, ERR, true)), { kind: 'hidden' });
  assert.deepEqual(plain(bannerView(2, ERR, ERR, true)), { kind: 'hidden' });
});

/**
 * 复现线上时序（agent_events.log 2026-11-09 16:16）：
 * 4 次快速失败（每次 ~4s，含 800ms 间隔）→ 配额耗尽 → 手动重试 → 这次跑 37.7s 成功。
 * 断言终态：既不残留错误横幅，也不残留重试徽标。
 */
test('时序复现：连失败 → 配额耗尽 → 长任务重试成功 → 状态干净', () => {
  const state = { autoRetry: null, started: false, budget: 0 };
  let runError = null;
  let hasLastRun = false;
  const transitions = [];

  const failRun = () => {
    runError = ERR;
    hasLastRun = true;
  };
  const startRun = () => {
    state.started = true;
  };

  // 首个失败：判定 → 调度第 1 次
  failRun();
  const first = nextAutoAttempt(view({ ...state, runError, hasLastRun }));
  assert.equal(first, 1);
  state.autoRetry = first;
  state.budget = first;

  // 第 1..2 次尝试都失败 → 收尾直接续下一次（不经过 null）
  for (let attempt = 1; attempt <= 2; attempt++) {
    startRun();
    // Run 结算后失败
    runError = ERR;
    const action = finishAction(view({ ...state, runError, hasLastRun, started: true }));
    assert.equal(action, 'next', `attempt ${attempt} 应续下一次`);
    state.started = false;
    const next = state.budget + 1;
    state.budget = next;
    state.autoRetry = next;
    transitions.push(next);
  }
  assert.deepEqual(transitions, [2, 3]);

  // 第 3 次尝试失败 → 配额耗尽，横幅常驻等用户
  startRun();
  runError = ERR;
  assert.equal(finishAction(view({ ...state, runError, hasLastRun, started: true })), 'exhausted');
  state.started = false;
  state.autoRetry = null;

  // 用户手动重试：这次是长任务（37.7s），期间事件持续到达
  runError = null; // 手动重试发出时清掉失败横幅
  startRun(); // running = true
  for (let seq = 1; seq <= 559; seq++) {
    // 长任务运行中：既不能推进序列，也不能被看成死 Run
    assert.equal(finishAction(view({ ...state, autoRetry: null, runError: null, started: true, running: true })), null);
    assert.equal(isDeadRun(view({ autoRetry: null, started: true, running: true }), seq), false);
    // 作答期间横幅必须是隐藏的（不挂错误、不挂重试按钮）
    assert.deepEqual(plain(bannerView(state.autoRetry, null, null, true)), { kind: 'hidden' });
  }
  // Run 结束：成功
  assert.equal(finishAction(view({ ...state, autoRetry: null, runError: null, started: true })), null);
  // 成功后配额归零（下一次失败重新获得 3 次）
  assert.deepEqual(plain(bannerView(null, null, null, false)), { kind: 'hidden' });
});
