// The window's layout holds at the smallest size it allows (doc 08: 960 x 600): nothing in the
// header, toolbar or status bar is cut off, many tabs and long names shorten instead of pushing
// the rest away, a long dialog keeps its buttons in sight, and remembered panel sizes leave the
// waveform room.
import { copyFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test, type ElectronApplication, type Locator, type Page } from '@playwright/test';
import { fixture, start } from './helpers';

const MIN = { width: 960, height: 600 };

async function size(app: ElectronApplication, page: Page, width: number, height: number): Promise<void> {
  await app.evaluate(({ BrowserWindow }, [w, h]) => BrowserWindow.getAllWindows()[0]?.setContentSize(w!, h!), [width, height]);
  await expect.poll(() => page.evaluate(() => [window.innerWidth, window.innerHeight])).toEqual([width, height]);
}

/** The element's box lies wholly inside the window. */
async function inWindow(l: Locator, what: string): Promise<void> {
  const box = await l.boundingBox();
  expect(box, `${what} is shown`).not.toBeNull();
  const { width, height } = await l.page().evaluate(() => ({ width: window.innerWidth, height: window.innerHeight }));
  expect(box!.x, `${what}: left edge`).toBeGreaterThanOrEqual(0);
  expect(box!.y, `${what}: top edge`).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width, `${what}: right edge`).toBeLessThanOrEqual(width + 0.5);
  expect(box!.y + box!.height, `${what}: bottom edge`).toBeLessThanOrEqual(height + 0.5);
}

test('many tabs with long names fit the smallest window; header, toolbar and status bar stay whole', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'logica-layout-'));
  const long = join(dir, 'a_very_long_capture_file_name_from_the_lab_bench_session_run_0042.demo');
  copyFileSync(fixture('captures/mystery_C.demo'), long);
  const others = ['1_UART_easy.demo', '2_UART_mystery_baud.demo', '3_I2C_bus.demo', 'mystery_A.demo', 'mystery_B.demo'];
  const { app, page } = await start([long, ...others.map((n) => fixture(`captures/${n}`))]);
  await expect(page.locator('.tabs__tab')).toHaveCount(6, { timeout: 20_000 });
  await size(app, page, MIN.width, MIN.height);

  // the tab bar keeps its height (no scroll bar inside it) and its buttons stay in the window
  const bar = page.locator('.tabs');
  expect((await bar.boundingBox())!.height).toBe(38);
  const strip = page.locator('.tabs__ul');
  expect(await strip.evaluate((e) => e.offsetHeight - e.clientHeight), 'no horizontal scroll bar in the tab strip').toBe(0);
  for (const name of ['Open a capture', 'Capture live']) await inWindow(page.getByRole('button', { name, exact: true }), name);
  await inWindow(page.getByRole('button', { name: /^Search commands/ }), 'the palette button');
  await inWindow(page.locator('.agent-indicator'), 'the agent indicator');

  // a long tab title shortens with "…" and says it in full on hover; its close button stays
  const first = page.locator('.tabs__tab').first();
  const label = first.locator('.tabs__label');
  expect(await label.evaluate((e) => e.scrollWidth > e.clientWidth), 'the long title is shortened').toBe(true);
  await expect(first.locator('.tabs__title')).toHaveAttribute('title', /a_very_long_capture_file_name/);
  await inWindow(first.getByRole('button', { name: /^Close a_very_long/ }), 'the first tab’s close button');

  // the active tab is scrolled into sight, also the last one
  const last = page.locator('.tabs__title').last();
  await last.click();
  await expect.poll(async () => {
    const s = await strip.boundingBox();
    const t = await page.locator('.tabs__tab[data-active]').boundingBox();
    return t !== null && s !== null && t.x >= s.x - 0.5 && t.x + t.width <= s.x + s.width + 0.5;
  }).toBe(true);

  for (const name of ['Sidebar', 'Zoom in', 'Zoom out', 'Undo', 'Redo', 'Bottom panel', 'Inspector']) {
    await inWindow(page.getByRole('toolbar', { name: 'Capture' }).getByRole('button', { name, exact: true }), name);
  }
  await inWindow(page.getByRole('toolbar', { name: 'Capture' }).getByRole('button', { name: 'Save' }), 'Save');
  // the status bar's last item (MCP) is not pushed out of the window
  await inWindow(page.getByLabel('Status').locator('> :last-child'), 'the MCP status');
  await app.close();
});

test('a long dialog keeps its title and buttons in sight in the smallest window, centred', async () => {
  const { app, page } = await start([fixture('captures/4_SPI_wrong_mode.demo')]);
  const card = page.getByRole('region', { name: /Bus CH3/ });
  await card.getByRole('button', { name: 'Add decoder' }).click({ timeout: 20_000 });
  await size(app, page, MIN.width, MIN.height);
  await page.getByRole('button', { name: /^Settings of SPI/ }).click();
  const dialog = page.getByRole('dialog', { name: 'SPI settings' });
  await expect(dialog).toBeVisible();
  await inWindow(dialog, 'the dialog');
  await inWindow(dialog.getByRole('heading', { name: 'SPI settings' }), 'the title');
  await inWindow(dialog.getByRole('button', { name: 'Done' }), 'the Done button');
  // a form longer than the window scrolls inside, between the title and the buttons
  const body = dialog.locator('.lg-dialog__body');
  expect(await body.evaluate((e) => getComputedStyle(e).overflowY), 'the body is what scrolls').toBe('auto');
  await body.evaluate((e) => (e.scrollTop = e.scrollHeight));
  await inWindow(dialog.getByRole('button', { name: 'Done' }), 'the Done button after scrolling');
  await inWindow(dialog.getByRole('heading', { name: 'SPI settings' }), 'the title after scrolling');
  // centred, also while it opens (the opening animation must not move it sideways)
  const box = (await dialog.boundingBox())!;
  expect(Math.abs(box.x + box.width / 2 - MIN.width / 2)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.y + box.height / 2 - MIN.height / 2)).toBeLessThanOrEqual(1);
  expect(await dialog.evaluate((e) => getComputedStyle(e).translate)).toBe('-50% -50%');
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(dialog).toHaveCount(0);
  await app.close();
});

test('remembered panel sizes from a larger window leave the waveform room in the smallest one', async () => {
  const { app, page } = await start([fixture('captures/mystery_C.demo')]);
  await page.evaluate(() => {
    localStorage.setItem('logica.bottomH', '600');
    localStorage.setItem('logica.sidebarW', '420');
    localStorage.setItem('logica.inspectorW', '"not a number"');
  });
  await page.reload();
  await size(app, page, MIN.width, MIN.height);
  const wave = page.getByRole('region', { name: 'Waveform' });
  await expect(wave).toBeVisible({ timeout: 20_000 });
  const w = (await wave.boundingBox())!;
  expect(w.height, 'the waveform keeps at least 140 px').toBeGreaterThanOrEqual(140);
  expect(w.width, 'the waveform keeps at least 320 px').toBeGreaterThanOrEqual(320);
  await inWindow(page.getByRole('region', { name: 'Bottom panel' }), 'the bottom panel');
  // the splitter offers no more than fits
  const splitter = page.getByRole('separator', { name: 'Bottom panel height' });
  expect(Number(await splitter.getAttribute('aria-valuemax'))).toBeLessThanOrEqual(MIN.height - 99 - 140);
  // a stored width that is not a number falls back to the default for the drawer
  await page.getByRole('toolbar', { name: 'Capture' }).getByRole('button', { name: 'Inspector' }).click();
  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  await expect(inspector).toBeVisible();
  expect(Math.round((await inspector.locator('.inspector__content').boundingBox())!.width)).toBe(340);
  await inWindow(inspector.getByRole('button', { name: 'Close the inspector' }), 'the drawer’s close button');
  await app.close();
});

test('measurements keep their rows together, and a long channel name shortens in the sidebar', async () => {
  const { app, page } = await start([fixture('captures/mystery_C.demo')]);
  await size(app, page, MIN.width, MIN.height);
  const name = page.locator('.channels__name').first();
  await name.dblclick();
  const edit = page.getByRole('textbox', { name: /^New name for CH/ });
  await edit.fill('A rather long channel name for the sensor clock line');
  await edit.press('Enter');
  await expect(name).toContainText('A rather long channel name');
  expect(await name.evaluate((e) => e.scrollWidth > e.clientWidth), 'the name is shortened').toBe(true);
  await inWindow(page.getByRole('complementary', { name: 'Channels' }).getByRole('button', { name: /^Hide CH/ }).first(), 'the hide button');

  await page.getByRole('tab', { name: 'Measurements' }).click();
  // the statistics beside the cursors are taller than they are; wait for them
  await expect(page.getByText('Narrowest pulse', { exact: true })).toBeVisible();
  const a = (await page.getByText('Cursor A', { exact: true }).boundingBox())!;
  const b = (await page.getByText('Cursor B', { exact: true }).boundingBox())!;
  expect(b.y - a.y, 'cursor B right under cursor A (one line and the gap)').toBeLessThan(40);
  await app.close();
});
