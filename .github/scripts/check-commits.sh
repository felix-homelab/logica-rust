#!/usr/bin/env bash
#
# Checks the header of every non-merge commit in <base>..<head>. Pull requests are merged with a
# merge commit, so every branch commit reaches main and must be a Conventional Commit header of at
# most 72 characters (CONTRIBUTING.md, "Commits"). Fix-up commits must be folded in before merging.
# Commits of the dependency bot may start their subject with a capital letter and be longer.
#
# Usage: check-commits.sh <base-sha> <head-sha>
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

base=${1:?base commit required}
head=${2:?head commit required}
title_check="$(dirname "${BASH_SOURCE[0]}")/check-pr-title.sh"

if ! commits=$(git rev-list --no-merges "$base..$head"); then
  fail "cannot list the commits of $base..$head"
  finish
fi
for c in $commits; do
  subject=$(git log -1 --format='%s' "$c")
  if [[ $subject =~ ^(fixup|squash|amend)! ]]; then
    fail "commit ${c:0:12} ('$subject') is a fix-up commit; fold it in before merging with" \
      "'git rebase --autosquash $base'"
    continue
  fi
  bot=()
  if [[ $(git log -1 --format='%ae' "$c") == "$BOT_EMAIL" ]]; then
    bot=(--bot)
  fi
  if ! out=$("$title_check" "${bot[@]}" "$subject" 2>&1); then
    fail "commit ${c:0:12} ('$subject'): ${out//$'\n'/; }"
  fi
done
finish
