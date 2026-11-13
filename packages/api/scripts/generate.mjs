// Generates the TypeScript types of the command API from the checked-in contract
// (schema/contract.json, written by the engine). Never edit the generated files by hand (doc 03).
// With --check, fails when the generated files are out of date.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { compile } from 'json-schema-to-typescript';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
// The contract is JSON Schema 2020-12; json-schema-to-typescript reads tuples only in the older
// form (`items` as an array), so `prefixItems` is rewritten to that.
const adapt = (node) => {
  if (Array.isArray(node)) return node.map(adapt);
  if (node === null || typeof node !== 'object') return node;
  const out = Object.fromEntries(Object.entries(node).map(([k, v]) => [k, adapt(v)]));
  if (Array.isArray(out.prefixItems)) {
    out.items = out.prefixItems;
    out.additionalItems = false;
    delete out.prefixItems;
  }
  // an object without fields is `{}` in TypeScript, which accepts anything; the engine refuses
  // unknown fields, so say "no fields" instead
  const noFields = out.properties === undefined || Object.keys(out.properties).length === 0;
  if (out.type === 'object' && out.additionalProperties === false && noFields) {
    out.tsType = 'Record<string, never>';
  }
  return out;
};
const contract = adapt(JSON.parse(readFileSync(join(root, 'schema', 'contract.json'), 'utf8')));
const names = Object.keys(contract.commands).sort();

const byCommand = (part) => ({
  type: 'object',
  additionalProperties: false,
  required: names,
  properties: Object.fromEntries(names.map((n) => [n, contract.commands[n][part]])),
});

const schema = {
  title: 'Contract',
  type: 'object',
  additionalProperties: false,
  required: ['inputs', 'outputs', 'event', 'error', 'invocation', 'historyEntry'],
  properties: {
    inputs: byCommand('input'),
    outputs: byCommand('output'),
    event: contract.event,
    error: contract.error,
    invocation: contract.invocation,
    historyEntry: contract.historyEntry,
  },
  $defs: contract.$defs,
};

const banner = `// Generated from schema/contract.json by scripts/generate.mjs. Do not edit by hand.\n// Command API version ${contract.apiVersion}.`;
const types = await compile(schema, 'Contract', {
  bannerComment: banner,
  additionalProperties: false,
  unreachableDefinitions: false,
  format: false,
});

const specs = names.map((n) => {
  const c = contract.commands[n];
  return { name: n, summary: c.summary, safety: c.safety, lease: c.lease, longRunning: c.longRunning, undo: c.undo };
});
const commands = `${banner}

export const API_VERSION = ${JSON.stringify(contract.apiVersion)};

export interface CommandSpec {
  name: string;
  summary: string;
  safety: 'read' | 'capture' | 'output';
  lease: boolean;
  longRunning: boolean;
  undo: { kind: 'query' } | { kind: 'inverse' } | { kind: 'never'; reason: string };
}

export const COMMANDS: readonly CommandSpec[] = ${JSON.stringify(specs, null, 2)};
`;

const out = [
  [join(root, 'src', 'contract.gen.ts'), types],
  [join(root, 'src', 'commands.gen.ts'), commands],
];

if (process.argv.includes('--check')) {
  const stale = out.filter(([p, text]) => !existsSync(p) || readFileSync(p, 'utf8') !== text).map(([p]) => p);
  if (stale.length) {
    console.error(`Out of date: ${stale.join(', ')}. Run \`npm run generate -w packages/api\` and commit the result.`);
    process.exit(1);
  }
} else {
  for (const [p, text] of out) writeFileSync(p, text);
}
