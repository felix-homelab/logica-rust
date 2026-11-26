# Releasing Logica

How Logica is versioned, how the changelog is kept, and how a release is cut, published, supported
and, if necessary, withdrawn. Branch, commit and review rules are in
[CONTRIBUTING.md](CONTRIBUTING.md). The decisions behind this file are D14 and D15 in
[doc 12](docs/12-decisions-and-risks.md).

The first release is v0.1.0: milestones M0 to M2.

## Contents

- [Versioning](#versioning)
- [Tags](#tags)
- [Changelog](#changelog)
- [Release process](#release-process)
- [Patch releases](#patch-releases)
- [Maintenance branches](#maintenance-branches)
- [Supported versions](#supported-versions)
- [Security releases](#security-releases)
- [Withdrawing a release](#withdrawing-a-release)
- [Roles](#roles)

## Versioning

Logica has **one version** for the whole product: the desktop app and its installers, the
`logica` command-line tool, every crate of the Cargo workspace and every package of the npm
workspace. Nothing is published to crates.io or npm (decision D7). The version follows
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html): `MAJOR.MINOR.PATCH`.

The version is written in the files, not taken from the tag:
`node scripts/set-version.mjs X.Y.Z` sets it in `Cargo.toml`, `Cargo.lock`, every `package.json`
(with the pins between the `@logica/` packages) and `package-lock.json`, and
`node scripts/set-version.mjs --check X.Y.Z` checks it. CI checks on every PR that all files hold
the same version, and the release build refuses a tag whose version the files do not hold.

**The stable interfaces** are the ones SemVer protects:

- **`.lacap` files** ([doc 09](docs/09-file-formats.md)): a capture saved by an earlier release
  opens in every later release of the same major.
- **The command API and MCP** ([doc 03](docs/03-architecture.md), [doc 07](docs/07-mcp-server.md)):
  the commands with their inputs, outputs and events, and the MCP tools, resources and prompts.
- **The `logica` command line**: subcommands, flags, JSON output and exit codes.

The custom protocol JSON format, the window's layout and the settings file are not stable
interfaces; a change to them is noted in the changelog but is not a MAJOR change.

| Bump | When | Examples |
|---|---|---|
| **MAJOR** | Upgrading needs action from users, or something they rely on stops working | A command, MCP tool or CLI flag removed or renamed; an input made required, or an output field removed; a new major of the command API (`API_VERSION`) or of the `.lacap` format; a `.lacap` written by an earlier release of the major no longer opens; a removed feature |
| **MINOR** | Backwards-compatible additions | New devices, decoders, importers, commands, MCP tools, CLI subcommands, optional inputs and output fields; a new minor of the command API or `.lacap`; deprecations |
| **PATCH** | Backwards-compatible fixes only | Bug and security fixes. No new features or settings |

- **Before 1.0.0** (`0.y.z`) a minor release may contain breaking changes; they are still marked as
  breaking in commits and the changelog.
- **Releases follow the milestones** of [doc 11](docs/11-roadmap.md): v0.1.0 is M0 to M2, v0.2.0
  is M3 (the DLA), v0.3.0 is M4 (FX2), v0.4.0 is M5 (formats and protocol depth), and v1.0.0 is
  M6. Patch releases come in between as needed.
- **Pre-releases** are `X.Y.Z-rc.N`, with N counting from 1; `-alpha.N` and `-beta.N` are allowed
  but not planned. Every minor and major release has at least one release candidate; after `rc.1`
  only fixes are merged for that release. Patch releases have none.
- **Updates.** A pre-release is published as a GitHub pre-release. electron-updater offers
  installed stable versions only stable releases; an installed pre-release is offered later
  pre-releases and the stable release (electron-updater's documented default for
  `allowPrerelease`). So only people who installed a release candidate on purpose run one.
- The **command API** (`API_VERSION` in `crates/engine/src/contract.rs`, "major.minor") and the
  **`.lacap` format** (`version [major, minor]`) have their own version numbers. A new major of
  either changes only with a MAJOR release.

## Tags

- **Release tags** are `v` plus the version: `v0.2.0`, `v0.2.0-rc.1`. Inside files and artifacts the
  version has no `v` (`logica-0.2.0-linux-x64`).
- Release tags are **annotated and signed** (`git tag -s`), created on a commit of `main` (or of a
  `release/<major>.<minor>` branch) whose CI is green. The tag message is the version's changelog
  section. The signing key is registered with GitHub, so the tag shows as verified.
- **Tags are immutable.** A tag is never moved or deleted, not even for a broken release; a broken
  release gets a new patch version ([Withdrawing a release](#withdrawing-a-release)).
- The release workflow builds only from tags that match the release pattern:

  ```
  ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$
  ```

## Changelog

[CHANGELOG.md](CHANGELOG.md) at the repository root is the release history for the people who use
Logica, its MCP server or its command line. It follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

**Who writes it**

- The author of a PR adds its entry under `## [Unreleased]`, **in the same PR** as the change.
- A PR without a user-visible change carries the `no-changelog` label instead. CI fails a PR that
  neither changes `CHANGELOG.md` nor carries the label.
- When cutting a release, the maintainer only moves, groups and tidies entries.

**What goes in.** Everything a user notices: new features, devices, decoders and formats; changed
behaviour; deprecations; removals; fixes; security fixes; changed defaults; steps needed when
upgrading. **What stays out:** refactoring, tests, CI, internal tooling, changes to the design
documents (their history is in git and in the ADRs).

**Sections** per version, in this order, each only if it has entries:

| Section | For | Usual commit type |
|---|---|---|
| `### Added` | New features | `feat` |
| `### Changed` | Changes to existing behaviour or defaults | `feat`, `perf` |
| `### Deprecated` | Features that will be removed; name the version that removes them | `feat`, `refactor` |
| `### Removed` | Features removed in this version | `feat!` |
| `### Fixed` | Bug fixes | `fix` |
| `### Security` | Vulnerability fixes and security-relevant changes | `fix` |

**Entries**

- One bullet per change, written for a user: what changes for them, in the present tense. Start
  with the affected part when it helps (`App:`, `Devices:`, `Decoders:`, `Files:`, `MCP:`,
  `CLI:`).
- End with the PR reference, e.g. `(#123)`. A change made in several PRs is one entry with all
  references.
- **Breaking changes** start with `**Breaking:**` and say what the user has to do.
- **Security** entries name the advisory, the severity and the affected versions.
- **Deprecated** entries name the version in which the feature is removed.
- Plain English, sentence case, as everywhere in Logica's copy.

Example:

```markdown
## [0.2.0] - 2027-03-04

### Added

- Devices: capture from the FNIRSI DLA-32 Plus. (#41, #44)

### Changed

- **Breaking:** MCP: `capture_start` takes `channels` as a list of numbers instead of a mask;
  agents' saved prompts that pass a mask must change. (#52)
```

**Versions and links**

- Version headings are `## [X.Y.Z] - YYYY-MM-DD`, the date of the tag in UTC.
- Sections are ordered by version, highest first. `## [Unreleased]` is always on top, even when it
  is empty.
- Pre-releases get no section of their own; their release notes are the `Unreleased` content at
  the time of the tag.
- Every version heading is a reference link, defined at the bottom of the file:
  `[Unreleased]: https://github.com/felix-homelab/logica-rust/compare/vX.Y.Z...HEAD` and
  `[X.Y.Z]: https://github.com/felix-homelab/logica-rust/compare/vPREVIOUS...vX.Y.Z`. The first
  release links to its tag: `…/releases/tag/v0.1.0`.
- A withdrawn version is marked `## [X.Y.Z] - YYYY-MM-DD [YANKED]` with a line saying why and
  which version to use instead.
- Released sections are not edited, except to fix mistakes or to mark a version yanked.

## Release process

For a minor or major release. A patch release is the same without step 3, starting from the
branch described in [Patch releases](#patch-releases).

1. **Open the release issue** from the *Release* issue template (`Release vX.Y.0`, label
   `type:release`). The issue holds this checklist.
2. **Check readiness.**
   - `main` is green.
   - Every issue in the `vX.Y.0` milestone is closed or moved out.
   - `Unreleased` is complete: compare it with `git log --oneline --no-merges vPREVIOUS..main`
     (every `feat` and `fix` commit maps to an entry, or was labelled `no-changelog`
     deliberately).
   - Breaking changes have upgrade notes.
3. **Release candidate.**
   - A PR from `improvement/release-vX.Y.0-rc.1`, titled `chore(release): vX.Y.0-rc.1`, runs
     `node scripts/set-version.mjs X.Y.0-rc.1` and nothing else; it carries `no-changelog`. Merge
     it on green CI.
   - Tag the merge commit and push the tag:
     `git tag -s vX.Y.0-rc.1 -m "Logica X.Y.0-rc.1"`, then `git push origin vX.Y.0-rc.1`. The
     release workflow builds the draft (step 6); publish it as it is, a pre-release.
   - Test the release candidate:
     - [docs/manual-tests.md](docs/manual-tests.md) on the rc installers, on Windows and Linux.
       Install the previous release first and install the rc over it: settings and recent files
       stay. macOS is tried if a Mac is at hand; otherwise the release notes say it was not.
     - Benchmarks: `cargo bench` and the doc 08 budgets (`LOGICA_PERF=1 npm run test:e2e`) on the
       reference machine, recorded in [docs/benchmarks.md](docs/benchmarks.md). A regression of
       more than 10 % against the last release is explained in the release issue, or fixed.
   - Fixes go to `main` as normal PRs, followed by `rc.2` the same way if needed.
4. **Release PR** from `improvement/release-vX.Y.0`, titled `chore(release): vX.Y.0`:
   - `node scripts/set-version.mjs X.Y.0`;
   - rename `## [Unreleased]` to `## [X.Y.0] - YYYY-MM-DD` and add a new empty `## [Unreleased]`
     above it;
   - update the compare links at the bottom;
   - update version references and the status line in the README and documents, if any.

   Merge it on green CI with a merge commit.
5. **Tag** the merge commit with the changelog section as the message and push it: copy the
   section's text into a file, then `git tag -s vX.Y.0 --cleanup=verbatim -F <file>` (without
   `--cleanup=verbatim`, git drops the `###` headings as comments) and
   `git push origin vX.Y.0`.
6. **Build.** The tag runs the workflow `release` (`.github/workflows/release.yml`):
   - the tag must match the release pattern, be annotated and point at a commit of `main` (or of
     `release/X.Y`); every file must hold the tag's version; `CHANGELOG.md` must have the
     version's dated section, whose text becomes the release notes. A pre-release takes them from
     `## [Unreleased]`, which must not be empty;
   - a **draft** GitHub release `Logica X.Y.Z` with those notes, marked as a pre-release for an rc
     tag;
   - the unsigned installers and update feeds from electron-builder: NSIS on Windows, AppImage on
     Linux, dmg on macOS (decision D8);
   - the standalone command-line tool `logica-<version>-<os>-<arch>` (`.exe` on Windows) for
     Linux x64, Windows x64 and macOS arm64;
   - `SHA256SUMS` over every file of the release.
7. **Publish.** Check the draft: every file is there, the notes read well. Add a short note that
   the installers are unsigned (how to open them on Windows and macOS) and what was not tried
   (for example macOS). Publish it. From then on installed copies on Windows and Linux are
   offered the update.
8. **Close.**
   - On a Windows and a Linux machine with the release candidate installed, the app offers the
     new release and updates to it.
   - Close the milestone and the release issue.

## Patch releases

- If `main` contains only fixes since the last release, the patch is released from `main` as
  above, without a release candidate.
- Otherwise the patch is released from the [maintenance branch](#maintenance-branches) of that
  minor, so that unreleased features on `main` do not ship in a patch.

## Maintenance branches

- `release/<major>.<minor>` is created from the latest tag of that minor when its first patch
  cannot come from `main`: `git switch -c release/0.2 v0.2.0`. It has the same protection as
  `main`.
- **Fix `main` first.** A fix is merged to `main` through a normal `bugfix/` PR. It is then
  backported by cherry-picking the PR's commits (`git cherry-pick -x <first>^..<last>`) in a PR
  that targets the release branch, from a branch like `bugfix/57-vcd-scope-names-0.2`. The title
  repeats the original title, and the body says `Backport of #<PR>`.
- A fix only goes directly to a release branch when the code no longer exists on `main`. The PR
  says so.
- The patch's changelog section is written on the release branch. After the release it is copied
  into `main`'s `CHANGELOG.md` through a `merge/release-0.2-into-main` PR, so `main` holds the
  complete history.
- Maintenance branches are left as they are after their last patch (not deleted, because tags
  point into them).

## Supported versions

Only the **latest release** gets fixes. The app updates itself on Windows and Linux, so the latest
release is where everyone ends up; on macOS, people download it again.

## Security releases

- Vulnerabilities are reported privately ([SECURITY.md](SECURITY.md)) and fixed in a GitHub private
  security advisory, not in a public PR.
- The fix is released as a patch of the latest release. The advisory, with a CVE where applicable,
  is published when the release is out.
- The changelog's `### Security` entry names the advisory, the severity and the affected versions.

## Withdrawing a release

When a release turns out to be broken:

1. Mark it: changelog heading `[YANKED]` with the reason, and the GitHub release's title and notes
   say "withdrawn — use vX.Y.Z+1".
2. Release a fixed patch version as soon as possible: until it is out, installed copies keep being
   offered the broken release.
3. Never delete or move the tag or the release files: installed copies and bug reports refer to
   them.

## Roles

Logica has one maintainer (decision D14), who holds every role:

- **Release manager:** merges the release PRs, creates and signs every `v*` tag, publishes the
  drafts, runs the checklist and owns the release issue.
- **Tester:** runs the manual tests and the benchmarks on the release candidate, on Windows and
  Linux, and records the results in the release issue.
