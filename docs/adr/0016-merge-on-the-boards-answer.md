---
status: accepted
date: 2026-11-23
---

# Merge on the board's answer

## Context and problem statement

Issue #229 counts where ticket turns go. ADR 0013 took a worker's own gates
past the ticket, and ADR 0015 had the service open the merge card from the
worker's `ready` event. The user's answer to that card still woke the
ticket. The ticket then ran a fixed procedure: check the head, call
`wf.merge`, start a merge watch for a running check, delete the remote
branch, and send the worker "Merged #<n> at <sha>. Hand off." A `queued` or
`waiting` result, the merged event and each card GitHub reconcile raised
woke it again. On a merge-only PR, none of those turns needed judgment.

`wf.merge` also refused a pull request with a running check on a repo with
no merge queue (ADR 0003). The ticket then had to wait the check out with a
scheduled merge watch of its own.

## Decision drivers

- An answer that needs no judgment should not wake the ticket.
- The ticket must wake once a merge leaves stages for it to run.
- An approval holds for one head, as ADR 0003 and ADR 0015 already enforce.
- Code never merges over a review that requests changes. In this repo, an
  admin merges over CodeRabbit on GitHub when the user decides to.
- Where the service cannot decide, the ticket gets the message it gets
  today.

## Considered options

1. Run the merge in code on the card's answer, and hold every unmerged
   approval in the merge wait.
2. Keep the merge in the ticket, and only stop the merged event from waking
   it.
3. Merge in code, but leave a running check to the ticket's merge watch.

## Decision outcome

Chosen: option 1.

- **The answer.** A button answer that starts with `merge` on a merge card
  that `wf.pr` `ready` opened goes to `wf.merge` at the card's head, called
  for the ticket. The method comes from the worker's brief:
  `wf.openChild({ pr: { method } })`, which a repo with no PR format can
  pass alone. The card stores the delivery stages
  that follow `merge` for this PR, so the answer's approved prefix and the
  rest are known without the ticket. The ticket's digest records the answer.
- **The merge wait.** `wf.merge` on a repo with no merge queue now records a
  `BLOCKED`, `UNKNOWN` or `UNSTABLE`-with-a-running-check pull request as
  `waiting`, as it already did on a merge-queue repo. GitHub reconcile
  merges it once GitHub shows it mergeable at the approved head. That
  supersedes ADR 0003's `refused` for a running check without a merge
  queue. `BEHIND` and `DIRTY` stay `refused`, because only a push clears
  them, and a push drops the approval. So does an `UNSTABLE` pull request
  whose checks GitHub did not return, since nothing would end that wait. A `CLEAN` or `HAS_HOOKS` pull request
  is now held the same way while any check GitHub shows on its head is
  running, and refused when one failed. ADR 0003 merged those states
  unread. With no ticket checking the head before the merge, a check that
  re-runs after the card opens would otherwise be skipped.
- **After the merge.** Once GitHub shows the pull request merged, the
  service deletes its head branch through the GraphQL `deleteRef` mutation.
  It skips a branch GitHub already deleted, a branch on a fork, and a branch
  whose tip, when the service reads it, is no longer the merged head.
  `deleteRef` takes no expected tip, so a push in the moment between that
  read and the delete can still go with the branch. It
  resolves the ticket's merge cards for the pull request. When no `pr`-scoped
  stage remains after `merge`, it sends the worker "Merged #<n> at <merge
  commit>. Hand off." It wakes the ticket once, naming the remaining stages
  and the approved ones, when a `pr`-scoped stage remains, or a
  `ticket`-scoped one on the ticket's last PR. Otherwise the ticket reads the
  merge in its digest and next wakes on the worker's handoff.
- **A review block.** A card whose merge state is `BLOCKED` by a change
  request keeps its usual buttons. Its action line already says an admin can
  merge it over the block on GitHub. A `merge` answer holds the approval in
  the merge wait. `wf.merge` never merges over the block, and GitHub refuses
  it anyway. When an admin merges the pull request on GitHub, reconcile
  takes that as the merge and runs the steps above with the answer's
  approved stages. When the block clears, as when the reviewer approves,
  reconcile merges it at the approved head.
- **A merge on GitHub with no answer.** A pull request merged on GitHub
  while its merge card is still open counts as `merge` alone, as the ticket
  skill already said.
- **Reconcile cards.** On a card GitHub reconcile raised for a pull request
  the service approved, `requeue` and `merge` call `wf.merge` again at the
  card's head, and `ok` accepts the queued head and holds a new approval
  for it. At a head other than the approved one, the stages the earlier
  answer approved no longer count, so each opens its own gate. Any other answer, `hold` and `fix` included, goes to the ticket
  and drops the service's approval and its merge wait, so nothing merges
  the pull request on its own.
- **The fallback.** The ticket gets the answer as before when the brief has
  no `method`, the reconcile is off, no open worker link holds the pull
  request, the card predates the stored stages, or the answer is not one of
  the card's buttons. A `refused` result, or a `wf.merge` that throws, goes
  to the ticket with the outcome.

Option 2 keeps three turns per PR: the answer, the merge watch and the
handoff message. Option 3 keeps the merge watch, which is the most common
of them, since a merge card often opens while a bot's check re-runs.

### Consequences

- Good: a merge-only PR goes from the board's answer to the worker's
  handoff with no ticket turn, and a chained answer wakes the ticket once.
- Good: the ticket skill's merge watch now serves only a root ticket and a
  reconcile that is off.
- Bad: the service holds an approval as long as GitHub blocks the merge,
  and the reconcile reads that pull request at the active interval until it
  merges, closes or moves.
- Neutral: a `wf.merge` caller on a repo with no merge queue now gets
  `waiting` where it got `refused`, and must treat it as the merge-queue
  case already did.
