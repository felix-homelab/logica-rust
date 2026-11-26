#!/usr/bin/env bash
#
# Checks that a pull-request title (or a commit header) is a Conventional Commit header of at most
# 72 characters (CONTRIBUTING.md, "Commits" and "Pull requests"). With --bot, for the dependency
# bot's titles and commits, the subject may start with a capital letter and the line may be longer;
# the type and the scope are checked as for everyone.
#
# Usage: check-pr-title.sh [--bot] "<title>"
set -euo pipefail
export LC_ALL=C.UTF-8
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

bot=false
if [[ ${1-} == --bot ]]; then
  bot=true
  shift
fi
title=${1-}
header="^(${COMMIT_TYPES})(\\((${COMMIT_SCOPES})\\))?!?: [^ ]"

if [[ -z $title ]]; then
  fail "the title is empty"
  finish
fi
if ! $bot && ((${#title} > 72)); then
  fail "the title has ${#title} characters; the limit is 72"
fi
if [[ ! $title =~ $header ]]; then
  fail "the title must start with '<type>(<scope>)!: ' where type is one of" \
    "${COMMIT_TYPES//|/, } and the optional scope one of ${COMMIT_SCOPES//|/, } (got: '$title')"
else
  subject=${title#*: }
  if ! $bot && [[ ! $subject =~ ^[a-z0-9\`] ]]; then
    fail "the subject must start with a lowercase letter, a digit or a backtick (got: '$subject')"
  fi
  if [[ $subject == *. ]]; then
    fail "the subject must not end with a period"
  fi
  if [[ $subject =~ [[:space:]]$ || $subject =~ [[:space:]]{2} ]]; then
    fail "the subject has trailing or repeated whitespace"
  fi
fi
finish
