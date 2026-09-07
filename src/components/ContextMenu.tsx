import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Icon, type IconName } from './Icon';

export interface MenuAction { label: string; icon?: IconName; disabled?: boolean; danger?: boolean; shortcut?: string; run: () => void }
export interface MenuState { x: number; y: number; actions: (MenuAction | null)[] }

export function ContextMenu({ menu, onClose }: { menu: MenuState; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: menu.x, top: menu.y });
  useLayoutEffect(() => {
    const bounds = ref.current?.getBoundingClientRect();
    if (bounds) setPosition({ left: Math.max(8, Math.min(menu.x, window.innerWidth - bounds.width - 8)),
      top: Math.max(8, Math.min(menu.y, window.innerHeight - bounds.height - 8)) });
    ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }, [menu]);
  useEffect(() => {
    const close = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose(); };
    window.addEventListener('pointerdown', close);
    window.addEventListener('resize', onClose);
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('resize', onClose); };
  }, [onClose]);
  return <div className="context-menu" role="menu" ref={ref} style={position} onContextMenu={event => event.preventDefault()}
    onKeyDown={event => {
      if (event.key === 'Escape') { onClose(); return; }
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
      const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 :
        (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
      buttons[next]?.focus();
    }}>
    {menu.actions.map((action, index) => action ? <button key={index} role="menuitem" type="button"
      className={action.danger ? 'danger-text' : ''} disabled={action.disabled}
      onClick={() => { onClose(); action.run(); }}>{action.icon && <Icon name={action.icon} size={16} />}
      <span>{action.label}</span>{action.shortcut && <kbd>{action.shortcut}</kbd>}</button> :
      <div key={index} className="menu-separator" role="separator" />)}
  </div>;
}
