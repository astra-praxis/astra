import { HashRouter, Routes, Route, Navigate } from 'react-router';
import AppLayout from './components/layout/AppLayout';
import WorkspacePage from './pages/WorkspacePage';
import AgentPage from './pages/AgentPage';
import ToolsPage from './pages/ToolsPage';
import DownloaderPage from './pages/DownloaderPage';
import NmPage from './pages/NmPage';
import LogsPage from './pages/LogsPage';
import ArtifactsPage from './pages/ArtifactsPage';
import ModelsPage from './pages/ModelsPage';
import ModelDetailPage from './pages/ModelDetailPage';
import ProvidersPage from './pages/ProvidersPage';
import ProviderDetailPage from './pages/ProviderDetailPage';
import SkillsPage from './pages/SkillsPage';
import SkillDetailPage from './pages/SkillDetailPage';
import UserSkillsPage from './pages/UserSkillsPage';
import MarketMcpPage from './pages/MarketMcpPage';
import MarketSkillPage from './pages/MarketSkillPage';
import ExtensionsPage from './pages/ExtensionsPage';
import SettingsPage from './pages/SettingsPage';
import DevModePage from './pages/DevModePage';

export default function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<WorkspacePage />} />
          <Route path="/agent" element={<AgentPage />} />
          <Route path="/tools" element={<ToolsPage />} />
          <Route path="/tools/downloader" element={<DownloaderPage />} />
          <Route path="/tools/nm" element={<NmPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/artifacts" element={<ArtifactsPage />} />
          <Route path="/models" element={<ModelsPage />} />
          <Route path="/models/:id" element={<ModelDetailPage />} />
          <Route path="/providers" element={<ProvidersPage />} />
          <Route path="/providers/:id" element={<ProviderDetailPage />} />
          <Route path="/skills" element={<SkillsPage />} />
          <Route path="/skills/:name" element={<SkillDetailPage />} />
          <Route path="/my-skills" element={<UserSkillsPage />} />
          <Route path="/market/mcp" element={<MarketMcpPage />} />
          <Route path="/market/skills" element={<MarketSkillPage />} />
          <Route path="/extensions" element={<ExtensionsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/dev-mode/:encoded" element={<DevModePage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}
