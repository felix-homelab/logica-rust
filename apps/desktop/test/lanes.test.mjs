// Channels grouped by decoder, the waveform's rows, and the text row's strings.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { groupChannels, layoutRows, runs, joinRun, roleOf, ROW_HEIGHT } from '../src/renderer/src/lanes.ts';

const ch = (index, extra = {}) => ({ index, name: `CH${index}`, hidden: false, ...extra });
const channels = [0, 1, 2, 3, 4, 5, 6].map((i) => ch(i));
const spi = { id: 'spi-1', config: { name: 'SPI mode 1', roles: { clk: 3, miso: 4, cs: 5, mosi: 6 }, dataFormat: 'hex' }, lanes: [4, 6], channels: [3, 5, 6, 4] };
const i2c = { id: 'i2c-1', config: { name: 'I2C', roles: { scl: 1, sda: 2 } }, lanes: [2] };
const shape = (rows) => rows.map((r) => `${r.kind}:${r.label}`);

test("a decoder's channels are grouped together, in the capture's channel order", () => {
  const g = groupChannels(channels, [spi, i2c]);
  assert.deepEqual(
    g.groups.map((x) => [x.decoder.id, x.channels.map((c) => c.index)]),
    [
      ['i2c-1', [1, 2]],
      ['spi-1', [3, 5, 6, 4]],
    ],
  );
  assert.deepEqual(
    g.other.map((c) => c.index),
    [0],
  );
  assert.deepEqual(g.loose, []);
});

test('a second decoder on the same wires joins the first one’s group; one on no channel is loose', () => {
  const uart9600 = { id: 'u1', config: { name: 'UART 9600', roles: { data: 0 } }, lanes: [0] };
  const uart115 = { id: 'u2', config: { name: 'UART 115200', roles: { data: 0 } }, lanes: [0] };
  const gone = { id: 'x', config: { name: 'Elsewhere', roles: { data: 42, unused: null } }, lanes: [42] };
  const g = groupChannels(channels, [uart9600, uart115, gone]);
  assert.equal(g.groups.length, 1);
  assert.deepEqual(
    g.groups[0].extra.map((d) => d.id),
    ['u2'],
  );
  assert.deepEqual(
    g.loose.map((d) => d.id),
    ['x'],
  );
  const rows = layoutRows(channels.slice(0, 2), [uart9600, uart115, gone], 0);
  assert.deepEqual(shape(rows), [
    'group:UART 9600 + UART 115200',
    'channel:CH0 DATA',
    'data:DATA',
    'text:ASCII',
    // the second decoder under the same channel says which it is
    'data:UART 115200',
    'text:ASCII',
    'channel:CH1',
    'data:Elsewhere',
    'text:ASCII',
  ]);
});

test('rows: a header per group, data rows under the channel the data is on, other channels last', () => {
  const rows = layoutRows(channels, [spi, i2c], 38);
  assert.deepEqual(shape(rows), [
    'group:I2C',
    'channel:CH1 SCL',
    'channel:CH2 SDA',
    'data:SDA',
    'text:ASCII',
    'group:SPI mode 1',
    'channel:CH3 CLK',
    'channel:CH5 CS',
    'channel:CH6 MOSI',
    'data:MOSI',
    'text:Hex',
    'channel:CH4 MISO',
    'data:MISO',
    'text:Hex',
    'channel:CH0',
  ]);
  // stacked without gaps from the top given
  assert.equal(rows[0].y, 38);
  for (let i = 1; i < rows.length; i++) assert.equal(rows[i].y, rows[i - 1].y + ROW_HEIGHT[rows[i - 1].kind]);
  // the data rows know their channel; the groups are numbered for their brackets
  assert.equal(rows.find((r) => r.label === 'MISO').lane, 4);
  assert.deepEqual([...new Set(rows.map((r) => r.group))], [0, 1, -1]);
  // trace colours follow the capture's order, not the grouping
  assert.equal(rows.find((r) => r.channel === 0).colourIndex, 0);
  assert.equal(rows.find((r) => r.channel === 3).colourIndex, 3);
});

test("a channel's own name wins over its role; hidden channels leave their data at the group's end", () => {
  const named = channels.map((c) => (c.index === 1 ? { ...c, name: 'clock' } : c.index === 2 ? { ...c, hidden: true } : c));
  const rows = layoutRows(named, [i2c], 0);
  assert.deepEqual(shape(rows).slice(0, 4), ['group:I2C', 'channel:CH1 clock', 'data:I2C', 'text:ASCII']);
  assert.equal(rows[2].lane, null, 'all of its frames, as its channel is not shown');
});

test('a decoder without data frames (yet) has one row and no text row', () => {
  const pwm = { id: 'p', config: { name: 'PWM', roles: { signal: 0 } }, lanes: [] };
  assert.deepEqual(shape(layoutRows(channels.slice(0, 1), [pwm], 0)), ['group:PWM', 'channel:CH0 SIGNAL', 'data:PWM']);
});

test('roles are written as on the wire', () => {
  assert.equal(roleOf(spi, 6), 'MOSI');
  assert.equal(roleOf(spi, 0), null);
});

test('data frames next to each other in one packet on one channel form one run', () => {
  const f = (start, packet, lane, text = 'x') => ({ start, end: start + 10, packet, lane, text });
  const r = runs([f(20, 1, 6), f(0, 1, 6), f(10, 1, 6), f(40, 2, 6), f(0, 1, 4), f(50, 2, 6, null)]);
  assert.deepEqual(
    r.map((run) => run.map((x) => `${x.lane}@${x.start}`)),
    [['4@0'], ['6@0', '6@10', '6@20'], ['6@40']],
  );
  assert.deepEqual(runs([]), []);
});

test('a run is one string: ASCII together, the other formats spaced', () => {
  assert.equal(joinRun(['O', 'K', '\\r', '\\n'], 'ascii'), 'OK\\r\\n');
  assert.equal(joinRun(['4F', '4B'], 'hex'), '4F 4B');
  assert.equal(joinRun(['79'], 'decimal'), '79');
});

test("a group follows the protocol's role order, whatever wires the signals are on", () => {
  // wired MISO, CS, CLK, MOSI on CH0 to CH3: shown CLK, CS, MOSI, MISO
  const wired = { id: 's', config: { name: 'SPI', roles: { miso: 0, cs: 1, clk: 2, mosi: 3 } }, lanes: [0, 3], channels: [2, 1, 3, 0] };
  const g = groupChannels(channels.slice(0, 4), [wired]);
  assert.deepEqual(
    g.groups[0].channels.map((c) => roleOf(wired, c.index)),
    ['CLK', 'CS', 'MOSI', 'MISO'],
  );
  // an engine that does not say (older) keeps the capture's order
  const unordered = { ...wired, channels: undefined };
  assert.deepEqual(
    groupChannels(channels.slice(0, 4), [unordered]).groups[0].channels.map((c) => c.index),
    [0, 1, 2, 3],
  );
  // groups are still placed by where their channels are in the capture
  const uart = { id: 'u', config: { name: 'UART', roles: { data: 5 } }, lanes: [5], channels: [5] };
  const late = { ...wired, config: { ...wired.config, roles: { miso: 6, cs: 1, clk: 2, mosi: 3 } }, channels: [2, 1, 3, 6] };
  assert.deepEqual(
    groupChannels(channels, [uart, late]).groups.map((x) => x.decoder.id),
    ['s', 'u'],
  );
});
