// The waveform's view arithmetic and the number formats the window shows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clampRange, zoomAt, pan, sampleToX, xToSample, snap, centreOn, MIN_SPAN, atEnd, following, liveView, adoptEngineView } from '../src/renderer/src/viewmath.ts';
import { time, elapsed, hz, int, tick, tickStep, at } from '../src/renderer/src/format.ts';

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

import { activityNeeds, sessionLiveNeeds, sessionNeeds, tableNeeds, waveformNeeds } from '../src/renderer/src/relevance.ts';

test("the person's own panning refreshes nothing but their view", () => {
  const pan = { event: 'view.changed', capture: 'c1', actor: 'user' };
  assert.equal(sessionNeeds(pan), false);
  assert.equal(waveformNeeds(pan, 'c1'), false);
  assert.equal(tableNeeds(pan, 'c1', 'd1'), false);
  assert.equal(activityNeeds(pan), false);
  // an agent moving the view is news
  assert.equal(sessionNeeds({ ...pan, actor: 'agent:claude-code' }), true);
});

test('each part reads again only for its own capture and decoder', () => {
  const dec = { event: 'decoder.updated', capture: 'c1', decoder: 'd1' };
  assert.equal(waveformNeeds(dec, 'c1'), true);
  assert.equal(waveformNeeds(dec, 'c2'), false);
  assert.equal(tableNeeds(dec, 'c1', 'd1'), true);
  assert.equal(tableNeeds(dec, 'c1', 'd2'), false);
  assert.equal(tableNeeds(dec, 'c1', null), true);
  assert.equal(waveformNeeds({ event: 'capture.done', capture: 'c1' }, 'c1'), true);
  assert.equal(activityNeeds({ event: 'command.executed' }), true);
  assert.equal(activityNeeds(dec), false);
  assert.equal(sessionNeeds({ event: 'state.changed' }), true);
  assert.equal(sessionNeeds({ event: 'capture.progress', capture: 'c1' }), false);
});

test('a live capture refreshes the session a few times a second, not on every update', () => {
  const progress = { event: 'capture.progress', capture: 'c1', samples: 400 };
  const dec = { event: 'decoder.updated', capture: 'c1', decoder: 'd1' };
  // both come about 20 times a second while capturing, so they wait for the slower refresh
  assert.equal(sessionNeeds(progress), false);
  assert.equal(sessionNeeds(dec), false);
  assert.equal(sessionLiveNeeds(progress), true);
  assert.equal(sessionLiveNeeds(dec), true);
  assert.equal(sessionLiveNeeds({ event: 'state.changed' }), false);
  // the end of a capture is read at once
  assert.equal(sessionNeeds({ event: 'capture.done', capture: 'c1' }), true);
});

test('a live view follows the newest data until the person scrolls away', () => {
  assert.deepEqual(following(100, 1000), { from: 900, to: 1000 });
  assert.deepEqual(following(100, 40), { from: 0, to: 40 }, 'a young capture shows whole');
  assert.equal(atEnd({ from: 900, to: 999 }, 1000), true, 'within 2 % of the view');
  assert.equal(atEnd({ from: 800, to: 900 }, 1000), false);
  const shown = following(100, 1000);
  // a cursor or selection change keeps following
  assert.deepEqual(liveView(shown, shown, 1200, 100), { span: 100, range: { from: 1100, to: 1200 } });
  // zooming keeps following, at the new width
  assert.deepEqual(liveView(shown, zoomAt(shown, 0.5, 0.5, 1000), 1000, 100), { span: 50, range: { from: 950, to: 1000 } });
  // panning back stops following and keeps the place the person chose
  assert.deepEqual(liveView(shown, pan(shown, -0.5, 1000), 1000, 100), { span: null, range: { from: 850, to: 950 } });
  // a small nudge that still shows the end keeps following
  assert.deepEqual(liveView(shown, { from: 899, to: 999 }, 1000, 100), { span: 100, range: { from: 900, to: 1000 } });
  // not following: moving around stays put; coming back to the end follows again
  assert.deepEqual(liveView({ from: 0, to: 100 }, { from: 10, to: 110 }, 5000, null), { span: null, range: { from: 10, to: 110 } });
  assert.deepEqual(liveView({ from: 0, to: 100 }, { from: 4900, to: 5000 }, 5000, null), { span: 100, range: { from: 4900, to: 5000 } });
});

test('a running time keeps two decimals, so it does not jump while it counts', () => {
  assert.equal(elapsed(0), '0.00 s');
  assert.equal(elapsed(12.3), '12.30 s');
  assert.equal(elapsed(12.349), '12.34 s', 'cut, not rounded');
  assert.equal(elapsed(59.999), '59.99 s', 'never shows a minute before it is reached');
  assert.equal(elapsed(60), '1 min 00.00 s');
  assert.equal(elapsed(65.25), '1 min 05.25 s');
  assert.equal(elapsed(7385.25), '2 h 03 min 05.25 s');
  assert.equal(elapsed(-1), '–');
  assert.equal(elapsed(Number.NaN), '–');
  // the same width from 10 s to 59.99 s
  const widths = new Set([10, 10.1, 11.11, 25.5, 59.99].map((t) => elapsed(t).length));
  assert.equal(widths.size, 1);
});

test("a session does not take back the person's own view; an agent's change does", () => {
  const v = (mine, engine, seen, sending = false) => adoptEngineView({ mine, engine, seen, sending });
  // a new tab takes the engine's view
  assert.equal(v(null, 'e', null), true);
  // the engine's view unchanged since last seen: the person's new cursor stays
  assert.equal(v('mine+cursor', 'e', 'e'), false);
  // never seen before (the first click on a tab): it stays too; this was the bug
  assert.equal(v('mine+cursor', 'e', null), false);
  // while the person's change is being sent, nothing replaces it
  assert.equal(v('mine', 'agent', 'e', true), false);
  // an agent moved the view: it differs from what was seen and from the person's
  assert.equal(v('mine', 'agent', 'e'), true);
  // the engine already shows the person's view: nothing to take
  assert.equal(v('same', 'same', 'e'), false);
});
