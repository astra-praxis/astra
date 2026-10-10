<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import Modal from '$lib/ui/Modal.svelte';

  // ===== 类型 =====
  interface SkillCard {
    name: string;
    description: string;
    path: string;
    version: string;
    is_git: boolean;
  }

  // ===== 状态 =====
  let skills = $state<SkillCard[]>([]);
  let loading = $state(true);
  let error = $state('');
  let deleteTarget = $state<SkillCard | null>(null);
  let updatingSkills = $state<Set<string>>(new Set()); // 正在更新的 skill 集合
  let updatingAll = $state(false); // 是否正在更新所有
  let updateResults = $state<Map<string, string>>(new Map()); // 更新结果

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

  // ===== 更新 =====
  async function updateSingleSkill(skill: SkillCard) {
    if (!skill.is_git) return;
    
    const skillPath = skill.path;
    updatingSkills = new Set([...updatingSkills, skillPath]);
    updateResults = new Map(updateResults);
    updateResults.delete(skillPath);
    updateResults = new Map(updateResults);
    
    try {
      const result = await invoke<string>('update_skill', { path: skillPath });
      const newResults = new Map(updateResults);
      newResults.set(skillPath, `✅ 更新成功\n${result}`);
      updateResults = newResults;
    } catch (e) {
      const newResults = new Map(updateResults);
      newResults.set(skillPath, `❌ 更新失败: ${e}`);
      updateResults = newResults;
    } finally {
      const newSet = new Set(updatingSkills);
      newSet.delete(skillPath);
      updatingSkills = newSet;
    }
  }

  async function updateAllSkills() {
    updatingAll = true;
    error = '';
    
    try {
      const results = await invoke<Array<[string, {Ok: string} | {Err: string}]>>('update_all_skills');
      
      // 处理结果
      const newResults = new Map(updateResults);
      for (const [name, result] of results) {
        const skill = skills.find(s => s.name === name);
        if (skill) {
          const skillPath = skill.path;
          if ('Ok' in result) {
            newResults.set(skillPath, `✅ 更新成功\n${result.Ok}`);
          } else {
            newResults.set(skillPath, `❌ 更新失败: ${result.Err}`);
          }
        }
      }
      updateResults = newResults;
      
      // 刷新列表
      await loadSkills();
    } catch (e) {
      error = `批量更新失败: ${e}`;
    } finally {
      updatingAll = false;
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
    <div class="header-actions">
      <button 
        class="btn-update-all" 
        onclick={updateAllSkills} 
        disabled={updatingAll || loading}
      >
        {#if updatingAll}
          <div class="spinner-small"></div>
          <span>更新中...</span>
        {:else}
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
          <span>一键更新所有</span>
        {/if}
      </button>
      <button class="btn-refresh" onclick={loadSkills} disabled={loading}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
        刷新
      </button>
    </div>
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
            {#if skill.is_git}
              <button 
                class="btn-update" 
                onclick={() => updateSingleSkill(skill)}
                disabled={updatingSkills.has(skill.path)}
              >
                {#if updatingSkills.has(skill.path)}
                  <div class="spinner-small"></div>
                  <span>更新中...</span>
                {:else}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
                  <span>更新</span>
                {/if}
              </button>
            {:else}
              <span class="skill-local-hint">本地生成</span>
            {/if}
            <button class="btn-delete" onclick={() => confirmDelete(skill)}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              删除
            </button>
            <a href="/skills/{skill.name}" class="btn-detail">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
              详情
            </a>
          </div>
          {#if updateResults.has(skill.path)}
            <div class="update-result">
              <pre>{updateResults.get(skill.path)}</pre>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <!-- 删除确认弹窗 -->
  {#if deleteTarget}
    <Modal title="确认删除" onClose={cancelDelete} variant="confirm">
      <div class="modal-body">
        <div class="confirm-icon">⚠️</div>
        <p class="confirm-text">
          确定要删除 Skill <strong>{deleteTarget.name}</strong> 吗？
        </p>
        <p class="confirm-hint">此操作将永久删除该技能的文件夹，不可恢复。</p>
        <div class="confirm-path">{deleteTarget.path}</div>
      </div>
      <div class="modal-footer">

        <button class="btn-danger" onclick={doDelete}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          确认删除
        </button>
      </div>
    </Modal>
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
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border-light);
  }

  .btn-detail {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 14px;
    background: none;
    border: 1px solid var(--border-light);
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    color: var(--accent);
    cursor: pointer;
    text-decoration: none;
    transition: all 0.2s;
  }

  .btn-detail:hover {
    background: var(--accent-bg);
    border-color: var(--accent);
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

  .btn-update {
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

  .btn-update:hover:not(:disabled) {
    color: var(--accent);
    background: var(--accent-bg);
    border-color: var(--accent);
  }

  .btn-update:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .skill-local-hint {
    font-size: 12px;
    color: var(--text-muted);
    padding: 6px 14px;
    background: var(--bg-subtle);
    border-radius: 8px;
    border: 1px solid var(--border-light);
  }

  .update-result {
    margin-top: 8px;
    padding: 10px 12px;
    background: var(--bg-subtle);
    border: 1px solid var(--border-light);
    border-radius: 8px;
    font-size: 12px;
  }

  .update-result pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, monospace;
    color: var(--text-secondary);
  }

  .header-actions {
    display: flex;
    gap: 10px;
  }

  .btn-update-all {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-update-all:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }

  .btn-update-all:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinner-small {
    width: 14px;
    height: 14px;
    border: 2px solid white;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

</style>
