/**
 * 我的 Skills 页面（用户技能库，~/.config/astra/skills）
 *
 * 与 设置 → Skills 管理（~/.agents/skills 全局 Git 技能）不同：
 * 这里管理应用自有技能库 —— 新建 / 导入文件夹 / 多文件编辑，
 * 供 Agent 页输入框 /技能名 调用与 AI 自动发现（list_skills / read_skill 工具）。
 *
 * 布局：卡片网格（可搜索）→ 点卡片进详情页（文件列表 + Markdown 编辑/预览）。
 */
import { useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import {
  ArrowLeft,
  ChevronLeft,
  Download,
  Eye,
  FileText,
  FolderInput,
  ListPlus,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  SquarePen,
  Trash2,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import { useWorkspaceStore } from '@/stores/workspace';
import './UserSkillsPage.scss';

interface SyncReport {
  imported: string[];
  updated: string[];
  skipped: [string, string][];
}

interface UserSkillMeta {
  name: string;
  display_name: string;
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

function isMarkdown(relPath: string | null): boolean {
  if (!relPath) return false;
  const lower = relPath.toLowerCase();
  return lower.endsWith('.md') || lower.endsWith('.markdown');
}

// 剥离 YAML frontmatter（预览正文用）
function stripFrontmatter(content: string): string {
  const trimmed = content.trimStart();
  if (trimmed.startsWith('---')) {
    const end = trimmed.indexOf('---', 3);
    if (end !== -1) return trimmed.slice(end + 3);
  }
  return content;
}

export default function UserSkillsPage() {
  const [skills, setSkills] = useState<UserSkillMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [query, setQuery] = useState('');

  // 编辑器设置（设置页配置，与工作空间「用编辑器打开」同一来源）
  const editor = useWorkspaceStore((s) => s.editor);

  // 当前详情页中的技能（null = 卡片网格）
  const [activeName, setActiveName] = useState<string | null>(null);
  const [files, setFiles] = useState<UserSkillFile[]>([]);
  const [activeFile, setActiveFile] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState('');
  const [fileDirty, setFileDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [fileView, setFileView] = useState<'edit' | 'preview'>('edit');

  // 新建 / 删除弹层
  const [createOpen, setCreateOpen] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [newFileOpen, setNewFileOpen] = useState(false);
  const [newFilePath, setNewFilePath] = useState('');
  const [deleteSkill, setDeleteSkill] = useState<UserSkillMeta | null>(null);
  const [deleteFile, setDeleteFile] = useState<string | null>(null);

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
      setFileView('edit');
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
    setFileView('edit');
    setError('');
    await loadFiles(name);
  }

  function closeDetail() {
    if (fileDirty && !window.confirm('当前文件有未保存的修改，返回后将丢失，确定？')) return;
    setActiveName(null);
    setActiveFile(null);
    setFileContent('');
    setFileDirty(false);
    setFileView('edit');
  }

  async function saveFile() {
    if (!activeName || !activeFile) return;
    setSaving(true);
    try {
      await invoke('write_user_skill_file', {
        name: activeName,
        relPath: activeFile,
        content: fileContent,
      });
      setFileDirty(false);
      await loadSkills(activeName);
    } catch (e) {
      setError(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  }

  async function openInEditor(path: string) {
    try {
      const editorCommand = editor.command || editor.name;
      await invoke('open_in_editor', { editorCommand, path });
    } catch (e) {
      setError(`打开编辑器失败: ${e}`);
    }
  }

  async function doCreate() {
    try {
      await invoke('create_user_skill', { name: newName, description: newDesc });
      setCreateOpen(false);
      setNewName('');
      setNewDesc('');
      await loadSkills(newName.trim());
    } catch (e) {
      setError(`创建技能失败: ${e}`);
    }
  }

  async function doImport() {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: true,
        title: '选择要导入的技能文件夹（含 SKILL.md）',
      });
      if (!picked) return;
      const sources = Array.isArray(picked) ? picked : [picked];
      let imported = 0;
      const failed: string[] = [];
      for (const source of sources) {
        try {
          await invoke('import_user_skill', { source });
          imported += 1;
        } catch {
          failed.push(source);
        }
      }
      if (failed.length > 0) setError(`有 ${failed.length} 个文件夹导入失败：${failed.join('、')}`);
      else if (imported > 0) setNotice(`已导入 ${imported} 个技能`);
      await loadSkills();
    } catch (e) {
      setError(`导入失败: ${e}`);
    }
  }

  async function doSyncGlobal() {
    setError('');
    setNotice('');
    setSyncing(true);
    try {
      const report = await invoke<SyncReport>('sync_global_skills');
      const parts = [`新增 ${report.imported.length}`, `更新 ${report.updated.length}`];
      if (report.skipped.length > 0) parts.push(`跳过 ${report.skipped.length}`);
      setNotice(`同步完成：${parts.join(' · ')}`);
      await loadSkills(activeName ?? undefined);
    } catch (e) {
      setError(`同步全局技能失败: ${e}`);
    } finally {
      setSyncing(false);
    }
  }

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
      await loadSkills(activeName);
      await openFile(activeName, newFilePath.trim());
    } catch (e) {
      setError(`新建文件失败: ${e}`);
    }
  }

  async function doDeleteFile() {
    if (!activeName || !deleteFile) return;
    try {
      await invoke('delete_user_skill_file', { name: activeName, relPath: deleteFile });
      if (activeFile === deleteFile) {
        setActiveFile(null);
        setFileContent('');
        setFileDirty(false);
      }
      setDeleteFile(null);
      await loadFiles(activeName);
      await loadSkills(activeName);
    } catch (e) {
      setError(`删除文件失败: ${e}`);
    }
  }

  async function doDeleteSkill() {
    if (!deleteSkill) return;
    try {
      await invoke('delete_user_skill', { name: deleteSkill.name });
      setDeleteSkill(null);
      if (activeName === deleteSkill.name) closeDetail();
      await loadSkills();
    } catch (e) {
      setError(`删除技能失败: ${e}`);
    }
  }

  const activeMeta = skills.find((s) => s.name === activeName) ?? null;
  const filtered = skills.filter((s) => {
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return (
      s.name.toLowerCase().includes(q) ||
      s.display_name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q)
    );
  });

  // ===== 详情页 =====
  if (activeMeta) {
    return (
      <div className="uskills-page">
        <div className="uskills-top">
          <button type="button" className="uskills-back" onClick={closeDetail} title="返回技能列表">
            <ArrowLeft size={14} strokeWidth={2.5} />
            返回列表
          </button>
          <h1 className="uskills-title">{activeMeta.display_name}</h1>
          <span className="uskills-chip">/{activeMeta.name}</span>
          <div className="uskills-actions">
            <button
              type="button"
              className="uskills-btn"
              onClick={() => void openInEditor(activeMeta.path)}
              title={`用 ${editor.name || '编辑器'} 打开技能文件夹`}
            >
              <SquarePen size={14} strokeWidth={2} />
              编辑器打开
            </button>
            <button type="button" className="uskills-btn" onClick={() => setNewFileOpen(true)}>
              <FileText size={14} strokeWidth={2} />
              新建文件
            </button>
            <button type="button" className="uskills-btn uskills-btn--danger" onClick={() => setDeleteSkill(activeMeta)}>
              <Trash2 size={14} strokeWidth={2} />
              删除技能
            </button>
          </div>
        </div>
        <p className="uskills-subtitle">
          Agent 对话框输入 <kbd>/{activeMeta.name}</kbd> 调用 · {activeMeta.file_count} 个文件
        </p>

        {(error || notice) && (
          <div className={`uskills-tip${error ? ' uskills-tip--error' : ''}`}>{error || notice}</div>
        )}

        <div className="uskills-detail">
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
                    onClick={() => setDeleteFile(f.rel_path)}
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
                {isMarkdown(activeFile) && (
                  <div className="uskills-tabs">
                    <button
                      type="button"
                      className={`uskills-tab${fileView === 'edit' ? ' is-active' : ''}`}
                      onClick={() => setFileView('edit')}
                    >
                      <Pencil size={12} strokeWidth={2} />
                      编辑
                    </button>
                    <button
                      type="button"
                      className={`uskills-tab${fileView === 'preview' ? ' is-active' : ''}`}
                      onClick={() => setFileView('preview')}
                    >
                      <Eye size={12} strokeWidth={2} />
                      预览
                    </button>
                  </div>
                )}
                {fileView === 'preview' && isMarkdown(activeFile) ? (
                  <div className="uskills-preview">
                    <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]}>
                      {stripFrontmatter(fileContent)}
                    </ReactMarkdown>
                  </div>
                ) : (
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
                )}
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
                <ListPlus size={24} strokeWidth={1.5} />
                <p>选择左侧文件进行编辑或预览</p>
              </div>
            )}
          </div>
        </div>

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

        {/* 删除文件确认 */}
        <Modal
          open={deleteFile !== null}
          onClose={() => setDeleteFile(null)}
          title="删除文件"
          variant="confirm"
          footer={
            <>
              <button type="button" className="uskills-btn" onClick={() => setDeleteFile(null)}>
                取消
              </button>
              <button
                type="button"
                className="uskills-btn uskills-btn--danger"
                onClick={() => void doDeleteFile()}
              >
                删除
              </button>
            </>
          }
        >
          <p className="uskills-confirm-text">
            确定删除文件 <strong>{deleteFile}</strong>？删除后不可恢复。
          </p>
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

  // ===== 卡片网格 =====
  return (
    <div className="uskills-page">
      <div className="uskills-top">
        <Link to="/settings" className="uskills-back" title="返回设置">
          <ChevronLeft size={14} strokeWidth={2.5} />
          返回设置
        </Link>
        <h1 className="uskills-title">我的 Skills</h1>
        <div className="uskills-search">
          <Search size={13} strokeWidth={2} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索技能名称或描述…"
          />
        </div>
        <div className="uskills-actions">
          <button type="button" className="uskills-btn" onClick={doImport}>
            <FolderInput size={14} strokeWidth={2} />
            导入文件夹
          </button>
          <button
            type="button"
            className="uskills-btn"
            onClick={() => void doSyncGlobal()}
            disabled={syncing}
            title="从 ~/.agents/skills 同步到应用技能库（同名覆盖）"
          >
            <Download size={14} strokeWidth={2} />
            {syncing ? '同步中…' : '同步全局 Skills'}
          </button>
          <button type="button" className="uskills-btn uskills-btn--primary" onClick={() => setCreateOpen(true)}>
            <Plus size={14} strokeWidth={2.5} />
            新建技能
          </button>
          <button
            type="button"
            className="uskills-btn uskills-btn--icon"
            onClick={() => void loadSkills()}
            title="刷新"
            aria-label="刷新"
          >
            <RefreshCw size={14} strokeWidth={2} />
          </button>
        </div>
      </div>
      <p className="uskills-subtitle">
        应用自有技能库，Agent 对话框输入 <kbd>/</kbd> 调用，AI 也会按任务自动选用
      </p>

      {(error || notice) && (
        <div className={`uskills-tip${error ? ' uskills-tip--error' : ''}`}>{error || notice}</div>
      )}

      <div className="uskills-grid">
        {loading ? (
          <div className="uskills-empty uskills-empty--grid">加载中…</div>
        ) : filtered.length === 0 ? (
          <div className="uskills-empty uskills-empty--grid">
            {query.trim() ? (
              <p>没有匹配「{query.trim()}」的技能</p>
            ) : (
              <>
                <p>还没有技能</p>
                <p className="uskills-empty-hint">新建、导入或从全局同步一个含 SKILL.md 的文件夹</p>
              </>
            )}
          </div>
        ) : (
          filtered.map((s) => (
            <div
              key={s.name}
              className="uskills-card"
              onClick={() => void selectSkill(s.name)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === 'Enter' && void selectSkill(s.name)}
            >
              <div className="uskills-card-head">
                <span className="uskills-card-title">{s.display_name}</span>
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
              <p className="uskills-card-desc" title={s.description}>
                {s.description || '（暂无描述）'}
              </p>
              <div className="uskills-card-foot">
                <span className="uskills-card-hint">/{s.name}</span>
                <span className="uskills-card-count">{s.file_count} 文件</span>
              </div>
            </div>
          ))
        )}
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

      {/* 删除技能确认（网格视图） */}
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
