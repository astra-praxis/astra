<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';

  // ===== 类型 =====
  interface ModelConfig {
    id: string;
    name: string;
    model_path: string;
    server_path: string;
    port: number;
    ngl: number;
  }

  // ===== 状态 =====
  let model = $state<ModelConfig | null>(null);
  let status = $state<string | null>(null);
  let loading = $state(true);
  let error = $state('');
  let actionLoading = $state(false);
  let activeTab = $state<'curl' | 'python' | 'rust'>('curl');
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  // 从 URL 获取 id
  const modelId = $derived($page.params.id);

  // ===== 派生值 =====
  const isRunning = $derived(status === 'running');

  // llama-server Web UI 地址
  const webuiUrl = $derived(model ? `http://localhost:${model.port}` : '');

  const startCommand = $derived(
    model
      ? `${model.server_path} serve -m ${model.model_path} --host 0.0.0.0 --port ${model.port}${model.ngl > 0 ? ` -ngl ${model.ngl}` : ''}`
      : ''
  );

  const curlExample = $derived(
    model
      ? `curl http://localhost:${model.port}/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "local",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'`
      : ''
  );

  const pythonExample = $derived(
    model
      ? `import requests

response = requests.post(
    "http://localhost:${model.port}/v1/chat/completions",
    json={
        "model": "local",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    },
    headers={"Content-Type": "application/json"}
)

print(response.json())`
      : ''
  );

  const rustExample = $derived(
    model
      ? `use reqwest;
use serde_json::json;

async fn chat() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let resp = client
        .post("http://localhost:${model.port}/completion")
        .json(&json!({
            "prompt": "Hello",
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    println!("{:?}", resp);
    Ok(())
}`
      : ''
  );

  // ===== 加载 =====
  onMount(async () => {
    await loadModel();
    startPolling();
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  async function loadModel() {
    loading = true;
    error = '';
    try {
      const all = await invoke<ModelConfig[]>('list_model_configs');
      const found = all.find(m => m.id === modelId);
      if (!found) {
        error = '未找到该模型配置';
      } else {
        model = found;
        await pollStatus();
      }
    } catch (e) {
      error = `加载失败: ${e}`;
    } finally {
      loading = false;
    }
  }

  async function pollStatus() {
    if (!model) return;
    try {
      status = await invoke<string | null>('check_server_status', { port: model.port });
    } catch {
      // 静默忽略
    }
  }

  function startPolling() {
    pollStatus();
    pollInterval = setInterval(pollStatus, 3000);
  }

  // ===== 启动/停止 =====
  async function startModel() {
    if (!model) return;
    actionLoading = true;
    try {
      await invoke('start_llama_server', {
        serverPath: model.server_path,
        modelPath: model.model_path,
        port: model.port,
        ngl: model.ngl,
        modelName: model.name
      });
      await pollStatus();
    } catch (e) {
      error = `启动失败: ${e}`;
    } finally {
      actionLoading = false;
    }
  }

  async function stopModel() {
    if (!model) return;
    actionLoading = true;
    try {
      await invoke('stop_llama_server', { port: model.port });
      await pollStatus();
    } catch (e) {
      error = `停止失败: ${e}`;
    } finally {
      actionLoading = false;
    }
  }

  // ===== 复制到剪贴板 =====
  let copied = $state(false);
  function copyText(text: string) {
    navigator.clipboard.writeText(text);
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }

  function copyCurrentCode() {
    const code = activeTab === 'curl' ? curlExample : activeTab === 'python' ? pythonExample : rustExample;
    copyText(code);
  }
</script>

<div class="detail-page">
  <!-- 页头 -->
  <div class="page-header">
    <button class="btn-back" onclick={() => goto('/models')}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="19" y1="12" x2="5" y2="12"/><polyline points="12 19 5 12 12 5"/></svg>
      返回
    </button>
  </div>

  <!-- 加载 -->
  {#if loading}
    <div class="loading">
      <div class="spinner"></div>
      <span>正在加载...</span>
    </div>
  {/if}

  <!-- 错误 -->
  {#if !loading && error}
    <div class="error-banner">
      <span>⚠️</span>
      <span>{error}</span>
    </div>
  {/if}

  {#if !loading && model}
    <div class="detail-content">
      <!-- 模型信息卡片 -->
      <div class="info-card">
        <div class="info-header">
          <div class="info-title">
            <span class="model-icon">🧩</span>
            <div>
              <h1>{model.name}</h1>
              <div class="info-meta">
                <span class="status-badge {isRunning ? 'status-running' : 'status-stopped'}">
                  {isRunning ? '运行中' : '已停止'}
                </span>
                <span class="meta-tag">端口 {model.port}</span>
                <span class="meta-tag">ngl {model.ngl}</span>
              </div>
            </div>
          </div>
          <div class="info-actions">
            {#if isRunning}
              <button class="btn-stop" onclick={stopModel} disabled={actionLoading}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                {actionLoading ? '处理中...' : '停止服务'}
              </button>
            {:else}
              <button class="btn-start" onclick={startModel} disabled={actionLoading}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                {actionLoading ? '处理中...' : '启动服务'}
              </button>
            {/if}
          </div>
        </div>

        <div class="info-grid">
          <div class="info-item">
            <span class="info-label">模型名称</span>
            <span class="info-value">{model.name}</span>
          </div>
          <div class="info-item">
            <span class="info-label">端口</span>
            <span class="info-value">{model.port}</span>
          </div>
          <div class="info-item">
            <span class="info-label">ngl (GPU 层数)</span>
            <span class="info-value">{model.ngl}</span>
          </div>
          <div class="info-item">
            <span class="info-label">模型 ID</span>
            <span class="info-value mono">{model.id}</span>
          </div>
          <div class="info-item info-item-full">
            <span class="info-label">模型文件路径</span>
            <span class="info-value mono path-value" title={model.model_path}>{model.model_path}</span>
          </div>
          <div class="info-item info-item-full">
            <span class="info-label">llama-server 路径</span>
            <span class="info-value mono path-value" title={model.server_path}>{model.server_path}</span>
          </div>
        </div>
      </div>

      <!-- 启动命令 -->
      <div class="section-card">
        <div class="section-title">
          <h2>启动命令</h2>
          <button class="btn-copy" onclick={() => copyText(startCommand)}>
            {copied ? '✓ 已复制' : '复制'}
          </button>
        </div>
        <div class="code-block">
          <pre><code>{startCommand}</code></pre>
        </div>
      </div>

      <!-- API 使用文档 -->
      <div class="section-card">
        <div class="section-title">
          <h2>API 使用文档</h2>
        </div>
        <div class="api-note">
          <strong>💡 OpenAI 兼容:</strong>
          llama-server 提供了 OpenAI 兼容的 API，可以在 <code>/v1/chat/completions</code> 端点使用标准的 OpenAI 格式进行对话。
        </div>

        <!-- 选项卡 -->
        <div class="tabs">
          <button class="tab {activeTab === 'curl' ? 'active' : ''}" onclick={() => activeTab = 'curl'}>curl</button>
          <button class="tab {activeTab === 'python' ? 'active' : ''}" onclick={() => activeTab = 'python'}>Python</button>
          <button class="tab {activeTab === 'rust' ? 'active' : ''}" onclick={() => activeTab = 'rust'}>Rust</button>
          <button class="btn-copy-inline" onclick={copyCurrentCode}>
            {copied ? '✓ 已复制' : '📋 复制'}
          </button>
        </div>

        <!-- curl -->
        {#if activeTab === 'curl'}
          <div class="code-block">
            <pre><code>{curlExample}</code></pre>
          </div>
        {/if}

        <!-- Python -->
        {#if activeTab === 'python'}
          <div class="code-block">
            <pre><code>{pythonExample}</code></pre>
          </div>
        {/if}

        <!-- Rust -->
        {#if activeTab === 'rust'}
          <div class="code-block">
            <pre><code>{rustExample}</code></pre>
          </div>
        {/if}

        <!-- API 端点信息 -->
        <div class="endpoint-info">
          <div class="endpoint-item">
            <span class="endpoint-method">POST</span>
            <span class="endpoint-path">/v1/chat/completions</span>
            <span class="endpoint-desc">OpenAI 兼容聊天接口</span>
          </div>
          <div class="endpoint-item">
            <span class="endpoint-method">POST</span>
            <span class="endpoint-path">/completion</span>
            <span class="endpoint-desc">llama.cpp 原生补全接口</span>
          </div>
        </div>
      </div>

      <!-- llama-server Web UI -->
      <div class="section-card">
        <div class="section-title">
          <h2>在线对话</h2>
          {#if isRunning}
            <a href={webuiUrl} target="_blank" rel="noopener" class="btn-copy">🔗 新窗口打开</a>
          {/if}
        </div>
        {#if isRunning}
          <iframe
            src={webuiUrl}
            title="llama-server Web UI"
            class="webui-iframe"
            sandbox="allow-scripts allow-same-origin allow-forms"
          ></iframe>
        {:else}
          <div class="webui-placeholder">
            <span class="placeholder-icon">💤</span>
            <p>模型未运行，请先启动服务以加载在线对话界面</p>
          </div>
        {/if}
      </div>
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

  /* 页头 */
  .page-header {
    margin-bottom: 20px;
  }

  .btn-back {
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

  .btn-back:hover {
    background: var(--bg-subtle);
    color: var(--text-primary);
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
    font-size: 14px;
  }

  /* 信息卡片 */
  .info-card {
    background: var(--bg-card);
    border-radius: 16px;
    padding: 24px;
    border: 1px solid var(--border-light);
    box-shadow: 0 1px 3px var(--shadow-sm);
    margin-bottom: 20px;
  }

  .info-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 24px;
  }

  .info-title {
    display: flex;
    align-items: flex-start;
    gap: 14px;
  }

  .model-icon {
    font-size: 32px;
    flex-shrink: 0;
    width: 56px;
    height: 56px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-subtle);
    border-radius: 14px;
  }

  .info-title h1 {
    font-size: 22px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 8px;
  }

  .info-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .meta-tag {
    color: var(--text-secondary);
    background: var(--bg-subtle);
    padding: 2px 10px;
    border-radius: 6px;
    font-size: 12px;
  }

  /* 状态徽章 */
  .status-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 4px 10px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
  }

  .status-running {
    color: var(--success-text);
    background: var(--success-bg);
  }

  .status-running::before {
    content: '';
    display: inline-block;
    width: 6px;
    height: 6px;
    background: var(--success-text);
    border-radius: 50%;
    margin-right: 6px;
    animation: pulse 1.5s ease infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .status-stopped {
    color: var(--text-muted);
    background: var(--bg-subtle);
  }

  /* 信息网格 */
  .info-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  .info-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .info-item-full {
    grid-column: 1 / -1;
  }

  .info-label {
    font-size: 12px;
    color: var(--text-muted);
    font-weight: 500;
  }

  .info-value {
    font-size: 14px;
    color: var(--text-primary);
  }

  .info-value.mono {
    font-family: ui-monospace, monospace;
    font-size: 13px;
  }

  .path-value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* 操作按钮 */
  .info-actions {
    flex-shrink: 0;
  }

  .btn-start, .btn-stop {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 10px 20px;
    border: none;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-start {
    background: var(--accent);
  }

  .btn-start:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: translateY(-1px);
  }

  .btn-stop {
    background: var(--error-text);
  }

  .btn-stop:hover:not(:disabled) {
    background: #b91c1c;
    transform: translateY(-1px);
  }

  .btn-start:disabled, .btn-stop:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* 区块卡片 */
  .section-card {
    background: var(--bg-card);
    border-radius: 16px;
    padding: 24px;
    border: 1px solid var(--border-light);
    box-shadow: 0 1px 3px var(--shadow-sm);
    margin-bottom: 20px;
  }

  .section-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .section-title h2 {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .btn-copy {
    padding: 6px 14px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-copy:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  /* 代码块 */
  .code-block {
    background: #1e1e2e;
    border-radius: 12px;
    padding: 20px;
    overflow-x: auto;
  }

  .code-block pre {
    margin: 0;
  }

  .code-block code {
    font-family: 'SF Mono', 'Fira Code', ui-monospace, monospace;
    font-size: 13px;
    line-height: 1.6;
    color: #cdd6f4;
    white-space: pre;
  }

  /* API 说明 */
  .api-note {
    padding: 14px 16px;
    background: var(--accent-bg);
    border: 1px solid var(--accent-ring);
    border-radius: 12px;
    font-size: 13px;
    color: var(--text-secondary);
    margin-bottom: 16px;
    line-height: 1.6;
  }

  .api-note code {
    font-family: ui-monospace, monospace;
    background: var(--bg-subtle);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 12px;
  }

  /* 选项卡 */
  .tabs {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--border-light);
    padding-bottom: 0;
  }

  .tab {
    padding: 8px 18px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.2s;
    margin-bottom: -1px;
    border-radius: 0;
  }

  .tab:hover {
    color: var(--text-secondary);
  }

  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .btn-copy-inline {
    margin-left: auto;
    padding: 6px 14px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-copy-inline:hover {
    background: var(--bg-card-hover);
    color: var(--text-primary);
  }

  /* 端点信息 */
  .endpoint-info {
    margin-top: 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .endpoint-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background: var(--bg-subtle);
    border-radius: 10px;
    font-size: 13px;
  }

  .endpoint-method {
    font-family: ui-monospace, monospace;
    font-size: 11px;
    font-weight: 700;
    color: var(--success-text);
    background: var(--success-bg);
    padding: 3px 8px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .endpoint-path {
    font-family: ui-monospace, monospace;
    color: var(--text-primary);
    font-weight: 500;
    flex-shrink: 0;
  }

  .endpoint-desc {
    color: var(--text-muted);
    margin-left: auto;
  }

  /* ===== Web UI iframe ===== */
  .webui-iframe {
    width: 100%;
    height: 600px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: #fff;
  }

  .webui-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 400px;
    border: 1px dashed var(--border);
    border-radius: 10px;
    color: var(--text-muted);
    text-align: center;
    gap: 12px;
  }

  .placeholder-icon {
    font-size: 48px;
  }

  .webui-placeholder p {
    font-size: 14px;
    margin: 0;
  }

  /* 响应式 */
  @media (max-width: 640px) {
    .info-grid {
      grid-template-columns: 1fr;
    }
    .info-header {
      flex-direction: column;
    }
    .endpoint-item {
      flex-wrap: wrap;
    }
    .endpoint-desc {
      margin-left: 0;
      width: 100%;
    }
  }
</style>
