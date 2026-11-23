// What happens when things go wrong: the engine crashes or cannot start, work is unsaved when a
// tab or the app closes, the window is reopened; and every failure leaves a line in the log.
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type ElectronApplication } from '@playwright/test';
import { fixture, start } from './helpers';

const captureTab = (page: import('@playwright/test').Page, name: string) =>
  page.getByRole('navigation', { name: 'Open captures' }).getByRole('button', { name: new RegExp(`^${name}`) });

/** Ends the engine process the hard way, as a crash would. */
async function killEngine(app: ElectronApplication): Promise<void> {
  const pid = await app.evaluate(({ app: a }) => {
    const p = a.getAppMetrics().find((m) => m.type === 'Utility' && (m.serviceName === 'Logica engine' || m.name === 'Logica engine'));
    if (!p) throw new Error('no engine process');
    process.kill(p.pid, 'SIGKILL');
    return p.pid;
  });
  expect(pid).toBeGreaterThan(0);
}

const prompts = (app: ElectronApplication) => app.evaluate(() => (globalThis as unknown as { __prompts: { message: string; detail: string }[] }).__prompts);

test('a crashed engine is restarted, says so, and the files can be opened again', async () => {
  const { app, page, dataDir } = await start([fixture('captures/mystery_C.demo')]);
  await expect(captureTab(page, 'mystery_C.demo')).toBeVisible();
  await killEngine(app);
  const banner = page.getByRole('alert').filter({ hasText: 'The engine was restarted' });
  await expect(banner).toBeVisible({ timeout: 15_000 });
  await expect(captureTab(page, 'mystery_C.demo')).toHaveCount(0);
  await banner.getByRole('button', { name: 'Open mystery_C.demo again' }).click();
  await expect(captureTab(page, 'mystery_C.demo')).toBeVisible();
  // the new engine works: the analysis runs again
  await expect(page.getByRole('region', { name: /Bus CH4, CH5/ }).getByText('100 % sure')).toBeVisible();
  const log = readFileSync(join(dataDir, 'logs', 'logica.log'), 'utf8');
  expect(log).toMatch(/The engine process ended .* restarted/);
  await app.close();
});

test('an engine that cannot start says why and offers to start it again', async () => {
  const { app, page, dataDir } = await start([], { env: { LOGICA_NATIVE_PATH: '/nowhere/logica.node' } });
  const alert = page.getByRole('alert').filter({ hasText: 'The engine is not running' });
  await expect(alert).toBeVisible({ timeout: 15_000 });
  await expect(alert).toContainText('could not be loaded');
  await expect(alert.getByRole('button', { name: 'Start the engine again' })).toBeVisible();
  expect(readFileSync(join(dataDir, 'logs', 'logica.log'), 'utf8')).toContain('could not be loaded');
  await app.close();
});

test('closing a tab with unsaved changes asks first; a saved one closes at once', async () => {
  const { app, page } = await start([fixture('captures/1_UART_easy.demo'), fixture('captures/mystery_A.demo')]);
  await expect(captureTab(page, '1_UART_easy.demo')).toBeVisible();
  await expect(captureTab(page, 'mystery_A.demo')).toBeVisible();
  // an opened file has nothing unsaved: it closes without a question
  await page.getByRole('button', { name: 'Close mystery_A.demo', exact: true }).click();
  await expect(captureTab(page, 'mystery_A.demo')).toHaveCount(0);

  const channels = page.getByRole('complementary', { name: 'Channels' });
  await channels.getByRole('button', { name: /^CH0/ }).dblclick();
  await page.getByLabel('New name for CH0').fill('RX line');
  await page.getByLabel('New name for CH0').press('Enter');
  await expect(captureTab(page, '1_UART_easy.demo')).toContainText('unsaved changes');

  await page.getByRole('button', { name: 'Close 1_UART_easy.demo', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Close 1_UART_easy.demo?' });
  await expect(dialog).toContainText('Closing cannot be undone');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(captureTab(page, '1_UART_easy.demo')).toBeVisible();

  await page.getByRole('button', { name: 'Close 1_UART_easy.demo', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Close without saving' }).click();
  await expect(captureTab(page, '1_UART_easy.demo')).toHaveCount(0);
  await app.close();
});

test('quitting with unsaved work asks first, and cancelling keeps the window', async () => {
  const { app, page } = await start([fixture('captures/1_UART_easy.demo')]);
  await expect(captureTab(page, '1_UART_easy.demo')).toBeVisible();
  const channels = page.getByRole('complementary', { name: 'Channels' });
  await channels.getByRole('button', { name: /^CH0/ }).dblclick();
  await page.getByLabel('New name for CH0').fill('RX line');
  await page.getByLabel('New name for CH0').press('Enter');
  await expect(captureTab(page, '1_UART_easy.demo')).toContainText('unsaved changes');

  await app.evaluate(({ BrowserWindow }) => {
    (globalThis as unknown as { __promptAnswer: number }).__promptAnswer = 1; // Cancel
    BrowserWindow.getAllWindows()[0]!.close();
  });
  await expect.poll(async () => (await prompts(app)).length).toBe(1);
  const [p] = await prompts(app);
  expect(p!.message).toBe('A capture has unsaved changes.');
  expect(p!.detail).toContain('1_UART_easy.demo');
  await expect(captureTab(page, '1_UART_easy.demo')).toBeVisible();
  // now let it close
  await app.evaluate(() => ((globalThis as unknown as { __promptAnswer: number }).__promptAnswer = 0));
  await app.close();
});

test('the window opens again where it was, at its size', async () => {
  const first = await start([], { keepSize: true });
  await first.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.setBounds({ x: 40, y: 40, width: 1210, height: 790 }));
  await first.app.close();
  expect(existsSync(join(first.dataDir, 'window.json'))).toBe(true);
  const second = await start([], { dataDir: first.dataDir, keepSize: true });
  const size = await second.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.getSize());
  expect(size).toEqual([1210, 790]);
  await second.app.close();
});

test('a file that arrives twice at once opens once', async () => {
  const { app, page } = await start();
  await expect(page.getByRole('heading', { name: 'Start' })).toBeVisible();
  await app.evaluate(({ BrowserWindow }, f) => {
    const w = BrowserWindow.getAllWindows()[0]!;
    w.webContents.send('logica:open-files', [f]);
    w.webContents.send('logica:open-files', [f]);
  }, fixture('captures/mystery_D.demo'));
  await expect(captureTab(page, 'mystery_D.demo')).toHaveCount(1);
  await page.waitForTimeout(500);
  await expect(captureTab(page, 'mystery_D.demo')).toHaveCount(1);
  await app.close();
});
