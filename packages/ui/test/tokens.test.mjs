// The token set meets WCAG AA in both themes (doc 08): text 4.5:1 on every surface it sits on,
// control outlines and the focus ring 3:1, channel colours readable on the waveform.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'tokens.css'), 'utf8');

/** The custom properties declared in the block that starts at `selector`. */
function block(selector) {
  const at = css.indexOf(selector);
  assert.ok(at >= 0, `${selector} is missing`);
  let i = css.indexOf('{', at) + 1;
  // the media query wraps the rule; step into the inner rule
  if (selector.startsWith('@media')) {
    i = css.indexOf('{', i) + 1;
  }
  const body = css.slice(i, css.indexOf('}', i));
  return Object.fromEntries([...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

const light = block(':root {');
const darkForced = block(':root[data-theme="dark"]');
const darkOs = block('@media (prefers-color-scheme: dark)');
const dark = { ...light, ...darkForced };

const lum = (hex) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = c.map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

test('the dark theme is the same whether forced or following the system', () => {
  assert.deepEqual(darkOs, darkForced);
});

for (const [name, t] of [['light', light], ['dark', dark]]) {
  test(`${name}: text meets 4.5:1 on every surface`, () => {
    for (const fg of ['--text', '--text-secondary', '--text-muted', '--accent', '--danger', '--warning', '--success', '--agent']) {
      for (const bg of ['--bg-app', '--bg-surface', '--bg-raised', '--bg-hover']) {
        const r = ratio(t[fg], t[bg]);
        assert.ok(r >= 4.5, `${name} ${fg} on ${bg}: ${r.toFixed(2)}`);
      }
    }
    assert.ok(ratio(t['--on-accent'], t['--accent-bg']) >= 4.5, `${name} --on-accent on --accent-bg`);
    assert.ok(ratio(t['--on-accent'], t['--accent-bg-hover']) >= 4.5, `${name} --on-accent on --accent-bg-hover`);
    for (const [fg, bg] of [['--danger', '--danger-bg'], ['--warning', '--warning-bg'], ['--success', '--success-bg'], ['--agent', '--agent-bg'], ['--text', '--bg-selected']]) {
      assert.ok(ratio(t[fg], t[bg]) >= 4.5, `${name} ${fg} on ${bg}: ${ratio(t[fg], t[bg]).toFixed(2)}`);
    }
  });

  test(`${name}: control outlines and the focus ring meet 3:1`, () => {
    for (const fg of ['--border-strong', '--focus-ring']) {
      for (const bg of ['--bg-app', '--bg-surface', '--bg-raised']) {
        const r = ratio(t[fg], t[bg]);
        assert.ok(r >= 3, `${name} ${fg} on ${bg}: ${r.toFixed(2)}`);
      }
    }
  });
}

test('channel colours, markers and labels are readable on the waveform', () => {
  for (const c of ['--channel-1', '--channel-2', '--channel-3', '--channel-4', '--wave-marker', '--wave-agent', '--wave-problem', '--wave-axis-text', '--wave-cursor']) {
    const r = ratio(light[c], light['--wave-bg']);
    assert.ok(r >= 4.5, `${c} on --wave-bg: ${r.toFixed(2)}`);
  }
});
