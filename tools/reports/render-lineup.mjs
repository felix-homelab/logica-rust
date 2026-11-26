// Renders the line-up measurements (crates/engine/examples/lineup_report.rs) as one self-contained
// HTML report: SVG charts with hover, a table view for every chart, light and dark.
//
//   node tools/reports/render-lineup.mjs <lineup.json> <report.html>
import { readFileSync, writeFileSync } from 'node:fs';

const [, , input, output] = process.argv;
if (!input || !output) {
  console.error('Usage: node tools/reports/render-lineup.mjs <lineup.json> <report.html>');
  process.exit(2);
}
const data = JSON.parse(readFileSync(input, 'utf8'));
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const ms = (us) => us / 1000;
const fmt = (x, d = 2) => (Object.is(Math.round(x * 10 ** d) / 10 ** d, -0) ? 0 : Math.round(x * 10 ** d) / 10 ** d).toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d });
const signed = (x, d = 2) => (x > 0 ? '+' : x < 0 ? '−' : '') + fmt(Math.abs(x), d);

// ---- the numbers the text quotes -------------------------------------------------------------
const drift = data.drift;
const ppms = [...new Set(drift.map((d) => d.ppm))];
const run = (ppm, align) => drift.find((d) => d.ppm === ppm && d.align === align);
const worst = (rs) => Math.max(...rs.flatMap((r) => r.points.map((p) => Math.abs(p[1]))));
const synced = drift.filter((d) => d.align);
const unsynced = drift.filter((d) => !d.align);
const periodsSynced = synced.reduce((n, r) => n + r.points.length, 0);
const starts = data.starts;
const startsSync = starts.filter((s) => s.align);
const startsFree = starts.filter((s) => !s.align);
const slips = startsSync.filter((s) => Math.abs(s.points[0]?.[1] ?? 0) > 0.3);
const periodUs = 1e6 / data.sharedClockHz;
const gapMs = (s) => ms(s.trueOffsetUs);
const free = startsFree.map((s) => ({ gap: gapMs(s), err: ms(s.points[0][1]), asked: s.delayMs }));
const sync = startsSync.map((s) => ({ gap: gapMs(s), err: ms(s.points[0][1]), asked: s.delayMs }));

// ---- charts ----------------------------------------------------------------------------------
const W = 360;
const H = 210;
const M = { l: 52, r: 96, t: 14, b: 34 };
const pw = W - M.l - M.r;
const ph = H - M.t - M.b;

function ticks(lo, hi, step) {
  const out = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) out.push(Math.round(v * 1e6) / 1e6);
  return out;
}

function axes({ x0, x1, y0, y1, xs, ys, xLabel, yLabel, xFmt = (v) => v, yFmt = (v) => v }) {
  const X = (v) => M.l + ((v - x0) / (x1 - x0)) * pw;
  const Y = (v) => M.t + (1 - (v - y0) / (y1 - y0)) * ph;
  let g = '';
  for (const v of ys) g += `<line class="grid" x1="${M.l}" x2="${M.l + pw}" y1="${Y(v)}" y2="${Y(v)}"/><text class="tick" x="${M.l - 6}" y="${Y(v) + 4}" text-anchor="end">${yFmt(v)}</text>`;
  for (const v of xs) g += `<text class="tick" x="${X(v)}" y="${M.t + ph + 16}" text-anchor="middle">${xFmt(v)}</text>`;
  g += `<line class="axis" x1="${M.l}" x2="${M.l + pw}" y1="${M.t + ph}" y2="${M.t + ph}"/>`;
  g += `<text class="axis-title" x="${M.l + pw / 2}" y="${H - 4}" text-anchor="middle">${esc(xLabel)}</text>`;
  g += `<text class="axis-title" transform="translate(12 ${M.t + ph / 2}) rotate(-90)" text-anchor="middle">${esc(yLabel)}</text>`;
  return { g, X, Y };
}

// small multiple: where B's edge lands relative to A's, over the capture, one panel per drift
const Y0 = -3;
const Y1 = 3;
function driftPanel(ppm, k) {
  const free = run(ppm, false);
  const sync = run(ppm, true);
  const { g, X, Y } = axes({
    x0: 0,
    x1: data.seconds,
    y0: Y0,
    y1: Y1,
    xs: ticks(0, data.seconds, 2),
    ys: ticks(Y0, Y1, 1),
    xLabel: 'Time into the capture (s)',
    yLabel: 'B minus A (ms)',
    yFmt: (v) => (v > 0 ? `+${v}` : v < 0 ? `−${-v}` : '0'),
  });
  const path = (pts) =>
    pts.map((p, i) => `${i ? 'L' : 'M'}${X(p[0]).toFixed(1)},${Y(Math.max(Y0, Math.min(Y1, ms(p[1])))).toFixed(1)}`).join('');
  const endF = free.points.at(-1);
  const endS = sync.points.at(-1);
  let lf = Y(ms(endF[1]));
  let ls = Y(ms(endS[1]));
  // end labels kept apart without leaving their lines: a short leader when they would touch
  if (Math.abs(lf - ls) < 14) lf = ls + (lf >= ls ? 14 : -14);
  const ex = M.l + pw;
  return `<figure class="panel">
  <figcaption><strong>B's clock ${ppm} ppm fast</strong></figcaption>
  <svg viewBox="0 0 ${W} ${H}" role="img" aria-label="B's clock ${ppm} ppm fast: at their starts, B is ${signed(ms(free.points[0][1]))} ms at 0 s and ${signed(ms(endF[1]))} ms at ${data.seconds} s; by the shared clock, 0 ms throughout." data-panel="${k}" tabindex="0">
    ${g}
    <path class="line s2" d="${path(free.points)}"/>
    <path class="line s1" d="${path(sync.points)}"/>
    <circle class="end s2" cx="${ex}" cy="${Y(ms(endF[1]))}" r="4"/>
    <circle class="end s1" cx="${ex}" cy="${Y(ms(endS[1]))}" r="4"/>
    <line class="leader" x1="${ex + 6}" x2="${ex + 12}" y1="${Y(ms(endF[1]))}" y2="${lf}"/>
    <text class="label" x="${ex + 14}" y="${lf + 4}">At starts</text>
    <text class="label" x="${ex + 14}" y="${ls + 4}">Shared clock</text>
    <line class="cross" x1="0" x2="0" y1="${M.t}" y2="${M.t + ph}" visibility="hidden"/>
    <rect class="hit" x="${M.l}" y="${M.t}" width="${pw}" height="${ph}"/>
  </svg>
</figure>`;
}

// scatter: the error at the start of every capture against the start gap that happened
function scatter({ id, points, cls, x0 = 0, x1, y0, y1, xs, ys, yLabel, note }) {
  const { g, X, Y } = axes({
    x0,
    x1,
    y0,
    y1,
    xs,
    ys,
    xLabel: 'How much later B started (ms, measured)',
    yLabel,
    yFmt: (v) => (v > 0 ? `+${v}` : v < 0 ? `−${-v}` : '0'),
  });
  const dots = points
    .map(
      (p, i) =>
        `<circle class="dot ${cls}" cx="${X(p.gap).toFixed(1)}" cy="${Y(p.err).toFixed(1)}" r="4" tabindex="0" data-tip="Started ${fmt(p.gap)} ms later (asked ${fmt(p.asked, 1)} ms); error at the start ${signed(p.err)} ms" aria-label="Capture ${i + 1}: started ${fmt(p.gap)} ms later; error at the start ${signed(p.err)} ms"/>`,
    )
    .join('');
  return `<svg viewBox="0 0 ${W} ${H}" role="img" aria-labelledby="${id}-t" class="scatter">${g}${dots}${note ? note(X, Y) : ''}</svg>`;
}

const panels = ppms.map(driftPanel).join('\n');
const freeMaxGap = Math.max(...free.map((p) => p.gap));
const freeMinGap = Math.min(...free.map((p) => p.gap));
const fx0 = Math.min(0, Math.floor(freeMinGap * 2) / 2);
const scatterFree = scatter({
  id: 'sf',
  points: free,
  cls: 's2',
  x0: fx0,
  x1: Math.ceil(freeMaxGap + 0.5),
  y0: -Math.ceil(freeMaxGap + 0.5),
  y1: 1,
  xs: ticks(0, Math.ceil(freeMaxGap + 0.5), 1),
  ys: ticks(-Math.ceil(freeMaxGap + 0.5), 1, 1),
  yLabel: 'B minus A at the start (ms)',
});
const syncMaxGap = Math.max(...sync.map((p) => p.gap));
const syncMinErr = Math.min(...sync.map((p) => p.err));
const sx1 = Math.ceil((syncMaxGap + 2) / 10) * 10;
const sy0 = Math.floor((syncMinErr - 5) / 10) * 10;
const scatterSync = scatter({
  id: 'ss',
  points: sync,
  cls: 's1',
  x1: sx1,
  y0: sy0,
  y1: 5,
  xs: ticks(0, sx1, 10),
  ys: ticks(sy0, 0, 10),
  yLabel: 'B minus A at the start (ms)',
  note: (X, Y) => {
    const exact = sync.filter((p) => Math.abs(p.err) < 0.001);
    const right = Math.max(...exact.map((p) => p.gap));
    const head = exact.length ? `<text class="label" x="${X(right) + 10}" y="${Y(0) + 4}">${exact.length} captures, all exactly 0</text>` : '';
    return head + slips
      .map((s) => {
        const p = { gap: gapMs(s), err: ms(s.points[0][1]) };
        const n = Math.round(-p.err / (periodUs / 1000));
        return `<text class="label" x="${X(p.gap) + 4}" y="${Y(p.err) - 10}" text-anchor="end">${n} periods off: B started ${fmt(p.gap, 1)} ms late</text>`;
      })
      .join('');
  },
});

// ---- tables ----------------------------------------------------------------------------------
const driftRows = ppms
  .map((ppm) => {
    const f = run(ppm, false);
    const s = run(ppm, true);
    return `<tr><td>${ppm}</td><td>${fmt(s.measuredPpm ?? NaN, 1)}</td><td>${fmt(ms(f.trueOffsetUs))}</td><td>${signed(ms(f.points[0][1]))}</td><td>${signed(ms(f.points.at(-1)[1]))}</td><td>${fmt(ms(s.trueOffsetUs))}</td><td>${signed((s.foundOffsetUs - s.trueOffsetUs) / 1000, 4)}</td><td>${fmt(worst([s]) * 1000, 0)}</td><td>${s.points.length}</td></tr>`;
  })
  .join('');
const startRows = starts
  .filter((s) => !s.align)
  .map((f, i) => {
    const s = startsSync[i];
    return `<tr><td>${fmt(f.delayMs, 1)}</td><td>${fmt(gapMs(f))}</td><td>${signed(ms(f.points[0][1]))}</td><td>${fmt(gapMs(s))}</td><td>${signed(ms(s.points[0][1]))}</td></tr>`;
  })
  .join('');

// ---- the page --------------------------------------------------------------------------------
const worstFree = worst(unsynced);
const worstSync = worst(synced);
const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lining up two devices in one capture: with and without a shared clock</title>
<style>
.viz-root {
  color-scheme: light;
  --surface-1: #ffffff; --page: #f9f9f9; --text-primary: #191919; --text-secondary: #666666;
  --grid: #e7e7e7; --axis: #c2c2c2; --border: rgba(11,11,11,0.10);
  --series-1: #257de0; --series-2: #f47230;
}
@media (prefers-color-scheme: dark) {
  :root:where(:not([data-theme="light"])) .viz-root {
    color-scheme: dark;
    --surface-1: #191919; --page: #0b0b0b; --text-primary: #efefef; --text-secondary: #9c9c9c;
    --grid: #303030; --axis: #474747; --border: rgba(255,255,255,0.10);
    --series-1: #338ced; --series-2: #e76624;
  }
}
:root[data-theme="dark"] .viz-root {
  color-scheme: dark;
  --surface-1: #191919; --page: #0b0b0b; --text-primary: #efefef; --text-secondary: #9c9c9c;
  --grid: #303030; --axis: #474747; --border: rgba(255,255,255,0.10);
  --series-1: #338ced; --series-2: #e76624;
}
* { box-sizing: border-box; }
body { margin: 0; }
.viz-root { background: var(--page); color: var(--text-primary); font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; min-height: 100vh; }
main { max-width: 960px; margin: 0 auto; padding: 32px 24px 64px; }
h1 { font-size: 26px; margin: 0 0 4px; }
h2 { font-size: 19px; margin: 40px 0 8px; }
p, li { max-width: 72ch; }
.muted { color: var(--text-secondary); }
.card { background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
.tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; margin: 20px 0; }
.tile .label-t { color: var(--text-secondary); font-size: 13px; }
.tile .value { font-size: 28px; font-weight: 600; margin: 4px 0 2px; }
.tile .sub { color: var(--text-secondary); font-size: 13px; }
.legend { display: flex; gap: 18px; flex-wrap: wrap; color: var(--text-secondary); font-size: 13px; margin: 8px 0 4px; }
.key { display: inline-block; width: 16px; height: 2px; vertical-align: middle; margin-right: 6px; border-radius: 1px; }
.key.s1 { background: var(--series-1); } .key.s2 { background: var(--series-2); }
.grid2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); gap: 12px; }
figure { margin: 0; }
figcaption { font-size: 14px; margin-bottom: 4px; }
svg { width: 100%; height: auto; display: block; overflow: visible; }
svg:focus-visible, circle:focus-visible { outline: 2px solid var(--series-1); outline-offset: 2px; }
.grid { stroke: var(--grid); stroke-width: 1; }
.axis { stroke: var(--axis); stroke-width: 1; }
.tick { fill: var(--text-secondary); font-size: 11px; font-variant-numeric: tabular-nums; }
.axis-title { fill: var(--text-secondary); font-size: 11px; }
.label { fill: var(--text-primary); font-size: 12px; }
.line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.line.s1 { stroke: var(--series-1); } .line.s2 { stroke: var(--series-2); }
.end, .dot { stroke: var(--surface-1); stroke-width: 2; }
.end.s1, .dot.s1 { fill: var(--series-1); } .end.s2, .dot.s2 { fill: var(--series-2); }
.leader { stroke: var(--axis); stroke-width: 1; }
.cross { stroke: var(--axis); stroke-width: 1; }
.hit { fill: transparent; }
.tip { position: fixed; pointer-events: none; background: var(--surface-1); color: var(--text-primary); border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; font-size: 12px; box-shadow: 0 2px 8px rgba(0,0,0,.12); display: none; z-index: 10; }
.tip .row { display: flex; align-items: center; gap: 6px; }
details { margin-top: 10px; }
summary { cursor: pointer; color: var(--text-secondary); font-size: 13px; }
table { border-collapse: collapse; font-size: 13px; margin-top: 8px; font-variant-numeric: tabular-nums; }
th, td { text-align: right; padding: 4px 10px; border-bottom: 1px solid var(--grid); }
th:first-child, td:first-child { text-align: left; }
th { color: var(--text-secondary); font-weight: 500; }
.note { border-left: 3px solid var(--axis); padding: 4px 12px; color: var(--text-secondary); }
code { font-size: 13px; }
.theme { float: right; font-size: 13px; }
</style>
</head>
<body>
<div class="viz-root">
<main>
<button class="theme" type="button" onclick="const r=document.documentElement; r.dataset.theme = (r.dataset.theme || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')) === 'dark' ? 'light' : 'dark'">Light or dark</button>
<h1>Lining up two devices in one capture</h1>
<p class="muted">With a shared clock, and without. Measured on ${esc(new Date().toISOString().slice(0, 10))} with Logica's capture merger and two virtual analysers. Written with AI assistance (Claude Code); review it before relying on it.</p>

<div class="tiles">
  <div class="card tile"><div class="label-t">Worst error, lined up by the shared clock</div><div class="value">${fmt(worstSync * 1000, 0)} ns</div><div class="sub">${periodsSynced.toLocaleString('en-US')} clock periods in ${synced.length} captures of ${data.seconds} s, 0 to 500 ppm drift</div></div>
  <div class="card tile"><div class="label-t">Worst error, lined up at their starts</div><div class="value">${fmt(ms(worstFree))} ms</div><div class="sub">the same captures: start gap plus drift</div></div>
  <div class="card tile"><div class="label-t">Captures lined up by the wrong clock period</div><div class="value">${slips.length} of ${startsSync.length}</div><div class="sub">B started more than half a period (${fmt(periodUs / 2000, 0)} ms) late</div></div>
</div>

<h2>What was measured</h2>
<p>Two virtual analysers (<code>logica-vla</code>) on loopback ports, captured together into one capture by the engine's merger (<code>crates/engine/src/group.rs</code>), each setting twice: lined up by a clock both devices see, and lined up at their starts. The shared clock is the analysers' <code>shared</code> signal: ${data.sharedClockHz} Hz, following the computer's clock, so both see the same edges, each at its own samples. Device A's clock is the capture's. Device B starts later (it waits before starting, as a slower device would) and its sample clock runs fast by a set number of ppm. Both sample at ${data.rateHz / 1e6} MHz, so one sample is ${fmt(1e9 / data.rateHz, 0)} ns.</p>
<p>The truth comes from the analysers: each says where its sample 0 is on the computer's clock, so every rising edge of the shared clock belongs to a known period, on either device. The error of a period is where B's edge of that period landed in the capture, minus where A's did. Zero means the two devices agree on when it happened.</p>

<h2>Over a ${data.seconds}-second capture</h2>
<p>B started about 2 ms after A. Lined up at their starts, B's events land early by that gap from the first sample, and its fast clock moves them by its drift as the capture runs: 500 ppm is half a millisecond a second. Lined up by the shared clock, every one of the ${periodsSynced.toLocaleString('en-US')} periods lands on the same sample on both devices, whatever the drift.</p>
<div class="legend"><span><span class="key s2"></span>At their starts</span><span><span class="key s1"></span>By the shared clock</span></div>
<div class="grid2">
${panels}
</div>
<details><summary>Table view</summary>
<table>
<thead><tr><th>B's drift (ppm, set)</th><th>Drift found (ppm)</th><th>Start gap, at starts (ms)</th><th>Error at 0 s, at starts (ms)</th><th>Error at ${data.seconds} s, at starts (ms)</th><th>Start gap, shared clock (ms)</th><th>Offset found minus true (ms)</th><th>Worst error, shared clock (ns)</th><th>Periods compared</th></tr></thead>
<tbody>${driftRows}</tbody>
</table>
<p class="muted">Each row is two captures, so their start gaps differ a little. The merger finds the drift from the shared clock and says so in a note on the capture; it matched the set drift in every capture.</p>
</details>

<h2>At the start of the capture</h2>
<p>${startsFree.length} short captures each way, B asked to start 0 to ${fmt(Math.max(...starts.map((s) => s.delayMs)), 1)} ms after A, its clock 50 ppm fast. Lined up at their starts, the error at the start is the start gap, which no one chooses: here B started anywhere from ${freeMinGap < 0 ? `${fmt(-freeMinGap)} ms before A` : `${fmt(freeMinGap)} ms after A`} to ${fmt(freeMaxGap)} ms after it. Lined up by the shared clock, the error is zero, except where B started more than half a clock period (${fmt(periodUs / 2000, 0)} ms) late: a steady clock looks the same one period later, and the merger takes the smallest shift that matches, so it lines B up by the wrong period.</p>
<div class="grid2">
<figure class="card"><figcaption id="sf-t"><strong>At their starts</strong>: error at the start against the start gap</figcaption>${scatterFree}</figure>
<figure class="card"><figcaption id="ss-t"><strong>By the shared clock</strong>: error at the start against the start gap</figcaption>${scatterSync}</figure>
</div>
<details><summary>Table view</summary>
<table>
<thead><tr><th>Asked delay (ms)</th><th>Start gap, at starts (ms)</th><th>Error, at starts (ms)</th><th>Start gap, shared clock (ms)</th><th>Error, shared clock (ms)</th></tr></thead>
<tbody>${startRows}</tbody>
</table>
</details>

<h2>What this means</h2>
<ul>
<li><strong>Lined up by a shared signal, two devices agree to the sample</strong>, from the first edge to the last, at any drift tried (0 to 500 ppm; crystals are typically within 50). The drift itself is measured on the way and reported.</li>
<li><strong>Lined up at their starts, they disagree by the start gap plus the drift.</strong> Here that was up to ${fmt(ms(worstFree))} ms. At ${data.rateHz / 1e6} MHz a millisecond is ${(data.rateHz / 1000).toLocaleString('en-US')} samples, so a bus transfer on one device cannot be lined up with its cause on the other, and the gap differs from capture to capture.</li>
<li><strong>A steady shared clock is ambiguous by whole periods.</strong> In ${slips.length} of ${startsSync.length} captures B started more than ${fmt(periodUs / 2000, 0)} ms late (asked for under 4 ms) and was lined up ${slips.map((s) => Math.round(-ms(s.points[0][1]) / (periodUs / 1000))).join(' and ')} periods off: exact within the period, and wrong. An earlier run of the same measurement had 2 of 12. The large gaps came in steps near 30 to 40 ms, which looks like a network acknowledgement delay on the analysers' connection; that is a guess, not measured.</li>
</ul>
<p class="note">What would fix the ambiguity, not built yet: use the measured moment each device began (the merger sees it to about a millisecond) to choose the period instead of the smallest shift; start devices' connections without send delays (<code>TCP_NODELAY</code> on the virtual analyser's connection); or line up by an irregular shared signal (a data line), which has no repeating period. Until then, a slower shared clock tolerates more: at 10 Hz a start gap up to 50 ms is safe.</p>

<h2>Limits of this measurement</h2>
<ul>
<li>Virtual analysers, not hardware: their edges are exact to the sample, and their drift is constant. Real devices add edge jitter and drift that wanders with temperature; the straight-line fit follows the average.</li>
<li>Both analysers ran on one computer and read one clock, so "the truth" is exact here. With hardware the truth would need a reference instrument.</li>
<li>Rising edges of the shared clock only; the result between them follows from the same straight line and was not measured edge by edge.</li>
</ul>

<h2>Reproduce</h2>
<p><code>cargo run --release -p logica-engine --example lineup_report -- lineup.json</code> then <code>node tools/reports/render-lineup.mjs lineup.json report.html</code>. The start gaps are whatever the computer gives, so the slip count varies from run to run.</p>
</main>
<div class="tip" role="tooltip"></div>
</div>
<script>
const DATA = ${JSON.stringify({
  ppms,
  seconds: data.seconds,
  panels: ppms.map((p) => ({ free: run(p, false).points.map((q) => [q[0], q[1] / 1000]), sync: run(p, true).points.map((q) => [q[0], q[1] / 1000]) })),
  M,
  pw,
  W,
})};
const tip = document.querySelector('.tip');
const show = (html, x, y) => { tip.innerHTML = html; tip.style.display = 'block'; tip.style.left = Math.min(x + 14, innerWidth - tip.offsetWidth - 8) + 'px'; tip.style.top = (y + 14) + 'px'; };
const hide = () => { tip.style.display = 'none'; };
const sgn = (v) => (v > 0 ? '+' : v < 0 ? '−' : '') + Math.abs(v).toFixed(3) + ' ms';
const row = (cls, name, v) => '<div class="row"><span class="key ' + cls + '"></span>' + name + ': ' + sgn(v) + '</div>';
document.querySelectorAll('svg[data-panel]').forEach((svg) => {
  const k = +svg.dataset.panel; const d = DATA.panels[k]; const cross = svg.querySelector('.cross');
  const at = (t) => { const i = Math.max(0, Math.min(d.free.length - 1, Math.round(t / DATA.seconds * (d.free.length - 1)))); return i; };
  const put = (i, cx, cy) => {
    const f = d.free[i], s = d.sync[Math.min(i, d.sync.length - 1)];
    const x = DATA.M.l + f[0] / DATA.seconds * DATA.pw;
    cross.setAttribute('x1', x); cross.setAttribute('x2', x); cross.setAttribute('visibility', 'visible');
    show('<strong>' + f[0].toFixed(2) + ' s</strong>' + row('s2', 'At their starts', f[1]) + row('s1', 'By the shared clock', s[1]), cx, cy);
  };
  svg.addEventListener('mousemove', (e) => {
    const r = svg.getBoundingClientRect(); const px = (e.clientX - r.left) / r.width * DATA.W;
    put(at((px - DATA.M.l) / DATA.pw * DATA.seconds), e.clientX, e.clientY);
  });
  svg.addEventListener('mouseleave', () => { hide(); cross.setAttribute('visibility', 'hidden'); });
  let i = 0;
  svg.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault(); i = Math.max(0, Math.min(d.free.length - 1, i + (e.key === 'ArrowRight' ? 25 : -25)));
    const r = svg.getBoundingClientRect(); put(i, r.left + r.width / 2, r.top);
  });
  svg.addEventListener('blur', () => { hide(); cross.setAttribute('visibility', 'hidden'); });
});
document.querySelectorAll('circle[data-tip]').forEach((c) => {
  const on = (x, y) => show(c.dataset.tip, x, y);
  c.addEventListener('mouseenter', (e) => on(e.clientX, e.clientY));
  c.addEventListener('mouseleave', hide);
  c.addEventListener('focus', () => { const r = c.getBoundingClientRect(); on(r.right, r.top); });
  c.addEventListener('blur', hide);
});
</script>
</body>
</html>
`;
writeFileSync(output, html);
console.log(`wrote ${output}`);
