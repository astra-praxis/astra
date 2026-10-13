import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import ts from 'typescript';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';

const require = createRequire(import.meta.url);
async function load(path, overrides = {}) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX },
  });
  const exports = {};
  vm.runInNewContext(outputText, { exports, setTimeout, clearTimeout, require: (id) => {
    if (id in overrides) return overrides[id];
    if (id.endsWith('.scss') || id.endsWith('.css')) return {};
    return require(id);
  } });
  return exports;
}
const latest = await load('../../hooks/latestValue.ts');
const hook = await load('../../hooks/useStreamingText.ts', { './latestValue': latest });
let mermaidProps = null;
const { default: MarkdownMessage } = await load('./MarkdownMessage.tsx', {
  'react-markdown': { default: ReactMarkdown },
  'remark-gfm': { default: remarkGfm },
  'rehype-highlight': { default: rehypeHighlight },
  '@/hooks/useStreamingText': hook,
  './action-links': { parseActionHref: () => null, executeAction: () => { throw Error('unexpected action'); } },
  './MermaidBlock': { default: (props) => { mermaidProps = props; return null; } },
  '@tauri-apps/api/core': { invoke: () => { throw Error('unexpected IPC'); } },
  '@tauri-apps/api/event': { listen: () => { throw Error('unexpected subscription in SSR'); } },
});
const render = (content, props = {}) => renderToStaticMarkup(React.createElement(MarkdownMessage, { content, ...props }));
test('default Chat rendering remains immediate, with GFM and highlighted shell controls', () => {
  const html = render('| a | b |\n| - | - |\n| 1 | 2 |\n\n```bash\necho hello\n```');
  assert.match(html, /<table>/);
  assert.match(html, /hljs/);
  assert.match(html, /code-copy-btn/);
  assert.match(html, /code-exec-btn/);
});
test('streaming first render and final render retain full content, including an unclosed fence', () => {
  const content = '# Long output\n\n```typescript\n' + 'const value = 123;\n'.repeat(1000);
  for (const streaming of [true, false]) {
    const html = render(content, { streaming });
    assert.match(html, /Long output/);
    assert.equal((html.match(/123/g) ?? []).length, 1000);
  }
});
test('unknown language, malformed markdown and raw HTML do not crash or inject executable HTML', () => {
  const html = render('```unknown-language\nx < y\n```\n\n[unfinished](\n\n<script>alert(1)</script>');
  assert.match(html, /x &lt; y/);
  assert.doesNotMatch(html, /<script>/);
});
test('mermaid code blocks route to MermaidBlock with raw source and streaming flag', () => {
  mermaidProps = null;
  render('```mermaid\ngraph TD; A-->B;\n```', { streaming: true });
  assert.ok(mermaidProps, 'mermaid block should render via MermaidBlock');
  assert.equal(mermaidProps.code, 'graph TD; A-->B;\n');
  assert.equal(mermaidProps.streaming, true);
});
