/** SDK v2 event payload; metadata is arbitrary JSON at the protocol boundary. */
export interface SdkArtifact {
  id: string;
  kind: string;
  location: string;
  metadata?: unknown;
  preview?: string | null;
}

export interface LegacyArtifact {
  path: string;
  kind: string;
}

export type Artifact = SdkArtifact | LegacyArtifact;

const kindLabels: Record<string, string> = {
  file_change: '文件变更',
  file_created: '新建文件',
  file_modified: '修改文件',
  document: '文档',
  pdf: 'PDF 文档',
  image: '图片',
  command_output: '命令输出',
  data: '数据',
  generic: '产物',
};
const modeLabels: Record<string, string> = {
  create: '新建', overwrite: '覆盖', edit: '编辑', copy: '复制', move: '移动',
};

function labelFor(labels: Record<string, string>, key: string): string | undefined {
  return Object.hasOwn(labels, key) ? labels[key] : undefined;
}

function count(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
    ? value : undefined;
}

export function describeArtifact(artifact: Artifact) {
  const sdk = 'location' in artifact;
  const rawMetadata = sdk ? artifact.metadata : undefined;
  const metadata = rawMetadata && typeof rawMetadata === 'object' && !Array.isArray(rawMetadata)
    ? rawMetadata as Record<string, unknown> : {};
  const fileChange = ['file_change', 'file_created', 'file_modified'].includes(artifact.kind);
  const statistics: string[] = [];
  if (fileChange) {
    const insertions = count(metadata.insertions);
    const deletions = count(metadata.deletions);
    const bytes = count(metadata.bytes);
    if (insertions !== undefined) statistics.push(`新增 ${insertions} 行`);
    if (deletions !== undefined) statistics.push(`删除 ${deletions} 行`);
    if (bytes !== undefined) statistics.push(`${bytes} 字节`);
  }
  if (artifact.kind === 'image') {
    const width = count(metadata.width);
    const height = count(metadata.height);
    const bytes = count(metadata.encoded_bytes);
    if (metadata.normalized_mime === 'image/png') statistics.push('PNG（模型输入）');
    if (width && height) statistics.push(`${width} × ${height} px`);
    if (bytes !== undefined) statistics.push(`${bytes} 编码字节`);
  }
  return {
    location: sdk ? artifact.location : artifact.path,
    label: labelFor(kindLabels, artifact.kind) ?? '未知类型',
    mode: fileChange && typeof metadata.mode === 'string'
      ? labelFor(modeLabels, metadata.mode) : undefined,
    from: fileChange && typeof metadata.from === 'string' ? metadata.from : undefined,
    statistics,
    preview: sdk && typeof artifact.preview === 'string' ? artifact.preview : undefined,
  };
}
