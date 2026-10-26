/**
 * 设置中心「账户」分区：本地多账户（praxis 式登录，冷切换架构）。
 *
 * - 显示当前启动模式（账户 / legacy）与当前账户
 * - 列出已有账户：点击「启动」展开密码输入 → 验证密码冷切换
 *   （老账户无凭据：首次启动设置初始密码）
 * - 创建新账户：名称 + 登录密码（首个账户自动承接 legacy 数据迁移）
 * - 更换登录密码：旧密码 + 新密码（KEK 不变，仅重新包装）
 *
 * legacy 模式用户经此进入多账户体系（账户门在 legacy 模式渲染登录列表，
 * 这里是补充入口）。
 */
import { useEffect, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowRightCircle, ShieldCheck } from 'lucide-react';
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import './AccountSettings.scss';

interface Account {
  account_id: string;
  display_name: string;
  created_at: number;
  status: string;
  has_auth?: boolean;
}

interface AccountStatus {
  mode: 'account' | 'legacy';
  account_id: string | null;
  display_name: string | null;
  accounts: Account[];
  password_protected: boolean;
  locked: boolean;
}

export default function AccountSettings() {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  // 展开密码输入的账户（启动流程）
  const [launching, setLaunching] = useState<string | null>(null);
  const [error, setError] = useState('');

  const refresh = () =>
    invoke<AccountStatus>('account_status').then(setStatus).catch(() => {});

  useEffect(() => {
    invoke<AccountStatus>('account_status')
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, []);

  async function launch(accountId: string, password: string) {
    setError('');
    setLaunching(accountId);
    try {
      // 成功时后端 spawn 新进程并 exit(0)，Promise 不会 resolve
      await invoke('account_launch', { accountId, password });
      setLaunching(null);
    } catch (e) {
      setError(String(e));
      setLaunching(null);
    }
  }

  async function createAccount(
    e: FormEvent,
    name: string,
    password: string,
    reset: () => void,
  ): Promise<void> {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || !password) return;
    setError('');
    try {
      const account = await invoke<{ account_id: string }>('account_create', {
        displayName: trimmed,
        password,
      });
      reset();
      // 创建成功即冷切换进入新账户；首个账户会自动迁移 legacy 数据
      await launch(account.account_id, password);
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <SettingsSection iconId="account" title="账户" desc="本地多账户：数据彼此独立，仅保存在本机">
      {status && (
        <SettingsRow
          title="当前模式"
          desc={
            status.mode === 'account'
              ? '已绑定账户启动，所有会话、记忆与配置归属该账户'
              : 'legacy 单用户模式：沿用旧数据目录，登录列表与创建入口见下方'
          }
        >
          <span className={`acct-mode acct-mode--${status.mode}`}>
            {status.mode === 'account' ? status.display_name ?? '账户' : 'legacy'}
          </span>
        </SettingsRow>
      )}

      {status && status.accounts.length > 0 && (
        <SettingsRow
          stacked
          title="已有账户"
          desc="启动需输入该账户的登录密码；老账户首次启动时设置初始密码"
        >
          <ul className="acct-list">
            {status.accounts.map((a) => (
              <li key={a.account_id} className="acct-list__entry">
                <div className="acct-list__item">
                  <span className="acct-list__avatar">{a.display_name.slice(0, 1)}</span>
                  <span className="acct-list__name">
                    {a.display_name}
                    {status.account_id === a.account_id && (
                      <span className="acct-list__badge">
                        <ShieldCheck size={12} strokeWidth={2} /> 当前
                      </span>
                    )}
                  </span>
                  {a.has_auth !== true && <span className="acct-list__initial">设初始密码</span>}
                  {status.account_id !== a.account_id && (
                    <button
                      type="button"
                      className="acct-list__launch"
                      disabled={launching !== null}
                      onClick={() =>
                        setLaunching(launching === a.account_id ? null : a.account_id)
                      }
                    >
                      <ArrowRightCircle size={13} strokeWidth={2} />
                      启动
                    </button>
                  )}
                </div>
                {launching === a.account_id && (
                  <LaunchPasswordForm
                    account={a}
                    busy={launching !== null}
                    onSubmit={(password) => launch(a.account_id, password)}
                    onCancel={() => setLaunching(null)}
                  />
                )}
              </li>
            ))}
          </ul>
        </SettingsRow>
      )}

      <CreateAccountRow
        disabled={launching !== null}
        isFirst={!!status && status.accounts.length === 0}
        onSubmit={createAccount}
      />

      <PasswordRow
        status={status}
        onChanged={() => invoke<AccountStatus>('account_status').then(setStatus).catch(() => {})}
      />

      {error && <p className="acct-error">{error}</p>}
    </SettingsSection>
  );
}

/** 启动密码表单（行内展开）：有凭据验证密码；老账户设初始密码（两次输入） */
function LaunchPasswordForm({
  account,
  busy,
  onSubmit,
  onCancel,
}: {
  account: Account;
  busy: boolean;
  onSubmit: (password: string) => void;
  onCancel: () => void;
}) {
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [error, setError] = useState('');
  const needInit = account.has_auth !== true;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!password || busy) return;
    if (needInit && password !== confirm) {
      setError('两次输入的密码不一致');
      return;
    }
    setError('');
    // 成功时进程退出，无需恢复状态
    onSubmit(password);
  }

  return (
    <form className="acct-list__pwform" onSubmit={submit}>
      <input
        className="acct-create__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder={needInit ? '设置初始登录密码（6-128 位）' : '登录密码'}
        maxLength={128}
        autoFocus
        disabled={busy}
      />
      {needInit && (
        <input
          className="acct-create__input"
          type="password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          placeholder="确认初始密码"
          maxLength={128}
          disabled={busy}
        />
      )}
      <button type="submit" className="btn-save acct-create__btn" disabled={busy || !password}>
        {needInit ? '设置并启动' : '启动'}
      </button>
      <button type="button" className="btn-discard acct-create__btn" onClick={onCancel}>
        取消
      </button>
      {error && <p className="acct-error">{error}</p>}
    </form>
  );
}

/**
 * 更换登录密码：旧密码 + 新密码（仅当前账户；KEK 不变，已加密数据无需重加密）。
 * 老账户（未设密码）先启动一次完成初始密码设置，再回来更换。
 */
function PasswordRow({
  status,
  onChanged,
}: {
  status: AccountStatus | null;
  onChanged: () => void;
}) {
  const [oldPassword, setOldPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState('');
  const [err, setErr] = useState('');
  const locked = !!status?.locked;
  const hasPassword = !!status?.password_protected;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!oldPassword || !newPassword || busy) return;
    setBusy(true);
    setErr('');
    setMsg('');
    try {
      await invoke('account_password_change', {
        oldPassword,
        newPassword,
      });
      setMsg('登录密码已更换，下次启动生效');
      setOldPassword('');
      setNewPassword('');
      onChanged();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <SettingsRow
      stacked
      title="登录密码"
      desc={
        hasPassword
          ? '保护本账户数据；仅本地验证，不同步到云端'
          : '当前账户尚未设置登录密码：先在登录/启动时设置初始密码，再在此更换'
      }
    >
      {locked ? (
        <p className="acct-password__hint">账户处于锁定状态，请先在启动界面解锁</p>
      ) : (
        <form className="acct-create acct-create--stacked" onSubmit={submit}>
          <input
            className="acct-create__input"
            type="password"
            value={oldPassword}
            onChange={(e) => setOldPassword(e.target.value)}
            placeholder={hasPassword ? '当前密码' : '初始密码（未设置过则填启动时设的密码）'}
            maxLength={128}
            disabled={busy}
          />
          <input
            className="acct-create__input"
            type="password"
            value={newPassword}
            onChange={(e) => setNewPassword(e.target.value)}
            placeholder="新密码（6-128 位）"
            maxLength={128}
            disabled={busy}
          />
          <button
            type="submit"
            className="btn-save acct-create__btn"
            disabled={busy || !oldPassword || !newPassword}
          >
            {busy ? '提交中…' : '更换密码'}
          </button>
          {msg && <p className="acct-password__hint acct-password__hint--ok">{msg}</p>}
          {err && <p className="acct-error">{err}</p>}
        </form>
      )}
    </SettingsRow>
  );
}

function CreateAccountRow({
  disabled,
  isFirst,
  onSubmit,
}: {
  disabled: boolean;
  isFirst: boolean;
  onSubmit: (
    e: FormEvent,
    name: string,
    password: string,
    reset: () => void,
  ) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const ready = name.trim() && password && password === confirm;

  return (
    <SettingsRow
      stacked
      title="创建新账户"
      desc={
        isFirst
          ? '首个账户会自动把当前（legacy）的会话、记忆与配置迁移过来，之后以该账户启动'
          : '新账户从空白开始：独立的会话、记忆、项目与凭据'
      }
    >
      <form
        className="acct-create acct-create--stacked"
        onSubmit={(e) => {
          if (password !== confirm) {
            setError('两次输入的密码不一致');
            return;
          }
          setError('');
          setBusy(true);
          onSubmit(e, name, password, () => {
            setName('');
            setPassword('');
            setConfirm('');
          }).finally(() => setBusy(false));
        }}
      >
        <input
          className="acct-create__input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="账户名称"
          maxLength={32}
          disabled={disabled || busy}
        />
        <input
          className="acct-create__input"
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="登录密码（6-128 位）"
          maxLength={128}
          disabled={disabled || busy}
        />
        <input
          className="acct-create__input"
          type="password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          placeholder="确认登录密码"
          maxLength={128}
          disabled={disabled || busy}
        />
        <button
          type="submit"
          className="btn-save acct-create__btn"
          disabled={disabled || busy || !ready}
        >
          {busy ? '创建中…' : isFirst ? '创建并迁移' : '创建'}
        </button>
        {error && <p className="acct-error">{error}</p>}
      </form>
    </SettingsRow>
  );
}
