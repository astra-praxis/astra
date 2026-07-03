<script lang="ts">
  import { page } from '$app/stores';
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { marked } from 'marked';
  import hljs from 'highlight.js';
  import 'highlight.js/styles/github-dark.css';

  // 配置 marked 使用 highlight.js
  const renderer = {
    code({ text, lang }: { text: string; lang?: string }) {
      const language = (lang && hljs.getLanguage(lang)) ? lang : 'plaintext';
      const highlighted = hljs.highlight(text, { language }).value;
      return `<pre><code class="hljs language-${language}">${highlighted}</code></pre>`;
    }
  };

  marked.use({ renderer });

  interface SkillCard {
    name: string;
    description: string;
    path: string;
    version: string;
  }

  let loading = $state(true);
  let error = $state('');
  let skill = $state<SkillCard | null>(null);
  let docHtml = $state('');

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

  onMount(async () => {
    const skillName = $page.params.name;
    if (!skillName) {
      error = '缺少技能名称';
      loading = false;
      return;
    }

    try {
      // 查找匹配的技能
      const allSkills = await invoke<SkillCard[]>('list_skills');
      const found = allSkills.find(s => s.name === skillName);
      if (!found) {
        error = `未找到技能「${skillName}」`;
        loading = false;
        return;
      }
      skill = found;

      // 读取并渲染文档
      const content = await invoke<string>('read_skill_doc', { path: found.path });
      docHtml = await marked.parse(stripFrontmatter(content));
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  });
</script>

<div class="detail-page">
  <div class="page-header">
    <div class="header-left">
      <a href="/skills" class="btn-back" title="返回列表">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
        返回列表
      </a>
    </div>
  </div>

  {#if loading}
    <div class="loading">
      <div class="spinner"></div>
      <span>加载中...</span>
    </div>
  {/if}

  {#if error}
    <div class="error-banner">
      <span>⚠️</span>
      <span>{error}</span>
    </div>
  {/if}

  {#if skill && !loading}
    <div class="skill-header-card">
      <div class="skill-header-row">
        <span class="skill-big-icon">{skillIcon(skill.name)}</span>
        <div class="skill-meta">
          <h1>{skill.name}</h1>
          {#if skill.version}
            <span class="skill-version-badge">{versionLabel(skill.version)}</span>
          {/if}
          {#if skill.description}
            <p class="skill-desc-text">{skill.description}</p>
          {/if}
        </div>
      </div>
      <div class="skill-path-row" title={skill.path}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
        <span>{skill.path}</span>
      </div>
    </div>

    <div class="doc-card">
      <div class="doc-header">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/></svg>
        <span>文档内容</span>
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="doc-content markdown-body">{@html docHtml}</div>
    </div>
  {/if}
</div>



<style>
  .detail-page {
    max-width: 900px;
    margin: 0 auto;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .page-header {
    margin-bottom: 20px;
  }

  .btn-back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px 5px 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-subtle);
    border: 1px solid var(--border-light);
    border-radius: 8px;
    text-decoration: none;
    align-self: flex-start;
    transition: background 0.2s ease, color 0.2s ease;
  }

  .btn-back:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 60px;
    color: var(--text-secondary);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    border-radius: 12px;
    color: var(--error-text);
    margin-bottom: 20px;
    font-size: 14px;
  }

  .skill-header-card {
    background: var(--bg-card);
    border-radius: 16px;
    padding: 24px;
    border: 1px solid var(--border-light);
    box-shadow: 0 1px 3px var(--shadow-sm);
    margin-bottom: 20px;
  }

  .skill-header-row {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }

  .skill-big-icon {
    font-size: 40px;
    flex-shrink: 0;
    width: 64px;
    height: 64px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border-radius: 16px;
  }

  .skill-meta {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .skill-meta h1 {
    font-size: 22px;
    font-weight: 700;
    color: var(--text-primary);
    font-family: ui-monospace, monospace;
  }

  .skill-version-badge {
    font-size: 11px;
    color: var(--accent);
    background: var(--accent-bg);
    padding: 2px 10px;
    border-radius: 10px;
    font-weight: 500;
    align-self: flex-start;
  }

  .skill-desc-text {
    font-size: 14px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin-top: 2px;
  }

  .skill-path-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px solid var(--border-light);
    font-size: 12px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .skill-path-row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* 文档卡片 */
  .doc-card {
    background: var(--bg-card);
    border-radius: 16px;
    border: 1px solid var(--border-light);
    box-shadow: 0 1px 3px var(--shadow-sm);
    overflow: hidden;
  }

  .doc-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 20px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border-light);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .doc-content {
    padding: 28px 32px;
    background: var(--bg-app);
    overflow-x: auto;
  }

  /* Markdown 渲染样式 */
  .doc-content :global(h1),
  .doc-content :global(h2),
  .doc-content :global(h3),
  .doc-content :global(h4) {
    color: var(--text-primary);
    font-weight: 700;
    margin-top: 1.5em;
    margin-bottom: 0.5em;
    line-height: 1.3;
  }

  .doc-content :global(h1) {
    font-size: 1.6em;
    border-bottom: 1px solid var(--border-light);
    padding-bottom: 0.3em;
  }

  .doc-content :global(h2) {
    font-size: 1.3em;
    padding: 0 0 0.3em 14px;
    border-left: 4px solid var(--accent);
    border-radius: 0 2px 2px 0;
    background: linear-gradient(to right, var(--accent-light), transparent);
  }

  .doc-content :global(h3) {
    font-size: 1.1em;
  }

  .doc-content :global(p) {
    margin: 0.8em 0;
    line-height: 1.7;
    color: var(--text-primary);
    font-size: 14px;
  }

  .doc-content :global(ul),
  .doc-content :global(ol) {
    margin: 0.5em 0;
    padding-left: 1.8em;
    color: var(--text-primary);
    font-size: 14px;
  }

  .doc-content :global(li) {
    margin: 0.3em 0;
    line-height: 1.6;
  }

  .doc-content :global(li > p) {
    margin: 0;
  }

  .doc-content :global(strong) {
    color: var(--text-primary);
    font-weight: 600;
  }

  .doc-content :global(a) {
    color: var(--accent);
    text-decoration: none;
  }

  .doc-content :global(a:hover) {
    text-decoration: underline;
  }

  .doc-content :global(blockquote) {
    margin: 0.8em 0;
    padding: 8px 16px;
    border-left: 4px solid var(--accent);
    background: var(--bg-subtle);
    border-radius: 0 8px 8px 0;
    color: var(--text-secondary);
  }

  .doc-content :global(blockquote p) {
    margin: 0;
  }

  .doc-content :global(hr) {
    border: none;
    border-top: 1px solid var(--border-light);
    margin: 1.5em 0;
  }

  .doc-content :global(code) {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    background: var(--bg-subtle);
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--accent);
  }

  .doc-content :global(pre) {
    margin: 1em 0;
    border-radius: 12px;
    overflow: hidden;
    border: 1px solid var(--border-light);
  }

  .doc-content :global(pre code) {
    padding: 0;
    font-size: 13px;
    line-height: 1.6;
  }

  .doc-content :global(table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1em 0;
    font-size: 13px;
  }

  .doc-content :global(th),
  .doc-content :global(td) {
    padding: 8px 12px;
    border: 1px solid var(--border-light);
    text-align: left;
  }

  .doc-content :global(th) {
    background: var(--bg-subtle);
    font-weight: 600;
    color: var(--text-primary);
  }

  .doc-content :global(td) {
    color: var(--text-primary);
  }

  .doc-content :global(img) {
    max-width: 100%;
    border-radius: 8px;
    margin: 1em 0;
  }

  .doc-content :global(::-webkit-scrollbar) {
    height: 6px;
  }

  .doc-content :global(::-webkit-scrollbar-thumb) {
    background: var(--border);
    border-radius: 3px;
  }
</style>
