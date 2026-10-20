/**
 * 工作空间：添加 / 切换 / 移除，操作后立即执行（不经过保存栏）
 */
import { Folder, X } from 'lucide-react';
import SettingsSection from '../components/SettingsSection';
import { useWorkspaceSettings } from '../hooks/useWorkspaceSettings';

export default function WorkspaceSettings() {
  const { workspaces, activeWorkspace, addWorkspace, removeWorkspace, setActiveWorkspace } =
    useWorkspaceSettings();

  return (
    <SettingsSection iconId="workspace" title="工作空间" desc="添加多个工作空间目录，在主页可一键切换；操作立即生效">
      {workspaces.length > 0 ? (
        <div className="ws-list">
          {workspaces.map((ws) => (
            <div
              key={ws.path}
              className={`ws-item${ws.path === activeWorkspace ? ' active' : ''}`}
            >
              <div className="ws-item-left">
                <Folder size={16} strokeWidth={2} />
                <div className="ws-item-info">
                  <span className="ws-item-name">{ws.name}</span>
                  <span className="ws-item-path">{ws.path}</span>
                </div>
              </div>
              <div className="ws-item-actions">
                {ws.path !== activeWorkspace ? (
                  <button
                    className="ws-btn-activate"
                    onClick={() => void setActiveWorkspace(ws.path)}
                    title="切换为此工作空间"
                  >
                    切换
                  </button>
                ) : (
                  <span className="ws-badge-active">当前</span>
                )}
                <button
                  className="ws-btn-remove"
                  onClick={() => void removeWorkspace(ws.path)}
                  title="移除"
                >
                  <X size={14} strokeWidth={2.5} />
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="ws-empty">
          <span>还没有添加任何工作空间</span>
        </div>
      )}

      <button className="btn-select-folder" onClick={() => void addWorkspace()}>
        <Folder size={18} strokeWidth={2} />
        添加工作空间
      </button>
    </SettingsSection>
  );
}
