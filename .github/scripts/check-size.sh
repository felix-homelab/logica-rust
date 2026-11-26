#!/usr/bin/env bash
#
# Counts the changed lines of production code in a pull request and reports them against the size
# guidance of CONTRIBUTING.md, "Size of changes": below 400 is fine, 400–800 gives a warning (the
# PR must say why it cannot be split), more than 800 a stronger warning (split it, unless it is
# labelled `mechanical`). Until v1.0.0 the check only warns; it never fails a PR.
#
# Production code is source code (.rs, .ts, .tsx, .js, .jsx, .mjs, .cjs, .css, .sh) outside tests,
# benchmarks, examples, generated files (*.gen.ts), fixtures and CI configuration. Documents and
# lockfiles are not source code by these rules.
#
# Usage: check-size.sh <base-sha> <head-sha> "<space-separated labels>"
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

base=${1:?base commit required}
head=${2:?head commit required}
labels=" ${3-} "
warn_at=400
split_above=800

if ! numstat=$(git diff --numstat --no-renames "$base...$head"); then
  fail "cannot compare $base...$head"
  finish
fi
counted=0
files=0
while IFS=$'\t' read -r added deleted path; do
  [[ -z ${path-} || $added == - ]] && continue # nothing changed, or a binary file
  [[ $path =~ \.(rs|ts|tsx|js|jsx|mjs|cjs|css|sh)$ ]] || continue
  [[ $path =~ \.(test|spec|check)\.(ts|tsx|js|jsx|mjs)$ || $path =~ \.gen\.ts$ ]] && continue
  [[ $path =~ (^|/)(tests|test|benches|examples|e2e|fixtures|node_modules)/ ]] && continue
  [[ $path =~ ^\.github/ ]] && continue
  counted=$((counted + added + deleted))
  files=$((files + 1))
done <<<"$numstat"

summary="Production code changed: $counted lines in $files files (guidance: say why from $warn_at, split above $split_above)."
echo "$summary"
if [[ -n ${GITHUB_STEP_SUMMARY-} ]]; then
  echo "$summary" >>"$GITHUB_STEP_SUMMARY"
fi
if [[ $labels == *" mechanical "* ]]; then
  :
elif ((counted > split_above)); then
  warn "$counted lines of production code; more than $split_above should be split unless the change" \
    "is mechanical (label 'mechanical'), see CONTRIBUTING.md, 'Size of changes'"
elif ((counted >= warn_at)); then
  warn "$counted lines of production code: the PR description must say why it cannot be split" \
    "(CONTRIBUTING.md, 'Size of changes')"
fi
finish
