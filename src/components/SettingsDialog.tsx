import { useState, type FormEvent } from 'react';
import { api, errorMessage } from '../bridge';
import type { ProxyMode, ProxyTestResult, Settings, Snapshot } from '../types';
import { Icon } from './Icon';
import { Modal, Switch } from './Modal';

export function SettingsDialog({ snapshot, onSave, onClose, onOpenDirectory }: {
  snapshot: Snapshot; onSave: (settings: Settings) => Promise<void>; onClose: () => void; onOpenDirectory: () => void;
}) {
  const [draft, setDraft] = useState({ ...snapshot.config.settings });
  const [interval, setInterval] = useState(String(draft.autoStartIntervalMs));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ProxyTestResult | null>(null);
  const toggle = (key: keyof Settings, value: boolean) => {
    setSaved(false);
    setDraft(previous => ({
      ...previous, [key]: value, ...(key === 'startWithWindows' && !value ? { startMinimizedToTray: false } : {}),
    }));
  };
  const changeProxy = <K extends keyof Settings['proxy']>(key: K, value: Settings['proxy'][K]) => {
    setTestResult(null);
    setSaved(false);
    setDraft(previous => ({ ...previous, proxy: { ...previous.proxy, [key]: value } }));
  };
  const proxySummary = draft.proxy.mode === 'custom'
    ? draft.proxy.url.trim() || '自定义代理'
    : draft.proxy.mode === 'system' ? 'Windows 系统代理' : '不使用代理';
  async function testConnection() {
    setError('');
    if (draft.proxy.mode === 'custom' && !draft.proxy.url.trim()) {
      setError('请先填写代理地址，再测试连接。'); return;
    }
    if (!draft.proxy.testUrl.trim()) {
      setError('请填写测试网站地址。'); return;
    }
    setTesting(true); setTestResult(null);
    try { setTestResult(await api.testProxy(draft.proxy.testUrl, draft.proxy)); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setTesting(false); }
  }
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
    if (draft.proxy.mode === 'custom' && !draft.proxy.url.trim()) {
      setError('请填写自定义代理的地址，或把默认代理改为不使用。'); return;
    }
    setBusy(true); setError(''); setSaved(false);
    try {
      await onSave({ ...draft, autoStartIntervalMs: value, startMinimizedToTray: draft.startWithWindows && draft.startMinimizedToTray });
      setSaved(true);
    }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  return <Modal title="启动器设置" onClose={onClose} busy={busy}>
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
              value={interval} disabled={busy} onChange={event => { setInterval(event.target.value); setSaved(false); }} /><span>毫秒</span></label>
          </div>
        </div>
        <details className="settings-card settings-collapsible">
          <summary>
            <h3>网络代理</h3>
            <span className="collapse-hint">{proxySummary}</span>
            <Icon name="chevron" size={16} className="collapse-icon" />
          </summary>
          <div className="settings-collapsible-body">
            <div className="setting-row"><div><strong>默认代理</strong><p>新启动的服务默认使用的代理方式</p></div>
              <label className="setting-control"><select value={draft.proxy.mode} disabled={busy}
                onChange={event => changeProxy('mode', event.target.value as ProxyMode)}>
                <option value="none">不使用代理</option>
                <option value="system">Windows 系统代理</option>
                <option value="custom">自定义代理</option>
              </select></label>
            </div>
            {draft.proxy.mode === 'custom' && <>
              <div className="setting-row"><div><strong>代理地址</strong><p>支持 http、https、socks5，可带用户名和密码</p></div>
                <label className="setting-control"><input spellCheck={false} value={draft.proxy.url} disabled={busy}
                  placeholder="http://127.0.0.1:7890" onChange={event => changeProxy('url', event.target.value)} /></label>
              </div>
              <div className="setting-row"><div><strong>不走代理的地址</strong><p>逗号或分号分隔，留空表示全部走代理</p></div>
                <label className="setting-control"><input spellCheck={false} value={draft.proxy.noProxy} disabled={busy}
                  placeholder=".internal, 10.0.0.0/8" onChange={event => changeProxy('noProxy', event.target.value)} /></label>
              </div>
            </>}
            <p className="field-hint">{draft.proxy.mode === 'system'
              ? '启动时读取当前 Windows 的 Internet 代理设置；PAC 脚本无法转换为固定地址，需要改用自定义。'
              : draft.proxy.mode === 'custom'
                ? '代理会写入子进程的 HTTP_PROXY、HTTPS_PROXY 与 ALL_PROXY；地址中的凭据对子进程可见。'
                : '不设置代理变量，并清除从启动器所在终端继承的同名变量。'}</p>
            <div className="proxy-test-block">
              <strong>测试连接</strong>
              <p>使用上方的代理设置访问测试网站</p>
              <div className="proxy-test">
                <input spellCheck={false} value={draft.proxy.testUrl} disabled={busy || testing}
                  aria-label="测试网站" placeholder="https://www.google.com"
                  onChange={event => changeProxy('testUrl', event.target.value)} />
                <button type="button" className="button compact" disabled={busy || testing}
                  onClick={() => void testConnection()}>{testing ? '测试中…' : '测试'}</button>
              </div>
            </div>
            {testResult && <p className={'proxy-result ' + (testResult.ok ? 'ok' : 'failed')} role="status">
              <Icon name={testResult.ok ? 'check' : 'warning'} size={13} />
              <span>{testResult.message} · {testResult.elapsedMs} ms · {testResult.effectiveProxy ?? '直连'}</span>
            </p>}
          </div>
        </details>
        <div className="config-location"><Icon name="folder" /><div><span>配置保存在程序目录</span><code title={snapshot.configPath}>{snapshot.configPath}</code></div>
          <button className="button compact" type="button" onClick={onOpenDirectory}>打开目录</button></div>
        {draft.startWithWindows !== snapshot.autoStartRegistered && <p className="field-hint">保存时会同步 Windows 开机自启动设置。</p>}
        {error && <p className="form-error" role="alert"><Icon name="warning" />{error}</p>}
      </div>
      <footer className="modal-footer">{saved && <span className="footer-saved" role="status"><Icon name="check" size={13} />已保存</span>}
        <button className="button" type="button" disabled={busy} onClick={onClose}>{saved ? '关闭' : '取消'}</button>
        <button className="button primary" type="submit" disabled={busy}>{busy ? '正在保存…' : '保存设置'}</button></footer>
    </form>
  </Modal>;
}
