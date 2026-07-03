<script lang="ts">
  import { marked } from 'marked';
  import hljs from 'highlight.js/lib/core';
  import 'highlight.js/styles/github-dark.css';
  import javascript from 'highlight.js/lib/languages/javascript';
  import typescript from 'highlight.js/lib/languages/typescript';
  import python from 'highlight.js/lib/languages/python';
  import rust from 'highlight.js/lib/languages/rust';
  import css from 'highlight.js/lib/languages/css';
  import xml from 'highlight.js/lib/languages/xml';
  import bash from 'highlight.js/lib/languages/bash';
  import json from 'highlight.js/lib/languages/json';
  import sql from 'highlight.js/lib/languages/sql';
  import plaintext from 'highlight.js/lib/languages/plaintext';
  import { onMount, tick } from 'svelte';

  hljs.registerLanguage('javascript', javascript);
  hljs.registerLanguage('typescript', typescript);
  hljs.registerLanguage('python', python);
  hljs.registerLanguage('rust', rust);
  hljs.registerLanguage('css', css);
  hljs.registerLanguage('html', xml);
  hljs.registerLanguage('xml', xml);
  hljs.registerLanguage('bash', bash);
  hljs.registerLanguage('sh', bash);
  hljs.registerLanguage('json', json);
  hljs.registerLanguage('sql', sql);
  hljs.registerLanguage('text', plaintext);
  hljs.registerLanguage('plaintext', plaintext);

  // 配置 marked
  marked.setOptions({
    breaks: true,
    gfm: true,
  });

  let { content = '' } = $props();
  let containerEl: HTMLElement | null = null;
  let rendered = $state('');

  // 渲染 markdown
  $effect(() => {
    if (!content) {
      rendered = '';
      return;
    }
    rendered = marked.parse(content) as string;
  });

  // 渲染后处理：高亮 + 复制按钮
  $effect(() => {
    if (!rendered || !containerEl) return;
    // 等 DOM 更新
    tick().then(() => {
      if (!containerEl) return;
      // 高亮代码块
      containerEl.querySelectorAll('pre code').forEach((el) => {
        hljs.highlightElement(el as HTMLElement);
      });
      // 添加复制按钮
      containerEl.querySelectorAll('pre').forEach((pre) => {
        // 跳过已处理的
        if (pre.querySelector('.code-copy-btn')) return;
        const btn = document.createElement('button');
        btn.className = 'code-copy-btn';
        btn.innerHTML = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg> 复制';
        btn.addEventListener('click', async () => {
          const code = pre.querySelector('code')?.textContent ?? '';
          try {
            await navigator.clipboard.writeText(code);
            btn.textContent = '✓ 已复制';
            btn.classList.add('copied');
            setTimeout(() => {
              btn.innerHTML = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg> 复制';
              btn.classList.remove('copied');
            }, 2000);
          } catch {
            btn.textContent = '复制失败';
          }
        });
        pre.style.position = 'relative';
        pre.prepend(btn);
      });
    });
  });
</script>

<div class="md-content" bind:this={containerEl}>
  {#if rendered}
    {@html rendered}
  {:else}
    <span class="md-empty">{content}</span>
  {/if}
</div>

<style>
  .md-content {
    line-height: 1.7;
    font-size: 14.5px;
  }

  .md-content :global(p) {
    margin: 0 0 8px;
  }

  .md-content :global(p:last-child) {
    margin-bottom: 0;
  }

  .md-content :global(h1),
  .md-content :global(h2),
  .md-content :global(h3),
  .md-content :global(h4) {
    margin: 16px 0 8px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .md-content :global(h1) { font-size: 1.3em; }
  .md-content :global(h2) { font-size: 1.15em; }
  .md-content :global(h3) { font-size: 1.05em; }

  .md-content :global(ul),
  .md-content :global(ol) {
    margin: 4px 0 8px;
    padding-left: 20px;
  }

  .md-content :global(li) {
    margin-bottom: 2px;
  }

  .md-content :global(blockquote) {
    margin: 8px 0;
    padding: 4px 12px;
    border-left: 3px solid var(--accent);
    color: var(--text-secondary);
    background: var(--bg-subtle);
    border-radius: 0 6px 6px 0;
  }

  .md-content :global(code) {
    font-family: ui-monospace, 'SF Mono', Monaco, monospace;
    font-size: 0.88em;
    padding: 1px 5px;
    background: var(--bg-subtle);
    border-radius: 4px;
    color: var(--accent);
  }

  .md-content :global(pre) {
    margin: 10px 0;
    border-radius: 10px;
    overflow: hidden;
    background: #0f0f1a;
    position: relative;
  }

  .md-content :global(pre code) {
    display: block;
    padding: 14px 16px;
    background: transparent;
    font-size: 13px;
    line-height: 1.55;
    overflow-x: auto;
    color: #e2e8f0;
    font-family: ui-monospace, 'SF Mono', Monaco, monospace;
  }

  /* 复制按钮 */
  .md-content :global(.code-copy-btn) {
    position: absolute;
    top: 6px;
    right: 6px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    font-size: 11px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.7);
    cursor: pointer;
    transition: all 0.15s;
    opacity: 0;
    font-family: inherit;
  }

  .md-content :global(pre:hover .code-copy-btn) {
    opacity: 1;
  }

  .md-content :global(.code-copy-btn:hover) {
    background: rgba(255, 255, 255, 0.15);
    color: white;
  }

  .md-content :global(.code-copy-btn.copied) {
    background: rgba(74, 222, 128, 0.15);
    color: #4ade80;
    border-color: rgba(74, 222, 128, 0.3);
  }

  .md-content :global(table) {
    border-collapse: collapse;
    width: 100%;
    margin: 8px 0;
    font-size: 13px;
  }

  .md-content :global(th),
  .md-content :global(td) {
    border: 1px solid var(--border);
    padding: 6px 10px;
    text-align: left;
  }

  .md-content :global(th) {
    background: var(--bg-subtle);
    font-weight: 600;
  }

  .md-content :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 16px 0;
  }

  .md-content :global(a) {
    color: var(--link);
    text-decoration: underline;
  }

  .md-content :global(a:hover) {
    color: var(--link-hover);
  }

  .md-content :global(img) {
    max-width: 100%;
    border-radius: 8px;
    margin: 8px 0;
  }

  .md-empty {
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
