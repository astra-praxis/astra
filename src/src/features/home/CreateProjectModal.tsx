import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import Modal from '@/components/ui/Modal';
import type { ProjectCard } from './types';
import { SquarePen } from 'lucide-react';
import './CreateProjectModal.scss';

interface Props {
  workspacePath: string;
  onClose: () => void;
  onCreated: (project: ProjectCard) => void;
  onError: (message: string) => void;
}

export default function CreateProjectModal({
  workspacePath,
  onClose,
  onCreated,
  onError,
}: Props) {
  const [newFolderName, setNewFolderName] = useState('');
  const [newProjectName, setNewProjectName] = useState('');
  const [creating, setCreating] = useState(false);

  function requestClose() {
    if (creating) return;
    onClose();
  }

  async function createNewProject() {
    const folder = newFolderName.trim();
    if (!folder) return;
    setCreating(true);
    try {
      const newProject = await invoke<ProjectCard>('create_project', {
        workspacePath,
        folderName: folder,
        projectName: newProjectName.trim(),
      });
      onCreated(newProject);
    } catch (e) {
      onError(`创建失败: ${e}`);
    } finally {
      setCreating(false);
    }
  }

  return (
    <Modal
      open
      title="新建项目"
      onClose={requestClose}
      closeDisabled={creating}
      closeOnOverlay={false}
      footer={
        <button
          className="create-project-modal__confirm-btn"
          onClick={createNewProject}
          disabled={!newFolderName.trim() || !newProjectName.trim() || creating}
        >
          {creating ? (
            <>
              <span className="create-project-modal__btn-spinner" />
              创建中...
            </>
          ) : (
            <>
              <SquarePen size={16} strokeWidth={2.5} />
              创建项目
            </>
          )}
        </button>
      }
    >
      <div className="form-group">
        <label htmlFor="folder-name">
          文件夹名称 <span className="required">*</span>
        </label>
        <input
          id="folder-name"
          type="text"
          placeholder="例如：my-project"
          value={newFolderName}
          onChange={(e) => setNewFolderName(e.target.value)}
          disabled={creating}
          onKeyDown={(e) => e.key === 'Enter' && createNewProject()}
        />
        <p className="input-hint">将在当前工作空间下创建此名称的子文件夹</p>
      </div>
      <div className="form-group">
        <label htmlFor="project-name">
          项目名称 <span className="required">*</span>
        </label>
        <input
          id="project-name"
          type="text"
          placeholder="输入项目显示名称"
          value={newProjectName}
          onChange={(e) => setNewProjectName(e.target.value)}
          disabled={creating}
          onKeyDown={(e) => e.key === 'Enter' && createNewProject()}
        />
        <p className="input-hint">项目 README.md 的标题</p>
      </div>
    </Modal>
  );
}
