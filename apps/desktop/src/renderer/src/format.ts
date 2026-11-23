// Numbers as the engine writes them (crates/protocols/src/format.rs): time in seconds with a
// sensible unit, frequencies, counts with separators. Time inside the engine is the sample index;
// seconds appear only here, at the edge.

/** `x` with `digits` significant digits, trailing zeros dropped. */
export function sig(x: number, digits: number): string {
  if (!Number.isFinite(x)) return '–';
  if (x === 0) return '0';
  const s = Number(x.toPrecision(digits)).toString();
  return s.includes('e') ? x.toPrecision(digits) : s;
}

/** "8.68 µs", "1.5 ms", "20 ns", "0 s". */
export function time(t: number): string {
  if (!Number.isFinite(t)) return '–';
  const a = Math.abs(t);
  if (a === 0) return '0 s';
  if (a >= 1) return `${sig(t, 4)} s`;
  if (a >= 1e-3) return `${sig(t * 1e3, 4)} ms`;
  if (a >= 1e-6) return `${sig(t * 1e6, 4)} µs`;
  return `${sig(t * 1e9, 4)} ns`;
}

/** "115.2 kHz", "1 MHz", "50 Hz". */
export function hz(f: number): string {
  if (!Number.isFinite(f)) return '–';
  const a = Math.abs(f);
  if (a >= 1e9) return `${sig(f / 1e9, 4)} GHz`;
  if (a >= 1e6) return `${sig(f / 1e6, 4)} MHz`;
  if (a >= 1e3) return `${sig(f / 1e3, 4)} kHz`;
  return `${sig(f, 4)} Hz`;
}

/** "12,000,000". */
export function int(n: number): string {
  return Math.round(n).toLocaleString('en-US');
}

/** A sample as time, with the sample for exact reference: "1.25 ms (sample 12,500)". */
export function at(sample: number, rate: number): string {
  return rate > 0 ? `${time(sample / rate)} (sample ${int(sample)})` : `sample ${int(sample)}`;
}

/** A tick label with as many decimals as the step needs ("660.005 ms"). */
export function tick(t: number, step: number): string {
  if (t === 0) return '0';
  const a = Math.abs(t);
  const [unit, name] = a >= 1 ? [1, 's'] : a >= 1e-3 ? [1e-3, 'ms'] : a >= 1e-6 ? [1e-6, 'µs'] : [1e-9, 'ns'];
  const decimals = Math.min(9, Math.max(0, Math.ceil(-Math.log10(step / unit))));
  return `${(t / unit).toFixed(decimals)} ${name}`;
}

/** A round tick step (1, 2 or 5 × 10^n) for about `count` ticks over `span` seconds. */
export function tickStep(span: number, count: number): number {
  const raw = span / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? mag * 10;
}
