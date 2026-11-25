// Live capture (doc 08) from the virtual analyser (`logica-vla`, a separate program on a loopback
// port): the capture appears as a tab at once, the view follows the newest data, a decoder added
// while capturing keeps decoding, and stopping ends it with what arrived.
import { spawn, type ChildProcess } from 'node:child_process';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { repo, start } from './helpers';

async function shot(page: Page, name: string): Promise<void> {
  if (process.env.LOGICA_SHOTS) await page.screenshot({ path: `${process.env.LOGICA_SHOTS}/${name}.png` });
}

/** Starts the virtual analyser on a free port (built by cargo if needed); resolves with its address. */
function analyser(): Promise<{ at: string; proc: ChildProcess }> {
  const proc = spawn('cargo', ['run', '-q', '-p', 'logica-vla', '--', '--listen', '127.0.0.1:0', '--seed', '5'], { cwd: repo });
  return new Promise((resolve, reject) => {
    let out = '';
    const timer = setTimeout(() => reject(new Error(`logica-vla did not start within 3 minutes: ${out}`)), 180_000);
    proc.stdout?.on('data', (b: Buffer) => {
      out += String(b);
      const m = /Listening on (\S+)\./.exec(out);
      if (m?.[1]) {
        clearTimeout(timer);
        resolve({ at: m[1], proc });
      }
    });
    proc.stderr?.on('data', (b: Buffer) => (out += String(b)));
    proc.on('exit', (code) => reject(new Error(`logica-vla ended (${code}): ${out}`)));
  });
}

const seconds = async (page: Page): Promise<string> => (await page.locator('.live__bar .lg-num').textContent()) ?? '';

test('a live capture follows the newest data, decodes while it runs and stops', async () => {
  test.setTimeout(300_000);
  const vla = await analyser();
  try {
    const { app, page } = await start([], { env: { LOGICA_VLA: vla.at } });
    // the start screen finds the analyser and offers to capture from it
    await page.getByRole('button', { name: 'Capture live' }).click();
    const bar = page.getByRole('group', { name: 'Live capture' });
    await expect(bar).toBeVisible();
    await expect(bar).toContainText('Following the newest data.');
    await expect(page.locator('.tabs').getByRole('button', { name: /^Live: Bench/ })).toBeVisible();
    const first = await seconds(page);
    await expect.poll(() => seconds(page), { timeout: 10_000 }).not.toBe(first);

    // a UART decoder on CH0 while capturing: its packets keep coming
    await page.getByRole('tab', { name: 'Decoders' }).click();
    const add = page.getByRole('region', { name: 'Add a decoder' });
    await add.getByLabel('Protocol').selectOption({ label: 'UART' });
    await add.getByLabel(/^Data/).selectOption('0');
    await add.getByRole('button', { name: /^Add UART/ }).click();
    const packets = page.getByRole('status').filter({ hasText: /packets$/ });
    await expect(packets).toBeVisible();
    const count = async () => Number(((await packets.textContent()) ?? '0').replace(/\D/g, ''));
    const before = await count();
    await expect.poll(count, { timeout: 15_000 }).toBeGreaterThan(before + 2);
    await shot(page, 'live-following');

    // looking back stops following; "Follow" goes back to the newest data
    await page.locator('.wave__canvas').focus();
    await page.keyboard.press('Home');
    await expect(bar).toContainText('You are looking back');
    await bar.getByRole('button', { name: 'Follow' }).click();
    await expect(bar).toContainText('Following the newest data.');

    const r = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);

    // stopping ends the capture with what arrived; the live bar goes, the decode stays
    await bar.getByRole('button', { name: 'Stop capture' }).click();
    await expect(bar).toHaveCount(0);
    await expect(page.locator('.tabs').getByText('capturing')).toHaveCount(0);
    expect(await count()).toBeGreaterThan(before + 2);
    await shot(page, 'live-stopped');
    await app.close();
  } finally {
    vla.proc.kill();
  }
});
