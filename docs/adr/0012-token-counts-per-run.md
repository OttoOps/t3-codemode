---
status: accepted
date: 2026-11-22
---

# Token counts per run come through a source seam

## Context and problem statement

Issue #216 asks for a chart of token volume per provider instance on the
board. t3cm records which instance each pick chose and has no record of the
tokens the runs on that instance used, so it needs input and output counts
per run.

t3cm reaches T3 only through its MCP tools, and none of them carries token
counts. I checked the tools on 2026-11-22: `t3_thread_list`, and
`t3_thread_read` with its `recentRuns`, carry `runId`, `status`,
`providerInstanceId`, `model`, `executedModel` and the run's timestamps, and
nothing about tokens.

## Decision drivers

- t3cm must not read provider credentials or call provider usage endpoints,
  for the reason [ADR 0001](0001-usage-signal-for-pools.md) gives.
- Against today's T3, the new fields must show that no counts exist, so that
  no caller reads them as zero tokens, and they must cost no extra T3 calls.
- The store and the `wf.usageStats` fields should not need to change again
  when counts arrive.

## Considered options

1. A `TokenSource` trait beside `UsageSource`, with a T3 source that reports
   nothing until T3 sends counts.
2. Read a guessed token field from `recentRuns` now.
3. Read counts from the providers directly.
4. Wait for T3 to send counts and build nothing yet.

## Decision outcome

Chosen: option 1.

- `TokenSource` says whether it reports counts at all, and gives the input
  and output counts of one run, or none.
- `T3Tokens` is the source in production. It reports nothing, so the watcher
  pass makes no T3 call for tokens and stores nothing.
- With a source that reports, the watcher pass lists the threads updated in
  the past 24 hours whose latest run it has not stored, and reads their
  `recentRuns`. For each run that ended in that time and is not stored yet,
  it asks the source once and stores the counts per thread and run in
  `store.db`, with the run's own instance, its driver and its `completedAt`.
  A thread moved to another instance keeps its earlier runs on the instance
  that ran them. A run the source has no counts for is stored without them,
  so it is not read again. A failed read stores nothing, and the next pass
  tries again. Each pass prunes rows older than 400 days.
- `wf.usageStats` adds `tokens`, which holds the totals with `byInstance`
  and `byProvider`, a `tokens` field on each member, and `tokenSeries`, which
  uses the same 24 buckets as the picks `series`. All of them are `null` when
  the source does not report.
- Tests drive the path with `FakeTokens` and the fake T3's `recentRuns`.

### Consequences

- Good: the board chart and any caller can build on the fields now, and they
  fill in once a source reports, with no schema change.
- Good: against today's T3 the pass costs nothing, and the fields are `null`,
  which no caller can mistake for zero tokens.
- Bad: with a source that reports, the pass reads every thread whose latest
  run is still going, on each pass, until that run ends and is stored.
- Bad: the pass sees the latest 20 runs of a thread, so a thread that ends
  more than 20 runs between two passes loses the older ones.
- Bad: t3cm keeps the threads with a failed run read in memory, so a restart
  before the retry succeeds loses that run once the thread's latest run is
  stored.
- Neutral: when T3 adds counts, `T3Tokens` reads them and says it reports,
  and the rest stays as it is. Getting T3 to send counts is an upstream
  request.

## Pros and cons of the other options

- **Option 2, guessing the field.** Bad: T3 has not named the field, so the
  guess would most likely never match, and every pass would read each thread
  for nothing.
- **Option 3, provider usage.** Ruled out, because t3cm would hold
  credentials for every account.
- **Option 4, waiting.** Bad: the board chart and its data path would wait on
  upstream work, and nothing would be tested until then.
