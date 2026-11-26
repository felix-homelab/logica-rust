#!/usr/bin/env bash
#
# Checks that a pull request changes CHANGELOG.md or carries the no-changelog label
# (RELEASING.md, "Changelog").
#
# Usage: check-changelog.sh <base-sha> <head-sha> "<space-separated labels>"
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

base=${1:?base commit required}
head=${2:?head commit required}
labels=" ${3-} "

if [[ $labels == *" no-changelog "* ]]; then
  finish
  exit 0
fi
if ! changed=$(git diff --name-only "$base...$head" --); then
  fail "cannot compare $base...$head"
  finish
fi
if ! grep -qx 'CHANGELOG.md' <<<"$changed"; then
  fail "the PR changes no CHANGELOG.md entry; add one under '## [Unreleased]', or label the PR" \
    "no-changelog if it has no user-visible change"
fi
finish
