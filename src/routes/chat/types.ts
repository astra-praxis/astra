/** 对话页共享类型 */

export interface ChatMessage {
    role: "user" | "assistant";
    content: string;
    reasoning?: string;
    timestamp: number;
    error?: boolean;
    favorite?: boolean;
    showReasoning?: boolean;
    isFresh?: boolean; // 新消息标记（用于控制 action 链接是否自动执行）
}

export type ChatSource =
    | { type: "model"; port: number; model_name: string }
    | { type: "provider"; provider_id: string; model: string | null };

export interface ChatSession {
    id: string;
    title: string;
    source: ChatSource;
    messages: Array<{
        role: string;
        content: string;
        reasoning?: string;
        timestamp?: number;
        error?: boolean;
    }>;
    created_at: number;
    updated_at: number;
}
