---
status: accepted
date: 2026-11-23
---

# Release PRs from release-plz

## Context and problem statement

`release.yml` builds, signs, attests and publishes a release when a `v*` tag
is pushed. A release meant a hand-written PR that bumped `Cargo.toml`, then a
hand-pushed tag on the merge commit. v0.9.0 merged with no tag pushed, so no
release came out until someone noticed. The tag was a separate manual step
that nothing enforced.

## Decision drivers

- Merging the release PR is the release. No step follows it by hand.
- The version comes from the Conventional Commits since the last tag.
- `release.yml` stays the one place that builds and publishes.
- The crate is not on crates.io (`publish = false`).

## Considered options

1. release-plz, in git-only mode.
2. release-please with the `rust` release type.
3. A workflow that tags `main` when the `Cargo.toml` version changes.

## Decision outcome

Chosen: option 1, the Rust ecosystem's common choice for this.

- **The release PR.** On every push to `main`, `release-plz release-pr`
  opens or updates a `chore: release vX.Y.Z` PR that bumps `Cargo.toml` and
  `Cargo.lock` and adds the release to `CHANGELOG.md`. It is reviewed and
  merged like any other PR.
- **The release.** On the push that merges that PR, `release-plz release`
  pushes the `vX.Y.Z` tag (`release_always = false`). The tag starts
  `release.yml`, which creates the GitHub Release once the macOS binaries are
  signed, so release-plz creates no release itself
  (`git_release_enable = false`).
- **Versions.** `git_only = true` reads the current version from `v*` tags.
  A `feat` commit bumps the minor version even in `0.x`
  (`features_always_increment_minor = true`), as this project's releases
  have so far. A `fix` bumps the patch.
- **The token.** Tags and PRs that the default `GITHUB_TOKEN` creates start
  no other workflows, so the tag would not start `release.yml` and CI would
  not run on the release PR. The workflow uses a GitHub App installation
  token instead: app client ID in the `RELEASE_APP_CLIENT_ID` repository
  variable, private key in the `RELEASE_APP_PRIVATE_KEY` secret. The app
  needs Contents and Pull requests read and write. Its commits are bot
  commits, which the DCO check exempts.

### Consequences

- A release needs no local step: merge the release PR.
- The changelog lives in `CHANGELOG.md`. The GitHub Release still carries
  GitHub's generated notes.
- Option 2 would also work, but it keeps its version in a manifest file
  and, for Rust, leans on crates.io conventions this crate does not use.
  Option 3 automates the tag but leaves the version bump and changelog by
  hand.
