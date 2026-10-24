import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import { initTheme } from './lib/theme';
import { reportError } from './lib/log';
import './styles/global.scss';
import './styles/ui.scss';

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

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
