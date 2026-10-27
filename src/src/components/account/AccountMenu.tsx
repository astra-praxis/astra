/**
 * 侧栏底部用户入口（praxis 式）：头像 + 名称，点击弹出菜单。
 * 菜单：账户设置（深链 /settings/accounts）、更换登录密码（弹窗）、
 * 切换用户（account_logout 冷切换回登录列表）。
 * legacy 模式（无绑定账户）不渲染。
 */
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { KeyRound, LogOut, Settings2 } from 'lucide-react';
import ChangePasswordModal from './ChangePasswordModal';
import './AccountMenu.scss';

interface AccountStatus {
  mode: 'account' | 'legacy';
  account_id: string | null;
  display_name: string | null;
  accounts: unknown[];
  password_protected: boolean;
  locked: boolean;
}

export default function AccountMenu({ collapsed }: { collapsed: boolean }) {
  const navigate = useNavigate();
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [pwOpen, setPwOpen] = useState(false);
  const [switching, setSwitching] = useState(false);

  const refresh = () =>
    invoke<AccountStatus>('account_status').then(setStatus).catch(() => {});

  useEffect(() => {
    void refresh();
  }, []);

  if (!status || status.mode !== 'account') return null;

  const name = status.display_name ?? '账户';

  async function switchUser() {
    if (switching) return;
    setSwitching(true);
    try {
      // 成功时后端 spawn 新进程并 exit(0)，Promise 不会 resolve
      await invoke('account_logout');
    } catch (e) {
      console.error('account_logout failed', e);
      setSwitching(false);
    }
  }

  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <button
          type="button"
          className={`acct-menu__trigger${collapsed ? ' acct-menu__trigger--collapsed' : ''}`}
          title={name}
        >
          <span className="acct-menu__avatar" aria-hidden>
            {name.slice(0, 1)}
          </span>
          {!collapsed && <span className="acct-menu__name">{name}</span>}
        </button>
      </DropdownMenu.Trigger>

      <DropdownMenu.Portal>
        <DropdownMenu.Content className="acct-menu__content" sideOffset={8} align="start">
          <div className="acct-menu__head">
            <span className="acct-menu__avatar" aria-hidden>
              {name.slice(0, 1)}
            </span>
            <div className="acct-menu__meta">
              <span className="acct-menu__meta-name">{name}</span>
              <span className="acct-menu__meta-sub">本地账户</span>
            </div>
          </div>
          <DropdownMenu.Item
            className="acct-menu__item"
            onSelect={() => navigate('/settings/accounts')}
          >
            <Settings2 size={15} strokeWidth={1.8} />
            账户设置
          </DropdownMenu.Item>
          <DropdownMenu.Item className="acct-menu__item" onSelect={() => setPwOpen(true)}>
            <KeyRound size={15} strokeWidth={1.8} />
            更换登录密码
          </DropdownMenu.Item>
          <DropdownMenu.Separator className="acct-menu__sep" />
          <DropdownMenu.Item
            className="acct-menu__item acct-menu__item--danger"
            disabled={switching}
            onSelect={() => void switchUser()}
          >
            <LogOut size={15} strokeWidth={1.8} />
            {switching ? '切换中…' : '切换用户'}
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>

      <ChangePasswordModal
        open={pwOpen}
        onClose={() => setPwOpen(false)}
        hasPassword={status.password_protected}
        onChanged={refresh}
      />
    </DropdownMenu.Root>
  );
}
