---
status: accepted
date: 2026-11-19
---

# Pools learn of usage limits from the threads T3 fails on them

## Context and problem statement

Issue #74 adds pools, which spread launches across several T3 provider
instances of one driver. A pool has to stop picking an account that hits its
usage limit and pick it again once the limit resets, so t3cm needs a signal
that says an account is out and when it comes back.

T3 builds a per-account usage report for its composer, but no MCP tool returns
it, and t3cm reaches T3 only through those tools. I checked what the tools do
show against a live T3 and its source on 2026-11-19:

- `t3_thread_list` gives each thread's `status`, `providerInstanceId`,
  `snoozed` and `snoozedUntil`. A snooze carries no reason, and a snooze the
  user set by hand looks the same as one T3 set for a limit.
- When a run stops on a usage limit, T3 sets the thread's status to `failed`
  and ends the run with an `error` item. T3 titles that item
  `Usage limit reached` for exactly the failures it classes as usage limits,
  and `Provider error` for the rest. `t3_thread_read` returns the item with
  its `type`, `title` and `runId`.
- T3 keeps the reset time in its own projection as `usageLimitResetAt`, and
  `t3_thread_read` does not return it. The item's text is the provider's own
  message. Codex writes "try again at Nov 5th, 2026 11:55 AM" with no time
  zone, and Claude words it differently again.
- T3's "snooze limited threads" setting is off by default. With it on, T3
  snoozes a limit-stopped thread until the reset, so `snoozedUntil` is the
  reset time.

## Decision drivers

- t3cm must not read provider credentials or call provider usage endpoints,
  since that is the credential proxy the issue ruled out, by another route.
- A wrong "limited" mark takes an account out of service for no reason, while
  a missed mark costs one failed launch before the next pass catches it.
- A user's own snooze must never take an account out of a pool.

## Considered options

1. Mark from thread stops: a failed thread whose latest run ends with T3's
   `Usage limit reached` error item.
2. Mark from snoozes: any snoozed thread on a pool member.
3. Parse the reset time out of the provider's message text.
4. Read usage from the providers directly.
5. Wait for T3 to expose its usage report as an MCP tool.

## Decision outcome

Chosen: option 1 now, with a seam for option 5.

- The watcher pass lists failed threads on pool members from the last 24
  hours and reads each failed run once for the `Usage limit reached` item.
  t3cm stores the verdict per run, so it never reads a run twice.
- The reset time is `snoozedUntil` when the thread is snoozed the first time
  t3cm sees the stop, because that snooze is T3's own limit snooze. Otherwise
  t3cm cannot know the reset, so it holds the member for `limitHoldMinutes`,
  60 by default, from when it saw the stop. It ignores a snooze that shows up
  later, because that one may be the user's.
- A stop with a later reset extends a member's mark, and one with an earlier
  reset leaves it as it is. The pass clears the mark at its reset.
- `UsageSource` is a trait that reports each instance's share of its usage
  window left and its reset time. Nothing implements it in production. Once a
  source is in place, a reserve becomes a real floor, a member with no usage left is
  limited until its reset, and the `headroom` strategy picks the member with
  the most left, with weights breaking ties. Tests drive it with a fake.
- Config load keeps rejecting `strategy: "headroom"`. With no source behind
  it, a pool configured for headroom would run on weights, and
  nothing in the config would show that. The PR that adds a source lifts the
  check.

### Consequences

- Good: a limit stop takes the account out of its pools within one watcher
  pass, with no new T3 API and no provider access.
- Good: a snooze the user set never takes an account out of its pools.
- Bad: with T3's limit snooze off, the reset time is a guess. A member can come
  back before its account resets, and the next launch on it fails and marks it
  again. The docs tell users to turn the setting on.
- Bad: the detection depends on T3's item title, so if T3 renames it, marking
  stops without any error. The fake T3 tests today's title, which catches a
  change in t3cm and does nothing about one in T3.
- Neutral: whatever source ships decides which usage window drives
  `headroom`, which is issue question 4. `UsageSource` reports one number per
  instance.

## Pros and cons of the other options

- **Option 2, snoozes alone.** Bad: a snooze carries no reason, so this would
  take an account out of its pools whenever the user snoozes a thread for
  their own reasons.
- **Option 3, parsing the message.** Bad: the Codex text has no time zone, each
  provider words it differently, and providers change the wording without
  notice, so a wrong parse would look just like a right one.
- **Option 4, provider usage endpoints.** The issue ruled this out, because
  t3cm would hold credentials for every account.
- **Option 5, waiting for the T3 tool.** Good: it gives headroom and exact resets.
  Bad: it blocks phase 1 on upstream work. The seam keeps that path open, and
  exposing `lastErrorClass` and `usageLimitResetAt` on `t3_thread_list` would
  also make option 1 exact. Both are upstream requests.
