// The engine's Node-API module as the engine process loads it, in plain Node (no Electron):
// answers as JSON envelopes, errors that never throw, events by long poll.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { existsSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const modulePath = join(here, '..', 'native', 'logica.node');
const repo = join(here, '..', '..', '..');

test('the module is built (npm run engine)', () => {
  assert.ok(existsSync(modulePath), `${modulePath} is missing; run npm run build first`);
});

const { LogicaEngine } = createRequire(import.meta.url)(modulePath);
const engine = new LogicaEngine(mkdtempSync(join(tmpdir(), 'logica-node-test-')));
const run = async (command, input = {}) => JSON.parse(await engine.execute('user', command, JSON.stringify(input)));

test('commands answer with the command API', async () => {
  assert.equal(engine.apiVersion(), '1.0');
  const r = await run('session.get');
  assert.equal(r.ok, true);
  assert.equal(r.output.apiVersion, '1.0');
  assert.ok(r.output.devices.length > 0);
});

test('errors come back as envelopes, not exceptions', async () => {
  const r = await run('capture.close', { capture: 'c99' });
  assert.deepEqual([r.ok, r.error.code], [false, 'not-found']);
  const bad = JSON.parse(await engine.execute('robot', 'session.get', '{}'));
  assert.match(bad.error.message, /is not an actor/);
  const nojson = JSON.parse(await engine.execute('user', 'session.get', '{nope'));
  assert.match(nojson.error.message, /is not JSON/);
});

test('a recording opens and its events arrive by long poll', async () => {
  const empty = JSON.parse(await engine.nextEvents(20));
  assert.ok(Array.isArray(empty));
  const r = await run('file.open', { path: join(repo, 'fixtures', 'captures', 'mystery_C.demo') });
  assert.equal(r.ok, true, JSON.stringify(r.error));
  assert.equal(r.output.capture.sampleCount, 10_000_008);
  const events = JSON.parse(await engine.nextEvents(2000)).map((e) => e.event);
  assert.ok(events.includes('state.changed'), JSON.stringify(events));
});

test('commands run off the JavaScript thread', async () => {
  // several detections at once: the event loop keeps turning while they run
  let ticks = 0;
  const timer = setInterval(() => ticks++, 1);
  const all = await Promise.all([1, 2, 3].map(() => run('analysis.detect', { capture: 'c1' })));
  clearInterval(timer);
  assert.ok(all.every((r) => r.ok));
  assert.ok(ticks > 0, 'the event loop was blocked');
});
