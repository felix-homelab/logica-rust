// Decoders in the window: their channels grouped together, and their data written out in the
// format chosen per decoder (the waveform's text row), undoable like every change.
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { fixture, start } from './helpers';

async function shot(page: Page, name: string): Promise<void> {
  if (process.env.LOGICA_SHOTS) await page.screenshot({ path: `${process.env.LOGICA_SHOTS}/${name}.png` });
}

test("a decoder's channels are grouped, and its data is shown in the format chosen", async () => {
  const { app, page } = await start([fixture('captures/4_SPI_wrong_mode.demo')]);
  const channels = page.getByRole('complementary', { name: 'Channels' });
  await expect(channels.getByRole('button', { name: /^CH3/ })).toBeVisible();
  // no decoder yet: one plain list
  await expect(channels.getByRole('region')).toHaveCount(0);

  await page.getByRole('tab', { name: 'Decoders' }).click();
  const add = page.getByRole('region', { name: 'Add a decoder' });
  await add.getByLabel('Protocol').selectOption({ label: 'SPI' });
  for (const [role, ch] of [
    ['CLK', '3'],
    ['MISO', '4'],
    ['CS', '5'],
    ['MOSI', '6'],
  ] as const) {
    await add.getByLabel(new RegExp(`^${role}`)).selectOption(ch);
  }
  await add.getByRole('button', { name: /^Add SPI/ }).click();

  // the four SPI channels under the decoder's name, in SPI's order (CLK, CS, MOSI, MISO) rather
  // than the file's (MISO is CH4); the file names them after their roles already, so the role is
  // not repeated ("CS#" says CS); no other channels are left
  const group = channels.getByRole('region', { name: /^Channels of SPI/ });
  await expect(group.getByRole('button', { name: /^CH/ })).toHaveText(['CH3 CLK', 'CH5 CS#', 'CH6 MOSI', 'CH4 MISO']);
  await expect(channels.getByRole('region', { name: 'Other channels' })).toHaveCount(0);

  const card = page.getByRole('region', { name: /^Decoder SPI/ });
  const format = card.getByLabel('Show data as');
  await expect(format).toHaveValue('ascii');
  const description = page.locator('#wave-description');
  await expect(description).toContainText('data shown as ascii');
  await format.selectOption({ label: 'Binary' });
  await expect(description).toContainText('data shown as binary');
  await expect(format).toHaveValue('binary');
  await page.waitForTimeout(300);
  await shot(page, 'decoder-binary');
  await page.getByRole('button', { name: /^3\s+700 ms/ }).click();
  await page.waitForTimeout(300);
  await shot(page, 'decoder-binary-packet');

  // the change is in the history: Ctrl+Z puts ASCII back
  await page.locator('.wave__canvas').focus();
  await page.keyboard.press('Control+z');
  await expect(format).toHaveValue('ascii');
  await expect(description).toContainText('data shown as ascii');

  let r = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);

  // the same settings straight from the channel list
  await group.getByRole('button', { name: 'Settings of SPI' }).click();
  const dialog = page.getByRole('dialog', { name: 'SPI settings' });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel('Show data as').selectOption({ label: 'Decimal' });
  await expect(description).toContainText('data shown as decimal');
  // a protocol setting decodes again: mode 1 is right for this recording (the file's notes)
  await dialog.getByLabel('Mode').selectOption('1');
  await expect(dialog.getByLabel('Mode')).toHaveValue('1');
  await expect(page.getByText(/Problems changed|Decoded again/)).toBeVisible();
  r = await new AxeBuilder({ page }).setLegacyMode(true).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
  await shot(page, 'decoder-settings-dialog');
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(dialog).toHaveCount(0);
  // the panel shows the same decoder, changed (it is hidden from assistive tech while the dialog is open)
  await expect(card.getByLabel('Show data as')).toHaveValue('decimal');
  await app.close();
});

test('UART text reads as a string under its bytes', async () => {
  const { app, page } = await start([fixture('captures/1_UART_easy.demo')]);
  const card = page.getByRole('region', { name: /Bus CH0/ });
  await card.getByRole('button', { name: 'Add decoder' }).click();
  await expect(card.getByRole('button', { name: 'Decoder added' })).toBeDisabled();
  // the channel's own name ("RX"), then the role it has in the decoder
  const group = page.getByRole('complementary', { name: 'Channels' }).getByRole('region', { name: /^Channels of UART/ });
  await expect(group.getByRole('button', { name: /^CH0/ })).toHaveText('CH0 RX DATA');
  // jump to "OK\r\n" (100 ms) and look at it
  await page.getByRole('button', { name: /OK\\r\\n/ }).first().click();
  await page.waitForTimeout(400);
  await shot(page, 'uart-text');
  // further out the bytes are too narrow for one value each: the packet reads as one string
  await page.locator('.wave__canvas').focus();
  for (let i = 0; i < 3; i++) await page.keyboard.press('-');
  await page.waitForTimeout(400);
  await shot(page, 'uart-text-out');
  await app.close();
});

test('a decoder gets a name when added, and is renamed in its settings', async () => {
  const { app, page } = await start([fixture('captures/4_SPI_wrong_mode.demo')]);
  await page.getByRole('tab', { name: 'Decoders' }).click();
  const add = page.getByRole('region', { name: 'Add a decoder' });
  await add.getByLabel('Protocol').selectOption({ label: 'SPI' });
  for (const [role, ch] of [
    ['CLK', '3'],
    ['MISO', '4'],
    ['CS', '5'],
    ['MOSI', '6'],
  ] as const) {
    await add.getByLabel(new RegExp(`^${role}`)).selectOption(ch);
  }
  // the protocol's name is the suggestion
  await expect(add.getByLabel('Name (optional)')).toHaveAttribute('placeholder', 'SPI');
  await add.getByLabel('Name (optional)').fill('Flash bus');
  await add.getByRole('button', { name: /^Add SPI/ }).click();
  const channels = page.getByRole('complementary', { name: 'Channels' });
  await expect(channels.getByRole('region', { name: 'Channels of Flash bus' })).toBeVisible();
  // the form is ready for the next one
  await expect(add.getByLabel('Name (optional)')).toHaveValue('');

  // renamed in its settings; Enter takes it
  await channels.getByRole('button', { name: 'Settings of Flash bus' }).click();
  const dialog = page.getByRole('dialog', { name: 'Flash bus settings' });
  await dialog.getByLabel('Name', { exact: true }).fill('Sensor bus');
  await dialog.getByLabel('Name', { exact: true }).press('Enter');
  // (the channel list is hidden from assistive tech while the dialog is open)
  const renamed = page.getByRole('dialog', { name: 'Sensor bus settings' });
  await expect(renamed).toBeVisible();
  // an empty name is not taken: the field shows the name again
  await renamed.getByLabel('Name', { exact: true }).fill('');
  await renamed.getByLabel('Name', { exact: true }).press('Enter');
  await expect(renamed.getByLabel('Name', { exact: true })).toHaveValue('Sensor bus');
  await renamed.getByRole('button', { name: 'Done' }).click();
  await expect(channels.getByRole('region', { name: 'Channels of Sensor bus' })).toBeVisible();
  // undo brings the old name back
  await page.locator('.wave__canvas').focus();
  await page.keyboard.press('Control+z');
  await expect(channels.getByRole('region', { name: 'Channels of Flash bus' })).toBeVisible();
  await app.close();
});
