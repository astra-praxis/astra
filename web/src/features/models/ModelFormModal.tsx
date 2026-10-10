import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import Modal from '@/components/ui/Modal';
import type { ModelConfig, PathCheckResult } from './types';
import './ModelFormModal.scss';

interface ModelFormModalProps {
  /** null = 添加模式；传入模型 = 编辑模式 */
  model?: ModelConfig | null;
  onClose: () => void;
  /** 保存成功后由页面刷新列表 */
  onSaved: () => void;
}

export default function ModelFormModal({ model = null, onClose, onSaved }: ModelFormModalProps) {
  // 仅捕获初值：弹窗每次打开都全新挂载
  const isEdit = model !== null;
  const initial = model ?? {
    name: '',
    server_path: '',
    model_path: '',
    port: 8080,
    ngl: 999,
  };

  const [name, setName] = useState(initial.name);
  const [serverPath, setServerPath] = useState(initial.server_path);
  const [modelPath, setModelPath] = useState(initial.model_path);
  const [port, setPort] = useState(String(initial.port));
  const [ngl, setNgl] = useState(String(initial.ngl));
  const [formError, setFormError] = useState('');
  const [formSaving, setFormSaving] = useState(false);
  const [pathChecking, setPathChecking] = useState(false);
  const [pathResult, setPathResult] = useState<PathCheckResult | null>(null);

  async function save() {
    setFormError('');
    if (!name.trim()) {
      setFormError('请输入模型名称');
      return;
    }
    if (!modelPath.trim()) {
      setFormError('请输入模型文件路径');
      return;
    }
    const portNum = Number(port);
    if (!port || portNum < 1 || portNum > 65535) {
      setFormError('端口号范围 1-65535');
      return;
    }
    const nglNum = Number(ngl);
    if (ngl.trim() === '' || nglNum < 0 || !Number.isInteger(nglNum)) {
      setFormError('ngl 必须为非负整数');
      return;
    }

    setFormSaving(true);
    try {
      const config: ModelConfig = {
        // 编辑保持原 ID；添加按名称+端口生成
        id: model
          ? model.id
          : `model-${name.trim().toLowerCase().replace(/\s+/g, '-').trim()}-${portNum}`,
        name: name.trim(),
        model_path: modelPath.trim(),
        server_path: serverPath.trim() || 'llama', // 空则默认使用 'llama'
        port: portNum,
        ngl: nglNum,
      };
      await invoke('save_model_config', { model: config });
      onClose();
      onSaved();
    } catch (e) {
      setFormError(`保存失败: ${e}`);
    } finally {
      setFormSaving(false);
    }
  }

  async function checkPaths() {
    setPathChecking(true);
    setPathResult(null);
    try {
      const [modelValid, modelError, serverValid, serverError] = await invoke<
        [boolean, string, boolean, string]
      >('check_model_paths', { modelPath, serverPath });
      setPathResult({ modelValid, modelError, serverValid, serverError });
    } catch (e) {
      setPathResult({
        modelValid: false,
        modelError: `校验失败: ${e}`,
        serverValid: false,
        serverError: '',
      });
    } finally {
      setPathChecking(false);
    }
  }

  return (
    <Modal
      open
      title={isEdit ? '编辑模型' : '添加模型'}
      onClose={onClose}
      closeOnOverlay={false}
      footer={
        <button className="btn-save" onClick={save} disabled={formSaving}>
          {formSaving ? '保存中...' : '保存'}
        </button>
      }
    >
      {formError && <div className="form-error">{formError}</div>}
      <div className="form-group">
        <label>模型名称</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="如: qwen2.5-7b"
        />
      </div>
      <div className="form-group">
        <label>
          llama 路径 <span className="optional-tag">(可选)</span>
        </label>
        <input
          type="text"
          value={serverPath}
          onChange={(e) => setServerPath(e.target.value)}
          placeholder="留空使用系统 PATH 中的 llama"
        />
        <p className="input-hint">
          若已配置环境变量，可留空直接使用 <code>llama</code> 命令
        </p>
      </div>
      <div className="form-group">
        <label>模型文件路径 (.gguf)</label>
        <input
          type="text"
          value={modelPath}
          onChange={(e) => setModelPath(e.target.value)}
          placeholder="/path/to/model.gguf"
        />
      </div>
      <div className="form-row">
        <div className="form-group">
          <label>端口</label>
          <input
            type="number"
            value={port}
            onChange={(e) => setPort(e.target.value)}
            min={1}
            max={65535}
          />
        </div>
        <div className="form-group">
          <label>ngl (GPU 层数)</label>
          <input type="number" value={ngl} onChange={(e) => setNgl(e.target.value)} min={0} />
        </div>
      </div>

      {/* 路径校验区域 */}
      <div className="path-check-section">
        <button className="btn-check" onClick={checkPaths} disabled={pathChecking}>
          {pathChecking ? (
            <>
              <span className="btn-spinner" />
              校验中...
            </>
          ) : (
            <>🔍 校验路径</>
          )}
        </button>
        {pathResult && (
          <div className="path-result">
            <div
              className={`path-result-item ${
                pathResult.modelValid ? 'path-ok' : 'path-fail'
              }`}
            >
              {pathResult.modelValid ? '✅' : '❌'} 模型文件：
              {pathResult.modelValid ? '有效' : pathResult.modelError}
            </div>
            <div
              className={`path-result-item ${
                pathResult.serverValid ? 'path-ok' : 'path-fail'
              }`}
            >
              {pathResult.serverValid ? '✅' : '❌'} llama 路径：
              {pathResult.serverValid ? '有效' : pathResult.serverError}
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
