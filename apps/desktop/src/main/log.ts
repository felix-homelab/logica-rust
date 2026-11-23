// The app's log file (logs/logica.log in the data folder): what went wrong, from the main process,
// the engine process and the window, kept for support. Rotated at 1 MB, three old files kept.
// Logging never throws: a full disk must not take the app down.
import { appendFileSync, existsSync, mkdirSync, renameSync, statSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';

const MAX_BYTES = 1 << 20;
const KEEP = 3;

let dir: string | null = null;

export function init(logDir: string): void {
  dir = logDir;
  try {
    mkdirSync(dir, { recursive: true });
  } catch {
    dir = null;
  }
}

export const file = (): string | null => (dir ? join(dir, 'logica.log') : null);

/** Moves logica.log to logica.1.log (and so on) once it is over the size limit. */
export function rotate(path: string, maxBytes = MAX_BYTES, keep = KEEP): void {
  try {
    if (!existsSync(path) || statSync(path).size < maxBytes) return;
    const old = (n: number) => path.replace(/\.log$/, `.${n}.log`);
    if (existsSync(old(keep))) unlinkSync(old(keep));
    for (let n = keep - 1; n >= 1; n--) {
      if (existsSync(old(n))) renameSync(old(n), old(n + 1));
    }
    renameSync(path, old(1));
  } catch {
    // keep writing to the big file rather than lose messages
  }
}

export type Level = 'info' | 'warn' | 'error';

export function write(level: Level, source: string, message: string): void {
  const line = `${new Date().toISOString()} ${level.padEnd(5)} ${source}: ${message.replace(/\s+$/, '')}\n`;
  if (level !== 'info') process.stderr.write(line);
  const f = file();
  if (!f) return;
  try {
    rotate(f);
    appendFileSync(f, line);
  } catch {
    // logging must never take the app down
  }
}
