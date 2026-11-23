// The engine process (doc 03): a thin Node host around the Rust engine's Node-API module. It
// answers commands arriving on the window's MessagePort and passes the engine's events on.
import { createRequire } from 'node:module';
import type { MessagePortMain } from 'electron';

interface NativeEngine {
  apiVersion(): string;
  execute(actor: string, command: string, input: string): Promise<string>;
  nextEvents(timeoutMs: number): Promise<string>;
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

interface Call {
  id: number;
  command: string;
  input?: unknown;
}

function serve(port: MessagePortMain): void {
  port.on('message', (e) => {
    const { id, command, input } = e.data as Call;
    // the window is the person: every command from it carries the actor "user" (doc 03)
    engine
      .execute('user', command, JSON.stringify(input ?? {}))
      .then((response) => port.postMessage({ id, response }))
      .catch((err: unknown) =>
        port.postMessage({
          id,
          response: JSON.stringify({ ok: false, error: { code: 'failed', message: String(err) } }),
        }),
      );
  });
  port.start();
  // events: long polls, so the engine's threads never call into JavaScript
  void (async () => {
    for (;;) {
      const events = JSON.parse(await engine.nextEvents(1000)) as unknown[];
      for (const event of events) {
        port.postMessage({ event });
      }
    }
  })();
}

const parent = (process as unknown as { parentPort: Electron.ParentPort }).parentPort;
parent.on('message', (e) => {
  const port = e.ports[0];
  if (port) {
    serve(port);
  }
});
