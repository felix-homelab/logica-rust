// Panel sizes fitted to the window: remembered sizes never push the waveform off a small window.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fitPanels, DEFAULTS, LIMITS, MIN_WAVE_W, MIN_WAVE_H, CHROME_H, DRAWER_GAP } from '../src/renderer/src/panelsize.ts';

const both = { sidebar: true, dockedInspector: true };

test('sizes that fit are kept as they are', () => {
  const f = fitPanels({ sidebar: 260, inspector: 380, bottom: 300 }, { width: 1600, height: 1000 }, both);
  assert.deepEqual(f.size, { sidebar: 260, inspector: 380, bottom: 300 });
  assert.deepEqual(f.max, { sidebar: 420, inspector: 560, bottom: 600 });
});

test('the largest sizes in the smallest docked window leave the waveform its room', () => {
  const win = { width: 1280, height: 720 };
  const f = fitPanels({ sidebar: 420, inspector: 560, bottom: 600 }, win, both);
  assert.ok(win.width - f.size.sidebar - f.size.inspector >= MIN_WAVE_W, `waveform ${win.width - f.size.sidebar - f.size.inspector} px wide`);
  assert.ok(win.height - CHROME_H - f.size.bottom >= MIN_WAVE_H, `waveform ${win.height - CHROME_H - f.size.bottom} px high`);
  assert.equal(f.size.sidebar, 420, 'the sidebar keeps its size; the inspector gives way');
  assert.equal(f.size.inspector, 540);
  assert.equal(f.max.inspector, 540);
});

test('the bottom panel never covers the waveform in the minimum window (960 x 600)', () => {
  const f = fitPanels({ bottom: 600 }, { width: 960, height: 600 }, { sidebar: true, dockedInspector: false });
  assert.equal(f.size.bottom, 600 - CHROME_H - MIN_WAVE_H);
  assert.equal(f.max.bottom, f.size.bottom);
});

test('a drawer inspector is sized to the window, not to the sidebar', () => {
  const f = fitPanels({ sidebar: 420, inspector: 560 }, { width: 500, height: 600 }, { sidebar: true, dockedInspector: false });
  assert.equal(f.size.inspector, 500 - DRAWER_GAP);
  // a hidden sidebar gives the docked inspector its room
  const g = fitPanels({ sidebar: 420, inspector: 560 }, { width: 900, height: 600 }, { sidebar: false, dockedInspector: true });
  assert.equal(g.size.inspector, 560);
});

test('panels never go below their minimum, even in a window too small for everything', () => {
  const f = fitPanels({ sidebar: 10, inspector: 10, bottom: 10 }, { width: 200, height: 100 }, both);
  assert.deepEqual(f.size, { sidebar: LIMITS.sidebar[0], inspector: LIMITS.inspector[0], bottom: LIMITS.bottom[0] });
  for (const k of ['sidebar', 'inspector', 'bottom']) assert.ok(f.max[k] >= f.size[k], `${k}: max below the size`);
});

test('stored values that are not numbers fall back to the defaults', () => {
  const f = fitPanels({ sidebar: 'wide', inspector: null, bottom: Number.NaN }, { width: 1600, height: 1000 }, both);
  assert.deepEqual(f.size, DEFAULTS);
  const g = fitPanels({ sidebar: Infinity, inspector: 300.6 }, { width: 1600, height: 1000 }, both);
  assert.equal(g.size.sidebar, DEFAULTS.sidebar);
  assert.equal(g.size.inspector, 301, 'rounded to whole pixels');
});
