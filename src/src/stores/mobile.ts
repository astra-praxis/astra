/**
 * 手机遥控桥（前端转接层）。
 *
 * mobile_bridge.rs 在局域网起 HTTP+WS 服务（扫码即绑定）；对话真相仍在 agent store，
 * 队列真相也是（Zed 式排队 + 既有续发 effect）。本模块只做四件事：
 * 1. 把手机的 WS 指令（发送 / 排序 / 删除）翻译成 agent store 的既有动作
 *    （目标空闲 → 直接起跑；目标在跑 → 入队等既有续发）
 * 2. 把桌面队列变化镜像给桥（hello 快照 + 增量广播），配对数同步展示
 * 3. 驻留会话 Run 结算且仍有排队时，把它换到前台，让 AgentPage 既有续发 effect 接手
 * 4. 把桌面配置（模型来源 / 思考 / 放权）镜像给桥，并接收手机的配置修改
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import { useAgentStore, ensureAgentListeners } from '@/stores/agent';
import { useChatToolbarStore } from './chatToolbar';
import type { AgentSessionMeta } from '@/pages/agent/AgentHistoryPanel';
import type { EnqueuePlan, MobileSkillLite } from './mobileLogic';
import {
  normalizeAutoMode,
  normalizeEffort,
  parseMobileSend,
  planEnqueue,
  planModelSelection,
  sanitizeQueueSnapshot,
  shouldActivateAfterSettle,
  createEnqueueDeduper,
} from './mobileLogic';

export interface MobileBridgeInfo {
  running: boolean;
  port: number;
  /** 手机浏览器打开的入口地址（token 在 fragment 里） */
  url: string;
  token: string;
  /** 入口地址的二维码（SVG 字符串） */
  qr_svg: string;
  lan_ip: string;
  /** 中继模式：非 null = 入口经公网 relay（url/qr 已指向中继） */
  relay: { host: string; control_port: number; data_port: number } | null;
}

interface BridgeStatus {
  running: boolean;
  port: number | null;
  paired: number;
  lan_ip: string;
  relay: { host: string; control_port: number; data_port: number } | null;
}

interface MobileBridgeState {
  /** 最近一次启动返回的入口信息（null = 未运行） */
  info: MobileBridgeInfo | null;
  running: boolean;
  /** 已配对（WS 在线）的手机数 */
  paired: number;
  starting: boolean;
  error: string;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  /** 对账：刷新运行态与配对数；桥在跑但前端丢了二维码（webview 重载）→ 重开换新 QR */
  refresh: () => Promise<void>;
  /** 持久设备 key 列表 */
  keys: BridgeKeyView[];
  keysLoading: boolean;
  /** 创建持久设备 key（二维码只出现这一次） */
  createKey: (opts: { name: string; scopeSessionId?: string | null; expiresDays?: number | null }) => Promise<CreatedKey>;
  /** 撤销设备 key（返回踢掉的在线连接数） */
  revokeKey: (id: string) => Promise<number>;
  refreshKeys: () => Promise<void>;
  /** 公网中继：桌面端主动连出 relay 服务器，手机从任意网络接入（不再限局域网） */
  relayOn: boolean;
  relayStarting: boolean;
  relayStopping: boolean;
  relayError: string;
  relayStart: (opts: { host: string; controlPort: number; token: string }) => Promise<void>;
  relayStop: () => Promise<void>;
}

/** 设备 key 列表视图（Rust to_view：不含 hash/secret） */
export interface BridgeKeyView {
  id: string;
  name: string;
  scope: { kind: 'all' | 'session'; session_id?: string };
  scope_tag: 'all' | 'one';
  created_at: number;
  expires_at: number | null;
  revoked_at: number | null;
  last_used_at: number | null;
  last_ip: string | null;
}

/** 创建返回（secret 只出现一次，QR 立即展示） */
export interface CreatedKey {
  id: string;
  secret: string;
  url: string;
  qr_svg: string;
  name: string;
  scope: { kind: 'all' | 'session'; session_id?: string };
}

export const useMobileBridgeStore = create<MobileBridgeState>((set, get) => ({
  info: null,
  running: false,
  paired: 0,
  starting: false,
  error: '',
  keys: [],
  keysLoading: false,
  start: async () => {
    if (get().starting || get().running) return;
    set({ starting: true, error: '' });
    try {
      const info = await invoke<MobileBridgeInfo>('mobile_bridge_start');
      set({ info, running: true, starting: false });
      // 重启 = 新 token = 桥侧空镜像；立即推一次让 hello 快照有数据
      pushQueueSnapshot();
      pushConfig();
      // view 镜像同理：桥重启后 current 丢失，强制重推当前查看的会话。
      // draft 会话磁盘上不存在，不推（手机停在「新对话」等真实会话出现）
      lastPushedView = undefined;
      const ag = useAgentStore.getState();
      if (ag.sessionId && !ag.isDraft) pushView(ag.sessionId);
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e), starting: false });
    }
  },
  stop: async () => {
    try {
      await invoke('mobile_bridge_stop');
      set({ error: '' });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
    set({ info: null, running: false, paired: 0 });
  },
  refresh: async () => {
    try {
      const status = await invoke<BridgeStatus>('mobile_bridge_status');
      if (status.running && !get().info) {
        // Rust 侧在运行但前端没有二维码（如 dev 模式 webview 重载）：
        // 旧 token 只存在于丢失的前端状态里，重开服务换新 QR 才能继续配对
        await get().stop();
        await get().start();
        return;
      }
      set({ running: status.running, paired: status.running ? status.paired : 0, relayOn: status.relay != null });
    } catch {
      // 后端不可用（本地调试）时忽略
    }
  },
  createKey: async ({ name, scopeSessionId, expiresDays }) => {
    const created = await invoke<CreatedKey>('mobile_bridge_key_create', {
      name,
      scopeSessionId: scopeSessionId || null,
      expiresDays: expiresDays ?? null,
    });
    void get().refreshKeys();
    return created;
  },
  revokeKey: async (id) => {
    const kicked = await invoke<number>('mobile_bridge_key_revoke', { keyId: id });
    void get().refreshKeys();
    return kicked;
  },
  refreshKeys: async () => {
    set({ keysLoading: true });
    try {
      const keys = await invoke<BridgeKeyView[]>('mobile_bridge_key_list');
      set({ keys, keysLoading: false });
    } catch {
      set({ keysLoading: false });
    }
  },
  relayOn: false,
  relayStarting: false,
  relayStopping: false,
  relayError: '',
  relayStart: async ({ host, controlPort, token }) => {
    if (get().relayStarting || get().relayOn) return;
    set({ relayStarting: true, relayError: '' });
    try {
      const info = await invoke<MobileBridgeInfo>('mobile_bridge_relay_start', {
        host,
        controlPort,
        token,
      });
      // 成功后 info.url/qr 已切换为中继入口（桥没开会自动开）
      set({ info, running: true, relayOn: true, relayStarting: false });
    } catch (e) {
      set({
        relayError: e instanceof Error ? e.message : String(e),
        relayStarting: false,
      });
    }
  },
  relayStop: async () => {
    set({ relayStopping: true, relayError: '' });
    try {
      await invoke('mobile_bridge_relay_stop');
      // 桥仍在跑，重新拿 LAN 入口（mobile_bridge_start 已在跑则原样返回）
      const info = await invoke<MobileBridgeInfo>('mobile_bridge_start');
      set({ info, relayOn: false, relayStopping: false });
    } catch (e) {
      set({
        relayError: e instanceof Error ? e.message : String(e),
        relayStopping: false,
      });
    }
  },
}));

// ===== 手机指令 → agent store =====

/** 给手机端弹 toast（复用桥的广播通道） */
function phoneToast(text: string) {
  invoke('mobile_bridge_server_event', { value: { type: 'toast', text } }).catch(() => {});
}

/** 手机入队消息去重器（模块级单例：跨会话、跨页面生命周期生效） */
const enqueueDeduper = createEnqueueDeduper();

/** 驻留/历史会话兜底 meta：会话刚结算必有驻留快照，meta 仅是 activateSession 的形参 */
function stubMeta(id: string): AgentSessionMeta {
  return {
    id,
    project_path: '',
    project_name: '',
    title: '',
    entries: [],
    created_at: 0,
    updated_at: 0,
    archived: false,
  };
}

let skillsCache: MobileSkillLite[] | null = null;
async function knownSkills(): Promise<MobileSkillLite[]> {
  if (skillsCache) return skillsCache;
  try {
    const list = await invoke<Array<{ name: string; description: string }>>('list_user_skills');
    skillsCache = list.map((s) => ({ name: s.name, description: s.description }));
  } catch {
    skillsCache = [];
  }
  return skillsCache;
}

// ===== 当前查看会话镜像（hello 的 current / view 广播） =====

let lastPushedView: string | undefined;

function pushView(sessionId: string) {
  if (lastPushedView === sessionId) return;
  lastPushedView = sessionId;
  invoke('mobile_bridge_push_view', { sessionId }).catch(() => {});
}

function applyPlan(plan: EnqueuePlan): boolean {
  const st = useAgentStore.getState();
  switch (plan.action) {
    case 'reuse-current':
    case 'enqueue-active':
      return true;
    case 'new-tab': {
      st.newTab();
      // newTab 在 sessionBusy 下静默 no-op（plan 阶段已排除，这里兜底防竞态）；
      // 没切过去就绝不入队，否则消息进错会话
      if (useAgentStore.getState().sessionId === st.sessionId && !(st.isDraft && st.entries.length === 0)) {
        phoneToast('桌面端正在运行其他会话，暂时无法切换，请稍后再试');
        return false;
      }
      return true;
    }
    case 'activate': {
      const meta = st.sessions.find((s) => s.id === plan.id) ?? stubMeta(plan.id);
      st.activateSession(meta);
      // activateSession 在 sessionBusy 下静默 no-op：换失败绝不入队，否则消息进错会话
      if (useAgentStore.getState().sessionId !== plan.id) {
        phoneToast('桌面端正在运行其他会话，暂时无法切换，请稍后再试');
        return false;
      }
      return true;
    }
    case 'missing':
      phoneToast('会话不存在');
      return false;
    case 'blocked':
      phoneToast('桌面端正在运行其他会话，暂时无法切换，请稍后再试');
      return false;
  }
}

/** 手机直发结果：
 *  - 'sent'：已发出（pushUser + agent_start_run 成功）
 *  - 'queued'：守卫拦住（准入被占 / 会话被切走）→ 未动对话，调用方兜底入队
 *  - 'failed'：确定性失败。校验类失败发生在 pushUser 之前（气泡未进对话，入队也必失败）；
 *    invoke 失败发生在 pushUser 之后（气泡已进对话，再入队会导致同一消息 pushUser
 *    两次）——两种都不兜底入队 */
type DirectSendOutcome = 'sent' | 'queued' | 'failed';

/** 用户消息即时镜像：任一终端按下发送（pushUser）即广播 user_message 给所有手机，
 *  不等 Run 准入（checkpoint / MCP 装配等启动开销可达数秒，原先手机端气泡几乎与
 *  agent 回复同时出现）。载荷与原 run.rs 准入广播一致：text 是剥过 /技能 前缀的
 *  任务文本（手机端「新对话」收养的文本匹配依赖它）；sender 缺省 = 桌面本机发送 */
export function broadcastUserMessage(sessionId: string, text: string, sender?: string): void {
  invoke('mobile_bridge_push_event', {
    kind: 'agent-event',
    payload: {
      type: 'user_message',
      session_id: sessionId,
      text,
      ...(sender ? { sender } : {}),
    },
  }).catch(() => {});
}

/** 手机直发（目标会话空闲时跳过队列）：复刻 AgentPage.startRun 的核心序
 *  （校验 → 监听 → 守卫 → 清横幅 → pushUser → setRunState → invoke）。
 *  与桌面手动发送的等价性：不清输入框草稿（等价 keepInput）、清 runError/error、
 *  invoke 失败回滚运行态。重试机制（lastRunRef / 自动重试）是页面组件状态，不碰。 */
async function directSend(
  raw: string,
  text: string,
  skill: string | null,
  expectedId: string,
  sender?: string,
): Promise<DirectSendOutcome> {
  const toolbar = useChatToolbarStore.getState();
  const invalid = (msg: string): DirectSendOutcome => {
    useAgentStore.getState().setError(msg);
    phoneToast(msg);
    return 'failed';
  };
  // 工具栏校验（与 startRun 同语义）：确定性失败，入队也必失败 → 只提示不入队
  if (toolbar.sourceType === 'model' && !toolbar.selectedModelPort) {
    return invalid('请先在标题栏选择一个运行中的本地模型');
  }
  if (toolbar.sourceType === 'provider') {
    const provider = toolbar.providers.find((p) => p.id === toolbar.selectedProviderId) ?? null;
    const name = toolbar.overrideModelName ?? provider?.active_model ?? null;
    if (!provider) return invalid('请先在输入框下方选择一个 API 提供者');
    if (!name) return invalid('请先在输入框下方选择模型');
  }
  try {
    await ensureAgentListeners();
  } catch (e) {
    return invalid(`事件监听失败：${String(e)}`);
  }
  // 后置守卫（plan 阶段是前置守卫）：applyPlan 之后状态可能已变
  const st = useAgentStore.getState();
  if (st.running || st.settling || st.sessionBusy || st.sessionId !== expectedId) return 'queued';
  // 新消息 = 新意图接管：清旧失败横幅与输入框错误；绝不清 input（等价 keepInput）
  st.setRunError(null);
  st.setError('');
  st.pushUser(raw, undefined, sender); // 会话记录保留原始 /技能名 文本，回放时可读；sender 供多设备共享时展示来源
  broadcastUserMessage(st.sessionId, text, sender); // 按下发送即镜像给所有手机，不等 Run 准入
  st.setRunState(true, 'analyzing');
  const provider = toolbar.providers.find((p) => p.id === toolbar.selectedProviderId) ?? null;
  try {
    await invoke('agent_start_run', {
      req: {
        session_id: st.sessionId,
        user_text: text,
        images: [],
        skill,
        provider_id: toolbar.sourceType === 'provider' ? toolbar.selectedProviderId : null,
        model_port: toolbar.sourceType === 'model' ? toolbar.selectedModelPort : null,
        model:
          toolbar.sourceType === 'provider'
            ? (toolbar.overrideModelName ?? provider?.active_model ?? null)
            : (toolbar.runningModels.find((m) => m.port === toolbar.selectedModelPort)?.name ?? null),
        project_root: st.projectPath || null,
        system_prompt: toolbar.systemPrompt.trim() || null,
        auto_compact_tokens: st.autoCompactTokens,
        image_recognition: st.imageRecognition,
        thinking_enabled: st.thinkingEnabled,
        thinking_effort: st.thinkingEffort,
      },
    });
    return 'sent';
  } catch (e) {
    useAgentStore.getState().setRunState(false, null);
    invalid(`发送失败：${String(e)}`);
    return 'failed';
  }
}

async function handleEnqueue(payload: { session_id: string | null; text: string; sender?: string }) {
  const parsed = parseMobileSend(String(payload.text ?? ''), await knownSkills());
  if (!parsed.ok) {
    phoneToast(parsed.error);
    return;
  }
  let st = useAgentStore.getState();
  const target = payload.session_id ?? null;
  // 目标不在列表里先刷新一次（手机看到的列表可能比桌面旧）
  if (target !== null && !st.sessions.some((s) => s.id === target)) {
    await st.refreshSessions();
    st = useAgentStore.getState();
  }
  const plan = planEnqueue(
    {
      activeId: st.sessionId,
      isDraft: st.isDraft,
      entriesLen: st.entries.length,
      busy: st.sessionBusy,
      knownSessionIds: st.sessions.map((s) => s.id),
      // S0 单活动 Run：flat 运行中 + 驻留运行/结算中的会话都占着全局准入
      runningIds: [
        ...(st.running ? [st.sessionId] : []),
        ...Object.entries(st.parked)
          .filter(([, p]) => p.running || p.settling)
          .map(([id]) => id),
      ],
    },
    target,
  );
  if (!applyPlan(plan)) return;
  if (plan.action === 'missing' || plan.action === 'blocked') return;
  if (plan.direct) {
    // 空闲直发：跳过队列直接起跑（消息不在队列面板闪现）
    const expectedId = useAgentStore.getState().sessionId;
    const outcome = await directSend(parsed.raw, parsed.text, parsed.skill, expectedId, payload.sender);
    if (outcome === 'sent') phoneToast('已发送');
    if (outcome === 'queued') {
      // PC 此刻忙 / 会话被切走：兜底入队（准入拒绝不丢消息），续发 effect 接手
      useAgentStore.getState().enqueueMessage(parsed.raw, parsed.text, parsed.skill, { sender: payload.sender });
      phoneToast('PC 正忙，已加入队列');
    }
    // 'failed'：提示已在 directSend 内弹出，不入队（见 DirectSendOutcome 注释）
    return;
  }
  // 目标正在运行：入队等本轮结束后的既有续发
  useAgentStore.getState().enqueueMessage(parsed.raw, parsed.text, parsed.skill, { sender: payload.sender });
  phoneToast('已加入队列，本轮结束后自动发送');
}

// ===== 队列镜像 → 桥 =====

let pushTimer: ReturnType<typeof setTimeout> | null = null;

function pushQueueSnapshot() {
  if (pushTimer) {
    clearTimeout(pushTimer);
    pushTimer = null;
  }
  const items = sanitizeQueueSnapshot(useAgentStore.getState().queue);
  invoke('mobile_bridge_push_queue', { items }).catch(() => {});
}

/** 队列高频抖动（连续入队/排序）合并为一次快照推送 */
function scheduleQueuePush() {
  if (pushTimer) return;
  pushTimer = setTimeout(() => {
    pushTimer = null;
    pushQueueSnapshot();
  }, 80);
}

// ===== 配置镜像（模型 / 思考 / 放权）→ 桥 =====

interface MobileConfig {
  source_type: 'model' | 'provider';
  model_port: number | null;
  override_model: string | null;
  provider_id: string | null;
  providers: Array<{ id: string; name: string; active_model: string | null }>;
  provider_models: Array<{ id: string; name: string }>;
  running_models: Array<{ port: number; label: string }>;
  thinking_enabled: boolean;
  thinking_effort: string;
  auto_mode: string;
}

/** 桌面当前配置快照。队列续发在 dispatch 时才读工具栏 store，所以手机发的消息
 *  天然用 PC 当前配置——这里只负责让手机看得见、改得动这套配置 */
function buildConfigPayload(): { type: 'config'; config: MobileConfig } {
  const tb = useChatToolbarStore.getState();
  const ag = useAgentStore.getState();
  return {
    type: 'config',
    config: {
      source_type: tb.sourceType,
      model_port: tb.selectedModelPort,
      override_model: tb.overrideModelName,
      provider_id: tb.selectedProviderId,
      providers: tb.providers.map((p) => ({ id: p.id, name: p.name, active_model: p.active_model })),
      provider_models: tb.providerModels.map((m) => ({ id: m.id, name: m.name ?? m.id })),
      running_models: tb.runningModels.map((m) => ({
        port: m.port,
        label: m.model_name || m.name || `端口 ${m.port}`,
      })),
      thinking_enabled: ag.thinkingEnabled,
      thinking_effort: ag.thinkingEffort,
      auto_mode: ag.autoMode,
    },
  };
}

let configTimer: ReturnType<typeof setTimeout> | null = null;

function pushConfig() {
  if (configTimer) {
    clearTimeout(configTimer);
    configTimer = null;
  }
  invoke('mobile_bridge_server_event', { value: buildConfigPayload() }).catch(() => {});
}

/** 配置变化低频（切模型/开关），120ms 合并连发的字段赋值 */
function scheduleConfigPush() {
  if (configTimer) return;
  configTimer = setTimeout(() => {
    configTimer = null;
    pushConfig();
  }, 120);
}

// ===== 桥事件 =====

interface BridgeEvent {
  type: string;
  count?: number;
  msg_id?: number;
  session_id?: string | null;
  text?: string;
  skill?: string | null;
  /** send 载荷的发送者设备名（多设备共享时标记来源） */
  sender?: string;
  id?: string;
  target_id?: string;
  after?: boolean;
  /** set_model / set_thinking / set_auto_mode 载荷（rust 侧原样透传，类型未知） */
  source?: unknown;
  port?: unknown;
  provider_id?: unknown;
  model?: unknown;
  enabled?: unknown;
  effort?: unknown;
  mode?: unknown;
}

async function handleBridgeEvent(ev: BridgeEvent) {
  switch (ev.type) {
    case 'enqueue':
      // 双层去重（msg_id LRU + 同会话同文本短窗口）：WS 消息重复投递或手机端
      // 触摸双击时只入队一次，杜绝「直接发送 + 队列再续发一遍」的二连跑
      if (enqueueDeduper.duplicate(ev.msg_id, String(ev.text ?? ''), ev.session_id ?? null, Date.now())) break;
      await handleEnqueue({
        session_id: ev.session_id ?? null,
        text: String(ev.text ?? ''),
        sender: ev.sender,
      });
      break;
    case 'queue_reorder':
      // 只允许同会话内排序的校验在 reorderQueued 里（跨会话/未知 id 一律不动）
      useAgentStore.getState().reorderQueued(String(ev.id ?? ''), String(ev.target_id ?? ''), Boolean(ev.after));
      break;
    case 'queue_remove':
      useAgentStore.getState().removeQueued(String(ev.id ?? ''));
      break;
    case 'set_model': {
      const plan = planModelSelection({
        source: ev.source,
        port: ev.port,
        provider_id: ev.provider_id,
        model: ev.model,
        runningPorts: useChatToolbarStore.getState().runningModels.map((m) => m.port),
        providerIds: useChatToolbarStore.getState().providers.map((p) => p.id),
      });
      if (!plan.ok) {
        phoneToast(plan.error);
        break;
      }
      if (plan.kind === 'local') {
        // 直设而非 setter：避免 fetchProviderModels 的 fallback 覆盖选择（本地来源不涉及该异步）
        useChatToolbarStore.setState({ sourceType: 'model', selectedModelPort: plan.port });
        phoneToast('已切换本地模型');
      } else {
        // 两步流：先切 provider（override 同步带过），模型列表拉完后 config 重推，
        // 手机端再选具体模型。直设绕开 selectProvider 的同步竞态
        useChatToolbarStore.setState({
          sourceType: 'provider',
          selectedProviderId: plan.providerId,
          overrideModelName: plan.model,
        });
        void useChatToolbarStore.getState().fetchProviderModels();
        phoneToast(plan.model ? '已切换模型' : '已切换提供者');
      }
      scheduleConfigPush();
      break;
    }
    case 'set_thinking': {
      const effort = normalizeEffort(ev.effort);
      const ag = useAgentStore.getState();
      ag.setThinkingEnabled(Boolean(ev.enabled));
      if (effort) ag.setThinkingEffort(effort);
      scheduleConfigPush();
      break;
    }
    case 'set_auto_mode': {
      const mode = normalizeAutoMode(ev.mode);
      if (!mode) {
        phoneToast('未知的放权级别');
        break;
      }
      useAgentStore.getState().setAutoMode(mode);
      scheduleConfigPush();
      break;
    }
    case 'pair':
      useMobileBridgeStore.setState({ paired: Number(ev.count ?? 0) });
      // 新手机连上：桥侧镜像可能是空的（重启后），推一次保证其界面同步
      scheduleQueuePush();
      scheduleConfigPush();
      break;
    default:
      break;
  }
}

// ===== 模块级单例监听（切页不丢；AgentPage 之外也要接手机指令） =====

let bridgePromise: Promise<void> | undefined;

/** 应用布局 mount 时调用；发送/接收前无需 await（监听注册成功即生效）。失败可重试。 */
export function ensureMobileBridge(): Promise<void> {
  if (bridgePromise) return bridgePromise;
  bridgePromise = (async () => {
    let ready = false;
    const results = await Promise.allSettled([
      Promise.resolve().then(() => listen<BridgeEvent>('mobile-bridge-event', (e) => {
        if (ready) void handleBridgeEvent(e.payload);
      })),
      Promise.resolve().then(() => listen<{ session_id: string }>('agent-run-settled', (e) => {
        if (!ready) return;
        const st = useAgentStore.getState();
        // 激活会话的结算由 AgentPage 既有续发 effect 处理；驻留会话在这里拉回前台
        if (shouldActivateAfterSettle(st.queue, e.payload.session_id, st.sessionId)) {
          const meta =
            st.sessions.find((s) => s.id === e.payload.session_id) ?? stubMeta(e.payload.session_id);
          st.activateSession(meta);
        }
      })),
      // 队列变化（入队/排序/删除/续发摘除）→ 去抖推快照给桥
      Promise.resolve().then(() => useAgentStore.subscribe((s, prev) => {
        if (s.queue !== prev.queue) scheduleQueuePush();
      })),
      // 模型/提供者选择变化 → 重推配置镜像
      Promise.resolve().then(() => useChatToolbarStore.subscribe((s, prev) => {
        if (
          s.sourceType !== prev.sourceType ||
          s.selectedProviderId !== prev.selectedProviderId ||
          s.selectedModelPort !== prev.selectedModelPort ||
          s.overrideModelName !== prev.overrideModelName ||
          s.providers !== prev.providers ||
          s.providerModels !== prev.providerModels ||
          s.runningModels !== prev.runningModels
        ) {
          scheduleConfigPush();
        }
      })),
      // 思考 / 放权变化 → 重推配置镜像
      Promise.resolve().then(() => useAgentStore.subscribe((s, prev) => {
        if (
          s.thinkingEnabled !== prev.thinkingEnabled ||
          s.thinkingEffort !== prev.thinkingEffort ||
          s.autoMode !== prev.autoMode
        ) {
          scheduleConfigPush();
        }
      })),
      // 当前查看会话变化 → 推 view 给手机（停在「新对话」的手机跟进收养）。
      // draft 会话磁盘上不存在，不推；等它起跑转正（isDraft 翻转）再推
      Promise.resolve().then(() => useAgentStore.subscribe((s, prev) => {
        if ((s.sessionId !== prev.sessionId || s.isDraft !== prev.isDraft) && !s.isDraft) {
          pushView(s.sessionId);
        }
      })),
    ]);
    const errors: unknown[] = [];
    for (const result of results) {
      if (result.status === 'rejected') errors.push(result.reason);
    }
    if (errors.length) {
      for (const result of results) {
        if (result.status === 'fulfilled') {
          try { result.value(); } catch (error) { errors.push(error); }
        }
      }
      throw new AggregateError(errors, '手机遥控桥监听注册失败');
    }
    ready = true;
  })().catch((error: unknown) => {
    bridgePromise = undefined;
    throw error;
  });
  return bridgePromise;
}
