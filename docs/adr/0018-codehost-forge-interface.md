---
status: accepted
date: 2026-11-24
---

# A code host interface under the reconcile, wf.pr and wf.merge

## Context and problem statement

Issue #298 asks t3cm to follow GitLab merge requests the way it follows
GitHub pull requests. Today the reconcile, `wf.pr` and `wf.merge` each build
their own GitHub GraphQL queries and read the GitHub JSON that comes back.
The only interface under them is `trait GitHub`, a raw GraphQL call. A
GitLab version of any of the three would mean a second copy of its logic, or
GitLab branches inside it.

This record sets the interface those three callers use instead, and how the
GitHub code becomes its first implementation. The GitLab implementation, its
config and its store keys come in later pull requests of #298.

## Decision drivers

- The callers decide by state, as ADR 0002 says. They should decide on the
  same plain data from any forge, never on one forge's JSON.
- The existing reconcile, merge, `wf.pr` and chaos suites must pass
  unchanged. They are the proof that this step changes no behavior.
- The GitHub calls stay as they are: one batched query per 100 tracked pull
  requests, the same mutations, and the same `expectedHeadOid` guard.
- `FakeGitHub` is also the fake for issues, flow and labels, which keep
  using `trait GitHub`.

## Considered options

For the interface:

1. A trait at the level of what the callers need: read, merge, delete the
   branch, and the URL.
2. Keep `trait GitHub` and add a second raw trait for GitLab, with each caller
   matching on the forge.

For the fake:

1. Make `FakeGitHub` a fake of the new trait.
2. Keep `FakeGitHub` as a fake GraphQL server, and run the real GitHub
   implementation against it.

## Decision outcome

Chosen: interface option 1 and fake option 2.

- **The types.** `src/codehost.rs` holds `Forge`, which is `Github` or
  `Gitlab` with its host, `ChangeRef { forge, repo, number }`, and `Change`,
  the plain record the reconcile's `Obs` used to build from JSON. It carries the
  state, draft flag, head, node id, merge state, review decision, check
  rollup, queue flags and position, merge commit, end time, queue removal,
  default merge method and closing issue. The merge state keeps GitHub's
  `mergeStateStatus` words. GitLab maps its `detailed_merge_status` onto them,
  as the issue lists.
- **The trait.** `CodeHost` has five methods:
  - `read(&[ChangeRef])` batches the status read for the reconcile. It
    returns one `Read` per ref: found, not found, or failed with a message,
    so one missing pull request never fails the batch.
  - `view(&ChangeRef)` reads one change in full for `wf.merge` and `wf.pr`.
    The full read adds the description, the first 100 changed files, and the
    reviews that request changes.
  - `merge(ref, id, head, landing)` either queues the change or merges it
    with a method, guarded by the head. It returns queued, merged or refused.
    GitLab answers a merge request on a project with merge trains by queueing
    it, so the callers handle either answer from either call.
  - `delete_branch(ref)` reads the branch and deletes it only when the change
    merged, the branch is not on a fork, and its tip is the merged head. It
    returns what it found, and the caller words the result.
  - `url(ref)` gives the change's web URL.
- **Readiness stays above the trait.** `ready`, `check_state` and `checks` in
  `src/merge.rs` judge a `Change`. Only the conversion of GitHub's check
  contexts into the rollup lives in the GitHub implementation, so ADR 0003's
  `UNSTABLE` rule applies to every forge.
- **The GitHub implementation.** `GithubHost` in `src/github_host.rs` wraps
  `trait GitHub` and holds every pull request query and mutation that
  `src/merge.rs`, `src/reconcile.rs`, `src/wf_pr.rs` and `src/merge_answer.rs`
  used to build. It turns away a ref on another forge. `Wf` keeps its
  `github` field for issues and flow, and gains a `host` field built from the
  same `GitHub`.
- **The fake.** `FakeGitHub` stays a fake GraphQL server, and the test
  harness runs `GithubHost` against it. A fake of the trait would skip the
  GitHub implementation, which is the code this change moves, and the suites
  set GitHub pull request JSON and assert on GraphQL operations and
  variables. The GitLab pull request brings a fake `glab api` at the same
  level.

Interface option 2 leaves every caller reading forge JSON, and the next
forge would add one more branch to each of them.

### Consequences

- Good: the reconcile, `wf.pr` and `wf.merge` read no GitHub JSON. A GitLab
  implementation of `CodeHost` reaches all three without touching their logic.
- Good: the existing suites cover the GitHub implementation end to end.
- Bad: `wf.merge` now uses the full read, so each of its reads also fetches
  the description, up to 100 files and up to 100 reviews. That costs a few
  GraphQL points per merge, against the 5,000-point hourly budget ADR 0002
  describes. The batched reconcile read is unchanged.
- Bad: `wf.pr` shares that query, so an error on a field only `wf.merge`
  needs, such as `viewerDefaultMergeMethod`, sends `wf.pr` down its
  read-failure path to the ticket.
- Neutral: store keys, the `github.com` URL parsing in `pr_refs`, and the
  "on GitHub" wording stay GitHub-only until the GitLab pull requests.
