# Shared helpers for the repository checks in this directory (CONTRIBUTING.md, RELEASING.md).
# Sourced, not executed.
# shellcheck shell=bash disable=SC2034 # the constants are used by the scripts that source this

# fail prints an error (as a GitHub Actions annotation when running in CI) and records it.
failures=0
fail() {
  if [[ ${GITHUB_ACTIONS-} == true ]]; then
    echo "::error::$*"
  else
    echo "error: $*" >&2
  fi
  failures=$((failures + 1))
}

# warn prints a warning that does not fail the check.
warn() {
  if [[ ${GITHUB_ACTIONS-} == true ]]; then
    echo "::warning::$*"
  else
    echo "warning: $*" >&2
  fi
}

# finish exits non-zero if fail was called.
finish() {
  if ((failures > 0)); then
    exit 1
  fi
}

# repo_root prints the top-level directory of the git work tree that contains this script.
repo_root() {
  git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel
}

# The commit types and scopes of CONTRIBUTING.md, "Commits". The scopes are the area: labels plus
# adr, deps and release.
COMMIT_TYPES='feat|fix|perf|refactor|docs|test|build|ci|chore|revert'
COMMIT_SCOPES='core|protocols|drivers|engine|fileio|mcp|node|cli|desktop|ui|api|vla|ops|adr|deps|release'

# The author address of the dependency bot's commits.
BOT_EMAIL='49699333+dependabot[bot]@users.noreply.github.com'

# The release pattern of RELEASING.md, "Tags".
RELEASE_TAG_RE='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'
