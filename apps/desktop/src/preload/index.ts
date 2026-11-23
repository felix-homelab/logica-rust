// The preload (doc 03): the window's only way to the engine. It exposes one object, the command
// API transport and an event subscription, over the MessagePort the main process hands it. No
// ipcRenderer, file system or shell reaches the page.
import { contextBridge, ipcRenderer } from 'electron';

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
  if (!p) {
    return;
  }
  port = p;
  p.onmessage = (m: MessageEvent) => {
    const data = m.data as { id?: number; response?: string; event?: unknown };
    if (data.event !== undefined) {
      listeners.forEach((l) => l(data.event));
      return;
    }
    const waiting = data.id === undefined ? undefined : pending.get(data.id);
    if (!waiting || data.id === undefined) {
      return;
    }
    pending.delete(data.id);
    const r = JSON.parse(data.response ?? '{}') as { ok: boolean; output?: unknown; error?: Failure };
    if (r.ok) {
      waiting.resolve(r.output);
    } else {
      waiting.reject(r.error ?? { code: 'failed', message: 'The engine gave no answer.' });
    }
  };
  p.start();
  for (const msg of queued.splice(0)) {
    p.postMessage(msg);
  }
});

contextBridge.exposeInMainWorld('logica', {
  /** Runs a command; rejects with { code, message } (the client turns it into a CommandError). */
  call(command: string, input: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      const msg = { id, command, input };
      if (port) {
        port.postMessage(msg);
      } else {
        queued.push(msg);
      }
    });
  },
  /** Engine events (doc 03); returns the function that ends the subscription. */
  onEvent(listener: (event: unknown) => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** When the app process started (milliseconds since the epoch), for the cold start. */
  launchedAt(): number {
    return launchedAt;
  },
  /** Tells the app the window has its first answer from the engine. */
  ready(coldStartMs: number): void {
    ipcRenderer.send('logica:ready', coldStartMs);
  },
  failed(message: string): void {
    ipcRenderer.send('logica:failed', message);
  },
});
