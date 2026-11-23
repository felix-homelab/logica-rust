// The desktop app end to end (doc 10, doc 11 M2 exit): the start screen, opening and decoding a
// recording, an agent's marker and view change appearing live, the palette, undo, and axe checks
// in both themes.
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { Agent, fixture, start } from './helpers';

async function axe(page: Page, what: string): Promise<void> {
  // legacy mode runs inside the page; Electron cannot open the helper page the default needs
  const r = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
  const problems = r.violations.map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(' ')).slice(0, 3).join('; ')})`);
  expect(problems, `axe on ${what}`).toEqual([]);
}

/** Keeps a picture for checking by eye when LOGICA_SHOTS names a folder. */
async function shot(page: Page, name: string): Promise<void> {
  if (process.env.LOGICA_SHOTS) await page.screenshot({ path: `${process.env.LOGICA_SHOTS}/${name}.png` });
}

const captureTab = (page: Page, name: string) => page.getByRole('navigation', { name: 'Open captures' }).getByRole('button', { name, exact: true });

async function theme(page: Page, t: 'light' | 'dark'): Promise<void> {
  await page.evaluate((x) => (document.documentElement.dataset.theme = x), t);
}

test('the start screen offers files, a demo and agents, and passes axe in both themes', async () => {
  const { app, page } = await start();
  await expect(page.getByRole('heading', { name: 'Start' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open file…' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try a demo capture' })).toBeVisible();
  await theme(page, 'light');
  await axe(page, 'the start screen (light)');
  await theme(page, 'dark');
  await axe(page, 'the start screen (dark)');
  await app.close();
});

test('a recording opens, is analysed, decoded with one click and read in the table', async () => {
  const { app, page } = await start([fixture('captures/mystery_C.demo')]);
  await expect(captureTab(page, 'mystery_C.demo')).toBeVisible();
  const card = page.getByRole('region', { name: /Bus CH4, CH5/ });
  await expect(card.getByText('I2C', { exact: true })).toBeVisible();
  await expect(card.getByText('100 % sure')).toBeVisible();
  await expect(card.getByText(/The clock runs at 400 kHz/)).toBeVisible();
  await card.getByRole('button', { name: 'Add decoder' }).click();
  await expect(card.getByRole('button', { name: 'Decoder added' })).toBeDisabled();

  const row = page.getByRole('button', { name: /CALIBRATED 2026/ });
  await expect(row).toBeVisible();
  await row.click();
  // the view moved to the transaction (about 660 ms)
  await expect(page.getByLabel('Status')).toContainText(/66\d(\.\d+)? ms/);
  await page.waitForTimeout(300);
  await shot(page, 'decoded-light');
  await theme(page, 'dark');
  await shot(page, 'decoded-dark');
  await axe(page, 'the workbench with a decoder (dark)');
  await theme(page, 'light');
  await axe(page, 'the workbench with a decoder (light)');
  await app.close();
});

test("an agent's marker and view change appear live in the open window", async () => {
  const { app, page, dataDir } = await start([fixture('captures/mystery_C.demo')]);
  await expect(captureTab(page, 'mystery_C.demo')).toBeVisible();
  const agent = await Agent.connect(dataDir, 'claude-code');
  const said = await agent.tool('show_user', { capture: 'c1', at: 6_600_000, label: 'calibration date', moveView: true });
  expect(said).toContain('The person now sees marker');

  // the person is told, sees the moved view and the marker, and can find the action
  await expect(page.getByRole('status').filter({ hasText: 'claude-code moved your view.' })).toBeVisible();
  await expect(page.locator('#wave-description')).toContainText('1 marker in view');
  await page.getByRole('tab', { name: 'Activity' }).click();
  const entry = page.getByRole('list', { name: 'Activity' }).getByRole('listitem').filter({ hasText: 'claude-code' });
  await expect(entry.first()).toContainText('calibration date');
  await shot(page, 'agent-marker');

  // the person undoes it from the feed
  await entry.first().getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('#wave-description')).toContainText('0 markers in view');
  await app.close();
});

test('the palette finds and runs actions; Ctrl+Z undoes the last action', async () => {
  const { app, page } = await start([fixture('captures/1_UART_easy.demo')]);
  await expect(captureTab(page, '1_UART_easy.demo')).toBeVisible();
  await page.keyboard.press('Control+K');
  const input = page.getByRole('combobox', { name: 'Search commands' });
  await expect(input).toBeFocused();
  await input.fill('hide sidebar');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary', { name: 'Channels' })).toHaveCount(0);

  // rename a channel, then undo with the keyboard
  await page.keyboard.press('Control+K');
  await input.fill('show or hide the sidebar');
  await page.keyboard.press('Enter');
  const name = page.getByRole('complementary', { name: 'Channels' }).getByRole('button', { name: /^CH0/ });
  await name.dblclick();
  const edit = page.getByLabel('New name for CH0');
  await edit.fill('RX line');
  await edit.press('Enter');
  await expect(page.getByRole('complementary', { name: 'Channels' })).toContainText('RX line');
  await page.locator('.wave__canvas').focus();
  await page.keyboard.press('Control+Z');
  await expect(page.getByRole('complementary', { name: 'Channels' })).not.toContainText('RX line');
  await app.close();
});

test('the component gallery passes axe in both themes', async () => {
  const { app, page } = await start();
  await test.step('open the gallery', async () => {
    await page.evaluate(() => (window.location.hash = 'gallery'));
    await expect(page.getByRole('heading', { name: 'Component gallery' })).toBeVisible();
  });
  await test.step('light', async () => {
    await page.getByRole('button', { name: 'Light', exact: true }).click();
    await axe(page, 'the gallery (light)');
  });
  await test.step('dark', async () => {
    await page.getByRole('button', { name: 'Dark', exact: true }).click();
    await axe(page, 'the gallery (dark)');
  });
  await app.close();
});

test('app:// serves the window and nothing outside its folder', async () => {
  const { app, page } = await start();
  expect(page.url()).toMatch(/^app:\/\/logica\/index\.html/);
  const status = (path: string) => page.evaluate(async (p) => fetch(p).then((r) => r.status, (e: unknown) => String(e)), path);
  expect(await status('app://logica/index.html')).toBe(200);
  expect(await status('app://logica/%2e%2e/main/index.js')).toBe(404);
  expect(await status('app://logica/%2e%2e%2f%2e%2e%2fpackage.json')).toBe(404);
  await app.close();
});
