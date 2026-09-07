import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ActionResult, DroppedItems, LaunchItem, LogBatch, LogPage, Notice, ServiceAction, Settings, Snapshot } from './types';

export const desktop = isTauri();
export const api = {
  snapshot: () => invoke<Snapshot>('get_snapshot'),
  saveItem: (item: LaunchItem) => invoke<Snapshot>('save_item', { item }),
  deleteItems: (ids: string[]) => invoke<ActionResult>('delete_items', { ids }),
  action: (ids: string[], action: ServiceAction) => invoke<ActionResult>('service_action', { ids, action }),
  stopAll: () => invoke<ActionResult>('stop_all'),
  startAuto: () => invoke<Snapshot>('start_auto'),
  setAutoStart: (ids: string[], enabled: boolean) => invoke<Snapshot>('set_auto_start', { ids, enabled }),
  reload: () => invoke<Snapshot>('reload_config'),
  saveSettings: (settings: Settings) => invoke<Snapshot>('save_settings', { settings }),
  logs: (id: string | null) => invoke<LogPage>('get_logs', { id }),
  clearLogs: (id: string | null) => invoke<LogPage>('clear_logs', { id }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openDirectory: (id: string | null) => invoke<void>('open_directory', { id }),
  dropped: (paths: string[]) => invoke<DroppedItems>('prepare_dropped_items', { paths }),
  exit: () => invoke<void>('exit_app'),
};

export const onSnapshot = (handler: (snapshot: Snapshot) => void) =>
  listen<Snapshot>('snapshot-changed', event => handler(event.payload));
export const onLogBatch = (handler: (batch: LogBatch) => void) =>
  listen<LogBatch>('log-batch', event => handler(event.payload));
export const onNotice = (handler: (notice: Notice) => void) =>
  listen<Notice>('app-notice', event => handler(event.payload));

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
