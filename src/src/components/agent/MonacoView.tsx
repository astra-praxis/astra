/**
 * MonacoView —— 通用只读代码视图（基于 VSCode 核心编辑器 monaco-editor）。
 *
 * 用法：
 * - 普通代码内容：<MonacoView content={text} language={languageForPath(path)} />
 * - unified diff 文本：<MonacoView content={diff} language={DIFF_LANGUAGE} />
 *
 * 体积大（monaco 主包），必须经 React.lazy 懒加载成独立 chunk；
 * 本组件只负责创建/销毁编辑器实例，不参与业务状态。
 */
import { useEffect, useRef } from 'react';
import { createReadonlyMonaco, DIFF_LANGUAGE } from './monacoSetup';

export { DIFF_LANGUAGE, languageForPath } from './monacoSetup';

interface MonacoViewProps {
  /** 要渲染的文本（普通代码 或 unified diff） */
  content: string;
  /** monaco 语言 id（如 'typescript'、'rust'、DIFF_LANGUAGE） */
  language: string;
  /** 标题/数据属性用；不参与渲染 */
  path?: string;
}

export default function MonacoView({ content, language, path }: MonacoViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const editor = createReadonlyMonaco(el, { content, language });
    return () => {
      const editorAny = editor as unknown as {
        __themeObserver?: MutationObserver;
        __clickFix?: { dispose(): void };
      };
      editorAny.__themeObserver?.disconnect();
      editorAny.__clickFix?.dispose();
      editor.dispose();
    };
    // content/language 变化由调用方通过 key 重建（避免 Monaco 大文本 setValue 卡顿）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={containerRef} className="monaco-view" data-path={path ?? ''} style={{ width: '100%', height: '100%', paddingTop: 8, paddingBottom: 8, boxSizing: 'border-box' }} />;
}
