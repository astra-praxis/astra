/**
 * Skills 管理页面
 * 移植自 src/routes/skills/+page.svelte
 */
import { useEffect, useState } from 'react';
import { useNavigate, Link } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import {
  ChevronLeft,
  Download,
  RefreshCw,
  Trash2,
} from 'lucide-react';
import Modal from '@/components/ui/Modal';
import './SkillsPage.scss';

interface SkillCard {
  name: string;
  description: string;
  path: string;
  version: string;
  is_git: boolean;
}

type UpdateResultPayload = { Ok: string } | { Err: string };

function versionLabel(v: string): string {
  return v ? `v${v}` : '';
}

function skillIcon(name: string): string {
  const icons: Record<string, string> = {
    'alter-cli': '🔧',
    'browser-control': '🌐',
    'card-converter': '🃏',
    'casp-pack': '📦',
    'find-skills': '🔍',
    'tauri-v2': '🖥️',
    'vercel-react-best-practices': '⚛️',
  };
  return icons[name] || '🧩';
}

export default function SkillsPage() {
  const navigate = useNavigate();
  const [skills, setSkills] = useState<SkillCard[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<SkillCard | null>(null);
  const [updatingSkills, setUpdatingSkills] = useState<Set<string>>(new Set());
  const [updatingAll, setUpdatingAll] = useState(false);
  const [updateResults, setUpdateResults] = useState<Map<string, string>>(new Map());

  // ===== 加载 =====
  async function loadSkills() {
    setLoading(true);
    setError('');
    try {
      setSkills(await invoke<SkillCard[]>('list_skills'));
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadSkills();
  }, []);

  // ===== 删除 =====
  async function doDelete() {
    const target = deleteTarget;
    if (!target) return;
    try {
      await invoke('delete_skill', { path: target.path });
      setSkills((prev) => prev.filter((s) => s.path !== target.path));
      setDeleteTarget(null);
    } catch (e) {
      setError(`删除失败: ${e}`);
      setDeleteTarget(null);
    }
  }

  // ===== 更新 =====
  async function updateSingleSkill(skill: SkillCard) {
    if (!skill.is_git) return;

    const skillPath = skill.path;
    setUpdatingSkills((prev) => new Set([...prev, skillPath]));
    setUpdateResults((prev) => {
      const next = new Map(prev);
      next.delete(skillPath);
      return next;
    });

    try {
      const result = await invoke<string>('update_skill', { path: skillPath });
      setUpdateResults((prev) => new Map(prev).set(skillPath, `✅ 更新成功\n${result}`));
    } catch (e) {
      setUpdateResults((prev) => new Map(prev).set(skillPath, `❌ 更新失败: ${e}`));
    } finally {
      setUpdatingSkills((prev) => {
        const next = new Set(prev);
        next.delete(skillPath);
        return next;
      });
    }
  }

  async function updateAllSkills() {
    setUpdatingAll(true);
    setError('');

    try {
      const results = await invoke<Array<[string, UpdateResultPayload]>>('update_all_skills');

      setUpdateResults((prev) => {
        const newResults = new Map(prev);
        for (const [name, result] of results) {
          const skill = skills.find((s) => s.name === name);
          if (skill) {
            if ('Ok' in result) {
              newResults.set(skill.path, `✅ 更新成功\n${result.Ok}`);
            } else {
              newResults.set(skill.path, `❌ 更新失败: ${result.Err}`);
            }
          }
        }
        return newResults;
      });

      // 刷新列表
      await loadSkills();
    } catch (e) {
      setError(`批量更新失败: ${e}`);
    } finally {
      setUpdatingAll(false);
    }
  }

  return (
    <div className="skills-page">
      <div className="page-header">
        <div className="header-left">
          <Link to="/settings" className="btn-back" title="返回设置">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回设置
          </Link>
          <h1>Skills 管理</h1>
          <p className="subtitle">管理和配置本地的 AI 技能</p>
        </div>
        <div className="header-actions">
          <button
            className="btn-update-all"
            onClick={updateAllSkills}
            disabled={updatingAll || loading}
          >
            {updatingAll ? (
              <>
                <div className="spinner-small" />
                <span>更新中...</span>
              </>
            ) : (
              <>
                <Download size={16} strokeWidth={2.5} />
                <span>一键更新所有</span>
              </>
            )}
          </button>
          <button className="btn-refresh" onClick={loadSkills} disabled={loading}>
            <RefreshCw size={16} strokeWidth={2.5} />
            刷新
          </button>
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div className="error-banner">
          <span>⚠️</span>
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => setError('')}>
            ✕
          </button>
        </div>
      )}

      {/* 加载 */}
      {loading && (
        <div className="loading">
          <div className="spinner" />
          <span>正在加载 Skills...</span>
        </div>
      )}

      {/* 空状态 */}
      {!loading && skills.length === 0 && !error && (
        <div className="empty-state">
          <span className="empty-icon">📭</span>
          <h3>还没有安装任何 Skill</h3>
          <p>
            使用 <code>npx skills add</code> 安装技能后，它们会出现在这里
          </p>
        </div>
      )}

      {/* 技能卡片 */}
      {!loading && skills.length > 0 && (
        <>
          <div className="skills-count">{skills.length} 个技能</div>
          <div className="skills-grid">
            {skills.map((skill) => (
              <div
                key={skill.path}
                className="skill-card"
                onClick={() => navigate(`/skills/${skill.name}`)}
              >
                <div className="skill-header">
                  <span className="skill-icon">{skillIcon(skill.name)}</span>
                  <div className="skill-info">
                    <h3 className="skill-name">{skill.name}</h3>
                    {skill.version && (
                      <span className="skill-version">{versionLabel(skill.version)}</span>
                    )}
                  </div>
                </div>
                {skill.description && <p className="skill-desc">{skill.description}</p>}
                <div className="skill-footer">
                  {skill.is_git ? (
                    <button
                      className="btn-update"
                      onClick={(e) => {
                        e.stopPropagation();
                        updateSingleSkill(skill);
                      }}
                      disabled={updatingSkills.has(skill.path)}
                    >
                      {updatingSkills.has(skill.path) ? (
                        <>
                          <div className="spinner-small" />
                          <span>更新中...</span>
                        </>
                      ) : (
                        <>
                          <Download size={14} strokeWidth={2.5} />
                          <span>更新</span>
                        </>
                      )}
                    </button>
                  ) : (
                    <span className="skill-local-hint">本地生成</span>
                  )}
                  <button
                    className="btn-delete"
                    onClick={(e) => {
                      e.stopPropagation();
                      setDeleteTarget(skill);
                    }}
                  >
                    <Trash2 size={14} strokeWidth={2.5} />
                    删除
                  </button>
                </div>
                {updateResults.has(skill.path) && (
                  <div className="update-result">
                    <pre>{updateResults.get(skill.path)}</pre>
                  </div>
                )}
              </div>
            ))}
          </div>
        </>
      )}

      {/* 删除确认弹窗 */}
      <Modal
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        title="确认删除"
        variant="confirm"
        footer={
          <button className="btn-danger" onClick={doDelete}>
            <Trash2 size={16} strokeWidth={2.5} />
            确认删除
          </button>
        }
      >
        {deleteTarget && (
          <>
            <div className="confirm-icon">⚠️</div>
            <p className="confirm-text">
              确定要删除 Skill <strong>{deleteTarget.name}</strong> 吗？
            </p>
            <p className="confirm-hint">此操作将永久删除该技能的文件夹，不可恢复。</p>
            <div className="confirm-path">{deleteTarget.path}</div>
          </>
        )}
      </Modal>
    </div>
  );
}
