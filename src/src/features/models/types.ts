/** 本地模型（llama.cpp）相关类型，与 Svelte 版 src/routes/models/types.ts 一致 */

export interface ModelConfig {
  id: string;
  name: string;
  model_path: string;
  server_path: string;
  port: number;
  ngl: number;
}

export interface RunningModelInfo extends ModelConfig {
  status: string;
  pid: number | null;
  started_at: number;
  /** true = 由 astra 之外的进程启动（如终端/脚本），应用内无法停止 */
  external?: boolean;
  /** llama.cpp /props 报告的视觉能力（加载了 mmproj 时为 true） */
  vision?: boolean;
}

export interface ModelFileInfo {
  path: string;
  filename: string;
  size_bytes: number;
  size_display: string;
}

export interface PathCheckResult {
  modelValid: boolean;
  modelError: string;
  serverValid: boolean;
  serverError: string;
}
