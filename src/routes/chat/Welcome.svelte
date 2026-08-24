<script lang="ts">
    interface Props {
        sourceLabel: string;
        modelName: string | null;
        onPick: (text: string) => void;
    }

    let { sourceLabel, modelName, onPick }: Props = $props();
</script>

<div class="welcome">
    <div class="welcome-orb"></div>
    <h1 class="welcome-greet">有什么可以帮你？</h1>
    <p class="welcome-sub">
        连接到 <strong>{sourceLabel}</strong>
        {#if modelName}<span class="welcome-model">{modelName}</span>{/if}
        开始对话
    </p>
    <div class="welcome-examples">
        <button onclick={() => onPick("帮我解释一下什么是 Transformer 架构")}>
            <span>💡</span> 解释 Transformer 架构
        </button>
        <button onclick={() => onPick("用 Python 实现一个快速排序")}>
            <span>⚡</span> Python 快速排序
        </button>
        <button
            onclick={() => onPick("帮我把这段话翻译成英文：今天天气很好")}
        >
            <span>🌐</span> 中英翻译
        </button>
        <button onclick={() => onPick("给我讲一个关于程序员的冷笑话")}>
            <span>😄</span> 来个冷笑话
        </button>
    </div>
</div>

<style>
    .welcome {
        margin: auto;
        text-align: center;
        padding: 40px 20px;
        max-width: 600px;
    }

    .welcome-orb {
        width: 80px;
        height: 80px;
        border-radius: 50%;
        margin: 0 auto 24px;
        background: var(--accent-gradient);
        position: relative;
        box-shadow: 0 12px 40px var(--accent-shadow);
        animation: orbFloat 4s ease-in-out infinite;
    }

    .welcome-orb::before {
        content: "";
        position: absolute;
        inset: -8px;
        border-radius: 50%;
        background: var(--accent-gradient);
        opacity: 0.25;
        filter: blur(16px);
        animation: orbPulse 4s ease-in-out infinite;
    }

    @keyframes orbFloat {
        0%,
        100% {
            transform: translateY(0);
        }
        50% {
            transform: translateY(-8px);
        }
    }

    @keyframes orbPulse {
        0%,
        100% {
            transform: scale(1);
            opacity: 0.25;
        }
        50% {
            transform: scale(1.15);
            opacity: 0.4;
        }
    }

    .welcome-greet {
        font-size: 28px;
        font-weight: 700;
        color: var(--text-primary);
        margin-bottom: 8px;
        letter-spacing: -0.02em;
    }

    .welcome-sub {
        font-size: 14px;
        color: var(--text-muted);
        margin-bottom: 32px;
    }

    .welcome-sub strong {
        color: var(--text-secondary);
        font-weight: 600;
    }

    .welcome-model {
        display: inline-block;
        margin-left: 6px;
        padding: 2px 8px;
        background: var(--accent-bg);
        color: var(--accent);
        border-radius: 6px;
        font-family: ui-monospace, monospace;
        font-size: 12px;
    }

    .welcome-examples {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 10px;
        text-align: left;
    }

    .welcome-examples button {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 16px;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        font-size: 13px;
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.2s;
        text-align: left;
        font-family: inherit;
    }

    .welcome-examples button:hover {
        border-color: var(--accent);
        color: var(--accent);
        transform: translateY(-2px);
        box-shadow: 0 6px 16px var(--shadow-md);
    }

    .welcome-examples button span {
        font-size: 18px;
    }

    @media (max-width: 640px) {
        .welcome-examples {
            grid-template-columns: 1fr;
        }
    }
</style>
