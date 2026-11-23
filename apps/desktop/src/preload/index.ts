// The preload (doc 03): the window's only way out. It exposes one object: the command API
// transport and event subscription over the MessagePort, and the few window services (native
// dialogs, the path of a dropped file, settings, menu actions). No ipcRenderer, file system or
// shell reaches the page.
import { contextBridge, ipcRenderer, webUtils } from 'electron';

interface Failure {
  code: string;
  message: string;
}

type Pending = { resolve: (v: unknown) => void; reject: (e: Failure) => void };

let port: MessagePort | undefined;
let launchedAt = 0;
let nextId = 1;
const pending = new Map<number, Pending>();
const queued: { id: number; command: string; input: unknown }[] = [];
const listeners = new Set<(event: unknown) => void>();

ipcRenderer.on('logica:engine-port', (e, info: { launchedAt: number }) => {
  launchedAt = info.launchedAt;
  const p = e.ports[0];
  if (!p) return;
  port = p;
  p.onmessage = (m: MessageEvent) => {
    const data = m.data as { id?: number; response?: string; event?: unknown };
    if (data.event !== undefined) {
      listeners.forEach((l) => l(data.event));
      return;
    }
    const waiting = data.id === undefined ? undefined : pending.get(data.id);
    if (!waiting || data.id === undefined) return;
    pending.delete(data.id);
    const r = JSON.parse(data.response ?? '{}') as { ok: boolean; output?: unknown; error?: Failure };
    if (r.ok) waiting.resolve(r.output);
    else waiting.reject(r.error ?? { code: 'failed', message: 'The engine gave no answer.' });
  };
  p.start();
  for (const msg of queued.splice(0)) p.postMessage(msg);
});

// Main may send before the page has subscribed: keep the latest value of each channel (and every
// file to open) and hand them to subscribers when they arrive.
const latest = new Map<string, unknown>();
const fileQueue: string[][] = [];
const subscribers = new Map<string, Set<(v: unknown) => void>>();
for (const channel of ['logica:update', 'logica:env', 'logica:settings', 'logica:mcp', 'logica:open-files', 'logica:menu', 'logica:started-by-agent']) {
  ipcRenderer.on(channel, (_e, value: unknown) => {
    const subs = subscribers.get(channel);
    if (channel === 'logica:open-files' && !subs?.size) {
      fileQueue.push(value as string[]);
      return;
    }
    if (channel !== 'logica:menu' && channel !== 'logica:open-files') {
      latest.set(channel, value === undefined ? true : value);
    }
    subs?.forEach((s) => s(value));
  });
}

/** Subscribes to a main-process channel, replaying what came before; returns the unsubscribe. */
function on<T>(channel: string, listener: (value: T) => void): () => void {
  const f = listener as (v: unknown) => void;
  const subs = subscribers.get(channel) ?? new Set();
  subscribers.set(channel, subs);
  subs.add(f);
  if (latest.has(channel)) queueMicrotask(() => f(latest.get(channel)));
  if (channel === 'logica:open-files') {
    for (const files of fileQueue.splice(0)) queueMicrotask(() => f(files));
  }
  return () => subs.delete(f);
}

contextBridge.exposeInMainWorld('logica', {
  /** Runs a command; rejects with { code, message } (the client turns it into a CommandError). */
  call(command: string, input: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      const msg = { id, command, input };
      if (port) port.postMessage(msg);
      else queued.push(msg);
    });
  },
  /** Engine events (doc 03); returns the function that ends the subscription. */
  onEvent(listener: (event: unknown) => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  launchedAt: (): number => launchedAt,
  ready: (coldStartMs: number): void => ipcRenderer.send('logica:ready', coldStartMs),
  failed: (message: string): void => ipcRenderer.send('logica:failed', message),

  /** Native dialogs (doc 08): paths the person picks go to the engine as command inputs. */
  chooseFiles: (): Promise<string[]> => ipcRenderer.invoke('logica:choose-files') as Promise<string[]>,
  chooseSave: (suggested: string): Promise<string | null> => ipcRenderer.invoke('logica:choose-save', suggested) as Promise<string | null>,
  /** The path of a file dropped onto the window. */
  pathOf: (file: File): string => webUtils.getPathForFile(file),
  opened: (file: string): void => ipcRenderer.send('logica:opened', file),

  getSettings: (): Promise<unknown> => ipcRenderer.invoke('logica:settings-get'),
  setSettings: (s: unknown): Promise<unknown> => ipcRenderer.invoke('logica:settings-set', s),
  openSettings: (): void => ipcRenderer.send('logica:open-settings'),
  onSettings: (l: (s: unknown) => void) => on('logica:settings', l),
  onMcp: (l: (m: { port: number | null; error: string | null }) => void) => on('logica:mcp', l),
  onOpenFiles: (l: (files: string[]) => void) => on('logica:open-files', l),
  onMenu: (l: (id: string) => void) => on('logica:menu', l),
  onStartedByAgent: (l: () => void) => on('logica:started-by-agent', l),
  onEnv: (l: (e: { packaged: boolean }) => void) => on('logica:env', l),
  onUpdate: (l: (u: { version: string }) => void) => on('logica:update', l),
  platform: process.platform,
});
