/**
 * 更换登录密码弹窗（当前账户）。
 * 旧密码 + 新密码；KEK 不变（仅重新包装），已加密数据无需重加密。
 * 账户菜单（侧栏底部）与设置-账户分区共用。
 */
import { useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import Modal from '../ui/Modal';

export default function ChangePasswordModal({
  open,
  onClose,
  hasPassword,
  onChanged,
}: {
  open: boolean;
  onClose: () => void;
  /** 当前账户是否已设密码（未设时旧密码框提示初始密码） */
  hasPassword: boolean;
  onChanged: () => void;
}) {
  const [oldPassword, setOldPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const ready = oldPassword && newPassword && newPassword === confirm;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ready || busy) return;
    if (newPassword !== confirm) {
      setError('两次输入的新密码不一致');
      return;
    }
    setBusy(true);
    setError('');
    try {
      await invoke('account_password_change', { oldPassword, newPassword });
      setOldPassword('');
      setNewPassword('');
      setConfirm('');
      onChanged();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="更换登录密码"
      closeOnOverlay={false}
      closeDisabled={busy}
      footer={
        <>
          <button type="button" className="btn-discard" onClick={onClose} disabled={busy}>
            取消
          </button>
          <button type="submit" form="change-password-form" className="btn-save" disabled={busy || !ready}>
            {busy ? '提交中…' : '确认更换'}
          </button>
        </>
      }
    >
      <form id="change-password-form" className="acct-modal-form" onSubmit={submit}>
        <label className="acct-modal-form__field">
          <span>当前密码</span>
          <input
            type="password"
            value={oldPassword}
            onChange={(e) => setOldPassword(e.target.value)}
            placeholder={hasPassword ? '输入当前登录密码' : '未设过密码则填启动时设置的初始密码'}
            maxLength={128}
            autoFocus
            disabled={busy}
          />
        </label>
        <label className="acct-modal-form__field">
          <span>新密码</span>
          <input
            type="password"
            value={newPassword}
            onChange={(e) => setNewPassword(e.target.value)}
            placeholder="6-128 位"
            maxLength={128}
            disabled={busy}
          />
        </label>
        <label className="acct-modal-form__field">
          <span>确认新密码</span>
          <input
            type="password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            placeholder="再次输入新密码"
            maxLength={128}
            disabled={busy}
          />
        </label>
        {newPassword && confirm && newPassword !== confirm && (
          <p className="acct-modal-form__error">两次输入的新密码不一致</p>
        )}
        {error && <p className="acct-modal-form__error">{error}</p>}
        <p className="acct-modal-form__hint">仅本地验证，不同步到云端；下次启动用新密码登录</p>
      </form>
    </Modal>
  );
}
