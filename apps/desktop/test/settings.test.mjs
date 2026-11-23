// The app's settings file: broken or unknown values fall back, the recent list stays short and
// unique, and only capture files among the command-line arguments are opened.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DEFAULTS, filesIn, load, sanitize, save, withRecent } from '../src/main/settings.ts';

const dir = mkdtempSync(join(tmpdir(), 'logica-settings-'));

test('a missing or broken file gives the defaults', () => {
  assert.deepEqual(load(join(dir, 'missing.json')), DEFAULTS);
  writeFileSync(join(dir, 'broken.json'), '{ not json');
  assert.deepEqual(load(join(dir, 'broken.json')), DEFAULTS);
});

test('values out of range or of the wrong kind fall back', () => {
  const s = sanitize({ ...DEFAULTS, theme: 'neon', mcp: 'yes', memoryMb: 1, recent: ['a.demo', 3, 'b.sr'] });
  assert.equal(s.theme, 'system');
  assert.equal(s.mcp, true);
  assert.equal(s.memoryMb, 64);
  assert.deepEqual(s.recent, ['a.demo', 'b.sr']);
  assert.equal(sanitize({ ...DEFAULTS, memoryMb: 1e9 }).memoryMb, 65536);
});

test('settings survive a save and load', () => {
  const path = join(dir, 'sub', 'settings.json');
  const s = { ...DEFAULTS, theme: 'dark', expert: true, recent: ['x.lacap'] };
  save(path, s);
  assert.deepEqual(load(path), s);
});

test('the recent list puts the newest first, once, and keeps ten', () => {
  let s = { ...DEFAULTS };
  for (let i = 0; i < 12; i++) s = withRecent(s, `f${i}.demo`);
  s = withRecent(s, 'f5.demo');
  assert.equal(s.recent.length, 10);
  assert.equal(s.recent[0], 'f5.demo');
  assert.equal(s.recent.filter((f) => f === 'f5.demo').length, 1);
});

test('only capture files among the arguments are opened', () => {
  assert.deepEqual(filesIn(['/opt/Logica/logica-app', '--started-by-agent', '/home/a/x.DEMO', 'notes.txt', 'b.lacap', '--flag=a.sr']), [
    '/home/a/x.DEMO',
    'b.lacap',
  ]);
});
