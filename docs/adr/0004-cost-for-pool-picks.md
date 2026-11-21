---
status: accepted
date: 2026-11-20
---

# Pools pick the cheapest ready member under a strategy of its own

## Context and problem statement

Pool weights are static and set by hand, and
[ADR 0001](0001-usage-signal-for-pools.md) covers how pools learn of limits.
Nothing in a pick accounts for one member
costing less to run than another, so a metered API account takes launches
that a flat-rate subscription account could have run for nothing. Issue #109
asks for a member cost that favours cheaper members while they have room,
shown in the pick reason and in `wf.pools()`, that never overrides a reserve
or a limit. It leaves two questions open:

1. Where do costs come from: config only, or provider pricing and plan data?
2. How does a flat-rate subscription account, with no marginal cost until its
   limit, rank against a metered account?

## Decision drivers

- Cost must never pick a member that a reserve or a limit mark would skip.
  Cost decides only between members that are ready.
- Pools already configured for `weighted` or `priority` must pick exactly as
  they do now.
- t3cm must not read provider credentials or call provider endpoints, the
  same line ADR 0001 draws for usage.
- The pick reason has to say why cost sent a launch where it went.

## Considered options

1. A new strategy, `cheapest`: take the lowest-cost group of ready members and
   spread launches inside it by weight.
2. A cost factor inside `weighted`, such as an effective weight of
   `weight / cost`.
3. Costs read from provider pricing and plan data.

## Decision outcome

Chosen: option 1, with costs from config only.

- Each member takes an optional `cost`, a finite number of 0 or more that
  defaults to 1. Config load rejects a negative cost and names the member. A
  number too large for JSON, such as `1e400`, fails the config file's JSON
  parse before any pool check runs, and that error gives its line and column.
  The number has no unit, because only its order against the pool's other
  members counts.
- Under `cheapest`, a pick first sorts members out exactly as before: limit
  marks, T3's own state, and reserves. Among the members left, it takes those
  with the lowest cost and spreads launches across them with the same smooth
  weighted round robin `weighted` uses. It moves to the next cost only when
  every cheaper member is out. A member held at its reserve stays a last
  resort whatever its cost.
- The reason names the cost and the weighting, as in
  `cheapest: cost 0, weighted: weight 1 of 2; skipped claude-b limited until 14:00 MST`.
  `wf.pools()` shows each member's `cost`.
- Costs come from config only, which answers question 1. t3cm has no source
  for provider pricing or plan data, for the reason ADR 0001 gives for usage.
  No T3 tool returns it, and reading it from the providers would mean holding
  their credentials.
- A flat-rate subscription account is cost 0, which answers question 2. The
  pool picks it first until limit marking takes it out, and the metered accounts take over
  by cost after that. When the subscription resets, its mark clears and it
  takes the launches back.

### Consequences

- Good: a pool of one subscription and one metered account spends nothing on
  the metered one until the subscription hits its limit.
- Good: `weighted` and `priority` pools behave as before, and a cost set on
  their members has no effect, so adding costs to a config is safe before
  switching its strategy.
- Bad: a cost-0 subscription takes every launch until one fails on its limit,
  and the next watcher pass then marks the member, as ADR 0001 describes. Until t3cm has a usage source, a reserve on the
  subscription is the only way to leave room on it, and that reserve makes it
  a last resort.
- Bad: the user keeps the costs up to date by hand, so a price change does not
  reach the pool until someone edits the config.
- Neutral: a usage source would let `cheapest` stop short of the limit by
  reading how much each member has left. That waits on the source ADR 0001
  leaves open.

## Pros and cons of the other options

- **Option 2, a cost factor in `weighted`.** Good: no new strategy name. Bad:
  it changes how every existing `weighted` pool picks once a cost is set, and
  it still sends a share of launches to the expensive member while the cheap
  one has room, which is the waste the issue describes.
- **Option 3, provider pricing and plan data.** Good: no costs to keep by
  hand. Bad: no T3 tool returns pricing or plan data, so t3cm would have to
  call the providers with their credentials, which ADR 0001 and #74 rule out.
