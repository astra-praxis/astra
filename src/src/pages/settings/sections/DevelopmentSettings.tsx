/**
 * 默认编辑器：预设网格 + 自定义命令 + 测试启动（测试是独立操作，不是保存）
 */
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Play } from 'lucide-react';
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import EditorIcon from '../components/EditorIcon';
import { useSettingsStore } from '../hooks/useSettings';

type TestResult = 'idle' | 'success' | 'error';

export default function DevelopmentSettings() {
  const draft = useSettingsStore((s) => s.draft);
  const editors = useSettingsStore((s) => s.editors);
  const update = useSettingsStore((s) => s.update);

  const [testResult, setTestResult] = useState<TestResult>('idle');
  const [testMessage, setTestMessage] = useState('');

  if (!draft) return null;

  const currentEditorName = draft.useCustomEditor
    ? draft.editorCommand.trim() || '未设置'
    : editors.find((e) => e.command === draft.editorCommand)?.name || draft.editorName || '未设置';

  function selectPreset(name: string, command: string) {
    update({ editorName: name, editorCommand: command, useCustomEditor: false });
    setTestResult('idle');
  }

  function enableCustom() {
    // 不清空已有命令，避免用户切换回来后需要重填
    update({ useCustomEditor: true });
    setTestResult('idle');
  }

  async function testEditor() {
    setTestResult('idle');
    setTestMessage('');
    const command = draft?.editorCommand.trim();
    if (!command) return;
    try {
      // 打开临时目录测试（当前用户目录）
      await invoke('open_in_editor', { path: '~', editorCommand: command });
      setTestResult('success');
      setTestMessage(`${command} 启动成功！`);
      setTimeout(() => setTestResult('idle'), 3000);
    } catch (e) {
      setTestResult('error');
      setTestMessage(`${e}`);
    }
  }

  return (
    <SettingsSection
      icon="⚡"
      title="默认编辑器"
      desc="点击工作空间的项目卡片时，使用此编辑器打开项目目录"
    >
      <div className="current-selection">
        <span className="selection-label">当前选择：</span>
        <span className="selection-value">{currentEditorName}</span>
      </div>

      <div className="editor-grid">
        {editors.map((editor) => (
          <button
            key={editor.command}
            className={`editor-option${
              !draft.useCustomEditor && draft.editorCommand === editor.command ? ' selected' : ''
            }`}
            onClick={() => selectPreset(editor.name, editor.command)}
          >
            <span className="editor-icon">
              <EditorIcon editor={editor} />
            </span>
            <span className="editor-name">{editor.name}</span>
            <span className="editor-command">({editor.command})</span>
            {!draft.useCustomEditor && draft.editorCommand === editor.command && (
              <span className="check-mark">✓</span>
            )}
          </button>
        ))}

        <button
          className={`editor-option custom-option${draft.useCustomEditor ? ' selected' : ''}`}
          onClick={enableCustom}
        >
          <span className="editor-icon">
            <svg
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="3" />
              <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
            </svg>
          </span>
          <span className="editor-name">自定义</span>
          <span className="editor-command">(输入命令)</span>
          {draft.useCustomEditor && <span className="check-mark">✓</span>}
        </button>
      </div>

      {draft.useCustomEditor && (
        <div className="custom-input-group">
          <label htmlFor="custom-command">命令行工具名称或路径</label>
          <input
            id="custom-command"
            type="text"
            placeholder="如：code、zed、/usr/local/bin/code ..."
            value={draft.editorCommand}
            onChange={(e) => update({ editorCommand: e.target.value })}
          />
          <p className="input-hint">输入编辑器在终端中的启动命令，或者完整路径</p>
        </div>
      )}

      <SettingsRow
        title="测试启动"
        desc="用当前编辑器命令打开用户目录验证配置，不会保存设置。"
      >
        <div className="test-bar">
          <button
            className="btn-test"
            onClick={() => void testEditor()}
            disabled={!draft.editorCommand.trim()}
          >
            <Play size={16} strokeWidth={2} />
            测试启动编辑器
          </button>
          {testResult !== 'idle' && (
            <span
              className={`test-hint${testResult === 'success' ? ' test-success' : ''}${
                testResult === 'error' ? ' test-error' : ''
              }`}
            >
              {testResult === 'success' ? '✅' : '❌'}
              {testMessage}
            </span>
          )}
        </div>
      </SettingsRow>
    </SettingsSection>
  );
}
