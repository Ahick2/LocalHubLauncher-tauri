import type { LaunchItem, LogRecord, RuntimeStatus, View } from './types';

export type SortKey = 'name' | 'category' | 'target' | 'state' | 'pid' | 'autoStart';
export type StatusMap = Map<string, RuntimeStatus>;
export const isRunning = (status?: RuntimeStatus) => !!status && status.state !== 'stopped';

export function visibleItems(items: LaunchItem[], statuses: StatusMap, view: View, search: string): LaunchItem[] {
  const query = search.trim().toLocaleLowerCase();
  return items.filter(item =>
    (view !== 'running' || isRunning(statuses.get(item.id))) &&
    (view !== 'auto' || (item.enabled && item.autoStart)) &&
    (!query || [item.name, item.category, item.target].some(value => value.toLocaleLowerCase().includes(query))));
}

export function sortedItems(items: LaunchItem[], statuses: StatusMap, key: SortKey, descending: boolean): LaunchItem[] {
  const ranks = { stopped: 0, starting: 1, running: 2, stopping: 3 };
  const value = (item: LaunchItem): string | number => {
    if (key === 'state') return ranks[statuses.get(item.id)?.state ?? 'stopped'];
    if (key === 'pid') return statuses.get(item.id)?.pid ?? -1;
    if (key === 'autoStart') return Number(item.autoStart);
    return item[key];
  };
  return items.map((item, index) => ({ item, index })).sort((a, b) => {
    const left = value(a.item), right = value(b.item);
    const order = typeof left === 'number' && typeof right === 'number'
      ? left - right : String(left).localeCompare(String(right), 'zh-CN', { numeric: true });
    return (descending ? -order : order) || a.index - b.index;
  }).map(({ item }) => item);
}

export function selectRow(
  selected: Set<string>, id: string, ordered: string[], anchor: string | null,
  modifiers: { toggle?: boolean; range?: boolean; context?: boolean },
): Set<string> {
  if (modifiers.context) return new Set([...selected, id]);
  if (modifiers.range && anchor && ordered.includes(anchor)) {
    const a = ordered.indexOf(anchor), b = ordered.indexOf(id);
    const range = ordered.slice(Math.min(a, b), Math.max(a, b) + 1);
    return new Set(modifiers.toggle ? [...selected, ...range] : range);
  }
  if (modifiers.toggle) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  }
  return new Set([id]);
}

export function mergeLogs(current: LogRecord[], incoming: LogRecord[], limit = 4000): LogRecord[] {
  const bySequence = new Map(current.map(record => [record.sequence, record]));
  for (const record of incoming) bySequence.set(record.sequence, record);
  const records = [...bySequence.values()].sort((a, b) => a.sequence - b.sequence).slice(-limit);
  let size = 0;
  let start = records.length;
  while (start > 0 && size + records[start - 1].text.length <= 4 * 1024 * 1024) {
    start--; size += records[start].text.length;
  }
  return records.slice(start);
}

export const formatLog = (record: LogRecord, combined = true) =>
  '[' + new Date(record.timestamp).toLocaleTimeString('zh-CN', { hour12: false }) + '] ' +
  (combined ? '[' + record.itemName + '] ' : '') + record.text;

export function acceptSnapshot(currentRevision: number, incomingRevision: number): boolean {
  return incomingRevision >= currentRevision;
}
