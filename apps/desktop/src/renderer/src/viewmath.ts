// The waveform's view arithmetic, in samples (doc 04: time inside is the sample index). Kept free
// of the DOM so it is tested in plain Node.

export interface Range {
  from: number;
  to: number;
}

/** Fewest samples a view shows: zooming stops at about 8 pixels a sample. */
export const MIN_SPAN = 16;

/** Keeps a range inside [0, count) and at least MIN_SPAN wide (or the whole capture). */
export function clampRange(r: Range, count: number): Range {
  const min = Math.min(MIN_SPAN, count);
  let span = Math.max(min, Math.min(count, r.to - r.from));
  let from = Math.round(r.from);
  span = Math.round(span);
  from = Math.max(0, Math.min(count - span, from));
  return { from, to: from + span };
}

/** Zooms by `factor` (<1 zooms in) keeping the sample under `at` (0 to 1 across the view) still. */
export function zoomAt(r: Range, factor: number, at: number, count: number): Range {
  const span = r.to - r.from;
  const anchor = r.from + span * at;
  const next = span * factor;
  return clampRange({ from: anchor - next * at, to: anchor - next * at + next }, count);
}

/** Moves by `fraction` of the view's width (negative: towards the start). */
export function pan(r: Range, fraction: number, count: number): Range {
  const d = (r.to - r.from) * fraction;
  return clampRange({ from: r.from + d, to: r.to + d }, count);
}

export const sampleToX = (s: number, r: Range, width: number): number => ((s - r.from) / (r.to - r.from)) * width;

export const xToSample = (x: number, r: Range, width: number): number => r.from + (x / width) * (r.to - r.from);

/** The edge nearest to `sample` within `px` pixels, else `sample` itself. `edges` ascending. */
export function snap(sample: number, edges: readonly number[], r: Range, width: number, px = 6): number {
  if (edges.length === 0) return Math.round(sample);
  let lo = 0;
  let hi = edges.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((edges[mid] ?? 0) < sample) lo = mid + 1;
    else hi = mid;
  }
  const near = [edges[lo - 1], edges[lo]].filter((e): e is number => e !== undefined);
  const best = near.reduce((a, b) => (Math.abs(b - sample) < Math.abs(a - sample) ? b : a));
  const limit = (px / width) * (r.to - r.from);
  return Math.abs(best - sample) <= limit ? best : Math.round(sample);
}

/** A range of `span` samples centred on `sample`. */
export function centreOn(sample: number, span: number, count: number): Range {
  return clampRange({ from: sample - span / 2, to: sample + span / 2 }, count);
}
