import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

// Runs against the real compiled application, with its own configuration and
// WebView2 profile. No existing service definitions or startup settings are used.
const project = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executable = resolve(project, process.argv[2] ?? 'src-tauri/target/release/LocalHubLauncher.exe');
const directory = join(project, '.cache', `desktop-smoke-${Date.now()}-${randomUUID().slice(0, 8)}`);
const configPath = join(directory, 'config.json');
const legacyId = randomUUID();
const command = 'Write-Output "hello 中文"; [Console]::Error.WriteLine("stderr 中文"); '
  + 'Write-Output "http://127.0.0.1:43210/health"; Start-Sleep -Seconds 120';
const passed = [];
let application;
let browser;
let page;
let ownsWorkspace = false;
let applicationError;
let nativeOutput = '';
const pageErrors = [];

async function until(read, accepts, description, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  let value;
  while (Date.now() < deadline) {
    value = await read();
    if (accepts(value)) return value;
    await delay(100);
  }
  throw new Error(`${description}: ${JSON.stringify(value)}`);
}

async function step(name, run) {
  await run();
  passed.push(name);
  console.log(`PASS ${name}`);
}

function invoke(command, args = {}) {
  return page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
}
const snapshot = () => invoke('get_snapshot');
const row = id => page.locator(`[data-row-id="${id}"]`);
const toolbar = () => page.getByRole('toolbar', { name: '启动项操作' });
const navigate = name => page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: new RegExp(`^${name}`) }).click();
async function ready() {
  await until(() => page.locator('.status-bar > span').first().innerText(), text => text === '就绪', '操作未完成');
}
async function status(id, state) {
  return until(async () => (await snapshot()).statuses.find(entry => entry.itemId === id),
    entry => entry?.state === state, `服务没有进入 ${state} 状态`);
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const port = server.address().port;
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  return port;
}

async function connect(endpoint) {
  await until(async () => {
    if (applicationError) throw applicationError;
    if (application.exitCode !== null || application.signalCode !== null) {
      throw new Error(`应用提前退出 (${application.exitCode})。请确认没有另一个 Tauri 实例正在运行。\n${nativeOutput}`);
    }
    try { return (await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(1000) })).ok; }
    catch { return false; }
  }, Boolean, 'WebView2 调试连接未就绪', 45_000);
  browser = await chromium.connectOverCDP(endpoint);
  const context = browser.contexts()[0];
  page = context.pages()[0] ?? await context.waitForEvent('page');
  page.setDefaultTimeout(15_000);
  page.on('pageerror', error => pageErrors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') pageErrors.push(message.text()); });
  await page.getByRole('heading', { name: '全部启动项', exact: true }).waitFor();
  await ready();
}

try {
  assert.equal(process.platform, 'win32', '桌面冒烟测试目前支持 Windows。');
  await access(executable);
  await mkdir(directory, { recursive: true });
  await writeFile(configPath, JSON.stringify({
    Version: 1,
    Items: [{ Id: legacyId, Name: '旧配置示例', Target: 'Write-Output "legacy"', LaunchType: 'WindowsPowerShell',
      Category: '兼容性', Enabled: true, AutoStart: false, HideWindow: true, CustomFlag: 'preserve' }],
    Settings: { StartWithWindows: false, CloseToTray: false, MinimizeToTray: false, ConfirmBeforeStopAll: true },
  }, null, 2));
  const port = await availablePort();
  application = spawn(executable, [], {
    cwd: directory, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, LOCALHUB_CONFIG_DIR: directory,
      WEBVIEW2_USER_DATA_FOLDER: join(directory, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` },
  });
  application.on('error', error => { applicationError = error; });
  for (const stream of [application.stdout, application.stderr]) {
    stream.on('data', data => { nativeOutput = (nativeOutput + data).slice(-16_000); });
  }
  await connect(`http://127.0.0.1:${port}`);
  const initial = await snapshot();
  assert.equal(resolve(initial.configPath).toLowerCase(), configPath.toLowerCase(), '拒绝操作非测试配置。');
  ownsWorkspace = true;

  await step('旧版配置通过真实 IPC 加载，扩展字段保留', async () => {
    assert.equal(initial.loadError, null);
    assert.equal(initial.config.items[0].id, legacyId);
    assert.equal(initial.config.items[0].CustomFlag, 'preserve');
    assert.equal(initial.config.version, 2);
    assert.equal(await page.locator('.preview-banner').count(), 0);
    await row(legacyId).waitFor();
  });

  let serviceId;
  let copyId;
  await step('表单新增、快捷键复制及编辑', async () => {
    await toolbar().getByRole('button', { name: '添加启动项', exact: true }).click();
    let dialog = page.getByRole('dialog');
    await dialog.getByRole('textbox', { name: /^名称/ }).fill('桌面验证服务');
    await dialog.getByRole('textbox', { name: '分类', exact: true }).fill('开发服务');
    await dialog.getByRole('combobox').selectOption('WindowsPowerShell');
    await dialog.getByRole('textbox', { name: /^启动文件或命令/ }).fill(command);
    await dialog.getByRole('button', { name: '保存启动项', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' });
    serviceId = (await snapshot()).config.items.find(item => item.name === '桌面验证服务').id;
    await row(serviceId).locator('.name-cell').click();
    await page.keyboard.press('Control+d');
    await ready();
    copyId = (await until(snapshot, value => value.config.items.length === 3, '复制未保存'))
      .config.items.find(item => item.name.endsWith(' - 副本')).id;
    await row(copyId).locator('.name-cell').dblclick();
    dialog = page.getByRole('dialog');
    await dialog.getByRole('textbox', { name: /^名称/ }).fill('第二个服务');
    await dialog.getByRole('button', { name: '保存启动项', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' });
    assert.equal((await snapshot()).config.items.find(item => item.id === copyId).name, '第二个服务');
  });

  await step('多选、排序、右键菜单和自动启动筛选', async () => {
    await row(serviceId).locator('.name-cell').click();
    await row(copyId).locator('.name-cell').click({ modifiers: ['Control'] });
    await page.getByRole('button', { name: '服务名称', exact: true }).click();
    assert.equal(await row(serviceId).getAttribute('aria-selected'), 'true');
    assert.equal(await row(copyId).getAttribute('aria-selected'), 'true');
    await row(copyId).click({ button: 'right' });
    assert.equal(await row(serviceId).getAttribute('aria-selected'), 'true');
    await page.getByRole('menuitem', { name: '设置为自启动', exact: true }).click();
    await ready();
    await navigate('自动启动');
    await until(() => page.locator('tbody tr').count(), count => count === 2, '自动启动筛选不正确');
    await navigate('全部启动项');
  });

  let generation;
  await step('真实服务启动、PID 和运行中筛选', async () => {
    await row(serviceId).locator('.name-cell').click();
    await toolbar().getByRole('button', { name: '启动', exact: true }).click();
    const running = await status(serviceId, 'running');
    assert.ok(running.pid > 0);
    generation = running.generation;
    await ready();
    await navigate('正在运行');
    await row(serviceId).waitFor();
    assert.equal(await page.locator('tbody tr').count(), 1);
    await navigate('全部启动项');
    await page.screenshot({ path: join(directory, 'services.png') });
  });

  await step('独立日志导航、中文输出、错误筛选与清空', async () => {
    await row(serviceId).click({ button: 'right' });
    await page.getByRole('menuitem', { name: '查看独立日志', exact: true }).click();
    await page.getByRole('heading', { name: '服务日志', exact: true }).waitFor();
    await page.getByRole('log').getByText('hello 中文', { exact: true }).waitFor();
    await page.getByRole('log').getByText('stderr 中文', { exact: true }).waitFor();
    assert.ok((await invoke('get_logs', { id: serviceId })).entries.every(entry => !entry.text.includes('CLIXML')));
    assert.equal(await page.getByRole('tab', { name: '桌面验证服务', exact: true }).getAttribute('aria-selected'), 'true');
    await page.getByLabel('仅错误', { exact: true }).check();
    await page.getByRole('log').getByText('stderr 中文', { exact: true }).waitFor();
    assert.equal(await page.getByRole('log').getByText('hello 中文', { exact: true }).count(), 0);
    await page.getByLabel('仅错误', { exact: true }).uncheck();
    await page.screenshot({ path: join(directory, 'logs.png') });
    await page.getByRole('button', { name: '清空当前日志', exact: true }).click();
    await page.getByRole('heading', { name: '等待服务输出', exact: true }).waitFor();
    await delay(350);
    assert.equal((await invoke('get_logs', { id: serviceId })).entries.length, 0);
    assert.equal(await page.locator('.log-row').count(), 0);
  });

  await step('日志标签右键重启与停止', async () => {
    await page.getByRole('tab', { name: '桌面验证服务', exact: true }).click({ button: 'right' });
    await page.getByRole('menuitem', { name: '重新启动', exact: true }).click();
    await until(async () => (await snapshot()).statuses.find(entry => entry.itemId === serviceId),
      entry => entry.state === 'running' && entry.generation > generation, '重启未创建新进程');
    await ready();
    await page.getByRole('log').getByText('hello 中文', { exact: true }).waitFor();
    await page.getByRole('tab', { name: '桌面验证服务', exact: true }).click({ button: 'right' });
    await page.getByRole('menuitem', { name: '停止服务', exact: true }).click();
    await status(serviceId, 'stopped');
    await ready();
    assert.equal((await snapshot()).statuses.find(entry => entry.itemId === serviceId).requestedStop, true);
  });

  await step('设置保存及最小窗口布局', async () => {
    await page.getByRole('button', { name: '启动器设置', exact: true }).click();
    await page.getByRole('spinbutton', { name: '自动启动间隔', exact: true }).fill('250');
    const session = await page.context().newCDPSession(page);
    await session.send('Emulation.setDeviceMetricsOverride', { width: 980, height: 650, deviceScaleFactor: 1, mobile: false });
    const layout = await page.evaluate(() => {
      const dialog = document.querySelector('dialog').getBoundingClientRect();
      const saveButton = document.querySelector('dialog button[type="submit"]').getBoundingClientRect();
      return { left: dialog.left, right: dialog.right, top: dialog.top, bottom: dialog.bottom,
        saveBottom: saveButton.bottom, width: innerWidth, height: innerHeight,
        overflow: document.documentElement.scrollWidth > innerWidth };
    });
    assert.ok(layout.left >= 0 && layout.right <= layout.width && layout.top >= 0 && layout.bottom <= layout.height);
    assert.equal(layout.overflow, false);
    assert.ok(layout.saveBottom < layout.bottom, '最小窗口下保存按钮必须直接可见');
    await page.screenshot({ path: join(directory, 'settings-small.png') });
    await page.getByRole('button', { name: '保存设置', exact: true }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    assert.equal((await snapshot()).config.settings.autoStartIntervalMs, 250);
    await session.send('Emulation.clearDeviceMetricsOverride');
    await session.detach();
  });

  await step('文件夹识别、无效外部配置保护和刷新恢复', async () => {
    const droppedDirectory = join(directory, 'dropped');
    await mkdir(droppedDirectory);
    await writeFile(join(droppedDirectory, 'start-demo.cmd'), '@echo off\necho example\n');
    const prepared = await invoke('prepare_dropped_items', { paths: [droppedDirectory] });
    assert.equal(prepared.failures.length, 0);
    assert.ok(prepared.items[0].target.endsWith('start-demo.cmd'));
    await navigate('全部启动项');
    const saved = await readFile(configPath, 'utf8');
    await writeFile(configPath, 'null');
    await page.getByRole('button', { name: '重新读取配置', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: '配置文件无效' }).waitFor();
    assert.equal((await snapshot()).config.items.length, 3);
    assert.equal(await readFile(configPath, 'utf8'), 'null');
    const restored = JSON.parse(saved);
    restored.items.find(item => item.id === legacyId).name = '旧配置已刷新';
    await writeFile(configPath, JSON.stringify(restored, null, 2));
    await page.keyboard.press('F5');
    await row(legacyId).getByText('旧配置已刷新', { exact: true }).waitFor();
    await ready();
  });

  await step('批量删除及持久化', async () => {
    await row(serviceId).locator('.name-cell').click();
    await row(copyId).locator('.name-cell').click({ modifiers: ['Control'] });
    await page.keyboard.press('Delete');
    await page.getByRole('dialog').getByRole('button', { name: '确认删除', exact: true }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    assert.equal((await snapshot()).config.items.length, 1);
    assert.equal(JSON.parse(await readFile(configPath, 'utf8')).items.length, 1);
    assert.deepEqual(pageErrors, []);
  });
  console.log(`Desktop smoke: ${passed.length} workflows passed. Artifacts: ${directory}`);
} catch (error) {
  if (page) await page.screenshot({ path: join(directory, 'failure.png') }).catch(() => {});
  console.error(error);
  if (pageErrors.length) console.error('WebView errors:', pageErrors);
  if (nativeOutput) console.error('Application output:', nativeOutput);
  console.error(`Artifacts: ${directory}`);
  process.exitCode = 1;
} finally {
  if (ownsWorkspace && page && !page.isClosed()) {
    await invoke('stop_all').catch(() => {});
    await invoke('exit_app').catch(() => {});
  }
  if (browser) await browser.close().catch(() => {});
  if (application && application.exitCode === null && application.signalCode === null) {
    await Promise.race([new Promise(resolve => application.once('exit', resolve)), delay(3000)]);
    if (application.exitCode === null && application.signalCode === null) application.kill();
  }
}
