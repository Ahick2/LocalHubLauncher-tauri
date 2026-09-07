export type LaunchType = 'Auto' | 'Executable' | 'Batch' | 'PowerShell' | 'Python' |
  'Command' | 'Pwsh' | 'WindowsPowerShell' | 'Wsl' | 'GitBash';
export type ProcessState = 'stopped' | 'starting' | 'running' | 'stopping';
export type View = 'all' | 'running' | 'auto' | 'logs';
export type ServiceAction = 'start' | 'stop' | 'restart';

export interface LaunchItem {
  id: string;
  name: string;
  category: string;
  target: string;
  arguments: string;
  workingDirectory: string;
  url: string;
  launchType: LaunchType;
  autoStart: boolean;
  hideWindow: boolean;
  enabled: boolean;
  [extension: string]: unknown;
}

export interface Settings {
  startWithWindows: boolean;
  startMinimizedToTray: boolean;
  minimizeToTray: boolean;
  closeToTray: boolean;
  confirmBeforeStopAll: boolean;
  autoStartIntervalMs: number;
  [extension: string]: unknown;
}

export interface LauncherConfig {
  version: number;
  items: LaunchItem[];
  settings: Settings;
}

export interface RuntimeStatus {
  itemId: string;
  generation: number;
  state: ProcessState;
  pid: number | null;
  startedAt: number | null;
  exitCode: number | null;
  requestedStop: boolean;
  error: string | null;
}

export interface LogTab { id: string; name: string; count: number }
export interface Snapshot {
  revision: number;
  config: LauncherConfig;
  statuses: RuntimeStatus[];
  logTabs: LogTab[];
  configPath: string;
  loadError: string | null;
  autoStartRegistered: boolean;
  autoStarting: boolean;
  platform: string;
  version: string;
}

export interface LogRecord {
  sequence: number;
  itemId: string;
  itemName: string;
  text: string;
  stream: 'stdout' | 'stderr' | 'system';
  timestamp: number;
}
export interface LogPage { entries: LogRecord[]; cursor: number }
export interface LogBatch { entries: LogRecord[]; dropped: number }
export interface ActionResult {
  snapshot: Snapshot;
  failures: { id: string; name: string; error: string }[];
}
export interface Notice { level: 'success' | 'error' | 'info'; message: string }
export interface DroppedItems { items: LaunchItem[]; failures: string[] }

export const launchTypes: { value: LaunchType; label: string; hint: string }[] = [
  { value: 'Auto', label: '自动识别', hint: '识别程序和脚本文件；完整命令交给 cmd 执行。' },
  { value: 'Executable', label: '可执行程序', hint: '选择可执行文件，在下方单独填写参数。' },
  { value: 'Batch', label: '批处理 · .bat / .cmd', hint: '批处理及其子进程会一起受到管理。' },
  { value: 'PowerShell', label: 'PowerShell 脚本 · .ps1', hint: '优先使用 PowerShell 7，未安装时使用 Windows PowerShell。' },
  { value: 'Python', label: 'Python 脚本 · .py', hint: '以 UTF-8、无缓冲模式运行。虚拟环境可改选其中的 python.exe。' },
  { value: 'Command', label: '单命令 · cmd', hint: '例如 npm start、node server.js 或 python server.py。' },
  { value: 'Pwsh', label: '单命令 · PowerShell 7', hint: '保留原始引号与表达式，需要已安装 pwsh。' },
  { value: 'WindowsPowerShell', label: '单命令 · Windows PowerShell', hint: '使用 Windows 内置 PowerShell 执行完整命令。' },
  { value: 'Wsl', label: '单命令 · WSL', hint: '使用默认 WSL 发行版中的 bash 执行命令。' },
  { value: 'GitBash', label: '单命令 · Git Bash', hint: '使用 Git for Windows 中的 bash 执行命令。' },
];

export const emptySnapshot: Snapshot = {
  revision: 0, config: { version: 2, items: [], settings: {
    startWithWindows: false, startMinimizedToTray: false, minimizeToTray: true,
    closeToTray: true, confirmBeforeStopAll: true, autoStartIntervalMs: 800,
  } }, statuses: [], logTabs: [], configPath: '', loadError: null,
  autoStartRegistered: false, autoStarting: false, platform: 'windows', version: '2.0.0',
};

export function newItem(autoStart = false): LaunchItem {
  return { id: crypto.randomUUID(), name: '', category: '本地服务', target: '',
    arguments: '', workingDirectory: '', url: '', launchType: 'Auto',
    autoStart, hideWindow: true, enabled: true };
}
