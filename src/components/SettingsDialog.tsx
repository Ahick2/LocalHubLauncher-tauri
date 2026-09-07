import { useState, type FormEvent } from 'react';
import { errorMessage } from '../bridge';
import type { Settings, Snapshot } from '../types';
import { Icon } from './Icon';
import { Modal, Switch } from './Modal';

export function SettingsDialog({ snapshot, onSave, onClose, onOpenDirectory }: {
  snapshot: Snapshot; onSave: (settings: Settings) => Promise<void>; onClose: () => void; onOpenDirectory: () => void;
}) {
  const [draft, setDraft] = useState({ ...snapshot.config.settings });
  const [interval, setInterval] = useState(String(draft.autoStartIntervalMs));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const toggle = (key: keyof Settings, value: boolean) => setDraft(previous => ({
    ...previous, [key]: value, ...(key === 'startWithWindows' && !value ? { startMinimizedToTray: false } : {}),
  }));
  const rows: { key: keyof Settings; title: string; description: string; disabled?: boolean }[] = [
    { key: 'startWithWindows', title: '开机自启动', description: '登录 Windows 后自动打开 Local Hub', disabled: snapshot.platform !== 'windows' },
    { key: 'startMinimizedToTray', title: '开机时隐藏主窗口', description: '仅在开机自启动时生效，手动打开仍显示窗口', disabled: !draft.startWithWindows },
    { key: 'minimizeToTray', title: '最小化到托盘', description: '最小化窗口时收进系统托盘，服务继续运行' },
    { key: 'closeToTray', title: '关闭到托盘', description: '点击窗口关闭按钮时，保持服务在后台运行' },
    { key: 'confirmBeforeStopAll', title: '停止全部前确认', description: '避免误操作中断所有正在运行的服务' },
  ];
  async function submit(event: FormEvent) {
    event.preventDefault();
    const value = Number(interval);
    if (!interval.trim() || !Number.isInteger(value) || value < 0 || value > 10000) {
      setError('自动启动间隔必须是 0 到 10000 之间的整数。'); return;
    }
    setBusy(true); setError('');
    try { await onSave({ ...draft, autoStartIntervalMs: value, startMinimizedToTray: draft.startWithWindows && draft.startMinimizedToTray }); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  return <Modal title="启动器设置" subtitle="按你的习惯，安排启动与窗口行为。" onClose={onClose} busy={busy}>
    <form onSubmit={event => void submit(event)}>
      <div className="modal-body">
        <div className="settings-card">
          <h3>启动与窗口</h3>
          {rows.map(row => <div className="setting-row" key={row.key}><div><strong>{row.title}</strong><p>{row.description}</p></div>
            <Switch label={row.title} checked={Boolean(draft[row.key])} disabled={row.disabled || busy} onChange={value => toggle(row.key, value)} /></div>)}
        </div>
        <div className="settings-card">
          <div className="setting-row"><div><strong>自动启动间隔</strong><p>多个服务依次启动时的等待时间</p></div>
            <label className="interval-control"><input type="number" min={0} max={10000} step={1} aria-label="自动启动间隔"
              value={interval} disabled={busy} onChange={event => setInterval(event.target.value)} /><span>毫秒</span></label>
          </div>
        </div>
        <div className="config-location"><Icon name="folder" /><div><span>配置保存在程序目录</span><code title={snapshot.configPath}>{snapshot.configPath}</code></div>
          <button className="button compact" type="button" onClick={onOpenDirectory}>打开目录</button></div>
        {draft.startWithWindows !== snapshot.autoStartRegistered && <p className="field-hint">保存时会同步 Windows 开机自启动设置。</p>}
        {error && <p className="form-error" role="alert"><Icon name="warning" />{error}</p>}
      </div>
      <footer className="modal-footer"><button className="button" type="button" disabled={busy} onClick={onClose}>取消</button>
        <button className="button primary" type="submit" disabled={busy}>{busy ? '正在保存…' : '保存设置'}</button></footer>
    </form>
  </Modal>;
}
