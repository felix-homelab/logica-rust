// The waveform (doc 08): LOD rendering from waveform.get, zoom and pan, cursors that snap to edges,
// a range selection, markers (agents' with a badge), and each decoder's channels grouped under its
// name with its rows below the channel its data is on: the frames, then the data as text in the
// decoder's format (lanes.ts). The canvas
// draws at once from the data it has, mapped to the current view, and asks the engine for the
// new range in the background, so panning never waits on it.
import type { KeyboardEvent, PointerEvent, WheelEvent } from 'react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Output } from '@logica/api';
import { CommandError } from '@logica/api';
import { Button } from '@logica/ui';
import { bridge, client } from '../bridge';
import { at, tick, tickStep, time } from '../format';
import { useEventCount, type CaptureInfo } from '../store';
import { analogNeeds, waveformNeeds } from '../relevance';
import { joinRun, layoutRows, runs, type Row } from '../lanes';
import { formatValue, tickCount, valueTickStep, valueLines, valueRange, valueTicks, valueY } from '../analogplot';
import { clampRange, pan, sampleToX, snap, xToSample, zoomAt, type Range } from '../viewmath';

export interface ViewState extends Range {
  cursorA: number | null;
  cursorB: number | null;
  selection: [number, number] | null;
}

type Wave = Output<'waveform.get'>;
type Values = Output<'analog.values'>;

const AXIS = 22;
const MARKERS = 16;
const LABEL_W = 88;
const TRACE_COLOURS = ['--channel-1', '--channel-2', '--channel-3', '--channel-4'];

/** The longest start of `text` that fits `room` pixels, with "…" when cut; "" when nothing fits. */
function fit(g: CanvasRenderingContext2D, text: string, room: number): string {
  if (g.measureText(text).width <= room) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (g.measureText(`${text.slice(0, mid)}…`).width <= room) lo = mid;
    else hi = mid - 1;
  }
  return lo === 0 ? '' : `${text.slice(0, lo)}…`;
}

/** Reads the token colours once per theme. */
function palette(): Record<string, string> {
  const s = getComputedStyle(document.documentElement);
  const v = (n: string) => s.getPropertyValue(n).trim();
  return {
    bg: v('--wave-bg'),
    grid: v('--wave-grid'),
    axis: v('--wave-axis-text'),
    cursor: v('--wave-cursor'),
    selection: v('--wave-selection'),
    marker: v('--wave-marker'),
    agent: v('--wave-agent'),
    problem: v('--wave-problem'),
    c1: v('--channel-1'),
    c2: v('--channel-2'),
    c3: v('--channel-3'),
    c4: v('--channel-4'),
    font: v('--font-ui'),
    mono: v('--font-mono'),
  };
}

const FRAME_FILL: Record<string, string> = {
  data: '#2c5c9e',
  field: '#2c5c9e',
  sample: '#2c5c9e',
  address: '#6a3fa0',
  command: '#6a3fa0',
  start: '#2e7d4f',
  'repeated-start': '#2e7d4f',
  reset: '#2e7d4f',
  presence: '#2e7d4f',
  stop: '#8a5a1e',
  break: '#8a5a1e',
  ack: '#3a6e6e',
  nack: '#8a3434',
  cycle: '#444a56',
  info: '#444a56',
};

export function Waveform({
  capture,
  view,
  onView,
  expert,
  follow = false,
}: {
  capture: CaptureInfo;
  view: ViewState;
  onView: (v: ViewState) => void;
  expert: boolean;
  /** A live view following the newest data (Live.tsx). */
  follow?: boolean;
}) {
  const changes = useEventCount((e) => waveformNeeds(e, capture.id));
  const analogChanges = useEventCount((e) => analogNeeds(e, capture.id));
  const [values, setValues] = useState<Map<string, Values>>(new Map());
  const [failed, setFailed] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(800);
  const [data, setData] = useState<Wave | null>(null);
  const [description, setDescription] = useState('');
  const drag = useRef<{ x: number; start: ViewState; moved: boolean; select: boolean } | null>(null);
  const colours = useRef<Record<string, string>>({});
  const count = capture.sampleCount;
  const rate = capture.sampleRateHz;
  const plotW = Math.max(1, width - LABEL_W);

  // a row being resized follows the pointer; the engine hears of it once, on release
  const [resized, setResized] = useState<{ id: string; h: number } | null>(null);
  const resize = useRef<{ id: string; y: number; h: number } | null>(null);
  const rows = useMemo<Row[]>(() => {
    const analog = capture.analog.map((a) => (a.id === resized?.id ? { ...a, config: { ...a.config, height: resized.h } } : a));
    return layoutRows(capture.channels, capture.decoders, AXIS + MARKERS, analog);
  }, [capture.channels, capture.decoders, capture.analog, resized]);
  const height = (rows.at(-1)?.y ?? AXIS + MARKERS) + (rows.at(-1)?.h ?? 0) + 8;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.floor(el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ask the engine for the visible range; a newer answer replaces an older one, never the reverse.
  // Answers to requests overtaken by newer ones are still shown: a live view moves before each
  // answer arrives, and waiting for "the latest" would leave it empty.
  const seq = useRef(0);
  const shownSeq = useRef(0);
  const captureId = useRef(capture.id);
  captureId.current = capture.id;
  useEffect(() => {
    const n = ++seq.current;
    const id = capture.id;
    const t = window.setTimeout(() => {
      client
        .run('waveform.get', { capture: id, from: view.from, to: view.to, buckets: Math.min(8192, Math.max(1, plotW)) })
        .then((w) => {
          if (n < shownSeq.current || id !== captureId.current) return;
          shownSeq.current = n;
          setData(w);
          setFailed(null);
        })
        .catch((e: unknown) => {
          if (n !== seq.current) return;
          // say so rather than keep showing data that may be out of date
          const message = e instanceof CommandError ? e.message : String(e);
          setFailed(message);
          bridge().log('warn', `waveform.get failed: ${message}`);
        });
    }, 16);
    return () => window.clearTimeout(t);
  }, [capture.id, view.from, view.to, plotW, changes, retry]);

  // the values of the virtual analog channels in view, in their units (ADR 0010); a scaling or
  // style change arrives as a new session, so the channel list is part of what is asked for
  const analogKey = capture.analog.map((a) => `${a.id}:${JSON.stringify(a.config)}`).join('|');
  useEffect(() => {
    const id = capture.id;
    const ids = capture.analog.map((a) => a.id);
    if (!ids.length) {
      setValues(new Map());
      return;
    }
    let stale = false;
    const t = window.setTimeout(() => {
      const buckets = Math.min(8192, Math.max(1, plotW));
      Promise.all(ids.map((analog) => client.run('analog.values', { capture: id, analog, from: view.from, to: view.to, buckets }).then((v) => [analog, v] as const)))
        .then((all) => {
          if (!stale) setValues(new Map(all));
        })
        .catch((e: unknown) => bridge().log('warn', `analog.values failed: ${e instanceof CommandError ? e.message : String(e)}`));
    }, 16);
    return () => {
      stale = true;
      window.clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- analogKey stands for capture.analog
  }, [capture.id, view.from, view.to, plotW, analogChanges, analogKey]);

  const draw = useCallback(() => {
    // a live view draws the range its data is for: following moves the view on before each
    // answer arrives, and the newest data would otherwise always be just off the edge
    const dv: ViewState = follow && data ? { ...view, from: data.from, to: data.to } : view;
    const c = canvas.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== Math.round(width * dpr) || c.height !== Math.round(height * dpr)) {
      c.width = Math.round(width * dpr);
      c.height = Math.round(height * dpr);
    }
    const g = c.getContext('2d');
    if (!g) return;
    if (!colours.current.bg) colours.current = palette();
    const k = colours.current;
    const col = (v: string) => (v.startsWith('--') ? (k[`c${v.slice(-1)}`] ?? '#3ddc84') : v);
    const traceColour = (r: Row) => col(r.colour ?? TRACE_COLOURS[(r.colourIndex ?? 0) % 4]!);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = k.bg!;
    g.fillRect(0, 0, width, height);
    g.font = `11px ${k.font}`;
    g.textBaseline = 'middle';
    const x = (s: number) => LABEL_W + sampleToX(s, dv, plotW);

    // grid and time axis
    const span = (dv.to - dv.from) / rate;
    const step = tickStep(span, Math.max(2, Math.floor(plotW / 110)));
    const t0 = dv.from / rate;
    for (let t = Math.ceil(t0 / step) * step; t <= dv.to / rate; t += step) {
      const px = Math.round(x(t * rate)) + 0.5;
      g.fillStyle = k.grid!;
      g.fillRect(px, AXIS - 4, 1, height);
      g.fillStyle = k.axis!;
      g.fillText(tick(t, step), px + 3, 10);
    }
    g.fillStyle = k.grid!;
    g.fillRect(0, AXIS - 1, width, 1);

    // selection, under everything else
    if (dv.selection) {
      const [a, b] = dv.selection;
      g.fillStyle = k.selection!;
      g.fillRect(x(Math.min(a, b)), AXIS, Math.abs(x(b) - x(a)), height);
    }

    // markers: lines under the traces, labels in their own row
    for (const m of capture.markers) {
      const agent = m.by.startsWith('agent:');
      const colour = agent ? k.agent! : k.marker!;
      const mx = x(m.at);
      if (m.end !== null && m.end !== undefined) {
        g.globalAlpha = 0.14;
        g.fillStyle = colour;
        g.fillRect(mx, AXIS, x(m.end) - mx, height);
        g.globalAlpha = 1;
      }
      if (mx < LABEL_W - 1 || mx > width) continue;
      g.fillStyle = colour;
      g.fillRect(Math.round(mx), AXIS, 1, height);
      const label = agent ? `◆ ${m.label}` : m.label;
      g.fillText(label, mx + 4, AXIS + MARKERS / 2);
    }

    // traces, groups and decoder rows
    const traces = new Map((data?.traces ?? []).map((t) => [t.channel, t]));
    const decs = new Map((data?.decoders ?? []).map((d) => [d.id, d]));
    const onLane = (r: Row, lane: number) => r.lane === null || r.lane === undefined || r.lane === lane;
    for (const r of rows) {
      if (r.kind === 'group') {
        // a hairline above each group
        g.fillStyle = k.grid!;
        g.fillRect(0, r.y, width, 1);
        continue;
      }
      if (r.kind === 'analog') {
        const a = capture.analog.find((v) => v.id === r.analog);
        const v = r.analog ? values.get(r.analog) : undefined;
        g.fillStyle = k.grid!;
        g.fillRect(LABEL_W, r.y, width - LABEL_W, 1);
        if (!a || !v) continue;
        const [lo, hi] = valueRange(a.min, a.max);
        const top = r.y + 8;
        const h = r.h - 14;
        const vy = (val: number) => valueY(val, lo, hi, top, h);
        for (const t of valueTicks(lo, hi, tickCount(h))) g.fillRect(LABEL_W, Math.round(vy(t)) + 0.5, width - LABEL_W, 1);
        g.strokeStyle = traceColour(r);
        g.fillStyle = traceColour(r);
        g.lineWidth = 1.5;
        if (v.points) {
          for (const line of valueLines(v.points, v.held, v.from, v.to, a.config.style ?? 'step')) {
            g.beginPath();
            line.forEach(([s, val], i) => (i ? g.lineTo(x(s), vy(val)) : g.moveTo(x(s), vy(val))));
            g.stroke();
          }
          // the points themselves when they are far enough apart to tell
          if (v.points.length && plotW / v.points.length > 10) {
            for (const p of v.points) if (p.value !== null && p.value !== undefined) g.fillRect(x(p.sample) - 1.5, vy(p.value) - 1.5, 3, 3);
          }
        } else {
          (v.buckets ?? []).forEach((b, i) => {
            const vals = [b.held, b.min, b.max].filter((n): n is number => n !== null && n !== undefined);
            if (!vals.length) return;
            const s0 = v.from + i * v.bucket;
            const x0 = x(s0);
            const x1 = Math.max(x0 + 1, x(Math.min(v.to, s0 + v.bucket)));
            const y0 = vy(Math.max(...vals));
            g.fillRect(x0, y0, x1 - x0, Math.max(1.5, vy(Math.min(...vals)) - y0));
          });
        }
        // problems: outlined and marked, never by colour alone
        g.strokeStyle = k.problem!;
        g.lineWidth = 1;
        for (const p of v.problems) {
          const x0 = Math.max(LABEL_W, x(p.start));
          const x1 = Math.max(x0 + 3, Math.min(width, x(p.end)));
          g.strokeRect(x0 + 0.5, r.y + 2.5, x1 - x0 - 1, r.h - 5);
          g.fillStyle = k.problem!;
          g.fillText('!', x0 + 3, r.y + 9);
        }
        continue;
      }
      if (r.kind === 'channel' && r.channel !== undefined) {
        const t = traces.get(r.channel);
        if (!t || !data) continue;
        const hi = r.y + 7;
        const lo = r.y + r.h - 7;
        const yl = (high: boolean) => (high ? hi : lo);
        g.strokeStyle = traceColour(r);
        g.fillStyle = traceColour(r);
        g.lineWidth = 1.5;
        g.beginPath();
        if (t.edges) {
          let level = t.startLevel === 'high';
          let px = x(data.from);
          for (const e of t.edges) {
            const ex = x(e);
            g.moveTo(px, yl(level) + 0.5);
            g.lineTo(ex, yl(level) + 0.5);
            level = !level;
            g.moveTo(Math.round(ex) + 0.5, hi);
            g.lineTo(Math.round(ex) + 0.5, lo + 1);
            px = ex;
          }
          g.moveTo(px, yl(level) + 0.5);
          g.lineTo(x(data.to), yl(level) + 0.5);
          g.stroke();
        } else if (t.counts && t.levels) {
          g.stroke();
          for (let i = 0; i < t.counts.length; i++) {
            const s0 = data.from + i * data.bucket;
            const x0 = x(s0);
            const x1 = Math.max(x0 + 1, x(Math.min(data.to, s0 + data.bucket)));
            if (x1 < LABEL_W || x0 > width) continue;
            const level = t.levels[i] === '1';
            if ((t.counts[i] ?? 0) === 0) {
              g.fillRect(x0, yl(level), x1 - x0, 1);
            } else if (x1 - x0 <= 2) {
              g.fillRect(x0, hi, x1 - x0, lo - hi + 1);
            } else {
              g.globalAlpha = 0.45;
              g.fillRect(x0, hi, x1 - x0, lo - hi + 1);
              g.globalAlpha = 1;
              g.fillRect(x0, hi, x1 - x0, 1);
              g.fillRect(x0, lo, x1 - x0, 1);
            }
          }
        }
        continue;
      }
      const d = r.decoder ? decs.get(r.decoder) : undefined;
      if (!d) continue;
      const y0 = r.y + 3;
      const h = r.h - 6;
      if (r.kind === 'data') {
        for (const f of d.frames ?? []) {
          if (!onLane(r, f.lane)) continue;
          const x0 = Math.max(LABEL_W, x(f.start));
          const x1 = Math.max(x0 + 1, Math.min(width, x(f.end)));
          if (x1 < LABEL_W || x0 > width) continue;
          g.fillStyle = FRAME_FILL[f.kind] ?? '#444a56';
          g.fillRect(x0, y0, x1 - x0, h);
          if (f.problem) {
            // not colour alone: an outline and a mark
            g.strokeStyle = k.problem!;
            g.strokeRect(x0 + 0.5, y0 + 0.5, x1 - x0 - 1, h - 1);
            g.fillStyle = k.problem!;
            g.fillText('!', x0 + 2, y0 + h / 2);
          }
          const w = g.measureText(f.label).width;
          if (w + 8 <= x1 - x0) {
            g.fillStyle = '#e6e8ec';
            g.fillText(f.label, x0 + (x1 - x0 - w) / 2, y0 + h / 2);
          }
        }
        for (const [s, e, problem, lane] of d.spans ?? []) {
          if (!onLane(r, lane)) continue;
          const x0 = Math.max(LABEL_W, x(s));
          const x1 = Math.max(x0 + 1, x(e));
          g.fillStyle = problem ? k.problem! : '#2c5c9e';
          g.fillRect(x0, y0, x1 - x0, h);
        }
      } else if (r.kind === 'text') {
        // each value under its frame when there is room, else the packet's values as one string
        g.font = `11px ${k.mono}`;
        // the font is monospaced: one measurement per draw instead of one per value
        const cw = g.measureText('0').width;
        const all = runs((d.frames ?? []).filter((f) => onLane(r, f.lane)));
        all.forEach((run, i) => {
          const rx0 = x(run[0]!.start);
          const rx1 = x(run.at(-1)!.end);
          // a string may run on into the free space up to the next packet on this channel
          const next = all[i + 1];
          const room = Math.min(width, next && next[0]!.lane === run[0]!.lane ? x(next[0]!.start) : width);
          if (rx1 < LABEL_W || rx0 > width) return;
          g.globalAlpha = 0.22;
          g.fillStyle = '#2c5c9e';
          g.fillRect(Math.max(LABEL_W, rx0), y0, Math.max(1, Math.min(width, rx1) - Math.max(LABEL_W, rx0)), h);
          g.globalAlpha = 1;
          g.fillStyle = k.cursor!;
          const each = run.every((f) => f.text!.length * cw + 4 <= x(f.end) - x(f.start));
          if (each) {
            for (const f of run) {
              const fx0 = x(f.start);
              const w = f.text!.length * cw;
              g.fillText(f.text!, fx0 + (x(f.end) - fx0 - w) / 2, y0 + h / 2);
            }
          } else {
            // from where the run is visible, so a long packet still reads at the left edge
            const left = Math.max(LABEL_W, rx0) + 3;
            const shown = fit(g, joinRun(run.map((f) => f.text!), r.format ?? 'ascii'), room - left - 6);
            if (shown) g.fillText(shown, left, y0 + h / 2);
          }
        });
        for (const [s, e, , lane] of d.spans ?? []) {
          if (!onLane(r, lane)) continue;
          const x0 = Math.max(LABEL_W, x(s));
          g.globalAlpha = 0.22;
          g.fillStyle = '#2c5c9e';
          g.fillRect(x0, y0, Math.max(1, x(e) - x0), h);
          g.globalAlpha = 1;
        }
        g.font = `11px ${k.font}`;
      }
    }

    // cursors on top
    for (const [s, name] of [
      [dv.cursorA, 'A'],
      [dv.cursorB, 'B'],
    ] as const) {
      if (s === null) continue;
      const cx = Math.round(x(s)) + 0.5;
      g.strokeStyle = k.cursor!;
      g.lineWidth = 1;
      g.setLineDash([4, 3]);
      g.beginPath();
      g.moveTo(cx, AXIS);
      g.lineTo(cx, height);
      g.stroke();
      g.setLineDash([]);
      g.fillStyle = k.cursor!;
      g.fillText(name, cx + 3, AXIS + MARKERS / 2);
    }
    // the label column covers what was drawn under it
    g.fillStyle = k.bg!;
    g.fillRect(0, AXIS, LABEL_W - 4, height);
    const extent = new Map<number, [number, number]>();
    for (const r of rows) {
      if (r.group >= 0) extent.set(r.group, [extent.get(r.group)?.[0] ?? r.y, r.y + r.h]);
      if (r.kind === 'group') {
        // the group's name has the whole row: nothing is drawn beside it
        g.font = `600 11px ${k.font}`;
        g.fillStyle = k.cursor!;
        g.fillText(fit(g, r.label, width - 12), 8, r.y + r.h / 2 + 1);
        g.font = `11px ${k.font}`;
        continue;
      }
      if (r.kind === 'analog') {
        // the name on top, the value axis below it, in the channel's unit
        const a = capture.analog.find((v) => v.id === r.analog);
        g.fillStyle = traceColour(r);
        g.fillText(fit(g, r.label, LABEL_W - 14), 8, r.y + 9);
        if (!a) continue;
        const [lo, hi] = valueRange(a.min, a.max);
        g.fillStyle = k.axis!;
        g.font = `10px ${k.mono}`;
        g.textAlign = 'right';
        const count = tickCount(r.h - 14);
        // as many decimals as the ticks are apart
        const apart = valueTickStep(lo, hi, count);
        for (const t of valueTicks(lo, hi, count)) {
          const ty = valueY(t, lo, hi, r.y + 8, r.h - 14);
          if (ty > r.y + 18) g.fillText(fit(g, formatValue(t, a.config.unit ?? '', apart), LABEL_W - 16), LABEL_W - 8, ty);
        }
        g.textAlign = 'left';
        g.font = `11px ${k.font}`;
        // a grip on the bottom edge, and a line along it: drag it to resize the row
        g.fillRect(LABEL_W / 2 - 12, r.y + r.h - 4, 24, 1);
        g.fillRect(LABEL_W / 2 - 12, r.y + r.h - 2, 24, 1);
        g.fillStyle = k.grid!;
        g.fillRect(0, r.y + r.h - 1, width, 1);
        continue;
      }
      g.fillStyle = r.kind === 'channel' ? traceColour(r) : k.axis!;
      g.fillText(fit(g, r.label, LABEL_W - 14), r.kind === 'channel' ? 8 : 14, r.y + r.h / 2);
    }
    // a bracket down the side of each group
    g.fillStyle = k.axis!;
    for (const [top, bottom] of extent.values()) g.fillRect(2, top + 4, 2, bottom - top - 6);
  }, [width, height, view, rows, data, capture.markers, capture.analog, values, plotW, rate, follow]);

  useEffect(() => {
    const id = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(id);
  }, [draw]);
  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const reset = () => {
      colours.current = {};
      requestAnimationFrame(draw);
    };
    mq.addEventListener('change', reset);
    return () => mq.removeEventListener('change', reset);
  }, [draw]);

  // a text description of what is shown (doc 08: the canvas has a live text description)
  useEffect(() => {
    const t = window.setTimeout(() => {
      const names = rows.filter((r) => r.kind === 'channel').map((r) => r.label);
      const decoders = capture.decoders.map((d) => `${d.config.name} (${d.frames} frames, data shown as ${d.config.dataFormat ?? 'ascii'})`);
      const marks = capture.markers.filter((m) => m.at >= view.from && m.at < view.to).length;
      const analog = capture.analog.map((a) => {
        const unit = a.config.unit ?? '';
        const span =
          a.min === null || a.min === undefined || a.max === null || a.max === undefined
            ? 'no values'
            : `${formatValue(a.min, unit, a.step)} to ${formatValue(a.max, unit, a.step)}`;
        return `${a.config.name} (${span})`;
      });
      setDescription(
        `${capture.title}: ${time(view.from / rate)} to ${time(view.to / rate)}. Channels ${names.join(', ') || 'none'}. ` +
          `${decoders.length ? `Decoders ${decoders.join(', ')}. ` : ''}` +
          `${analog.length ? `Virtual analog channels ${analog.join(', ')}. ` : ''}${marks} marker${marks === 1 ? '' : 's'} in view.` +
          (view.cursorA !== null ? ` Cursor A at ${at(view.cursorA, rate)}.` : '') +
          (view.cursorB !== null ? ` Cursor B at ${at(view.cursorB, rate)}.` : ''),
      );
    }, 400);
    return () => window.clearTimeout(t);
  }, [view, rows, capture, rate]);

  const local = (e: { clientX: number }) => {
    const rect = canvas.current!.getBoundingClientRect();
    return e.clientX - rect.left - LABEL_W;
  };
  const set = (r: Partial<ViewState>) => onView({ ...view, ...r });

  const onWheel = (e: WheelEvent<HTMLCanvasElement>) => {
    const horizontal = Math.abs(e.deltaX) > Math.abs(e.deltaY) || e.shiftKey;
    if (horizontal && !e.ctrlKey) {
      const d = e.shiftKey ? e.deltaY : e.deltaX;
      set(pan(view, d / plotW, count));
    } else {
      const at01 = Math.min(1, Math.max(0, local(e) / plotW));
      set(zoomAt(view, Math.exp(e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), at01, count));
    }
  };

  const edgesNear = (yPx: number): number[] => {
    const row = rows.find((r) => r.kind === 'channel' && yPx >= r.y && yPx < r.y + r.h);
    const t = data?.traces.find((tr) => tr.channel === row?.channel);
    return t?.edges ?? [];
  };

  /** The analog row whose bottom edge is under the pointer. */
  const edgeOf = (e: { clientY: number }): Row | undefined => {
    const y = e.clientY - canvas.current!.getBoundingClientRect().top;
    return rows.find((r) => r.kind === 'analog' && Math.abs(y - (r.y + r.h)) <= 5);
  };
  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const edge = edgeOf(e);
    if (edge?.analog) {
      resize.current = { id: edge.analog, y: e.clientY, h: edge.h };
      return;
    }
    drag.current = { x: e.clientX, start: view, moved: false, select: e.shiftKey };
  };
  const onPointerMove = (e: PointerEvent<HTMLCanvasElement>) => {
    const r = resize.current;
    if (r) {
      setResized({ id: r.id, h: Math.round(Math.min(800, Math.max(40, r.h + e.clientY - r.y))) });
      return;
    }
    const d = drag.current;
    e.currentTarget.style.cursor = !d && edgeOf(e) ? 'ns-resize' : '';
    if (!d) return;
    if (Math.abs(e.clientX - d.x) > 3) d.moved = true;
    if (!d.moved) return;
    if (d.select) {
      const a = xToSample(d.x - canvas.current!.getBoundingClientRect().left - LABEL_W, d.start, plotW);
      const b = xToSample(local(e), d.start, plotW);
      set({ selection: [Math.max(0, Math.round(Math.min(a, b))), Math.min(count, Math.round(Math.max(a, b)))] });
    } else {
      set(pan(d.start, -(e.clientX - d.x) / plotW, count));
    }
  };
  const onPointerUp = (e: PointerEvent<HTMLCanvasElement>) => {
    if (resize.current) {
      const id = resize.current.id;
      resize.current = null;
      const h = resized?.id === id ? resized.h : null;
      if (h !== null) {
        client
          .run('analog.update', { capture: capture.id, analog: id, height: h })
          .catch((x: unknown) => bridge().log('warn', `analog.update failed: ${x instanceof CommandError ? x.message : String(x)}`))
          .finally(() => setResized(null));
      }
      return;
    }
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved) return;
    const rect = canvas.current!.getBoundingClientRect();
    const raw = xToSample(local(e), view, plotW);
    if (raw < 0 || raw > count) return;
    const s = snap(raw, edgesNear(e.clientY - rect.top), view, plotW);
    if (e.altKey) set({ cursorB: s });
    else set({ cursorA: s });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLCanvasElement>) => {
    const keys: Record<string, () => ViewState> = {
      ArrowLeft: () => ({ ...view, ...pan(view, e.shiftKey ? -0.5 : -0.1, count) }),
      ArrowRight: () => ({ ...view, ...pan(view, e.shiftKey ? 0.5 : 0.1, count) }),
      '+': () => ({ ...view, ...zoomAt(view, 0.5, 0.5, count) }),
      '=': () => ({ ...view, ...zoomAt(view, 0.5, 0.5, count) }),
      '-': () => ({ ...view, ...zoomAt(view, 2, 0.5, count) }),
      Home: () => ({ ...view, ...clampRange({ from: 0, to: view.to - view.from }, count) }),
      End: () => ({ ...view, ...clampRange({ from: count - (view.to - view.from), to: count }, count) }),
      Escape: () => ({ ...view, selection: null, cursorA: null, cursorB: null }),
    };
    const f = keys[e.key];
    if (f && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      onView(f());
    }
  };

  return (
    <div ref={host} className="wave" data-expert={expert || undefined}>
      <canvas
        ref={canvas}
        className="wave__canvas"
        style={{ width, height }}
        tabIndex={0}
        role="img"
        aria-label={`Waveform of ${capture.title}. Arrow keys pan, plus and minus zoom, Home and End go to the ends.`}
        aria-describedby="wave-description"
        onWheel={onWheel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onKeyDown={onKeyDown}
      />
      {failed && (
        <div className="wave__failed" role="alert">
          <span>The waveform could not be drawn: {failed}</span>
          <Button size="sm" onClick={() => setRetry((r) => r + 1)}>
            Try again
          </Button>
        </div>
      )}
      <p id="wave-description" className="lg-visually-hidden" aria-live="polite">
        {description}
      </p>
    </div>
  );
}
