---
status: accepted
date: 2026-11-23
---

# Structured PR events from the worker

## Context and problem statement

Issue #229 counts where ticket turns go. ADR 0013 took a worker's blockers
and its own gates past the ticket. Three messages per PR still woke it: "PR
opened", "PR updated" and "ready to merge". Over one weekend, workers sent 33
"updated" messages alone, and the ticket skill pointed at an "updated"
section it never had. Each message woke the ticket only to run a fixed
procedure from the ticket skill. On "opened", the ticket checked the
description. On "ready to merge", it verified the PR on GitHub, resolved the
delivery slot and opened the merge gate. On "updated", it noted the head.

Those messages were free text, sometimes under a gate name the worker chose,
so the service could not act on them. Code needs the PR, its head and the
Publishing line as fields.

## Decision drivers

- A step that needs no judgment should run without a ticket turn.
- The ticket must still learn what happened, from its digest.
- Where code cannot decide, the ticket gets the message it gets today.
- A merge approval holds for one head, as the board already enforces.

## Considered options

1. A dedicated call, `wf.pr({ event, repo, pr, head, ... })`.
2. New fields on `wf.send`, such as `pr` and `head` on an `event`.
3. Parse the worker's free-text messages.

## Decision outcome

Chosen: option 1.

- **The call.** A worker reports its PR with
  `wf.pr({ event: "opened" | "updated" | "ready", repo, pr, head, publishing?, checks?, body? })`.
  `publishing` is required on `opened`, and `checks: "none"` there marks a
  repo with no CI. The result's `status` tells the worker what happened and
  what to do next. `wf.send` keeps its meaning, so no existing call changes.
- **The format travels with the link.** `wf.openChild` takes
  `pr: { format?, sections?, passes?, mergeOnly? }` and stores it on the
  worker's link. `format` is required unless `mergeOnly` is set. `format: "pr"` also requires the `pr` skill's headings and
  elements: a fenced block under Summary, `Before:` and `After:` under
  Evidence, and `Door:` and `Blast Radius:` under Merge Danger. `sections`
  adds headings, such as a repo template's, each with the text it must hold.
  The issue and parent issue on the link give the `Refs #<n>` lines.
- **opened.** The service checks that the Publishing line names the format,
  each pass the `passes` slot resolves for the worker, and each of the PR's
  own passes. It reads the description on GitHub and checks every required
  heading, element and `Refs` line. A gap returns `status: "rewrite"` with
  the rewrite request the ticket skill used to send. Otherwise the ticket
  gets a "handled for you" line, and the event shows in the board's feed.
- **updated.** The service records the new head on the worker's link and
  drops any open merge card for an older head of that PR. In a repo with no
  CI, the ticket ran the checks, so an update goes to the ticket. That releases
  the ticket's `merge` gate when no other own gate card holds it. The ticket
  gets a "handled for you" line.
- **ready.** The service reads the PR on GitHub. It must be open, not a
  draft, and at the head the worker named, with every check finished and
  none failed. It resolves the delivery slot for the PR's changed files and
  builds the merge card's answers as the ticket skill described. Answers
  start from the builtin `merge` stage and take the longest run of `chain`
  stages that follow, longest prefix first. They stop at `pr` scope except
  on the ticket's last PR (`part` n of n), which continues into `ticket`
  stages. They stop before a `manual` stage and never chain `post-handoff`.
  Within the board's 8-answer and 60-character limits, `merge` is always
  kept. On the last PR, `ticket` stages with `paths` filter on the files of
  every PR the ticket's workers sent through `wf.pr`, follow-up fix PRs
  included. The card is the
  ticket's own `merge` gate, so GitHub reconcile holds its approval to the
  head, and its answer wakes the ticket as before. A ticket that adopts the
  worker takes the card over with it. The worker's link holds
  a `merge` gate, so the watcher does not nudge it while the card waits.
- **Pending and failing checks.** A check still running, or a merge state
  GitHub has not computed, returns `waiting` and marks the worker's PR. Each
  GitHub reconcile pass reads marked PRs. Once their checks finish, it runs
  the same `ready` path, so the card opens with no ticket turn. A check that
  failed returns `failing` to the worker, and when reconcile finds one it
  sends the worker a reply. A draft, a moved head or a merge conflict is
  refused back to the worker the same way.
- **A blocked merge state.** Checks can all pass while `mergeStateStatus` is
  `BLOCKED`, for example on a stale requested-changes review, or `BEHIND`.
  The card still opens, with the same answers, and its action line names the
  merge state and review decision. It says an admin can merge over the block
  on GitHub, and reconcile picks up a merge made there, as it does for any
  tracked PR.
- **The ticket keeps today's path where code cannot decide.** The event goes
  to the ticket as the message the worker used to send, with a line saying
  why. That happens for a ticket under an overseer thread, `relay` off, a
  worker's second PR (a follow-up fix), `mergeOnly` (a targeted fix PR), no
  stored format on `opened`, `checks: "none"`, a delivery slot with no
  builtin `merge` stage or with a problem, an unknown `part`, an earlier PR
  of the ticket that never went through `wf.pr`, more than 100 changed
  files, no finished checks at the head, or a PR the service cannot read.
- **Tag wording.** The card names each chained stage by its id. It does not
  name a deploy's tag family and version, which come from the deploy skill's
  rules, and that skill computes them at deploy time anyway.

Option 2 overloads `wf.send` with fields that apply to one kind and one role,
and every send would have to validate them. Option 3 keeps the free text that
caused the misnamed gates.

### Consequences

- Good: opened, updated and ready reach the board with no ticket turn when
  nothing needs a decision.
- Good: a merge card always carries the answers the delivery slot calls for,
  built the same way every time.
- Bad: the worker and ticket skills must use `wf.pr` and `wf.openChild`'s
  `pr`, and the ticket skill keeps the old steps for the fallback path.
- Neutral: the merge itself, on the user's answer, still runs in the ticket
  until the last PR of #229 moves it. ADR 0013's rule that an answer starting
  with `merge` goes to the ticket stays; a worker now sends ready through
  `wf.pr`, not under a gate of its own.
