// Runs an npm script in every workspace that has it. npm itself fails with "No workspaces found"
// while packages/ and apps/ are empty (until M1), so this says so and succeeds instead.
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const script = process.argv[2];
if (!script) {
  console.error('Usage: node scripts/run-workspaces.mjs <script>');
  process.exit(2);
}
const found = ['packages', 'apps'].flatMap((dir) =>
  existsSync(dir) ? readdirSync(dir).filter((name) => existsSync(join(dir, name, 'package.json'))) : [],
);
if (found.length === 0) {
  console.log(`No npm packages yet (the window starts in M1); nothing to ${script}.`);
  process.exit(0);
}
const result = spawnSync('npm', ['run', script, '--workspaces', '--if-present'], { stdio: 'inherit', shell: process.platform === 'win32' });
process.exit(result.status ?? 1);
