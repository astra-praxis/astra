import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const source = await readFile(new URL('./mobileLogic.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});
function load() {
  const exports = {};
  vm.runInNewContext(outputText, {
    exports,
    console,
    require: (id) => {
      throw new Error(`mobileLogic 应保持零 import，收到：${id}`);
    },
  });
  return exports;
}

const SKILLS = [
  { name: 'demo', description: '演示技能' },
  { name: 'review', description: '代码评审' },
  { name: '写周报', description: '中文技能名' },
];

test('parseMobileSend: plain text passes through with skill null', () => {
  const { parseMobileSend } = load();
  const r = parseMobileSend('  帮我把首页改圆角  ', SKILLS);
  assert.equal(r.ok, true);
  assert.equal(r.raw, '帮我把首页改圆角');
  assert.equal(r.text, '帮我把首页改圆角');
  assert.equal(r.skill, null);
});

test('parseMobileSend: known skill is split into prefix and task', () => {
  const { parseMobileSend } = load();
  const r = parseMobileSend('/demo 把需求整理成方案', SKILLS);
  assert.equal(r.ok, true);
  assert.equal(r.raw, '/demo 把需求整理成方案');
  assert.equal(r.text, '把需求整理成方案');
  assert.equal(r.skill, 'demo');
  const zh = parseMobileSend('/写周报 本周完成手机遥控', SKILLS);
  assert.equal(zh.ok, true);
  assert.equal(zh.skill, '写周报');
  assert.equal(zh.text, '本周完成手机遥控');
});

test('parseMobileSend: unknown skill is rejected with hint', () => {
  const { parseMobileSend } = load();
  const r = parseMobileSend('/nope 干活', SKILLS);
  assert.equal(r.ok, false);
  assert.match(r.error, /未知技能 \/nope/);
  // 路径形文本没有空白分隔，同样按未知技能拒发（与桌面一致）
  const path = parseMobileSend('/Users/han/notes.md', SKILLS);
  assert.equal(path.ok, false);
});

test('parseMobileSend: bare skill or empty input is rejected', () => {
  const { parseMobileSend } = load();
  const bare = parseMobileSend('/demo', SKILLS);
  assert.equal(bare.ok, false);
  assert.match(bare.error, /请输入任务内容/);
  const bareSpace = parseMobileSend('/demo   ', SKILLS);
  assert.equal(bareSpace.ok, false);
  const empty = parseMobileSend('   ', SKILLS);
  assert.equal(empty.ok, false);
  assert.match(empty.error, /内容为空/);
});

const CTX = {
  activeId: 'a1',
  isDraft: false,
  entriesLen: 5,
  busy: false,
  knownSessionIds: ['a1', 'a2', 'a3'],
  runningIds: [],
};

test('planEnqueue: null target reuses empty draft, otherwise opens new tab', () => {
  const { planEnqueue } = load();
  // 注意：vm 跨 realm 对象原型不同，断言只能比字段不能 deepEqual
  let plan = planEnqueue({ ...CTX, isDraft: true, entriesLen: 0 }, null);
  assert.equal(plan.action, 'reuse-current');
  plan = planEnqueue(CTX, null);
  assert.equal(plan.action, 'new-tab');
  // busy 时新开 tab 也会被 no-op → 直接拒绝
  plan = planEnqueue({ ...CTX, busy: true }, null);
  assert.equal(plan.action, 'blocked');
  assert.equal(plan.id, null);
});

test('planEnqueue: while flat session runs, null target queues into the runner', () => {
  const { planEnqueue } = load();
  // flat 运行中：手机发「新会话」消息 → 排进正在跑的会话（Zed 式排队）
  const plan = planEnqueue({ ...CTX, runningIds: ['a1'] }, null);
  assert.equal(plan.action, 'enqueue-active');
  // 准入被驻留会话占着：null 目标拒收，避免新 tab 首条注定失败
  const parked = planEnqueue({ ...CTX, runningIds: ['a2'] }, null);
  assert.equal(parked.action, 'blocked');
});

test('planEnqueue: active target queues while it runs; blocked when another run holds admission', () => {
  const { planEnqueue } = load();
  assert.equal(planEnqueue({ ...CTX, runningIds: ['a1'] }, 'a1').action, 'enqueue-active');
  assert.equal(planEnqueue(CTX, 'a1').action, 'enqueue-active');
  // 激活会话空闲但驻留会话占着准入 → 排了也会立刻触发一次注定失败的 dispatch → 拒
  assert.equal(planEnqueue({ ...CTX, runningIds: ['a2'] }, 'a1').action, 'blocked');
});

test('planEnqueue: parked/history target activates; runner target activates; unknown missing; other runs block', () => {
  const { planEnqueue } = load();
  // 无运行：驻留/历史会话正常激活
  let plan = planEnqueue(CTX, 'a2');
  assert.equal(plan.action, 'activate');
  assert.equal(plan.id, 'a2');
  // 目标本身就是运行中的驻留会话：切到前台排队续发
  plan = planEnqueue({ ...CTX, runningIds: ['a2'] }, 'a2');
  assert.equal(plan.action, 'activate');
  assert.equal(plan.id, 'a2');
  // 目标空闲但别的会话（flat 或驻留）占着准入 → 拒
  plan = planEnqueue({ ...CTX, runningIds: ['a1'] }, 'a2');
  assert.equal(plan.action, 'blocked');
  assert.equal(plan.id, 'a2');
  plan = planEnqueue({ ...CTX, runningIds: ['a3'] }, 'a2');
  assert.equal(plan.action, 'blocked');
  // 未知会话
  plan = planEnqueue(CTX, 'ghost');
  assert.equal(plan.action, 'missing');
  assert.equal(plan.id, 'ghost');
});

test('planEnqueue: direct flag separates idle direct-send from running queue', () => {
  const { planEnqueue } = load();
  // 全局无运行：四条可执行路径全部直发（跳过队列）
  assert.equal(planEnqueue({ ...CTX, isDraft: true, entriesLen: 0 }, null).direct, true);
  assert.equal(planEnqueue(CTX, null).direct, true);
  assert.equal(planEnqueue(CTX, 'a1').direct, true);
  assert.equal(planEnqueue(CTX, 'a2').direct, true);
  // 目标在跑（flat 或驻留）→ direct false，仍走队列续发
  assert.equal(planEnqueue({ ...CTX, runningIds: ['a1'] }, null).direct, false);
  assert.equal(planEnqueue({ ...CTX, runningIds: ['a1'] }, 'a1').direct, false);
  assert.equal(planEnqueue({ ...CTX, runningIds: ['a2'] }, 'a2').direct, false);
  // blocked / missing 不执行，无 direct 字段
  assert.equal('direct' in planEnqueue({ ...CTX, runningIds: ['a2'] }, 'a1'), false);
  assert.equal('direct' in planEnqueue(CTX, 'ghost'), false);
});

test('sanitizeQueueSnapshot strips images and keeps the rest', () => {
  const { sanitizeQueueSnapshot } = load();
  const items = [
    { id: 'q1', raw: '/demo hi', text: 'hi', skill: 'demo', sessionId: 'a1', images: [{ data: 'xxx', width: 1, height: 1 }] },
    { id: 'q2', raw: 'plain', text: 'plain', skill: null, sessionId: 'a2' },
    { id: 'q3', raw: '手机消息', text: '手机消息', skill: null, sessionId: 'a1', sender: '乙' },
  ];
  const out = sanitizeQueueSnapshot(items);
  assert.equal(out.length, 3);
  assert.equal('images' in out[0], false);
  // 跨 realm 对象不能 deepEqual，用 JSON 串比较
  assert.equal(JSON.stringify(out[0]), JSON.stringify({ id: 'q1', raw: '/demo hi', text: 'hi', skill: 'demo', sessionId: 'a1' }));
  assert.equal(JSON.stringify(out[1]), JSON.stringify({ id: 'q2', raw: 'plain', text: 'plain', skill: null, sessionId: 'a2' }));
  // 手机入队的消息：sender 透传给桥（多设备时手机端展示来源）
  assert.equal(JSON.stringify(out[2]), JSON.stringify({ id: 'q3', raw: '手机消息', text: '手机消息', skill: null, sessionId: 'a1', sender: '乙' }));
});

test('shouldActivateAfterSettle only for parked sessions with pending queue items', () => {
  const { shouldActivateAfterSettle } = load();
  const queue = [
    { sessionId: 'a2' },
    { sessionId: 'a3' },
    { sessionId: 'a1' },
  ];
  // 激活会话自己结算 → AgentPage 既有 effect 负责，不需要换前台
  assert.equal(shouldActivateAfterSettle(queue, 'a1', 'a1'), false);
  // 驻留会话结算且有排队 → 拉回前台触发续发
  assert.equal(shouldActivateAfterSettle(queue, 'a2', 'a1'), true);
  // 驻留会话结算但无排队 → 不动
  assert.equal(shouldActivateAfterSettle(queue, 'a4', 'a1'), false);
});

test('normalizeEffort accepts the four efforts and rejects everything else', () => {
  const { normalizeEffort } = load();
  assert.equal(normalizeEffort('minimal'), 'minimal');
  assert.equal(normalizeEffort('low'), 'low');
  assert.equal(normalizeEffort('medium'), 'medium');
  assert.equal(normalizeEffort('high'), 'high');
  assert.equal(normalizeEffort('ultra'), null);
  assert.equal(normalizeEffort(undefined), null);
  assert.equal(normalizeEffort(42), null);
});

test('normalizeAutoMode accepts the four modes and rejects everything else', () => {
  const { normalizeAutoMode } = load();
  assert.equal(normalizeAutoMode('off'), 'off');
  assert.equal(normalizeAutoMode('project'), 'project');
  assert.equal(normalizeAutoMode('workspace'), 'workspace');
  assert.equal(normalizeAutoMode('all'), 'all');
  assert.equal(normalizeAutoMode('everything'), null);
  assert.equal(normalizeAutoMode(null), null);
});

test('planModelSelection: local source must hit a running port', () => {
  const { planModelSelection } = load();
  const base = { runningPorts: [8080, 9090], providerIds: ['p1'] };
  // 跨 realm 对象不能 deepEqual，只断言字段
  let plan = planModelSelection({ ...base, source: 'model', port: 8080, provider_id: null, model: null });
  assert.equal(plan.ok, true);
  assert.equal(plan.kind, 'local');
  assert.equal(plan.port, 8080);
  plan = planModelSelection({ ...base, source: 'model', port: 1234, provider_id: null, model: null });
  assert.equal(plan.ok, false);
  assert.match(plan.error, /未在 PC 上运行/);
  plan = planModelSelection({ ...base, source: 'model', port: null, provider_id: null, model: null });
  assert.equal(plan.ok, false);
  plan = planModelSelection({ ...base, source: 'model', port: '8080', provider_id: null, model: null });
  assert.equal(plan.ok, false);
});

test('planModelSelection: provider source requires known provider; model trimmed pass-through', () => {
  const { planModelSelection } = load();
  const base = { runningPorts: [8080], providerIds: ['p1', 'p2'] };
  let plan = planModelSelection({ ...base, source: 'provider', port: null, provider_id: 'p2', model: ' gpt-x ' });
  assert.equal(plan.ok, true);
  assert.equal(plan.kind, 'provider');
  assert.equal(plan.providerId, 'p2');
  assert.equal(plan.model, 'gpt-x');
  // 切 provider 不带 model（两步流第一步：桌面拉模型列表后重推 config）
  plan = planModelSelection({ ...base, source: 'provider', port: null, provider_id: 'p1', model: null });
  assert.equal(plan.ok, true);
  assert.equal(plan.model, null);
  // 空白 model 等同于不带
  plan = planModelSelection({ ...base, source: 'provider', port: null, provider_id: 'p1', model: '   ' });
  assert.equal(plan.model, null);
  plan = planModelSelection({ ...base, source: 'provider', port: null, provider_id: 'ghost', model: null });
  assert.equal(plan.ok, false);
  assert.match(plan.error, /提供者不存在/);
  plan = planModelSelection({ ...base, source: 'bogus', port: null, provider_id: null, model: null });
  assert.equal(plan.ok, false);
  assert.match(plan.error, /未知的模型来源/);
});

// ===== createEnqueueDeduper：手机入队消息去重 =====

test('deduper: same msg_id delivered twice is dropped once (duplicate delivery)', () => {
  const { createEnqueueDeduper } = load();
  const d = createEnqueueDeduper();
  assert.equal(d.duplicate(1, '你好', 's1', 1000), false);
  // 同 msg_id 重复投递（监听器重复注册等）→ 丢弃
  assert.equal(d.duplicate(1, '你好', 's1', 1100), true);
  assert.equal(d.duplicate(1, '你好', 's1', 99999), true);
  // 新 msg_id 正常放行
  assert.equal(d.duplicate(2, '再见', 's1', 1200), false);
});

test('deduper: identical text within 1500ms window is dropped (ghost double-tap)', () => {
  const { createEnqueueDeduper } = load();
  const d = createEnqueueDeduper();
  assert.equal(d.duplicate(10, '跑任务', 's1', 1000), false);
  // 不同 msg_id 但同会话同文本、窗口内 → 兜底丢弃
  assert.equal(d.duplicate(11, '跑任务', 's1', 2000), true);
  // 窗口外放行（合法的同文本重发）
  assert.equal(d.duplicate(12, '跑任务', 's1', 2600), false);
  // 不同会话不互斥
  assert.equal(d.duplicate(13, '跑任务', 's2', 2700), false);
  // 不同文本不互斥
  assert.equal(d.duplicate(14, '换个任务', 's2', 2800), false);
});

test('deduper: empty text skips text-window dedup but still dedups msg_id', () => {
  const { createEnqueueDeduper } = load();
  const d = createEnqueueDeduper();
  assert.equal(d.duplicate(null, '', 's1', 1000), false);
  assert.equal(d.duplicate(null, '', 's1', 1100), false); // 空文本不做窗口去重
  assert.equal(d.duplicate(20, '干活', 's1', 1200), false);
  assert.equal(d.duplicate(20, '干活', 's1', 1300), true); // msg_id 去重不受影响
});

test('deduper: LRU caps at 128 entries, oldest msg_id is forgotten', () => {
  const { createEnqueueDeduper } = load();
  const d = createEnqueueDeduper();
  // 灌满 128 个
  for (let i = 0; i < 128; i++) assert.equal(d.duplicate(i, `m${i}`, 's1', 1000 + i), false);
  // 第 129 个把最老的（msg_id=0）挤出
  assert.equal(d.duplicate(128, 'm128', 's1', 1200), false);
  // msg_id=0 已被挤出 → 不再判定为重复（放行）
  assert.equal(d.duplicate(0, 'm0', 's1', 1300), false);
  // 重插 0 时按 LRU 把次老的 1 挤出（正确行为），最近的 128 仍在 → 重复
  assert.equal(d.duplicate(128, 'm128', 's1', 1400), true);
});
