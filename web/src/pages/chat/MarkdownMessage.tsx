/**
 * Markdown 消息渲染 + action 链接执行 + shell 代码块执行
 * 移植自 src/routes/chat/MarkdownMessage.svelte。
 *
 * 与原版差异：
 * - marked + hljs 手动渲染 → react-markdown + remark-gfm + rehype-highlight
 * - 代码块按钮不再手动操作 DOM，改为 React 组件 + 状态
 * - 命令输出事件通过模块级 sink 注册表广播（与原版"广播给所有执行中任务"一致）
 */
import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  isValidElement,
  type ReactNode,
} from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Copy, Play } from 'lucide-react';
import { parseActionHref, executeAction } from './action-links';
import './MarkdownMessage.scss';
import 'highlight.js/styles/github-dark.css';

// ============================================================
// 上下文（components 必须定义在模块级，保证组件类型稳定，避免重挂载）
// ============================================================

interface MdContextValue {
  workspacePath: string;
  isFresh: boolean;
}

const MdContext = createContext<MdContextValue>({ workspacePath: '', isFresh: false });

interface MarkdownMessageProps {
  content?: string;
  workspacePath?: string;
  isFresh?: boolean;
}

export default function MarkdownMessage({
  content = '',
  workspacePath = '',
  isFresh = false,
}: MarkdownMessageProps) {
  // 命令输出事件监听（每个实例注册，卸载时清理）
  useEffect(() => {
    const listeners: Promise<UnlistenFn>[] = [
      listen<string>('command-stdout', (e) => {
        runningSinks.forEach((s) => s.onLine(e.payload, 'stdout'));
      }),
      listen<string>('command-stderr', (e) => {
        runningSinks.forEach((s) => s.onLine(e.payload, 'stderr'));
      }),
      listen('command-done', () => {
        runningSinks.forEach((s) => s.onFinish(true));
      }),
      listen<string>('command-error', (e) => {
        runningSinks.forEach((s) => s.onFinish(false, e.payload));
      }),
    ];
    return () => {
      Promise.all(listeners).then((fns) => fns.forEach((fn) => fn()));
    };
  }, []);

  return (
    <MdContext.Provider value={{ workspacePath, isFresh }}>
      <div className="md-content">
        {content ? (
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            rehypePlugins={[rehypeHighlight]}
            components={mdComponents}
          >
            {content}
          </ReactMarkdown>
        ) : (
          <span className="md-empty">{content}</span>
        )}
      </div>
    </MdContext.Provider>
  );
}

// ============================================================
// 命令输出广播（模块级：所有 MarkdownMessage 实例共享）
// ============================================================

type LineType = 'stdout' | 'stderr' | 'error';

interface OutputLine {
  text: string;
  type: LineType;
}

interface OutputSink {
  onLine: (text: string, type: LineType) => void;
  onFinish: (ok: boolean, errMsg?: string) => void;
}

const runningSinks = new Set<OutputSink>();

// ============================================================
// markdown 组件映射（模块级，类型稳定）
// ============================================================

const mdComponents: Components = {
  pre: PreRenderer,
  a: ActionAnchor,
};

function PreRenderer({ children }: { children?: ReactNode }) {
  const codeEl = isValidElement(children) ? children : null;
  const codeProps = (codeEl?.props ?? {}) as { className?: string; children?: ReactNode };
  const raw = nodeText(codeProps.children ?? null);
  return (
    <CodeBlock className={codeProps.className ?? ''} rawText={raw}>
      {children}
    </CodeBlock>
  );
}

function ActionAnchor({ href, children }: { href?: string; children?: ReactNode }) {
  const { isFresh } = useContext(MdContext);
  if (href && href.startsWith('action://')) {
    return (
      <ActionLink href={href} isFresh={isFresh}>
        {children}
      </ActionLink>
    );
  }
  return <a href={href}>{children}</a>;
}

/** 从高亮后的 React 子树中提取纯文本 */
function nodeText(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(nodeText).join('');
  if (isValidElement(node)) {
    return nodeText((node.props as { children?: ReactNode }).children);
  }
  return '';
}

// ============================================================
// 代码块（复制按钮 + shell 执行按钮 + 确认弹窗 + 输出展示）
// ============================================================

type ExecStatus = 'confirming' | 'running' | 'done' | 'error';

interface ExecState {
  status: ExecStatus;
  lines: OutputLine[];
}

function CodeBlock({ className, rawText, children }: { className: string; rawText: string; children?: ReactNode }) {
  const { workspacePath } = useContext(MdContext);
  const lang = /language-([\w-]+)/.exec(className)?.[1] ?? '';
  const isShell = ['bash', 'sh', 'shell'].includes(lang);

  const [copied, setCopied] = useState(false);
  const [exec, setExec] = useState<ExecState | null>(null);
  const sinkRef = useRef<OutputSink | null>(null);

  // 卸载时注销未完成的输出 sink
  useEffect(
    () => () => {
      if (sinkRef.current) {
        runningSinks.delete(sinkRef.current);
        sinkRef.current = null;
      }
    },
    [],
  );

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(rawText);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // 复制失败忽略
    }
  }

  function handleExecClick() {
    if (exec && (exec.status === 'running' || exec.status === 'confirming')) return;
    setExec({ status: 'confirming', lines: [] });
  }

  function cancelExec() {
    setExec(null);
  }

  async function confirmExec() {
    setExec({ status: 'running', lines: [] });
    const sink: OutputSink = {
      onLine: (text, type) => {
        setExec((prev) =>
          prev && prev.status === 'running' ? { ...prev, lines: [...prev.lines, { text, type }] } : prev,
        );
      },
      onFinish: (ok, errMsg) => {
        runningSinks.delete(sink);
        sinkRef.current = null;
        setExec((prev) => {
          if (!prev || prev.status !== 'running') return prev;
          const lines = errMsg ? [...prev.lines, { text: `❌ ${errMsg}`, type: 'error' as const }] : prev.lines;
          return { ...prev, status: ok ? 'done' : 'error', lines };
        });
      },
    };
    sinkRef.current = sink;
    runningSinks.add(sink);
    try {
      await invoke('run_command', {
        command: rawText,
        cwd: workspacePath || undefined,
        timeoutSecs: 300,
      });
    } catch (err) {
      sink.onFinish(false, String(err));
    }
  }

  return (
    <>
      <pre>
        {isShell && (
          <button type="button" className="code-x-btn code-exec-btn" title="执行该命令" onClick={handleExecClick}>
            <Play size={12} strokeWidth={0} fill="currentColor" /> 执行
          </button>
        )}
        <button
          type="button"
          className={`code-x-btn code-copy-btn${copied ? ' copied' : ''}`}
          onClick={() => void handleCopy()}
        >
          <Copy size={12} strokeWidth={2.2} /> {copied ? '已复制' : '复制'}
        </button>
        {children}
      </pre>
      {exec && (
        <CommandOutput exec={exec} command={rawText} onCancel={cancelExec} onConfirm={() => void confirmExec()} />
      )}
    </>
  );
}

function CommandOutput({
  exec,
  command,
  onCancel,
  onConfirm,
}: {
  exec: ExecState;
  command: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="command-output">
      <div className="output-header">
        <span className="output-title">命令输出</span>
        <button type="button" className="output-close" onClick={onCancel}>
          ×
        </button>
      </div>
      <div className="output-body">
        {exec.status === 'confirming' ? (
          <div className="confirm-dialog">
            <div className="confirm-title">⚠️ 确认执行命令？</div>
            <pre className="confirm-preview">{command}</pre>
            <div className="confirm-actions">
              <button type="button" className="btn-cancel" onClick={onCancel}>
                取消
              </button>
              <button type="button" className="btn-confirm" onClick={onConfirm}>
                允许执行
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className={`output-status ${exec.status}`}>
              {exec.status === 'running' ? '正在执行...' : exec.status === 'done' ? '✓ 执行完成' : '✗ 执行失败'}
            </div>
            <div className="output-lines">
              {exec.lines.map((line, i) => (
                <div key={i} className={`output-line ${line.type}`}>
                  {line.text}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ============================================================
// action:// 链接（点击执行 + 新消息自动执行 open_project）
// ============================================================

function ActionLink({ href, isFresh, children }: { href: string; isFresh: boolean; children?: ReactNode }) {
  const info = parseActionHref(href);
  const [status, setStatus] = useState<'idle' | 'running' | 'done' | 'error'>('idle');
  const [label, setLabel] = useState<string | null>(null);
  const statusRef = useRef<'idle' | 'running' | 'done' | 'error'>('idle');
  const autoRanRef = useRef(false);

  async function run() {
    if (!info || statusRef.current !== 'idle') return;
    statusRef.current = 'running';
    setStatus('running');
    const result = await executeAction(info);
    statusRef.current = result.ok ? 'done' : 'error';
    setStatus(result.ok ? 'done' : 'error');
    setLabel(result.label);
  }

  // 仅对新消息自动执行 open_project（与原版 isFresh 逻辑一致）
  useEffect(() => {
    if (isFresh && info?.actionType === 'open_project' && !autoRanRef.current) {
      autoRanRef.current = true;
      void run();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runningLabel = info?.actionType === 'open_project' ? '⏳ 正在打开...' : '⏳ 正在执行...';
  const cls =
    status === 'running' ? 'action-executed' : status === 'done' ? 'action-done' : status === 'error' ? 'action-error' : '';

  return (
    <a
      href={href}
      className={cls || undefined}
      onClick={(e) => {
        e.preventDefault();
        void run();
      }}
    >
      {status === 'running' ? runningLabel : (label ?? children)}
    </a>
  );
}
