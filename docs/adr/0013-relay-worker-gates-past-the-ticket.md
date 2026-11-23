---
status: accepted
date: 2026-11-23
---

# Relay worker gates past the ticket

## Context and problem statement

Issue #229 counts where ticket turns go. Over one weekend, 31 ticket threads
took 507 turns, and 376 of them were `reply`, `gate` and `event` wakes. Many
of those wakes only pass a message along. A worker's blocker or review
findings wake the ticket, the ticket sends them up to the board unchanged, and
the board's answer wakes the ticket again so it can forward the answer to the
worker. Each pass costs a full turn of a model that holds the ticket's whole
context.

The issue left three questions open. Where do the routing rules live? How
does the ticket learn what the service handled for it? Does a ticket under an
overseer thread follow the same rules?

## Decision drivers

- A message that needs no judgment should not wake the ticket.
- The ticket must still see everything that happened for it, so its picture
  of the work stays current.
- One config switch must turn the routing off without a release.
- A ticket under an overseer thread sends its gates up a second hop, to a
  thread that speaks to the user. Changing that path needs more design.

## Considered options

1. Routing rules in code, with a per-link `relay` switch in the roles config.
2. Routing rules declared per link in config, as a list of kinds and gates.
3. Leave the relay to the ticket skill, on a cheaper ticket model.

## Decision outcome

Chosen: option 1.

- **The rules live in code.** A worker's `blocker`, and every `gate` it
  opens except the ticket-owned `merge` and `delivery:<id>`, go straight to
  a board card when its ticket's own link goes to the board. Workers name
  their gates freely, and one weekend's runs used `review`, `cr-round2`,
  `s1-member-retry`, `prod-verify` and `user` among others, so an allowlist
  of names would miss most of them. The two excluded names start delivery
  stages, which the ticket runs. The card sits under the ticket, carries the worker's label,
  thread and gate, and links to the worker's thread. While any relayed card
  waits, the ticket's link holds a `relay:<gate>` gate, derived from the open
  cards, so the watcher does not nudge it. The ticket's own gate takes
  precedence, and the hold comes back when that gate clears. A relayed
  blocker also opens a `relay:blocker` gate on the worker's link, so the
  watcher neither nudges the worker nor escalates to the ticket while the
  user decides.
- **Answers route on their text.** An answer that is one of the card's
  buttons, or `hold`, goes to the worker as a `reply` from the board. Any
  other answer needs judgment, so it goes to the ticket, with a note that the
  worker still waits on it. An answer that starts with `merge` also goes to
  the ticket, because a worker that sent "ready to merge" under a gate such
  as `user` cannot merge, and the ticket can. A later PR of #229 moves
  "ready to merge" to a structured worker event.
- **A per-link `relay` switch** sits in `Config::link_policy` next to `wake`
  and `bodyCap`, `true` by default. `"relay": false` on
  `roles.ticket.children.worker` sends these messages through the ticket
  again.
- **The ticket learns from its digest.** Each message the service relays, and
  each answer it routes back, is stored as a `handled` message for the
  ticket. Its next real wake lists each one as a "handled for you" line in
  "Notes since your last wake". `wf.inbox()` lists them too.
- **A ticket under an overseer thread keeps today's path** whatever `relay`
  says, because its gates travel a second hop through the overseer thread.
  A follow-up issue can extend the routing there.
- A worker that clears its gate, or hands off, resolves its own relayed
  cards. A ticket's gate clear leaves them open, and a relayed gate and a
  ticket's own gate do not supersede each other's cards. A worker's later
  `merge` or `delivery:<id>` gate resolves its open relayed gate cards. A dismissed card
  releases its holds like an answered one, and so do a worker link that
  closes and a ticket that finishes. A ticket that adopts a worker takes
  over its relayed cards, and a worker moved to a fork keeps them. GitHub reconcile leaves relayed cards out entirely, and an open relayed
  card never keeps a ticket's `merge` gate set once its merge card resolves.
- The worker writes the card. Its relayed gates and blockers open with an
  action line and pass `options` for the answers that need no judgment.

Option 2 makes every rule a config surface to validate and document before
anyone needs to change one. Option 3 lowers the cost of each wake but keeps
every wake.

### Consequences

- Good: a worker's blocker or gate is raised and answered with no ticket
  turn, whatever the worker named the gate.
- Good: the digest keeps the ticket's view whole without a turn per message.
- Bad: a ticket now has to read its digest to see a blocker it did not
  relay, and the ticket and worker skills must stop describing the relay
  steps the service runs.
- Neutral: the merge gate, the delivery gates and the merge itself still go
  through the ticket until the later PRs of #229 move them.
