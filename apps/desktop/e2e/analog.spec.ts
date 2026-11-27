// Virtual analog channels in the window (ADR 0010): added from the channel list through a dialog
// built from the engine's sources, drawn as a row of their own, changed and undone like every edit.
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { fixture, start } from './helpers';

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
