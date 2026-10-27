/**
 * 本地账户门（praxis 式登录：选用户 → 输密码 → 进应用）。
 *
 * 应用启动时先查询账户状态，四种渲染分支：
 * 1. locked（绑定账户启动但尚未解锁）：输入该账户密码解锁
 * 2. legacy 且已有账户：头像选择器——点选用户 → 输密码 → 热切换登录
 *    （老账户无凭据：首次登录只输一次，该密码即设为登录密码）
 * 3. legacy 且无账户：首次使用——创建账户（名称 + 登录密码）后自动登录
 * 4. account 且未锁定：放行主应用
 *
 * 登录 = 热切换：后端验证密码 → 进程内换绑账户（重置 AgentState、
 * 重绑数据目录与 KEK）→ 前端整页 reload 重新拉取状态。
 * 前端不传递身份，账户归属由后端 ACTIVE 状态决定（见 src-tauri/src/app/accounts.rs）。
 */
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, LogIn } from 'lucide-react';
import { formatAccountName } from '../../lib/format';
import './AccountGate.scss';

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

export default function AccountGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  // 登录选择器里点选的用户（null = 显示头像网格）
  const [selected, setSelected] = useState<Account | null>(null);

  useEffect(() => {
    invoke<AccountStatus>('account_status')
      .then(setStatus)
      .catch((e) => {
        // 状态查询失败不阻断应用（向后兼容：命令不存在等异常场景）
        console.error('account_status failed', e);
        setStatus({
          mode: 'legacy',
          account_id: null,
          display_name: null,
          accounts: [],
          password_protected: false,
          locked: false,
        });
      });
  }, []);

  // 未加载完成前不渲染任何内容，避免主应用闪现
  if (!status) return <div className="account-gate account-gate--loading" />;

  // 已绑定账户且未锁定：直接放行
  if (status.mode === 'account' && !status.locked) {
    return <>{children}</>;
  }

  // 绑定账户启动但处于锁定态：输入该账户密码解锁
  if (status.mode === 'account' && status.locked) {
    return (
      <GateShell
        subtitle={`「${formatAccountName(status.display_name)}」已加密，输入登录密码解锁`}
      >
        <UnlockForm />
      </GateShell>
    );
  }

  // legacy 模式 + 已有账户：praxis 式头像选择器
  if (status.accounts.length > 0) {
    return selected ? (
      <GateShell subtitle={`以「${formatAccountName(selected.display_name)}」身份登录`}>
        <div className="account-gate__picked">
          <UserAvatar name={selected.display_name} size={64} />
          <LoginForm
            account={selected}
            onBack={() => {
              setSelected(null);
            }}
          />
        </div>
      </GateShell>
    ) : (
      <GateShell subtitle="选择账户，数据各自独立">
        <div className="account-gate__picker" role="listbox" aria-label="选择账户">
          {status.accounts.map((a) => (
            <button
              key={a.account_id}
              type="button"
              className="account-gate__user"
              role="option"
              aria-selected={false}
              onClick={() => setSelected(a)}
            >
              <UserAvatar name={a.display_name} size={64} />
              <span className="account-gate__user-name">{formatAccountName(a.display_name)}</span>
              {a.has_auth !== true && <span className="account-gate__user-flag">首次登录</span>}
            </button>
          ))}
        </div>
      </GateShell>
    );
  }

  // legacy 模式 + 无账户：首次使用，创建第一个账户（承接 legacy 数据）
  return (
    <GateShell subtitle="创建本机账户，会话、记忆与配置将归属于它">
      <FirstAccountForm />
    </GateShell>
  );
}

/** 登录页外壳：logo + 副标题 + 内容，一次入场动画，背景品牌光晕 */
function GateShell({ subtitle, children }: { subtitle: string; children: ReactNode }) {
  return (
    <div className="account-gate">
      <div className="account-gate__glow" aria-hidden />
      <div className="account-gate__panel">
        <img className="account-gate__logo" src="/logo.png" alt="Astra" />
        <h1 className="account-gate__title">Astra</h1>
        <p className="account-gate__subtitle">{subtitle}</p>
        {children}
      </div>
    </div>
  );
}

/** 圆形用户头像：品牌渐变底 + 首字（首字母大写） */
function UserAvatar({ name, size }: { name: string; size: number }) {
  return (
    <span
      className="account-gate__avatar"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.36) }}
      aria-hidden
    >
      {formatAccountName(name).slice(0, 1)}
    </span>
  );
}

/** 绑定账户的解锁表单（locked 态，单密码框） */
function UnlockForm() {
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!password) return;
    setBusy(true);
    setError('');
    try {
      await invoke('account_unlock', { password });
      // 解锁后后端已补做记忆初始化，刷新前端以正常加载数据
      window.location.reload();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <form className="account-gate__form" onSubmit={submit}>
      <input
        className="account-gate__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder="登录密码"
        autoFocus
        disabled={busy}
      />
      <button type="submit" className="account-gate__go" disabled={busy || !password}>
        {busy ? '解锁中…' : '解锁'}
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}

/**
 * 点选用户后的密码表单（单密码框）。
 * - has_auth：验证密码 → 热切换
 * - 老账户无凭据：输入的密码即设为该账户的登录密码（只输一次）
 * 成功时后端进程内换绑账户，前端整页 reload。
 */
function LoginForm({ account, onBack }: { account: Account; onBack: () => void }) {
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const needInit = account.has_auth !== true;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!password || busy) return;
    setBusy(true);
    setError('');
    try {
      await invoke('account_switch', { accountId: account.account_id, password });
      // 热切换成功：进程内已换绑账户，整页 reload 以新账户身份重建前端
      window.location.reload();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <form className="account-gate__form" onSubmit={submit}>
      <input
        className="account-gate__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder={needInit ? '设置登录密码' : '登录密码'}
        autoFocus
        disabled={busy}
      />
      {needInit && (
        <p className="account-gate__notice">首次登录：输入的密码将设为该账户的登录密码</p>
      )}
      <button type="submit" className="account-gate__go" disabled={busy || !password}>
        <LogIn size={15} strokeWidth={2} />
        {busy ? '登录中…' : needInit ? '设置并登录' : '登录'}
      </button>
      <button type="button" className="account-gate__back" onClick={onBack} disabled={busy}>
        <ArrowLeft size={13} strokeWidth={2} /> 返回
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}

/** 首个账户创建（legacy 无账户态）：创建后自动登录 */
function FirstAccountForm() {
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function submit(e: FormEvent) {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || !password || busy) return;
    if (password !== confirm) {
      setError('两次输入的密码不一致');
      return;
    }
    setBusy(true);
    setError('');
    try {
      // 首个账户：后端自动把 legacy 会话/记忆/配置迁移进来
      const account = await invoke<{ account_id: string }>('account_create', {
        displayName: trimmed,
        password,
      });
      await invoke('account_switch', { accountId: account.account_id, password });
      // 热切换成功：进程内已换绑账户，整页 reload 以新账户身份重建前端
      window.location.reload();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  const ready = name.trim() && password && confirm;

  return (
    <form className="account-gate__form account-gate__form--create" onSubmit={submit}>
      <input
        className="account-gate__input"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="账户名称"
        maxLength={32}
        autoFocus
        disabled={busy}
      />
      <input
        className="account-gate__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder="登录密码（6-128 位）"
        maxLength={128}
        disabled={busy}
      />
      <input
        className="account-gate__input"
        type="password"
        value={confirm}
        onChange={(e) => setConfirm(e.target.value)}
        placeholder="确认登录密码"
        maxLength={128}
        disabled={busy}
      />
      <button type="submit" className="account-gate__go" disabled={busy || !ready}>
        {busy ? '创建中…' : '创建账户'}
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}
