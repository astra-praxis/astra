import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('./artifact.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
});
const { describeArtifact } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

const sdk = (fields = {}) => ({ id: 'art_1', kind: 'file_change', location: 'src/a.ts', ...fields });

test('image metadata describes normalized input without inventing a thumbnail', () => {
  const result = describeArtifact(sdk({ kind: 'image', metadata: {
    normalized_mime: 'image/png', width: 100, height: 50, encoded_bytes: 300,
  } }));
  assert.deepEqual(result.statistics, ['PNG（模型输入）', '100 × 50 px', '300 编码字节']);
  assert.equal(result.preview, undefined);
  for (const metadata of [null, {}, { width: -1, height: 50 }, { width: '100', height: 50 }]) {
    assert.deepEqual(describeArtifact(sdk({ kind: 'image', metadata })).statistics, []);
  }
});

test('SDK edit statistics preserve zero and text preview verbatim', () => {
  const preview = '- <script>alert(1)</script>\n+ safe';
  const result = describeArtifact(sdk({ metadata: { mode: 'edit', insertions: 0, deletions: 2, bytes: 0 }, preview }));
  assert.equal(result.label, '文件变更');
  assert.equal(result.location, 'src/a.ts');
  assert.equal(result.mode, '编辑');
  assert.deepEqual(result.statistics, ['新增 0 行', '删除 2 行', '0 字节']);
  assert.equal(result.preview, preview);
});

test('legacy artifact paths and labels remain supported', () => {
  assert.equal(describeArtifact({ kind: 'file_created', path: '旧路径.txt' }).label, '新建文件');
  const result = describeArtifact({ kind: 'file_modified', path: '旧路径.txt' });
  assert.equal(result.location, '旧路径.txt');
  assert.equal(result.label, '修改文件');
  assert.deepEqual(result.statistics, []);
  assert.equal(result.preview, undefined);
});

test('missing, null and malformed metadata do not fabricate statistics', () => {
  for (const metadata of [undefined, null, [], 'text', 3, { insertions: -1, deletions: '2', bytes: NaN }, { insertions: 1.2, deletions: Infinity }]) {
    assert.deepEqual(describeArtifact(sdk({ metadata })).statistics, []);
  }
});

test('all SDK kinds have Chinese labels, with a safe unknown fallback', () => {
  for (const kind of ['file_change', 'document', 'pdf', 'image', 'command_output', 'data', 'generic']) {
    assert.notEqual(describeArtifact(sdk({ kind })).label, '未知类型');
  }
  for (const kind of ['future', '__proto__', 'constructor']) {
    assert.equal(describeArtifact(sdk({ kind })).label, '未知类型');
  }
});

test('copy and move expose origin; unrelated kinds do not show file statistics', () => {
  assert.equal(describeArtifact(sdk({ metadata: { mode: 'move', from: 'old.txt' } })).from, 'old.txt');
  assert.equal(describeArtifact(sdk({ metadata: { mode: 'copy' } })).mode, '复制');
  assert.deepEqual(describeArtifact(sdk({ kind: 'data', metadata: { insertions: 9 } })).statistics, []);
});
