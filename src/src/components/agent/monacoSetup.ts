/**
 * Monaco 共享初始化（VSCode 核心编辑器 monaco-editor 的前端集成）。
 *
 * 被 MonacoView（通用只读代码视图）使用；FileChangesCard 的差异视图、
 * StreamBlock 的文件内容/修改对照都经由它渲染，获得行号、语法高亮、
 * 折叠、查找等编辑器能力。
 *
 * 工程约束（Tauri 桌面端）：
 * - editor.worker 用 vite 的 ?worker&inline 内联（vite 的 worker 语法
 *   只对相对路径生效，故用相对路径指向 node_modules），避免 webview
 *   单独加载 worker 文件的协议/路径问题；即便 worker 失败，Monaco
 *   也会同步降级运行（功能完整，仅大文件处理在主线程）。
 * - monaco 是模块级单例：worker 环境只设置一次（__monacoWorkerReady）。
 * - 本文件 import monaco-editor（体积大），只能被 MonacoView（懒加载）
 *   引用；主 bundle 的同步消费方一律 import 轻量的 monacoLanguages。
 */
import * as monaco from 'monaco-editor';
// 内联 worker：避免 Tauri webview 单独加载 worker 文件的协议问题
import EditorWorker from '../../../node_modules/monaco-editor/esm/vs/editor/editor.worker.js?worker&inline';

// 轻量常量/工具（无 monaco 依赖），同步 re-export 供 MonacoView 使用
export { DIFF_LANGUAGE, languageForPath } from './monacoLanguages';
import { DIFF_LANGUAGE } from './monacoLanguages';

declare global {
  // eslint-disable-next-line no-var
  var __monacoWorkerReady: boolean;
}

if (!globalThis.__monacoWorkerReady) {
  globalThis.__monacoWorkerReady = true;
  self.MonacoEnvironment = {
    getWorker: () => new EditorWorker(),
  };
}

/** 注册自定义 unified diff 语言（edit_file 的 - old / + new 对照文本） */
function ensureDiffLanguage() {
  const registered = monaco.languages.getLanguages().some((l) => l.id === DIFF_LANGUAGE);
  if (registered) return;
  monaco.languages.register({ id: DIFF_LANGUAGE, extensions: ['.diff'] });
  monaco.languages.setMonarchTokensProvider(DIFF_LANGUAGE, {
    tokenizer: {
      root: [
        [/^(diff --git|index |--- |\+\+\+ |new file|deleted file|rename from|rename to) .*$/, 'diff-header'],
        [/^@@[^@]*@@.*$/, 'diff-hunk'],
        [/^\+.*$/, 'diff-add'],
        [/^-.*$/, 'diff-del'],
        [/^\s.*$/, 'diff-context'],
        [/^.*$/, 'diff-context'],
      ],
    },
  });
}

/** 是否已跟随全局亮暗主题（html[data-theme]）切换 */
function applyTheme() {
  const dark = document.documentElement.getAttribute('data-theme') === 'dark';
  void monaco.editor.setTheme(dark ? THEME_DARK : THEME_LIGHT);
}

// 自定义主题名（base 继承 vs / vs-dark，仅叠加 diff 配色）
const THEME_LIGHT = 'astra-light';
const THEME_DARK = 'astra-dark';

// diff token → 颜色（加绿减红，与 Monaco 内置 diff 语义一致）：
// 背景用带透明度的 8 位 hex（#RRGGBBAA），行整体着色；前景深色保证文字可读。
// 注意：Monaco 0.57 的 token 主题匹配要求完整类型（Monarch 会给 token
// 自动附加语言后缀 <id>，即实际类型是 diff-add.<languageId>），
// 单独写 diff-add 不生效，故这里同时列出带后缀与不带后缀两条。
const DIFF_RULES_LIGHT: monaco.editor.ITokenThemeRule[] = [
  { token: `diff-add.${DIFF_LANGUAGE}`, foreground: '#0F7B3E', background: '#2EA04326' },
  { token: `diff-del.${DIFF_LANGUAGE}`, foreground: '#C22529', background: '#F8514926' },
  { token: `diff-header.${DIFF_LANGUAGE}`, foreground: '#8250DF', fontStyle: 'bold' },
  { token: `diff-hunk.${DIFF_LANGUAGE}`, foreground: '#0969DA', fontStyle: 'bold' },
  { token: `diff-context.${DIFF_LANGUAGE}`, foreground: '#6E7781' },
  { token: 'diff-add', foreground: '#0F7B3E', background: '#2EA04326' },
  { token: 'diff-del', foreground: '#C22529', background: '#F8514926' },
];

// 暗色主题：背景透明度略低、前景偏亮，保证对比度
const DIFF_RULES_DARK: monaco.editor.ITokenThemeRule[] = [
  { token: `diff-add.${DIFF_LANGUAGE}`, foreground: '#3FB950', background: '#2386363D' },
  { token: `diff-del.${DIFF_LANGUAGE}`, foreground: '#F85149', background: '#DA36333D' },
  { token: `diff-header.${DIFF_LANGUAGE}`, foreground: '#D2A8FF', fontStyle: 'bold' },
  { token: `diff-hunk.${DIFF_LANGUAGE}`, foreground: '#58A6FF', fontStyle: 'bold' },
  { token: `diff-context.${DIFF_LANGUAGE}`, foreground: '#8B949E' },
  { token: 'diff-add', foreground: '#3FB950', background: '#2386363D' },
  { token: 'diff-del', foreground: '#F85149', background: '#DA36333D' },
];

let themesDefined = false;
function ensureThemes() {
  if (themesDefined) return;
  themesDefined = true;
  monaco.editor.defineTheme(THEME_LIGHT, {
    base: 'vs',
    inherit: true,
    rules: DIFF_RULES_LIGHT,
    colors: {},
  });
  monaco.editor.defineTheme(THEME_DARK, {
    base: 'vs-dark',
    inherit: true,
    rules: DIFF_RULES_DARK,
    colors: {},
  });
}

/** 创建只读 Monaco 编辑器实例（主题跟随全局亮暗切换） */
export function createReadonlyMonaco(
  container: HTMLElement,
  options: { content: string; language: string },
): monaco.editor.IStandaloneCodeEditor {
  ensureDiffLanguage();
  ensureThemes();
  applyTheme();
  const editor = monaco.editor.create(container, {
    value: options.content,
    language: options.language,
    theme: document.documentElement.getAttribute('data-theme') === 'dark' ? THEME_DARK : THEME_LIGHT,
    readOnly: true,
    renderLineHighlight: 'gutter',
    minimap: { enabled: false },
    folding: true,
    lineNumbers: 'on',
    scrollBeyondLastLine: false,
    automaticLayout: true,
    fontSize: 12,
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
    glyphMargin: false,
    overviewRulerLanes: 0,
    // 不用 Monaco 的 padding 选项（WKWebView 下坐标命中会偏行）：
    // 顶部留白改由容器 CSS padding 承担，编辑器内容坐标从 0 起，
    // 行号点击选中与视觉一致。
    tabSize: 2,
    wordWrap: 'off',
  });
  // 跟随全局亮暗主题切换
  const observer = new MutationObserver(() => applyTheme());
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  // 把 observer 挂到 editor 上，dispose 时一并清理（monaco 允许附加自定义字段）
  (editor as unknown as { __themeObserver?: MutationObserver }).__themeObserver = observer;

  // WKWebView 行号点击错位修复：Monaco 的坐标命中在 WKWebView 下会偏一行
  // （点击行号 N 实际选中 N-1）。这里不依赖坐标计算，而是直接读取被点击的
  // .line-numbers 元素文本（视觉行号，绝对可靠），强制选中该行。
  // Monaco 时序：内部先用坐标设置 selection（可能错位）→ 再派发 onMouseDown
  // （本 handler 最后执行，覆盖为正确行）；mouseup 不重算 selection，故无后续覆盖。
  // 仅单击时纠正；双击保留 Monaco 原生行为（选中全部）。
  const clickFix = editor.onMouseDown((e) => {
    if (e.event.detail !== 1) return;
    const domEl = e.event.target;
    const lineEl = domEl && typeof domEl.closest === 'function' ? domEl.closest('.line-numbers') : null;
    if (!lineEl) return;
    const n = parseInt(lineEl.textContent || '', 10);
    const model = editor.getModel();
    if (!model || !Number.isFinite(n) || n <= 0) return;
    editor.setPosition({ lineNumber: n, column: 1 });
    editor.setSelection(new monaco.Selection(n, 1, n, model.getLineMaxColumn(n)));
  });
  // 挂到 editor 上，dispose 时一并清理
  (editor as unknown as { __clickFix?: monaco.IDisposable }).__clickFix = clickFix;
  return editor;
}

export { monaco };
