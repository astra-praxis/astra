/**
 * 会话流单条条目渲染（从 AgentPage 拆出）：
 * EntryView 按 entry.kind 分发到用户气泡 / Markdown / 思考块 / 工具卡 / 提问卡 / 权限卡 / 终态卡。
 * EntriesFooter + VIRTUOSO_COMPONENTS 供 AgentPage 的 Virtuoso 列表使用。
 */
import { forwardRef, memo, useCallback, useEffect, useState, type HTMLAttributes } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Brain,
  BrainCircuit,
  Check,
  ChevronDown,
  CircleAlert,
  CircleCheck,
  CircleX,
  Copy,
  FileCode,
  FileText,
  HelpCircle,
  Loader2,
  MonitorUp,
  Pencil,
  RotateCcw,
  Sparkles,
  Terminal,
  Wrench,
  X,
} from 'lucide-react';
import MarkdownMessage from '../chat/MarkdownMessage';
import { StreamBlock } from '@/components/agent/StreamBlock';
import { ArtifactList } from '@/components/agent/ArtifactList';
import FilePathText from '@/components/agent/FilePathText';
import { useAgentStore, userImageUrl, type Entry } from '@/stores/agent';
import { useMascotStore } from '@/stores/mascot';
import type { UserSkillLite } from '@/hooks/useSlash';

const STATUS_LABEL: Record<string, string> = {
  analyzing: '分析中',
  executing_tool: '执行工具',
  awaiting_permission: '等待确认',
  finalizing: '收尾中',
  compacting: '压缩上下文',
  persisting: '保存上下文',
};

/** agent_tasks_running 里的子 Agent 任务条目（delegate_task 卡实时活动用） */
interface SubagentLiveTask {
  kind: string;
  id: string;
  label: string;
  elapsed_secs: number;
  detail?: string | null;
  last_tool?: string | null;
  session_id?: string | null;
  /** 发起本次委托的 delegate_task 调用 id（v1.6 精确卡片关联） */
  parent_tool_call_id?: string | null;
}

// 权限确认卡文案按工具区分（未收录的工具用兜底文案）
const PERM_ACTION_LABEL: Record<string, string> = {
  write_file: '请求写入文件',
  edit_file: '请求修改文件',
  copy_path: '请求复制文件/目录',
  move_path: '请求移动/重命名',
  create_directory: '请求创建目录',
  shell_execute: '请求执行命令',
  shell_background: '请求后台启动进程',
  shell_kill: '请求停止后台进程',
  build_docx: '请求生成 docx 文档',
  extract_document: '请求转换文档为 Markdown',
  fetch: '请求抓取网页',
  read_file: '请求读取文件',
  list_files: '请求列出目录',
};


// ===== 单条条目渲染 =====

/**
 * 工具卡按工具类型区分视觉（term 终端 / edit 编辑 / read 阅读 / memory 记忆 / misc 其他）：
 * 不同类别用不同的卡片造型、图标与配色，避免所有工具长得一样。
 */
export type ToolKind = 'term' | 'edit' | 'read' | 'memory' | 'misc';

export function toolKindOf(name: string): ToolKind {
  const n = name.toLowerCase();
  // 终端 / 命令 / 任务类
  if (
    n.includes('shell') ||
    n.includes('exec') ||
    n.includes('command') ||
    n.includes('terminal') ||
    n.includes('task') ||
    n.includes('run_command') ||
    n.includes('build_docx')
  )
    return 'term';
  // 写 / 改文件类
  if (
    n.includes('write') ||
    n.includes('edit') ||
    n.includes('patch') ||
    n.includes('append') ||
    n.includes('create_file')
  )
    return 'edit';
  // 读 / 查 / 搜 / 网络抓取
  if (
    n.includes('read') ||
    n.includes('list') ||
    n.includes('grep') ||
    n.includes('find') ||
    n.includes('search') ||
    n.includes('fetch') ||
    n.includes('open') ||
    n.includes('show') ||
    n.includes('get')
  )
    return 'read';
  // 记忆 / 嵌入 / 检查点
  if (n.includes('memory') || n.includes('remember') || n.includes('chunk') || n.includes('embed') || n.includes('checkpoint'))
    return 'memory';
  return 'misc';
}

interface UserRef {
  kind: 'skill' | 'file';
  name: string;
}

/**
 * 解析用户消息里的 /技能名（前缀，须命中已知技能）与 @文件相对路径（任意位置，须命中项目文件列表），
 * 命中的以 chip 呈现并从正文中移除；未命中的原样保留。
 * 历史回放的条目走同一解析，样式一致。
 */
function parseUserRefs(
  text: string,
  skills: UserSkillLite[],
  fileSet: Set<string>,
): { refs: UserRef[]; body: string } {
  let body = text;
  const refs: UserRef[] = [];

  // 前缀 /技能名（与发送时 parseSlashCommand 同规则）
  const sm = body.match(/^\/([\w\u4e00-\u9fff.-]+)(?:\s+([\s\S]*))?$/);
  if (sm && skills.some((s) => s.name === sm[1])) {
    refs.push({ kind: 'skill', name: sm[1] });
    body = sm[2] ?? '';
  }

  // @文件路径 token（命中项目文件才提为 chip，否则原样保留）
  body = body.replace(/@([\w\u4e00-\u9fff./-]+)/g, (full, name: string) => {
    // 贪婪匹配可能吞掉句尾点（如 @a.tsx.），去尾点后再比对；命中的 chip 只取文件路径，尾点留在正文
    const bare = name.replace(/\.+$/, '');
    const hit = fileSet.has(name) ? name : fileSet.has(bare) ? bare : null;
    if (!hit) return full;
    refs.push({ kind: 'file', name: hit });
    return name.slice(hit.length);
  });

  return { refs, body: body.trim() };
}

/** 圆点悬停预览：压成单行、限 60 字符 */
export function userMsgPreview(text: string | undefined): string {
  if (!text) return '';
  const oneLine = text.replace(/\s+/g, ' ').trim();
  return oneLine.length > 60 ? `${oneLine.slice(0, 60)}…` : oneLine;
}

/** 消息级复制按钮（用户气泡 / 助手消息右上角，hover 浮现） */
function MessageCopyButton({ text, className = '' }: { text: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className={`aentry__copy${copied ? ' is-copied' : ''}${className ? ` ${className}` : ''}`}
      title={copied ? '已复制' : '复制原文'}
      onClick={async (e) => {
        e.stopPropagation();
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 1600);
          useMascotStore.getState().say('收好啦！', { key: 'copy-result', cooldownMs: 5000 });
        } catch {
          // 剪贴板不可用时忽略
        }
      }}
    >
      {copied ? <Check size={12} strokeWidth={2.2} /> : <Copy size={12} strokeWidth={1.8} />}
    </button>
  );
}

// 助手消息底部的常驻复制（区别于用户气泡右上角的悬浮复制）：非流式时才出现
function AssistantCopyFooter({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="aentry__foot">
      <button
        type="button"
        className={`aentry__foot-copy${copied ? ' is-copied' : ''}`}
        title={copied ? '已复制' : '复制全文'}
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1600);
            useMascotStore.getState().say('收好啦！', { key: 'copy-result', cooldownMs: 5000 });
          } catch {
            // 剪贴板不可用时忽略
          }
        }}
      >
        {copied ? <Check size={12} strokeWidth={2.2} /> : <Copy size={12} strokeWidth={1.8} />}
        {copied ? '已复制' : '复制'}
      </button>
    </div>
  );
}

/**
 * 运行中状态指示：三点脉冲 loading（状态文案与停止操作已按需求移除；
 * 悬停 title 保留当前状态，取消走输入框右侧的停止按钮）
 */
// Virtuoso 条目包装：条目是普通块级元素（非 flex 子项），原 __content 的 flex gap 失效，
// 间距改由条目自身 padding（padding-bottom 纳入条目测量高度，滚动定位不累积误差）
const EntryItem = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  function EntryItem(props, ref) {
    return <div ref={ref} {...props} className="agent-entries__item" />;
  },
);

// 运行中的状态条：作为列表末尾哨兵条目渲染（不用 components.Footer ——
// followOutput 的粘底对齐只针对最后一条数据条目，Footer 会留在可视区外）。
// 必须用模块级稳定组件订阅 store，
// 不能在 AgentPage 渲染时内联定义（会因组件引用变化反复重挂导致闪烁）
export function EntriesFooter() {
  const running = useAgentStore((s) => s.running || s.sessionBusy);
  const status = useAgentStore((s) => s.status);
  if (!running) return null;
  return <StatusLine status={status} />;
}

export const VIRTUOSO_COMPONENTS = { Item: EntryItem };

function StatusLine({ status }: { status: string | null }) {
  return (
    <div className="agent-statusline" title={(status && STATUS_LABEL[status]) || '运行中'}>
      <span className="agent-statusline__dot" />
      <span className="agent-statusline__dot" />
      <span className="agent-statusline__dot" />
    </div>
  );
}

export const EntryView = memo(function EntryView({
  entry,
  index,
  isLast,
  fullText,
  isRunFinalAssistant = false,
  onDecide,
  onAnswer,
  onEdit,
  onFailureRewind,
  skills,
  fileSet,
  linkPreview = false,
}: {
  entry: Entry;
  /** 条目在会话流中的序号：写入 data-idx，供右侧圆点导轨快速定位 */
  index: number;
  /** 是否为会话流最后一条：失败卡片的重试/编辑动作只在末尾提供
   *  （更早的失败轮次走消息 hover 的「编辑重发」） */
  isLast?: boolean;
  /** 本轮 Run 聚合后的完整回复（多段 assistant 时存在）；复制用 */
  fullText?: string;
  /** 是否为该 Run 的最后一条 assistant 条目：只有它展示底部「复制全文」，避免一段回答出现多个复制按钮 */
  isRunFinalAssistant?: boolean;
  onDecide: (callId: string, allow: boolean) => Promise<void>;
  onAnswer: (questionId: string, answer: string | null) => void;
  /** hover 编辑重发（仅用户消息） */
  onEdit?: (index: number) => void;
  /** 失败卡片「重试 / 编辑后重发」 */
  onFailureRewind?: (mode: 'retry' | 'edit') => void;
  skills: UserSkillLite[];
  fileSet: Set<string>;
  /** 开启链接拦截：assistant 消息里的 http(s) 链接在右侧预览面板打开 */
  linkPreview?: boolean;
}) {
  switch (entry.kind) {
    case 'user': {
      const { refs, body } = parseUserRefs(entry.text, skills, fileSet);
      return (
        <div className="aentry aentry--user" data-idx={index}>
          <div className="aentry__bubble">
            {onEdit && (
              <button
                type="button"
                className="aentry__editbtn"
                title="编辑重发"
                aria-label="编辑重发"
                onClick={() => onEdit(index)}
              >
                <Pencil size={12} strokeWidth={1.8} />
              </button>
            )}
            <MessageCopyButton text={entry.text} />
            {entry.images && entry.images.length > 0 && (
              <div className="aentry__imgs">
                {entry.images.map((img, i) => (
                  <img
                    key={i}
                    className="aentry__img"
                    src={userImageUrl(img)}
                    alt={`附图 ${i + 1}`}
                    loading="lazy"
                  />
                ))}
              </div>
            )}
            {refs.length > 0 && (
              <div className="aentry__refs">
                {refs.map((r, i) =>
                  r.kind === 'skill' ? (
                    <span key={i} className="aentry__ref aentry__ref--skill" title={`技能 ${r.name}`}>
                      <Sparkles size={11} strokeWidth={2} />/{r.name}
                    </span>
                  ) : (
                    <span key={i} className="aentry__ref aentry__ref--file" title={`文件 ${r.name}`}>
                      <FileText size={11} strokeWidth={2} />{r.name}
                    </span>
                  ),
                )}
              </div>
            )}
            {body && <div className="aentry__text">{body}</div>}
          </div>
        </div>
      );
    }
    case 'assistant':
      // 对齐 chat 页：助手消息用 MarkdownMessage 渲染（GFM + 代码高亮），不加气泡。
      // 不渲染独立光标 span：markdown 最后一块是块级元素（p/ul/pre），
      // 光标会掉到下一行而非紧跟文字；chat 页同样只在空文本时显示 typing
      return (
        <div className={`aentry aentry--assistant${entry.streaming ? ' is-streaming' : ''}`} data-idx={index}>
          {entry.text ? (
            <>
              <div className="aentry__md">
                <MarkdownMessage content={entry.text} streaming={entry.streaming} linkPreview={linkPreview} filePreview={linkPreview} />
              </div>
              {!entry.streaming && isRunFinalAssistant && <AssistantCopyFooter text={fullText ?? entry.text} />}
            </>
          ) : (
            <div className="aentry__typing">
              <span />
              <span />
              <span />
            </div>
          )}
        </div>
      );
    case 'thinking':
      return <StreamBlock text={entry.text} done={entry.done} title="思考过程" icon={<Brain size={13} strokeWidth={1.8} />} />;
    case 'cat':
      // 猫猫互动消息：以「猫猫」前缀展示，与会话里的 agent 消息明确区分
      return (
        <div className="aentry aentry--cat" data-idx={index}>
          <div className="aentry__cat-head">
            <span className="aentry__cat-avatar" aria-hidden>🐱</span>
            <span className="aentry__cat-name">猫猫</span>
          </div>
          <div className="aentry__cat-bubble">{entry.text}</div>
        </div>
      );
    case 'tool':
      return <ToolEntry entry={entry} onDecide={onDecide} />;
    case 'question':
      return <QuestionEntry entry={entry} onAnswer={onAnswer} />;
    case 'permission': {
      const pending = entry.state === 'pending';
      const label = PERM_ACTION_LABEL[entry.toolName] ?? '请求执行操作';
      return (
        <div className={`aentry aentry--permission is-${entry.state}`}>
          <div className="aentry__perm-head">
            <CircleAlert size={14} strokeWidth={1.8} />
            <b>{entry.toolName}</b> {label}
          </div>
          <code className="aentry__perm-summary">{entry.summary}</code>
          {pending ? (
            <div className="aentry__perm-actions">
              <button type="button" className="is-allow" onClick={() => void onDecide(entry.callId, true)}>
                <Check size={13} strokeWidth={2} />
                批准
              </button>
              <button type="button" className="is-deny" onClick={() => void onDecide(entry.callId, false)}>
                <X size={13} strokeWidth={2} />
                拒绝
              </button>
            </div>
          ) : (
            <div className={`aentry__perm-state is-${entry.state}`}>
              {entry.state === 'allowed' ? '已批准' : entry.state === 'expired' ? '已失效' : '已拒绝'}
            </div>
          )}
        </div>
      );
    }
    case 'notice':
      return <div className="aentry aentry--terminal" role="status">{entry.text}</div>;
    case 'terminal':
      return (
        <div className={`aentry aentry--terminal is-${entry.outcome}`}>
          {entry.outcome === 'completed' && (
            <>
              <CircleCheck size={13} strokeWidth={1.8} /> 任务完成
            </>
          )}
          {entry.outcome === 'failed' && (
            <>
              <CircleX size={13} strokeWidth={1.8} /> 任务失败{entry.text ? `：${entry.text}` : ''}
              {entry.text?.includes('UnsupportedModel') && (
                <div className="aentry__hint">
                  所选模型与当前模型服务端点不兼容（如火山编码计划端点仅支持计划内编码模型）。
                  请在模型列表选择兼容模型，或改用标准 API 端点后新建会话。
                </div>
              )}
              {isLast && onFailureRewind && (
                <div className="aentry__rewind">
                  <button type="button" className="aentry__rewind-btn" onClick={() => onFailureRewind('retry')}>
                    <RotateCcw size={12} strokeWidth={1.8} />
                    重试
                  </button>
                  <button type="button" className="aentry__rewind-btn" onClick={() => onFailureRewind('edit')}>
                    <Pencil size={12} strokeWidth={1.8} />
                    编辑后重发
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      );
  }
});

// ===== 提问卡（ask_user：选项按钮 + 自由文本；答案 verbatim 回传模型） =====

function QuestionEntry({
  entry,
  onAnswer,
}: {
  entry: Extract<Entry, { kind: 'question' }>;
  onAnswer: (questionId: string, answer: string | null) => void;
}) {
  const [freeText, setFreeText] = useState('');
  const answered = entry.answer !== undefined || entry.declined;
  return (
    <div className="aentry aentry--question is-answered">
      <div className="aentry__q-head">
        <HelpCircle size={14} strokeWidth={1.8} />
        <span>{entry.question}</span>
      </div>
      {answered ? (
        <div className={`aentry__q-state${entry.declined ? ' is-declined' : ''}`}>
          {entry.declined ? '未回答' : `你的回答：${entry.answer}`}
        </div>
      ) : (
        <div className="aentry__q-body">
          {entry.options.length > 0 && (
            <div className="aentry__q-options">
              {entry.options.map((o) => (
                <button
                  key={o}
                  type="button"
                  onClick={() => onAnswer(entry.questionId, o)}
                >
                  {o}
                </button>
              ))}
            </div>
          )}
          {entry.allowFreeText && (
            <div className="aentry__q-free">
              <input
                type="text"
                value={freeText}
                placeholder={entry.options.length > 0 ? '或输入自定义回答…' : '输入你的回答…'}
                onChange={(e) => setFreeText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && freeText.trim()) {
                    onAnswer(entry.questionId, freeText.trim());
                  }
                }}
              />
              <button
                type="button"
                className="is-allow"
                disabled={!freeText.trim()}
                onClick={() => onAnswer(entry.questionId, freeText.trim())}
              >
                <Check size={13} strokeWidth={2} />
                回答
              </button>
            </div>
          )}
          <div className="aentry__q-actions">
            <button type="button" className="is-deny" onClick={() => onAnswer(entry.questionId, null)}>
              <X size={13} strokeWidth={2} />
              取消回答
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ===== 工具卡片（结果默认折叠，点击展开，避免大段文件/终端输出刷屏） =====

/**
 * 从不完整的参数 JSON 中宽松提取可预览文本（流式生成中 args 未到齐）。
 * 逐字符扫描顶层 "key": "value" 对，容忍尾部被截断的字符串。
 * 返回 undefined 表示暂无可展示内容（如 JSON 还没出现任何键）。
 */
export function extractPartialArgsPreview(
  toolName: string,
  raw: string,
): string | undefined {
  if (!raw) return undefined;
  const pairs: Array<[string, string]> = [];
  let i = 0;
  const n = raw.length;
  while (i < n) {
    // 找下一个键的起始引号（跳过 { , 空白）
    if (raw[i] !== '"') {
      i += 1;
      continue;
    }
    const keyStart = i + 1;
    const keyEnd = raw.indexOf('"', keyStart);
    if (keyEnd < 0) break;
    const key = raw.slice(keyStart, keyEnd);
    let j = keyEnd + 1;
    while (j < n && raw[j] !== ':') j += 1;
    j += 1; // 跳过冒号
    while (j < n && raw[j] === ' ') j += 1;
    if (j >= n) break;
    if (raw[j] === '"') {
      // 字符串值：可能未闭合（尾部截断）
      let value = '';
      let k = j + 1;
      let closed = false;
      while (k < n) {
        const ch = raw[k];
        if (ch === '\\' && k + 1 < n) {
          // 转义：还原常见控制符，避免预览里出现 \n 字面量
          const esc = raw[k + 1];
          value += esc === 'n' ? '\n' : esc === 't' ? '\t' : esc;
          k += 2;
          continue;
        }
        if (ch === '"') {
          closed = true;
          break;
        }
        value += ch;
        k += 1;
      }
      pairs.push([key, value]);
      i = closed ? k + 1 : n; // 未闭合说明参数已截断，到此为止
    } else {
      // 非字符串值（数字/布尔）：取到逗号或结尾
      let k = j;
      while (k < n && raw[k] !== ',' && raw[k] !== '}') k += 1;
      pairs.push([key, raw.slice(j, k).trim()]);
      i = k;
    }
  }
  if (pairs.length === 0) return undefined;
  // 按工具形态组织预览：与 contentPreview 的约定一致
  const get = (k: string) => pairs.find(([key]) => key === k)?.[1];
  if (toolName === 'write_file' || toolName === 'read_file') {
    const content = get('content');
    return content !== undefined ? content : pairs.map(([k, v]) => `${k}: ${v}`).join('\n');
  }
  if (toolName === 'edit_file') {
    const oldT = get('old_text');
    const newT = get('new_text');
    if (oldT !== undefined || newT !== undefined) {
      return `- ${oldT ?? ''}\n+ ${newT ?? ''}`;
    }
  }
  if (toolName === 'shell_execute') {
    const cmd = get('command');
    return cmd !== undefined ? cmd : undefined;
  }
  return pairs.map(([k, v]) => `${k}: ${v}`).join('\n');
}

function ToolEntry({
  entry,
  onDecide,
}: {
  entry: Extract<Entry, { kind: 'tool' }>;
  onDecide: (callId: string, allow: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const hasBody = !!(entry.result || (entry.artifacts && entry.artifacts.length > 0));
  const perm = entry.perm;
  const toolKind = toolKindOf(entry.toolName);
  // 各类工具的头部图标（终端 / 编辑 / 阅读 / 记忆 / 其他）
  const kindIcon =
    toolKind === 'term' ? (
      <Terminal size={12} strokeWidth={1.8} />
    ) : toolKind === 'edit' ? (
      <FileCode size={12} strokeWidth={1.8} />
    ) : toolKind === 'read' ? (
      <FileText size={12} strokeWidth={1.8} />
    ) : toolKind === 'memory' ? (
      <BrainCircuit size={12} strokeWidth={1.8} />
    ) : (
      <Wrench size={12} strokeWidth={1.8} />
    );
  // 写文件类工具展示完整内容，shell 展示完整命令，edit 展示新旧文本对照
  // （args 由 tool_call_started 事件携带，旧会话无此字段）
  const contentPreview =
    entry.args && typeof entry.args.content === 'string'
      ? entry.args.content
      : entry.toolName === 'edit_file' &&
        entry.args &&
        typeof entry.args.old_text === 'string' &&
        typeof entry.args.new_text === 'string'
        ? `- ${entry.args.old_text}\n+ ${entry.args.new_text}`
        : entry.toolName === 'shell_execute' && entry.args && typeof entry.args.command === 'string'
          ? entry.args.command
          : undefined;
  // 参数还在流式生成中（tool_call_started 未到）：宽松解析不完整 JSON 展示已生成部分。
  // 同一工具卡：args 到齐后切 contentPreview，流式阶段展示 streamingPreview
  const streamingPreview =
    !contentPreview && entry.argsStreaming !== undefined
      ? extractPartialArgsPreview(entry.toolName, entry.argsStreaming)
      : undefined;
  const previewText = contentPreview ?? streamingPreview;
  // 内容已完成 = args 已到齐（区别于流式生成中）；驱动 StreamBlock 自动收起
  const previewDone = contentPreview !== undefined;
  const previewTitle =
    entry.toolName === 'write_file'
      ? '写入内容'
      : entry.toolName === 'read_file'
        ? '文件内容'
        : entry.toolName === 'edit_file'
          ? '修改对照'
          : entry.toolName === 'shell_execute' || entry.toolName === 'shell_background'
            ? '命令'
            : '参数';
  // shell_background 完成后：从结果中取 task_id，提供「预览」入口（host 从日志检测地址后开窗）
  const bgTaskId =
    entry.toolName === 'shell_background' && entry.status === 'completed' && entry.result
      ? (entry.result.match(/task_id=(\d+)/)?.[1] ?? null)
      : null;
  const [previewHint, setPreviewHint] = useState('');
  const openPreview = useCallback(async () => {
    if (!bgTaskId) return;
    setPreviewHint('');
    try {
      await invoke('agent_open_preview', { taskId: Number(bgTaskId) });
    } catch (e) {
      setPreviewHint(String(e).replace(/^.*错误:/, '').trim());
    }
  }, [bgTaskId]);

  // delegate_task 阻塞等子任务完成（可达数分钟），运行中结果为空、卡片不可展开。
  // 轮询运行中子任务清单，把本会话的子 Agent 实时状态（阶段 + 最近工具调用）
  // 内联到卡片上，让用户随时知道子任务在干嘛、不是死等。等待确认阶段还没有
  // 子任务，不展示。
  const isDelegate = entry.toolName === 'delegate_task';
  const delegateLive =
    isDelegate && entry.status === 'running' && perm?.state !== 'pending';
  const [subTask, setSubTask] = useState<SubagentLiveTask | null>(null);
  useEffect(() => {
    if (!delegateLive) {
      setSubTask(null);
      return;
    }
    let alive = true;
    const poll = async () => {
      try {
        const list = await invoke<SubagentLiveTask[]>('agent_tasks_running');
        if (!alive) return;
        // v1.6 并行委托：按发起本次 delegate_task 的 tool_call_id 精确关联
        // （后端 parent_tool_call_id），不做 session 模糊匹配——同一会话
        // 可能有多个子任务同时跑。后端没带 id 时保持无实时信息（显式，
        // 不静默回退到错误关联）。
        setSubTask(
          list.find(
            (t) =>
              t.kind === 'subagent' &&
              !!t.parent_tool_call_id &&
              t.parent_tool_call_id === entry.callId,
          ) ?? null,
        );
      } catch {
        // 纯 web 调试等后端不可用场景：静默
      }
    };
    void poll();
    const timer = setInterval(poll, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [delegateLive, entry.callId]);
  return (
    <div
      className={`aentry aentry--tool is-${entry.status}${perm?.state === 'pending' ? ' is-perm' : ''}`}
      data-tool-kind={toolKind}
    >
      <button
        type="button"
        className="aentry__tool-head"
        onClick={() => hasBody && setOpen((v) => !v)}
        title={hasBody ? (open ? '收起' : '展开结果') : undefined}
      >
        {entry.status === 'running' ? (
          <Loader2 size={13} strokeWidth={2} className="aentry__spin" />
        ) : entry.status === 'completed' ? (
          <CircleCheck size={13} strokeWidth={1.8} />
        ) : entry.status === 'denied' ? (
          <CircleX size={13} strokeWidth={1.8} />
        ) : (
          <CircleAlert size={13} strokeWidth={1.8} />
        )}
        {kindIcon}
        {toolKind === 'term' && (
          <span className="aentry__term-dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
        )}
        <code className="aentry__tool-name">{entry.toolName}</code>
        <span className="aentry__tool-status">
          {perm?.state === 'pending'
            ? '等待确认'
            : entry.status === 'running'
              ? '执行中'
              : entry.status === 'completed'
                ? '完成'
                : entry.status === 'denied'
                  ? '已拒绝'
                  : entry.status === 'interrupted' ? '已中断' : '失败'}
        </span>
        {hasBody && (
          <ChevronDown size={12} strokeWidth={2} className={`aentry__tool-chev${open ? ' rotated' : ''}`} />
        )}
      </button>
      {bgTaskId && (
        <div className="aentry__preview">
          <button type="button" className="aentry__preview-btn" onClick={() => void openPreview()}>
            <MonitorUp size={12} strokeWidth={1.8} />
            预览
          </button>
          {previewHint && <span className="aentry__preview-hint">{previewHint}</span>}
        </div>
      )}
      {previewText && (
        <StreamBlock
          variant="tool"
          title={previewTitle}
          icon={<FileText size={12} strokeWidth={1.8} />}
          text={previewText}
          done={previewDone}
          diff={toolKind === 'edit' && entry.toolName === 'edit_file'}
        />
      )}
      {delegateLive && subTask?.last_tool && (() => {
        // last_tool = "tool_name · args_summary"（core observer 拼装）：
        // 拆开分层展示——工具名强调、参数弱化省略；title 保留完整内容。
        // 无工具活动时整行不渲染：头部已有 loading，左下角不再放 spinner
        const sep = subTask.last_tool.indexOf(' · ');
        const name = sep >= 0 ? subTask.last_tool.slice(0, sep) : subTask.last_tool;
        const args = sep >= 0 ? subTask.last_tool.slice(sep + 3) : '';
        return (
          <div className="aentry__subtask has-tool">
            <span className="aentry__subtask-tool" title={subTask.last_tool}>
              <code className="aentry__subtask-tool-name">{name}</code>
              {args && <span className="aentry__subtask-tool-args">{args}</span>}
            </span>
          </div>
        );
      })()}
      {/* Confirm 工具的批准/拒绝直接内嵌在卡片里 */}
      {perm?.state === 'pending' && (
        <div className="aentry__perm-inline">
          <code className="aentry__perm-summary">{perm.summary}</code>
          <div className="aentry__perm-actions">
            <button type="button" className="is-allow" onClick={() => void onDecide(entry.callId, true)}>
              <Check size={13} strokeWidth={2} />
              批准
            </button>
            <button type="button" className="is-deny" onClick={() => void onDecide(entry.callId, false)}>
              <X size={13} strokeWidth={2} />
              拒绝
            </button>
          </div>
        </div>
      )}
      {open && hasBody && (
        <div className="aentry__tool-body">
          {entry.result && <pre className="aentry__tool-result"><FilePathText text={entry.result} /></pre>}
          {entry.artifacts && entry.artifacts.length > 0 && (
            <ArtifactList artifacts={entry.artifacts} />
          )}
        </div>
      )}
    </div>
  );
}
