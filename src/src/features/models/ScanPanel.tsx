import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronDown, Search } from 'lucide-react';
import type { ModelConfig, ModelFileInfo } from './types';
import './ScanPanel.scss';

interface ScanPanelProps {
  /** 当前已配置模型（用于端口分配与已添加判断） */
  models: ModelConfig[];
  /** 添加成功后由页面刷新列表 */
  onAdded: () => Promise<void> | void;
  /** 添加失败时上报页面错误横幅 */
  onError: (msg: string) => void;
}

export default function ScanPanel({ models, onAdded, onError }: ScanPanelProps) {
  const [showScanSection, setShowScanSection] = useState(false);
  const [scanDir, setScanDir] = useState('');
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState('');
  const [scannedModels, setScannedModels] = useState<ModelFileInfo[]>([]);
  const [addingIds, setAddingIds] = useState<Set<string>>(new Set());

  async function scanDirectory() {
    if (!scanDir.trim()) {
      setScanError('请输入目录路径');
      return;
    }
    setScanning(true);
    setScanError('');
    setScannedModels([]);
    try {
      const result = await invoke<ModelFileInfo[]>('scan_model_directory', {
        dir: scanDir.trim(),
      });
      setScannedModels(result);
      if (result.length === 0) {
        setScanError('该目录下未找到 .gguf 模型文件');
      }
    } catch (e) {
      setScanError(`扫描失败: ${e}`);
    } finally {
      setScanning(false);
    }
  }

  async function addScannedModel(file: ModelFileInfo) {
    const name = file.filename.replace(/\.gguf$/i, '');
    const port = 8080 + models.length;
    setAddingIds((prev) => new Set(prev).add(file.path));
    try {
      const newConfig: ModelConfig = {
        id: `model-${name.toLowerCase().replace(/\s+/g, '-').trim()}-${port}`,
        name,
        model_path: file.path,
        server_path: 'llama',
        port,
        ngl: 999,
      };
      await invoke('save_model_config', { model: newConfig });
      await onAdded();
      setScannedModels((prev) => prev.filter((m) => m.path !== file.path));
    } catch (e) {
      onError(`添加失败: ${e}`);
    } finally {
      setAddingIds((prev) => {
        const next = new Set(prev);
        next.delete(file.path);
        return next;
      });
    }
  }

  async function addAllScannedModels() {
    for (const file of scannedModels) {
      await addScannedModel(file);
    }
  }

  function isAlreadyAdded(file: ModelFileInfo): boolean {
    return models.some((m) => m.model_path === file.path);
  }

  return (
    <div className="scan-panel">
      <button
        className="scan-panel__toggle"
        onClick={() => setShowScanSection(!showScanSection)}
        aria-expanded={showScanSection}
      >
        <Search size={16} strokeWidth={1.8} />
        扫描模型目录
        <ChevronDown
          size={14}
          strokeWidth={1.8}
          className={`scan-panel__chevron ${showScanSection ? 'scan-panel__chevron--open' : ''}`}
        />
      </button>

      {showScanSection && (
        <div className="scan-panel__body">
          <div className="scan-panel__input-row">
            <input
              type="text"
              value={scanDir}
              onChange={(e) => setScanDir(e.target.value)}
              placeholder="输入模型目录路径，如 ~/models"
              disabled={scanning}
            />
            <button className="scan-panel__btn-scan" onClick={scanDirectory} disabled={scanning}>
              {scanning ? (
                <>
                  <span className="mini-spinner" />
                  扫描中...
                </>
              ) : (
                '扫描'
              )}
            </button>
          </div>
          {scanError && <div className="scan-panel__error">{scanError}</div>}
          {scannedModels.length > 0 && (
            <div className="scan-panel__results">
              <div className="scan-panel__results-header">
                <span>发现 {scannedModels.length} 个模型文件</span>
                <button className="scan-panel__btn-add-all" onClick={addAllScannedModels}>
                  全部添加
                </button>
              </div>
              <div className="scan-panel__list">
                {scannedModels.map((file) => {
                  const alreadyAdded = isAlreadyAdded(file);
                  return (
                    <div
                      key={file.path}
                      className={`scan-panel__item ${
                        alreadyAdded ? 'scan-panel__item--added' : ''
                      }`}
                    >
                      <span className="scan-panel__item-icon">🧩</span>
                      <div className="scan-panel__item-info">
                        <span className="scan-panel__item-name">{file.filename}</span>
                        <span className="scan-panel__item-size">{file.size_display}</span>
                      </div>
                      {alreadyAdded ? (
                        <span className="scan-panel__item-added">已添加</span>
                      ) : (
                        <button
                          className="scan-panel__btn-add-item"
                          onClick={() => addScannedModel(file)}
                          disabled={addingIds.has(file.path)}
                        >
                          {addingIds.has(file.path) ? '添加中...' : '+ 添加'}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
