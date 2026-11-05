import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { initTheme } from './lib/theme';
import { reportError } from './lib/log';
import './styles/global.scss';
import './styles/ui.scss';

// 宿主内嵌控制台窗口（桌面端拉起时带 ?console=1）：跳过桌面端应用
// （AccountGate/Agent 会话全依赖 Tauri IPC，外链窗口里拿不到），
// 直接渲染纯 HTTP 的 Web 控制台（token + Entity 列表）。
const isEmbeddedConsole =
  typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('console');

// 主题初始化必须在渲染之前调用，否则刷新时闪白
initTheme();

// 全局错误上报：Windows GUI 无控制台，前端未捕获异常默认无人可见。
// 落盘到 Rust 端 astra.log（feature=frontend），供用户反馈定位。
window.addEventListener('error', (event) => {
  const detail = event.error?.stack || event.message || String(event.filename || 'unknown');
  reportError('frontend', `uncaught error: ${detail}`);
});
window.addEventListener('unhandledrejection', (event) => {
  const reason = event.reason;
  const detail = reason?.stack || reason?.message || String(reason);
  reportError('frontend', `unhandled rejection: ${detail}`);
});

if (isEmbeddedConsole) {
  void Promise.all([import('./pages/serve/ServeConsolePage'), import('react-dom/client')]).then(
    ([{ default: ServeConsolePage }, { createRoot }]) => {
      createRoot(document.getElementById('root')!).render(
        <StrictMode>
          <ServeConsolePage webMode />
        </StrictMode>,
      );
    },
  );
} else {
  void Promise.all([import('./App'), import('react')]).then(([{ default: App }, { StrictMode }]) => {
    createRoot(document.getElementById('root')!).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  });
}
