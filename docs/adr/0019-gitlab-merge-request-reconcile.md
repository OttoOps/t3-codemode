---
status: accepted
date: 2026-11-24
---

# Reconcile GitLab merge requests through glab and forge-qualified keys

## Context and problem statement

Issue #298 reported a GitLab merge request that merged through a merge train
while its ticket and worker sat settled, and nobody heard about it. ADR 0018
put a `CodeHost` interface under the reconcile, `wf.pr` and `wf.merge`, with
GitHub as its only implementation. This record covers the GitLab read path
behind it, the config that turns it on, and how the store tells a GitLab merge
request from a GitHub pull request with the same `owner/name#n`.

`wf.pr` and `wf.merge` for GitLab come in later pull requests of #298.

## Decision drivers

- t3cm holds no tokens. It reads GitHub through `gh` as the service's user,
  and GitLab should work the same way.
- A merge train swaps the merge request's head pipeline for the train's,
  whose commit never matches the merge request's head.
- A squash merge leaves `merge_commit_sha` null. GraphQL on GitLab 19.5 has
  no `squashCommitSha`.
- `merge_waits`, `pr_seen` and `pr_events` key on `(repo, pr)`. Rows written
  by earlier versions must keep working.
- A broken `glab` must not stop the GitHub checks.

## Considered options

For the store keys:

1. Add a forge or host column to the three tables, with a migration.
2. Key a GitLab merge request by `<host>/<full path>`, and leave the GitHub
   key as `owner/name`.

For the GitLab reads:

1. REST only, one call per merge request.
2. One batched GraphQL query, with REST for the fields GraphQL lacks.

## Decision outcome

Chosen: keys option 2 and reads option 2.

- **Keys.** `ChangeRef::key()` gives `owner/name` for GitHub and
  `<host>/<group>/.../<project>` for GitLab, lowercased, and
  `ChangeRef::of_key` reads it back. A GitHub key has exactly two segments and
  a GitLab key at least three, so the two never collide, and the rows earlier
  versions wrote are already valid keys. No migration rewrites them.
  `Tracked`, `LinkPr`, `MergeWait`, `CardPr` and a link's `gate_pr` all carry
  the key in their `repo` field.
- **Config.** A user-level `forges` map, next to `trackers`, lists each GitLab
  host with `kind: "gitlab"` and the `cli` to run, `glab` by default.
  github.com is built in. A host that is not listed keeps the behavior from
  before: nothing tracks its merge requests.
- **Parsing.** `change_refs` finds GitHub pull request links and
  `https://<host>/<path>/-/merge_requests/<iid>` links on listed hosts.
  `pr_refs` stays GitHub-only, and `parse_ref` still turns merge requests
  away as issues.
- **The wire.** `trait GitLab` has a GraphQL call and a REST `GET`, each run
  as `<cli> api --hostname <host>` with prompts off and a 30-second deadline.
  REST runs with `--include`, so the caller sees the HTTP status.
  `FakeGitLab` fakes it at that level, and the suites run the real
  `GitlabHost` against it.
- **The read.** One aliased GraphQL query per 10 merge requests on a host
  reads `state draft diffHeadSha detailedMergeStatus mergedAt squashOnMerge
  mergeCommitSha headPipeline { status ref } mergeTrainCar { status index }`.
  GitLab caps query complexity, so each query holds 10 merge requests. Then:
  - an open merge request gets `GET .../merge_requests/:iid/pipelines`, and
    readiness takes the newest pipeline at the head whose ref is not the
    train's (`.../train`). `success` passes, `skipped` and `manual` settle,
    the waiting and running states are pending, and the rest fail.
  - a merged one with no `mergeCommitSha`, and every closed one, gets
    `GET .../merge_requests/:iid` for `merge_commit_sha ?? squash_commit_sha`
    and `closed_at`. The close time keys the close event, so it comes from
    GitLab, which gives every pass the same key.
- **Merge state.** `detailedMergeStatus` maps onto `mergeStateStatus` words:
  `mergeable` to `CLEAN`, `conflict` to `DIRTY`, `need_rebase` to `BEHIND`,
  `draft_status` to `DRAFT`, the still-computing statuses to `UNKNOWN`, and
  everything else to `BLOCKED`. `ci_still_running` and `ci_must_pass` land on
  `BLOCKED`, as GitHub reports pending required checks.
- **The train.** A car in `idle`, `fresh`, `stale` or `merging` is queued,
  and its position is `index + 1`. GitLab records no reason a car left, so
  `removal` stays empty and an exit falls to ADR 0002's no-reason path.
- **Routing.** `Router` sends each ref to its forge's host. `GitlabHost` turns
  a failed call into a failed read for its own merge requests and never
  fails the batch, so a missing `glab` or an expired sign-in leaves GitHub
  checked.
- **Wording.** The reconcile's messages name the forge, so a merge request
  "merged on GitLab", and its events and feed notes come `from=gitlab`.

Keys option 1 changes every store call and its callers for no gain over a key
that already holds the host. Reads option 1 costs one call per merge request
per pass where GraphQL batches them.

### Consequences

- Good: a merge request that merges or closes, by train, by hand, squashed or
  not, wakes its ticket once with the commit on the target branch.
- Good: the PR chaos suite runs against GitLab as well as GitHub.
- Bad: an open merge request costs a REST call per pass, and a closed one
  costs one on every pass while something still tracks it.
- Bad: a host removed from `forges` leaves its rows failing to read until
  nothing tracks them.
- Neutral: `view`, `merge` and `delete_branch` refuse GitLab refs until the
  `wf.pr` and `wf.merge` pull requests, so the GitLab chaos run leaves out the
  ops that call them.
