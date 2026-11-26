/**
 * 设置 · 设备与中继：持久设备 key 管理（创建 / 撤销 / 删除）与公网中继配置。
 * 从手机遥控页（RemotePage）整体迁入：桥的生命周期与配对二维码留在遥控页，
 * 长期绑定的密钥与中继属于「配置」，归设置中心。
 * 样式复用 RemotePage.scss（类名沿用 remote-page__ 前缀，避免样式双份漂移）。
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Check, Copy, Globe, KeyRound, Plus, RefreshCw, Square, Trash2, Play,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useMobileBridgeStore, type BridgeKeyView, type CreatedKey } from '@/stores/mobile';
import type { AgentSessionMeta } from '@/pages/agent/AgentHistoryPanel';
import '../../remote/RemotePage.scss';

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

export default function RemoteSettings() {
  const info = useMobileBridgeStore((s) => s.info);
  const keys = useMobileBridgeStore((s) => s.keys);
  const keysLoading = useMobileBridgeStore((s) => s.keysLoading);
  const refreshKeys = useMobileBridgeStore((s) => s.refreshKeys);
  const createKey = useMobileBridgeStore((s) => s.createKey);
  const revokeKey = useMobileBridgeStore((s) => s.revokeKey);
  const deleteKey = useMobileBridgeStore((s) => s.deleteKey);
  const relayStarting = useMobileBridgeStore((s) => s.relayStarting);
  const relayStopping = useMobileBridgeStore((s) => s.relayStopping);
  const relayError = useMobileBridgeStore((s) => s.relayError);
  const relayStart = useMobileBridgeStore((s) => s.relayStart);
  const relayStop = useMobileBridgeStore((s) => s.relayStop);
  const relayConfig = useMobileBridgeStore((s) => s.relayConfig);
  const relayConfigLoaded = useMobileBridgeStore((s) => s.relayConfigLoaded);
  const relayConfigLoad = useMobileBridgeStore((s) => s.relayConfigLoad);

  // 持久 key 表单
  const [keyName, setKeyName] = useState('');
  const [scopeSessionId, setScopeSessionId] = useState('');
  const [expiresDays, setExpiresDays] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [formError, setFormError] = useState('');
  const [created, setCreated] = useState<CreatedKey | null>(null);
  const [createdCopied, setCreatedCopied] = useState(false);
  // 公网中继表单
  const [relayHost, setRelayHost] = useState('');
  const [relayPort, setRelayPort] = useState('21000');
  const [relayToken, setRelayToken] = useState('');
  const [relayTls, setRelayTls] = useState(false);
  // 会话列表（scope 单会话下拉）
  const [sessions, setSessions] = useState<AgentSessionMeta[]>([]);
  const [revokingId, setRevokingId] = useState('');
  const [deletingId, setDeletingId] = useState('');

  useEffect(() => {
    void refreshKeys();
    void relayConfigLoad();
  }, [refreshKeys, relayConfigLoad]);

  // 已入库的中继配置回填表单（仅当表单还是初始空值时，避免覆盖正在输入的内容）
  useEffect(() => {
    if (!relayConfigLoaded || !relayConfig) return;
    setRelayHost((h) => h || relayConfig.host);
    setRelayPort((p) => (p === '21000' ? String(relayConfig.control_port) : p));
    setRelayToken((t) => t || relayConfig.token);
    setRelayTls(relayConfig.tls);
  }, [relayConfigLoaded, relayConfig]);

  // scope 下拉数据：非归档会话按更新时间倒序（后端已是倒序，前端再保险）
  useEffect(() => {
    invoke<AgentSessionMeta[]>('list_agent_sessions')
      .then((list) => setSessions([...list].filter((s) => !s.archived).sort((a, b) => b.updated_at - a.updated_at)))
      .catch(() => {});
  }, []);

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

  const submitDelete = useCallback(async (k: BridgeKeyView) => {
    if (!window.confirm(`删除已撤销的「${k.name}」？仅清理记录，不影响任何在用设备。`)) return;
    setDeletingId(k.id);
    try {
      await deleteKey(k.id);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setDeletingId('');
    }
  }, [deleteKey]);

  const activeKeys = useMemo(() => keys.filter((k) => !k.revoked_at), [keys]);
  const revokedKeys = useMemo(() => keys.filter((k) => k.revoked_at), [keys]);

  const submitRelayStart = useCallback(() => {
    const port = Number(relayPort) || 21000;
    void relayStart({ host: relayHost, controlPort: port, token: relayToken, tls: relayTls });
  }, [relayHost, relayPort, relayToken, relayTls, relayStart]);

  return (
    <div className="remote-settings">
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
          经中继接入。开启后遥控页的二维码与设备密钥自动切换为中继入口。
        </p>
        {info?.relay ? (
          <p className="remote-page__hint">
            中继入口：{info.relay.tls ? 'https' : 'http'}://{info.relay.host}:{info.relay.data_port}
            （数据端口随令牌固定，重连不变）。手机照常扫码即可，流量经服务器转发但端到端凭证验证不变。
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
              placeholder="接入令牌（≥32 字符，openssl rand -hex 24）"
              value={relayToken}
              autoComplete="off"
              onChange={(e) => setRelayToken(e.target.value)}
            />
            <label className="remote-page__relay-tls">
              <input
                type="checkbox"
                checked={relayTls}
                onChange={(e) => setRelayTls(e.target.checked)}
              />
              TLS（服务器已前置 TLS 终止，入口用 https://）
            </label>
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
            <p className="remote-page__hint">还没有设备密钥。遥控页的临时扫码每次重启都要重扫；密钥不会。</p>
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
              <button
                type="button"
                className="remote-page__danger remote-page__key-revoke"
                disabled={deletingId === k.id}
                onClick={() => void submitDelete(k)}
              >
                <Trash2 size={12} />
                {deletingId === k.id ? '删除中…' : '删除'}
              </button>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
