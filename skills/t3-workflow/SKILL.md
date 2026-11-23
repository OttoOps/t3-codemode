---
name: t3-workflow
description: The T3 workflow for T3 Code, and the router to its skills - the link protocol under every workflow thread (links, notes, wakes, gates, nudges, code mode), setting up its slots (reviewer, passes, delivery, models), installing t3cm, and the overseer, ticket and worker roles. Use when a role says to, when a message opens with a [t3cm header, when a launch brief carries a "t3-codemode link" or "t3-workflow link" block, or when the user asks how the T3 workflow works, to set it up or change its reviewer, passes, delivery or models, what those are set to, or about installing t3cm.
---

## Which skill

| The user or brief wants | Go to |
|---|---|
| to set up the workflow, or change its reviewer, passes, credential renewal, delivery or models | the `setup-t3-workflow` skill |
| what the slots are set to now | `wf.slots()` or `t3cm slots` ([Slots](#slots)) |
| several issues worked in parallel | the `t3-overseer` skill |
| one issue, or issues one after another | the `t3-ticket` skill |
| a launch brief for one PR's worker | the `t3-worker` skill |
| t3cm, or installing it | [codemode.md, Installing t3cm](codemode.md#installing-t3cm) |
| a thread filed a GitHub issue | [Filed issues](#filed-issues) |
| how links, messages and gates work | the rest of this file |

## The protocol

Every thread in the T3 workflow is a **node** with at most one parent
and any number of children. A parent and child pair is a **link**. The
`t3cm` service holds every link and every message body, and its
**watcher** reads each child's live state from T3 and does all the
timekeeping. Threads keep no timers and never check in.

The protocol keeps bulk out of context. A thread's context is re-read on
every model call, so each wake and each pasted payload costs again on every
later call:

- Routine progress is a **note**: stored, never a wake. It reaches the parent
  as one digest line at the parent's next wake.
- A wake carries a capped body and a **handle** (`m42`); `wf.show("m42")`
  returns the rest only when it is needed.
- Wakes to a parent that is mid-turn **coalesce** into one message at its
  turn end.
- Questions about threads go through `wf.board()`, `wf.inbox()` or a short
  `t3.thread_read` snippet in `execute`, which answer in a few lines, in
  place of pasting whole threads into context.

Only the **root**, the node with no parent, speaks to the user. T3 settles a
thread at every turn end, so a settled thread is not a finished one: only a
`handoff` or `closed` message closes a link.

## Reaching t3cm

`t3cm` is the single engine for the T3 workflow. Reach it in this order:

1. **`mcp__t3cm__execute` is listed:** call it with an async arrow function.
2. **It is not listed** (Codex, any other non-Claude agent, or a session
   that started before t3cm connected): pipe the same async arrow function
   to `t3cm exec -`, using `~/.local/bin/t3cm` when `t3cm` is not on `PATH`.
   It talks to the same service, uses the same store, and reads this
   thread's id from `$T3CODE_THREAD_ID`.
3. **The service cannot be reached:** send the start-the-service action line
   in [codemode.md, When the service is down](codemode.md#when-the-service-is-down)
   and stop. There is no fallback.

The full code-mode guide, the board, and installing t3cm:
[codemode.md](codemode.md).

## The link calls

Run every call below inside `mcp__t3cm__execute`, or pipe the same
`async () => ...` function to `t3cm exec -`:

| Moment | Call |
|---|---|
| Root thread starts | `await wf.init({ role: "overseer" })` |
| Ticket starts with no brief link | `await wf.init({ role: "ticket" })` joins the board under the thread's T3 title, or under `label` when you pass one; `root: true` keeps it off the board |
| This thread starts with a link block in its brief | the block's `wf.join(...)` call |
| Before you launch a child | `const { brief } = await wf.openChild({ role: "<child role>", label: "<label>" })`; put `brief` at the end of the launch message. The child binds the link when it joins |
| Progress worth knowing, nothing for the parent to do | `await wf.send({ kind: "note", body: "<one line>" })` |
| The parent must act or decide | `await wf.send({ kind: "event", body })` (or `blocker`, `report`) |
| A message to a child | `await wf.reply({ to: "<child id or label>", body })` |
| You start waiting on a person | `await wf.send({ kind: "gate", gate: "<name>", body: actionLine })`; later `await wf.gate({ clear: true })` |
| You wait on something else for a long spell, with nothing to send | `await wf.gate({ open: "<what>" })`; `await wf.gate({ clear: true })` when it ends |
| The work is parked on purpose, with nothing to wait on | `await wf.pause()` (a parent passes `child: "<label>"`); `await wf.pause({ paused: false })` resumes it |
| Merging a PR on the user's word | `await wf.merge({ repo: "<owner>/<name>", pr: <n>, head: "<sha>", method: "<method>" })` |
| The work is done | `await wf.send({ kind: "handoff", body })` (closes the link) |
| The work stopped without finishing | `await wf.send({ kind: "closed", body })` (closes the link) |
| A child's work ends without its handoff | `await wf.close({ child: "<label or thread id>" })` |
| Where do my children stand? | `(await wf.board()).children` (live status, gate, link) |
| Notes not yet seen | `await wf.inbox()` |
| A body cut short with `wf.show("mN")` | `(await wf.show("mN")).body` |
| This thread takes over a retiring thread's children | `await wf.init({ role: "<same role>" })`, then `await wf.adopt({ from: "<old id>" })` |
| This thread is about to finish | `await wf.sweep()` |

Which kinds wake the parent is set per link by t3cm's `roles` config
(`wake`, `bodyCap`): a ticket's events reach an overseer as notes. Pass
`wake: true` or `wake: false` on `wf.send` to override one message.

## Messages that arrive

A protocol message opens with `[t3cm <kind> from=<label> msg=<handle>]`.
Act on its body as your role says; nothing needs acknowledging. Lines under
"Notes since your last wake" are the digest. A "handled for you" line
there is a message t3cm routed past this thread, such as a worker's blocker
it took to the board, or the board's answer it sent back to the worker.

- **nudge** (from the watcher): this thread sat idle with its parent link
  open and no gate. Run your role's resume step, then say what changed
  (`kind: "note"` when nothing needs the parent), open a gate for a wait, or
  send the handoff. After `miss` unanswered nudges the watcher escalates to
  the parent.
- **escalate** or **report** (about a child): peek the child thread in
  `execute` ([codemode.md, Peek a thread](codemode.md#peek-a-thread)), then
  reply, relaunch, or close the link. At the root, show it to the user as an
  action line.

## Slots

The roles' review, passes, credential renewal, delivery and models are
**slots**, set per user and per repository and resolved by the `t3cm`
service. Each step looks up only the slot it needs at the point of use with
`wf.slot(name)` (or `wf.renewal({ output })` when a command fails). A model
slot can name a t3cm pool, which spreads launches across several accounts.
From a shell, `t3cm slots` prints the effective settings and `t3cm doctor`
checks the whole install. To set or change slots, run the
`setup-t3-workflow` skill.

How a role acts on each slot result, including a skill that no longer
resolves: [slots.md, Looking up slots in a role](slots.md#looking-up-slots-in-a-role).
Config file, `AGENTS.md` keys and skill search: [slots.md](slots.md).

## Filed issues

Every thread that files a GitHub issue reports it to the t3cm board with
`wf.issue`, as part of filing it. The rule and its single copy are in
[rules/report-filed-issues.md](rules/report-filed-issues.md); follow it.

Most issues are filed from threads that run no T3 role skill and load no
part of this one, so the rule cannot live here alone. `t3cm skills install`,
`t3cm skills update` and `setup-t3-workflow` install it where each agent
loads instructions in every session: Claude Code's user rules folder, and a
marked block in Codex's global `AGENTS.md`.

## What each role supplies

- **Resume step:** what to re-check when nudged.
- **Link numbers:** t3cm's `roles` config sets, per child role, `down` (idle
  minutes before a nudge), `miss`, `stall` (busy minutes with nothing sent
  before the parent hears of it), and optionally `wake` and `bodyCap`.
  `t3cm doctor` validates the config.

A child the watcher sees running, holding background tasks, or blocked on a
thread wait is busy and never nudged. A child waiting on an approval or a
question in its own thread is reported to the parent once.

## Action lines

A message that needs the user to do something opens with one action line per
action:

`**ACTION NEEDED:** <plain verb phrase>, here: <bare URL>`

For example:

`**ACTION NEEDED:** reply "merge" to merge #81 at head d4db97c, or review it first, here: https://github.com/acme/widgets/pull/81`

- The verb phrase names what the user clicks, runs, or answers, in plain
  words. The protocol's own terms, such as "gate", "link" and "nudge", stay
  out of it.
- The URL is bare text, with no markdown link label, and points at the thing
  to act on. When the action is a command, put the exact command in
  backticks after `here:`. When there is nothing to open or run, end the
  line on the verb phrase.
- When the user answers by replying, quote each answer they can give after
  `reply`, `answer` or `say` on the action line, as in the example. The
  board turns each such quoted answer into a button, up to 8, and drops any
  answer over 60 characters. Quoted text elsewhere in a message with an
  action line makes no button. Keep file and branch names out of quotes on
  that line, or they become buttons too.
- Context, timings, and evidence come after the action lines.
- A message that needs nothing from the user carries no marker at all.

A gate sent to the board can set its card's buttons with
`wf.send({ kind: "gate", gate, body, options: ["merge", "hold"] })`. A
thread parent gets only the body, so there the action line carries the
answers. See
[codemode.md](codemode.md#the-board-is-the-overseer).

Only the root shows action lines to the user. A thread with a parent sends
each one up with `kind: "gate"`, and may also show it in its own thread.

**Thread links.** Name a thread to the user by the `link` field T3 returns
for it, verbatim; `wf.board()` returns each child's.
