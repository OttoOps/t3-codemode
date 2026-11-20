---
status: accepted
date: 2026-11-20
---

# Merge an UNSTABLE pull request only when no check failed

## Context and problem statement

`wf.merge` and the GitHub reconcile ([ADR 0002](0002-github-reconcile.md))
decide whether a pull request is ready from its `mergeStateStatus`. Both
treated `CLEAN`, `HAS_HOOKS` and `UNSTABLE` as mergeable, the states GitHub's
own merge button accepts. `UNSTABLE` means every required check passed and at
least one other check did not, so a failing optional check let the board queue
or merge a pull request.

The ticket skill reads the checks before it calls `wf.merge` and sends a
failing one back to the worker, but the reconcile reads none. When a pull
request that `wf.merge` saved as waiting later turned `UNSTABLE` because an
optional check failed, the next reconcile pass queued it with nobody looking.

Refusing every `UNSTABLE` pull request is too strict for this repository,
because CodeRabbit's status often ends `CANCELLED` when a newer push or its
review pause stops a run, and that alone makes a pull request `UNSTABLE`. A
strict rule would bounce pull requests that should merge. Issue #91, item 3, has the
history.

## Decision drivers

- The board must never queue or merge past a check that failed, required or
  not.
- A bot's cancelled or skipped run must not hold a pull request that is
  otherwise ready.
- The reconcile decides by state and reads GitHub once per pass, as ADR 0002
  sets out, so the check outcomes have to come back in the same batched read.
- The read shares the user's GitHub rate limit.

## Considered options

- Keep `UNSTABLE` mergeable
- Treat `UNSTABLE` as not mergeable
- Read the checks on an `UNSTABLE` pull request and decide by their outcomes

## Decision outcome

Chosen: read the checks. `CLEAN` and `HAS_HOOKS` stay mergeable as before. On
an `UNSTABLE` pull request, both `wf.merge` and the reconcile read every check
run and commit status on the head commit, from `statusCheckRollup`:

| The checks | `wf.merge` | The reconcile, for a waiting approval |
|---|---|---|
| Every check that did not pass ended `CANCELLED`, `SKIPPED` or `NEUTRAL` | Queues or merges | Queues or merges |
| One ended `FAILURE`, `ERROR`, `TIMED_OUT`, `ACTION_REQUIRED`, `STALE`, or any other outcome | `refused`, naming the failed checks | Holds the approval and opens one `report` card per approval |
| One is still running, and none failed | `waiting` with a merge queue, `refused` without one | Holds the approval and waits |
| GitHub returned no checks for the head, or more than 100 and none of the first 100 failed | `waiting` or `refused`, as for a running check | Holds the approval and waits |

A held approval stays. When someone re-runs the failed check and it passes,
the next reconcile pass queues the pull request at its approved head and
resolves the card. A push moves the head, so
the reconcile replaces the approval with a new merge gate as before. The
report card offers `fix`, which the ticket takes as "send it back to the
worker".

The rule keeps pending and failing apart because a running check has no
outcome yet and a merge-queue repository can wait it out, while a failure
needs a person to act.

### What the read costs

Both queries now add `commits(last: 1)` and its `statusCheckRollup.contexts(first: 100)`
to each pull request. A live read-only run against this repository on
2026-11-20 returned a `rateLimit.cost` of 1 for 3, 33 and 34 pull requests in
one query, 2 for 67 and 3 for 100. A board tracking under 50 pull requests
still pays 1 point per pass, and the worst case from ADR 0002 rises from 30
to 90 points an hour, under 2% of the hourly budget.

### Consequences

- Good: an optional check that failed blocks the merge in both paths, and the
  user hears of it from the board.
- Good: CodeRabbit's cancelled status no longer bounces a pull request whose
  real checks passed.
- Good: one batched read still covers every tracked pull request.
- Bad: a pull request with more than 100 checks on its head can never merge
  through the board while it is `UNSTABLE`. It can still merge once it is
  `CLEAN`, or by hand.
- Bad: a check that fails and is then re-run on the same head raises no
  second card for the same approval.
- Neutral: GitHub's merge button still accepts these pull requests. The rule
  binds only the board and `wf.merge`.

## Pros and cons of the options

### Keep `UNSTABLE` mergeable

- Good: no extra read and no new state.
- Bad: the reconcile queues past a failed optional check, which is the bug.

### Treat `UNSTABLE` as not mergeable

- Good: the simplest safe rule.
- Bad: a cancelled bot run holds a ready pull request until someone re-runs
  the bot or merges by hand.

### Read the checks and decide by their outcomes

- Good: blocks real failures and lets cancelled, skipped and neutral runs
  through.
- Bad: a larger query, and a list of outcomes to keep current with GitHub's
  API. An outcome the list does not name counts as a failure.

## More information

Issue #91, item 3. `src/merge.rs` holds the rule in `ready()`. The GitHub
reconcile section of `docs/src/board.md` and the `wf.merge` section of
`docs/src/globals.md` describe the behavior.
