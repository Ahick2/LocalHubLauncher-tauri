import { describe, expect, it } from 'vitest';
import { acceptSnapshot, mergeLogs, selectRow, sortedItems, visibleItems } from './state';
import type { LaunchItem, LogRecord, RuntimeStatus } from './types';

const item = (id: string, name = id): LaunchItem => ({
  id, name, target: 'node server.js', category: '本地服务', arguments: '', workingDirectory: '',
  url: '', launchType: 'Auto', autoStart: false, hideWindow: true, enabled: true,
});
const status = (itemId: string, pid: number): RuntimeStatus => ({
  itemId, pid, generation: 1, state: 'running', startedAt: 0, exitCode: null, requestedStop: false, error: null,
});
const log = (sequence: number): LogRecord => ({
  sequence, itemId: 'a', itemName: 'demo', text: String(sequence), stream: 'stdout', timestamp: 0,
});

describe('service selection and sorting', () => {
  it('preserves a group on right-click and extends Shift selection', () => {
    expect([...selectRow(new Set(['a', 'b']), 'b', ['a', 'b', 'c'], 'a', { context: true })]).toEqual(['a', 'b']);
    expect([...selectRow(new Set(['a']), 'c', ['a', 'b', 'c'], 'a', { range: true })]).toEqual(['a', 'b', 'c']);
  });
  it('sorts PIDs numerically without changing selection identity', () => {
    const selected = new Set(['b']);
    const sorted = sortedItems([item('a'), item('b')], new Map([['a', status('a', 100)], ['b', status('b', 9)]]), 'pid', false);
    expect(sorted.map(row => row.id)).toEqual(['b', 'a']);
    expect(selected.has(sorted[0].id)).toBe(true);
  });
  it('combines view filtering with name/category/command search', () => {
    const items = [item('a', 'API'), { ...item('b'), autoStart: true, enabled: false }];
    expect(visibleItems(items, new Map([['a', status('a', 9)]]), 'running', 'api')).toEqual([items[0]]);
    expect(visibleItems(items, new Map(), 'auto', '')).toEqual([]);
  });
});

describe('batched log and snapshot updates', () => {
  it('deduplicates history/live overlap and retains the latest bounded records', () => {
    expect(mergeLogs([log(1), log(3)], [log(2), log(3), log(4)], 3).map(record => record.sequence)).toEqual([2, 3, 4]);
  });
  it('ignores stale snapshots arriving after a newer action', () => {
    expect(acceptSnapshot(8, 7)).toBe(false);
    expect(acceptSnapshot(8, 9)).toBe(true);
  });
});
