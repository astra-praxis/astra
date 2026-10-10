/**
 * 空状态欢迎页
 * 移植自 src/routes/chat/Welcome.svelte
 */
import './Welcome.scss';

interface Props {
  sourceLabel: string;
  modelName: string | null;
  onPick: (text: string) => void;
}

export default function Welcome({ sourceLabel, modelName, onPick }: Props) {
  return (
    <div className="welcome">
      <div className="welcome-orb" />
      <h1 className="welcome-greet">有什么可以帮你？</h1>
      <p className="welcome-sub">
        连接到 <strong>{sourceLabel}</strong>
        {modelName && <span className="welcome-model">{modelName}</span>}
        开始对话
      </p>
      <div className="welcome-examples">
        <button type="button" onClick={() => onPick('帮我解释一下什么是 Transformer 架构')}>
          <span>💡</span> 解释 Transformer 架构
        </button>
        <button type="button" onClick={() => onPick('用 Python 实现一个快速排序')}>
          <span>⚡</span> Python 快速排序
        </button>
        <button type="button" onClick={() => onPick('帮我把这段话翻译成英文：今天天气很好')}>
          <span>🌐</span> 中英翻译
        </button>
        <button type="button" onClick={() => onPick('给我讲一个关于程序员的冷笑话')}>
          <span>😄</span> 来个冷笑话
        </button>
      </div>
    </div>
  );
}
