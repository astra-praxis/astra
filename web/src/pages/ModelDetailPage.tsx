import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigate, useParams } from 'react-router';
import { ArrowLeft, Play, Square } from 'lucide-react';
import type { ModelConfig } from '@/features/models/types';
import './ModelDetailPage.scss';

type ApiTab = 'curl' | 'python' | 'rust';

export default function ModelDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  // ===== 状态 =====
  const [model, setModel] = useState<ModelConfig | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [actionLoading, setActionLoading] = useState(false);
  const [activeTab, setActiveTab] = useState<ApiTab>('curl');
  const [copied, setCopied] = useState(false);

  const modelRef = useRef<ModelConfig | null>(null);
  const copyTimerRef = useRef<number | null>(null);

  // ===== 派生值 =====
  const isRunning = status === 'running';

  // llama-server Web UI 地址
  const webuiUrl = model ? `http://localhost:${model.port}` : '';

  const startCommand = model
    ? `${model.server_path} serve -m ${model.model_path} --host 0.0.0.0 --port ${model.port}${
        model.ngl > 0 ? ` -ngl ${model.ngl}` : ''
      }`
    : '';

  const curlExample = model
    ? `curl http://localhost:${model.port}/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "local",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'`
    : '';

  const pythonExample = model
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
    : '';

  const rustExample = model
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
    : '';

  // ===== 加载 =====
  useEffect(() => {
    void loadModel();
    const interval = setInterval(() => {
      void pollStatus();
    }, 3000);
    return () => {
      clearInterval(interval);
      if (copyTimerRef.current !== null) {
        clearTimeout(copyTimerRef.current);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  async function loadModel() {
    setLoading(true);
    setError('');
    try {
      const all = await invoke<ModelConfig[]>('list_model_configs');
      const found = all.find((m) => m.id === id);
      if (!found) {
        setError('未找到该模型配置');
      } else {
        modelRef.current = found;
        setModel(found);
        await pollStatus();
      }
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  async function pollStatus() {
    const current = modelRef.current;
    if (!current) return;
    try {
      setStatus(await invoke<string | null>('check_server_status', { port: current.port }));
    } catch {
      // 静默忽略
    }
  }

  // ===== 启动/停止 =====
  async function startModel() {
    if (!model) return;
    setActionLoading(true);
    try {
      await invoke('start_llama_server', {
        serverPath: model.server_path,
        modelPath: model.model_path,
        port: model.port,
        ngl: model.ngl,
        modelName: model.name,
      });
      await pollStatus();
    } catch (e) {
      setError(`启动失败: ${e}`);
    } finally {
      setActionLoading(false);
    }
  }

  async function stopModel() {
    if (!model) return;
    setActionLoading(true);
    try {
      await invoke('stop_llama_server', { port: model.port });
      await pollStatus();
    } catch (e) {
      setError(`停止失败: ${e}`);
    } finally {
      setActionLoading(false);
    }
  }

  // ===== 复制到剪贴板 =====
  function copyText(text: string) {
    navigator.clipboard.writeText(text);
    setCopied(true);
    if (copyTimerRef.current !== null) {
      clearTimeout(copyTimerRef.current);
    }
    copyTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      copyTimerRef.current = null;
    }, 2000);
  }

  function copyCurrentCode() {
    const code =
      activeTab === 'curl' ? curlExample : activeTab === 'python' ? pythonExample : rustExample;
    copyText(code);
  }

  return (
    <div className="model-detail">
      {/* 页头 */}
      <div className="model-detail__page-header">
        <button className="model-detail__back" onClick={() => navigate('/models')}>
          <ArrowLeft size={18} strokeWidth={2.5} />
          返回
        </button>
      </div>

      {/* 加载 */}
      {loading && (
        <div className="model-detail__loading">
          <div className="spinner" />
          <span>正在加载...</span>
        </div>
      )}

      {/* 错误 */}
      {!loading && error && (
        <div className="model-detail__error">
          <span>⚠️</span>
          <span>{error}</span>
        </div>
      )}

      {!loading && model && (
        <div className="model-detail__content">
          {/* 模型信息卡片 */}
          <div className="model-detail__info-card">
            <div className="model-detail__info-header">
              <div className="model-detail__info-title">
                <span className="model-detail__icon">🧩</span>
                <div>
                  <h1 className="model-detail__name">{model.name}</h1>
                  <div className="model-detail__meta">
                    <span
                      className={`model-detail__status ${
                        isRunning
                          ? 'model-detail__status--running'
                          : 'model-detail__status--stopped'
                      }`}
                    >
                      {isRunning ? '运行中' : '已停止'}
                    </span>
                    <span className="model-detail__meta-tag">端口 {model.port}</span>
                    <span className="model-detail__meta-tag">ngl {model.ngl}</span>
                  </div>
                </div>
              </div>
              <div className="model-detail__info-actions">
                {isRunning ? (
                  <button
                    className="model-detail__btn-stop"
                    onClick={stopModel}
                    disabled={actionLoading}
                  >
                    <Square size={14} strokeWidth={2.5} />
                    {actionLoading ? '处理中...' : '停止服务'}
                  </button>
                ) : (
                  <button
                    className="model-detail__btn-start"
                    onClick={startModel}
                    disabled={actionLoading}
                  >
                    <Play size={14} strokeWidth={2.5} />
                    {actionLoading ? '处理中...' : '启动服务'}
                  </button>
                )}
              </div>
            </div>

            <div className="model-detail__info-grid">
              <div className="model-detail__info-item">
                <span className="model-detail__info-label">模型名称</span>
                <span className="model-detail__info-value">{model.name}</span>
              </div>
              <div className="model-detail__info-item">
                <span className="model-detail__info-label">端口</span>
                <span className="model-detail__info-value">{model.port}</span>
              </div>
              <div className="model-detail__info-item">
                <span className="model-detail__info-label">ngl (GPU 层数)</span>
                <span className="model-detail__info-value">{model.ngl}</span>
              </div>
              <div className="model-detail__info-item">
                <span className="model-detail__info-label">模型 ID</span>
                <span className="model-detail__info-value model-detail__info-value--mono">
                  {model.id}
                </span>
              </div>
              <div className="model-detail__info-item model-detail__info-item--full">
                <span className="model-detail__info-label">模型文件路径</span>
                <span
                  className="model-detail__info-value model-detail__info-value--mono model-detail__path-value"
                  title={model.model_path}
                >
                  {model.model_path}
                </span>
              </div>
              <div className="model-detail__info-item model-detail__info-item--full">
                <span className="model-detail__info-label">llama-server 路径</span>
                <span
                  className="model-detail__info-value model-detail__info-value--mono model-detail__path-value"
                  title={model.server_path}
                >
                  {model.server_path}
                </span>
              </div>
            </div>
          </div>

          {/* 启动命令 */}
          <div className="model-detail__section-card">
            <div className="model-detail__section-title">
              <h2>启动命令</h2>
              <button className="model-detail__btn-copy" onClick={() => copyText(startCommand)}>
                {copied ? '✓ 已复制' : '复制'}
              </button>
            </div>
            <div className="model-detail__code-block">
              <pre>
                <code>{startCommand}</code>
              </pre>
            </div>
          </div>

          {/* API 使用文档 */}
          <div className="model-detail__section-card">
            <div className="model-detail__section-title">
              <h2>API 使用文档</h2>
            </div>
            <div className="model-detail__api-note">
              <strong>💡 OpenAI 兼容:</strong>
              llama-server 提供了 OpenAI 兼容的 API，可以在{' '}
              <code>/v1/chat/completions</code> 端点使用标准的 OpenAI 格式进行对话。
            </div>

            {/* 选项卡 */}
            <div className="model-detail__tabs">
              <button
                className={`model-detail__tab ${activeTab === 'curl' ? 'model-detail__tab--active' : ''}`}
                onClick={() => setActiveTab('curl')}
              >
                curl
              </button>
              <button
                className={`model-detail__tab ${activeTab === 'python' ? 'model-detail__tab--active' : ''}`}
                onClick={() => setActiveTab('python')}
              >
                Python
              </button>
              <button
                className={`model-detail__tab ${activeTab === 'rust' ? 'model-detail__tab--active' : ''}`}
                onClick={() => setActiveTab('rust')}
              >
                Rust
              </button>
              <button className="model-detail__btn-copy-inline" onClick={copyCurrentCode}>
                {copied ? '✓ 已复制' : '📋 复制'}
              </button>
            </div>

            {/* curl */}
            {activeTab === 'curl' && (
              <div className="model-detail__code-block">
                <pre>
                  <code>{curlExample}</code>
                </pre>
              </div>
            )}

            {/* Python */}
            {activeTab === 'python' && (
              <div className="model-detail__code-block">
                <pre>
                  <code>{pythonExample}</code>
                </pre>
              </div>
            )}

            {/* Rust */}
            {activeTab === 'rust' && (
              <div className="model-detail__code-block">
                <pre>
                  <code>{rustExample}</code>
                </pre>
              </div>
            )}

            {/* API 端点信息 */}
            <div className="model-detail__endpoint-info">
              <div className="model-detail__endpoint-item">
                <span className="model-detail__endpoint-method">POST</span>
                <span className="model-detail__endpoint-path">/v1/chat/completions</span>
                <span className="model-detail__endpoint-desc">OpenAI 兼容聊天接口</span>
              </div>
              <div className="model-detail__endpoint-item">
                <span className="model-detail__endpoint-method">POST</span>
                <span className="model-detail__endpoint-path">/completion</span>
                <span className="model-detail__endpoint-desc">llama.cpp 原生补全接口</span>
              </div>
            </div>
          </div>

          {/* llama-server Web UI */}
          <div className="model-detail__section-card">
            <div className="model-detail__section-title">
              <h2>在线对话</h2>
              {isRunning && (
                <a href={webuiUrl} target="_blank" rel="noopener" className="model-detail__btn-copy">
                  🔗 新窗口打开
                </a>
              )}
            </div>
            {isRunning ? (
              <iframe
                src={webuiUrl}
                title="llama-server Web UI"
                className="model-detail__webui-iframe"
                sandbox="allow-scripts allow-same-origin allow-forms"
              />
            ) : (
              <div className="model-detail__webui-placeholder">
                <span className="model-detail__placeholder-icon">💤</span>
                <p>模型未运行，请先启动服务以加载在线对话界面</p>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
