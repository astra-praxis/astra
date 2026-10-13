/**
 * Skill 详情页（Markdown 文档渲染）
 * 移植自 src/routes/skills/[name]/+page.svelte
 * 原 Svelte 版使用 marked + highlight.js，此处使用 react-markdown + rehype-highlight（功能一致）
 */
import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { ChevronLeft, Folder, FileText } from 'lucide-react';
import 'highlight.js/styles/github-dark.css';
import './SkillDetailPage.scss';

interface SkillCard {
  name: string;
  description: string;
  path: string;
  version: string;
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

function versionLabel(v: string): string {
  return v ? `v${v}` : '';
}

// 剥离 YAML frontmatter
function stripFrontmatter(content: string): string {
  const trimmed = content.trimStart();
  if (trimmed.startsWith('---')) {
    const end = trimmed.indexOf('---', 3);
    if (end !== -1) {
      return trimmed.slice(end + 3);
    }
  }
  return content;
}

export default function SkillDetailPage() {
  const { name: skillName } = useParams<{ name: string }>();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [skill, setSkill] = useState<SkillCard | null>(null);
  const [docMarkdown, setDocMarkdown] = useState('');

  useEffect(() => {
    let disposed = false;

    async function load() {
      if (!skillName) {
        setError('缺少技能名称');
        setLoading(false);
        return;
      }

      try {
        // 查找匹配的技能
        const allSkills = await invoke<SkillCard[]>('list_skills');
        const found = allSkills.find((s) => s.name === skillName);
        if (!found) {
          if (!disposed) {
            setError(`未找到技能「${skillName}」`);
            setLoading(false);
          }
          return;
        }
        if (disposed) return;
        setSkill(found);

        // 读取并渲染文档
        const content = await invoke<string>('read_skill_doc', { path: found.path });
        if (!disposed) {
          setDocMarkdown(stripFrontmatter(content));
        }
      } catch (e) {
        if (!disposed) setError(`加载失败: ${e}`);
      } finally {
        if (!disposed) setLoading(false);
      }
    }

    void load();
    return () => {
      disposed = true;
    };
  }, [skillName]);

  return (
    <div className="detail-page">
      <div className="page-header">
        <div className="header-left">
          <Link to="/skills" className="btn-back" title="返回列表">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回列表
          </Link>
        </div>
      </div>

      {loading && (
        <div className="loading">
          <div className="spinner" />
          <span>加载中...</span>
        </div>
      )}

      {error && (
        <div className="error-banner">
          <span>⚠️</span>
          <span>{error}</span>
        </div>
      )}

      {skill && !loading && (
        <>
          <div className="skill-header-card">
            <div className="skill-header-row">
              <span className="skill-big-icon">{skillIcon(skill.name)}</span>
              <div className="skill-meta">
                <h1>{skill.name}</h1>
                {skill.version && (
                  <span className="skill-version-badge">{versionLabel(skill.version)}</span>
                )}
                {skill.description && <p className="skill-desc-text">{skill.description}</p>}
              </div>
            </div>
            <div className="skill-path-row" title={skill.path}>
              <Folder size={14} strokeWidth={2} />
              <span>{skill.path}</span>
            </div>
          </div>

          <div className="doc-card">
            <div className="doc-header">
              <FileText size={16} strokeWidth={2} />
              <span>文档内容</span>
            </div>
            <div className="doc-content markdown-body">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                rehypePlugins={[rehypeHighlight]}
              >
                {docMarkdown}
              </ReactMarkdown>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
