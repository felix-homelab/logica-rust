// Virtual analog channels in the window (ADR 0010): added from the channel list through a dialog
// built from the engine's sources, drawn as a row of their own, changed and undone like every edit.
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { execFile, spawn } from 'node:child_process';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { Agent, fixture, repo, start } from './helpers';

async function shot(page: Page, name: string): Promise<void> {
  if (process.env.LOGICA_SHOTS) await page.screenshot({ path: `${process.env.LOGICA_SHOTS}/${name}.png` });
}

test('a PWM level is added as a virtual analog channel, rescaled and undone', async () => {
  // mystery_D has a 1 kHz PWM at 30 % duty on CH0
  const { app, page } = await start([fixture('captures/mystery_D.demo')]);
  const channels = page.getByRole('complementary', { name: 'Channels' });
  await channels.getByRole('button', { name: 'Add virtual analog channel' }).click();

  const dialog = page.getByRole('dialog', { name: 'Add a virtual analog channel' });
  const next = dialog.getByRole('button', { name: 'Next' });
  // step 1, the source: nothing goes on before a source and its channel are chosen, and the
  // later steps cannot be skipped to
  await expect(next).toBeDisabled();
  await expect(dialog.getByRole('button', { name: /Scaling/ })).toBeDisabled();
  await dialog.getByLabel('Source').selectOption({ label: 'PWM level' });
  await expect(dialog.getByText(/One value per period/)).toBeVisible();
  await expect(dialog.getByText('Choose a channel for every role that is not optional.')).toBeVisible();
  await dialog.getByLabel('Signal').selectOption('0');
  await next.click();
  // step 2, the PWM's own settings: duty is the default
  await expect(dialog.getByLabel('Value')).toHaveValue('duty');
  await next.click();
  // step 3, the scaling
  await dialog.getByLabel('Lowest value').fill('0');
  await dialog.getByLabel('Highest value').fill('100');
  await dialog.getByLabel('Unit').fill('%');
  const axe = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
  expect(axe.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  await shot(page, 'analog-dialog');
  await next.click();
  // step 4, name and look: the row height starts at 160 pixels
  await expect(dialog.getByLabel('Row height (pixels)')).toHaveValue('160');
  await dialog.getByRole('button', { name: 'Add channel' }).click();
  await expect(dialog).toHaveCount(0);

  const list = channels.getByRole('region', { name: 'Virtual analog channels' });
  await expect(list).toContainText('PWM level');
  await expect(list).toContainText('%');
  await expect(page.locator('#wave-description')).toContainText(/Virtual analog channels PWM level \(30(\.\d+)? %/, { timeout: 10_000 });
  await shot(page, 'analog-row');

  // rescaled to volts in the same dialog: nothing is computed again, the values follow
  await list.getByRole('button', { name: 'Settings of PWM level' }).click();
  const change = page.getByRole('dialog', { name: 'PWM level settings' });
  // an existing channel opens on its first step, and any step can be visited
  await change.getByRole('button', { name: /Scaling/ }).click();
  await change.getByLabel('Highest value').fill('5');
  await change.getByLabel('Unit').fill('V');
  await change.getByRole('button', { name: 'Apply' }).click();
  await expect(page.locator('#wave-description')).toContainText(/PWM level \(1\.5\d* V/, { timeout: 10_000 });

  // undo takes the scaling back, and once more the channel itself
  await page.locator('.wave__canvas').focus();
  await page.keyboard.press('Control+z');
  await expect(page.locator('#wave-description')).toContainText(/PWM level \(30(\.\d+)? %/, { timeout: 10_000 });
  await page.keyboard.press('Control+z');
  await expect(channels.getByRole('region', { name: 'Virtual analog channels' })).toHaveCount(0);
  await expect(page.locator('#wave-description')).not.toContainText('Virtual analog');
  await app.close();
});

test('a PWM found by the analysis becomes a virtual analog channel in one click', async () => {
  const { app, page } = await start([fixture('captures/mystery_D.demo')]);
  const add = page.getByRole('button', { name: 'Add PWM level' });
  await expect(add).toBeVisible({ timeout: 20_000 });
  await add.click();
  const list = page.getByRole('complementary', { name: 'Channels' }).getByRole('region', { name: 'Virtual analog channels' });
  await expect(list).toContainText('PWM level');
  await expect(list).toContainText('30');
  // once there, the analysis says so instead of adding a second one
  await expect(page.getByRole('button', { name: 'PWM level added' })).toBeDisabled();
  // a cursor in the middle reads the level there
  const canvas = page.locator('.wave__canvas');
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width * 0.6, y: 40 } });
  await page.getByRole('tab', { name: 'Measurements' }).click();
  await expect(page.getByText(/A: 30(\.\d+)? %/)).toBeVisible();
  await app.close();
});

test("an ADC on SPI from the virtual analyser, set up with the chip's preset", async () => {
  test.setTimeout(300_000);
  // the analyser's built-in mix sends an MCP3201 sine (10 % to 90 % of full scale) on CH13 to CH15
  await promisify(execFile)('cargo', ['build', '-q', '-p', 'logica-vla'], { cwd: repo, timeout: 600_000 });
  const proc = spawn(join(repo, 'target', 'debug', process.platform === 'win32' ? 'logica-vla.exe' : 'logica-vla'), ['--listen', '127.0.0.1:0', '--seed', '5'], { cwd: repo });
  try {
    const at = await new Promise<string>((resolve, reject) => {
      let out = '';
      proc.stdout?.on('data', (b: Buffer) => {
        out += String(b);
        const m = /Listening on (\S+)\./.exec(out);
        if (m?.[1]) resolve(m[1]);
      });
      proc.on('exit', (code) => reject(new Error(`logica-vla ended (${code}): ${out}`)));
    });
    const { app, page, dataDir } = await start([], { env: { LOGICA_VLA: at } });
    await expect(page.getByRole('button', { name: 'Capture live…', exact: true })).toBeVisible();
    // a second of it, captured as an agent would
    const agent = await Agent.connect(dataDir, 'test');
    const cmd = (command: string, input: unknown) => agent.tool('run_command', { command, input });
    await cmd('device.connect', { device: `vla/${at}` });
    await cmd('device.configure', { device: `vla/${at}`, config: { channels: [13, 14, 15], sampleRateHz: 4e6, samples: 4_000_000, mode: 'stream' } });
    await cmd('capture.start', { device: `vla/${at}`, title: 'ADC' });

    await page.getByRole('complementary', { name: 'Channels' }).getByRole('button', { name: 'Add virtual analog channel' }).click();
    const dialog = page.getByRole('dialog', { name: 'Add a virtual analog channel' });
    await dialog.getByLabel('Source').selectOption({ label: 'ADC on SPI' });
    await dialog.getByLabel(/^CLK/).selectOption('13');
    await dialog.getByLabel(/^CS/).selectOption('14');
    await dialog.getByLabel(/^MISO/).selectOption('15');
    await dialog.getByRole('button', { name: 'Next' }).click();
    await dialog.getByLabel('ADC chip').selectOption({ label: 'MCP3201 (12 bits, SPI)' });
    await expect(dialog.getByText(/two sampling bits and a null bit/)).toBeVisible();
    await expect(dialog.getByLabel('Bits before the sample')).toHaveValue('3');
    await dialog.getByRole('button', { name: 'Next' }).click();
    // the chip's scaling: 0 to 3.3 V
    await expect(dialog.getByLabel('Highest value')).toHaveValue('3.3');
    await expect(dialog.getByLabel('Unit')).toHaveValue('V');
    await dialog.getByRole('button', { name: 'Next' }).click();
    await dialog.getByRole('button', { name: 'Add channel' }).click();
    // 10 % to 90 % of 3.3 V
    await expect(page.locator('#wave-description')).toContainText(/MCP3201 \(0\.3\d* V to 2\.9\d* V\)/, { timeout: 20_000 });
    await app.close();
  } finally {
    proc.kill();
  }
});
