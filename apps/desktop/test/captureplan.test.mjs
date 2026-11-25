// Planning a capture from one or several devices (the "Capture live" dialog): shared rates, first
// choices, sample counts, and problems explained before anything starts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { UNTIL_STOPPED_SECONDS, channelLabel, commonRates, configFor, defaultChoice, problems, samplesFor, titleFor } from '../src/renderer/src/captureplan.ts';
import { hz } from '../src/renderer/src/format.ts';

const vla = {
  channels: [0, 1, 2],
  channelNames: ['TX', 'CH1', 'SCL'],
  ratesHz: [4e6, 10e6, 25e6],
  limits: [],
  maxSamples: null,
  modes: ['stream'],
  threshold: null,
};
const dla = {
  channels: [0, 1, 2, 3, 4, 5, 6, 7],
  channelNames: [],
  ratesHz: [1e6, 10e6, 25e6, 100e6],
  limits: [
    { maxChannels: 4, maxRateHz: 100e6 },
    { maxChannels: 8, maxRateHz: 25e6 },
  ],
  maxSamples: 1_000_000,
  modes: ['buffer', 'stream'],
  threshold: [-5, 5, 0.1],
};
const seconds = (s) => ({ untilStopped: false, seconds: s });

test('devices share the rates each of them offers', () => {
  assert.deepEqual(commonRates([vla, dla]), [10e6, 25e6]);
  assert.deepEqual(commonRates([vla]), [4e6, 10e6, 25e6]);
  assert.deepEqual(commonRates([vla, { ...dla, ratesHz: [1e6] }]), [], 'no rate in common');
  assert.deepEqual(commonRates([]), []);
});

test('channels show the names their device gives them', () => {
  assert.equal(channelLabel(vla, 0), 'CH0 TX');
  assert.equal(channelLabel(vla, 1), 'CH1', 'a name that only repeats the number is left out');
  assert.equal(channelLabel(dla, 5), 'CH5', 'a device that names none');
});

test('a first choice takes every channel, a streaming device its lowest rate, a buffer its fastest allowed', () => {
  const v = defaultChoice(vla, 'Virtual analyser: Bench');
  assert.deepEqual([v.channels, v.rateHz, v.mode, v.title], [[0, 1, 2], 4e6, 'stream', 'Live: Bench']);
  const b = defaultChoice({ ...dla, modes: ['buffer'] }, 'DLA-16 Plus');
  assert.deepEqual([b.channels.length, b.rateHz, b.mode, b.title], [8, 25e6, 'buffer', 'Live: DLA-16 Plus']);
  assert.equal(titleFor('Virtual analyser: Second bench'), 'Live: Second bench');
});

test('the length comes to samples per channel; until stopped is a day or what the device holds', () => {
  assert.equal(samplesFor(vla, 4e6, seconds(2.5)), 10_000_000);
  assert.equal(samplesFor(vla, 4e6, seconds(1e-9)), 1, 'never zero');
  assert.equal(samplesFor(vla, 4e6, { untilStopped: true, seconds: 0 }), 4e6 * UNTIL_STOPPED_SECONDS);
  assert.equal(samplesFor(dla, 1e6, { untilStopped: true, seconds: 0 }), 1_000_000);
});

test('a good choice has no problems and becomes the configuration the engine takes', () => {
  const c = { ...defaultChoice(vla, 'Virtual analyser: Bench'), channels: [2, 0] };
  assert.deepEqual(problems(vla, c, seconds(1), hz), []);
  assert.deepEqual(configFor(vla, c, seconds(1)), { channels: [0, 2], sampleRateHz: 4e6, samples: 4e6, mode: 'stream' });
  const t = { ...defaultChoice(dla, 'DLA'), channels: [0], rateHz: 1e6, thresholdVolts: 1.6 };
  assert.equal(configFor(dla, t, seconds(0.5)).thresholdVolts, 1.6);
});

test('problems are explained, one sentence each', () => {
  const base = { channels: [0, 1], rateHz: 10e6, mode: 'stream', thresholdVolts: null, title: 'Live: x' };
  const say = (caps, c, len = seconds(1)) => problems(caps, { ...base, ...c }, len, hz);
  assert.deepEqual(say(vla, { channels: [] }), ['Choose at least one channel.']);
  assert.deepEqual(say(vla, { title: '  ' }), ['Give the capture a name.']);
  assert.deepEqual(say(vla, {}, seconds(0)), ['Choose a length above zero seconds.']);
  assert.match(say(vla, { rateHz: 1e6 })[0], /^1 MHz is not a rate this device offers; it offers 4 MHz, 10 MHz, 25 MHz\.$/);
  assert.match(say(dla, { channels: [0, 1, 2, 3, 4], rateHz: 100e6 }, seconds(0.001))[0], /100 MHz is not available with 5 channels; the most is 25 MHz/);
  assert.deepEqual(say(dla, { channels: [0, 1, 2, 3], rateHz: 100e6 }, seconds(0.001)), [], 'four channels may go faster');
  assert.match(say(dla, { rateHz: 1e6 }, seconds(2))[0], /holds at most 1,000,000 samples per channel, 1 s at this rate/);
  assert.deepEqual(say(dla, { rateHz: 1e6 }, { untilStopped: true, seconds: 0 }), [], 'until stopped fills the device');
  assert.deepEqual(say(vla, { mode: 'buffer' }), ['This device cannot capture in that mode.']);
  assert.deepEqual(say(vla, { thresholdVolts: 1 }), ['This device has no adjustable threshold.']);
  assert.deepEqual(say(dla, { thresholdVolts: 9, rateHz: 1e6 }, seconds(0.5)), ['The threshold must be between -5 V and 5 V.']);
});
