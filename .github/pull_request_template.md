<!--
Title: a Conventional Commit header, at most 72 characters, e.g. `feat(drivers): capture from the DLA-32 Plus`.
The merge commit on main records it. Rules: CONTRIBUTING.md.
-->

## What and why

<!-- What changes for people using Logica (or for the design), and why. -->

Closes #

## How it was tested

<!-- Commands, test names, manual checks. Include error cases and edge conditions. -->

## Not verified

<!-- Anything that could not be run or checked here, e.g. "needs real hardware", "macOS not tried". Write "Nothing" if everything was verified. -->

## Breaking changes and upgrade notes

<!-- "None", or what users must do. Breaking changes also need `!` in the title and a `BREAKING CHANGE:` footer. -->

## Checklist

- [ ] Title is a Conventional Commit header; branch name follows CONTRIBUTING.md
- [ ] One logical change, within the size guidance (or the reason is given above)
- [ ] Tests cover error cases and edge conditions, not only the happy path
- [ ] Documents updated in the same PR (or not needed)
- [ ] `CHANGELOG.md` entry under `Unreleased`, or the `no-changelog` label
- [ ] New dependencies justified in one sentence each
- [ ] Hardware outputs stay off by default and are confirmed in the app (CLAUDE.md), if touched
