import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { api, desktop, errorMessage, onNotice, onSnapshot } from './bridge';
import { acceptSnapshot, isRunning, selectRow, sortedItems, visibleItems, type SortKey } from './state';
import { emptySnapshot, newItem, type ActionResult, type LaunchItem, type Notice, type ServiceAction, type Snapshot, type View } from './types';
import { ContextMenu, type MenuState } from './components/ContextMenu';
import { Icon, type IconName } from './components/Icon';
import { ItemDialog } from './components/ItemDialog';
import { LogView } from './components/LogView';
import { Modal } from './components/Modal';
import { ServiceTable } from './components/ServiceTable';
import { SettingsDialog } from './components/SettingsDialog';

const pages: Record<View, { title: string; description: string; icon: IconName }> = {
  all: { title: '全部启动项', description: '让每一个本地服务，都井然有序。', icon: 'grid' },
  running: { title: '正在运行', description: '关注正在工作的服务，随时掌握运行状态。', icon: 'activity' },
  auto: { title: '自动启动', description: '打开 Local Hub，让常用服务依次就绪。', icon: 'bolt' },
  logs: { title: '服务日志', description: '每一次输出，都有迹可循。', icon: 'terminal' },
};
interface Editor { item: LaunchItem; editing: boolean; remaining: LaunchItem[] }
interface Confirmation { title: string; message: string; label: string; run: () => Promise<void> }

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot>(emptySnapshot);
  const [loading, setLoading] = useState(desktop);
  const [view, setView] = useState<View>('all');
  const [search, setSearch] = useState('');
  const [sortKey, setSortKey] = useState<SortKey>('name');
  const [descending, setDescending] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [anchor, setAnchor] = useState<string | null>(null);
  const [busy, setBusy] = useState('');
  const [editor, setEditor] = useState<Editor | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [activeLog, setActiveLog] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const dropHandler = useRef<(paths: string[]) => Promise<void>>(async () => {});
  const notify = useCallback((notice: Notice) => setNotice({ ...notice }), []);
  const applySnapshot = useCallback((incoming: Snapshot) =>
    setSnapshot(current => acceptSnapshot(current.revision, incoming.revision) ? incoming : current), []);
  const statuses = useMemo(() => new Map(snapshot.statuses.map(status => [status.itemId, status])), [snapshot.statuses]);
  const rows = useMemo(() => sortedItems(visibleItems(snapshot.config.items, statuses, view, search), statuses, sortKey, descending),
    [snapshot.config.items, statuses, view, search, sortKey, descending]);
  const rowIds = rows.map(item => item.id);
  const visibleIdentity = [...rowIds].sort().join(',');
  const runningCount = snapshot.statuses.filter(isRunning).length;
  const counts = { all: snapshot.config.items.length, running: runningCount,
    auto: snapshot.config.items.filter(item => item.enabled && item.autoStart).length, logs: snapshot.logTabs.length };
  const disabled = !!busy || loading || !!snapshot.loadError || !desktop;
  const primary = rows.find(item => item.id === anchor && selected.has(item.id)) ?? rows.find(item => selected.has(item.id));

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => { if (cancelled) stop(); else stops.push(stop); };
    void Promise.all([
      onSnapshot(applySnapshot).then(keep), onNotice(notify).then(keep),
      getCurrentWebviewWindow().onDragDropEvent(event => {
        if (event.payload.type === 'drop') { setDragging(false); void dropHandler.current(event.payload.paths); }
        else setDragging(event.payload.type !== 'leave');
      }).then(keep),
    ]).then(async () => {
      const state = await api.snapshot();
      if (!cancelled) applySnapshot(state);
    }).catch(error => notify({ level: 'error', message: errorMessage(error) }))
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; stops.forEach(stop => stop()); };
  }, [applySnapshot, notify]);
  useEffect(() => {
    const visible = new Set(visibleIdentity.split(','));
    setSelected(previous => {
      const next = new Set([...previous].filter(id => visible.has(id)));
      return next.size === previous.size ? previous : next;
    });
  }, [visibleIdentity]);
  useEffect(() => {
    if (activeLog && !snapshot.config.items.some(item => item.id === activeLog)) setActiveLog(null);
  }, [activeLog, snapshot.config.items]);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), notice.level === 'error' ? 12000 : 5000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  async function run(label: string, work: () => Promise<void>) {
    if (!desktop) return;
    setBusy(label);
    try { await work(); } catch (error) { notify({ level: 'error', message: errorMessage(error) }); }
    finally { setBusy(''); }
  }
  function report(result: ActionResult, success: string) {
    applySnapshot(result.snapshot);
    notify(result.failures.length
      ? { level: 'error', message: result.failures.slice(0, 3).map(failure => failure.name + '：' + failure.error).join('\n') +
        (result.failures.length > 3 ? '\n另有 ' + (result.failures.length - 3) + ' 项失败' : '') }
      : { level: 'success', message: success });
  }
  function perform(ids: string[], action: ServiceAction) {
    if (!ids.length || disabled) return;
    const label = { start: '启动', stop: '停止', restart: '重启' }[action];
    void run('正在' + label + '服务…', async () => report(await api.action(ids, action), label + '操作完成'));
  }
  function addItem() {
    if (disabled) return;
    setEditor({ item: newItem(view === 'auto'), editing: false, remaining: [] });
  }
  function editItem(item: LaunchItem) {
    if (disabled) return;
    if (isRunning(statuses.get(item.id))) { notify({ level: 'info', message: '请先停止此服务，再修改启动配置。' }); return; }
    setEditor({ item: { ...item }, editing: true, remaining: [] });
  }
  function finishEditor() {
    setEditor(previous => previous?.remaining.length ? { item: previous.remaining[0], editing: false, remaining: previous.remaining.slice(1) } : null);
  }
  async function saveItem(item: LaunchItem) {
    applySnapshot(await api.saveItem(item));
    setSelected(new Set([item.id])); setAnchor(item.id); setSearch('');
    notify({ level: 'success', message: '已保存「' + item.name + '」' });
    finishEditor();
  }
  function duplicate(item: LaunchItem) {
    void run('正在复制启动项…', async () => {
      const copy = { ...item, id: crypto.randomUUID(), name: item.name + ' - 副本', autoStart: false };
      applySnapshot(await api.saveItem(copy));
      setSelected(new Set([copy.id])); setAnchor(copy.id);
      notify({ level: 'success', message: '已复制「' + item.name + '」' });
    });
  }
  const selectAll = () => setSelected(rowIds.length && rowIds.every(id => selected.has(id)) ? new Set() : new Set(rowIds));
  function showLog(id: string) {
    setActiveLog(id); setView('logs'); setSearch(''); setMenu(null);
  }
  function requestDelete(ids: string[]) {
    if (!ids.length || disabled) return;
    setConfirmation({ title: '删除启动项', label: '确认删除',
      message: '确定删除选中的 ' + ids.length + ' 个启动项？\n正在运行的服务会先停止，程序文件不会被删除。',
      run: () => run('正在删除启动项…', async () => report(await api.deleteItems(ids), '已删除所选启动项')) });
  }
  function stopAll() {
    const stop = () => run('正在停止全部服务…', async () => report(await api.stopAll(), '全部服务已停止，待启动队列已取消'));
    if (snapshot.config.settings.confirmBeforeStopAll && runningCount > 0) {
      setConfirmation({ title: '停止全部服务', label: '确认停止', message: '确定停止全部 ' + runningCount + ' 个服务，并取消待启动的自动项？', run: stop });
    } else void stop();
  }
  const reload = () => run('正在重新读取配置…', async () => { applySnapshot(await api.reload()); notify({ level: 'success', message: '已重新读取配置文件' }); });
  const openDirectory = (id: string | null) => void run('正在打开目录…', () => api.openDirectory(id));
  function itemMenu(item: LaunchItem, x: number, y: number) {
    if (disabled) return;
    const selection = selectRow(selected, item.id, rowIds, anchor, { context: true });
    setSelected(selection); setAnchor(item.id);
    const ids = [...selection];
    const allAuto = snapshot.config.items.filter(entry => selection.has(entry.id)).every(entry => entry.autoStart);
    setMenu({ x, y, actions: [
      { label: '启动', icon: 'play', run: () => perform(ids, 'start') },
      { label: '停止', icon: 'stop', run: () => perform(ids, 'stop') },
      { label: '重新启动', icon: 'restart', run: () => perform(ids, 'restart') },
      { label: '查看独立日志', icon: 'terminal', run: () => showLog(item.id) }, null,
      { label: '打开服务网址', icon: 'link', disabled: !item.url, run: () => void run('正在打开网址…', () => api.openUrl(item.url)) },
      { label: '打开工作目录', icon: 'folder', run: () => openDirectory(item.id) }, null,
      { label: '编辑', icon: 'edit', shortcut: 'Enter', disabled: isRunning(statuses.get(item.id)), run: () => editItem(item) },
      { label: allAuto ? '取消自启动' : '设置为自启动', icon: 'bolt', run: () => void run('正在保存…', async () => {
        applySnapshot(await api.setAutoStart(ids, !allAuto)); notify({ level: 'success', message: allAuto ? '已取消自启动' : '已设置为自启动' });
      }) },
      { label: '删除', icon: 'trash', shortcut: 'Del', danger: true, run: () => requestDelete(ids) },
    ] });
  }
  const blankMenu = (x: number, y: number) => setMenu({ x, y, actions: [
    { label: '添加启动项', icon: 'plus', shortcut: 'Ctrl+N', disabled, run: addItem },
    { label: '全选', icon: 'check', shortcut: 'Ctrl+A', disabled: !rows.length, run: selectAll }, null,
    { label: '重新读取配置', icon: 'refresh', shortcut: 'F5', disabled: !!busy || !desktop, run: () => void reload() },
  ] });

  dropHandler.current = async paths => {
    if (disabled || editor || settingsOpen || confirmation) { notify({ level: 'info', message: '请先完成当前操作，再拖入文件。' }); return; }
    await run('正在识别启动文件…', async () => {
      const result = await api.dropped(paths);
      if (result.failures.length) notify({ level: 'error', message: result.failures.join('\n') });
      if (result.items.length) setEditor({ item: { ...result.items[0], autoStart: view === 'auto' }, editing: false,
        remaining: result.items.slice(1).map(item => ({ ...item, autoStart: view === 'auto' })) });
    });
  };

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (document.querySelector('dialog[open]')) return;
      if (event.key === 'F5') { event.preventDefault(); if (!busy && desktop) void reload(); return; }
      const target = event.target as Element | null;
      if (target?.closest('input,textarea,select,button,a,[contenteditable="true"]')) return;
      const control = event.ctrlKey || event.metaKey;
      if (control && event.key.toLowerCase() === 'n') { event.preventDefault(); addItem(); }
      if (view === 'logs') return;
      if (control && event.key.toLowerCase() === 'a') { event.preventDefault(); selectAll(); }
      if (control && event.key.toLowerCase() === 'd') { event.preventDefault(); if (primary && !disabled) duplicate(primary); }
      if (event.key === 'Enter' && primary) { event.preventDefault(); editItem(primary); }
      if (event.key === 'Delete') { event.preventDefault(); requestDelete([...selected]); }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  });

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><img src="/app.ico" width={36} height={36} alt="" /><div><strong>LOCAL HUB</strong><span>本地服务控制中心</span></div></div>
      <div className="nav-caption">工作空间</div>
      <nav aria-label="主导航">{(Object.keys(pages) as View[]).map(key => <button key={key} className={'nav-item ' + (view === key ? 'active' : '')}
        aria-current={view === key ? 'page' : undefined} onClick={() => { setView(key); setSearch(''); setMenu(null); }}>
        <Icon name={pages[key].icon} /><span>{pages[key].title}</span><b>{counts[key]}</b></button>)}</nav>
      <div className="sidebar-bottom">
        <div className="workspace-status"><i className={'live-dot ' + (!runningCount ? 'paused' : '')} /><span>{runningCount ? runningCount + ' 个服务正在运行' : '工作空间已就绪'}</span></div>
        <button className="nav-item settings-nav" onClick={() => setSettingsOpen(true)} disabled={disabled}><Icon name="settings" /><span>启动器设置</span></button>
        <div className="sidebar-footnote"><span>Local Hub <b>2.0</b></span><button className="icon-button" aria-label="退出 Local Hub" title="退出 Local Hub" disabled={!desktop || !!busy} onClick={() => void api.exit()}><Icon name="exit" size={16} /></button></div>
      </div>
    </aside>
    <main className="workspace">
      <header className="page-header"><div><div className="eyebrow">LOCAL WORKSPACE</div><h1>{pages[view].title}</h1><p>{pages[view].description}</p></div>
        <div className="header-actions">{snapshot.autoStarting && <span className="auto-progress"><i />正在依次启动</span>}
          {view !== 'logs' && <label className="search-box main-search"><Icon name="search" size={17} /><input aria-label="搜索启动项" value={search} placeholder="搜索名称、分类或命令"
            onChange={event => setSearch(event.target.value)} />{search && <button className="icon-button" aria-label="清除搜索" onClick={() => setSearch('')}><Icon name="close" size={14} /></button>}</label>}
          <button className="button square" title="重新读取配置 (F5)" aria-label="重新读取配置" disabled={!!busy || loading || !desktop} onClick={() => void reload()}><Icon name="refresh" className={busy.includes('配置') ? 'spinning' : ''} /></button>
        </div>
      </header>
      {!desktop && <div className="preview-banner"><Icon name="info" />当前为界面预览。请启动桌面应用来管理本地服务。</div>}
      {snapshot.loadError && <div className="config-error" role="alert"><Icon name="warning" size={24} /><div><h3>配置需要修复</h3><p>{snapshot.loadError}</p>
        <button className="button compact" onClick={() => void reload()} disabled={!!busy}>重新读取</button>
        <button className="button compact" onClick={() => openDirectory(null)}>打开配置目录</button></div></div>}
      {view !== 'logs' && <div className="toolbar" role="toolbar" aria-label="启动项操作">
        <div className="toolbar-group">
          <button className="button primary" onClick={addItem} disabled={disabled}><Icon name="plus" />{view === 'auto' ? '添加自动项' : '添加启动项'}</button>
          <span className="toolbar-divider" />
          <button className="button" onClick={() => perform([...selected], 'start')} disabled={disabled || !selected.size}><Icon name="play" size={15} />启动</button>
          <button className="button" onClick={() => perform([...selected], 'stop')} disabled={disabled || !selected.size}><Icon name="stop" size={14} />停止</button>
          <button className="button" onClick={() => perform([...selected], 'restart')} disabled={disabled || !selected.size}><Icon name="restart" size={16} />重启</button>
        </div>
        <div className="toolbar-group">
          <button className="button text-button" onClick={() => perform(rows.filter(item => item.enabled).map(item => item.id), 'start')} disabled={disabled || !rows.some(item => item.enabled)}><Icon name="play" size={14} />启动当前列表</button>
          <button className="button text-button danger-hover" onClick={stopAll} disabled={disabled || (!runningCount && !snapshot.autoStarting)}><Icon name="stop" size={14} />停止全部</button>
        </div>
      </div>}
      <div className="page-content">{view === 'logs'
        ? <LogView snapshot={snapshot} activeId={activeLog} onSelect={setActiveLog} onAction={perform} notify={notify} />
        : <ServiceTable items={rows} statuses={statuses} selected={selected} sortKey={sortKey} descending={descending}
          view={view} search={search} disabled={disabled}
          onSort={key => { setDescending(sortKey === key ? !descending : false); setSortKey(key); }}
          onSelect={(id, event) => { setSelected(previous => selectRow(previous, id, rowIds, anchor,
            { toggle: event.ctrlKey || event.metaKey, range: event.shiftKey })); if (!event.shiftKey) setAnchor(id); }}
          onSelectAll={selectAll} onEdit={editItem} onMenu={itemMenu} onBlankMenu={blankMenu} onAdd={addItem} />}</div>
      <footer className="status-bar"><span aria-live="polite"><i className={'live-dot ' + (busy || loading ? 'working' : 'paused')} />{busy || (loading ? '正在连接工作空间…' : '就绪')}</span>
        <span><kbd>Ctrl</kbd> + <kbd>N</kbd> 添加<span className="footer-divider">·</span>拖入文件或文件夹快速创建</span></footer>
    </main>
    {notice && <div className={'toast ' + notice.level} role={notice.level === 'error' ? 'alert' : 'status'}><Icon name={notice.level === 'error' ? 'warning' : notice.level === 'success' ? 'check' : 'info'} />
      <span>{notice.message}</span><button className="icon-button" aria-label="关闭提示" onClick={() => setNotice(null)}><Icon name="close" size={15} /></button></div>}
    {dragging && <div className="drop-overlay"><div><Icon name="upload" size={40} /><h2>松开即可添加启动项</h2><p>支持程序、脚本和文件夹</p></div></div>}
    {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
    {editor && <ItemDialog key={editor.item.id} item={editor.item} editing={editor.editing} onSave={saveItem} onClose={finishEditor} />}
    {settingsOpen && <SettingsDialog snapshot={snapshot} onClose={() => setSettingsOpen(false)} onOpenDirectory={() => openDirectory(null)}
      onSave={async settings => { applySnapshot(await api.saveSettings(settings)); setSettingsOpen(false); notify({ level: 'success', message: '设置已保存' }); }} />}
    {confirmation && <Modal title={confirmation.title} onClose={() => setConfirmation(null)} busy={!!busy} className="confirmation-modal">
      <div className="confirmation-body"><div className="confirmation-icon"><Icon name="warning" size={26} /></div><p>{confirmation.message}</p></div>
      <footer className="modal-footer"><button className="button" disabled={!!busy} onClick={() => setConfirmation(null)}>取消</button>
        <button className="button destructive" disabled={!!busy} onClick={() => { void confirmation.run().finally(() => setConfirmation(null)); }}>{busy || confirmation.label}</button></footer>
    </Modal>}
  </div>;
}
