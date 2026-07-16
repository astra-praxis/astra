<script lang="ts">
  import { goto } from '$app/navigation';

  // ===== 工具列表 =====
  interface Tool {
    id: string;
    name: string;
    description: string;
    icon: string;
    path: string;
    status: 'stable' | 'beta' | 'alpha';
  }

  const tools: Tool[] = [
    {
      id: 'downloader',
      name: '下载管理',
      description: '支持断点续传的文件下载工具，可管理多个下载任务',
      icon: '📥',
      path: '/tools/downloader',
      status: 'stable'
    },
    {
      id: 'nm-clean',
      name: 'node_modules 清理',
      description: '扫描工作空间中的所有 node_modules 文件夹，查看大小并一键清理，清理过程实时显示进度',
      icon: '🗑️',
      path: '/tools/nm',
      status: 'stable'
    },
  ];

  let searchQuery = $state('');

  const filteredTools = $derived(
    searchQuery.trim()
      ? tools.filter(
          t =>
            t.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
            t.description.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : tools
  );

  function openTool(tool: Tool) {
    goto(tool.path);
  }

  function getStatusLabel(status: string) {
    switch (status) {
      case 'stable':
        return { text: '稳定', class: 'status-stable' };
      case 'beta':
        return { text: '测试', class: 'status-beta' };
      case 'alpha':
        return { text: '实验', class: 'status-alpha' };
      default:
        return { text: '', class: '' };
    }
  }
</script>

<div class="tools-page">
  <!-- 页头 -->
  <div class="page-header">
    <div class="header-left">
      <a href="/settings" class="btn-back" title="返回设置">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="15 18 9 12 15 6" />
        </svg>
      </a>
      <h1>工具箱</h1>
    </div>
  </div>

  <!-- 搜索栏 -->
  <div class="search-section">
    <div class="search-box">
      <svg class="search-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8" />
        <line x1="21" y1="21" x2="16.65" y2="16.65" />
      </svg>
      <input
        type="text"
        placeholder="搜索工具..."
        bind:value={searchQuery}
      />
      {#if searchQuery}
        <button class="clear-btn" onclick={() => (searchQuery = '')} aria-label="清除搜索">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      {/if}
    </div>
    <div class="tools-count">{filteredTools.length} 个工具</div>
  </div>

  <!-- 工具卡片列表 -->
  <div class="tools-grid">
    {#each filteredTools as tool (tool.id)}
      {@const statusInfo = getStatusLabel(tool.status)}
      <div class="tool-card" onclick={() => openTool(tool)} role="button" tabindex="0" onkeydown={(e) => e.key === 'Enter' && openTool(tool)}>
        <div class="tool-header">
          <span class="tool-icon">{tool.icon}</span>
          <div class="tool-info">
            <h3 class="tool-name">
              {tool.name}
              <span class="status-badge {statusInfo.class}">{statusInfo.text}</span>
            </h3>
            <p class="tool-desc">{tool.description}</p>
          </div>
        </div>
        <div class="tool-footer">
          <span class="tool-action">
            打开
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="9 18 15 12 9 6" />
            </svg>
          </span>
        </div>
      </div>
    {/each}

    {#if filteredTools.length === 0}
      <div class="empty-state">
        <span class="empty-icon">🔍</span>
        <h3>未找到匹配的工具</h3>
        <p>尝试其他关键词搜索</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .tools-page {
    padding: 24px;
    max-width: 900px;
    margin: 0 auto;
  }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 24px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .btn-back {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-back:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  .page-header h1 {
    font-size: 24px;
    font-weight: 600;
    color: var(--text-primary);
  }

  /* 搜索栏 */
  .search-section {
    margin-bottom: 24px;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 10px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px 16px;
    transition: all 0.2s;
  }

  .search-box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-ring);
  }

  .search-icon {
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .search-box input {
    flex: 1;
    border: none;
    background: transparent;
    font-size: 15px;
    color: var(--text-primary);
    outline: none;
  }

  .search-box input::placeholder {
    color: var(--text-muted);
  }

  .clear-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: 4px;
  }

  .clear-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text-secondary);
  }

  .tools-count {
    margin-top: 12px;
    font-size: 13px;
    color: var(--text-muted);
  }

  /* 工具卡片 */
  .tools-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .tool-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    cursor: pointer;
    transition: all 0.2s;
  }

  .tool-card:hover {
    background: var(--bg-card-hover);
    border-color: var(--accent);
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  }

  .tool-header {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }

  .tool-icon {
    font-size: 36px;
    line-height: 1;
  }

  .tool-info {
    flex: 1;
  }

  .tool-name {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-badge {
    font-size: 11px;
    font-weight: 500;
    padding: 2px 6px;
    border-radius: 4px;
  }

  .status-stable {
    background: rgba(34, 197, 94, 0.15);
    color: #16a34a;
  }

  .status-beta {
    background: rgba(59, 130, 246, 0.15);
    color: #2563eb;
  }

  .status-alpha {
    background: rgba(245, 158, 11, 0.15);
    color: #d97706;
  }

  .tool-desc {
    font-size: 14px;
    color: var(--text-secondary);
    margin: 6px 0 0;
    line-height: 1.5;
  }

  .tool-footer {
    display: flex;
    justify-content: flex-end;
  }

  .tool-action {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 14px;
    font-weight: 500;
    color: var(--accent);
  }

  .tool-card:hover .tool-action {
    color: var(--accent-hover);
  }

  /* 空状态 */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px;
    color: var(--text-muted);
    text-align: center;
  }

  .empty-icon {
    font-size: 48px;
    margin-bottom: 16px;
  }

  .empty-state h3 {
    font-size: 16px;
    font-weight: 500;
    color: var(--text-secondary);
    margin: 0;
  }

  .empty-state p {
    font-size: 14px;
    margin: 8px 0 0;
  }

  @media (max-width: 640px) {
    .tools-page {
      padding: 16px;
    }

    .tool-header {
      flex-direction: column;
      gap: 12px;
    }

    .tool-icon {
      font-size: 32px;
    }
  }
</style>