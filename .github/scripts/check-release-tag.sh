#!/usr/bin/env bash
#
# Checks a release tag before anything is built from it (RELEASING.md, "Tags", "Versioning" and
# "Changelog"):
#
# - the tag matches the release pattern and is annotated;
# - it points at a commit of <branch> (origin/main, or origin/release/X.Y for a patch from a
#   maintenance branch);
# - every file that holds the product version holds the tag's version (scripts/set-version.mjs);
# - CHANGELOG.md has a dated section for the version. A pre-release gets no section of its own; its
#   notes are the Unreleased section, which must not be empty.
#
# On success it prints that section's text, the release notes, and nothing else on stdout.
#
# Usage: check-release-tag.sh <tag> [<branch> [<repository root>]]
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

tag=${1?tag required}
branch=${2:-origin/main}
root=${3:-$(repo_root)}

if ! [[ $tag =~ $RELEASE_TAG_RE ]]; then
  fail "'$tag' is not a release tag (RELEASING.md, \"Tags\")"
  finish
fi
version=${tag#v}

if ! git -C "$root" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  fail "there is no tag $tag in $root"
  finish
fi
if [[ $(git -C "$root" cat-file -t "refs/tags/$tag") != tag ]]; then
  fail "the tag $tag is not annotated; release tags are made with 'git tag -s' (RELEASING.md, \"Tags\")"
fi
if ! git -C "$root" merge-base --is-ancestor "refs/tags/$tag^{commit}" "$branch" 2>/dev/null; then
  fail "the tag $tag does not point at a commit of $branch (RELEASING.md, \"Tags\")"
fi
if ! out=$(node "$root/scripts/set-version.mjs" --root "$root" --check "$version" 2>&1); then
  fail "the files do not hold version $version: ${out//$'\n'/; }"
fi

if [[ $version == *-* ]]; then
  section=Unreleased
  heading=$(grep -nE '^## \[Unreleased\]$' "$root/CHANGELOG.md" || true)
  want="## [Unreleased]"
else
  section=$version
  heading=$(grep -nE "^## \[${version//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}\$" "$root/CHANGELOG.md" || true)
  want="## [$version] - YYYY-MM-DD"
fi
if [[ -z $heading ]]; then
  fail "CHANGELOG.md has no section '$want' (RELEASING.md, \"Release process\")"
  finish
fi
# The section runs to the next "## " heading or to the link definitions at the end, without the
# blank lines before it (the command substitution drops those after it).
notes=$(tail -n +"$((${heading%%:*} + 1))" "$root/CHANGELOG.md" | awk '/^## |^\[[^]]+\]: /{ exit } { print }' |
  sed -e '/./,$!d')
if [[ -z $notes ]]; then
  fail "the section $section of CHANGELOG.md is empty"
fi
finish
echo "$notes"
