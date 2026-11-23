// The waveform's view arithmetic and the number formats the window shows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clampRange, zoomAt, pan, sampleToX, xToSample, snap, centreOn, MIN_SPAN } from '../src/renderer/src/viewmath.ts';
import { time, hz, int, tick, tickStep, at } from '../src/renderer/src/format.ts';

test('ranges stay inside the capture and never get too small', () => {
  assert.deepEqual(clampRange({ from: -50, to: 50 }, 1000), { from: 0, to: 100 });
  assert.deepEqual(clampRange({ from: 950, to: 1100 }, 1000), { from: 850, to: 1000 });
  assert.deepEqual(clampRange({ from: 10, to: 12 }, 1000), { from: 10, to: 10 + MIN_SPAN });
  assert.deepEqual(clampRange({ from: 0, to: 5000 }, 1000), { from: 0, to: 1000 });
  assert.deepEqual(clampRange({ from: 3, to: 4 }, 5), { from: 0, to: 5 }, 'a tiny capture shows whole');
});

test('zooming keeps the sample under the pointer still', () => {
  const r = { from: 1000, to: 2000 };
  const z = zoomAt(r, 0.5, 0.25, 1e6);
  assert.deepEqual(z, { from: 1125, to: 1625 });
  assert.equal(xToSample(0.25 * 800, z, 800), xToSample(0.25 * 800, r, 800));
  // zooming out past the capture shows all of it
  assert.deepEqual(zoomAt(r, 1e6, 0.5, 5000), { from: 0, to: 5000 });
});

test('panning moves by a share of the view and stops at the ends', () => {
  assert.deepEqual(pan({ from: 100, to: 200 }, 0.1, 1000), { from: 110, to: 210 });
  assert.deepEqual(pan({ from: 100, to: 200 }, -5, 1000), { from: 0, to: 100 });
  assert.deepEqual(pan({ from: 800, to: 1000 }, 1, 1000), { from: 800, to: 1000 });
});

test('samples and pixels map both ways', () => {
  const r = { from: 0, to: 1000 };
  assert.equal(sampleToX(250, r, 400), 100);
  assert.equal(xToSample(100, r, 400), 250);
});

test('cursors snap to a nearby edge, not to a far one', () => {
  const r = { from: 0, to: 1000 };
  const edges = [100, 300, 302, 900];
  assert.equal(snap(305, edges, r, 1000), 302);
  assert.equal(snap(99.4, edges, r, 1000), 100);
  assert.equal(snap(500, edges, r, 1000), 500, 'nothing within 6 px');
  assert.equal(snap(42.4, [], r, 1000), 42);
  assert.equal(snap(2000, edges, r, 1000), 2000);
});

test('centring keeps the span and the capture bounds', () => {
  assert.deepEqual(centreOn(500, 100, 1000), { from: 450, to: 550 });
  assert.deepEqual(centreOn(10, 100, 1000), { from: 0, to: 100 });
});

test('numbers read as the engine writes them', () => {
  assert.equal(time(0), '0 s');
  assert.equal(time(8.6806e-6), '8.681 µs');
  assert.equal(time(1.5e-3), '1.5 ms');
  assert.equal(time(2), '2 s');
  assert.equal(hz(115200), '115.2 kHz');
  assert.equal(hz(4e5), '400 kHz');
  assert.equal(int(10000008), '10,000,008');
  assert.equal(at(6_600_000, 10e6), '660 ms (sample 6,600,000)');
  assert.equal(at(5, 0), 'sample 5');
  assert.equal(tick(0.660005, 5e-6), '660.005 ms');
  assert.equal(tick(0, 1), '0');
  assert.equal(tickStep(1, 8), 0.2);
  assert.equal(tickStep(0.0004, 8), 0.00005);
});
