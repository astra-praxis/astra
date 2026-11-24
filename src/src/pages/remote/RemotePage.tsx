/**
 * 手机遥控页：桌面端开启局域网桥 → 出二维码 → 手机扫码即绑定。
 *
 * 手机端（浏览器打开，无需装 App）可以看到桌面全部会话与正在进行的对话，
 * 发消息会进入 PC 的任务队列排队续发，也可以调整队列顺序、
 * 代答权限与提问卡片。本页只管桥的生命周期（开启/停止/刷新）、
 * 临时配对二维码与持久设备 key（失效时间 / 单会话分享 / 撤销）。
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Check, Copy, Globe, KeyRound, Play, Plus, RefreshCw, Smartphone, Square, Trash2, WifiOff,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { ensureMobileBridge, useMobileBridgeStore, type BridgeKeyView, type CreatedKey } from '@/stores/mobile';
import type { AgentSessionMeta } from '@/pages/agent/AgentHistoryPanel';
import './RemotePage.scss';

const EXPIRY_OPTIONS = [
  { days: null, label: '永久有效' },
  { days: 7, label: '7 天' },
  { days: 30, label: '30 天' },
] as const;

function formatTime(ms: number | null): string {
  if (!ms) return '—';
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default function RemotePage() {
  const info = useMobileBridgeStore((s) => s.info);
  const running = useMobileBridgeStore((s) => s.running);
  const paired = useMobileBridgeStore((s) => s.paired);
  const starting = useMobileBridgeStore((s) => s.starting);
  const error = useMobileBridgeStore((s) => s.error);
  const keys = useMobileBridgeStore((s) => s.keys);
  const keysLoading = useMobileBridgeStore((s) => s.keysLoading);
  const start = useMobileBridgeStore((s) => s.start);
  const stop = useMobileBridgeStore((s) => s.stop);
  const refresh = useMobileBridgeStore((s) => s.refresh);
  const refreshKeys = useMobileBridgeStore((s) => s.refreshKeys);
  const createKey = useMobileBridgeStore((s) => s.createKey);
  const revokeKey = useMobileBridgeStore((s) => s.revokeKey);
  const relayOn = useMobileBridgeStore((s) => s.relayOn);
  const relayStarting = useMobileBridgeStore((s) => s.relayStarting);
  const relayStopping = useMobileBridgeStore((s) => s.relayStopping);
  const relayError = useMobileBridgeStore((s) => s.relayError);
  const relayStart = useMobileBridgeStore((s) => s.relayStart);
  const relayStop = useMobileBridgeStore((s) => s.relayStop);

  const [urlCopied, setUrlCopied] = useState(false);
  // 持久 key 表单
  const [keyName, setKeyName] = useState('');
  const [scopeSessionId, setScopeSessionId] = useState<string>('');
  const [expiresDays, setExpiresDays] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [formError, setFormError] = useState('');
  const [created, setCreated] = useState<CreatedKey | null>(null);
  const [createdCopied, setCreatedCopied] = useState(false);
  // 公网中继表单
  const [relayHost, setRelayHost] = useState('');
  const [relayPort, setRelayPort] = useState('21000');
  const [relayToken, setRelayToken] = useState('');
  // 会话列表（scope 单会话下拉）
  const [sessions, setSessions] = useState<AgentSessionMeta[]>([]);
  const [revokingId, setRevokingId] = useState('');

  useEffect(() => {
    void ensureMobileBridge();
    void refresh();
    void refreshKeys();
  }, [refresh, refreshKeys]);

  // 配对数变化由 pair 事件驱动；轮询兜底对账（如手机直接关页面）
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void refresh(), 8000);
    return () => window.clearInterval(timer);
  }, [running, refresh]);

  // scope 下拉数据：非归档会话按更新时间倒序（后端已是倒序，前端再保险）
  useEffect(() => {
    invoke<AgentSessionMeta[]>('list_agent_sessions')
      .then((list) => setSessions([...list].filter((s) => !s.archived).sort((a, b) => b.updated_at - a.updated_at)))
      .catch(() => {});
  }, []);

  const copyUrl = useCallback(async () => {
    if (!info?.url) return;
    try {
      await navigator.clipboard.writeText(info.url);
      setUrlCopied(true);
      window.setTimeout(() => setUrlCopied(false), 1500);
    } catch {
      // 剪贴板不可用时忽略
    }
  }, [info]);

  const copyCreatedUrl = useCallback(async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.url);
      setCreatedCopied(true);
      window.setTimeout(() => setCreatedCopied(false), 1500);
    } catch {
      // 忽略
    }
  }, [created]);

  const submitCreate = useCallback(async () => {
    setFormError('');
    if (!keyName.trim()) {
      setFormError('请填写设备名称，例如「我的 iPhone」');
      return;
    }
    setCreating(true);
    try {
      const c = await createKey({
        name: keyName,
        scopeSessionId: scopeSessionId || null,
        expiresDays,
      });
      setCreated(c);
      setKeyName('');
      setScopeSessionId('');
      setExpiresDays(null);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }, [keyName, scopeSessionId, expiresDays, createKey]);

  const submitRevoke = useCallback(async (k: BridgeKeyView) => {
    if (!window.confirm(`撤销「${k.name}」？该设备的绑定会立即失效并断开连接。`)) return;
    setRevokingId(k.id);
    try {
      await revokeKey(k.id);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setRevokingId('');
    }
  }, [revokeKey]);

  const activeKeys = useMemo(() => keys.filter((k) => !k.revoked_at), [keys]);
  const revokedKeys = useMemo(() => keys.filter((k) => k.revoked_at), [keys]);

  const submitRelayStart = useCallback(() => {
    const port = Number(relayPort) || 21000;
    void relayStart({ host: relayHost, controlPort: port, token: relayToken });
  }, [relayHost, relayPort, relayToken, relayStart]);

  return (
    <div className="remote-page">
      <header className="remote-page__head">
        <h2>
          <Smartphone size={18} /> 手机遥控
        </h2>
        <p>
          在同一局域网内用手机扫码（或手输地址）绑定这台桌面端：手机上可查看全部会话与
          正在进行的对话，发消息会进入 PC 的任务队列排队续发，也可以调整队列顺序、
          代答权限与提问卡片。需要长期绑定可创建下面的设备密钥（重启不失效）。
        </p>
      </header>

      {error && (
        <div className="remote-page__error" role="alert">
          {error}
        </div>
      )}

      {!running ? (
        <section className="remote-page__card remote-page__card--empty">
          <WifiOff size={28} strokeWidth={1.6} />
          <p>桥服务未开启，手机还无法连接。</p>
          <button type="button" className="remote-page__primary" disabled={starting} onClick={() => void start()}>
            <Play size={14} />
            {starting ? '开启中…' : '开启手机遥控'}
          </button>
        </section>
      ) : (
        <>
          <section className="remote-page__card remote-page__status">
            <span className={`remote-page__dot${paired > 0 ? ' is-on' : ''}`} />
            <span className="remote-page__state">
              {paired > 0 ? `已连接 ${paired} 台手机` : '等待手机扫码…'}
            </span>
            <span className="remote-page__lan">
              {info?.relay
                ? `中继 ${info.relay.host}:${info.relay.data_port}`
                : `局域网 ${info?.lan_ip} · 端口 ${info?.port}`}
            </span>
            <button type="button" className="remote-page__icon-btn" onClick={() => void refresh()} title="刷新状态">
              <RefreshCw size={14} />
            </button>
            <button type="button" className="remote-page__danger" onClick={() => void stop()}>
              <Square size={12} strokeWidth={3} />
              停止
            </button>
          </section>

          <section className="remote-page__card remote-page__qr">
            <div
              className="remote-page__qr-box"
              // qrcode crate 生成的可信 SVG（内容是本页入口地址）
              dangerouslySetInnerHTML={{ __html: info?.qr_svg ?? '' }}
              aria-label="手机扫码绑定入口二维码"
            />
            <div className="remote-page__qr-side">
              <h3>临时扫码绑定</h3>
              <ol className="remote-page__steps">
                <li>手机连接同一 Wi-Fi / 局域网</li>
                <li>相机或扫码工具识别左侧二维码</li>
                <li>在浏览器中打开，即可看到桌面会话</li>
              </ol>
              <div className="remote-page__url">
                <code className="remote-page__url-text">{info?.url}</code>
                <button type="button" className="remote-page__icon-btn" onClick={() => void copyUrl()} title="复制地址">
                  {urlCopied ? <Check size={14} /> : <Copy size={14} />}
                </button>
              </div>
              <p className="remote-page__hint">
                临时绑定重启后失效，需重新扫码。想长期绑定（可设失效时间、只分享单个会话、
                随时撤销）请用下方的设备密钥。
              </p>
            </div>
          </section>

          {/* ===== 公网中继 ===== */}
          <section className="remote-page__card remote-page__relay">
            <div className="remote-page__keys-head">
              <h3>
                <Globe size={15} /> 公网中继
                {info?.relay && <span className="remote-page__relay-badge">已开启</span>}
              </h3>
              {info?.relay && (
                <button
                  type="button"
                  className="remote-page__danger"
                  disabled={relayStopping}
                  onClick={() => void relayStop()}
                >
                  <Square size={12} strokeWidth={3} />
                  {relayStopping ? '停止中…' : '停止中继'}
                </button>
              )}
            </div>
            <p className="remote-page__hint remote-page__keys-intro">
              局域网直连不可用（手机不在同一网络）时，可在一台公网/可达服务器上部署
              astra-relay（见仓库 relay/README.md），桌面端会主动连出，手机从任何网络
              经中继接入。开启后上方二维码与设备密钥自动切换为中继入口。
            </p>
            {info?.relay ? (
              <p className="remote-page__hint">
                中继入口：{info.relay.host}:{info.relay.data_port}（数据端口随令牌固定，
                重连不变）。手机照常扫码即可，流量经服务器转发但端到端凭证验证不变。
              </p>
            ) : (
              <div className="remote-page__key-form remote-page__relay-form">
                <input
                  type="text"
                  className="remote-page__key-input remote-page__relay-host"
                  placeholder="服务器地址，如 relay.example.com"
                  value={relayHost}
                  onChange={(e) => setRelayHost(e.target.value)}
                />
                <input
                  type="number"
                  className="remote-page__key-input remote-page__relay-port"
                  placeholder="控制端口"
                  value={relayPort}
                  min={1}
                  max={65535}
                  onChange={(e) => setRelayPort(e.target.value)}
                />
                <input
                  type="password"
                  className="remote-page__key-input remote-page__relay-token"
                  placeholder="接入令牌（ASTRA_RELAY_TOKEN）"
                  value={relayToken}
                  autoComplete="off"
                  onChange={(e) => setRelayToken(e.target.value)}
                />
                <button
                  type="button"
                  className="remote-page__primary remote-page__key-add"
                  disabled={relayStarting || !relayHost.trim() || !relayToken.trim()}
                  onClick={submitRelayStart}
                >
                  <Play size={14} />
                  {relayStarting ? '连接中…' : '开启中继'}
                </button>
              </div>
            )}
            {relayError && <div className="remote-page__form-error" role="alert">{relayError}</div>}
          </section>

          {/* ===== 持久设备 key ===== */}
          <section className="remote-page__card remote-page__keys">
            <div className="remote-page__keys-head">
              <h3><KeyRound size={15} /> 设备密钥</h3>
              <button
                type="button"
                className="remote-page__icon-btn"
                onClick={() => void refreshKeys()}
                title="刷新列表"
              >
                <RefreshCw size={14} />
              </button>
            </div>
            <p className="remote-page__hint remote-page__keys-intro">
              类似 API key：secret 只在创建时的二维码里出现一次，桌面端只存指纹；
              丢失即撤销重建。可限定只授权一个会话，也可随时手动撤销。
            </p>

            <div className="remote-page__key-form">
              <input
                type="text"
                className="remote-page__key-input"
                placeholder="设备名称，如「我的 iPhone」"
                value={keyName}
                maxLength={30}
                onChange={(e) => setKeyName(e.target.value)}
              />
              <select
                className="remote-page__key-select"
                value={scopeSessionId}
                onChange={(e) => setScopeSessionId(e.target.value)}
                title="授权范围"
              >
                <option value="">全部会话</option>
                {sessions.map((s) => (
                  <option key={s.id} value={s.id}>
                    仅「{s.title || '未命名会话'}」
                  </option>
                ))}
              </select>
              <select
                className="remote-page__key-select remote-page__key-select--expiry"
                value={expiresDays === null ? 'never' : String(expiresDays)}
                onChange={(e) => setExpiresDays(e.target.value === 'never' ? null : Number(e.target.value))}
                title="有效期"
              >
                {EXPIRY_OPTIONS.map((o) => (
                  <option key={o.label} value={o.days === null ? 'never' : String(o.days)}>
                    {o.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="remote-page__primary remote-page__key-add"
                disabled={creating}
                onClick={() => void submitCreate()}
              >
                <Plus size={14} />
                {creating ? '创建中…' : '创建'}
              </button>
            </div>
            {formError && <div className="remote-page__form-error" role="alert">{formError}</div>}

            {created && (
              <div className="remote-page__key-created">
                <div className="remote-page__key-created-body">
                  <strong>「{created.name}」的绑定二维码（只显示这一次）</strong>
                  <span className="remote-page__hint">
                    {created.scope.kind === 'session'
                      ? '仅授权单个会话。'
                      : '授权全部会话。'}
                    {' '}secret 不落盘，关闭后无法找回；保存或直接扫码。
                  </span>
                  <div className="remote-page__url">
                    <code className="remote-page__url-text">{created.url}</code>
                    <button type="button" className="remote-page__icon-btn" onClick={() => void copyCreatedUrl()} title="复制地址">
                      {createdCopied ? <Check size={14} /> : <Copy size={14} />}
                    </button>
                  </div>
                </div>
                <div
                  className="remote-page__qr-box remote-page__qr-box--sm"
                  dangerouslySetInnerHTML={{ __html: created.qr_svg }}
                  aria-label="设备密钥绑定二维码"
                />
                <button type="button" className="remote-page__icon-btn remote-page__key-created-close" onClick={() => setCreated(null)} title="关闭">
                  <Check size={16} />
                </button>
              </div>
            )}

            <div className="remote-page__key-list">
              {activeKeys.length === 0 && !keysLoading && (
                <p className="remote-page__hint">还没有设备密钥。上面的临时扫码每次重启都要重扫；密钥不会。</p>
              )}
              {activeKeys.map((k) => (
                <div key={k.id} className="remote-page__key-row">
                  <div className="remote-page__key-info">
                    <span className="remote-page__key-name">{k.name}</span>
                    <span className="remote-page__key-meta">
                      {k.scope_tag === 'one'
                        ? `单会话 · ${sessions.find((s) => s.id === k.scope.session_id)?.title || (k.scope.session_id || '').slice(0, 8)}`
                        : '全部会话'}
                      {' · '}
                      {k.expires_at ? `有效期至 ${formatTime(k.expires_at)}` : '永久'}
                      {k.last_used_at ? ` · 最近使用 ${formatTime(k.last_used_at)}` : ' · 从未使用'}
                    </span>
                  </div>
                  <button
                    type="button"
                    className="remote-page__danger remote-page__key-revoke"
                    disabled={revokingId === k.id}
                    onClick={() => void submitRevoke(k)}
                  >
                    <Trash2 size={12} />
                    {revokingId === k.id ? '撤销中…' : '撤销'}
                  </button>
                </div>
              ))}
              {revokedKeys.map((k) => (
                <div key={k.id} className="remote-page__key-row remote-page__key-row--revoked">
                  <div className="remote-page__key-info">
                    <span className="remote-page__key-name">{k.name}</span>
                    <span className="remote-page__key-meta">已撤销 · {formatTime(k.revoked_at)}</span>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
