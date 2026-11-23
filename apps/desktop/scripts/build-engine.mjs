// Builds the engine's Node-API module (crates/node) and puts it where the app loads it from:
// native/logica.node. Release build: the app's cold start and capture speed depend on it.
import { copyFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';

const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(app, '..', '..');
const local = join(homedir(), '.cargo', 'bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo');
const cargo = process.env.CARGO ?? (existsSync(local) ? local : 'cargo');
const r = spawnSync(cargo, ['build', '--release', '-p', 'logica-node', '-p', 'logica-cli'], { cwd: repo, stdio: 'inherit' });
if (r.status !== 0) {
  console.error('The engine module did not build; see the cargo output above. Rust is set up with rustup (see README).');
  process.exit(r.status ?? 1);
}
const lib = { win32: 'logica_node.dll', darwin: 'liblogica_node.dylib' }[process.platform] ?? 'liblogica_node.so';
mkdirSync(join(app, 'native'), { recursive: true });
copyFileSync(join(repo, 'target', 'release', lib), join(app, 'native', 'logica.node'));
// the command-line tool ships with the app (electron-builder.yml)
const exe = process.platform === 'win32' ? 'logica.exe' : 'logica';
mkdirSync(join(app, 'bin'), { recursive: true });
copyFileSync(join(repo, 'target', 'release', exe), join(app, 'bin', exe));
console.log(`Engine module: native/logica.node; command-line tool: bin/${exe}`);
