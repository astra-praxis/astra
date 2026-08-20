// 场景化测试：不同窗口宽度 + 不同项目名 + 点击前后布局对比
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const DEBUG_PORT = 9334;
const APP = 'http://localhost:5173';

// 参数：window width, project name, 是否展示更多服务(数量)
const W = parseInt(process.env.W || '1280', 10);
const NAME = process.env.NAME || 'casp-portal-V3.8.3';
const SERVERS = parseInt(process.env.SERVERS || '3', 10);

const MOCK = `
(() => {
  const callbacks = new Map();
  let cbId = 0, evId = 0;
  const listeners = new Map();
  const name = ${JSON.stringify(NAME)};
  const projectPath = '/Users/hanbiao/workplace/' + name;
  const cards = [
    { display_name: '主站点', folder_name: 'master', path: projectPath + '/master', category: 'main', sub_dirs: [
      { label: 'portal', key: 'master', work_dir: projectPath + '/master', has_package_json: true }
    ]},
    { display_name: '模板-通用', folder_name: 'sys-template-common', path: projectPath + '/tpl', category: 'template', sub_dirs: [
      { label: 'template', key: 'web', work_dir: projectPath + '/tpl', has_package_json: true }
    ]}
  ];
  for (let i = 0; i < ${SERVERS}; i++) {
    cards.push({ display_name: '卡片' + i, folder_name: 'cus-card-' + i, path: projectPath + '/cards/c' + i, category: 'card', sub_dirs: [
      { label: 'web', key: 'web', work_dir: projectPath + '/cards/c' + i, has_package_json: true }
    ]});
  }
  const now = Math.floor(Date.now()/1000);
  const servers = [
    { id: 's0', card_name: 'master', subdir: 'master', work_dir: projectPath + '/master', command: 'pnpm run dev', started_at: now - 3600, status: 'running', pid: 10101, port: 5173 }
  ];
  for (let i = 0; i < ${SERVERS}; i++) {
    servers.push({ id: 's' + (i+1), card_name: 'cus-card-' + i, subdir: 'web', work_dir: projectPath + '/cards/c' + i, command: 'pnpm run dev:shell', started_at: now - i * 600, status: 'running', pid: 10200 + i, port: 5174 + i });
  }
  const invoke = async (cmd, args = {}) => {
    switch (cmd) {
      case 'plugin:event|listen': { const cb = callbacks.get(args.handler); if (!listeners.has(args.event)) listeners.set(args.event, []); listeners.get(args.event).push({ cb }); return evId++; }
      case 'plugin:event|unlisten': return null;
      case 'plugin:event|emit': { const arr = listeners.get(args.event); if (arr) for (const l of [...arr]) l.cb({ event: args.event, id: 0, payload: args.payload }); return null; }
      case 'scan_dev_dirs': return cards;
      case 'list_dev_servers': return servers;
      case 'list_dev_history': return [];
      case 'list_providers': return [];
      case 'list_running_servers': return [];
      case 'get_settings': return { favorite_dev_mode: null };
      case 'start_dev_server': return 'mock-new';
      case 'stop_dev_server': return true;
      case 'stop_all_dev_servers': { await new Promise(r => setTimeout(r, 1500)); return servers.length; }
      default: return null;
    }
  };
  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback: (cb) => { const id = ++cbId; callbacks.set(id, cb); return id; },
    callback: (id, ...args) => { const cb = callbacks.get(id); if (cb) cb(...args); },
    metadata: { currentWindow: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__emitDevEvent = (event, payload) => { const arr = listeners.get(event); if (arr) for (const l of [...arr]) l.cb({ event, id: 0, payload }); };
})();
`;

let msgId = 0;
const pending = new Map();
let ws;
function send(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++msgId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
}
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function getWsUrl() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/list`)).json();
      const page = list.find(p => p.type === 'page');
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(200);
  }
  throw new Error('CDP not ready');
}
async function evaluate(expr) {
  const res = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (res.result?.exceptionDetails) throw new Error(JSON.stringify(res.result.exceptionDetails));
  return res.result?.result?.value;
}

const LAYOUT = `(() => {
  const q = s => { const el = document.querySelector(s); if (!el) return null; const b = el.getBoundingClientRect(); return { l: Math.round(b.left), r: Math.round(b.right), w: Math.round(b.width), h: Math.round(b.height) }; };
  const nav = document.querySelector('.dev-nav');
  const page = document.querySelector('.dev-mode-page');
  const sp = document.querySelector('.stop-all-nav .btn-spinner-sm');
  return {
    title: document.querySelector('.dev-nav-title')?.textContent,
    titleLines: Math.round(document.querySelector('.dev-nav-title')?.getBoundingClientRect().height / 18),
    nav: q('.dev-nav'), pageRight: Math.round(page.getBoundingClientRect().right),
    stopAll: q('.stop-all-nav'), refresh: q('.refresh-btn'),
    navSW: nav.scrollWidth, navCW: nav.clientWidth,
    spinnerCS: sp ? { w: getComputedStyle(sp).width, h: getComputedStyle(sp).height, box: getComputedStyle(sp).boxSizing } : null,
    innerWidth: window.innerWidth
  };
})()`;

async function main() {
  const url = `${APP}/dev-mode/${encodeURIComponent('/Users/hanbiao/workplace/' + NAME)}`;
  const chrome = spawn(CHROME, ['--headless=new', '--disable-gpu', '--no-sandbox', `--remote-debugging-port=${DEBUG_PORT}`, '--user-data-dir=/tmp/astra-cdp-scenario', `--window-size=${W},800`, '--force-device-scale-factor=1', 'about:blank'], { stdio: 'ignore' });
  const wsUrl = await getWsUrl();
  ws = new WebSocket(wsUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  ws.onmessage = ev => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m); }
  };
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: MOCK });
  await send('Emulation.setDeviceMetricsOverride', { width: W, height: 800, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url });
  for (let i = 0; i < 80; i++) {
    await sleep(250);
    if (await evaluate(`!!document.querySelector('.stop-all-nav') && !document.querySelector('.dev-loading')`).catch(() => false)) break;
  }
  await sleep(500);
  const before = await evaluate(LAYOUT);
  await evaluate(`document.querySelector('.stop-all-nav').click(); 'ok'`);
  await sleep(120);
  const during = await evaluate(LAYOUT);
  await sleep(2200);
  const after = await evaluate(`({ exists: !!document.querySelector('.stop-all-nav'), colExists: !!document.querySelector('.btn-stop-all') })`);
  console.log(JSON.stringify({ W, NAME, SERVERS, before, during, after }));
  ws.close(); chrome.kill();
}
main().catch(e => { console.error('FAIL:', e); process.exit(1); });
