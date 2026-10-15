/**
 * 设置页面
 * 移植自 src/routes/settings/+page.svelte
 */
import { useEffect, useState } from 'react';
import { Link } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Check, Play, Save, Folder, X, Zap } from 'lucide-react';
import { useThemeStore, type ThemeMode } from '@/lib/theme';
import { useAgentStore } from '@/stores/agent';
import './SettingsPage.scss';

/** 生成供应商配置（与后端 agent_generation.rs GenerationConfig 对齐） */
interface GenerationConfigState {
  enabled: boolean;
  base_url: string;
  api_key: string;
  model_id: string;
}

interface EditorSetting {
  name: string;
  command: string;
}

interface AppSettings {
  editor: EditorSetting;
  workspaces: WorkspaceConfig[];
  active_workspace: string | null;
  scan_depth: number;
  search_endpoint?: string | null;
}

interface WorkspaceConfig {
  name: string;
  path: string;
}

type TestResult = 'idle' | 'success' | 'error';

const themeOptions: { value: ThemeMode; label: string; icon: string }[] = [
  { value: 'light', label: '浅色', icon: '☀️' },
  { value: 'dark', label: '深色', icon: '🌙' },
  { value: 'system', label: '跟随系统', icon: '🖥️' },
];

export default function SettingsPage() {
  const currentTheme = useThemeStore((s) => s.mode);
  const setThemeMode = useThemeStore((s) => s.setMode);

  const [editors, setEditors] = useState<EditorSetting[]>([]);
  const [selectedEditor, setSelectedEditor] = useState<EditorSetting | null>(null);
  const [customCommand, setCustomCommand] = useState('');
  const [useCustom, setUseCustom] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [testResult, setTestResult] = useState<TestResult>('idle');
  const [testMessage, setTestMessage] = useState('');

  // 工作空间状态
  const [workspaces, setWorkspaces] = useState<WorkspaceConfig[]>([]);
  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);

  // 扫描深度状态
  const [scanDepth, setScanDepth] = useState(3);

  // Agent 权限：放权模式（全局偏好，进 Agent 页后同步到后端）
  const autoApprove = useAgentStore((s) => s.autoApprove);
  const setAutoApprove = useAgentStore((s) => s.setAutoApprove);
  const maxTurns = useAgentStore((s) => s.maxTurns);
  const setMaxTurns = useAgentStore((s) => s.setMaxTurns);
  const autoCompactTokens = useAgentStore((s) => s.autoCompactTokens);
  const setAutoCompactTokens = useAgentStore((s) => s.setAutoCompactTokens);
  // 图片识别：read_file 图片上传所选模型的显式授权（仅新会话生效）
  const imageRecognition = useAgentStore((s) => s.imageRecognition);
  const setImageRecognition = useAgentStore((s) => s.setImageRecognition);
  // Agent search_web 的 SearXNG 实例地址（可选；空 = 用 DuckDuckGo）
  const [searchEndpoint, setSearchEndpoint] = useState('');
  // 图片生成（generate_image 工具）：OpenAI Images 兼容网关配置
  //（保存后新会话生效；api_key 仅存本机 generation.json）
  const [genConfig, setGenConfig] = useState<GenerationConfigState>({
    enabled: false,
    base_url: 'https://api.openai.com/v1',
    api_key: '',
    model_id: 'gpt-image-1',
  });
  const [genSaved, setGenSaved] = useState(false);

  // ===== 加载设置 =====
  useEffect(() => {
    let disposed = false;

    (async () => {
      try {
        const [settings, presetEditors, gen] = await Promise.all([
          invoke<AppSettings>('get_settings'),
          invoke<EditorSetting[]>('get_preset_editors'),
          invoke<GenerationConfigState>('generation_get_config'),
        ]);
        if (disposed) return;

        setEditors(presetEditors);
        setWorkspaces(settings.workspaces);
        setActiveWorkspace(settings.active_workspace);
        setScanDepth(settings.scan_depth || 3);
        setSearchEndpoint(settings.search_endpoint ?? '');
        setGenConfig((prev) => ({ ...prev, ...gen }));

        // 检查当前设置是否在预设中
        const match = presetEditors.find((e) => e.command === settings.editor.command);
        if (match) {
          setSelectedEditor(match);
          setUseCustom(false);
        } else {
          // 自定义编辑器
          setUseCustom(true);
          setCustomCommand(settings.editor.command);
          setSelectedEditor(settings.editor);
        }
      } catch (e) {
        console.error('加载设置失败:', e);
      }
    })();

    return () => {
      disposed = true;
    };
  }, []);

  // ===== 选择预设编辑器 =====
  function selectEditor(editor: EditorSetting) {
    setSelectedEditor(editor);
    setUseCustom(false);
    setSaved(false);
    setTestResult('idle');
  }

  // ===== 切换自定义 =====
  function enableCustom() {
    setUseCustom(true);
    setCustomCommand('');
    setSelectedEditor({ name: '自定义', command: '' });
    setSaved(false);
    setTestResult('idle');
  }

  // ===== 保存设置 =====
  async function saveSettings() {
    setSaving(true);
    setSaved(false);

    try {
      const editor = useCustom
        ? { name: '自定义', command: customCommand.trim() }
        : selectedEditor;

      if (!editor || !editor.command.trim()) {
        return;
      }

      await invoke('save_settings', {
        settings: {
          editor,
          workspaces,
          active_workspace: activeWorkspace,
          scan_depth: scanDepth,
          search_endpoint: searchEndpoint.trim() || null,
        },
      });

      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      console.error('保存设置失败:', e);
    } finally {
      setSaving(false);
    }
  }

  // ===== 测试编辑器 =====
  async function testEditor() {
    setTestResult('idle');
    setTestMessage('');

    const command = useCustom ? customCommand.trim() : selectedEditor?.command || '';
    if (!command) return;

    try {
      // 打开临时目录测试（当前用户目录）
      await invoke('open_in_editor', {
        path: '~',
        editorCommand: command,
      });
      setTestResult('success');
      setTestMessage(`${command} 启动成功！`);
      setTimeout(() => setTestResult('idle'), 3000);
    } catch (e) {
      setTestResult('error');
      setTestMessage(`${e}`);
    }
  }

  // ===== 表单是否有效 =====
  const isValid = useCustom ? customCommand.trim().length > 0 : selectedEditor !== null;
  const currentEditorName = useCustom
    ? customCommand.trim() || '未设置'
    : selectedEditor?.name || '未设置';

  // ===== 选择工作空间目录 =====
  async function addWorkspace() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择工作空间目录',
      });

      if (selected) {
        // 提取文件夹名作为默认名称
        const name = selected.split('/').filter(Boolean).pop() || selected;
        const result = await invoke<AppSettings>('add_workspace', { name, path: selected });
        setWorkspaces(result.workspaces);
        setActiveWorkspace(result.active_workspace ?? null);
        setSaved(false);
      }
    } catch (e) {
      console.error('添加工作空间失败:', e);
    }
  }

  // ===== 删除工作空间 =====
  async function removeWorkspace(path: string) {
    try {
      const result = await invoke<AppSettings>('remove_workspace', { path });
      setWorkspaces(result.workspaces);
      setActiveWorkspace(result.active_workspace ?? null);
      setSaved(false);
    } catch (e) {
      console.error('删除工作空间失败:', e);
    }
  }

  // ===== 设为活跃 =====
  async function setActiveWs(path: string) {
    try {
      const result = await invoke<AppSettings>('set_active_workspace', { path });
      setActiveWorkspace(result.active_workspace ?? null);
      setSaved(false);
    } catch (e) {
      console.error('设置活跃工作空间失败:', e);
    }
  }

  // ===== 编辑器图标 =====
  function renderEditorIcon(editor: EditorSetting) {
    if (editor.name === 'VS Code') {
      return (
        <svg width="24" height="24" viewBox="0 0 24 24">
          <path
            fill="#007ACC"
            d="M23.15 2.587L18.21.21a1.49 1.49 0 0 0-1.705.29l-9.46 8.63-4.12-3.128a.999.999 0 0 0-1.144.06L.805 7.347a1 1 0 0 0 .002 1.518L4.368 12 .805 15.136a1 1 0 0 0-.002 1.518l1.004.785a.999.999 0 0 0 1.144.06l4.12-3.128 9.46 8.63a1.49 1.49 0 0 0 1.704.29l4.942-2.377A1.5 1.5 0 0 0 24 19.414V4.586a1.5 1.5 0 0 0-.85-1.999z"
          />
        </svg>
      );
    }
    if (editor.name === 'Zed') {
      return (
        <svg width="24" height="24" viewBox="0 0 24 24">
          <rect width="24" height="24" rx="4" fill="#084B33" />
          <text
            x="12"
            y="17"
            textAnchor="middle"
            fill="white"
            fontSize="14"
            fontWeight="bold"
            fontFamily="sans-serif"
          >
            Z
          </text>
        </svg>
      );
    }
    if (editor.name === 'Cursor') return <span className="emoji-icon">🖱️</span>;
    if (editor.name === 'WebStorm') {
      return (
        <svg width="24" height="24" viewBox="0 0 24 24">
          <circle cx="12" cy="12" r="12" fill="#07C3F2" />
          <text
            x="12"
            y="17"
            textAnchor="middle"
            fill="white"
            fontSize="12"
            fontWeight="bold"
            fontFamily="sans-serif"
          >
            WS
          </text>
        </svg>
      );
    }
    if (editor.name === 'IntelliJ IDEA') {
      return (
        <svg width="24" height="24" viewBox="0 0 24 24">
          <rect width="24" height="24" rx="4" fill="#FC3752" />
          <text
            x="12"
            y="17"
            textAnchor="middle"
            fill="white"
            fontSize="10"
            fontWeight="bold"
            fontFamily="sans-serif"
          >
            IJ
          </text>
        </svg>
      );
    }
    if (editor.name === 'Sublime Text') return <span className="emoji-icon">📝</span>;
    if (editor.name === 'Atom') return <span className="emoji-icon">⚛️</span>;
    if (editor.name === 'Windsurf') return <span className="emoji-icon">🏄</span>;
    return <span className="emoji-icon">🔧</span>;
  }

  return (
    <div className="settings-page">
      <div className="page-header">
        <div className="header-left">
          <h1>设置</h1>
          <p className="subtitle">配置你的开发工具偏好</p>
        </div>
      </div>

      {/* 导航入口卡片 */}
      <div className="settings-card nav-card">
        <div className="card-section-header">
          <div className="section-icon">🧭</div>
          <div className="section-text">
            <h3>高级管理</h3>
            <p>以下功能已从侧边栏移入设置，点击进入对应页面</p>
          </div>
        </div>

        <div className="nav-grid">
          <Link to="/models" className="nav-entry">
            <span className="nav-entry-icon">🤖</span>
            <span className="nav-entry-body">
              <span className="nav-entry-title">模型管理</span>
              <span className="nav-entry-desc">管理本地与服务端模型</span>
            </span>
            <span className="nav-entry-arrow">›</span>
          </Link>
          <Link to="/providers" className="nav-entry">
            <span className="nav-entry-icon">🔌</span>
            <span className="nav-entry-body">
              <span className="nav-entry-title">API 接口</span>
              <span className="nav-entry-desc">配置 AI 服务提供商与密钥</span>
            </span>
            <span className="nav-entry-arrow">›</span>
          </Link>
          <Link to="/skills" className="nav-entry">
            <span className="nav-entry-icon">🧩</span>
            <span className="nav-entry-body">
              <span className="nav-entry-title">Skills 管理</span>
              <span className="nav-entry-desc">管理可用的技能模块</span>
            </span>
            <span className="nav-entry-arrow">›</span>
          </Link>
          <Link to="/my-skills" className="nav-entry">
            <span className="nav-entry-icon">⚡</span>
            <span className="nav-entry-body">
              <span className="nav-entry-title">我的 Skills</span>
              <span className="nav-entry-desc">新建或导入技能，Agent 中 / 调用</span>
            </span>
            <span className="nav-entry-arrow">›</span>
          </Link>
        </div>
      </div>

      {/* 主题设置卡片 */}
      <div className="settings-card theme-card">
        <div className="card-section-header">
          <div className="section-icon">🎨</div>
          <div className="section-text">
            <h3>外观主题</h3>
            <p>选择浅色、深色或跟随系统主题</p>
          </div>
        </div>

        <div className="theme-grid">
          {themeOptions.map((opt) => (
            <button
              key={opt.value}
              className={`theme-option${currentTheme === opt.value ? ' selected' : ''}`}
              onClick={() => setThemeMode(opt.value)}
            >
              <span className="theme-icon">{opt.icon}</span>
              <span className="theme-name">{opt.label}</span>
              {currentTheme === opt.value && <span className="check-mark">✓</span>}
            </button>
          ))}
        </div>
      </div>

      {/* 扫描深度设置卡片 */}
      <div className="settings-card scan-depth-card">
        <div className="card-section-header">
          <div className="section-icon">🔍</div>
          <div className="section-text">
            <h3>扫描深度</h3>
            <p>点击项目卡片查看详情时，递归遍历子目录的最大层级数</p>
          </div>
        </div>

        <div className="custom-input-group">
          <label htmlFor="scan-depth">递归深度</label>
          <input
            id="scan-depth"
            type="number"
            min={1}
            max={5}
            value={scanDepth}
            onChange={(e) => setScanDepth(Number(e.target.value))}
          />
          <p className="input-hint">范围 1–5，数值越大遍历越深（默认 3）</p>
        </div>
      </div>

      {/* Agent 权限设置卡片 */}
      <div className="settings-card agent-perm-card">
        <div className="card-section-header">
          <div className="section-icon">
            <Zap size={20} strokeWidth={1.8} />
          </div>
          <div className="section-text">
            <h3>Agent 权限</h3>
            <p>控制 Agent 执行工具时的确认策略</p>
          </div>
        </div>

        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">放权模式（自动批准所有操作）</span>
            <span className="agent-perm-desc">
              开启后，Agent 的所有确认类操作（写文件、编辑、Shell 命令、网络请求、构建文档等）自动批准，不再弹窗等待点击。
              未注册工具、提权命令、项目外路径逃逸仍会被硬规则拦截。
            </span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={autoApprove}
            className={`agent-perm-switch${autoApprove ? ' is-on' : ''}`}
            title={autoApprove ? '已开启：所有操作自动批准，点击关闭' : '已关闭：点击开启'}
            onClick={() => setAutoApprove(!autoApprove)}
          >
            <span className="agent-perm-knob" />
          </button>
        </div>

        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">最大执行轮数</span>
            <span className="agent-perm-desc">
              Agent 单次任务内「模型请求 → 工具执行」的最大循环次数，防止无限工具循环。超出后任务报错结束，修改后新会话生效。
            </span>
          </div>
          <input
            type="number"
            min={1}
            max={200}
            value={maxTurns}
            className="agent-perm-input"
            onChange={(e) => {
              const n = Number(e.target.value);
              if (Number.isFinite(n) && n >= 1) setMaxTurns(n);
            }}
          />
        </div>

        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">上下文自动压缩</span>
            <span className="agent-perm-desc">
              长对话超过该估算 token 阈值时，Agent 自动把历史压缩成结构化摘要后继续任务（对齐 Zed 的 /compact），
              避免超出模型上下文上限。设为 0 关闭；修改后新会话生效。
            </span>
          </div>
          <input
            type="number"
            min={0}
            max={1000000}
            step={1000}
            value={autoCompactTokens}
            className="agent-perm-input"
            onChange={(e) => {
              const n = Number(e.target.value);
              if (Number.isFinite(n) && n >= 0) setAutoCompactTokens(n);
            }}
          />
        </div>
        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">图片识别</span>
            <span className="agent-perm-desc">
              开启表示你已确认所选模型及网关支持 vision / image_url，并同意将 read_file 读取的项目图片上传至所选模型。
              图片内容会写入本机 SQLite 会话历史（含 WAL、备份），删除源图片不会删除历史；不支持时会报错，不会静默降级。
              仅新会话生效；旧会话保持原设置，配置或模型来源不一致时拒绝续聊。支持 PNG、JPEG、WebP。
            </span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={imageRecognition}
            className={`agent-perm-switch${imageRecognition ? ' is-on' : ''}`}
            title={imageRecognition ? '已开启：新会话允许读取图片（需模型支持）' : '已关闭：点击开启'}
            onClick={() => setImageRecognition(!imageRecognition)}
          >
            <span className="agent-perm-knob" />
          </button>
        </div>
        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">搜索服务地址（可选）</span>
            <span className="agent-perm-desc">
              Agent 的 search_web 工具默认用 DuckDuckGo（部分网络不可达）。填写一个 SearXNG 实例地址（如 https://searx.example.org）可切换到该实例，保存后新会话生效。
            </span>
          </div>
          <input
            type="text"
            value={searchEndpoint}
            placeholder="https://searx.example.org"
            className="agent-perm-input agent-perm-input--wide"
            onChange={(e) => setSearchEndpoint(e.target.value)}
          />
        </div>
        <div className="agent-perm-row">
          <div className="agent-perm-text">
            <span className="agent-perm-title">图片生成（generate_image）</span>
            <span className="agent-perm-desc">
              开启后 Agent 获得 generate_image 工具，可提交图片生成任务（付费外部调用，每次提交都会弹出确认卡）。
              填写 OpenAI Images 兼容网关地址与模型（如 gpt-image-1）；API Key 仅保存在本机，不上传。保存后新会话生效。
            </span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={genConfig.enabled}
            className={`agent-perm-switch${genConfig.enabled ? ' is-on' : ''}`}
            title={genConfig.enabled ? '已开启：新会话注册 generate_image 工具' : '已关闭：点击开启'}
            onClick={() => setGenConfig((prev) => ({ ...prev, enabled: !prev.enabled }))}
          >
            <span className="agent-perm-knob" />
          </button>
        </div>
        {genConfig.enabled && (
          <div className="agent-perm-row">
            <div className="agent-perm-text">
              <span className="agent-perm-title">生成网关与模型</span>
              <span className="agent-perm-desc">
                地址以 /v1 结尾（如 https://api.openai.com/v1）；模型为供应商的图片生成型号。未经真实网关验证前请先用小图试跑。
              </span>
            </div>
            <div className="agent-gen-fields">
              <input
                type="text"
                value={genConfig.base_url}
                placeholder="https://api.openai.com/v1"
                className="agent-perm-input agent-perm-input--wide"
                aria-label="生成网关地址"
                onChange={(e) => setGenConfig((prev) => ({ ...prev, base_url: e.target.value }))}
              />
              <input
                type="text"
                value={genConfig.model_id}
                placeholder="gpt-image-1"
                className="agent-perm-input"
                aria-label="生成模型"
                onChange={(e) => setGenConfig((prev) => ({ ...prev, model_id: e.target.value }))}
              />
              <input
                type="password"
                value={genConfig.api_key}
                placeholder="API Key（仅存本机）"
                className="agent-perm-input"
                aria-label="API Key"
                autoComplete="off"
                onChange={(e) => setGenConfig((prev) => ({ ...prev, api_key: e.target.value }))}
              />
              <button
                type="button"
                className="agent-perm-save"
                onClick={async () => {
                  try {
                    await invoke('generation_set_config', { config: genConfig });
                    setGenSaved(true);
                    window.setTimeout(() => setGenSaved(false), 2000);
                  } catch (e) {
                    console.error('保存生成配置失败:', e);
                  }
                }}
              >
                {genSaved ? <Check size={14} /> : <Save size={14} />}
                {genSaved ? '已保存' : '保存生成配置'}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 编辑器设置卡片 */}
      <div className="settings-card">
        <div className="card-section-header">
          <div className="section-icon">⚡</div>
          <div className="section-text">
            <h3>默认编辑器</h3>
            <p>点击工作空间的项目卡片时，使用此编辑器打开项目目录</p>
          </div>
        </div>

        {/* 当前选择状态 */}
        <div className="current-selection">
          <span className="selection-label">当前选择：</span>
          <span className="selection-value">{currentEditorName}</span>
        </div>

        {/* 预设编辑器列表 */}
        <div className="editor-grid">
          {editors.map((editor) => (
            <button
              key={editor.command}
              className={`editor-option${
                !useCustom && selectedEditor?.command === editor.command ? ' selected' : ''
              }`}
              onClick={() => selectEditor(editor)}
            >
              <span className="editor-icon">{renderEditorIcon(editor)}</span>
              <span className="editor-name">{editor.name}</span>
              <span className="editor-command">({editor.command})</span>
              {!useCustom && selectedEditor?.command === editor.command && (
                <span className="check-mark">✓</span>
              )}
            </button>
          ))}

          {/* 自定义选项 */}
          <button
            className={`editor-option custom-option${useCustom ? ' selected' : ''}`}
            onClick={enableCustom}
          >
            <span className="editor-icon">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
            </span>
            <span className="editor-name">自定义</span>
            <span className="editor-command">(输入命令)</span>
            {useCustom && <span className="check-mark">✓</span>}
          </button>
        </div>

        {/* 自定义命令输入 */}
        {useCustom && (
          <div className="custom-input-group">
            <label htmlFor="custom-command">命令行工具名称或路径</label>
            <input
              id="custom-command"
              type="text"
              placeholder="如：code、zed、/usr/local/bin/code ..."
              value={customCommand}
              onChange={(e) => {
                setCustomCommand(e.target.value);
                setSaved(false);
              }}
            />
            <p className="input-hint">输入编辑器在终端中的启动命令，或者完整路径</p>
          </div>
        )}

        {/* 测试按钮（仅测试，不保存） */}
        <div className="test-bar">
          <button className="btn-test" onClick={testEditor} disabled={!isValid}>
            <Play size={16} strokeWidth={2} />
            测试启动编辑器
          </button>

          {testResult !== 'idle' && (
            <span
              className={`test-hint${
                testResult === 'success' ? ' test-success' : ''
              }${testResult === 'error' ? ' test-error' : ''}`}
            >
              {testResult === 'success' ? '✅' : '❌'}
              {testMessage}
            </span>
          )}
        </div>
      </div>

      {/* ===== 工作空间设置 ===== */}
      <div className="settings-card workspace-card">
        <div className="card-section-header">
          <div className="section-icon">📁</div>
          <div className="section-text">
            <h3>工作空间管理</h3>
            <p>添加多个工作空间目录，在主页可一键切换</p>
          </div>
        </div>

        {workspaces.length > 0 ? (
          <div className="ws-list">
            {workspaces.map((ws) => (
              <div
                key={ws.path}
                className={`ws-item${ws.path === activeWorkspace ? ' active' : ''}`}
              >
                <div className="ws-item-left">
                  <Folder size={16} strokeWidth={2} />
                  <div className="ws-item-info">
                    <span className="ws-item-name">{ws.name}</span>
                    <span className="ws-item-path">{ws.path}</span>
                  </div>
                </div>
                <div className="ws-item-actions">
                  {ws.path !== activeWorkspace ? (
                    <button
                      className="ws-btn-activate"
                      onClick={() => setActiveWs(ws.path)}
                      title="切换为此工作空间"
                    >
                      切换
                    </button>
                  ) : (
                    <span className="ws-badge-active">当前</span>
                  )}
                  <button
                    className="ws-btn-remove"
                    onClick={() => removeWorkspace(ws.path)}
                    title="移除"
                  >
                    <X size={14} strokeWidth={2.5} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="ws-empty">
            <span>还没有添加任何工作空间</span>
          </div>
        )}

        <button className="btn-select-folder" onClick={addWorkspace}>
          <Folder size={18} strokeWidth={2} />
          添加工作空间
        </button>
      </div>

      {/* ===== 全局保存栏 ===== */}
      <div className="save-bar">
        <div className="save-bar-info">
          <span className="save-dot" />
          <span>修改了编辑器和工作空间配置后，点击保存生效</span>
        </div>
        <button className="btn-save" onClick={saveSettings} disabled={saving}>
          {saved ? (
            <>
              <Check size={16} strokeWidth={3} />
              已保存
            </>
          ) : saving ? (
            '保存中...'
          ) : (
            <>
              <Save size={16} strokeWidth={2.5} />
              保存设置
            </>
          )}
        </button>
      </div>
    </div>
  );
}
