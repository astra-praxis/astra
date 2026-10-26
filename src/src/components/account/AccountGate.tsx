/**
 * 本地账户门（多账户 P0，冷切换架构）。
 *
 * 应用启动时先查询账户状态：
 * - legacy（无 --account 参数启动）：直接放行——旧版本行为不变，开发模式也走这里
 * - account（已绑定账户启动）：直接放行，展示绑定账户的应用
 * - 需要选择/创建账户（无参数但存在账户，或尚无任何账户）：渲染账户门，
 *   阻断主应用直到用户「创建账户并启动」或「选择已有账户启动」
 *
 * 启动 = 冷切换：后端释放实例锁 → spawn 自身带 --account=<id> → 当前进程退出。
 * 前端不传递身份，账户归属由进程参数决定（见 src-tauri/src/app/accounts.rs）。
 */
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './AccountGate.scss';

interface Account {
  account_id: string;
  display_name: string;
  created_at: number;
  status: string;
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

  // 已绑定账户或 legacy 启动且未锁定：直接放行
  if (!status.locked && (status.mode === 'account' || status.mode === 'legacy')) {
    return <>{children}</>;
  }

  // 锁定态：设了访问密码且本进程尚未解锁，输密码后刷新进入
  return (
    <div className="account-gate">
      <div className="account-gate__panel">
        <h1 className="account-gate__title">Astra</h1>
        <p className="account-gate__hint">
          {status.display_name ? `「${status.display_name}」已启用访问密码` : '本机数据已启用访问密码'}
          ，输入密码解锁后使用。
        </p>
        <UnlockForm />
      </div>
    </div>
  );
}

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
        placeholder="访问密码"
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

function CreateAccountForm({
  disabled,
  onCreated,
}: {
  disabled: boolean;
  onCreated: (accountId: string) => void;
}) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function submit(e: FormEvent) {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setBusy(true);
    setError('');
    try {
      const account = await invoke<{ account_id: string }>('account_create', {
        displayName: trimmed,
      });
      onCreated(account.account_id);
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <form className="account-gate__create" onSubmit={submit}>
      <input
        className="account-gate__input"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="新账户名称"
        maxLength={32}
        disabled={disabled || busy}
      />
      <button
        type="submit"
        className="account-gate__submit"
        disabled={disabled || busy || !name.trim()}
      >
        {busy ? '创建中…' : '创建并启动'}
      </button>
      {error && <p className="account-gate__error">{error}</p>}
    </form>
  );
}
