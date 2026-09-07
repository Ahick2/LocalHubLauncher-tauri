import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { api, desktop, errorMessage } from '../bridge';
import { formatLog } from '../state';
import type { LogRecord, Notice, ServiceAction, Snapshot } from '../types';
import { useLogs } from '../useLogs';
import { ContextMenu, type MenuState } from './ContextMenu';
import { Icon } from './Icon';

const ROW_HEIGHT = 24;

function LogText({ text, onLink }: { text: string; onLink: (url: string) => void }) {
  return <>{text.split(/(https?:\/\/[^\s<>"']+)/g).map((part, index) => /^https?:\/\//.test(part)
    ? <a href={part} key={index} title="Ctrl + 点击打开网址" onClick={event => {
      event.preventDefault(); if (event.ctrlKey || event.metaKey) onLink(part);
    }}>{part}</a> : <span key={index}>{part}</span>)}</>;
}

export function LogView({ snapshot, activeId, onSelect, onAction, notify }: {
  snapshot: Snapshot; activeId: string | null; onSelect: (id: string | null) => void;
  onAction: (ids: string[], action: ServiceAction) => void; notify: (notice: Notice) => void;
}) {
  const { entries, loading, reload, clear } = useLogs(activeId, notify);
  const [search, setSearch] = useState('');
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  const [allSelected, setAllSelected] = useState(false);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const filtered = useMemo(() => entries.filter(entry =>
    (!errorsOnly || entry.stream === 'stderr') &&
    (!search || (entry.text + ' ' + entry.itemName).toLocaleLowerCase().includes(search.toLocaleLowerCase()))),
  [entries, search, errorsOnly]);
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 8);
  const end = Math.min(filtered.length, start + Math.ceil(height / ROW_HEIGHT) + 20);

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { followingRef.current = true; setFollowing(true); setAllSelected(false); }, [activeId]);
  useLayoutEffect(() => {
    if (followingRef.current && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [filtered, height]);

  const copy = async (text: string) => {
    if (!text) return;
    try { await navigator.clipboard.writeText(text); notify({ level: 'success', message: '日志已复制' }); }
    catch (error) { notify({ level: 'error', message: '复制失败：' + errorMessage(error) }); }
  };
  const textOf = (records: LogRecord[]) => records.map(record => formatLog(record, activeId === null)).join('\n');
  const openLink = (url: string) => { void api.openUrl(url).catch(error => notify({ level: 'error', message: errorMessage(error) })); };
  const toggleFollowing = () => {
    followingRef.current = !followingRef.current; setFollowing(followingRef.current);
    if (followingRef.current && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  };
  const logContext = (x: number, y: number) => {
    const selectedText = allSelected ? textOf(filtered) : window.getSelection()?.toString() ?? '';
    setMenu({ x, y, actions: [
      { label: '复制', icon: 'copy', disabled: !selectedText, shortcut: 'Ctrl+C', run: () => void copy(selectedText) },
      { label: '复制全部', icon: 'copy', disabled: entries.length === 0, run: () => void copy(textOf(entries)) },
      { label: '全选当前结果', icon: 'check', disabled: filtered.length === 0, shortcut: 'Ctrl+A', run: () => { setAllSelected(true); viewport.current?.focus(); } },
      null,
      { label: '刷新日志', icon: 'refresh', disabled: loading, run: () => void reload() },
      { label: '清空当前日志', icon: 'trash', disabled: entries.length === 0 || loading, run: () => void clear() },
    ] });
  };

  return <section className="logs-card" aria-label="服务日志">
    <div className="log-tabs" role="tablist" aria-label="日志来源">
      <button className={activeId === null ? 'active' : ''} role="tab" aria-selected={activeId === null} onClick={() => onSelect(null)}>
        <Icon name="terminal" size={16} />汇总日志</button>
      {snapshot.logTabs.map(tab => <button key={tab.id} role="tab" aria-selected={activeId === tab.id}
        className={activeId === tab.id ? 'active' : ''} onClick={() => onSelect(tab.id)}
        onContextMenu={event => {
          event.preventDefault(); onSelect(tab.id);
          setMenu({ x: event.clientX, y: event.clientY, actions: [
            { label: '重新启动', icon: 'restart', run: () => onAction([tab.id], 'restart') },
            { label: '停止服务', icon: 'stop', run: () => onAction([tab.id], 'stop') },
          ] });
        }} title={tab.name}><i className={'tab-dot ' + (snapshot.statuses.find(status => status.itemId === tab.id)?.state ?? 'stopped')} />
        {snapshot.config.items.find(item => item.id === tab.id)?.name ?? tab.name}</button>)}
    </div>
    <div className="log-toolbar">
      <label className="search-box"><Icon name="search" size={16} /><input aria-label="搜索日志" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索日志内容…" /></label>
      <label className="error-filter"><input type="checkbox" checked={errorsOnly} onChange={event => setErrorsOnly(event.target.checked)} />仅错误</label>
      <div className="log-tools">
        <button className={'button compact ' + (following ? 'subtle-active' : '')} onClick={toggleFollowing}><Icon name="down" size={14} />{following ? '自动跟随' : '恢复跟随'}</button>
        <button className="icon-button" title="刷新日志" aria-label="刷新日志" onClick={() => void reload()} disabled={loading || !desktop}><Icon name="refresh" size={17} /></button>
        <button className="icon-button" title="复制全部日志" aria-label="复制全部日志" onClick={() => void copy(textOf(entries))} disabled={!entries.length}><Icon name="copy" size={17} /></button>
        <button className="icon-button" title="清空当前日志" aria-label="清空当前日志" onClick={() => void clear()} disabled={loading || !entries.length}><Icon name="trash" size={17} /></button>
      </div>
    </div>
    <div ref={viewport} className={'log-viewport ' + (allSelected ? 'all-selected' : '')} tabIndex={0} role="log" aria-label="日志内容" aria-live="off"
      onPointerDown={() => setAllSelected(false)}
      onScroll={event => {
        const element = event.currentTarget;
        setScrollTop(element.scrollTop);
        const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 32;
        followingRef.current = atBottom; setFollowing(atBottom);
      }}
      onKeyDown={event => {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); event.stopPropagation(); setAllSelected(true); }
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c' && allSelected) { event.preventDefault(); void copy(textOf(filtered)); }
      }}
      onContextMenu={event => { event.preventDefault(); logContext(event.clientX, event.clientY); }}>
      {filtered.length === 0 ? <div className="empty-logs"><Icon name="terminal" size={32} /><h3>{loading ? '正在读取日志…' : entries.length ? '没有匹配的日志' : '等待服务输出'}</h3>
        <p>{entries.length ? '调整关键词或错误筛选条件。' : '启动服务后，输出会实时出现在这里。'}</p></div> :
        <div className="log-canvas" style={{ height: filtered.length * ROW_HEIGHT }}>
          {filtered.slice(start, end).map((record, offset) => <div className={'log-row ' + record.stream} key={record.sequence} style={{ top: (start + offset) * ROW_HEIGHT }}>
            <span className="log-time">{new Date(record.timestamp).toLocaleTimeString('zh-CN', { hour12: false })}</span>
            {activeId === null && <span className="log-service">[{record.itemName}] </span>}
            <span className="log-message"><LogText text={record.text} onLink={openLink} /></span>
          </div>)}
        </div>}
    </div>
    <footer className="log-footer"><span><i className={'live-dot ' + (following ? '' : 'paused')} />{following ? '实时输出' : '已暂停跟随'}<span className="footer-divider">/</span>{filtered.length} 行</span>
      <span>内存滚动缓冲 · Ctrl + 点击打开网址</span></footer>
    {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
  </section>;
}
