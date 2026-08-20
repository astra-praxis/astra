// CDP 驱动：加载真实 SvelteKit 应用 + Tauri 前端 mock，复现 dev-mode 停止全部按钮遮挡问题
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const DEBUG_PORT = 9333;
const APP_URL = process.env.APP_URL || 'http://localhost:5173';

// ===== 注入的 Tauri mock（在页面脚本之前执行）=====
const MOCK_SOURCE = `
(() => {
  const callbacks = new Map();
  let cbId = 0;
  let evId = 0;
  const listeners = new Map();

  // 模拟数据
  const cards = [
    { display_name: '主站点', folder_name: 'master', path: '/Users/hanbiao/workplace/casp-portal-V3.8.3/master', category: 'main', sub_dirs: [
      { label: 'portal', key: 'master', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/master', has_package_json: true }
    ]},
    { display_name: '模板-通用', folder_name: 'sys-template-common', path: '/Users/hanbiao/workplace/casp-portal-V3.8.3/templates/sys-template-common', category: 'template', sub_dirs: [
      { label: 'template', key: 'web', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/templates/sys-template-common', has_package_json: true }
    ]},
    { display_name: '我的任务列表', folder_name: 'cus-card-myTaskList', path: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-myTaskList', category: 'card', sub_dirs: [
      { label: 'web', key: 'web', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-myTaskList', has_package_json: true }
    ]},
    { display_name: '南京审计-移动', folder_name: 'cus-card-naumobile-app', path: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-naumobile-app', category: 'card', sub_dirs: [
      { label: 'mobile', key: 'mobile', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-naumobile-app', has_package_json: true }
    ]}
  ];
  const now = Math.floor(Date.now() / 1000);
  const servers = [
    { id: 's1', card_name: 'master', subdir: 'master', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/master', command: 'pnpm run dev', started_at: now - 3600, status: 'running', pid: 10101, port: 5173 },
    { id: 's2', card_name: 'sys-template-common', subdir: 'web', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/templates/sys-template-common', command: 'pnpm run dev:shell', started_at: now - 1800, status: 'running', pid: 10102, port: 5174 },
    { id: 's3', card_name: 'cus-card-myTaskList', subdir: 'web', work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-myTaskList', command: 'pnpm run dev:shell', started_at: now - 900, status: 'running', pid: 10103, port: 5175 }
  ];
  const history = [
    { work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/master', card_name: 'master', subdir_key: 'master', command: 'pnpm run dev', project_path: '/Users/hanbiao/workplace/casp-portal-V3.8.3', last_started_at: now - 3600 },
    { work_dir: '/Users/hanbiao/workplace/casp-portal-V3.8.3/cards/cus-card-myTaskList', card_name: 'cus-card-myTaskList', subdir_key: 'web', command: 'pnpm run dev:shell', project_path: '/Users/hanbiao/workplace/casp-portal-V3.8.3', last_started_at: now - 900 }
  ];

  const invoke = async (cmd, args = {}) => {
    switch (cmd) {
      case 'plugin:event|listen': {
        const cb = callbacks.get(args.handler);
        if (!listeners.has(args.event)) listeners.set(args.event, []);
        listeners.get(args.event).push({ cb });
        return evId++;
      }
      case 'plugin:event|unlisten': return null;
      case 'plugin:event|emit': {
        const arr = listeners.get(args.event);
        if (arr) for (const l of [...arr]) { try { l.cb({ event: args.event, id: 0, payload: args.payload }); } catch (e) { console.error('emit handler error', e); } }
        return null;
      }
      case 'scan_dev_dirs': return cards;
      case 'list_dev_servers': return servers;
      case 'list_dev_history': return history;
      case 'list_providers': return [];
      case 'list_running_servers': return [];
      case 'get_settings': return { favorite_dev_mode: null };
      case 'start_dev_server': return 'mock-new-' + (Math.random() * 1e6 | 0);
      case 'stop_dev_server': return true;
      case 'stop_all_dev_servers': {
        // 模拟逐个停止耗时（真实后端每个服务约 500ms）
        await new Promise(r => setTimeout(r, 1500));
        return servers.length;
      }
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

  // 供驱动脚本使用
  window.__emitDevEvent = (event, payload) => {
    const arr = listeners.get(event);
    if (arr) for (const l of [...arr]) { try { l.cb({ event, id: 0, payload }); } catch (e) {} }
  };
})();
`;

// ===== 简单的 CDP 客户端 =====
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

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

async function getWsUrl() {
  for (let i = 0; i < 50; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/list`);
      const list = await res.json();
      const page = list.find(p => p.type === 'page');
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(200);
  }
  throw new Error('CDP endpoint not ready');
}

async function evaluate(expr) {
  const res = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (res.result?.exceptionDetails) throw new Error('eval error: ' + JSON.stringify(res.result.exceptionDetails));
  return res.result?.result?.value;
}

async function screenshot(path) {
  const res = await send('Page.captureScreenshot', { format: 'png' });
  if (res.result?.data) writeFileSync(path, Buffer.from(res.result.data, 'base64'));
}

async function main() {
  const url = process.argv[2];
  const chrome = spawn(CHROME, [
    '--headless=new', '--disable-gpu', '--no-sandbox',
    `--remote-debugging-port=${DEBUG_PORT}`,
    '--user-data-dir=/tmp/astra-cdp-profile',
    '--window-size=1280,800',
    '--force-device-scale-factor=1',
    'about:blank'
  ], { stdio: 'ignore' });

  const wsUrl = await getWsUrl();
  ws = new WebSocket(wsUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg);
    }
  };

  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: MOCK_SOURCE });
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });

  await send('Page.navigate', { url });
  // 等待页面渲染完成（loading 结束、nav 出现）
  for (let i = 0; i < 80; i++) {
    await sleep(250);
    const ready = await evaluate(`!!document.querySelector('.dev-nav') && !!document.querySelector('.stop-all-nav') && !document.querySelector('.dev-loading')`).catch(() => false);
    if (ready) break;
  }
  await sleep(800);

  // ---- 点击前状态 ----
  const before = await evaluate(`(() => {
    const q = s => { const el = document.querySelector(s); if (!el) return null; const b = el.getBoundingClientRect(); return { left: Math.round(b.left), top: Math.round(b.top), right: Math.round(b.right), bottom: Math.round(b.bottom), w: Math.round(b.width), h: Math.round(b.height) }; };
    const nav = document.querySelector('.dev-nav');
    const page = document.querySelector('.dev-mode-page');
    return {
      title: document.querySelector('.dev-nav-title')?.textContent,
      titleLines: Math.round(document.querySelector('.dev-nav-title')?.getBoundingClientRect().height / 18),
      nav: q('.dev-nav'),
      pageRight: Math.round(page.getBoundingClientRect().right),
      stopAll: q('.stop-all-nav'),
      refresh: q('.refresh-btn'),
      fav: q('.fav-btn'),
      navSW: nav.scrollWidth, navCW: nav.clientWidth,
      htmlOverflow: document.documentElement.scrollWidth,
      bodyOverflow: document.body.scrollWidth,
      innerWidth: window.innerWidth
    };
  })()`);
  console.log('BEFORE:', JSON.stringify(before, null, 1));

  await screenshot('.scratch/step1-before.png');

  // ---- 点击停止全部 ----
  await evaluate(`document.querySelector('.stop-all-nav').click(); 'clicked'`);

  // 点击后立即（spinner 状态）检查
  await sleep(100);
  const during = await evaluate(`(() => {
    const q = s => { const el = document.querySelector(s); if (!el) return null; const b = el.getBoundingClientRect(); return { left: Math.round(b.left), top: Math.round(b.top), right: Math.round(b.right), bottom: Math.round(b.bottom), w: Math.round(b.width), h: Math.round(b.height) }; };
    const nav = document.querySelector('.dev-nav');
    const page = document.querySelector('.dev-mode-page');
    const sp = document.querySelector('.stop-all-nav .btn-spinner-sm');
    return {
      nav: q('.dev-nav'),
      pageRight: Math.round(page.getBoundingClientRect().right),
      stopAll: q('.stop-all-nav'),
      refresh: q('.refresh-btn'),
      spinner: q('.stop-all-nav .btn-spinner-sm'),
      spinnerBox: sp ? getComputedStyle(sp).boxSizing : null,
      spinnerBorderW: sp ? parseFloat(getComputedStyle(sp).borderLeftWidth) : 0,
      disabled: document.querySelector('.stop-all-nav')?.disabled,
      navSW: nav.scrollWidth, navCW: nav.clientWidth,
      innerWidth: window.innerWidth
    };
  })()`);
  console.log('DURING(click+100ms):', JSON.stringify(during, null, 1));
  await screenshot('.scratch/step2-during.png');

  // 停止完成后状态
  await sleep(2500);
  const after = await evaluate(`(() => {
    const q = s => { const el = document.querySelector(s); if (!el) return null; const b = el.getBoundingClientRect(); return { left: Math.round(b.left), right: Math.round(b.right), w: Math.round(b.width) }; };
    return {
      stopAll: q('.stop-all-nav'),
      btnStopAllCol: q('.btn-stop-all'),
      stopAllNavExists: !!document.querySelector('.stop-all-nav'),
      runningCount: document.querySelectorAll('.run-card').length
    };
  })()`);
  console.log('AFTER(done):', JSON.stringify(after, null, 1));
  await screenshot('.scratch/step3-after.png');

  ws.close();
  chrome.kill();
}

main().catch(e => { console.error('FAIL:', e); process.exit(1); });
