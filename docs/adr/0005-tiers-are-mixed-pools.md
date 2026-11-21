---
status: accepted
date: 2026-11-20
---

# A capability tier is a mixed pool whose members name their models

## Context and problem statement

Phase 1 pools (issue #74) hold instances of one driver, and every launch from
a pool runs one model at one effort. The pick changes the account, never the
intelligence. Issue #108 asks for work where matching capability matters less
than spreading load, such as a research swarm on cheap models from several
vendors. Those launches need a pool whose members are instances of different
drivers, each with its own model and effort, and a way for a launch to ask for
a class of model such as "any frontier model" or "any cheap model".

Two questions follow. Where does a tier live: is it a pool, or a named group
inside one? And who decides which models belong to a tier: the config, or
t3cm grouping them from what `orchestrator_capabilities` reports?

## Decision drivers

- An existing pool must not turn into a mixed one by accident. A typo in a
  same-driver pool's config should fail at load, not quietly change which
  model a launch runs.
- A pick has to be explainable after the fact: which account, which model,
  which effort, and why.
- `orchestrator_capabilities` lists each instance's models and effort
  options. It does not rank or classify them, and no field marks a model as
  frontier or cheap.
- The picker, the limits, the reserves and the pick history already work per
  pool. A second level of grouping would need its own copy of each.

## Considered options

1. A tier is a pool named for its class. A pool opts in with `"mixed": true`,
   and each member lists its own model and effort.
2. A tier is a named group inside a pool, with its own members, and a launch
   asks for a pool and a tier.
3. A tier is a class label, and t3cm groups models into it from
   `orchestrator_capabilities` at pick time.

## Decision outcome

Chosen: option 1.

- A pool with `"mixed": true` has no pool-level `driver`, `model` or
  `effort`. Each member carries its own `model`, and its own `effort` when it
  sets one, on any driver. Config load rejects a mixed pool that sets any of
  the three at pool level, and a mixed member with no model.
- A pool without the flag keeps phase 1's rules: a pool-level `driver` and
  `model`, and the same-driver check against T3 at each pick. Config load
  rejects a `model` or `effort` set on one of its members, so one stray key
  cannot make it mixed.
- Tiers are pools named for their class, such as `frontier` or `cheap`, and a
  launch asks for one by pool name, as any pick does today.
- `wf.pick` returns the chosen member's driver, model and effort, and its
  reason starts with them, for every pool. `wf.pools` shows each member's
  driver, model and effort.

### Consequences

- Good: no new grouping level, so limits, reserves, weights, credits and pick
  history work unchanged on a mixed pool.
- Good: the config says exactly which model each account runs. A pick never
  lands on a model nobody listed.
- Good: phase 1 configs load and pick as before. The only visible change is
  the driver, model and effort at the start of each pick's reason.
- Bad: an account that belongs to two tiers is listed in two pools, and each
  pool keeps its own weight credits, so the two pools do not balance against
  each other. A usage limit still benches the account in both, because limits
  are per instance.
- Bad: when a provider ships a new model, someone edits the config to add it
  to a tier.
- Neutral: whether to warn about a model that only one member offers is the
  issue's second question, and a later PR answers it.

## Pros and cons of the other options

- **Option 2, tiers inside a pool.** Good: one account listed once. Bad: the
  picker, the credits, the limits view and `wf.pools` would all need a tier
  dimension, and a launch would need to name two things where one does today.
- **Option 3, grouping from capabilities.** Good: new models join a tier with
  no config change. Bad: T3 reports no class for a model, so t3cm would have
  to guess from model ids or keep its own table of models, and a wrong guess
  would launch frontier work on a cheap model with nothing in the config to
  show why.
