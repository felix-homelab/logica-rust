// The app's settings (doc 08: settings window): a JSON file in the data folder, read once and
// written on every change. Unknown or broken values fall back to the defaults.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export interface Settings {
  theme: 'system' | 'light' | 'dark';
  /** Serve MCP for agents (doc 07). */
  mcp: boolean;
  /** Hide explanations of detections and problems. */
  expert: boolean;
  /** The engine's chunk cache, in MB (doc 04). Takes effect at the next start. */
  memoryMb: number;
  /** Check GitHub Releases for updates (Windows and Linux). */
  updates: boolean;
  /** Recently opened files, newest first. */
  recent: string[];
}

export const DEFAULTS: Settings = { theme: 'system', mcp: true, expert: false, memoryMb: 512, updates: true, recent: [] };

const MAX_RECENT = 10;

export function load(path: string): Settings {
  try {
    const raw = JSON.parse(readFileSync(path, 'utf8')) as Partial<Settings>;
    return sanitize({ ...DEFAULTS, ...raw });
  } catch {
    return { ...DEFAULTS };
  }
}

export function sanitize(s: Settings): Settings {
  return {
    theme: ['system', 'light', 'dark'].includes(s.theme) ? s.theme : DEFAULTS.theme,
    mcp: typeof s.mcp === 'boolean' ? s.mcp : DEFAULTS.mcp,
    expert: typeof s.expert === 'boolean' ? s.expert : DEFAULTS.expert,
    memoryMb: Number.isFinite(s.memoryMb) ? Math.min(65536, Math.max(64, Math.round(s.memoryMb))) : DEFAULTS.memoryMb,
    updates: typeof s.updates === 'boolean' ? s.updates : DEFAULTS.updates,
    recent: Array.isArray(s.recent) ? s.recent.filter((r) => typeof r === 'string').slice(0, MAX_RECENT) : [],
  };
}

export function save(path: string, s: Settings): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(s, null, 2));
}

/** The recent list with `file` first. */
export function withRecent(s: Settings, file: string): Settings {
  return { ...s, recent: [file, ...s.recent.filter((r) => r !== file)].slice(0, MAX_RECENT) };
}

/** Capture files among command-line arguments (file associations, `logica open`). */
export function filesIn(argv: string[]): string[] {
  return argv.filter((a) => /\.(lacap|demo|sr|vcd)$/i.test(a) && !a.startsWith('-'));
}
