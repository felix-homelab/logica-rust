#!/usr/bin/env bash
#
# Tests of the repository checks in this directory and of scripts/set-version.mjs, run by CI (job
# "Repository checks") and by hand: bash .github/scripts/test-checks.sh
set -euo pipefail
unset GITHUB_ACTIONS GITHUB_STEP_SUMMARY

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
# The test repositories stay in the temporary directory (set TMPDIR to choose it); CI runners are
# thrown away after the job.
tmp=$(mktemp -d)

passed=0
failed=0
# expect <ok|fail> <description> <command…>: runs the command and compares its exit status.
expect() {
  local want=$1 what=$2
  shift 2
  local got=ok
  if ! out=$("$@" 2>&1); then got=fail; fi
  if [[ $got == "$want" ]]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $what: expected $want, got $got"
    sed 's/^/    /' <<<"$out"
  fi
}
# expect_output <description> <pattern> <command…>: the command succeeds and its output matches.
expect_output() {
  local what=$1 pattern=$2
  shift 2
  local out
  if out=$("$@" 2>&1) && grep -qE -- "$pattern" <<<"$out"; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $what: output does not match /$pattern/"
    sed 's/^/    /' <<<"$out"
  fi
}

# A git repository of its own for each test, with a fixed identity.
new_repo() {
  local dir=$tmp/$1
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.name Tester
  git -C "$dir" config user.email tester@example.org
  git -C "$dir" config commit.gpgsign false
  git -C "$dir" config tag.gpgsign false
  echo "$dir"
}
# inrepo <dir> <command…>: runs the command inside a test repository.
inrepo() { (cd "$1" && shift && "$@"); }
commit() { # commit <dir> <subject> [<file> <content>]
  local dir=$1 subject=$2 file=${3:-f$RANDOM$RANDOM.txt} content=${4:-$RANDOM$RANDOM}
  mkdir -p "$(dirname "$dir/$file")"
  printf '%s\n' "$content" >>"$dir/$file"
  git -C "$dir" add -A
  git -C "$dir" commit -q -m "$subject"
}

title=$here/check-pr-title.sh
branch=$here/check-branch-name.sh
commits=$here/check-commits.sh
changelog=$here/check-changelog.sh
size=$here/check-size.sh
release=$here/check-release-tag.sh

# --- PR titles and commit headers ---
expect ok 'plain title' "$title" 'feat: open sigrok files'
expect ok 'title with scope and breaking mark' "$title" 'feat(engine)!: rename capture.open to file.open'
expect ok 'subject starting with a backtick' "$title" 'fix(cli): `logica open` waits for the app'
expect ok 'subject starting with a digit' "$title" 'chore(release): 0.2.0 changelog'
expect ok 'exactly 72 characters' "$title" "docs: $(printf 'a%.0s' {1..66})"
expect fail 'empty title' "$title" ''
expect fail '73 characters' "$title" "docs: $(printf 'a%.0s' {1..67})"
expect fail 'sentence-style title' "$title" 'Capture tabs in the same style as the panel tabs'
expect fail 'unknown type' "$title" 'feature: open sigrok files'
expect fail 'unknown scope' "$title" 'feat(window): open sigrok files'
expect fail 'capital subject' "$title" 'feat: Open sigrok files'
expect fail 'trailing period' "$title" 'feat: open sigrok files.'
expect fail 'no space after colon' "$title" 'feat:open sigrok files'
expect fail 'repeated spaces' "$title" 'feat: open  sigrok files'
expect ok 'bot: capital subject and long line' "$title" --bot \
  "build(deps): Bump the npm group across 1 directory with 12 updates and some more words to be long"
expect fail 'bot: type still checked' "$title" --bot 'Bump electron from 45 to 46'

# --- branch names ---
expect ok 'feature branch' "$branch" feature/42-dla-driver main
expect ok 'bugfix branch with dots' "$branch" bugfix/7-fix-v0.2.0-crash main
expect ok 'improvement branch' "$branch" improvement/release-rules main
expect ok 'merge branch' "$branch" merge/release-0.2-into-main main
expect ok 'backport to a release branch' "$branch" bugfix/7-crash-0.2 release/0.2
expect ok 'dependency bot' "$branch" dependabot/cargo/serde-1.0.300 main
expect fail 'feature without issue' "$branch" feature/dla-driver main
expect fail 'improvement with issue-like uppercase' "$branch" improvement/Release-rules main
expect fail 'tmp branch' "$branch" tmp/s3-dla-spike main
expect fail 'release branch as head' "$branch" release/0.2 main
expect fail 'unknown prefix' "$branch" fix/crash main
expect fail 'double slash' "$branch" improvement/a/b main
expect fail 'longer than 60' "$branch" "improvement/$(printf 'a%.0s' {1..50})" main
expect fail 'base is not main' "$branch" improvement/x develop
expect ok 'stacked on an improvement branch' "$branch" improvement/readme improvement/release-rules
expect ok 'stacked on a feature branch' "$branch" feature/43-dla-capture feature/42-dla-usb
expect fail 'stacked on a tmp branch' "$branch" improvement/x tmp/spike
expect fail 'stacked on an invalid feature branch' "$branch" improvement/x feature/no-issue
expect fail 'base release branch without minor' "$branch" improvement/x release/1

# --- commit headers of a branch ---
r=$(new_repo commits)
commit "$r" 'chore: start'
base=$(git -C "$r" rev-parse HEAD)
git -C "$r" switch -q -c improvement/x
commit "$r" 'feat(engine): add a thing'
commit "$r" 'test: cover the thing'
expect ok 'good commit headers' inrepo "$r" "$commits" "$base" HEAD
git -C "$r" switch -q main
commit "$r" 'Sentence style on main'
git -C "$r" switch -q improvement/x
git -C "$r" merge -q --no-edit main
expect ok 'merge commits and commits of the base are not checked' inrepo "$r" "$commits" main HEAD
commit "$r" 'fixup! feat(engine): add a thing'
expect fail 'fix-up commit' inrepo "$r" "$commits" main HEAD
git -C "$r" reset -q --soft HEAD~1 && git -C "$r" commit -q -m 'Bad header'
expect fail 'sentence-style commit' inrepo "$r" "$commits" main HEAD
git -C "$r" reset -q --soft HEAD~1
git -C "$r" -c user.email='49699333+dependabot[bot]@users.noreply.github.com' commit -q -m \
  'build(deps): Bump serde from 1.0.1 to 1.0.2 in the cargo group with a very long subject line'
expect ok 'bot commit with capital subject' inrepo "$r" "$commits" main HEAD
expect fail 'unknown base' inrepo "$r" "$commits" 0000000000000000000000000000000000000000 HEAD

# --- changelog ---
r=$(new_repo changelog)
commit "$r" 'chore: start' CHANGELOG.md '# Changelog'
base=$(git -C "$r" rev-parse HEAD)
commit "$r" 'feat: thing' src/a.rs 'fn a() {}'
expect fail 'no changelog entry, no label' inrepo "$r" "$changelog" "$base" HEAD 'type:feature'
expect ok 'no changelog entry, labelled' inrepo "$r" "$changelog" "$base" HEAD 'type:feature no-changelog'
expect fail 'label that only contains the word' inrepo "$r" "$changelog" "$base" HEAD 'no-changelog-please'
commit "$r" 'docs: entry' CHANGELOG.md '- Thing.'
expect ok 'changelog entry' inrepo "$r" "$changelog" "$base" HEAD ''

# --- size ---
r=$(new_repo size)
commit "$r" 'chore: start'
base=$(git -C "$r" rev-parse HEAD)
commit "$r" 'test: many tests' crates/core/tests/big.rs "$(seq 1 900)"
commit "$r" 'feat: generated' packages/api/src/commands.gen.ts "$(seq 1 900)"
commit "$r" 'feat: ui test' packages/ui/test/a.test.tsx "$(seq 1 900)"
expect_output 'tests and generated files are not counted' 'changed: 0 lines in 0 files' \
  inrepo "$r" "$size" "$base" HEAD ''
commit "$r" 'feat: code' crates/core/src/big.rs "$(seq 1 500)"
expect_output 'production code is counted, and warns' 'warning: 500 lines' \
  inrepo "$r" "$size" "$base" HEAD ''
commit "$r" 'feat: more code' apps/desktop/src/x.tsx "$(seq 1 400)"
expect_output 'above 800 only warns' 'should be split' inrepo "$r" "$size" "$base" HEAD ''
expect ok 'above 800 never fails' inrepo "$r" "$size" "$base" HEAD ''
if out=$(cd "$r" && "$size" "$base" HEAD 'mechanical' 2>&1) && ! grep -q warning <<<"$out"; then
  passed=$((passed + 1))
else
  failed=$((failed + 1))
  echo "FAIL: a mechanical PR gets no warning"
fi

# --- set-version.mjs and release tags ---
r=$(new_repo release)
cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/package-lock.json" "$r/"
for f in packages/api/package.json packages/ui/package.json apps/desktop/package.json scripts/set-version.mjs; do
  mkdir -p "$r/$(dirname "$f")"
  cp "$repo/$f" "$r/$f"
done
setv() { node "$r/scripts/set-version.mjs" --root "$r" "$@"; }
setv 1.4.0 >/dev/null
expect ok 'set-version writes every file' setv --check 1.4.0
expect_output 'set-version prints the version' '^1\.4\.0$' setv --print
expect fail 'set-version check finds another version' setv --check 1.4.1
expect fail 'set-version refuses a leading v' setv v1.4.1
expect fail 'set-version refuses a non-release version' setv 1.4
expect fail 'set-version refuses other pre-release names' setv 1.4.0-dev.1
setv 1.5.0-rc.2 >/dev/null
expect ok 'set-version accepts a pre-release' setv --check 1.5.0-rc.2
expect_output 'apps/desktop pins the other packages' '"@logica/api": "1\.5\.0-rc\.2"' cat "$r/apps/desktop/package.json"
cp "$r/package-lock.json" "$tmp/lock.json"
node -e 'const f=process.argv[1];const l=require(f);delete l.packages["packages/ui"];require("fs").writeFileSync(f,JSON.stringify(l))' "$r/package-lock.json"
expect fail 'set-version refuses a lockfile without a workspace package' setv 1.6.0
cp "$tmp/lock.json" "$r/package-lock.json"
expect ok 'and writes nothing then' setv --check 1.5.0-rc.2

cat >"$r/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

### Added

- Pre-release thing. (#9)

## [1.4.0] - 2027-01-02

### Fixed

- A fix. (#8)

## [1.3.0] - 2026-12-01

- Old.

[Unreleased]: https://example.org/compare/v1.4.0...HEAD
[1.4.0]: https://example.org/compare/v1.3.0...v1.4.0
EOF
setv 1.4.0 >/dev/null
git -C "$r" add -A && git -C "$r" commit -q -m 'chore(release): v1.4.0'
git -C "$r" tag -a v1.4.0 -m v1.4.0
git -C "$r" update-ref refs/remotes/origin/main HEAD
expect_output 'release notes are the dated section' '^### Fixed' "$release" v1.4.0 origin/main "$r"
notes=$("$release" v1.4.0 origin/main "$r")
if [[ $notes == $'### Fixed\n\n- A fix. (#8)' ]]; then passed=$((passed + 1)); else
  failed=$((failed + 1))
  echo "FAIL: release notes are exactly the section, got:"
  sed 's/^/    /' <<<"$notes"
fi
expect fail 'not a release tag' "$release" v1.4 origin/main "$r"
expect_output 'a missing tag says so' 'no tag v9\.9\.9' bash -c "'$release' v9.9.9 origin/main '$r' || true"
expect fail 'unknown pre-release name' "$release" v1.4.0-pre.1 origin/main "$r"
git -C "$r" tag -d v1.4.0 >/dev/null
git -C "$r" tag v1.4.0
expect fail 'lightweight tag' "$release" v1.4.0 origin/main "$r"
git -C "$r" tag -a v1.3.0 -m v1.3.0
expect fail 'files hold another version' "$release" v1.3.0 origin/main "$r"
setv 1.5.0-rc.1 >/dev/null
git -C "$r" add -A && git -C "$r" commit -q -m 'chore(release): v1.5.0-rc.1'
git -C "$r" tag -a v1.5.0-rc.1 -m v1.5.0-rc.1
expect fail 'tag not on the branch' "$release" v1.5.0-rc.1 origin/main "$r"
git -C "$r" update-ref refs/remotes/origin/main HEAD
expect_output 'a pre-release takes the Unreleased section' 'Pre-release thing' "$release" v1.5.0-rc.1 origin/main "$r"
setv 1.6.0 >/dev/null
git -C "$r" add -A && git -C "$r" commit -q -m 'chore(release): v1.6.0'
git -C "$r" tag -a v1.6.0 -m v1.6.0
git -C "$r" update-ref refs/remotes/origin/main HEAD
expect fail 'no dated section for the version' "$release" v1.6.0 origin/main "$r"
sed -i 's/^- Pre-release thing. (#9)$//; s/^### Added$//' "$r/CHANGELOG.md"
setv 1.6.0-rc.1 >/dev/null
git -C "$r" add -A && git -C "$r" commit -q -m 'chore(release): v1.6.0-rc.1'
git -C "$r" tag -a v1.6.0-rc.1 -m v1.6.0-rc.1
git -C "$r" update-ref refs/remotes/origin/main HEAD
expect fail 'empty Unreleased section for a pre-release' "$release" v1.6.0-rc.1 origin/main "$r"

echo "$passed passed, $failed failed"
((failed == 0))
