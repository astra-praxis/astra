// 全面检查：两个停止全部按钮的位置、是否被其他元素覆盖、col-right 是否溢出
import { spawn } from 'node:child_process';

const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const DEBUG_PORT = 9335;
const APP = 'http://localhost:5173';
const W = parseInt(process.env.W || '1280', 10);
const SERVERS = parseInt(process.env.SERVERS || '3', 10);
const NAME = process.env.NAME || 'casp-portal-V3.8.3';

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
      case 'plugin:event|emit': return null;
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

const CHECK = `(() => {
  const q = s => { const el = document.querySelector(s); if (!el) return null; const b = el.getBoundingClientRect(); return { l: Math.round(b.left), r: Math.round(b.right), t: Math.round(b.top), b: Math.round(b.bottom), w: Math.round(b.width), h: Math.round(b.height) }; };
  const colRight = document.querySelector('.col-right');
  const header = document.querySelector('.col-header');
  const btn = document.querySelector('.btn-stop-all');
  const navBtn = document.querySelector('.stop-all-nav');
  const topAt = (x, y) => { const el = document.elementFromPoint(x, y); return el ? (el.className && typeof el.className === 'string' ? el.className : el.tagName) : 'none'; };
  // 检查按钮右半部分在元素最上层是什么
  const checkBtn = (name, btnEl) => {
    if (!btnEl) return name + ': missing';
    const b = btnEl.getBoundingClientRect();
    const midX = Math.round(b.left + b.width * 0.75);
    const midY = Math.round(b.top + b.height / 2);
    const midX2 = Math.round(b.left + b.width * 0.5);
    return {
      btn: q('.btn-stop-all'),
      topAt75pct: topAt(midX, midY),
      topAt50pct: topAt(midX2, midY),
      btnIsTop: btnEl === document.elementFromPoint(midX, midY)
    };
  };
  return {
    viewport: { w: window.innerWidth, h: window.innerHeight },
    colRight: { sw: colRight.scrollWidth, cw: colRight.clientWidth, overflowY: getComputedStyle(colRight).overflowY, scrollH: colRight.scrollHeight, clientH: colRight.clientHeight },
    header: q('.col-header'),
    navBtn: q('.stop-all-nav'),
    btnStopAll: q('.btn-stop-all'),
    navBtnCheck: (() => { if (!navBtn) return null; const b = navBtn.getBoundingClientRect(); const x = Math.round(b.left + b.width * 0.75), y = Math.round(b.top + b.height / 2); return { topAt: topAt(x, y), isTop: navBtn === document.elementFromPoint(x, y) }; })(),
    btnStopAllCheck: (() => { if (!btn) return null; const b = btn.getBoundingClientRect(); const x = Math.round(b.left + b.width * 0.75), y = Math.round(b.top + b.height / 2); return { topAt: topAt(x, y), isTop: btn === document.elementFromPoint(x, y) }; })(),
    fixedEls: [...document.querySelectorAll('*')].filter(el => { const p = getComputedStyle(el).position; return p === 'fixed' || p === 'absolute'; }).slice(0, 10).map(el => { const b = el.getBoundingClientRect(); return { cls: el.className || el.tagName, pos: getComputedStyle(el).position, rect: { l: Math.round(b.left), r: Math.round(b.right), t: Math.round(b.top), b: Math.round(b.bottom) }, z: getComputedStyle(el).zIndex }; })
  };
})()`;

async function main() {
  const url = `${APP}/dev-mode/${encodeURIComponent('/Users/hanbiao/workplace/' + NAME)}`;
  const chrome = spawn(CHROME, ['--headless=new', '--disable-gpu', '--no-sandbox', `--remote-debugging-port=${DEBUG_PORT}`, '--user-data-dir=/tmp/astra-cdp-check', `--window-size=${W},800`, '--force-device-scale-factor=1', 'about:blank'], { stdio: 'ignore' });
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
  const check1 = await evaluate(CHECK);
  // 模拟点击后（spinner 期间）
  await evaluate(`document.querySelector('.stop-all-nav').click(); 'ok'`);
  await sleep(150);
  const check2 = await evaluate(CHECK);
  console.log(JSON.stringify({ W, SERVERS, NAME, check1, check2 }, null, 1));
  ws.close(); chrome.kill();
}
main().catch(e => { console.error('FAIL:', e); process.exit(1); });
