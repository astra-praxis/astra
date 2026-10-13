import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const source = await readFile(new URL('./agent.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});
function load(listen = async () => () => {}, invoke = async () => {}, localStorage = { getItem: () => null }) {
  const exports = {};
  vm.runInNewContext(outputText, {
    exports, console, localStorage,
    require: (id) => {
      if (id === 'zustand') return require('zustand');
      if (id === '@tauri-apps/api/core') return { invoke };
      if (id === '@tauri-apps/api/event') return { listen };
      if (id === '@/stores/workspace') return { useWorkspaceStore: { getState: () => ({ projects: [] }) } };
      throw new Error(id);
    },
  });
  return exports;
}
test('image preference defaults off, persists explicitly and does not replace the session', () => {
  const values = new Map();
  const storage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const store = load(undefined, undefined, storage).useAgentStore;
  assert.equal(store.getState().imageRecognition, false);
  const id = store.getState().sessionId;
  store.getState().setImageRecognition(true);
  assert.equal(store.getState().sessionId, id);
  assert.equal(load(undefined, undefined, storage).useAgentStore.getState().imageRecognition, true);
  store.getState().setImageRecognition(false);
  assert.equal(load(undefined, undefined, storage).useAgentStore.getState().imageRecognition, false);
});
test('thinking preference defaults off/medium, persists to localStorage with invalid fallback', () => {
  const values = new Map();
  const storage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const store = load(undefined, undefined, storage).useAgentStore;
  assert.equal(store.getState().thinkingEnabled, false);
  assert.equal(store.getState().thinkingEffort, 'medium');
  const id = store.getState().sessionId;
  store.getState().setThinkingEnabled(true);
  store.getState().setThinkingEffort('high');
  assert.equal(store.getState().sessionId, id);
  assert.equal(values.get('astra-agent-thinking'), '1');
  assert.equal(values.get('astra-agent-effort'), 'high');
  const reloaded = load(undefined, undefined, storage).useAgentStore.getState();
  assert.equal(reloaded.thinkingEnabled, true);
  assert.equal(reloaded.thinkingEffort, 'high');
  values.set('astra-agent-effort', 'bogus');
  assert.equal(load(undefined, undefined, storage).useAgentStore.getState().thinkingEffort, 'medium');
});

const api = load();
const env = (type, sequence = 1, run_id = 'opaque-uuid', extra = {}) => ({
  schema_version: 2, event_id: `event-${sequence}`, timestamp: 123,
  session_id: 's', run_id, sequence, type, ...extra,
});
const state = () => ({ sessionId: 's', entries: [], running: false, status: null,
  activeRunId: null, lastSequence: 0, endedRuns: new Map() });
const pending = () => [
  { kind: 'assistant', text: 'hello', streaming: true },
  { kind: 'thinking', text: 'think' },
  { kind: 'tool', callId: 't', toolName: 'write', argsSummary: '', status: 'running', perm: { state: 'pending', summary: '' } },
  { kind: 'permission', callId: 'p', state: 'pending' },
  { kind: 'question', questionId: 'q' },
  { kind: 'question', questionId: 'answered', answer: '' },
  { kind: 'tool', callId: 'done', status: 'completed' },
];
function assertClosed(entries) {
  assert.equal(entries[0].streaming, false);
  assert.equal(entries[1].done, true);
  assert.equal(entries[2].status, 'interrupted');
  assert.equal(entries[2].perm.state, 'expired');
  assert.equal(entries[3].state, 'expired');
  assert.equal(entries[4].declined, true);
  assert.equal(entries[5].answer, '');
  assert.equal(entries[5].declined, undefined);
  assert.equal(entries[6].status, 'completed');
}
for (const type of ['run_completed', 'run_failed', 'run_cancelled']) {
  test(`${type} closes all live entries without mutating input`, () => {
    const prev = pending();
    assertClosed(api.applyEnvelope(prev, env(type)));
    assert.equal(prev[2].status, 'running');
    const next = api.reduceAgentEvent({ ...state(), running: true, entries: prev }, env(type));
    assert.equal(next.running, false);
    assert.equal(next.status, null);
    assert.equal(api.reduceAgentEvent(next, env('message_delta', 2)), next);
  });
}
test('usage_updated writes optional token fields and survives unrelated events; reset on new chat/load', () => {
  let s = { ...state(), running: true };
  s = api.reduceAgentEvent(s, env('usage_updated', 1, 'r1', { total_tokens: 1234, max_tokens: 8000 }));
  assert.equal(s.usage.input, undefined);
  assert.equal(s.usage.output, undefined);
  assert.equal(s.usage.total, 1234);
  assert.equal(s.usage.max, 8000);
  s = api.reduceAgentEvent(s, env('usage_updated', 2, 'r1'));
  assert.equal(s.usage.input, undefined);
  assert.equal(s.usage.total, undefined);
  s = api.reduceAgentEvent(s, env('usage_updated', 3, 'r1', { input_tokens: 100, output_tokens: 20, total_tokens: 120, max_tokens: 4000 }));
  assert.equal(s.usage.input, 100);
  assert.equal(s.usage.output, 20);
  assert.equal(s.usage.total, 120);
  assert.equal(s.usage.max, 4000);
  s = api.reduceAgentEvent(s, env('message_delta', 4, 'r1', { text: 'x' }));
  assert.equal(s.usage.total, 120);
  const { useAgentStore: store } = load();
  store.setState({ sessionId: 's', usage: { total: 42 } });
  store.getState().startNewChat();
  assert.equal(store.getState().usage, null);
  store.setState({ sessionId: 'x', usage: { total: 7 } });
  store.getState().loadSession({ id: 'y', entries: [] });
  assert.equal(store.getState().usage, null);
});
test('compaction is a notice, preserves run and tools; artifacts pass through', () => {
  let s = { ...state(), running: true, entries: pending() };
  s = api.reduceAgentEvent(s, env('compaction_completed'));
  assert.equal(s.entries.at(-1).kind, 'notice');
  assert.equal(s.running, true);
  assert.equal(s.entries[2].status, 'running');
  const artifacts = [{ id: 'a', kind: 'document', location: 'a.md', preview: 'text' }];
  s = api.reduceAgentEvent(s, env('tool_call_completed', 2, 'opaque-uuid', { tool_call_id: 't', artifacts }));
  assert.equal(s.entries[2].artifacts, artifacts);
});
test('window binds first event, rejects duplicate/out-of-order/foreign events and ended runs', () => {
  let s = api.transitionRun(state(), true, 'analyzing');
  for (const sequence of [0, -1, NaN, 1.5, Infinity]) assert.equal(api.reduceAgentEvent(s, env('message_delta', sequence)), s);
  assert.equal(api.reduceAgentEvent(s, env('message_delta', 1, 'other', { session_id: 'foreign' })), s);
  s = api.reduceAgentEvent(s, env('message_delta', 3, 'opaque-uuid', { text: 'a' }));
  for (const sequence of [1, 2, 3]) assert.equal(api.reduceAgentEvent(s, env('message_delta', sequence)), s);
  assert.equal(api.reduceAgentEvent(s, env('run_failed', 4, 'foreign')), s);
  s = api.reduceAgentEvent(s, env('status_changed', 4, 'opaque-uuid', { status: 'executing_tool' }));
  assert.equal(s.status, 'executing_tool');
  s = api.transitionRun(s, false, 'must-clear');
  assert.equal(s.status, null);
  s = api.transitionRun(s, true, 'analyzing');
  assert.equal(api.reduceAgentEvent(s, env('run_completed', 99)), s);
  s = api.reduceAgentEvent(s, env('message_delta', 1, 'new-uuid', { text: 'new' }));
  assert.equal(s.activeRunId, 'new-uuid');
  assert.equal(s.entries.at(-1).text, 'new');
});
test('history load and explicit stop normalize entries; historical questions cannot invoke', async () => {
  let calls = 0;
  const { useAgentStore: store } = load(undefined, async () => { calls++; });
  store.setState({ sessionId: 's', running: true, activeRunId: 'old', entries: pending() });
  store.getState().setRunState(false, 'bad');
  assertClosed(store.getState().entries);
  store.getState().loadSession({ id: 's', entries: pending() });
  assertClosed(store.getState().entries);
  await store.getState().answerQuestion('q', 'yes');
  assert.equal(calls, 0);
  store.getState().setRunState(true, null);
  assert.equal(api.reduceAgentEvent(store.getState(), env('message_delta', 2, 'old')), store.getState());
  store.getState().startNewChat();
  assert.equal(store.getState().running, true);
  store.getState().setRunState(false, null);
  store.getState().startNewChat();
  assert.equal(store.getState().running, false);
});
function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
async function harness(invoke = async () => []) {
  const callbacks = new Map();
  const api = load(async (name, callback) => {
    callbacks.set(name, callback);
    return () => callbacks.delete(name);
  }, invoke);
  await api.ensureAgentListeners();
  api.useAgentStore.setState({ sessionId: 's' });
  return { ...api, store: api.useAgentStore, emit: (name, payload) => callbacks.get(name)({ payload }) };
}

for (const type of ['run_completed', 'run_failed', 'run_cancelled']) {
  test(`${type} waits for host barrier and blocks session operations and late events`, async () => {
    const commands = [];
    const { store, emit } = await harness(async (command) => { commands.push(command); return []; });
    store.getState().setRunState(true, 'analyzing');
    store.setState({ entries: pending() });
    emit('agent-event', env(type));
    const entries = store.getState().entries;
    assertClosed(entries);
    assert.equal(store.getState().status, 'persisting');
    await store.getState().newChat();
    await store.getState().deleteSession('s');
    store.getState().startNewChat();
    store.getState().loadSession({ id: 'other', entries: [] });
    await store.getState().answerQuestion('q', 'yes');
    emit('agent-event', env('message_delta', 9, 'unseen-run', { text: 'late' }));
    emit('agent-question', { session_id: 's', question_id: 'late' });
    emit('agent-run-settled', { session_id: 'foreign', persistence_error: null });
    assert.equal(store.getState().running, true);
    assert.equal(store.getState().sessionId, 's');
    assert.equal(store.getState().entries, entries);
    assert.equal(commands.length, 0);
    emit('agent-run-settled', { session_id: 's', persistence_error: 'disk full' });
    assert.equal(store.getState().running, false);
    assert.match(store.getState().error, /保存失败.*disk full/);
    store.getState().setError('');
    assert.match(store.getState().error, /disk full/);
    store.getState().setRunState(true, 'analyzing');
    emit('agent-event', env('message_delta', 1, 'next-run', { text: 'retry' }));
    assert.equal(store.getState().activeRunId, 'next-run');
    emit('agent-event', env('run_completed', 2, 'next-run'));
    emit('agent-run-settled', { session_id: 's', persistence_error: null });
    assert.equal(store.getState().error, '');
    assert.equal(store.getState().persistenceErrors.s, undefined);
  });
}

test('new chat awaits reset, locks concurrent operations, and retains session on failure', async () => {
  const reset = deferred();
  const calls = [];
  const { store } = await harness(async (command) => {
    calls.push(command);
    if (command === 'agent_reset_session') return reset.promise;
    return [];
  });
  store.setState({ input: 'draft' });
  const task = store.getState().newChat();
  await Promise.resolve();
  assert.equal(store.getState().sessionBusy, true);
  assert.equal(store.getState().sessionId, 's');
  await store.getState().newChat();
  await store.getState().deleteSession('s');
  reset.reject(new Error('unsaved suffix'));
  await task;
  assert.equal(store.getState().sessionId, 's');
  assert.equal(store.getState().input, 'draft');
  assert.equal(store.getState().sessionBusy, false);
  assert.match(store.getState().error, /新对话失败.*unsaved suffix/);
  assert.deepEqual(calls, ['agent_reset_session']);
});

test('new chat saves UI then awaits reset before switching', async () => {
  const reset = deferred();
  const calls = [];
  const { store } = await harness(async (command) => {
    calls.push(command);
    if (command === 'agent_reset_session') return reset.promise;
    return [];
  });
  store.setState({ entries: [{ kind: 'user', text: 'hi' }] });
  const task = store.getState().newChat();
  await new Promise(setImmediate);
  assert.deepEqual(calls, ['save_agent_session', 'list_agent_sessions', 'agent_reset_session']);
  assert.equal(store.getState().sessionId, 's');
  reset.resolve();
  await task;
  assert.notEqual(store.getState().sessionId, 's');
});

for (const fails of [false, true]) {
  test(`delete awaits backend and never resets; failure=${fails}`, async () => {
    const deletion = deferred();
    const calls = [];
    const { store } = await harness(async (command) => {
      calls.push(command);
      if (command === 'delete_agent_session') return deletion.promise;
      return [];
    });
    const task = store.getState().deleteSession('s');
    assert.equal(store.getState().sessionId, 's');
    assert.equal(store.getState().sessionBusy, true);
    if (fails) deletion.reject(new Error('delete failed'));
    else deletion.resolve();
    await task;
    assert.equal(calls.includes('agent_reset_session'), false);
    assert.equal(store.getState().sessionId === 's', fails);
    assert.equal(store.getState().sessionBusy, false);
    if (fails) assert.match(store.getState().error, /删除会话失败/);
  });
}

test('UI save failure prevents reset and session switch', async () => {
  const calls = [];
  const { store } = await harness(async (command) => {
    calls.push(command);
    if (command === 'save_agent_session') throw new Error('JSON write failed');
    return [];
  });
  store.setState({ entries: [{ kind: 'user', text: 'keep me' }] });
  await store.getState().newChat();
  assert.deepEqual(calls, ['save_agent_session']);
  assert.equal(store.getState().sessionId, 's');
  assert.equal(store.getState().entries[0].text, 'keep me');
  assert.match(store.getState().error, /JSON write failed/);
});

test('partial listener registration cannot consume SDK or host events', async () => {
  const last = deferred();
  const callbacks = new Map();
  let removals = 0;
  const { ensureAgentListeners, useAgentStore: store } = load(async (name, callback) => {
    callbacks.set(name, callback);
    if (name === 'agent-run-settled') return last.promise;
    return () => { removals++; };
  });
  store.setState({ sessionId: 's', running: true });
  const task = ensureAgentListeners();
  await Promise.resolve();
  callbacks.get('agent-event')({ payload: env('run_completed') });
  callbacks.get('agent-run-settled')({ payload: { session_id: 's', persistence_error: 'not ready' } });
  assert.equal(store.getState().running, true);
  assert.equal(store.getState().settling, false);
  assert.equal(store.getState().error, '');
  last.reject(new Error('failed'));
  await assert.rejects(task, /监听注册失败/);
  assert.equal(removals, 2);
});

test('status recovers missed settlement, retained errors and successful retry', async () => {
  let result = { running: false, session_id: null, persistence_errors: { s: 'write failed' } };
  const { store, refreshAgentStatus } = await harness(async (command) => command === 'agent_status' ? result : []);
  store.getState().setRunState(true, 'analyzing');
  await refreshAgentStatus();
  assert.equal(store.getState().running, false);
  assert.match(store.getState().error, /write failed/);
  result = { running: false, session_id: null, persistence_errors: {} };
  await refreshAgentStatus();
  assert.equal(store.getState().error, '');
});

test('stale status cannot release a new run or overwrite settlement errors', async () => {
  let response = deferred();
  const { store, emit, refreshAgentStatus } = await harness(async (command) => command === 'agent_status' ? response.promise : []);
  const old = refreshAgentStatus();
  store.getState().setRunState(true, 'analyzing');
  response.resolve({ running: false, session_id: null, persistence_errors: {} });
  await old;
  assert.equal(store.getState().running, true);
  response = deferred();
  const beforeSettlement = refreshAgentStatus();
  emit('agent-run-settled', { session_id: 's', persistence_error: 'new failure' });
  response.resolve({ running: true, session_id: 's', persistence_errors: {} });
  await beforeSettlement;
  assert.equal(store.getState().running, false);
  assert.match(store.getState().error, /new failure/);
});

test('latest status request wins; active host recovered on mount waits for barrier', async () => {
  const first = deferred(), second = deferred();
  let count = 0;
  const { store, emit, refreshAgentStatus } = await harness(async () => (++count === 1 ? first.promise : second.promise));
  const one = refreshAgentStatus(), two = refreshAgentStatus();
  second.resolve({ running: true, session_id: 'recovered', persistence_errors: {} });
  await two;
  first.resolve({ running: false, session_id: null, persistence_errors: {} });
  await one;
  assert.equal(store.getState().sessionId, 'recovered');
  assert.equal(store.getState().running, true);
  assert.equal(store.getState().settling, true);
  emit('agent-run-settled', { session_id: 'recovered', persistence_error: null });
  assert.equal(store.getState().running, false);
});

test('long streams preserve historical entry identity and every token through terminal', () => {
  const history = Array.from({ length: 100 }, () => ({ kind: 'assistant', text: 'history', streaming: false }));
  let s = { ...state(), running: true, entries: history };
  for (let sequence = 1; sequence <= 1000; sequence++) {
    s = api.reduceAgentEvent(s, env('message_delta', sequence, 'stream', { text: 'x' }));
    for (let i = 0; i < history.length; i++) assert.equal(s.entries[i], history[i]);
  }
  s = api.reduceAgentEvent(s, env('run_completed', 1001, 'stream'));
  assert.equal(s.entries[100].text, 'x'.repeat(1000));
  assert.equal(s.entries[100].streaming, false);
});

test('thinking completion keeps unrelated entries stable, including on status events', () => {
  const stable = { kind: 'assistant', text: 'done', streaming: false };
  const prev = [stable, { kind: 'thinking', text: 'thinking' }];
  const next = api.applyEnvelope(prev, env('status_changed'));
  assert.equal(next[0], stable);
  assert.equal(next[1].done, true);
  assert.equal(prev[1].done, undefined);
  assert.equal(api.applyEnvelope(next, env('status_changed', 2)), next);
});

for (const failed of ['agent-event', 'agent-question', 'agent-run-settled']) {
  test(`listeners share promise, rollback ${failed} failure, retry and dedupe terminal persistence`, async () => {
    const callbacks = new Map();
    let fail = true;
    let registrations = 0;
    let removals = 0;
    let saves = 0;
    const { ensureAgentListeners, useAgentStore: store } = load(async (name, callback) => {
      registrations++;
      if (fail && name === failed) throw new Error('registration failed');
      callbacks.set(name, callback);
      return () => { removals++; callbacks.delete(name); };
    }, async (command) => { if (command === 'save_agent_session') saves++; return []; });
    const first = ensureAgentListeners();
    assert.equal(first, ensureAgentListeners());
    await assert.rejects(first, /监听注册失败/);
    assert.equal(registrations, 3);
    assert.equal(removals, 2);
    fail = false;
    await ensureAgentListeners();
    await ensureAgentListeners();
    assert.equal(registrations, 6);
    store.setState({ sessionId: 's' });
    store.getState().setRunState(true, null);
    callbacks.get('agent-question')({ payload: { session_id: 'foreign', question_id: 'bad' } });
    assert.equal(store.getState().entries.length, 0);
    callbacks.get('agent-event')({ payload: env('message_delta', 1, 'uuid', { text: 'hello' }) });
    const terminal = { payload: env('run_completed', 2, 'uuid') };
    callbacks.get('agent-event')(terminal);
    callbacks.get('agent-event')(terminal);
    callbacks.get('agent-question')({ payload: { session_id: 's', question_id: 'late' } });
    assert.equal(saves, 0);
    assert.equal(store.getState().running, true);
    assert.equal(store.getState().settling, true);
    assert.equal(store.getState().status, 'persisting');
    callbacks.get('agent-event')({ payload: env('message_delta', 1, 'different-run', { text: 'late' }) });
    assert.equal(store.getState().entries.length, 1);
    const settled = { payload: { session_id: 's', persistence_error: null } };
    callbacks.get('agent-run-settled')(settled);
    callbacks.get('agent-run-settled')(settled);
    assert.equal(store.getState().running, false);
    assert.equal(store.getState().settling, false);
    assert.equal(saves, 1);
  });
}
