#!/usr/bin/env node
// Applies the GitHub settings of CONTRIBUTING.md, "Repository settings", to
// github.com/felix-homelab/logica-rust: merge settings, security settings, the rulesets for main,
// release/* and v* tags, labels and milestones. It can be run again; it updates what exists.
//
// Without --apply it only reads the repository and prints what it would change. The maintainer
// runs it with a gh login that has admin rights on the repository.
//
// Usage: node .github/repo-settings.mjs [--apply]
import { execFileSync } from 'node:child_process';

const repo = 'felix-homelab/logica-rust';
const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && args[0] !== '--apply')) {
  console.error('usage: node .github/repo-settings.mjs [--apply]');
  process.exit(2);
}
const apply = args[0] === '--apply';

// gh <args…> [body]: runs gh and returns its output; a body is sent as JSON on stdin.
function gh(ghArgs, body) {
  const input = body === undefined ? undefined : JSON.stringify(body);
  return execFileSync('gh', body === undefined ? ghArgs : [...ghArgs, '--input', '-'], {
    input,
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'inherit'],
  });
}
const getJson = (path) => JSON.parse(gh(['api', '--paginate', '--slurp', path])).flat();

// change <description> <gh args> [body]: makes the change with --apply, prints it otherwise.
function change(what, ghArgs, body) {
  if (apply) {
    console.log(`== ${what}`);
    gh(ghArgs, body);
  } else {
    console.log(`would: ${what}`);
    console.log(`   gh ${ghArgs.map((a) => (/[\s'"]/.test(a) ? JSON.stringify(a) : a)).join(' ')}`);
    if (body !== undefined) console.log(JSON.stringify(body, null, 2).replace(/^/gm, '   '));
  }
}

// The required status checks: the pr-rules job and every job of ci.yml, by the names GitHub
// shows. A renamed job or a changed matrix changes these names; update them in the same PR.
const requiredChecks = [
  'pr-rules',
  'Repository checks - scripts, version',
  'Rust (Linux) - format, clippy, tests',
  'core and protocols build for WebAssembly',
  'Rust tests (windows-latest)',
  'Rust tests (macos-latest)',
  'npm workspace - build, lint, test',
  'Desktop app - packaged smoke test and end-to-end tests (ubuntu-latest)',
  'Desktop app - packaged smoke test and end-to-end tests (windows-latest)',
];
const GITHUB_ACTIONS_APP = 15368;
const REPOSITORY_ADMIN_ROLE = 5;

// --- Merge and security settings ---
change(
  'merge settings: merge commits (squash and rebase stay allowed), delete head branches; secret scanning and push protection',
  ['api', '-X', 'PATCH', `repos/${repo}`, '--silent'],
  {
    allow_merge_commit: true,
    allow_squash_merge: true,
    allow_rebase_merge: true,
    delete_branch_on_merge: true,
    has_issues: true,
    security_and_analysis: {
      secret_scanning: { status: 'enabled' },
      secret_scanning_push_protection: { status: 'enabled' },
    },
  },
);
change('enable private vulnerability reporting', [
  'api',
  '-X',
  'PUT',
  `repos/${repo}/private-vulnerability-reporting`,
  '--silent',
]);

// --- Rulesets ---
const branchRules = [
  { type: 'deletion' },
  { type: 'non_fast_forward' },
  {
    type: 'pull_request',
    parameters: {
      required_approving_review_count: 0,
      dismiss_stale_reviews_on_push: false,
      require_code_owner_review: false,
      require_last_push_approval: false,
      required_review_thread_resolution: true,
      allowed_merge_methods: ['merge', 'squash', 'rebase'],
    },
  },
  {
    type: 'required_status_checks',
    parameters: {
      strict_required_status_checks_policy: true,
      do_not_enforce_on_create: false,
      required_status_checks: requiredChecks.map((context) => ({ context, integration_id: GITHUB_ACTIONS_APP })),
    },
  },
];
const rulesets = [
  {
    name: 'main',
    target: 'branch',
    enforcement: 'active',
    bypass_actors: [],
    conditions: { ref_name: { include: ['~DEFAULT_BRANCH'], exclude: [] } },
    rules: branchRules,
  },
  {
    name: 'release',
    target: 'branch',
    enforcement: 'active',
    bypass_actors: [],
    conditions: { ref_name: { include: ['refs/heads/release/*'], exclude: [] } },
    rules: branchRules,
  },
  {
    // only repository admins (the maintainer) create v* tags; nobody moves or deletes them
    name: 'release-tags',
    target: 'tag',
    enforcement: 'active',
    bypass_actors: [{ actor_id: REPOSITORY_ADMIN_ROLE, actor_type: 'RepositoryRole', bypass_mode: 'always' }],
    conditions: { ref_name: { include: ['refs/tags/v*'], exclude: [] } },
    rules: [{ type: 'creation' }, { type: 'update' }, { type: 'deletion' }],
  },
];
const existingRulesets = getJson(`repos/${repo}/rulesets`);
for (const ruleset of rulesets) {
  const found = existingRulesets.find((r) => r.name === ruleset.name);
  if (found) {
    change(`update ruleset '${ruleset.name}' (${found.id})`, ['api', '-X', 'PUT', `repos/${repo}/rulesets/${found.id}`, '--silent'], ruleset);
  } else {
    change(`create ruleset '${ruleset.name}'`, ['api', '-X', 'POST', `repos/${repo}/rulesets`, '--silent'], ruleset);
  }
}

// --- Labels: GitHub's default labels are replaced by these (CONTRIBUTING.md) ---
const labels = [
  ['type:feature', '0e8a16', 'New or changed behaviour'],
  ['type:bug', 'd73a4a', 'Something does not work as it should'],
  ['type:improvement', 'c5def5', 'Refactoring, documentation, tooling; no behaviour change'],
  ['type:security', 'b60205', 'A vulnerability or a security-relevant change'],
  ['type:docs', '0075ca', 'Documents only'],
  ['type:release', '5319e7', 'Cutting a release (RELEASING.md)'],
  ['area:core', 'fbca04', 'crates/core: capture store, samples, LOD'],
  ['area:protocols', 'fbca04', 'crates/protocols: decoders, detection, analysis'],
  ['area:drivers', 'fbca04', 'crates/drivers: devices and transports'],
  ['area:engine', 'fbca04', 'crates/engine: the command API'],
  ['area:fileio', 'fbca04', 'crates/fileio: importers, exporters, .lacap'],
  ['area:mcp', 'fbca04', 'crates/mcp: the MCP server and logica mcp'],
  ['area:node', 'fbca04', 'crates/node: the engine as a Node-API module'],
  ['area:cli', 'fbca04', 'crates/cli: the logica command'],
  ['area:desktop', 'fbca04', 'apps/desktop: the Electron app'],
  ['area:ui', 'fbca04', 'packages/ui: design tokens and components'],
  ['area:api', 'fbca04', 'packages/api: the generated TypeScript contract'],
  ['area:vla', 'fbca04', 'tools/virtual-analyser'],
  ['area:ops', 'fbca04', 'CI, release tooling, the repository'],
  ['breaking', 'b60205', 'Needs action from users; marked in the changelog'],
  ['no-changelog', 'ededed', 'No user-visible change; no CHANGELOG entry needed'],
  ['mechanical', 'ededed', 'Rename, move or generated change; no size warning'],
  ['needs-adr', 'd4c5f9', 'Needs an architecture decision record first'],
  ['needs-hardware', 'd4c5f9', 'Can only be verified with a real analyser'],
];
const defaultLabels = ['bug', 'documentation', 'duplicate', 'enhancement', 'good first issue', 'help wanted', 'invalid', 'question', 'wontfix'];
const existingLabels = new Map(getJson(`repos/${repo}/labels?per_page=100`).map((l) => [l.name, l]));
for (const [name, color, description] of labels) {
  const l = existingLabels.get(name);
  if (l && l.color === color && l.description === description) continue;
  change(`${l ? 'update' : 'create'} label ${name}`, ['label', 'create', name, '--repo', repo, '--color', color, '--description', description, '--force']);
}
for (const name of defaultLabels) {
  if (existingLabels.has(name)) change(`delete GitHub's default label '${name}'`, ['label', 'delete', name, '--repo', repo, '--yes']);
}

// --- Milestones: the releases of RELEASING.md, "Versioning" ---
const milestones = [
  ['v0.1.0', 'M0 to M2: files, decoders, MCP, CLI, the desktop app, live capture with the virtual analyser'],
  ['v0.2.0', 'M3: FNIRSI DLA-32 Plus / DLA-16 Plus'],
  ['v0.3.0', 'M4: FX2 / fx2lafw'],
  ['v0.4.0', 'M5: formats and protocol depth'],
  ['v1.0.0', 'M6: release 1.0'],
];
const existingMilestones = new Set(getJson(`repos/${repo}/milestones?state=all&per_page=100`).map((m) => m.title));
for (const [title, description] of milestones) {
  if (!existingMilestones.has(title)) {
    change(`create milestone ${title}`, ['api', '-X', 'POST', `repos/${repo}/milestones`, '--silent'], { title, description });
  }
}

if (!apply) console.log('\nNothing was changed. Run again with --apply to apply the above.');
