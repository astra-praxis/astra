/**
 * 单条消息渲染
 * 移植自 src/routes/chat/ChatMessageView.svelte。
 * 推理过程折叠改为受控：通过 onToggleReasoning 回调更新父级 state。
 */
import { ChevronDown, Copy, Star } from 'lucide-react';
import MarkdownMessage from './MarkdownMessage';
import type { ChatMessage } from './types';
import './ChatMessageView.scss';

interface Props {
  msg: ChatMessage;
  isLast: boolean;
  isSending: boolean;
  workspacePath: string;
  onCopy: (msg: ChatMessage) => void;
  onToggleFavorite: (msg: ChatMessage) => void;
  onToggleReasoning: () => void;
}

export default function ChatMessageView({
  msg,
  isLast,
  isSending,
  workspacePath,
  onCopy,
  onToggleFavorite,
  onToggleReasoning,
}: Props) {
  const roleCls = msg.role === 'user' ? 'msg user' : 'msg assistant';

  return (
    <article className={roleCls}>
      <div className={`msg-content${msg.error ? ' err' : ''}`}>
        {/* 推理过程（可折叠） */}
        {msg.reasoning && (
          <div className="reasoning-wrap">
            <button type="button" className="reasoning-toggle" onClick={onToggleReasoning}>
              <span className="reasoning-dot" />
              <span>推理过程</span>
              <ChevronDown size={12} strokeWidth={2.5} className={msg.showReasoning ? 'rotated' : ''} />
            </button>
            {msg.showReasoning && <pre className="reasoning-text">{msg.reasoning}</pre>}
          </div>
        )}

        {/* 主回复内容 */}
        {!msg.content && isSending && isLast ? (
          <div className="typing">
            <span />
            <span />
            <span />
          </div>
        ) : msg.role === 'assistant' ? (
          <div className="msg-body">
            <MarkdownMessage content={msg.content} workspacePath={workspacePath} isFresh={msg.isFresh || false} />
          </div>
        ) : (
          <pre className="msg-text">{msg.content}</pre>
        )}

        {/* 底部操作按钮（仅 AI 输出） */}
        {msg.role === 'assistant' && msg.content && !(isSending && isLast) && (
          <div className="msg-footer">
            <button
              type="button"
              className="mf-btn"
              onClick={() => onCopy(msg)}
              title="复制全文"
              aria-label="复制全文"
            >
              <Copy size={14} strokeWidth={2} />
            </button>
            <button
              type="button"
              className={`mf-btn${msg.favorite ? ' faved' : ''}`}
              onClick={() => onToggleFavorite(msg)}
              title={msg.favorite ? '取消收藏' : '收藏'}
              aria-label="收藏"
            >
              <Star size={14} strokeWidth={2} fill={msg.favorite ? 'currentColor' : 'none'} />
            </button>
          </div>
        )}
      </div>
    </article>
  );
}
