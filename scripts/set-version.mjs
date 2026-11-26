#!/usr/bin/env node
// Sets, or checks, the one product version in every file that holds it (RELEASING.md,
// "Versioning"): the Cargo workspace version and the workspace crates in Cargo.lock, the version of
// every npm workspace package and its pins of the other @logica packages, and package-lock.json.
//
//   node scripts/set-version.mjs <version>           write the version (the release PR does this)
//   node scripts/set-version.mjs --check <version>   fail if any file holds another version
//   node scripts/set-version.mjs --print             print the version of Cargo.toml
//
// --root <dir> works on another checkout (the tests use it). The version has no leading "v".
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-(alpha|beta|rc)\.[1-9]\d*)?$/;

function usage(message) {
  console.error(`error: ${message}`);
  console.error('usage: set-version.mjs [--root <dir>] (<version> | --check <version> | --print)');
  process.exit(2);
}

const args = process.argv.slice(2);
let root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
let mode = 'set';
let version;
while (args.length > 0) {
  const a = args.shift();
  if (a === '--root') root = resolve(args.shift() ?? usage('--root needs a directory'));
  else if (a === '--check') mode = 'check';
  else if (a === '--print') mode = 'print';
  else if (version === undefined && !a.startsWith('-')) version = a;
  else usage(`unexpected argument '${a}'`);
}

const read = (f) => readFileSync(join(root, f), 'utf8');
const write = (f, text) => writeFileSync(join(root, f), text);

// [workspace.package] version = "…" in Cargo.toml.
const WORKSPACE_VERSION = /(\[workspace\.package\][^[]*?\nversion = ")([^"]*)(")/;

const cargoToml = read('Cargo.toml');
const current = cargoToml.match(WORKSPACE_VERSION)?.[2];
if (current === undefined) usage('Cargo.toml has no [workspace.package] version');

if (mode === 'print') {
  console.log(current);
  process.exit(0);
}
if (version === undefined) usage('a version is required');
if (version.startsWith('v')) usage(`'${version}': write the version without the leading "v"`);
if (!SEMVER.test(version)) {
  usage(`'${version}' is not a release version (X.Y.Z, or X.Y.Z-alpha.N, -beta.N, -rc.N)`);
}

const npmPackages = ['packages/api/package.json', 'packages/ui/package.json', 'apps/desktop/package.json'];
const mismatches = [];
// Files that do not look as expected: fatal in every mode, and nothing is written.
const problems = [];
const note = (file, what, found) => {
  if (found !== version) mismatches.push(`${file}: ${what} is ${JSON.stringify(found)}`);
};

// Every file is read and changed in memory first; they are written only when all of them are fine.
const outputs = new Map();

// Cargo.toml
note('Cargo.toml', '[workspace.package] version', current);
outputs.set('Cargo.toml', cargoToml.replace(WORKSPACE_VERSION, `$1${version}$3`));

// Cargo.lock: the workspace's own crates are the entries without a source.
const cargoLock = read('Cargo.lock');
const lockBlocks = cargoLock.split(/(?=^\[\[package\]\]$)/m);
let crates = 0;
const newLock = lockBlocks
  .map((block) => {
    if (!block.startsWith('[[package]]') || /^source = /m.test(block)) return block;
    const m = block.match(/^name = "([^"]+)"\nversion = "([^"]*)"$/m);
    if (!m) return block;
    crates += 1;
    note('Cargo.lock', `the version of ${m[1]}`, m[2]);
    return block.replace(/^(version = ")[^"]*(")$/m, `$1${version}$2`);
  })
  .join('');
if (crates === 0) problems.push('Cargo.lock: no workspace crates found');
outputs.set('Cargo.lock', newLock);

// package.json of every npm workspace package, and its pins of the other @logica packages.
const pinSections = ['dependencies', 'devDependencies', 'peerDependencies', 'optionalDependencies'];
function setPackage(file, pkg) {
  note(file, 'version', pkg.version);
  pkg.version = version;
  for (const section of pinSections) {
    for (const [name, spec] of Object.entries(pkg[section] ?? {})) {
      if (!name.startsWith('@logica/')) continue;
      note(file, `the ${section} pin of ${name}`, spec);
      pkg[section][name] = version;
    }
  }
}
for (const file of npmPackages) {
  const text = read(file);
  const pkg = JSON.parse(text);
  setPackage(file, pkg);
  outputs.set(file, `${JSON.stringify(pkg, null, 2)}\n`);
}

// package-lock.json: the same packages under "packages".
const lock = JSON.parse(read('package-lock.json'));
for (const file of npmPackages) {
  const key = dirname(file);
  const entry = lock.packages?.[key];
  if (entry === undefined) {
    problems.push(`package-lock.json: no entry for ${key}`);
    continue;
  }
  setPackage(`package-lock.json (${key})`, entry);
}
outputs.set('package-lock.json', `${JSON.stringify(lock, null, 2)}\n`);

if (problems.length > 0) {
  for (const p of problems) console.error(`error: ${p}`);
  process.exit(1);
}
if (mode === 'set') {
  for (const [file, text] of outputs) write(file, text);
  console.log(`Version ${current} → ${version} in ${[...outputs.keys()].join(', ')}.`);
} else if (mismatches.length > 0) {
  for (const m of mismatches) console.error(`error: ${m}, not "${version}"`);
  console.error(`Set it with: node scripts/set-version.mjs ${version}`);
  process.exit(1);
} else {
  console.log(`Every file holds version ${version}.`);
}
