// Decisions of the main process that are worth testing on their own: what to do when the engine
// process ends, and where to put the window.

export type EngineState =
  | { state: 'starting' }
  | { state: 'running'; restarts: number }
  | { state: 'restarting'; reason: string; restarts: number }
  | { state: 'failed'; reason: string };

export type AfterExit = { kind: 'restart'; delayMs: number; reason: string } | { kind: 'fail'; reason: string };

export const CRASH_WINDOW_MS = 60_000;
export const MAX_CRASHES = 3;

/**
 * What to do after the engine process ended unexpectedly. A start failure the engine reported
 * (its module cannot load) would only repeat, so it is final; otherwise it restarts with a growing
 * delay, until it ends more than MAX_CRASHES times within a minute. `crashes` holds the times of
 * earlier crashes and gets this one added.
 */
export function afterExit(code: number | null, fatal: string | null, crashes: number[], now: number): AfterExit {
  if (fatal) return { kind: 'fail', reason: `The engine could not start: ${fatal}` };
  crashes.push(now);
  while (crashes.length && crashes[0]! < now - CRASH_WINDOW_MS) crashes.shift();
  if (crashes.length > MAX_CRASHES) {
    return {
      kind: 'fail',
      reason: `The engine stopped ${crashes.length} times within a minute (last exit code ${code ?? 'unknown'}), so it is not started again by itself.`,
    };
  }
  return {
    kind: 'restart',
    delayMs: 500 * crashes.length,
    reason: `The engine stopped unexpectedly (exit code ${code ?? 'unknown'}) and is being restarted.`,
  };
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface WindowState {
  bounds: Rect;
  maximized: boolean;
}

/** The saved window state if it is sane and at least partly on a display now; else null. */
export function restorable(saved: unknown, displays: Rect[]): WindowState | null {
  const s = saved as Partial<WindowState> | null;
  const b = s?.bounds;
  if (!b || ![b.x, b.y, b.width, b.height].every(Number.isFinite) || b.width < 400 || b.height < 300) return null;
  // enough of the title bar on some display to grab it
  const d = displays.find((d) => b.x + b.width - 100 > d.x && b.x + 100 < d.x + d.width && b.y >= d.y - 10 && b.y + 40 < d.y + d.height);
  if (!d) return null;
  // never bigger than that display (it may be smaller than the one the size was saved on; Windows
  // would shrink the window anyway): then it is moved to lie fully on it
  if (b.width <= d.width && b.height <= d.height) return { bounds: b, maximized: s?.maximized === true };
  const width = Math.min(b.width, d.width);
  const height = Math.min(b.height, d.height);
  const x = Math.min(Math.max(b.x, d.x), d.x + d.width - width);
  const y = Math.min(Math.max(b.y, d.y), d.y + d.height - height);
  return { bounds: { x, y, width, height }, maximized: s?.maximized === true };
}
