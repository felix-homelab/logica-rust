// The window's performance budgets (doc 08), measured in the real app. Heavy, so only on request:
// LOGICA_PERF=1 LOGICA_PERF_CAPTURE=<a 16-channel .lacap from the make_capture example>
// npm run test:e2e -w apps/desktop -- perf. Prints the numbers for docs/benchmarks.md.
import { expect, test } from '@playwright/test';
import { fixture, start } from './helpers';

test.skip(!process.env.LOGICA_PERF, 'set LOGICA_PERF=1 (and LOGICA_PERF_CAPTURE) to measure');

test('the window meets the doc 08 budgets', async () => {
  const big = process.env.LOGICA_PERF_CAPTURE;
  const { app, page } = await start();
  await expect(page.getByRole('button', { name: 'Open file…' })).toBeVisible();

  // long tasks on the renderer's main thread, from here on
  await page.evaluate(() => {
    const w = window as unknown as { longTasks: number[] };
    w.longTasks = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => w.longTasks.push(e.duration))).observe({ type: 'longtask', buffered: true });
  });

  const result: Record<string, unknown> = {};

  // import and index a 50 M-sample, 6-channel .demo (budget: under 2 s)
  result.importDemo = await page.evaluate(async (path) => {
    const t0 = performance.now();
    const r = (await window.logica.call('file.open', { path })) as { capture: { id: string; sampleCount: number } };
    const t1 = performance.now();
    await window.logica.call('waveform.get', { capture: r.capture.id, from: 0, to: r.capture.sampleCount, buckets: 1200 });
    return { openMs: Math.round(t1 - t0), firstViewMs: Math.round(performance.now() - t0) };
  }, fixture('captures/mystery_D.demo'));

  if (big) {
    // open a 1e9-sample, 16-channel .lacap: first view (budget: under 2 s)
    result.openBig = await page.evaluate(async (path) => {
      const t0 = performance.now();
      const r = (await window.logica.call('file.open', { path })) as { capture: { id: string; sampleCount: number } };
      const t1 = performance.now();
      await window.logica.call('waveform.get', { capture: r.capture.id, from: 0, to: r.capture.sampleCount, buckets: 1200 });
      await window.logica.call('view.set', { capture: r.capture.id, activate: true });
      return { openMs: Math.round(t1 - t0), firstViewMs: Math.round(performance.now() - t0) };
    }, big);
    await expect(page.getByRole('navigation', { name: 'Open captures' }).getByRole('button', { name: big.split(/[\\/]/).pop()!, exact: true })).toBeVisible();
    await page.waitForTimeout(500);

    // pan and zoom, 16 channels (budget: 60 fps, no frame over 33 ms)
    result.panZoom = await page.evaluate(
      () =>
        new Promise((resolve) => {
          const c = document.querySelector('.wave__canvas') as HTMLCanvasElement;
          const r = c.getBoundingClientRect();
          const frames: number[] = [];
          const t0 = performance.now();
          let last = t0;
          let i = 0;
          const step = (t: number) => {
            frames.push(t - last);
            last = t;
            const zoomIn = Math.floor(i / 40) % 2 === 0;
            const at = { clientX: r.left + r.width * (0.3 + 0.4 * ((i % 7) / 7)), clientY: r.top + 60, bubbles: true, cancelable: true };
            c.dispatchEvent(new WheelEvent('wheel', { ...at, ctrlKey: true, deltaY: zoomIn ? -5 : 5 }));
            if (i % 3 === 0) c.dispatchEvent(new WheelEvent('wheel', { ...at, deltaX: 60 }));
            i++;
            if (t - t0 < 4000) requestAnimationFrame(step);
            else {
              const f = frames.slice(1).sort((a, b) => a - b);
              const pct = (p: number) => Math.round(f[Math.min(f.length - 1, Math.floor(f.length * p))]! * 10) / 10;
              resolve({ frames: f.length, fps: Math.round((f.length / (t - t0)) * 1000), p50: pct(0.5), p95: pct(0.95), max: pct(1), over33: f.filter((x) => x > 33).length });
            }
          };
          requestAnimationFrame(step);
        }),
    );
  }

  // change a decoder setting until the visible window can be redrawn (budget: under 100 ms)
  result.decoderChange = await page.evaluate(async (path) => {
    const r = (await window.logica.call('file.open', { path })) as { capture: { id: string } };
    const d = (await window.logica.call('decoder.add', { capture: r.capture.id, protocol: 'i2c', roles: { scl: 5, sda: 4 } })) as { decoder: { id: string } };
    const times: number[] = [];
    for (const fmt of ['8bit', '7bit', '8bit', '7bit', '8bit']) {
      const t0 = performance.now();
      await window.logica.call('decoder.update', { capture: r.capture.id, decoder: d.decoder.id, settings: { addressFormat: fmt } });
      await window.logica.call('waveform.get', { capture: r.capture.id, from: 6_590_000, to: 6_700_000, buckets: 1200 });
      times.push(performance.now() - t0);
    }
    times.sort((a, b) => a - b);
    return { medianMs: Math.round(times[2]!), maxMs: Math.round(times[4]!) };
  }, fixture('captures/mystery_C.demo'));

  result.longTasks = await page.evaluate(() => {
    const t = (window as unknown as { longTasks: number[] }).longTasks;
    return { count: t.length, over50: t.filter((x) => x > 50).length, maxMs: Math.round(Math.max(0, ...t)) };
  });

  console.log(`LOGICA_PERF ${JSON.stringify(result)}`);
  await app.close();
});
