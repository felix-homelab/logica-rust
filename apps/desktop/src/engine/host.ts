// The engine process (doc 03): a thin Node host around the Rust engine's Node-API module. It
// answers commands arriving on each window's MessagePort, passes the engine's events to every
// window, and serves MCP for agents when the settings say so (doc 07, ADR 0007). It tells the main
// process when it is ready, or why it could not start; main restarts it if it ends unexpectedly.
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
  LogicaEngine: new (workDir: string, cacheMb?: number) => NativeEngine;
}

const parent = (process as unknown as { parentPort: Electron.ParentPort }).parentPort;

/** Tells main the engine cannot start, and why; main shows it and does not retry. */
function fatal(message: string): never {
  console.error(`The engine could not start: ${message}`);
  parent.postMessage({ type: 'fatal', message });
  // give the message time to leave before ending
  setTimeout(() => process.exit(2), 100);
  throw new Error(message);
}

// anything unexpected ends the process; main logs it and starts a new one
process.on('uncaughtException', (e) => {
  console.error(`The engine host failed: ${e.stack ?? e.message}`);
  process.exit(1);
});
process.on('unhandledRejection', (e) => {
  console.error(`The engine host failed: ${e instanceof Error ? (e.stack ?? e.message) : String(e)}`);
  process.exit(1);
});

const nativePath = process.env.LOGICA_NATIVE;
const workDir = process.env.LOGICA_WORK_DIR;
if (!nativePath || !workDir) fatal('the app did not say where the engine module and the working folder are.');

let engine: NativeEngine;
try {
  const native = createRequire(__filename)(nativePath!) as Native;
  const cacheMb = Number(process.env.LOGICA_CACHE_MB);
  engine = new native.LogicaEngine(workDir!, Number.isFinite(cacheMb) ? cacheMb : undefined);
} catch (e) {
  fatal(`${nativePath} could not be loaded (${e instanceof Error ? e.message : String(e)}).`);
}

const ports = new Set<MessagePortMain>();

interface Call {
  id: number;
  command: string;
  input?: unknown;
}

const failure = (id: number, message: string) => ({ id, response: JSON.stringify({ ok: false, error: { code: 'failed', message } }) });

function serve(port: MessagePortMain): void {
  ports.add(port);
  port.on('message', (e) => {
    const { id, command, input } = (e.data ?? {}) as Partial<Call>;
    if (typeof id !== 'number' || typeof command !== 'string') return;
    let text: string;
    try {
      text = JSON.stringify(input ?? {});
    } catch {
      port.postMessage(failure(id, `The input of ${command} cannot be sent to the engine.`));
      return;
    }
    // a window is the person: every command from it carries the actor "user" (doc 03)
    engine
      .execute('user', command, text)
      .then((response) => port.postMessage({ id, response }))
      .catch((err: unknown) => port.postMessage(failure(id, `The engine failed on ${command}: ${String(err)}`)));
  });
  port.on('close', () => ports.delete(port));
  port.start();
}

// events: long polls, so the engine's threads never call into JavaScript; an error does not end
// the loop, it waits a moment and goes on
void (async () => {
  for (;;) {
    try {
      const events = JSON.parse(await engine.nextEvents(1000)) as unknown[];
      for (const event of events) {
        for (const p of ports) p.postMessage({ event });
      }
    } catch (e) {
      console.error(`Engine events could not be passed on: ${String(e)}`);
      await new Promise((r) => setTimeout(r, 500));
    }
  }
})();

function startMcp(dataDir: string): void {
  try {
    parent.postMessage({ type: 'mcp', port: engine.startMcp(dataDir) });
  } catch (e) {
    parent.postMessage({ type: 'mcp', error: e instanceof Error ? e.message : String(e) });
  }
}

function stopMcp(): void {
  try {
    engine.stopMcp();
  } finally {
    parent.postMessage({ type: 'mcp' });
  }
}

parent.on('message', (e) => {
  const msg = (e.data ?? {}) as { type?: string; dataDir?: string };
  if (msg.type === 'port' && e.ports[0]) serve(e.ports[0]);
  else if (msg.type === 'mcp-start' && msg.dataDir) startMcp(msg.dataDir);
  else if (msg.type === 'mcp-stop') stopMcp();
});

if (process.env.LOGICA_MCP === '1' && process.env.LOGICA_DATA_DIR) startMcp(process.env.LOGICA_DATA_DIR);

// the lock file must not outlive the engine
process.on('exit', () => {
  try {
    engine.stopMcp();
  } catch {
    // ending anyway
  }
});

parent.postMessage({ type: 'ready', apiVersion: engine.apiVersion() });
