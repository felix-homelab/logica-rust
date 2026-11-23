// The main process's decisions: restarting the engine after a crash, restoring the window, and the
// log file's rotation.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterExit, restorable, MAX_CRASHES } from '../src/main/policy.ts';
import { rotate } from '../src/main/log.ts';

test('an engine that cannot start is not restarted in a loop', () => {
  const r = afterExit(2, "Cannot find module 'logica.node'", [], 0);
  assert.equal(r.kind, 'fail');
  assert.match(r.reason, /could not start: Cannot find module/);
});

test('a crashed engine restarts with a growing delay, then gives up', () => {
  const crashes = [];
  const delays = [];
  for (let i = 0; i < MAX_CRASHES; i++) {
    const r = afterExit(1, null, crashes, 1000 + i * 1000);
    assert.equal(r.kind, 'restart');
    delays.push(r.delayMs);
  }
  assert.deepEqual(delays, [500, 1000, 1500]);
  const last = afterExit(null, null, crashes, 5000);
  assert.equal(last.kind, 'fail');
  assert.match(last.reason, /4 times within a minute/);
});

test('crashes older than a minute are forgiven', () => {
  const crashes = [0, 1000, 2000];
  const r = afterExit(1, null, crashes, 90_000);
  assert.equal(r.kind, 'restart');
  assert.equal(r.delayMs, 500);
});

test('a window comes back where it was, if that is still on a screen', () => {
  const displays = [{ x: 0, y: 0, width: 1920, height: 1080 }];
  const saved = { bounds: { x: 100, y: 80, width: 1200, height: 800 }, maximized: true };
  assert.deepEqual(restorable(saved, displays), saved);
  // on a screen that is gone, too small, or broken: the default
  assert.equal(restorable({ bounds: { x: 3000, y: 80, width: 1200, height: 800 } }, displays), null);
  assert.equal(restorable({ bounds: { x: 0, y: 0, width: 100, height: 80 } }, displays), null);
  assert.equal(restorable({ bounds: { x: 'a' } }, displays), null);
  assert.equal(restorable(null, displays), null);
  // a second screen to the left
  const two = [...displays, { x: -1920, y: 0, width: 1920, height: 1080 }];
  assert.ok(restorable({ bounds: { x: -1500, y: 50, width: 1000, height: 700 } }, two));
});

test('a window saved on a bigger screen comes back no bigger than the screen it opens on', () => {
  const small = [{ x: 0, y: 0, width: 1024, height: 728 }];
  assert.deepEqual(restorable({ bounds: { x: 200, y: 100, width: 1440, height: 900 } }, small), {
    bounds: { x: 0, y: 0, width: 1024, height: 728 },
    maximized: false,
  });
  // only one side too big: that side shrinks, the window moves just enough to fit
  assert.deepEqual(restorable({ bounds: { x: 300, y: 20, width: 900, height: 800 }, maximized: true }, small), {
    bounds: { x: 124, y: 0, width: 900, height: 728 },
    maximized: true,
  });
  // on the second of two screens, measured against that one
  const two = [{ x: 0, y: 0, width: 1920, height: 1080 }, { x: 1920, y: 0, width: 1280, height: 1024 }];
  assert.deepEqual(restorable({ bounds: { x: 2000, y: 10, width: 1600, height: 900 } }, two)?.bounds, { x: 1920, y: 10, width: 1280, height: 900 });
});

test('the log rotates at its size limit and keeps three old files', () => {
  const dir = mkdtempSync(join(tmpdir(), 'logica-log-'));
  const f = join(dir, 'logica.log');
  for (let i = 1; i <= 5; i++) {
    writeFileSync(f, `run ${i}\n`.repeat(20));
    rotate(f, 50, 3);
  }
  assert.equal(existsSync(f), false, 'the big file moved away');
  assert.equal(readFileSync(join(dir, 'logica.1.log'), 'utf8').split('\n')[0], 'run 5');
  assert.equal(readFileSync(join(dir, 'logica.3.log'), 'utf8').split('\n')[0], 'run 3');
  assert.equal(existsSync(join(dir, 'logica.4.log')), false);
  writeFileSync(f, 'small\n');
  rotate(f, 50, 3);
  assert.equal(readFileSync(f, 'utf8'), 'small\n', 'small files stay');
});
