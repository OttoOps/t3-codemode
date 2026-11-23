# Contributing to t3-codemode

Bug reports, fixes, tests, documentation, ports to other platforms and new
upstream integrations are welcome. t3-codemode is a prototype with no tagged
release, so the `execute` globals, the `wf` API, the board routes and the
config format can still change.

Participation follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report
suspected vulnerabilities through the private route in the
[security policy](SECURITY.md).

## Contributor signoff

Every human author, the maintainer included, must certify their contributions
under the [Developer Certificate of Origin 1.1](https://developercertificate.org/).
Read it before adding a `Signed-off-by` trailer to each commit you author:

```sh
git commit -s -m "fix: describe the change" --trailer "Github-Issue:#123"
```

Leave out the issue trailer when no issue applies.

**Your identity.** `-s` records your name and email from Git configuration, so
check that they identify you before committing. The signoff is a public, lasting
record, and a GitHub noreply email is acceptable when it matches your commit
identity. A DCO signoff is a rights attestation. It is separate from a
cryptographic commit signature and from GitHub's Verified badge.

**What you certify.** By signing off, you certify that you may submit the work
under the project's [MIT license](../LICENSE). The Code of Conduct keeps its
CC BY-SA 4.0 license. Confirm any employer permission you need. Preserve
third-party licenses and attribution, and raise unclear rights with the
maintainer before you submit the material.

**AI-assisted contributions.** A human must review the result, its sources and
licenses, and their own authority to submit it, then authorize their own
signoff. A tool must not add someone else's certification without that
authorization.

**Co-authors.** Each human co-author must add their own signoff as well as any
`Co-authored-by` credit.

**Fixing a missing signoff.** If your latest commit lacks your signoff and you
can make the certification:

```sh
git commit --amend --no-edit --signoff
git push --force-with-lease
```

For several unsigned commits, use an interactive rebase to edit and sign only
the commits you can certify, then push with `--force-with-lease`. Coordinate
before rewriting a shared contribution branch. Keep the original authors and
their signoffs, and never sign for someone else. Published `main` history is
not rewritten.

**Before merge.** The DCO check must pass alongside CI. It checks that every
commit in the PR has a `Signed-off-by` matching its author's email; bot commits
are exempt. Commits made in
GitHub's web editor require a signoff too.

## How changes land

Pull requests merge with **Rebase and merge**: your commits are replayed onto
`main` one by one, with no merge commit, and history stays linear. So each
commit in your branch should stand on its own:
- a Conventional Commit subject,
- a signoff,
- a build that passes.

Squash fixup commits yourself before review ends.

## Releases

You do not bump the version or push tags. On every push to `main`,
[release-plz](https://release-plz.dev) opens or updates a
`chore: release vX.Y.Z` pull request that bumps `Cargo.toml` and adds the
release to `CHANGELOG.md`, from the Conventional Commit subjects since the
last tag: `feat` bumps the minor version, anything else the patch. Merging
that pull request tags the release, and the tag starts the release
workflow, which builds, signs and publishes it (ADR 0017).

## What this project will and will not take

`t3cm` stays a local, single-user service in front of T3 Code. Changes that
expose it beyond loopback by default, weaken the sandbox limits, or let
`execute` code reach the filesystem, the network or the T3 token directly will
be closed.

Keep these out of code, fixtures, docs and issue text:
- secrets, tokens and pairing codes,
- machine-specific absolute paths,
- email addresses,
- personal data,
- private repository names.

Test fixtures use made-up names such as `acme/widgets`.

## Find or propose work

Search [existing issues](https://github.com/OttoOps/t3-codemode/issues)
before opening one. Describe the problem you want to solve and how someone can
reproduce it. Discuss the approach in an issue before implementing a
substantial feature or a change to:
- the `execute` globals,
- the `wf` protocol,
- the board API,
- the config format.

Small fixes and documentation changes can go straight to a pull request.

Look for work in issues labeled
[good first issue](https://github.com/OttoOps/t3-codemode/labels/good%20first%20issue)
or [help wanted](https://github.com/OttoOps/t3-codemode/labels/help%20wanted).
Comment on an issue you plan to take so others can coordinate with you. New bug
reports and feature requests start with `needs-triage`, and maintainers handle
the triage labels.

## Development setup

**Requirements.** Use Linux, macOS or Windows, Git, and Rust 1.95 or newer. CI
runs the checks on Linux and macOS. Windows CI is paused until we have a
code-signing certificate
([#123](https://github.com/OttoOps/t3-codemode/issues/123)), so run the checks
yourself when you change Windows code. To change the installers or workflows, also
install [ShellCheck](https://www.shellcheck.net/) and
[actionlint](https://github.com/rhysd/actionlint); `deploy/install.ps1` must
run on Windows PowerShell 5.1.

**Get the code.** Fork the repository, clone your fork, and create a branch:

```sh
git clone https://github.com/YOUR_USERNAME/t3-codemode.git
cd t3-codemode
git remote add upstream https://github.com/OttoOps/t3-codemode.git
git switch -c fix/describe-the-change
```

**What needs T3.** The test suite runs against an in-process fake T3 and fake
MCP servers, so you do not need T3 Code to work on most of the project. Only a
live run of the service (`deploy/install.sh` or `cargo run -- serve`) needs a
running T3 Code server.

**Installer changes.** The release workflow's `install` job installs this run's
own archives on Linux and macOS, starts the service, checks
`t3cm status`, and uninstalls. It runs on pull requests that touch what is
built into the binary, which is `src/`, `skills/`, `build.rs` and the Cargo
files, and on pull requests that touch `deploy/`, the workflow or its fake
release host. To try `install.sh` locally without replacing your own install,
run it in a clean environment with a scratch `HOME`, so it touches neither
your binary, your unit or agent, your running service, nor your Claude Code
config:

```sh
scratch=$(mktemp -d)
archive=~/Downloads/t3cm-0.3.0-x86_64-unknown-linux-musl.tar.gz  # the archive you downloaded
env -i HOME="$scratch" PATH=/usr/bin:/bin bash deploy/install.sh --files-only \
  --archive "$archive" --sums "$(dirname "$archive")/SHA256SUMS"
```

`install.ps1` installs under `LOCALAPPDATA` and registers through `claude`, so
on Windows try it in a throwaway VM or user account rather than your own.

## Make and check your change

Keep a pull request focused on one change and match the surrounding style. For
a behavior change, add a regression test that exercises what a caller can
observe: an `execute` or `search` result, a board route, or a message a thread
receives. The files in `tests/` show the pattern.

Run the checks CI runs, from the repository root:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
shellcheck deploy/*.sh              # when you change an installer
actionlint                          # when you change a workflow
```

`cargo fmt` applies the formatting rather than just reporting it.

For documentation-only changes, check the examples and links, and say that no
code tests were needed.

## Submit a pull request

1. Use a short, imperative Conventional Commit subject of 50 characters or
   less, such as `fix: reject an empty upstream name`. When a commit addresses
   an issue, add its trailer with
   `git commit -s --trailer "Github-Issue:#123"`.
2. Push your branch to your fork and open a PR against
   `OttoOps/t3-codemode`'s `main` branch. Draft PRs are useful for work that
   needs early feedback.
3. Explain the change, link the issue, and list the checks you ran. Use
   `Closes #123` only when the PR fully addresses that issue.
4. Keep your branch current with `main` and respond to review comments. Before
   merge, the DCO check and CI must pass and review conversations must be
   resolved.

Discuss the code and its behavior respectfully. Be specific when reporting a
problem or suggesting a change, and allow time for maintainer review.

## AI review

[CodeRabbit](https://www.coderabbit.ai/) reviews pull requests that are
ready for review: when one opens ready or leaves draft, and again after each
push until it has reviewed two commits, when it pauses. It does not review
drafts. Merging into `main` needs one approving review from a reviewer with
write access, and CodeRabbit's approval counts. Only Major and Critical
findings block: it requests changes for those and approves once its threads
are resolved and it has reviewed the latest commit. Minor and Trivial
findings are advisory; fix them or reply and resolve. A push dismisses
its earlier approval. It does not review the same commit twice, so to have
it review a pull request again from scratch, a member of the OttoOps
organization comments `@coderabbitai full review`. While it is paused,
`@coderabbitai review` reviews the latest commit and `@coderabbitai resume`
turns automatic reviews back on.

Changes to the CodeRabbit configuration and to `.github/`, which holds the
CI workflows and `CODEOWNERS`, also need approval from a code owner listed
in `.github/CODEOWNERS`. CodeRabbit's approval does not count for those
paths.

Repository admins are bypass actors on the `main` ruleset and can merge over
a CodeRabbit change request. CodeRabbit's settings are in
`.coderabbit.yaml`. Its inline comments are review conversations, so resolve
each one, with a fix or a reply, before merge.
