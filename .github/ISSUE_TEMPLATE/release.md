---
name: Release
about: Checklist for cutting a release (maintainer only)
title: "Release vX.Y.Z"
labels: "type:release"
---

Process: `RELEASING.md`, section "Release process".

- [ ] Milestone `vX.Y.Z` closed, or remaining issues moved
- [ ] `main` (or `release/X.Y`) green
- [ ] `Unreleased` complete and checked against `git log --oneline --no-merges vPREVIOUS..HEAD`
- [ ] Breaking changes marked and with upgrade notes
- [ ] Release candidate `vX.Y.Z-rc.N` tagged, its draft published as a pre-release (minor and major only)
- [ ] `docs/manual-tests.md` run on the rc installers on Windows and Linux, including the update from the previous release; macOS noted as untried if no Mac was used
- [ ] Benchmarks (`cargo bench`, `LOGICA_PERF=1 npm run test:e2e`) recorded in `docs/benchmarks.md`; any regression above 10 % against the last release explained
- [ ] Release PR `chore(release): vX.Y.Z` merged on green CI (version set, changelog section dated, links updated)
- [ ] Signed tag `vX.Y.Z` pushed; the release workflow green; draft has installers, `logica` binaries, `SHA256SUMS`
- [ ] Draft published; notes say "unsigned" and what was not tried
- [ ] Patch from a maintenance branch: changelog section merged back into `main`
- [ ] Milestone and this issue closed
