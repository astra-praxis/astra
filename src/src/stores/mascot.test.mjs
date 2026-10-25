/**
 * mascot.ts 动作调度器单测
 *
 * 与 agent.test.mjs 同一套 vm + ts.transpileModule 模式：把 TS 源码转译成
 * CommonJS，在沙箱里 require 真实 zustand，Date.now 用可拨动的假钟。
 * 注意：zustand set 产生新 state 对象，断言前必须重新 getState()。
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const source = await readFile(new URL('./mascot.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});

/** 加载 store；now 为可拨动的假钟（毫秒） */
function load(now = () => 1_000_000) {
  const exports = {};
  vm.runInNewContext(outputText, {
    exports,
    console,
    // 沙箱内遮蔽全局 Date：调度器只用 Date.now()
    Date: { now },
    require: (id) => {
      if (id === 'zustand') return require('zustand');
      throw new Error(id);
    },
  });
  return exports.useMascotStore;
}

test('interact emits event with seq increment, fields and default priority', () => {
  const mascot = load();
  assert.equal(mascot.getState().event, null);
  const ok = mascot.getState().interact('run', { fx: 'jump', bubble: '喵！', holdMs: 1200 });
  assert.equal(ok, true);
  const ev = mascot.getState().event;
  // 跨 vm 沙箱对象原型不同，deepStrictEqual 会误报，逐字段断言
  assert.equal(ev.seq, 1);
  assert.equal(ev.pose, 'run');
  assert.equal(ev.fx, 'jump');
  assert.equal(ev.bubble, '喵！');
  assert.equal(ev.holdMs, 1200);
  assert.equal(ev.priority, 'interact');
  mascot.getState().interact('waiting');
  assert.equal(mascot.getState().event.seq, 2);
  assert.equal(mascot.getState().event.fx, undefined);
  assert.equal(mascot.getState().event.priority, 'interact');
});

test('cooldown: same key within window is dropped, after window fires again', () => {
  let now = 1_000_000;
  const mascot = load(() => now);
  assert.equal(mascot.getState().interact('notice', { key: 'typing-notice', cooldownMs: 30_000 }), true);
  const firstSeq = mascot.getState().event.seq;
  // 窗口内重复触发：被拦截，seq 不前进
  assert.equal(mascot.getState().interact('notice', { key: 'typing-notice', cooldownMs: 30_000 }), false);
  assert.equal(mascot.getState().event.seq, firstSeq);
  now += 29_999;
  assert.equal(mascot.getState().interact('notice', { key: 'typing-notice', cooldownMs: 30_000 }), false);
  assert.equal(mascot.getState().event.seq, firstSeq);
  now += 1;
  assert.equal(mascot.getState().interact('notice', { key: 'typing-notice', cooldownMs: 30_000 }), true);
  assert.equal(mascot.getState().event.seq, firstSeq + 1);
});

test('cooldown is per key; events without key always fire', () => {
  let now = 1_000_000;
  const mascot = load(() => now);
  assert.equal(mascot.getState().interact('run', { key: 'send-sprint', cooldownMs: 5000 }), true);
  assert.equal(mascot.getState().interact('run', { key: 'other', cooldownMs: 5000 }), true);
  assert.equal(mascot.getState().interact('run'), true);
  assert.equal(mascot.getState().event.seq, 3);
  now += 1000;
  assert.equal(mascot.getState().interact('run', { key: 'send-sprint', cooldownMs: 5000 }), false);
  now += 5000;
  assert.equal(mascot.getState().interact('run', { key: 'send-sprint', cooldownMs: 5000 }), true);
});

test('say emits bubble-only event without pose', () => {
  const mascot = load();
  mascot.getState().say('轮到你啦喵。', { priority: 'work', key: 'queue-notice', cooldownMs: 15_000 });
  assert.equal(mascot.getState().event.pose, null);
  assert.equal(mascot.getState().event.bubble, '轮到你啦喵。');
  assert.equal(mascot.getState().event.priority, 'work');
  assert.equal(mascot.getState().event.fx, undefined);
});

test('explicit priority passes through; cooldown drops say too', () => {
  let now = 1_000_000;
  const mascot = load(() => now);
  mascot.getState().interact('run', { priority: 'critical' });
  assert.equal(mascot.getState().event.priority, 'critical');
  mascot.getState().say('喵？', { key: 'typing-notice', cooldownMs: 1000 });
  assert.equal(mascot.getState().event.bubble, '喵？');
  now += 500;
  assert.equal(mascot.getState().say('喵？', { key: 'typing-notice', cooldownMs: 1000 }), false);
  assert.equal(mascot.getState().event.bubble, '喵？');
});
