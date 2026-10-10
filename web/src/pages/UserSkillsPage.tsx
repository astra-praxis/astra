/**
 * 我的 Skills 页面（用户技能库，app_data_dir/skills）
 *
 * 与 设置 → Skills 管理（~/.agents/skills 全局 Git 技能）不同：
 * 这里管理应用自有技能库 —— 新建 / 导入文件夹 / 多文件编辑，
 * 供 Agent 页输入框 /技能名 调用与 AI 自动发现（list_skills / read_skill 工具）。
 */
import { useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import {
  ChevronLeft,
  FileText,
  FolderInput,
  ListPlus,
  Plus,
  RefreshCw,
  Save,
  SquarePen,
  Trash2,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import { useWorkspaceStore } from '@/stores/workspace';
import './UserSkillsPage.scss';

interface UserSkillMeta {
  name: string;
  description: string;
  path: string;
  file_count: number;
}

interface UserSkillFile {
  rel_path: string;
  size: number;
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export default function UserSkillsPage() {
  const [skills, setSkills] = useState<UserSkillMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  // 编辑器设置（设置页配置，与工作空间「用编辑器打开」同一来源）
  const editor = useWorkspaceStore((s) => s.editor);

  // 当前编辑中的技能
  const [activeName, setActiveName] = useState<string | null>(null);
  const [files, setFiles] = useState<UserSkillFile[]>([]);
  const [activeFile, setActiveFile] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState('');
  const [fileDirty, setFileDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  // 新建 / 删除弹层
  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [newFileOpen, setNewFileOpen] = useState(false);
  const [newFilePath, setNewFilePath] = useState('');
  const [deleteSkill, setDeleteSkill] = useState<UserSkillMeta | null>(null);

  async function loadSkills(select?: string) {
    setLoading(true);
    setError('');
    try {
      const list = await invoke<UserSkillMeta[]>('list_user_skills');
      setSkills(list);
      if (select) setActiveName(select);
    } catch (e) {
      setError(`加载技能库失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadSkills();
  }, []);

  // ===== 技能文件 =====
  const loadFiles = useCallback(async (name: string) => {
    try {
      const list = await invoke<UserSkillFile[]>('list_user_skill_files', { name });
      setFiles(list);
      return list;
    } catch (e) {
      setError(`加载文件列表失败: ${e}`);
      return [];
    }
  }, []);

  const openFile = useCallback(async (name: string, relPath: string) => {
    try {
      const content = await invoke<string>('read_user_skill_file', {
        name,
        relPath,
      });
      setActiveFile(relPath);
      setFileContent(content);
      setFileDirty(false);
    } catch (e) {
      setError(`读取文件失败: ${e}`);
    }
  }, []);

  async function selectSkill(name: string) {
    if (fileDirty && !window.confirm('当前文件有未保存的修改，切换后将丢失，确定？')) return;
    setActiveName(name);
    setActiveFile(null);
    setFileContent('');
    setFileDirty(false);
    setError('');
    await loadFiles(name);
  }

  async function saveFile() {
    if (!activeName || !activeFile) return;
    setSaving(true);
    setError('');
    try {
      await invoke('write_user_skill_file', {
        name: activeName,
        relPath: activeFile,
        content: fileContent,
      });
      setFileDirty(false);
      setNotice(`已保存 ${activeFile}`);
      setTimeout(() => setNotice(''), 2000);
      await loadFiles(activeName);
    } catch (e) {
      setError(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  }

  // ===== 用编辑器打开（技能文件夹或单文件；编辑器在设置页配置） =====
  async function openInEditor(path: string) {
    if (!editor.command) {
      setError('尚未配置编辑器，请前往 设置 → 编辑器 配置');
      return;
    }
    try {
      await invoke('open_in_editor', { path, editorCommand: editor.command });
    } catch (e) {
      setError(`打开编辑器失败: ${e}`);
    }
  }

  // ===== 新建技能 =====
  async function doCreate() {
    if (!newName.trim()) return;
    try {
      await invoke('create_user_skill', { name: newName.trim(), description: newDesc.trim() });
      setCreateOpen(false);
      setNewName('');
      setNewDesc('');
      await loadSkills(newName.trim());
      await loadFiles(newName.trim());
      setActiveFile('SKILL.md');
      await openFile(newName.trim(), 'SKILL.md');
    } catch (e) {
      setError(`创建失败: ${e}`);
    }
  }

  // ===== 导入技能（文件夹选择器 → 整目录复制） =====
  async function doImport() {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: '选择要导入的技能文件夹（含 SKILL.md）',
      });
      if (!picked) return;
      const meta = await invoke<UserSkillMeta>('import_user_skill', { source: picked });
      setNotice(`已导入技能 ${meta.name}`);
      setTimeout(() => setNotice(''), 2000);
      await loadSkills(meta.name);
      await loadFiles(meta.name);
    } catch (e) {
      setError(`导入失败: ${e}`);
    }
  }

  // ===== 新建 / 删除文件 =====
  async function doCreateFile() {
    if (!activeName || !newFilePath.trim()) return;
    try {
      await invoke('write_user_skill_file', {
        name: activeName,
        relPath: newFilePath.trim(),
        content: '',
      });
      setNewFileOpen(false);
      setNewFilePath('');
      await loadFiles(activeName);
      await openFile(activeName, newFilePath.trim());
    } catch (e) {
      setError(`新建文件失败: ${e}`);
    }
  }

  async function doDeleteFile(relPath: string) {
    if (!activeName) return;
    if (!window.confirm(`删除文件 ${relPath}？`)) return;
    try {
      await invoke('delete_user_skill_file', { name: activeName, relPath });
      if (activeFile === relPath) {
        setActiveFile(null);
        setFileContent('');
        setFileDirty(false);
      }
      await loadFiles(activeName);
    } catch (e) {
      setError(`删除文件失败: ${e}`);
    }
  }

  async function doDeleteSkill() {
    if (!deleteSkill) return;
    try {
      await invoke('delete_user_skill', { name: deleteSkill.name });
      setDeleteSkill(null);
      if (activeName === deleteSkill.name) {
        setActiveName(null);
        setActiveFile(null);
        setFileContent('');
        setFiles([]);
      }
      await loadSkills();
    } catch (e) {
      setError(`删除技能失败: ${e}`);
    }
  }

  const activeMeta = skills.find((s) => s.name === activeName) ?? null;

  return (
    <div className="uskills-page">
      <div className="uskills-top">
        <Link to="/settings" className="uskills-back" title="返回设置">
          <ChevronLeft size={14} strokeWidth={2.5} />
          返回设置
        </Link>
        <h1 className="uskills-title">我的 Skills</h1>
        <p className="uskills-subtitle">
          应用自有技能库，Agent 对话框输入 <kbd>/</kbd> 调用，AI 也会按任务自动选用
        </p>
        <div className="uskills-actions">
          <button type="button" className="uskills-btn" onClick={doImport}>
            <FolderInput size={14} strokeWidth={2} />
            导入文件夹
          </button>
          <button type="button" className="uskills-btn uskills-btn--primary" onClick={() => setCreateOpen(true)}>
            <Plus size={14} strokeWidth={2.5} />
            新建技能
          </button>
          <button
            type="button"
            className="uskills-btn uskills-btn--icon"
            onClick={() => void loadSkills(activeName ?? undefined)}
            title="刷新"
            aria-label="刷新"
          >
            <RefreshCw size={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      {(error || notice) && (
        <div className={`uskills-tip${error ? ' uskills-tip--error' : ''}`}>{error || notice}</div>
      )}

      <div className="uskills-body">
        {/* 左侧：技能列表 */}
        <aside className="uskills-list">
          {loading ? (
            <div className="uskills-empty">加载中…</div>
          ) : skills.length === 0 ? (
            <div className="uskills-empty">
              <p>还没有技能</p>
              <p className="uskills-empty-hint">新建或导入一个含 SKILL.md 的文件夹</p>
            </div>
          ) : (
            skills.map((s) => (
              <div
                key={s.name}
                className={`uskills-card${s.name === activeName ? ' is-active' : ''}`}
                onClick={() => void selectSkill(s.name)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => e.key === 'Enter' && void selectSkill(s.name)}
              >
                <div className="uskills-card-head">
                  <span className="uskills-card-name">/{s.name}</span>
                  <span className="uskills-card-count">{s.file_count} 文件</span>
                </div>
                <p className="uskills-card-desc" title={s.description}>
                  {s.description || '（暂无描述）'}
                </p>
                <div className="uskills-card-foot">
                  <span className="uskills-card-hint">输入 /{s.name} 调用</span>
                  <button
                    type="button"
                    className="uskills-card-del"
                    onClick={(e) => {
                      e.stopPropagation();
                      setDeleteSkill(s);
                    }}
                    title="删除技能"
                    aria-label={`删除技能 ${s.name}`}
                  >
                    <Trash2 size={12} strokeWidth={2} />
                  </button>
                </div>
              </div>
            ))
          )}
        </aside>

        {/* 右侧：技能编辑器 */}
        <section className="uskills-editor">
          {!activeMeta ? (
            <div className="uskills-editor-empty">
              <ListPlus size={28} strokeWidth={1.5} />
              <p>从左侧选择一个技能开始编辑</p>
            </div>
          ) : (
            <>
              <div className="uskills-editor-head">
                <span className="uskills-editor-name">/{activeMeta.name}</span>
                <div className="uskills-editor-head-actions">
                  <button
                    type="button"
                    className="uskills-btn uskills-btn--small"
                    onClick={() => void openInEditor(activeMeta.path)}
                    title={`用 ${editor.name || '编辑器'} 打开技能文件夹`}
                  >
                    <SquarePen size={13} strokeWidth={2} />
                    编辑器打开
                  </button>
                  <button
                    type="button"
                    className="uskills-btn uskills-btn--small"
                    onClick={() => setNewFileOpen(true)}
                  >
                    <FileText size={13} strokeWidth={2} />
                    新建文件
                  </button>
                </div>
              </div>
              <div className="uskills-editor-body">
                <div className="uskills-files">
                  {files.length === 0 ? (
                    <div className="uskills-files-empty">暂无文件</div>
                  ) : (
                    files.map((f) => (
                      <div key={f.rel_path} className="uskills-file-row">
                        <button
                          type="button"
                          className={`uskills-file${f.rel_path === activeFile ? ' is-active' : ''}`}
                          onClick={() => void openFile(activeMeta.name, f.rel_path)}
                          title={f.rel_path}
                        >
                          <FileText size={12} strokeWidth={2} />
                          <span className="uskills-file-name">{f.rel_path}</span>
                          <span className="uskills-file-size">{formatSize(f.size)}</span>
                        </button>
                        <button
                          type="button"
                          className="uskills-file-del"
                          onClick={() => void openInEditor(`${activeMeta.path}/${f.rel_path}`)}
                          title="用编辑器打开此文件"
                          aria-label={`用编辑器打开 ${f.rel_path}`}
                        >
                          <SquarePen size={11} strokeWidth={2} />
                        </button>
                        <button
                          type="button"
                          className="uskills-file-del"
                          onClick={() => void doDeleteFile(f.rel_path)}
                          title="删除文件"
                          aria-label={`删除文件 ${f.rel_path}`}
                        >
                          <Trash2 size={11} strokeWidth={2} />
                        </button>
                      </div>
                    ))
                  )}
                </div>
                <div className="uskills-code">
                  {activeFile ? (
                    <>
                      <textarea
                        className="uskills-textarea"
                        value={fileContent}
                        onChange={(e) => {
                          setFileContent(e.target.value);
                          setFileDirty(true);
                        }}
                        spellCheck={false}
                        placeholder="编辑技能文件内容（Markdown）…"
                      />
                      <div className="uskills-code-foot">
                        <span className="uskills-code-path">{activeFile}</span>
                        <button
                          type="button"
                          className="uskills-btn uskills-btn--primary uskills-btn--small"
                          onClick={() => void saveFile()}
                          disabled={!fileDirty || saving}
                        >
                          <Save size={13} strokeWidth={2} />
                          {saving ? '保存中…' : fileDirty ? '保存' : '已保存'}
                        </button>
                      </div>
                    </>
                  ) : (
                    <div className="uskills-editor-empty uskills-editor-empty--small">
                      <p>选择左侧文件进行编辑</p>
                    </div>
                  )}
                </div>
              </div>
            </>
          )}
        </section>
      </div>

      {/* 新建技能弹窗 */}
      <Modal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        title="新建技能"
        closeOnOverlay={false}
        footer={
          <>
            <button type="button" className="uskills-btn" onClick={() => setCreateOpen(false)}>
              取消
            </button>
            <button
              type="button"
              className="uskills-btn uskills-btn--primary"
              onClick={() => void doCreate()}
              disabled={!newName.trim()}
            >
              创建
            </button>
          </>
        }
      >
        <div className="uskills-form">
          <label className="uskills-label">
            技能名（/ 调用时使用）
            <input
              className="uskills-input"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="例如 bid-writing"
            />
          </label>
          <label className="uskills-label">
            描述（AI 判断是否选用时参考）
            <input
              className="uskills-input"
              value={newDesc}
              onChange={(e) => setNewDesc(e.target.value)}
              placeholder="例如：根据需求文档生成标书框架"
            />
          </label>
        </div>
      </Modal>

      {/* 新建文件弹窗 */}
      <Modal
        open={newFileOpen}
        onClose={() => setNewFileOpen(false)}
        title="新建文件"
        variant="confirm"
        closeOnOverlay={false}
        footer={
          <>
            <button type="button" className="uskills-btn" onClick={() => setNewFileOpen(false)}>
              取消
            </button>
            <button
              type="button"
              className="uskills-btn uskills-btn--primary"
              onClick={() => void doCreateFile()}
              disabled={!newFilePath.trim()}
            >
              创建
            </button>
          </>
        }
      >
        <div className="uskills-form">
          <label className="uskills-label">
            相对路径（如 docs/guide.md）
            <input
              className="uskills-input"
              value={newFilePath}
              onChange={(e) => setNewFilePath(e.target.value)}
              placeholder="guide.md"
            />
          </label>
        </div>
      </Modal>

      {/* 删除技能确认 */}
      <Modal
        open={deleteSkill !== null}
        onClose={() => setDeleteSkill(null)}
        title="删除技能"
        variant="confirm"
        footer={
          <>
            <button type="button" className="uskills-btn" onClick={() => setDeleteSkill(null)}>
              取消
            </button>
            <button
              type="button"
              className="uskills-btn uskills-btn--danger"
              onClick={() => void doDeleteSkill()}
            >
              删除
            </button>
          </>
        }
      >
        <p className="uskills-confirm-text">
          确定删除技能 <strong>/{deleteSkill?.name}</strong>？整个技能目录（含全部文件）将被移除，不可恢复。
        </p>
      </Modal>
    </div>
  );
}
