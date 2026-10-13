import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';

async function load(path) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  });
  const exports = {};
  vm.runInNewContext(outputText, { exports, setTimeout, clearTimeout });
  return exports;
}
const { createOutputScroll } = await load('./outputScroll.ts');
const { createLatestValue } = await load('./latestValue.ts');
function clock() {
  let id = 0;
  const jobs = new Map();
  return {
    jobs,
    schedule: (fn) => { jobs.set(++id, fn); return id; },
    cancel: (id) => jobs.delete(id),
    tick: () => { const callbacks = [...jobs.values()]; jobs.clear(); callbacks.forEach(fn => fn()); },
  };
}
function fixture() {
  const time = clock();
  let top = 0;
  const el = { scrollHeight: 2000, clientHeight: 500,
    get scrollTop() { return top; },
    set scrollTop(value) { top = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight)); },
  };
  const scroll = createOutputScroll(() => el, time.schedule, time.cancel);
  return { time, el, scroll };
}
test('100 output notifications coalesce to one frame and read the latest height', () => {
  const { time, el, scroll } = fixture();
  for (let i = 0; i < 100; i++) scroll.schedule();
  assert.equal(time.jobs.size, 1);
  el.scrollHeight = 3000;
  time.tick();
  assert.equal(el.scrollTop, 2500);
  assert.equal(time.jobs.size, 0);
});
test('upward intent cancels queued work before it can pull the reader down', () => {
  const { time, el, scroll } = fixture();
  scroll.schedule();
  scroll.pause();
  time.tick();
  assert.equal(el.scrollTop, 0);
  scroll.schedule();
  assert.equal(time.jobs.size, 0);
});
test('upward scroll within threshold pauses, downward return resumes', () => {
  const { time, el, scroll } = fixture();
  scroll.schedule(); time.tick(); scroll.onScroll();
  el.scrollTop -= 20; scroll.onScroll();
  scroll.schedule(); assert.equal(time.jobs.size, 0);
  el.scrollTop += 20; scroll.onScroll();
  el.scrollHeight += 200;
  scroll.schedule(); time.tick();
  assert.equal(el.scrollTop, 1700);
});
test('height growth alone does not disable follow; a collapsing block is not user intent', () => {
  const { time, el, scroll } = fixture();
  scroll.schedule(); time.tick();
  el.scrollHeight = 1000; el.scrollTop = 500; scroll.onScroll();
  el.scrollHeight = 3000; scroll.schedule(); time.tick();
  assert.equal(el.scrollTop, 2500);
});
test('unmount cancels frames and explicit session/send reset can resume', () => {
  const { time, el, scroll } = fixture();
  scroll.schedule(); scroll.dispose(); time.tick();
  assert.equal(el.scrollTop, 0);
  scroll.pause(); scroll.resume(); scroll.schedule(); time.tick();
  assert.equal(el.scrollTop, 1500);
});
test('continuous text is throttled, not debounced; newest full snapshot is lossless', () => {
  const time = clock();
  const seen = [];
  const stream = createLatestValue(value => seen.push(value), 100, time.schedule, time.cancel);
  for (let i = 1; i <= 100; i++) stream.push('x'.repeat(i));
  assert.equal(time.jobs.size, 1);
  time.tick();
  assert.deepEqual(seen, ['x'.repeat(100)]);
  stream.push('final'); time.tick();
  assert.equal(seen.at(-1), 'final');
});
test('finish/unmount cancels pending snapshots and cannot overwrite final text later', () => {
  const time = clock();
  const seen = [];
  const stream = createLatestValue(value => seen.push(value), 100, time.schedule, time.cancel);
  stream.push('partial'); stream.dispose(); time.tick();
  assert.deepEqual(seen, []);
  stream.push('next stream'); time.tick();
  assert.deepEqual(seen, ['next stream']);
});
