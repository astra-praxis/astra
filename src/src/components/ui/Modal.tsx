/**
 * 通用弹窗（基于 Radix Dialog）
 *
 * 用法约定（与原 Svelte 版一致）：
 * - 表单类弹窗：closeOnOverlay={false} 防误触
 * - 确认类弹窗：variant="confirm" 更紧凑
 * - 支持 Escape 关闭（closeDisabled 时禁用）
 */
import type { ReactNode } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { X } from 'lucide-react';

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  /** 底部按钮区；不传则不渲染 footer */
  footer?: ReactNode;
  /** "confirm" 更紧凑（420px） */
  variant?: 'default' | 'confirm';
  /** 点击遮罩是否关闭（表单弹窗建议 false 防误触丢输入） */
  closeOnOverlay?: boolean;
  /** 提交中时禁用关闭 */
  closeDisabled?: boolean;
}

export default function Modal({
  open,
  onClose,
  title,
  children,
  footer,
  variant = 'default',
  closeOnOverlay = true,
  closeDisabled = false,
}: ModalProps) {
  return (
    <Dialog.Root
      open={open}
      onOpenChange={(o) => {
        if (!o && !closeDisabled) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className={`modal${variant === 'confirm' ? ' compact' : ''}`}
          onInteractOutside={(e) => {
            if (!closeOnOverlay) e.preventDefault();
          }}
          onEscapeKeyDown={(e) => {
            if (closeDisabled) e.preventDefault();
          }}
        >
          <div className="modal-header">
            <Dialog.Title>{title}</Dialog.Title>
            <Dialog.Close asChild>
              <button className="modal-close" disabled={closeDisabled} aria-label="关闭">
                <X size={18} strokeWidth={1.8} />
              </button>
            </Dialog.Close>
          </div>
          <div className="modal-body">{children}</div>
          {footer && <div className="modal-footer">{footer}</div>}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
