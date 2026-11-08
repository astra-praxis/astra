/**
 * Monaco 相关的轻量常量/工具（无 monaco-editor 依赖，可被主 bundle 同步引用）。
 *
 * 注意：不要在此 import monaco-editor——否则会把 monaco 拉进主 bundle，
 * 破坏 MonacoView 的懒加载。monaco 本体只经 monacoSetup（MonacoView 内）加载。
 */

/** unified diff 的 monaco 语言 id（注册见 monacoSetup） */
export const DIFF_LANGUAGE = 'astra-unified-diff';

/** 根据文件名扩展名推断 monaco 语言（基础语言内置，无需额外 worker） */
export function languageForPath(path: string): string {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  const map: Record<string, string> = {
    ts: 'typescript',
    tsx: 'typescript',
    js: 'javascript',
    jsx: 'javascript',
    mjs: 'javascript',
    cjs: 'javascript',
    json: 'json',
    rs: 'rust',
    py: 'python',
    go: 'go',
    java: 'java',
    c: 'c',
    h: 'c',
    cpp: 'cpp',
    cc: 'cpp',
    hpp: 'cpp',
    cs: 'csharp',
    css: 'css',
    scss: 'scss',
    less: 'less',
    html: 'html',
    htm: 'html',
    vue: 'html',
    xml: 'xml',
    yaml: 'yaml',
    yml: 'yaml',
    toml: 'ini',
    ini: 'ini',
    md: 'markdown',
    markdown: 'markdown',
    sh: 'shell',
    bash: 'shell',
    zsh: 'shell',
    ps1: 'powershell',
    sql: 'sql',
    swift: 'swift',
    kt: 'kotlin',
    dart: 'dart',
    php: 'php',
    rb: 'ruby',
    lua: 'lua',
    r: 'r',
    zig: 'zig',
  };
  return map[ext] ?? 'plaintext';
}
