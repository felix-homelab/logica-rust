// Live capture (doc 08) from the virtual analyser (`logica-vla`, a separate program on a loopback
// port): the capture appears as a tab at once, the view follows the newest data, a decoder added
// while capturing keeps decoding, and stopping ends it with what arrived.
import { execFile, spawn, type ChildProcess } from 'node:child_process';
import { connect, createServer } from 'node:net';
import { join } from 'node:path';
import { promisify } from 'node:util';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { Agent, repo, start } from './helpers';

async function shot(page: Page, name: string): Promise<void> {
  if (process.env.LOGICA_SHOTS) await page.screenshot({ path: `${process.env.LOGICA_SHOTS}/${name}.png` });
}

// built once, then started directly: `cargo run` on Windows runs the program as its own child,
// which killing cargo leaves running (still listening), so a test could not make it go away
let built: Promise<string> | null = null;
function program(): Promise<string> {
  built ??= promisify(execFile)('cargo', ['build', '-q', '-p', 'logica-vla'], { cwd: repo, timeout: 600_000 }).then(() =>
    join(repo, 'target', 'debug', process.platform === 'win32' ? 'logica-vla.exe' : 'logica-vla'),
  );
  return built;
}

/** Starts the virtual analyser on a free port (built by cargo first); resolves with its address. */
async function analyser(extra: string[] = []): Promise<{ at: string; proc: ChildProcess }> {
  const proc = spawn(await program(), ['--listen', '127.0.0.1:0', '--seed', '5', ...extra], { cwd: repo });
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

/** What answers at `host:port`: the start of a greeting, a refusal, or silence. */
function greetOnce(at: string): Promise<string> {
  const [host, port] = at.split(':');
  return new Promise((resolve) => {
    const c = connect({ host, port: Number(port) });
    let got = '';
    const done = (what: string) => {
      c.destroy();
      resolve(what);
    };
    const timer = setTimeout(() => done(got ? `partial "${got.slice(0, 40)}"` : 'connected, no greeting'), 3_000);
    c.on('data', (b: Buffer) => {
      got += String(b);
      if (got.includes('\n')) {
        clearTimeout(timer);
        done(`greeted "${got.slice(0, 60)}"`);
      }
    });
    c.on('error', (e: NodeJS.ErrnoException) => {
      clearTimeout(timer);
      done(`error ${e.code}`);
    });
  });
}

/** A loopback port free at the moment. */
function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.once('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const address = srv.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

const seconds = async (page: Page): Promise<string> => (await page.locator('.live__bar .lg-num').textContent()) ?? '';

test('a live capture follows the newest data, decodes while it runs and stops', async () => {
  test.setTimeout(300_000);
  const vla = await analyser();
  try {
    const { app, page } = await start([], { env: { LOGICA_VLA: vla.at } });
    // "Capture live…" finds the analyser, picks it (the only one) and starts with its first settings
    await page.getByRole('button', { name: 'Capture live…', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Capture live' });
    await expect(dialog.getByRole('checkbox', { name: /Virtual analyser: Bench/ })).toBeChecked();
    // one step per thing to decide: devices, how long, the device's channels, then the start
    const steps = dialog.getByRole('navigation', { name: 'Steps' }).getByRole('button');
    await expect(steps).toHaveText(['1Devices', '2Capture', '3Bench', '4Check and start']);
    await dialog.getByRole('button', { name: 'Next: Capture' }).click();
    await expect(dialog.getByRole('radio', { name: 'For a time' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Next: Bench' }).click();
    await expect(dialog.getByRole('checkbox', { name: 'CH0 TX' })).toBeChecked();
    await dialog.getByRole('button', { name: 'Next: Check and start' }).click();
    await expect(dialog).toContainText('22 of 22 channels');
    await dialog.getByRole('button', { name: 'Start capture' }).click();
    await expect(dialog).toHaveCount(0);
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

test('several analysers capture together, each into its own tab, with settings checked first', async () => {
  test.setTimeout(300_000);
  const [one, two] = await Promise.all([analyser(), analyser(['--name', 'Second'])]);
  try {
    const { app, page } = await start([], { env: { LOGICA_VLA: `${one.at},${two.at}` } });
    await page.getByRole('button', { name: 'Capture live…', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Capture live' });
    const bench = dialog.getByRole('checkbox', { name: /Virtual analyser: Bench/ });
    const second = dialog.getByRole('checkbox', { name: /Virtual analyser: Second/ });
    // with two free devices, none is picked for the person
    await expect(bench).not.toBeChecked();
    await expect(dialog.getByRole('button', { name: 'Next: Capture' })).toBeDisabled();
    await bench.check();
    await second.check();
    // a step for each device
    const bar = dialog.getByRole('navigation', { name: 'Steps' });
    await expect(bar.getByRole('button')).toHaveText(['1Devices', '2Capture', '3Bench', '4Second', '5Check and start']);
    await dialog.getByRole('button', { name: 'Next: Capture' }).click();

    // one rate for both by default; one rate per device, and until stopped
    await expect(dialog.getByRole('radio', { name: 'The same for all devices' })).toBeChecked();
    await expect(dialog.getByLabel('Rate for all')).toBeVisible();
    await dialog.getByRole('radio', { name: 'One per device' }).check();
    await dialog.getByRole('radio', { name: 'Until I stop it' }).check();

    await dialog.getByRole('button', { name: 'Next: Bench' }).click();
    const benchSettings = dialog.getByRole('region', { name: 'Settings for Virtual analyser: Bench' });
    await expect(benchSettings.getByLabel('Tab name')).toHaveValue('Live: Bench');
    await expect(benchSettings.getByLabel('Sample rate')).toBeVisible();
    await dialog.getByRole('button', { name: 'Next: Second' }).click();
    const secondSettings = dialog.getByRole('region', { name: 'Settings for Virtual analyser: Second' });
    await expect(secondSettings.getByLabel('Tab name')).toHaveValue('Live: Second');

    // a problem is explained and holds the next steps back
    await secondSettings.getByRole('button', { name: 'none' }).click();
    await expect(secondSettings.getByRole('list', { name: /Problems/ })).toContainText('Choose at least one channel.');
    await expect(dialog.getByRole('button', { name: 'Next: Check and start' })).toBeDisabled();
    await expect(bar.getByRole('button', { name: /Check and start/ })).toBeDisabled();
    // earlier steps stay open: the step bar goes back and forth
    await bar.getByRole('button', { name: /Capture/ }).click();
    await expect(dialog.getByRole('radio', { name: 'Until I stop it' })).toBeChecked();
    await bar.getByRole('button', { name: /Second/ }).click();
    await secondSettings.getByRole('checkbox', { name: 'CH0 TX' }).check();
    await secondSettings.getByRole('checkbox', { name: 'CH9 NOISE' }).check();
    await expect(secondSettings).toContainText('2 of 22 channels');
    const r = await new AxeBuilder({ page }).include('[role="dialog"]').setLegacyMode(true).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
    await shot(page, 'capture-dialog');

    await dialog.getByRole('button', { name: 'Next: Check and start' }).click();
    await expect(dialog).toContainText('Each device into its own tab');
    await expect(dialog).toContainText('2 of 22 channels');
    const go = dialog.getByRole('button', { name: 'Start 2 captures' });
    await go.click();
    await expect(dialog).toHaveCount(0);
    const tabs = page.locator('.tabs');
    await expect(tabs.getByRole('button', { name: /^Live: Bench/ })).toBeVisible();
    await expect(tabs.getByRole('button', { name: /^Live: Second/ })).toBeVisible();
    await expect(tabs.getByText('capturing')).toHaveCount(2);

    // both stop from the device panel; each keeps what arrived
    await page.getByRole('tab', { name: 'Device', exact: true }).click();
    for (const name of ['Virtual analyser: Bench', 'Virtual analyser: Second']) {
      await page.getByRole('region', { name, exact: true }).getByRole('button', { name: 'Stop capture' }).click();
    }
    await expect(tabs.getByText('capturing')).toHaveCount(0);
    await app.close();
  } finally {
    one.proc.kill();
    two.proc.kill();
  }
});

test('two analysers go into one capture, lined up by the clock they share', async () => {
  test.setTimeout(300_000);
  const [one, two] = await Promise.all([analyser(), analyser(['--name', 'Second'])]);
  try {
    const { app, page } = await start([], { env: { LOGICA_VLA: `${one.at},${two.at}` } });
    await page.getByRole('button', { name: 'Capture live…', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Capture live' });
    await dialog.getByRole('checkbox', { name: /Virtual analyser: Bench/ }).check();
    await dialog.getByRole('checkbox', { name: /Virtual analyser: Second/ }).check();
    await dialog.getByRole('button', { name: 'Next: Capture' }).click();
    await dialog.getByRole('radio', { name: 'All into one capture' }).check();
    await expect(dialog.getByLabel('Capture name')).toHaveValue('Live: Bench + Second');
    // one capture has one rate; the shared SYNC line is found on both by its name
    await expect(dialog.getByRole('radio', { name: 'One per device' })).toHaveCount(0);
    // one capture adds a step: how the devices are lined up
    await expect(dialog.getByRole('navigation', { name: 'Steps' }).getByRole('button')).toHaveText(['Devices', '2Capture', '3Line up', '4Bench', '5Second', '6Check and start']);
    await dialog.getByRole('button', { name: 'Next: Line up' }).click();
    await expect(dialog.getByRole('radio', { name: 'By a signal they all see' })).toBeChecked();
    await expect(dialog.getByLabel('Shared signal on Virtual analyser: Second')).toHaveValue('10');

    // the line it is lined up by must be captured; the problem shows at the device it is about
    await dialog.getByRole('button', { name: 'Next: Bench' }).click();
    await dialog.getByRole('button', { name: 'Next: Second' }).click();
    const second = dialog.getByRole('region', { name: 'Settings for Virtual analyser: Second' });
    await second.getByRole('checkbox', { name: 'CH10 SYNC' }).uncheck();
    await expect(second.getByRole('list', { name: 'Problems with Virtual analyser: Second' })).toContainText('Virtual analyser: Second: capture CH10 SYNC too');
    await expect(dialog.getByRole('button', { name: 'Next: Check and start' })).toBeDisabled();
    await second.getByRole('checkbox', { name: 'CH10 SYNC' }).check();
    await dialog.getByRole('button', { name: 'Next: Check and start' }).click();
    await expect(dialog).toContainText('By Bench CH10 SYNC, Second CH10 SYNC');
    const go = dialog.getByRole('button', { name: 'Start capture' });
    const r = await new AxeBuilder({ page }).include('[role="dialog"]').setLegacyMode(true).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
    await shot(page, 'capture-together');

    await go.click();
    await expect(dialog).toHaveCount(0);
    const tabs = page.locator('.tabs');
    await expect(tabs.getByRole('button', { name: /^Live: Bench \+ Second/ })).toBeVisible();
    await expect(tabs.getByText('capturing')).toHaveCount(1);
    // both devices' channels, named by device
    await expect(page.getByText('Second: SYNC').first()).toBeVisible();
    await expect(page.getByText('Bench: SYNC').first()).toBeVisible();
    await shot(page, 'capture-together-live');
    await page.getByRole('group', { name: 'Live capture' }).getByRole('button', { name: 'Stop capture' }).click();
    await expect(tabs.getByText('capturing')).toHaveCount(0);
    await app.close();
  } finally {
    one.proc.kill();
    two.proc.kill();
  }
});

test('the start screen shows an analyser that starts or stops while it is open', async () => {
  test.setTimeout(300_000);
  const at = `127.0.0.1:${await freePort()}`;
  const { app, page, dataDir } = await start([], { env: { LOGICA_VLA: at } });
  await expect(page.getByText(/No analyser found/)).toBeVisible();
  const vla = await analyser(['--listen', at]);
  try {
    // nobody pressed anything: the window looks again by itself
    await expect(page.getByText('Virtual analyser: Bench')).toBeVisible({ timeout: 15_000 });
  } finally {
    vla.proc.kill();
  }
  // what is left once it was stopped (for a failure on one system only: Windows CI)
  const exited = await new Promise<string>((res) => {
    if (vla.proc.exitCode !== null || vla.proc.signalCode !== null) res(`exited (${vla.proc.exitCode ?? vla.proc.signalCode})`);
    else {
      vla.proc.once('exit', (c, s) => res(`exited (${c ?? s})`));
      setTimeout(() => res('still running after 5 s'), 5_000);
    }
  });
  console.log(`analyser ${exited}; ${at} after it stopped: ${await greetOnce(at)}`);
  await page.waitForTimeout(5_000);
  console.log(`start screen 5 s later: ${(await page.locator('.start').innerText()).replace(/\s+/g, ' ')}; ${at}: ${await greetOnce(at)}`);
  // the engine's own answer, asked as an agent would
  const agent = await Agent.connect(dataDir, 'diagnosis');
  for (const command of ['device.list', 'session.get']) {
    const said = await agent.tool('run_command', { command, input: {} }).catch((e: unknown) => String(e));
    console.log(`engine ${command}: ${said.includes('vla/') ? 'lists the analyser' : 'lists no analyser'}: ${said.slice(0, 200).replace(/\s+/g, ' ')}`);
  }
  await expect(page.getByText('Virtual analyser: Bench')).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByText(/No analyser found/)).toBeVisible();
  await app.close();
});
