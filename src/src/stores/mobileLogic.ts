/**
 * 手机遥控桥的纯决策逻辑（无 import，stores/mobile.ts 与 node 单测共用）。
 *
 * 真相在 agent store 与 mobile_bridge.rs；这里只回答「收到手机指令后该做什么」：
 * - parseMobileSend：/技能 前缀解析，语义对齐桌面 AgentPage.parseSendInput
 * - planEnqueue：发送路径决策（空闲直发 / 重用草稿 / 新开 tab / 激活会话 / 排队 / 拒绝）
 * - sanitizeQueueSnapshot：队列镜像剔除图片（base64 太重，手机端只展示文本）
 * - shouldActivateAfterSettle：驻留会话结算后是否需要换到前台触发既有续发
 */

export interface MobileSkillLite {
  name: string;
  description: string;
}

export interface ParsedSend {
  ok: boolean;
  /** ok = true：排队参数（raw 保留原文含 /前缀，入队可读） */
  raw: string;
  text: string;
  skill: string | null;
  /** ok = false：给手机端的 toast 文案 */
  error: string;
}

/** 与桌面 parseSendInput 同语义：/已知技能 → 拆前缀；未知技能 / 空内容 → 拒发并说明 */
export function parseMobileSend(text: string, skills: MobileSkillLite[]): ParsedSend {
  const trimmed = text.trim();
  if (!trimmed) return { ok: false, raw: '', text: '', skill: null, error: '消息内容为空' };
  if (!trimmed.startsWith('/')) {
    return { ok: true, raw: trimmed, text: trimmed, skill: null, error: '' };
  }
  const m = trimmed.match(/^\/([\w\u4e00-\u9fff.-]+)(?:\s+([\s\S]*))?$/);
  const name = m?.[1] ?? '';
  if (!m || !skills.some((s) => s.name === name)) {
    return {
      ok: false,
      raw: trimmed,
      text: '',
      skill: null,
      error: `未知技能 /${name}（可在桌面端 设置 → 我的 Skills 创建或导入）`,
    };
  }
  const rest = (m[2] ?? '').trim();
  if (!rest) {
    return {
      ok: false,
      raw: trimmed,
      text: '',
      skill: null,
      error: '请输入任务内容，例如 /demo 把需求整理成方案',
    };
  }
  return { ok: true, raw: trimmed, text: rest, skill: name, error: '' };
}

export interface EnqueueContext {
  /** 当前激活会话 */
  activeId: string;
  /** 当前是否草稿（未发首条消息） */
  isDraft: boolean;
  entriesLen: number;
  /** sessionBusy（新建/删除会话等互斥操作进行中） */
  busy: boolean;
  /** 已知会话 id 集合（sessions 列表） */
  knownSessionIds: string[];
  /** 正在运行/结算的会话 id（flat 运行中 + 驻留运行中）。P3 并发下只在
   *  判断「目标会话自己是否在跑」时用成员判断，不再做全局准入推断 */
  runningIds: string[];
}

export type EnqueuePlan =
  /** 手机未指定会话且当前是空草稿 → 直接排进当前会话 */
  | { action: 'reuse-current'; direct: boolean }
  /** 手机未指定会话 = 新对话意图 → 新开草稿 tab 直发（P3 并发：与运行中的
   *  会话并行起跑，不再排进正在跑的会话，也不因驻留会话运行而拒收） */
  | { action: 'new-tab'; direct: boolean }
  /** 目标就是当前激活会话（运行中也允许：排进同会话子队列等续发） */
  | { action: 'enqueue-active'; direct: boolean }
  /** 目标是驻留/历史会话 → 先激活再入队 */
  | { action: 'activate'; id: string; direct: boolean }
  /** 目标会话不存在 */
  | { action: 'missing'; id: string }
  /** 桌面端互斥操作进行中（新建/删除会话），此刻无法切换/入队 */
  | { action: 'blocked'; id: string | null };

/** 手机发来一条消息：决定发送路径（纯决策，执行在 stores/mobile.ts）
 *
 *  手机未指定会话（null）= 新对话意图：P3 并发下新会话可直接起跑（后端按会话
 *  串行、跨会话并行），一律 new-tab 直发，不再排进正在运行的会话。
 *
 *  指定会话时按会话各自判断（队列是每会话一条子队列，不是全局）：目标在跑
 *  → 入它自己的子队列等续发；目标空闲 → 直发（需要先激活）。唯一拒收是
 *  sessionBusy（新建/删除会话互斥中），其余情况消息都不会丢。
 *
 *  direct：目标空闲 → 手机消息跳过队列直接 agent_start_run；
 *  目标本身正在运行 → false，消息仍入队等既有续发。missing/blocked 不执行，无此字段。 */
export function planEnqueue(ctx: EnqueueContext, sessionId: string | null): EnqueuePlan {
  if (ctx.busy) return { action: 'blocked', id: sessionId };
  const flatRunning = ctx.runningIds.includes(ctx.activeId);
  if (sessionId === null) {
    // 新对话意图：消息属于新对话而不是正在跑的会话。空草稿直接复用当前会话；
    // 其余（包括激活/驻留会话运行中）一律新开 tab 直发（P3 并行起跑）
    if (!flatRunning && ctx.isDraft && ctx.entriesLen === 0) {
      return { action: 'reuse-current', direct: true };
    }
    return { action: 'new-tab', direct: true };
  }
  if (sessionId === ctx.activeId) {
    // 目标就是激活会话：只看它自己是否在跑（别的会话运行与它无关，P3 并发）
    return { action: 'enqueue-active', direct: !flatRunning };
  }
  if (!ctx.knownSessionIds.includes(sessionId)) return { action: 'missing', id: sessionId };
  // 目标会话自己正在跑：切到前台，消息排进它自己的子队列等续发
  if (ctx.runningIds.includes(sessionId)) return { action: 'activate', id: sessionId, direct: false };
  // 目标空闲：先激活再直发（别的会话在跑也不影响，P3 并发）
  return { action: 'activate', id: sessionId, direct: true };
}

// ===== 手机消息去重（重复投递 / 触摸双击防护） =====

export interface EnqueueDeduper {
  /** 返回 true = 应丢弃这次入队（重复投递或极短窗口内的同文本重发） */
  duplicate: (client: string, msgId: unknown, text: string, sessionId: string | null, now: number) => boolean;
}

/** 手机消息去重器（两层，均按客户端隔离）：
 *  1) client:msg_id 精确去重（LRU 128）——同一条 WS 消息被重复投递（监听器重复注册、
 *     事件系统重复派发等运行时形态）时只处理一次；
 *  2) 同客户端同会话同文本 1500ms 窗口去重——手机端触摸双击 / 回车+点击竞态发出
 *     两条不同 msg_id 但内容相同的消息时兜底。
 *  client 是桥分配的连接级 id：跨客户端（web 页 + 各手机 App）的 msg_id 各自从 1
 *  起步是常态，裸按 msg_id 判重会把他端消息误杀（真机实证：App 消息被静默丢弃）。
 *  只作用于手机入队路径；桌面手动入队不受影响。 */
export function createEnqueueDeduper(): EnqueueDeduper {
  const seenIds = new Map<string, number>();
  let lastClient = '';
  let lastText = '';
  let lastSession: string | null = null;
  let lastAt = -Infinity;
  return {
    duplicate(client, msgId, text, sessionId, now) {
      if (msgId !== null && msgId !== undefined) {
        const key = `${client}:${String(msgId)}`;
        if (seenIds.has(key)) {
          seenIds.delete(key);
          seenIds.set(key, now); // 触摸一次保 LRU 顺序
          return true;
        }
        seenIds.set(key, now);
        if (seenIds.size > 128) {
          const oldest = seenIds.keys().next().value;
          if (oldest !== undefined) seenIds.delete(oldest);
        }
      }
      if (text && client === lastClient && text === lastText && sessionId === lastSession && now - lastAt < 1500) {
        return true;
      }
      lastClient = client;
      lastText = text;
      lastSession = sessionId;
      lastAt = now;
      return false;
    },
  };
}

/** 队列快照投给桥之前剔除图片（手机端只展示文本；images 是重 base64） */
export function sanitizeQueueSnapshot<T extends { images?: unknown }>(
  queue: readonly T[],
): Array<Omit<T, 'images'>> {
  return queue.map((item) => {
    const clone = { ...item } as Record<string, unknown>;
    delete clone.images;
    return clone as Omit<T, 'images'>;
  });
}

/** 会话结算后是否需要把该会话换到前台以触发既有续发（≠激活会话 且 其子队列非空） */
export function shouldActivateAfterSettle(
  queue: Array<{ sessionId: string }>,
  settledId: string,
  activeId: string,
): boolean {
  if (settledId === activeId) return false;
  return queue.some((q) => q.sessionId === settledId);
}

// ===== 模型 / 思考 / 放权：手机端配置控制（与桌面 chatToolbar + agent store 同语义） =====

/** 校验手机发来的思考力度（THINKING_EFFORTS）：非法返回 null，桌面保持当前值 */
export function normalizeEffort(v: unknown): 'minimal' | 'low' | 'medium' | 'high' | null {
  return v === 'minimal' || v === 'low' || v === 'medium' || v === 'high' ? v : null;
}

/** 校验手机发来的放权级别（AUTO_MODES）：非法返回 null，桌面保持当前值 */
export function normalizeAutoMode(v: unknown): 'off' | 'project' | 'workspace' | 'all' | null {
  return v === 'off' || v === 'project' || v === 'workspace' || v === 'all' ? v : null;
}

export type ModelSelectionPlan =
  | { ok: true; kind: 'local'; port: number }
  | { ok: true; kind: 'provider'; providerId: string; model: string | null }
  | { ok: false; error: string };

/** 手机选择模型：本地来源必须命中 PC 运行中的端口；provider 来源必须命中已知提供者。
 *  provider + model 是两步流：先切 provider（不带 model，走其 active_model），
 *  桌面拉取该 provider 的模型列表重推 config 后，手机再选具体模型（带 model）。 */
export function planModelSelection(req: {
  source: unknown;
  port: unknown;
  provider_id: unknown;
  model: unknown;
  runningPorts: number[];
  providerIds: string[];
}): ModelSelectionPlan {
  const model = typeof req.model === 'string' && req.model.trim() ? req.model.trim() : null;
  if (req.source === 'model') {
    const port = typeof req.port === 'number' ? req.port : null;
    if (port === null || !req.runningPorts.includes(port)) {
      return { ok: false, error: '该本地模型未在 PC 上运行' };
    }
    return { ok: true, kind: 'local', port };
  }
  if (req.source === 'provider') {
    const providerId = typeof req.provider_id === 'string' ? req.provider_id : '';
    if (!providerId || !req.providerIds.includes(providerId)) {
      return { ok: false, error: '该 API 提供者不存在' };
    }
    return { ok: true, kind: 'provider', providerId, model };
  }
  return { ok: false, error: '未知的模型来源' };
}
