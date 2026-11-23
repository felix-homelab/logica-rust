// Spike S7's smoke test: starts the packaged app (npm run package:dir first), waits until the
// window has its first answer from the engine, and reports the cold start. Fails when the app
// does not get there, or when the median of three cold starts is over the budget (1.5 s, doc 11;
// LOGICA_COLD_START_LIMIT_MS overrides it for slow machines).
// With --dev it starts the built app with the Electron from node_modules instead.
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const dev = process.argv.includes('--dev');
const limit = Number(process.env.LOGICA_COLD_START_LIMIT_MS ?? 1500);

function command() {
  if (dev) {
    const electron = createRequire(import.meta.url)('electron');
    return [electron, [app]];
  }
  const packaged = {
    linux: join(app, 'dist', 'linux-unpacked', 'logica-app'),
    win32: join(app, 'dist', 'win-unpacked', 'Logica.exe'),
    darwin: join(app, 'dist', 'mac', 'Logica.app', 'Contents', 'MacOS', 'Logica'),
  }[process.platform];
  if (!packaged || !existsSync(packaged)) {
    console.error(`No packaged app at ${packaged}; run npm run package:dir first (or use --dev).`);
    process.exit(1);
  }
  return [packaged, []];
}

function once() {
  const [exe, args] = command();
  // Chromium's own sandbox needs a set-up helper on Linux that CI runners lack; the window's
  // JavaScript sandbox (sandbox: true) is unaffected
  const extra = process.platform === 'linux' && process.env.LOGICA_SMOKE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
  const env = { ...process.env, LOGICA_SMOKE: '1', LOGICA_DATA_DIR: mkdtempSync(join(tmpdir(), 'logica-smoke-')) };
  return new Promise((resolve, reject) => {
    const spawned = performance.now();
    const child = spawn(exe, [...args, ...extra], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    // measured from outside: from starting the executable to the window's first engine answer
    // (the app's own figure starts later, when Node starts inside it)
    let seen;
    child.stdout.on('data', (d) => {
      out += d;
      if (seen === undefined && /logica: cold start/.test(out)) {
        seen = performance.now() - spawned;
      }
    });
    child.stderr.on('data', (d) => (out += d));
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`The app did not report a cold start within 30 s:\n${out}`));
    }, 30_000);
    child.on('exit', (code) => {
      clearTimeout(timer);
      if (code !== 0 || seen === undefined) {
        reject(new Error(`The app ended with code ${code} before its window reached the engine:\n${out}`));
      } else {
        resolve(Math.round(seen));
      }
    });
  });
}

try {
  const times = [];
  for (let i = 0; i < 3; i++) {
    times.push(await once());
  }
  const median = [...times].sort((a, b) => a - b)[1];
  console.log(`Cold starts: ${times.join(', ')} ms; median ${median} ms (budget ${limit} ms).`);
  if (median > limit) {
    console.error('The median cold start is over the budget.');
    process.exit(1);
  }
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
