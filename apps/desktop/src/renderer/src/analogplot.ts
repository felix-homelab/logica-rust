// Drawing virtual analog channels (ADR 0010, doc 08): the value axis and the lines through the
// values. No window code here, so it is tested on its own.

/** The value range a row shows: the values with a little room, never empty. */
export function valueRange(min: number | null | undefined, max: number | null | undefined): [number, number] {
  if (min === null || min === undefined || max === null || max === undefined) return [0, 1];
  if (max === min) {
    const pad = Math.abs(min) * 0.1 || 1;
    return [min - pad, max + pad];
  }
  const pad = (max - min) * 0.08;
  return [min - pad, max + pad];
}

/** How many ticks a plot `h` pixels high has room for: one every 28 pixels, at least two. */
export function tickCount(h: number): number {
  return Math.max(2, Math.floor(h / 28));
}

/** The round distance between ticks for at most `count` of them in `[lo, hi]`; 0 when none fit. */
export function valueTickStep(lo: number, hi: number, count: number): number {
  if (!(hi > lo) || count < 1) return 0;
  const raw = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? 10 * mag;
}

/** Round tick values inside `[lo, hi]`, at most `count` of them. */
export function valueTicks(lo: number, hi: number, count: number): number[] {
  const step = valueTickStep(lo, hi, count);
  if (!step) return [];
  const out: number[] = [];
  for (let i = Math.ceil(lo / step); i * step <= hi + step * 1e-9; i++) out.push(i === 0 ? 0 : i * step);
  return out;
}

/** Where a value sits in a row from `top` with height `h`: `hi` at the top, `lo` at the bottom. */
export function valueY(v: number, lo: number, hi: number, top: number, h: number): number {
  return top + h - ((v - lo) / (hi - lo)) * h;
}

/** A value as text, with as many decimals as one step of the source tells apart. */
export function formatValue(v: number, unit: string, step?: number | null): string {
  let digits: number;
  if (step && step > 0) digits = Math.min(6, Math.max(0, Math.ceil(-Math.log10(step))));
  else digits = Math.abs(v) >= 100 ? 1 : Math.abs(v) >= 1 ? 3 : 4;
  const text = v.toFixed(digits);
  return unit ? `${text} ${unit}` : text;
}

export interface ValuePoint {
  sample: number;
  value?: number | null;
}

/**
 * The lines through a range's points, in (sample, value) pairs, broken where there is no value.
 * `held` is the value at `from` from a point before the range. Steps hold each value until the
 * next point; lines join the points.
 */
export function valueLines(points: ValuePoint[], held: number | null | undefined, from: number, to: number, style: 'step' | 'line'): [number, number][][] {
  const lines: [number, number][][] = [];
  let cur: [number, number][] | null = null;
  let last: number | null = held ?? null;
  if (last !== null) cur = [[from, last]];
  for (const p of points) {
    const v = p.value ?? null;
    if (cur && last !== null) {
      if (style === 'step' || v === null) cur.push([p.sample, last]);
      if (v === null) {
        lines.push(cur);
        cur = null;
      }
    }
    if (v !== null) {
      if (!cur) cur = [];
      cur.push([p.sample, v]);
    }
    last = v;
  }
  if (cur && last !== null) {
    cur.push([to, last]);
    lines.push(cur);
  }
  return lines;
}

export interface ReadValue {
  value?: number | null;
  code?: number | null;
}

/** A channel's value at a cursor, with its raw code where it has one. */
export function readout(v: ReadValue | undefined, unit: string, step?: number | null): string {
  if (v?.value === null || v?.value === undefined) return 'no value';
  const code = v.code !== null && v.code !== undefined ? ` (code ${v.code})` : '';
  return `${formatValue(v.value, unit, step)}${code}`;
}

/** The difference from A to B, when both have a value. */
export function difference(a: ReadValue | undefined, b: ReadValue | undefined, unit: string, step?: number | null): string | null {
  if (a?.value === null || a?.value === undefined || b?.value === null || b?.value === undefined) return null;
  const d = b.value - a.value;
  return `${d > 0 ? '+' : ''}${formatValue(d, unit, step)}`;
}
