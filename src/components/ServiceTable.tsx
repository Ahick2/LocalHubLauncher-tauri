import { useEffect, useRef } from 'react';
import type { LaunchItem, RuntimeStatus, View } from '../types';
import { isRunning, type SortKey, type StatusMap } from '../state';
import { Icon } from './Icon';

const statusLabels = { running: '运行中', stopped: '已停止', starting: '启动中', stopping: '停止中' };

export function StatusBadge({ status, enabled = true }: { status?: RuntimeStatus; enabled?: boolean }) {
  const state = status?.state ?? 'stopped';
  const failed = state === 'stopped' && !!status?.error;
  return <span title={status?.error ?? undefined} className={'status-badge ' + state + (failed ? ' failed' : '')}>
    <i />{failed ? '启动 / 运行异常' : !enabled && state === 'stopped' ? '已禁用' : statusLabels[state]}</span>;
}

export function ServiceTable({ items, statuses, selected, sortKey, descending, view, search, disabled,
  onSort, onSelect, onSelectAll, onEdit, onMenu, onBlankMenu, onAdd }: {
  items: LaunchItem[]; statuses: StatusMap; selected: Set<string>; sortKey: SortKey; descending: boolean;
  view: View; search: string; disabled: boolean;
  onSort: (key: SortKey) => void;
  onSelect: (id: string, event: { ctrlKey: boolean; shiftKey: boolean; metaKey?: boolean }) => void;
  onSelectAll: () => void; onEdit: (item: LaunchItem) => void;
  onMenu: (item: LaunchItem, x: number, y: number) => void;
  onBlankMenu: (x: number, y: number) => void; onAdd: () => void;
}) {
  const checkbox = useRef<HTMLInputElement>(null);
  const selectedCount = items.filter(item => selected.has(item.id)).length;
  useEffect(() => { if (checkbox.current) checkbox.current.indeterminate = selectedCount > 0 && selectedCount < items.length; }, [selectedCount, items.length]);
  const head = (key: SortKey, label: string) => <th aria-sort={sortKey === key ? descending ? 'descending' : 'ascending' : 'none'}>
    <button onClick={() => onSort(key)}>{label}<Icon name="chevron" size={13} className={sortKey === key ? 'sort-active ' + (descending ? '' : 'ascending') : 'sort-hidden'} /></button></th>;
  return <section className="table-card" aria-label="启动项列表">
    <div className="table-scroll" onContextMenu={event => {
      if (!(event.target as Element).closest('[data-row-id]')) { event.preventDefault(); onBlankMenu(event.clientX, event.clientY); }
    }}>
      <table>
        <thead><tr><th className="select-cell"><input ref={checkbox} type="checkbox" aria-label="选择当前列表全部启动项"
          checked={items.length > 0 && selectedCount === items.length} onChange={onSelectAll} disabled={items.length === 0} /></th>
          {head('state', '状态')}{head('name', '服务名称')}{head('category', '分类')}{head('target', '启动文件 / 命令')}
          {head('pid', 'PID')}{head('autoStart', '自启动')}<th className="row-actions"><span className="sr-only">操作</span></th></tr></thead>
        <tbody>{items.map(item => <tr key={item.id} data-row-id={item.id} aria-selected={selected.has(item.id)}
          className={(selected.has(item.id) ? 'selected ' : '') + (!item.enabled ? 'disabled-item' : '')}
          tabIndex={0} onClick={event => { if (!(event.target as Element).closest('button,input,a')) onSelect(item.id, event); }}
          onKeyDown={event => { if (event.key === ' ') { event.preventDefault(); onSelect(item.id, event); } }}
          onDoubleClick={event => { if (!(event.target as Element).closest('button,input,a')) onEdit(item); }}
          onContextMenu={event => { event.preventDefault(); onMenu(item, event.clientX, event.clientY); }}>
          <td className="select-cell"><input type="checkbox" aria-label={'选择 ' + item.name} checked={selected.has(item.id)}
            onChange={() => onSelect(item.id, { ctrlKey: true, shiftKey: false })} /></td>
          <td><StatusBadge status={statuses.get(item.id)} enabled={item.enabled} /></td>
          <td className="name-cell"><div className={'service-symbol ' + (isRunning(statuses.get(item.id)) ? 'active' : '')}><Icon name="server" size={17} /></div>
            <span title={item.name}>{item.name}</span></td>
          <td><span className="category-tag" title={item.category}>{item.category}</span></td>
          <td className="target-cell"><code title={item.target + (item.arguments ? ' ' + item.arguments : '')}>{item.target}</code></td>
          <td className="pid-cell">{statuses.get(item.id)?.pid ?? '—'}</td>
          <td><span className={item.autoStart ? 'auto-yes' : 'muted'}>{item.autoStart ? <><Icon name="bolt" size={13} />开启</> : '—'}</span></td>
          <td className="row-actions"><button className="icon-button" aria-label={'操作 ' + item.name} disabled={disabled}
            onClick={event => { const bounds = event.currentTarget.getBoundingClientRect(); onMenu(item, bounds.right, bounds.bottom + 4); }}><Icon name="more" /></button></td>
        </tr>)}</tbody>
      </table>
      {items.length === 0 && <div className="empty-state">
        <div className="empty-symbol"><Icon name={search ? 'search' : view === 'running' ? 'activity' : 'server'} size={32} /></div>
        <h3>{search ? '没有找到匹配的服务' : view === 'running' ? '暂时没有正在运行的服务' : view === 'auto' ? '让常用服务自动就绪' : '你的本地服务，从这里开始'}</h3>
        <p>{search ? '试试其他名称、分类或命令关键词。' : view === 'running' ? '启动服务后，可在这里查看它们的运行状态。' :
          view === 'auto' ? '将启动项标记为自动启动，打开 Local Hub 时便会依次运行。' : '添加一个程序或命令，也可以将文件、文件夹拖入窗口。'}</p>
        {!search && view !== 'running' && <button className="button primary" onClick={onAdd} disabled={disabled}><Icon name="plus" />添加启动项</button>}
      </div>}
    </div>
    <footer className="table-footer"><span>{items.length} 个启动项{selectedCount > 0 && <span className="selection-count"> · 已选择 {selectedCount} 项</span>}</span>
      <span>Ctrl / Shift 多选 · 右键更多操作</span></footer>
  </section>;
}
