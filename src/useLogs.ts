import { useEffect, useRef, useState } from 'react';
import { api, desktop, errorMessage, onLogBatch } from './bridge';
import { mergeLogs } from './state';
import type { LogRecord, Notice } from './types';

export function useLogs(id: string | null, notify: (notice: Notice) => void) {
  const [entries, setEntries] = useState<LogRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const refresh = useRef<(clear?: boolean) => Promise<void>>(async () => {});
  useEffect(() => {
    if (!desktop) { setLoading(false); return; }
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    let pending: LogRecord[] = [];
    let fetching = true;
    let cursor = 0;
    let serial = 0;
    let lastDroppedNotice = 0;
    setEntries([]);
    const accepts = (entry: LogRecord) => id === null || entry.itemId === id;
    const load = async (clear = false) => {
      const request = ++serial;
      fetching = true; pending = []; setLoading(true);
      try {
        const page = await (clear ? api.clearLogs(id) : api.logs(id));
        if (cancelled || request !== serial) return;
        cursor = page.cursor;
        setEntries(mergeLogs(page.entries, pending.filter(entry => entry.sequence > cursor)));
      } catch (error) {
        if (!cancelled) notify({ level: 'error', message: errorMessage(error) });
      } finally {
        if (!cancelled && request === serial) { fetching = false; pending = []; setLoading(false); }
      }
    };
    refresh.current = load;
    void onLogBatch(batch => {
      const incoming = batch.entries.filter(entry => accepts(entry) && entry.sequence > cursor);
      if (fetching) pending = mergeLogs(pending, incoming);
      else if (incoming.length) setEntries(previous => mergeLogs(previous, incoming));
      if (batch.dropped > 0 && Date.now() - lastDroppedNotice > 5000) {
        lastDroppedNotice = Date.now();
        notify({ level: 'info', message: '日志输出较多，实时视图已跳过部分行；可点击刷新读取当前缓冲。' });
      }
    }).then(stop => {
      if (cancelled) stop();
      else { unsubscribe = stop; void load(); }
    }).catch(error => { setLoading(false); notify({ level: 'error', message: errorMessage(error) }); });
    return () => { cancelled = true; unsubscribe?.(); };
  }, [id, notify]);
  return { entries, loading, reload: () => refresh.current(), clear: () => refresh.current(true) };
}
