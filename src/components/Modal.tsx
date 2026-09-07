import { useEffect, useRef, type ReactNode } from 'react';
import { Icon } from './Icon';

export function Modal({ title, children, onClose, busy = false, className = '' }: {
  title: string; children: ReactNode; onClose: () => void; busy?: boolean; className?: string;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);
  return <dialog ref={dialog} className={'modal ' + className} aria-labelledby="modal-title"
    onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <header className="modal-heading">
      <h2 id="modal-title">{title}</h2>
      <button type="button" className="icon-button" aria-label="关闭对话框" onClick={onClose} disabled={busy}><Icon name="close" /></button>
    </header>
    {children}
  </dialog>;
}

export function Switch({ checked, onChange, label, disabled = false }: {
  checked: boolean; onChange: (value: boolean) => void; label: string; disabled?: boolean;
}) {
  return <button type="button" role="switch" aria-checked={checked} aria-label={label}
    className={'switch ' + (checked ? 'checked' : '')} disabled={disabled} onClick={() => onChange(!checked)}><span /></button>;
}
