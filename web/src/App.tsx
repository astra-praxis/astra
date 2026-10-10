import { HashRouter, Routes, Route, Navigate } from 'react-router';
import AppLayout from './components/layout/AppLayout';
import WorkspacePage from './pages/WorkspacePage';
import ChatPage from './pages/chat/ChatPage';
import ToolsPage from './pages/ToolsPage';
import DownloaderPage from './pages/DownloaderPage';
import NmPage from './pages/NmPage';
import LogsPage from './pages/LogsPage';
import ModelsPage from './pages/ModelsPage';
import ModelDetailPage from './pages/ModelDetailPage';
import ProvidersPage from './pages/ProvidersPage';
import ProviderDetailPage from './pages/ProviderDetailPage';
import SkillsPage from './pages/SkillsPage';
import SkillDetailPage from './pages/SkillDetailPage';
import SettingsPage from './pages/SettingsPage';
import DevModePage from './pages/DevModePage';

export default function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<WorkspacePage />} />
          <Route path="/chat" element={<ChatPage />} />
          <Route path="/tools" element={<ToolsPage />} />
          <Route path="/tools/downloader" element={<DownloaderPage />} />
          <Route path="/tools/nm" element={<NmPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/models" element={<ModelsPage />} />
          <Route path="/models/:id" element={<ModelDetailPage />} />
          <Route path="/providers" element={<ProvidersPage />} />
          <Route path="/providers/:id" element={<ProviderDetailPage />} />
          <Route path="/skills" element={<SkillsPage />} />
          <Route path="/skills/:name" element={<SkillDetailPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/dev-mode/:encoded" element={<DevModePage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}
