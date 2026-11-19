---
status: accepted
date: 2026-11-19
---

# Check tracked pull requests against GitHub every 2 minutes while one waits, every 10 otherwise

## Context and problem statement

The board finds out about a merge only when a thread tells it. A pull request
merged by hand on GitHub, closed, or dropped from a merge queue leaves its gate
card open and its ticket asleep, because the watcher does not nudge a thread
with an open gate. On 2026-11-18 four merge cards asked for a "merge" hours
after their pull requests had merged, and issue #56 has the timeline.

t3cm now reconciles on a timer: one batched GraphQL query reads every tracked
pull request, and the board fixes what differs. This record sets how often
that query runs, given that every run spends the user's own GitHub rate limit.

## Decision drivers

- A pull request merged by hand should resolve its card and wake its ticket
  within minutes, without anyone touching the board.
- A pull request waiting to be queued should join the queue soon after it turns
  mergeable, since a merge queue only lands what it holds.
- The query runs as the user's own `gh` login and shares that user's GitHub
  rate limit with everything else they run.
- t3cm is a small local service. Each check starts one `gh` process.

## Considered options

- A fixed 1-minute check, on every watcher pass
- A fixed 10-minute check
- 2 minutes while a tracked pull request is queued or waiting, 10 minutes
  otherwise, plus a check when the board page loads
- GitHub webhooks

## Decision outcome

Chosen: 2 minutes while any tracked pull request is queued or waiting to be
queued, 10 minutes otherwise, and a check on each board page load at most once
every 30 seconds. `board.reconcileActiveMinutes` and
`board.reconcileIdleMinutes` set the two intervals, and an idle interval of `0`
turns the check off.

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
connection is `timelineItems(last: 1)` for the latest merge-queue removal. A
live run against this repository on 2026-11-19 returned a `rateLimit.cost` of 1
for one pull request and 1 for 52 pull requests in one query, and reported the
limit as 5,000. The service batches up to 100 pull requests per query, so a
check costs 1 point for any realistic board.

At these intervals that is 30 points an hour while something waits and 6 an
hour otherwise: under 1% of the hourly budget at worst. The one enqueue per
waiting pull request that turns mergeable adds 5 secondary-limit points, once.
A check that tracks nothing makes no GitHub call at all.

### What a check costs t3cm

Each check spawns one `gh api graphql`. The same live run took 1.3 s of wall
time and 0.14 s of CPU for the `gh` client. At most 30 of those an hour is
negligible next to the watcher, which reads every open thread from T3 each
minute. The check runs inside the existing board pass and holds no lock while
it waits on GitHub. A slow `gh` can hold up the next watcher pass by up to its
30-second timeout.

### Consequences

- Good: a hand merge resolves its card and wakes its ticket within 10 minutes,
  and within about 2 when the pull request was queued or waiting.
- Good: opening the board brings it up to date at once.
- Good: the GitHub cost stays near 1% of the user's budget in the worst case.
- Bad: an idle board can show a merged pull request as open for up to 10
  minutes. Loading the page fixes that.
- Neutral: both intervals are config values, so a busy user can trade budget
  for speed.

## Pros and cons of the options

### A fixed 1-minute check

- Good: the fastest reaction.
- Bad: 60 points an hour whether or not anything is in flight, and 60 `gh`
  processes an hour on a board that is mostly idle.

### A fixed 10-minute check

- Good: the cheapest.
- Bad: a pull request waiting to be queued sits up to 10 minutes after it turns
  mergeable, which is the stall issue #56 set out to end.

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
