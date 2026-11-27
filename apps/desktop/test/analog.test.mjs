import { test } from 'node:test';
import assert from 'node:assert/strict';
import { difference, formatValue, readout, tickCount, valueTickStep, valueLines, valueRange, valueTicks, valueY } from '../src/renderer/src/analogplot.ts';
import { layoutRows, ROW_HEIGHT } from '../src/renderer/src/lanes.ts';
import { analogNeeds, sessionLiveNeeds, waveformNeeds } from '../src/renderer/src/relevance.ts';

test('a value range has room around the values and is never empty', () => {
  assert.deepEqual(valueRange(0, 100), [-8, 108]);
  assert.deepEqual(valueRange(5, 5), [4.5, 5.5]);
  assert.deepEqual(valueRange(0, 0), [-1, 1]);
  assert.deepEqual(valueRange(null, undefined), [0, 1]);
});

test('ticks are round numbers inside the range', () => {
  assert.deepEqual(valueTicks(-8, 108, 4), [0, 50, 100]);
  assert.deepEqual(valueTicks(0, 3.3, 4), [0, 1, 2, 3]);
  assert.deepEqual(valueTicks(-0.25, 0.25, 4), [-0.2, 0, 0.2]);
  assert.deepEqual(valueTicks(-0.25, 0.25, 2), [0]);
  assert.deepEqual(valueTicks(1, 1, 4), []);
  assert.deepEqual(valueTicks(0, 1, 0), []);
});

test('a taller row gets more ticks, and the tick distance sets the decimals', () => {
  assert.equal(tickCount(146), 5);
  assert.equal(tickCount(30), 2);
  // the sine of the virtual analyser: 10 % to 90 % in a 160-pixel row
  const [lo, hi] = valueRange(10, 90);
  assert.deepEqual(valueTicks(lo, hi, tickCount(146)), [20, 40, 60, 80]);
  assert.equal(formatValue(40, '%', valueTickStep(lo, hi, 5)), '40 %');
  assert.equal(valueTickStep(0.1, 0.2, 2), 0.05);
  assert.equal(valueTickStep(1, 1, 3), 0);
});

test('values map to rows with the highest at the top', () => {
  assert.equal(valueY(10, 0, 10, 100, 50), 100);
  assert.equal(valueY(0, 0, 10, 100, 50), 150);
  assert.equal(valueY(5, 0, 10, 100, 50), 125);
});

test('values are written with the decimals one step tells apart', () => {
  assert.equal(formatValue(1.6304, 'V', 3.3 / 4096), '1.6304 V');
  assert.equal(formatValue(30, '%', 1), '30 %');
  assert.equal(formatValue(1000, 'Hz'), '1000.0 Hz');
  assert.equal(formatValue(0.00025, 's'), '0.0003 s');
  assert.equal(formatValue(2.5, ''), '2.500');
});

test('steps hold each value until the next point and stop at gaps', () => {
  const pts = [
    { sample: 10, value: 1 },
    { sample: 20, value: 2 },
    { sample: 30, value: null },
    { sample: 40, value: 3 },
  ];
  assert.deepEqual(valueLines(pts, 0.5, 0, 50, 'step'), [
    [[0, 0.5], [10, 0.5], [10, 1], [20, 1], [20, 2], [30, 2]],
    [[40, 3], [50, 3]],
  ]);
  // lines join the points; a range that starts in a gap has no held value
  assert.deepEqual(valueLines(pts, null, 0, 50, 'line'), [
    [[10, 1], [20, 2], [30, 2]],
    [[40, 3], [50, 3]],
  ]);
  // a gap at the end leaves nothing after it, and no points means the held value across
  assert.deepEqual(valueLines([{ sample: 5, value: null }], 1, 0, 9, 'step'), [[[0, 1], [5, 1]]]);
  assert.deepEqual(valueLines([], 2, 0, 9, 'step'), [[[0, 2], [9, 2]]]);
  assert.deepEqual(valueLines([], null, 0, 9, 'line'), []);
});

test('virtual analog channels get a row of their own after everything else', () => {
  const rows = layoutRows([{ index: 0, name: 'PWM', hidden: false }], [], 10, [{ id: 'analog-3', config: { name: 'Fan' } }]);
  assert.deepEqual(rows.map((r) => `${r.kind}:${r.label}`), ['channel:CH0 PWM', 'analog:Fan']);
  const a = rows[1];
  assert.equal(a.analog, 'analog-3');
  assert.equal(a.h, ROW_HEIGHT.analog);
  assert.equal(a.y, 10 + ROW_HEIGHT.channel);
  assert.equal(ROW_HEIGHT.analog, 160, 'tall enough to read a curve');
  // a height of its own, and the rows after it move down
  const two = layoutRows([], [], 0, [
    { id: 'a', config: { name: 'A', height: 300 } },
    { id: 'b', config: { name: 'B' } },
  ]);
  assert.deepEqual(two.map((r) => [r.y, r.h]), [
    [0, 300],
    [300, ROW_HEIGHT.analog],
  ]);
});

test('analog values are read again for their own capture only', () => {
  const up = { event: 'analog.updated', capture: 'c1', analog: 'analog-1' };
  assert.equal(analogNeeds(up, 'c1'), true);
  assert.equal(analogNeeds(up, 'c2'), false);
  assert.equal(analogNeeds({ event: 'capture.done', capture: 'c1' }, 'c1'), true);
  assert.equal(analogNeeds({ event: 'decoder.updated', capture: 'c1' }, 'c1'), false);
  assert.equal(waveformNeeds(up, 'c1'), false);
  assert.equal(sessionLiveNeeds(up), true);
});

test('cursor readouts give the value, the code where there is one, and the difference', () => {
  assert.equal(readout({ value: 1.6304, code: 2048 }, 'V', 3.3 / 4096), '1.6304 V (code 2048)');
  assert.equal(readout({ value: 30 }, '%', null), '30.000 %');
  assert.equal(readout({}, '%'), 'no value');
  assert.equal(readout(undefined, '%'), 'no value');
  assert.equal(difference({ value: 20 }, { value: 30 }, '%', 1), '+10 %');
  assert.equal(difference({ value: 30 }, { value: 20 }, '%', 1), '-10 %');
  assert.equal(difference({ value: 30 }, {}, '%', 1), null);
});

test('a range in one line', async () => {
  const { analogSummary } = await import('../src/renderer/src/analogplot.ts');
  assert.equal(
    analogSummary({ unit: 'V', min: 1.6, max: 1.6, peakToPeak: 0, mean: 1.6, rms: 1.6, covered: 0.98, step: null }),
    'mean 1.600 V · RMS 1.600 V · 1.600 V to 1.600 V · peak to peak 0.0000 V · values for 98 % of it',
  );
  assert.equal(analogSummary({ unit: '%', mean: 30, rms: 30, min: 29, max: 31, peakToPeak: 2, covered: 1, step: 1 }), 'mean 30 % · RMS 30 % · 29 % to 31 % · peak to peak 2 %');
  assert.equal(analogSummary({ unit: 'V', covered: 0 }), 'no value in this range');
});
