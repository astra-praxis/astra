/**
 * 设置中心「账户」分区：本地多账户（praxis 式登录，热切换架构）。
 *
 * - 显示当前启动模式（账户 / legacy）与当前账户
 * - 已有账户列表：「启动」→ 验证密码热切换；30 天内验证过密码的账户
 *   （trusted）免密直接进入
 * - 「创建账户」/「更换登录密码」均为弹窗，不在页面平铺表单
 *
 * legacy 模式用户经此进入多账户体系（账户门在 legacy 模式渲染登录选择器，
 * 这里是补充入口）。
 */
import { useEffect, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, ShieldCheck, UserRoundCog } from 'lucide-react';
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import Modal from '@/components/ui/Modal';
import ChangePasswordModal from '@/components/account/ChangePasswordModal';
import { formatAccountName } from '@/lib/format';
import './AccountSettings.scss';

interface Account {
  account_id: string;
  display_name: string;
  created_at: number;
  status: string;
  has_auth?: boolean;
  /** 30 天信任凭据有效：启动时免密直接进入 */
  trusted?: boolean;
}

interface AccountStatus {
  mode: 'account' | 'legacy';
  account_id: string | null;
  display_name: string | null;
  accounts: Account[];
  password_protected: boolean;
  locked: boolean;
}

type Dialog = null | 'create' | 'password' | { kind: 'launch'; account: Account };

export default function AccountSettings() {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [error, setError] = useState('');

  const refresh = () =>
    invoke<AccountStatus>('account_status').then(setStatus).catch(() => {});

  useEffect(() => {
    invoke<AccountStatus>('account_status')
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, []);

  async function launch(accountId: string, password?: string) {
    setError('');
    try {
      // 热切换：进程内换绑目标账户，成功后整页 reload 以新账户身份重建。
      // 无 password = 走 30 天信任凭据免密通道（trusted 账户）
      if (password === undefined) {
        await invoke('account_switch', { accountId });
      } else {
        await invoke('account_switch', { accountId, password });
      }
      window.location.reload();
    } catch (e) {
      setError(String(e));
      throw e; // 让弹窗保持打开并显示错误
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
              : 'legacy 单用户模式：沿用旧数据目录，登录选择器与创建入口见下方'
          }
        >
          <span className={`acct-mode acct-mode--${status.mode}`}>
            {status.mode === 'account' ? formatAccountName(status.display_name) : 'legacy'}
          </span>
        </SettingsRow>
      )}

      <SettingsRow
        title="已有账户"
        desc={
          status && status.accounts.length > 0
            ? '30 天内登录过的账户免密直接进入；其余需输入登录密码'
            : '还没有账户，创建后以独立身份使用'
        }
      >
        <button
          type="button"
          className="btn-save acct-action"
          onClick={() => setDialog('create')}
          disabled={!!status && status.accounts.length >= 16}
        >
          <Plus size={14} strokeWidth={2} />
          创建账户
        </button>
      </SettingsRow>

      {status && status.accounts.length > 0 && (
        <SettingsRow stacked title="账户列表">
          <ul className="acct-list">
            {status.accounts.map((a) => (
              <li key={a.account_id} className="acct-list__item">
                <span className="acct-list__avatar">{formatAccountName(a.display_name).slice(0, 1)}</span>
                <span className="acct-list__name">
                  {formatAccountName(a.display_name)}
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
                    onClick={() => {
                      // 受信账户免密直接启动；失败（凭据失效等）回退弹窗输密码
                      if (a.trusted) {
                        launch(a.account_id).catch(() => {
                          setDialog({ kind: 'launch', account: a });
                        });
                      } else {
                        setDialog({ kind: 'launch', account: a });
                      }
                    }}
                  >
                    <UserRoundCog size={13} strokeWidth={2} />
                    启动
                  </button>
                )}
              </li>
            ))}
          </ul>
        </SettingsRow>
      )}

      <SettingsRow
        title="登录密码"
        desc={
          status?.password_protected
            ? '保护本账户数据；仅本地验证，不同步到云端'
            : '当前账户尚未设置登录密码：先启动一次完成设置，再在此更换'
        }
      >
        <button
          type="button"
          className="btn-discard acct-action"
          onClick={() => setDialog('password')}
          disabled={!status || !!status.locked}
        >
          更换密码
        </button>
      </SettingsRow>

      {error && <p className="acct-error">{error}</p>}

      {/* 创建账户弹窗 */}
      <CreateAccountModal
        open={dialog === 'create'}
        isFirst={!!status && status.accounts.length === 0}
        onClose={() => setDialog(null)}
        onCreated={refresh}
        onLaunch={launch}
      />

      {/* 启动账户弹窗（验证密码 / 老账户设初始密码） */}
      {dialog && typeof dialog === 'object' && dialog.kind === 'launch' && (
        <LaunchAccountModal
          account={dialog.account}
          onClose={() => setDialog(null)}
          onLaunch={launch}
        />
      )}

      {/* 更换登录密码弹窗（与侧栏账户菜单共用） */}
      {status && dialog === 'password' && (
        <ChangePasswordModal
          open
          onClose={() => setDialog(null)}
          hasPassword={status.password_protected}
          onChanged={refresh}
        />
      )}
    </SettingsSection>
  );
}

/** 创建账户弹窗：名称 + 登录密码；创建成功后询问是否立即启动 */
function CreateAccountModal({
  open,
  isFirst,
  onClose,
  onCreated,
  onLaunch,
}: {
  open: boolean;
  isFirst: boolean;
  onClose: () => void;
  onCreated: () => void;
  onLaunch: (accountId: string, password: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const ready = name.trim() && password && password === confirm;

  function reset() {
    setName('');
    setPassword('');
    setConfirm('');
    setError('');
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ready || busy) return;
    if (password !== confirm) {
      setError('两次输入的密码不一致');
      return;
    }
    setBusy(true);
    setError('');
    try {
      const account = await invoke<{ account_id: string }>('account_create', {
        displayName: name.trim(),
        password,
      });
      onCreated();
      reset();
      onClose();
      // 创建成功即热切换进入新账户（首个账户自动迁移 legacy 数据）
      await onLaunch(account.account_id, password);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={() => {
        reset();
        onClose();
      }}
      title="创建账户"
      closeOnOverlay={false}
      closeDisabled={busy}
      footer={
        <>
          <button
            type="button"
            className="btn-discard"
            onClick={() => {
              reset();
              onClose();
            }}
            disabled={busy}
          >
            取消
          </button>
          <button
            type="submit"
            form="create-account-form"
            className="btn-save"
            disabled={busy || !ready}
          >
            {busy ? '创建中…' : '创建并进入'}
          </button>
        </>
      }
    >
      <form id="create-account-form" className="acct-modal-form" onSubmit={submit}>
        <label className="acct-modal-form__field">
          <span>账户名称</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="1-32 个字符"
            maxLength={32}
            autoFocus
            disabled={busy}
          />
        </label>
        <label className="acct-modal-form__field">
          <span>登录密码</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="6-128 位"
            maxLength={128}
            disabled={busy}
          />
        </label>
        <label className="acct-modal-form__field">
          <span>确认密码</span>
          <input
            type="password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            placeholder="再次输入密码"
            maxLength={128}
            disabled={busy}
          />
        </label>
        {password && confirm && password !== confirm && (
          <p className="acct-modal-form__error">两次输入的密码不一致</p>
        )}
        {error && <p className="acct-modal-form__error">{error}</p>}
        <p className="acct-modal-form__hint">
          {isFirst
            ? '首个账户会自动把当前（legacy）的会话、记忆与配置迁移过来'
            : '新账户从空白开始：独立的会话、记忆、项目与凭据'}
        </p>
      </form>
    </Modal>
  );
}

/** 启动账户弹窗：验证密码；老账户（无凭据）首次启动设置初始密码（单密码框） */
function LaunchAccountModal({
  account,
  onClose,
  onLaunch,
}: {
  account: Account;
  onClose: () => void;
  onLaunch: (accountId: string, password?: string) => Promise<void>;
}) {
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
      await onLaunch(account.account_id, password);
      // 成功时整页 reload，走不到这里
    } catch {
      // onLaunch 已 setError 并抛出，这里恢复可编辑
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open
      onClose={onClose}
      title={`启动「${formatAccountName(account.display_name)}」`}
      closeOnOverlay={false}
      closeDisabled={busy}
      footer={
        <>
          <button type="button" className="btn-discard" onClick={onClose} disabled={busy}>
            取消
          </button>
          <button
            type="submit"
            form="launch-account-form"
            className="btn-save"
            disabled={busy || !password}
          >
            {busy ? '启动中…' : '启动'}
          </button>
        </>
      }
    >
      <form id="launch-account-form" className="acct-modal-form" onSubmit={submit}>
        <label className="acct-modal-form__field">
          <span>登录密码</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="输入该账户的登录密码"
            maxLength={128}
            autoFocus
            disabled={busy}
          />
        </label>
        {needInit && (
          <p className="acct-modal-form__hint">该账户还没有登录密码，此次输入的密码将设为其登录密码</p>
        )}
        {error && <p className="acct-modal-form__error">{error}</p>}
      </form>
    </Modal>
  );
}
