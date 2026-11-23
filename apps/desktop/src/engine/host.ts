// The engine process (doc 03): a thin Node host around the Rust engine's Node-API module. It
// answers commands arriving on each window's MessagePort, passes the engine's events to every
// window, and serves MCP for agents when the settings say so (doc 07, ADR 0007).
import { createRequire } from 'node:module';
import type { MessagePortMain } from 'electron';

interface NativeEngine {
  apiVersion(): string;
  execute(actor: string, command: string, input: string): Promise<string>;
  nextEvents(timeoutMs: number): Promise<string>;
  startMcp(dataDir: string): number;
  stopMcp(): void;
}

interface Native {
  LogicaEngine: new (workDir: string) => NativeEngine;
}

const nativePath = process.env.LOGICA_NATIVE;
const workDir = process.env.LOGICA_WORK_DIR;
if (!nativePath || !workDir) {
  console.error('logica: the engine process needs LOGICA_NATIVE and LOGICA_WORK_DIR.');
  process.exit(2);
}

const native = createRequire(__filename)(nativePath) as Native;
const engine = new native.LogicaEngine(workDir);
const parent = (process as unknown as { parentPort: Electron.ParentPort }).parentPort;
const ports = new Set<MessagePortMain>();

interface Call {
  id: number;
  command: string;
  input?: unknown;
}

function serve(port: MessagePortMain): void {
  ports.add(port);
  port.on('message', (e) => {
    const { id, command, input } = e.data as Call;
    // a window is the person: every command from it carries the actor "user" (doc 03)
    engine
      .execute('user', command, JSON.stringify(input ?? {}))
      .then((response) => port.postMessage({ id, response }))
      .catch((err: unknown) =>
        port.postMessage({ id, response: JSON.stringify({ ok: false, error: { code: 'failed', message: String(err) } }) }),
      );
  });
  port.on('close', () => ports.delete(port));
  port.start();
}

// events: long polls, so the engine's threads never call into JavaScript
void (async () => {
  for (;;) {
    const events = JSON.parse(await engine.nextEvents(1000)) as unknown[];
    for (const event of events) {
      for (const p of ports) p.postMessage({ event });
    }
  }
})();

function startMcp(dataDir: string): void {
  try {
    const port = engine.startMcp(dataDir);
    parent.postMessage({ type: 'mcp', port });
  } catch (e) {
    parent.postMessage({ type: 'mcp', error: e instanceof Error ? e.message : String(e) });
  }
}

function stopMcp(): void {
  engine.stopMcp();
  parent.postMessage({ type: 'mcp' });
}

if (process.env.LOGICA_MCP === '1' && process.env.LOGICA_DATA_DIR) {
  startMcp(process.env.LOGICA_DATA_DIR);
}

parent.on('message', (e) => {
  const msg = e.data as { type: string; dataDir?: string };
  if (msg.type === 'port' && e.ports[0]) serve(e.ports[0]);
  else if (msg.type === 'mcp-start' && msg.dataDir) startMcp(msg.dataDir);
  else if (msg.type === 'mcp-stop') stopMcp();
});

// the lock file must not outlive the engine
process.on('exit', () => engine.stopMcp());
