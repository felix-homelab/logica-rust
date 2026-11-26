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

/**
 * Signal time a live view shows at first, in seconds: following moves the view by its own width
 * every that long, so half a second keeps the bursts readable; zoom in or "Stay here" for detail.
 */
export const LIVE_SECONDS = 0.5;

/** Whether a view of a growing capture still reaches its newest data (within 2 % of the view). */
export function atEnd(r: Range, count: number): boolean {
  return r.to >= count - Math.max(1, (r.to - r.from) * 0.02);
}

/** The newest `span` samples of a capture of `count` samples. */
export function following(span: number, count: number): Range {
  return clampRange({ from: count - span, to: count }, count);
}

/**
 * The person changed a live view from `prev` to `next` (doc 08: the view follows the newest data
 * unless the person scrolls away). `span` is the width followed, or null when not following.
 * Zooming while following keeps following at the new width; panning away stops it; coming back to
 * the end follows again.
 */
export function liveView(prev: Range, next: Range, count: number, span: number | null): { span: number | null; range: Range } {
  const width = next.to - next.from;
  if (span !== null) {
    if (next.from === prev.from && next.to === prev.to) return { span, range: following(span, count) };
    if (width !== prev.to - prev.from) return { span: width, range: following(width, count) };
    return atEnd(next, count) ? { span, range: following(span, count) } : { span: null, range: clampRange(next, count) };
  }
  return atEnd(next, count) ? { span: width, range: following(width, count) } : { span: null, range: clampRange(next, count) };
}
