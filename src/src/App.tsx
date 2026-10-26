import { HashRouter, Routes, Route, Navigate } from 'react-router';
import AccountGate from './components/account/AccountGate';
import AppLayout from './components/layout/AppLayout';
import WorkspacePage from './pages/workspace/WorkspacePage';
import AgentPage from './pages/agent/AgentPage';
import ToolsPage from './pages/tools/ToolsPage';
import DownloaderPage from './pages/tools/DownloaderPage';
import NmPage from './pages/tools/NmPage';
import LogsPage from './pages/logs/LogsPage';
import ArtifactsPage from './pages/artifacts/ArtifactsPage';
import ModelsPage from './pages/models/ModelsPage';
import ModelDetailPage from './pages/models/ModelDetailPage';
import ProvidersPage from './pages/providers/ProvidersPage';
import ProviderDetailPage from './pages/providers/ProviderDetailPage';
import SkillDetailPage from './pages/skills/SkillDetailPage';
import MarketMcpPage from './pages/market/MarketMcpPage';
import MarketSkillPage from './pages/market/MarketSkillPage';
import SettingsPage from './pages/settings/SettingsPage';
import DevModePage from './pages/dev-mode/DevModePage';

export default function App() {
  return (
    <AccountGate>
      <HashRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<Navigate to="/agent" replace />} />
          <Route path="/workspace" element={<WorkspacePage />} />
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
          <Route path="/skills/:name" element={<SkillDetailPage />} />
          <Route path="/market/mcp" element={<MarketMcpPage />} />
          <Route path="/market/skills" element={<MarketSkillPage />} />
          <Route path="/settings" element={<Navigate to="/settings/general" replace />} />
          <Route path="/settings/:section" element={<SettingsPage />} />
          <Route path="/dev-mode/:encoded" element={<DevModePage />} />
          <Route path="*" element={<Navigate to="/agent" replace />} />
        </Route>
      </Routes>
      </HashRouter>
    </AccountGate>
  );
}
