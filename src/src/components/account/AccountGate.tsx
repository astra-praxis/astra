/**
 * 本地账户门（praxis 式登录：选用户 → 输密码 → 进应用）。
 *
 * 应用启动时先查询账户状态，四种渲染分支：
 * 1. locked（绑定账户启动但尚未解锁）：输入该账户密码解锁
 * 2. legacy 且已有账户：用户列表——点选用户 → 输密码 → 冷切换登录
 * 3. legacy 且无账户：首次使用——创建账户（名称 + 登录密码）后自动登录
 * 4. account 且未锁定：放行主应用
 *
 * 登录 = 冷切换：后端验证密码 → KEK 写一次性交接 token →
 * 释放实例锁 → spawn 自身带 --account=<id> → 当前进程退出。
 * 老账户（legacy 时期创建、无登录凭据）首次登录即设置初始密码。
 * 前端不传递身份，账户归属由进程参数决定（见 src-tauri/src/app/accounts.rs）。
 */
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, LogIn, ShieldCheck } from 'lucide-react';
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
  // 登录列表里点选的用户（null = 显示列表）
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
      <GateFrame hint={`「${status.display_name ?? '当前账户'}」已加密，输入登录密码解锁。`}>
        <UnlockForm />
      </GateFrame>
    );
  }

  // legacy 模式 + 已有账户：praxis 式登录列表
  if (status.accounts.length > 0) {
    return (
      <GateFrame
        hint={selected ? `以「${selected.display_name}」身份登录` : '选择账户登录，数据各自独立'}
      >
        {selected ? (
          <LoginForm account={selected} onBack={() => setSelected(null)} />
        ) : (
          <ul className="account-gate__list">
            {status.accounts.map((a) => (
              <li key={a.account_id}>
                <button
                  type="button"
                  className="account-gate__item"
                  onClick={() => setSelected(a)}
                >
                  <span className="account-gate__avatar">{a.display_name.slice(0, 1)}</span>
                  <span className="account-gate__name">{a.display_name}</span>
                  {a.has_auth ? (
                    <ShieldCheck size={14} className="account-gate__lock" strokeWidth={2} />
                  ) : (
                    <span className="account-gate__initial">设初始密码</span>
                  )}
                  <LogIn size={15} className="account-gate__action" strokeWidth={2} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </GateFrame>
    );
  }

  // legacy 模式 + 无账户：首次使用，创建第一个账户（承接 legacy 数据）
  return (
    <GateFrame hint="创建本机账户：会话、记忆与配置将归属于它">
      <FirstAccountForm />
    </GateFrame>
  );
}

function GateFrame({ hint, children }: { hint: string; children: ReactNode }) {
  return (
    <div className="account-gate">
      <div className="account-gate__panel">
        <h1 className="account-gate__title">Astra</h1>
        <p className="account-gate__hint">{hint}</p>
        {children}
      </div>
    </div>
  );
}

/** 绑定账户的解锁表单（locked 态） */
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
    <form className="account-gate__create" onSubmit={submit}>
      <input
        className="account-gate__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder="登录密码"
        autoFocus
        disabled={busy}
      />
      <button type="submit" className="account-gate__submit" disabled={busy || !password}>
        {busy ? '解锁中…' : '解锁'}
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}

/**
 * 登录列表点选用户后的密码表单。
 * - has_auth：验证密码 → 冷切换
 * - 老账户无凭据：首次登录设置初始密码（两次输入）
 * 成功时后端 spawn 新进程并 exit(0)，Promise 不会 resolve。
 */
function LoginForm({
  account,
  onBack,
}: {
  account: Account;
  onBack: () => void;
}) {
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const needInit = account.has_auth !== true;

  function validate(): string {
    if (!password) return '';
    if (needInit && password !== confirm) return '两次输入的密码不一致';
    return '';
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!password || busy) return;
    const mismatch = validate();
    if (mismatch) {
      setError(mismatch);
      return;
    }
    setBusy(true);
    setError('');
    try {
      await invoke('account_launch', { accountId: account.account_id, password });
      // 成功时进程已退出，走不到这里
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <form className="account-gate__create" onSubmit={submit}>
      <input
        className="account-gate__input"
        type="password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder={needInit ? '设置初始登录密码（6-128 位）' : '登录密码'}
        autoFocus
        disabled={busy}
      />
      {needInit && (
        <input
          className="account-gate__input"
          type="password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          placeholder="确认初始密码"
          disabled={busy}
        />
      )}
      <button type="submit" className="account-gate__submit" disabled={busy || !password}>
        {busy ? '登录中…' : needInit ? '设置并登录' : '登录'}
      </button>
      <button type="button" className="account-gate__back" onClick={onBack} disabled={busy}>
        <ArrowLeft size={13} strokeWidth={2} /> 返回选择用户
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}

/** 首个账户创建（legacy 无账户态）：名称 + 登录密码 + 确认，创建后自动登录 */
function FirstAccountForm() {
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  function validate(): string {
    if (!name.trim() || !password) return '';
    if (password !== confirm) return '两次输入的密码不一致';
    return '';
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || !password || busy) return;
    const mismatch = validate();
    if (mismatch) {
      setError(mismatch);
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
      await invoke('account_launch', { accountId: account.account_id, password });
      // 成功时进程已退出，走不到这里
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  const ready = name.trim() && password && confirm;

  return (
    <form className="account-gate__create" onSubmit={submit}>
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
      <button type="submit" className="account-gate__submit" disabled={busy || !ready}>
        {busy ? '创建中…' : '创建账户'}
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}
