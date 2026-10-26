/**
 * 设置中心「账户」分区：本地多账户（P0 冷切换架构）。
 *
 * - 显示当前启动模式（账户 / legacy）与当前账户
 * - 列出已有账户，支持冷切换启动（释放锁 → spawn 新进程 → 当前进程退出）
 * - 创建新账户（首个账户自动承接 legacy 单用户数据迁移）
 *
 * legacy 模式用户经此进入多账户体系（账户门在 legacy 模式直接放行，
 * 这里是唯一的迁移入口）。
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
}

interface AccountStatus {
  mode: 'account' | 'legacy';
  account_id: string | null;
  display_name: string | null;
  accounts: Account[];
}

export default function AccountSettings() {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [launching, setLaunching] = useState<string | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    invoke<AccountStatus>('account_status')
      .then(setStatus)
      .catch((e) => setError(String(e)));
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

  async function createAccount(e: FormEvent, name: string, reset: () => void): Promise<void> {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setError('');
    try {
      const account = await invoke<{ account_id: string }>('account_create', {
        displayName: trimmed,
      });
      reset();
      // 创建成功即冷切换进入新账户；首个账户会自动迁移 legacy 数据
      await launch(account.account_id);
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
              : 'legacy 单用户模式：沿用旧数据目录，账户功能可在下方开启'
          }
        >
          <span className={`acct-mode acct-mode--${status.mode}`}>
            {status.mode === 'account' ? status.display_name ?? '账户' : 'legacy'}
          </span>
        </SettingsRow>
      )}

      {status && status.accounts.length > 0 && (
        <SettingsRow stacked title="已有账户" desc="启动即冷切换：退出当前进程并以该账户重启">
          <ul className="acct-list">
            {status.accounts.map((a) => (
              <li key={a.account_id} className="acct-list__item">
                <span className="acct-list__avatar">{a.display_name.slice(0, 1)}</span>
                <span className="acct-list__name">
                  {a.display_name}
                  {status.account_id === a.account_id && (
                    <span className="acct-list__badge">
                      <ShieldCheck size={12} strokeWidth={2} /> 当前
                    </span>
                  )}
                </span>
                {status.account_id !== a.account_id && (
                  <button
                    type="button"
                    className="acct-list__launch"
                    disabled={launching !== null}
                    onClick={() => launch(a.account_id)}
                  >
                    <ArrowRightCircle size={13} strokeWidth={2} />
                    {launching === a.account_id ? '启动中…' : '启动'}
                  </button>
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

      {error && <p className="acct-error">{error}</p>}
    </SettingsSection>
  );
}

function CreateAccountRow({
  disabled,
  isFirst,
  onSubmit,
}: {
  disabled: boolean;
  isFirst: boolean;
  onSubmit: (e: FormEvent, name: string, reset: () => void) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);

  return (
    <SettingsRow
      title="创建新账户"
      desc={
        isFirst
          ? '首个账户会自动把当前（legacy）的会话、记忆与配置迁移过来，之后以该账户启动'
          : '新账户从空白开始：独立的会话、记忆、项目与凭据'
      }
    >
      <form
        className="acct-create"
        onSubmit={(e) => {
          setBusy(true);
          onSubmit(e, name, () => setName('')).finally(() => setBusy(false));
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
        <button
          type="submit"
          className="btn-save acct-create__btn"
          disabled={disabled || busy || !name.trim()}
        >
          {busy ? '创建中…' : isFirst ? '创建并迁移' : '创建'}
        </button>
      </form>
    </SettingsRow>
  );
}
