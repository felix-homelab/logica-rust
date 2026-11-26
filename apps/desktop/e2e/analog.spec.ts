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
  const form = dialog.getByRole('form', { name: 'Add a virtual analog channel' });
  // nothing to add before a source and its channel are chosen
  await expect(form.getByRole('button', { name: 'Add channel' })).toBeDisabled();
  await form.getByLabel('Source').selectOption({ label: 'PWM level' });
  await expect(form.getByText(/One value per period/)).toBeVisible();
  await form.getByLabel(/^Signal/).selectOption('0');
  await form.getByLabel('Lowest value').fill('0');
  await form.getByLabel('Highest value').fill('100');
  await form.getByLabel('Unit').fill('%');
  const axe = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
  expect(axe.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  await shot(page, 'analog-dialog');
  await form.getByRole('button', { name: 'Add channel' }).click();
  await expect(dialog).toHaveCount(0);

  const list = channels.getByRole('region', { name: 'Virtual analog channels' });
  await expect(list).toContainText('PWM level');
  await expect(list).toContainText('%');
  await expect(page.locator('#wave-description')).toContainText(/Virtual analog channels PWM level \(30(\.\d+)? %/, { timeout: 10_000 });
  await shot(page, 'analog-row');

  // rescaled to volts in the same dialog: nothing is computed again, the values follow
  await list.getByRole('button', { name: 'Settings of PWM level' }).click();
  const change = page.getByRole('dialog', { name: 'PWM level settings' });
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
