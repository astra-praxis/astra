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
