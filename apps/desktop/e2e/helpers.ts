// Starting the app for a test, and talking MCP to it as an agent would (over HTTP, with the token
// from the lock file).
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { _electron as electron, type ElectronApplication, type Page } from '@playwright/test';

export const repo = resolve(__dirname, '..', '..', '..');
export const fixture = (name: string): string => join(repo, 'fixtures', name);

export interface Running {
  app: ElectronApplication;
  page: Page;
  dataDir: string;
}

export interface StartOptions {
  env?: Record<string, string>;
  /** Reuse a data folder (settings, window size, lock file) instead of a fresh one. */
  dataDir?: string;
  /** Keep the window size the app chooses (restored from the last run). */
  keepSize?: boolean;
}

/** The built app with its own, fresh data folder; `files` are opened as from a double-click. */
export async function start(files: string[] = [], options: StartOptions = {}): Promise<Running> {
  const env = options.env ?? {};
  const dataDir = options.dataDir ?? mkdtempSync(join(tmpdir(), 'logica-e2e-'));
  const extra = process.platform === 'linux' && process.env.LOGICA_SMOKE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
  const app = await electron.launch({
    args: [resolve(__dirname, '..'), ...extra, ...files],
    env: { ...process.env, LOGICA_DATA_DIR: dataDir, ...env } as Record<string, string>,
  });
  const page = await app.firstWindow();
  // the same layout on every machine, whatever its screen (CI runners have small ones); the
  // narrow-window test makes its own size
  if (!options.keepSize) await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.setSize(1440, 900));
  // the native "unsaved changes" prompt cannot be clicked in a test: record it and answer
  // "close without saving" (or what a test sets in __promptAnswer)
  await app.evaluate(({ dialog }) => {
    const g = globalThis as unknown as { __prompts: unknown[]; __promptAnswer: number };
    g.__prompts = [];
    g.__promptAnswer = 0;
    dialog.showMessageBoxSync = ((...args: unknown[]) => {
      g.__prompts.push(args[args.length - 1]);
      return g.__promptAnswer;
    }) as typeof dialog.showMessageBoxSync;
  });
  await page.waitForSelector('.app, .boot');
  return { app, page, dataDir };
}

/** A minimal MCP client over Streamable HTTP, as `logica mcp` relays for an agent. */
export class Agent {
  private session: string | null = null;
  private id = 1;
  constructor(
    private port: number,
    private token: string,
    readonly name: string,
  ) {}

  static async connect(dataDir: string, name: string): Promise<Agent> {
    const deadline = Date.now() + 15_000;
    for (;;) {
      try {
        const lock = JSON.parse(readFileSync(join(dataDir, 'logica.lock'), 'utf8')) as { port: number; token: string };
        const a = new Agent(lock.port, lock.token, name);
        await a.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name, version: '1' } });
        await a.post({ jsonrpc: '2.0', method: 'notifications/initialized' });
        return a;
      } catch (e) {
        if (Date.now() > deadline) throw e;
        await new Promise((r) => setTimeout(r, 200));
      }
    }
  }

  private async post(body: unknown): Promise<unknown> {
    const headers: Record<string, string> = {
      authorization: `Bearer ${this.token}`,
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    };
    if (this.session) {
      headers['mcp-session-id'] = this.session;
      headers['mcp-protocol-version'] = '2025-06-18';
    }
    const r = await fetch(`http://127.0.0.1:${this.port}/mcp`, { method: 'POST', headers, body: JSON.stringify(body) });
    if (!r.ok && r.status !== 202) throw new Error(`MCP answered HTTP ${r.status}: ${await r.text()}`);
    this.session ??= r.headers.get('mcp-session-id');
    const text = await r.text();
    if (!text) return null;
    if ((r.headers.get('content-type') ?? '').startsWith('text/event-stream')) {
      const data = text
        .split('\n')
        .filter((l) => l.startsWith('data:'))
        .map((l) => l.slice(5).trim())
        .filter(Boolean)
        .map((d) => JSON.parse(d) as { id?: number });
      return data.find((d) => d.id !== undefined) ?? null;
    }
    return JSON.parse(text);
  }

  async request(method: string, params: unknown): Promise<{ result?: unknown; error?: { message: string } }> {
    return (await this.post({ jsonrpc: '2.0', id: this.id++, method, params })) as { result?: unknown; error?: { message: string } };
  }

  /** Calls a tool and returns its text; throws on a tool error. */
  async tool(name: string, args: Record<string, unknown>): Promise<string> {
    const r = await this.request('tools/call', { name, arguments: args });
    const res = r.result as { content: { text?: string }[]; isError?: boolean } | undefined;
    if (!res) throw new Error(r.error?.message ?? 'no result');
    const text = res.content.map((c) => c.text ?? '').join('\n');
    if (res.isError) throw new Error(text);
    return text;
  }
}
