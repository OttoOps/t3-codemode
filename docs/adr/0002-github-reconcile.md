---
status: accepted
date: 2026-11-19
---

# Reconcile pull requests with GitHub by state, every 2 or 10 minutes

## Context and problem statement

The board finds out about a merge only when a thread tells it. A pull request
merged by hand on GitHub, closed, or dropped from a merge queue leaves its gate
card open and its ticket asleep, because the watcher does not nudge a thread
with an open gate. On 2026-11-18 four merge cards asked for a "merge" hours
after their pull requests had merged, and issue #56 has the timeline.

t3cm now reconciles on a timer: one batched GraphQL query reads every tracked
pull request, and the board fixes what differs. This record sets two things:
how the board decides what to fix, and how often the query runs, given that
every run spends the user's own GitHub rate limit.

The first version of this change (PR #78) decided by transition: it compared
each read with the last one it had stored and acted on what changed. Two
writers kept that stored copy, the check and `wf.merge`, and the check queued a
waiting pull request by calling `wf.merge`, which read GitHub again and decided
again. Nine review rounds then found the same kind of problem over and over,
in which the stored copy, the first read and the second read disagree for a
few seconds and an approval or a wake goes missing in the gap.

## Decision drivers

- A pull request merged by hand should resolve its card and wake its ticket
  within minutes, without anyone touching the board.
- A pull request waiting to be queued should join the queue soon after it
  turns mergeable, at the head the user approved and no other.
- A crash, a T3 outage or a slow GitHub must not lose a wake or repeat one.
- The query runs as the user's own `gh` login and shares that user's GitHub
  rate limit with everything else they run.
- t3cm is a small local service. Each check starts one `gh` process.

## Considered options

For deciding:

- By transition, from a stored copy of the last read, as PR #78 did
- By state: compare GitHub now with the board now, and record each event that
  has been acted on

For the cadence:

- A fixed 1-minute check, on every watcher pass
- A fixed 10-minute check
- 2 minutes while a tracked pull request is queued or waiting, 10 minutes
  otherwise, plus a check when the board page loads
- GitHub webhooks

## Decision outcome

Chosen: decide by state, and check every 2 minutes while any tracked pull
request is queued or waiting to be queued, every 10 minutes otherwise, and on
each board page load at most once every 30 seconds.
`board.reconcileActiveMinutes` and `board.reconcileIdleMinutes` set the two
intervals, and an idle interval of `0` turns the check off.

### Deciding by state

- **One writer for what GitHub showed.** Only the check writes the `pr_seen`
  table, and the board uses it for the `queued` chip, the cadence, and to keep
  following a pull request in the queue after its worker hands off. No decision
  compares one read with the last.
- **Approvals are records with an id.** `wf.merge` saves each approval it
  cannot finish at once, and each one it sent to the queue, as a row in
  `merge_waits` with a new random `approval` id. The check changes or deletes a
  row only while it still has the id the check read, so a newer approval always
  wins.
- **The check sends the mutation itself.** When the batch read shows a waiting
  pull request mergeable at the approved head, the check sends
  `enqueuePullRequest`, or `mergePullRequest` when the queue is off, with
  `expectedHeadOid` set to the approved head. GitHub enforces the head, so the
  check does not read again first. A refused mutation keeps the approval, and
  the third failure in a row opens a report.
- **Each event is acted on once per ticket.** The `pr_events` table records
  each wake and removal report under the pull request, an event key (the merge
  commit, the close time, or the removal time) and the ticket. The check writes
  a wake's record, its message and its outbox entry in one transaction, so a
  wake is never lost and never sent twice. Events from before the board started
  tracking the pull request, or from before the first start of this version,
  stay quiet.
- **Each card's pull request is stored when the card opens.** The board reads
  it once from the card's full message, so later checks never parse card text,
  and a replacement gate carries the head the check raised it for.
- **Every other fix follows from the current state** and can run again with
  the same result. A merged pull request resolves its cards opened before the
  merge, and the check replaces a gate at an old head with a card at the new
  head, creating the new card before it resolves the old one.

### What a check costs on GitHub

GitHub documents these GraphQL limits in
[Rate limits and query limits for the GraphQL API](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api):

- The primary limit is 5,000 points per hour per user, shared by every token
  and app acting for that user.
- A query costs the requests its connections need, divided by 100 and rounded,
  with a minimum of 1 point.
- The secondary limit is 2,000 points per minute. A query without mutations
  counts 1 point toward it and a mutation counts 5.

The reconcile query reads each pull request through an alias, and its one
connection is `timelineItems(last: 1)` for the latest merge-queue removal. Live
read-only runs against this repository on 2026-11-19 returned a
`rateLimit.cost` of 1 for 3 aliases in one query, and 1 for 52. Of those 52, 22
were numbers with no pull request: each came back as its own `NOT_FOUND` error,
with the other 30 pull requests' data in the same response. The service batches
up to 100 pull requests per query, so a check costs 1 point for any realistic
board.

At these intervals that is 30 points an hour while something waits and 6 an
hour otherwise: under 1% of the hourly budget at worst. Each enqueue or merge
the check sends adds 5 secondary-limit points. A check that tracks nothing
makes no GitHub call at all.

### What a check costs t3cm

Each check spawns one `gh api graphql`, plus one per mutation it sends. The
52-alias read took about 2 s of wall time and 0.1 s of CPU for the `gh` client.
At most 30
checks an hour is negligible next to the watcher, which reads every open thread
from T3 each minute. The check runs inside the board pass. It holds the
workflow lock only while it applies one pull request's fixes to the store, and
again to deliver the wakes it queued, never while it waits on GitHub. Each `gh`
call has a 30-second timeout, so a slow GitHub can hold up the next watcher
pass by that much for each call the check makes.

### Consequences

- Good: a hand merge resolves its card and wakes its ticket within 10 minutes,
  and within about 2 when the pull request was queued or waiting.
- Good: opening the board brings it up to date at once.
- Good: the GitHub cost stays near 1% of the user's budget in the worst case.
- Good: no decision depends on two reads agreeing, so the stale-read races
  the first version kept hitting cannot occur.
- Bad: an idle board can show a merged pull request as open for up to 10
  minutes. Loading the page fixes that.
- Bad: a gate that names no head is held to the first head a check sees, so a
  push before that check goes unnoticed until the ticket's gate text names its
  head.
- Bad: a crash between opening a card and recording its event can open the
  same report twice. It cannot lose one.
- Neutral: both intervals are config values, so a busy user can trade budget
  for speed.

## Pros and cons of the options

### Deciding by transition

- Good: a stored copy of the last read is one table and needs no event keys.
- Bad: every decision depends on that copy, the current read and any second
  read agreeing, and two writers keep the copy. Each disagreement needs its own
  fix.

### Deciding by state

- Good: each fix can run again with the same result, so a crash or a stale read
  costs at most one interval.
- Bad: one more table, and an event key per kind of event.

### A fixed 1-minute check

- Good: the fastest reaction.
- Bad: 60 points an hour whether or not anything is in flight, and 60 `gh`
  processes an hour on a board that is mostly idle.

### A fixed 10-minute check

- Good: the cheapest.
- Bad: a pull request waiting to be queued sits up to 10 minutes after it
  turns mergeable, which is the stall issue #56 set out to end.

### 2 minutes while queued or waiting, 10 otherwise, plus page loads

- Good: fast while a pull request has a queue to join or a merge to report,
  and cheap the rest of the time.
- Bad: two settings in place of one.

### GitHub webhooks

- Good: no polling, instant.
- Bad: t3cm listens only on loopback, so a webhook needs a public endpoint or a
  relay, which the project will not take.

## More information

Issue #56, part 3. The GitHub reconcile section of `docs/src/board.md`
describes what the board does, and `docs/src/config.md` lists the settings.
