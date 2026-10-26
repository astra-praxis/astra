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
}

export default function AccountGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [launching, setLaunching] = useState<string | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    invoke<AccountStatus>('account_status')
      .then(setStatus)
      .catch((e) => {
        // 状态查询失败不阻断应用（向后兼容：命令不存在等异常场景）
        console.error('account_status failed', e);
        setStatus({ mode: 'legacy', account_id: null, display_name: null, accounts: [] });
      });
  }, []);

  async function launch(accountId: string) {
    setError('');
    setLaunching(accountId);
    try {
      // 成功时后端 spawn 新进程并 exit(0)，Promise 不会 resolve
      await invoke('account_launch', { accountId });
      setLaunching(null);
    } catch (e) {
      setError(String(e));
      setLaunching(null);
    }
  }

  // 未加载完成前不渲染任何内容，避免主应用闪现
  if (!status) return <div className="account-gate account-gate--loading" />;

  // 已绑定账户或 legacy 启动：直接放行
  if (status.mode === 'account' || status.mode === 'legacy') return <>{children}</>;

  return (
    <div className="account-gate">
      <div className="account-gate__panel">
        <h1 className="account-gate__title">Astra</h1>
        <p className="account-gate__hint">
          {status.accounts.length === 0
            ? '创建你的第一个本地账户。账户数据彼此独立，仅保存在本机。'
            : '选择要启动的账户，或创建新账户。'}
        </p>

        {status.accounts.length > 0 && (
          <ul className="account-gate__list">
            {status.accounts.map((a) => (
              <li key={a.account_id}>
                <button
                  type="button"
                  className="account-gate__item"
                  disabled={launching !== null}
                  onClick={() => launch(a.account_id)}
                >
                  <span className="account-gate__avatar">{a.display_name.slice(0, 1)}</span>
                  <span className="account-gate__name">{a.display_name}</span>
                  <span className="account-gate__action">
                    {launching === a.account_id ? '启动中…' : '启动'}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}

        <CreateAccountForm disabled={launching !== null} onCreated={launch} />
        {error && <p className="account-gate__error">{error}</p>}
      </div>
    </div>
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
