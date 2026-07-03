<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';

  // ===== 类型 =====
  interface SkillCard {
    name: string;
    description: string;
    path: string;
    version: string;
  }

  // ===== 状态 =====
  let skills = $state<SkillCard[]>([]);
  let loading = $state(true);
  let error = $state('');
  let deleteTarget = $state<SkillCard | null>(null);

  // ===== 加载 =====
  onMount(loadSkills);

  async function loadSkills() {
    loading = true;
    error = '';
    try {
      skills = await invoke<SkillCard[]>('list_skills');
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  }

  // ===== 删除 =====
  function confirmDelete(skill: SkillCard) {
    deleteTarget = skill;
  }

  function cancelDelete() {
    deleteTarget = null;
  }

  async function doDelete() {
    const target = deleteTarget;
    if (!target) return;
    try {
      await invoke('delete_skill', { path: target.path });
      skills = skills.filter(s => s.path !== target.path);
      deleteTarget = null;
    } catch (e) {
      error = `删除失败: ${e}`;
      deleteTarget = null;
    }
  }

  // ===== 版本标签颜色 =====
  function versionLabel(v: string): string {
    return v ? `v${v}` : '';
  }

  // ===== 获取技能图标 =====
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
</script>

<div class="skills-page">
  <div class="page-header">
    <div class="header-left">
      <a href="/settings" class="btn-back" title="返回设置">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
        返回设置
      </a>
      <h1>Skills 管理</h1>
      <p class="subtitle">管理和配置本地的 AI 技能</p>
    </div>
    <button class="btn-refresh" onclick={loadSkills} disabled={loading}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
      刷新
    </button>
  </div>

  <!-- 错误提示 -->
  {#if error}
    <div class="error-banner">
      <span>⚠️</span>
      <span>{error}</span>
      <button class="error-dismiss" onclick={() => error = ''}>✕</button>
    </div>
  {/if}

  <!-- 加载 -->
  {#if loading}
    <div class="loading">
      <div class="spinner"></div>
      <span>正在加载 Skills...</span>
    </div>
  {/if}

  <!-- 空状态 -->
  {#if !loading && skills.length === 0 && !error}
    <div class="empty-state">
      <span class="empty-icon">📭</span>
      <h3>还没有安装任何 Skill</h3>
      <p>使用 <code>npx skills add</code> 安装技能后，它们会出现在这里</p>
    </div>
  {/if}

  <!-- 技能卡片 -->
  {#if !loading && skills.length > 0}
    <div class="skills-count">{skills.length} 个技能</div>
    <div class="skills-grid">
      {#each skills as skill}
        <div class="skill-card">
          <div class="skill-header">
            <span class="skill-icon">{skillIcon(skill.name)}</span>
            <div class="skill-info">
              <h3 class="skill-name">{skill.name}</h3>
              {#if skill.version}
                <span class="skill-version">{versionLabel(skill.version)}</span>
              {/if}
            </div>
          </div>
          {#if skill.description}
            <p class="skill-desc">{skill.description}</p>
          {/if}
          <div class="skill-path" title={skill.path}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
            <span>{skill.path}</span>
          </div>
          <div class="skill-footer">
            <button class="btn-delete" onclick={() => confirmDelete(skill)}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              删除
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- 删除确认弹窗 -->
  {#if deleteTarget}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
    <div class="modal-overlay" onclick={cancelDelete} role="presentation">
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_roles -->
      <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="确认删除" tabindex="-1">
        <div class="modal-header">
          <h2>确认删除</h2>
          <button class="modal-close" onclick={cancelDelete} aria-label="取消">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
        <div class="modal-body">
          <div class="confirm-icon">⚠️</div>
          <p class="confirm-text">
            确定要删除 Skill <strong>{deleteTarget.name}</strong> 吗？
          </p>
          <p class="confirm-hint">此操作将永久删除该技能的文件夹，不可恢复。</p>
          <div class="confirm-path">{deleteTarget.path}</div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" onclick={cancelDelete}>取消</button>
          <button class="btn-danger" onclick={doDelete}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            确认删除
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .skills-page {
    max-width: 1100px;
    margin: 0 auto;
    animation: fadeIn 0.3s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 28px;
  }

  .page-header h1 {
    font-size: 26px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 6px;
  }

  .subtitle {
    color: var(--text-muted);
    font-size: 14px;
  }

  .btn-back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 12px;
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

  .btn-refresh {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-refresh:hover:not(:disabled) {
    background: var(--bg-subtle);
    border-color: var(--border-strong);
  }

  .btn-refresh:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* 错误 */
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

  .error-dismiss {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--error-muted);
    cursor: pointer;
    font-size: 16px;
  }

  .error-dismiss:hover {
    color: var(--error-text);
  }

  /* 加载 */
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

  /* 空状态 */
  .empty-state {
    text-align: center;
    padding: 80px 20px;
    background: var(--bg-card);
    border-radius: 16px;
    border: 2px dashed var(--border);
  }

  .empty-icon {
    font-size: 56px;
    display: block;
    margin-bottom: 16px;
  }

  .empty-state h3 {
    font-size: 20px;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .empty-state p {
    color: var(--text-muted);
    font-size: 15px;
  }

  .empty-state code {
    background: var(--bg-subtle);
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 14px;
    color: var(--text-secondary);
  }

  /* 计数 */
  .skills-count {
    font-size: 14px;
    color: var(--text-muted);
    margin-bottom: 16px;
  }

  /* 卡片网格 */
  .skills-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
    gap: 16px;
  }

  .skill-card {
    background: var(--bg-card);
    border-radius: 14px;
    padding: 20px;
    box-shadow: 0 1px 3px var(--shadow-sm);
    border: 1px solid var(--border-light);
    display: flex;
    flex-direction: column;
    gap: 12px;
    transition: all 0.2s ease;
  }

  .skill-card:hover {
    box-shadow: 0 4px 12px var(--shadow-hover);
    border-color: var(--border);
  }

  .skill-header {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .skill-icon {
    font-size: 28px;
    flex-shrink: 0;
    width: 48px;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border-radius: 12px;
  }

  .skill-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .skill-name {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: ui-monospace, monospace;
  }

  .skill-version {
    font-size: 11px;
    color: var(--accent);
    background: var(--accent-bg);
    padding: 2px 8px;
    border-radius: 10px;
    font-weight: 500;
    align-self: flex-start;
  }

  .skill-desc {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .skill-path {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .skill-path svg {
    flex-shrink: 0;
  }

  .skill-path span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .skill-footer {
    display: flex;
    justify-content: flex-end;
    padding-top: 4px;
    border-top: 1px solid var(--border-light);
  }

  .btn-delete {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 14px;
    background: none;
    border: 1px solid var(--border-light);
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-delete:hover {
    color: var(--error-text);
    background: var(--error-bg);
    border-color: var(--error-border);
  }

  /* 弹窗 */
  .modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0,0,0,0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fadeInOverlay 0.2s ease;
  }

  @keyframes fadeInOverlay {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .modal {
    background: var(--bg-card);
    border-radius: 20px;
    width: 420px;
    max-width: 90vw;
    box-shadow: 0 24px 48px rgba(0,0,0,0.3);
    border: 1px solid var(--border);
    animation: slideUp 0.25s ease;
  }

  @keyframes slideUp {
    from { opacity: 0; transform: translateY(16px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 24px 0;
  }

  .modal-header h2 {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .modal-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
    transition: all 0.2s;
  }

  .modal-close:hover {
    background: var(--bg-subtle);
    color: var(--text-secondary);
  }

  .modal-body {
    padding: 20px 24px;
    text-align: center;
  }

  .confirm-icon {
    font-size: 40px;
    margin-bottom: 12px;
  }

  .confirm-text {
    font-size: 15px;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .confirm-hint {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 12px;
  }

  .confirm-path {
    font-size: 12px;
    color: var(--text-muted);
    font-family: ui-monospace, monospace;
    padding: 8px 12px;
    background: var(--bg-subtle);
    border: 1px solid var(--border-light);
    border-radius: 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 24px 20px;
  }

  .btn-cancel {
    padding: 10px 20px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-cancel:hover {
    background: var(--bg-card-hover);
  }

  .btn-danger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 10px 20px;
    background: var(--error-text);
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-danger:hover {
    background: #b91c1c;
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(220, 38, 38, 0.3);
  }
</style>
