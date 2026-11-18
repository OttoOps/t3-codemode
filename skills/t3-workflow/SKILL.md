---
name: t3-workflow
description: The link protocol under every T3 workflow thread - parent and child links watched by a machine-wide watcher, notes, wakes, gates, nudges, escalation, message handles and code mode, run through the t3wf script. Use when t3-overseer, t3-ticket or t3-worker says to, when a message opens with a [t3wf2 header, when a launch brief carries a "t3-workflow link" block, or when the user asks how the T3 workflow works, what its slots (reviewer, passes, delivery, models) are set to, or about installing t3cm. It is the base the role skills run on, not a role of its own.
---

Every thread in the T3 workflow is a **node** with at most one parent
and any number of children. A parent and child pair is a **link**. One
registry per machine holds every link and every message body, and a
**watcher** (`t3wf tick`, every 2 minutes) reads each child's live state from
T3 and does all the timekeeping. Threads keep no timers and never check in.

The protocol keeps bulk out of context. A thread's context is re-read on
every model call, so each wake and each pasted payload costs again on every
later call:

- Routine progress is a **note**: stored, never a wake. It reaches the parent
  as one digest line at the parent's next wake.
- A wake carries a capped body and a **handle** (`m42`); `t3wf show m42`
  prints the rest only when it is needed.
- Wakes to a parent that is mid-turn **coalesce** into one message at its
  turn end.
- Questions about threads go through `t3wf board`, `t3wf peek` or
  `t3wf exec`, which answer in a few lines, in place of `t3_thread_read` or
  `t3_thread_list`, which paste whole threads into context.

Only the **root**, the node with no parent, speaks to the user. T3 settles a
thread at every turn end, so a settled thread is not a finished one: only a
`handoff` or `closed` message closes a link.

## t3wf

When the `t3cm` MCP server is connected (`mcp__t3cm__execute` is listed),
run this protocol through it instead of `t3wf`: [codemode.md](codemode.md).
When it is not, `t3wf` below is the full protocol. If the user asks how the
T3 workflow works, how to make it cheaper or faster, or about t3cm itself,
mention t3cm once and offer to install it:
[codemode.md, Installing t3cm](codemode.md#installing-t3cm).

`t3wf` means `python3 <this skill's base directory>/scripts/t3wf.py`. `--self`
defaults to `$T3CODE_THREAD_ID`, which T3 sets in each thread's shell; `t3wf
whoami` prints it. Each command prints its outcome in a line or two. A
command that fails says why on stderr: fix what it names and run it again.

| Moment | Command |
|---|---|
| This thread starts with no parent | `t3wf init --role <role>` |
| This thread starts with a "t3-workflow link" block in its brief | the block's `t3wf join` line |
| Before you launch a child | `t3wf open-child --role <child role> --label '<label>'`; put its block at the end of the brief. The child binds the link when it joins. |
| Progress worth knowing, nothing for the parent to do | `t3wf send --kind note --body '<one line>'` |
| The parent must act or decide | `t3wf send --kind event` (or `blocker`, `report`) `--body -` |
| A message to a child | `t3wf reply --to <child id or label> --body -` |
| You start waiting on a person | `t3wf send --kind gate --gate <name> --body '<action line>'`; later `t3wf gate --clear` |
| You wait on something else for a long spell, with nothing to send | `t3wf gate --open <what>`; `t3wf gate --clear` when it ends |
| The work is done | `t3wf send --kind handoff --body -` (closes the link) |
| The work stopped without finishing | `t3wf send --kind closed --body -` (closes the link) |
| A child's work ends without its handoff | `t3wf close --link <child id>` |
| Where do my children stand? | `t3wf board` (one line each, live status) |
| One thread in a few lines | `t3wf peek <label or thread id>` |
| Notes not yet seen | `t3wf inbox` |
| A body cut short with `t3wf show mN` | `t3wf show mN` |
| This thread takes over a retiring thread's children | `t3wf init --role <same role>`, then `t3wf adopt --from <old id>` |
| This thread is about to finish | `t3wf sweep` |

Bodies with quotes or several lines go through a quoted heredoc:

```bash
t3wf send --kind event --body - <<'MSG'
PR 1/2 opened: https://github.com/o/r/pull/42 (head d4db97c)
MSG
```

Which kinds wake the parent is set per link by the parent role's
`t3-role.json` (`wake`, `bodyCap`) over `defaults.json`: a ticket's events
reach an overseer as notes. `--wake` or `--no-wake` overrides one message.

## Messages that arrive

A protocol message opens with `[t3wf2 <kind> from=<label> id=<thread>
msg=<handle>]`. Act on its body as your role says; nothing needs
acknowledging. Lines under "Notes since your last wake" are the digest.

- **nudge** (from the watcher): this thread sat idle with its parent link
  open and no gate. Run your role's resume step, then say what changed
  (`--kind note` when nothing needs the parent), open a gate for a wait, or
  send the handoff. After `miss` unanswered nudges the watcher escalates to
  the parent.
- **escalate** or **report** (about a child): `t3wf peek` it, then reply,
  relaunch, or close the link. At the root, show it to the user as an action
  line.

## Slots

The roles' review, passes, credential renewal, delivery and models are
**slots**, set per user and per repository. `t3wf slots` prints the
effective settings and how to run each skill; `t3wf doctor` checks them.
How a role acts on each line, including a skill that no longer resolves:
[slots.md, Reading slots in a role](slots.md#reading-slots-in-a-role).
Config file, `AGENTS.md` keys and skill search: [slots.md](slots.md).

## What each role supplies

- **Resume step:** what to re-check when nudged.
- **Link numbers:** the role's `t3-role.json` sets, per child role, `down`
  (idle minutes before a nudge), `miss`, `stall` (busy minutes with nothing
  sent before the parent hears of it), and optionally `wake` and `bodyCap`.
  `t3wf check-config` validates them.

A child the watcher sees running, holding background tasks, or blocked on a
thread wait is busy and never nudged. A child waiting on an approval or a
question in its own thread is reported to the parent once.

## The watcher

`t3wf daemon status` says whether it runs. `t3wf daemon install` sets up a
systemd user timer (`t3wf-tick.timer`) that survives sessions. Without it,
every t3wf command runs a pass when the last one is over 10 minutes old, so
links stay watched while threads are active.

The registry lives in `$XDG_STATE_HOME/t3-workflow/v2/` (by default
`~/.local/state/t3-workflow/v2/`); `T3WF_STATE_DIR` overrides the
`t3-workflow` folder. The first t3wf command on a machine that still has the
registry under `~/.claude/cbrip/t3-workflow/v2/` moves it here, leaves a
symlink at the old path, and swaps a `cbrip-t3wf-tick.timer` for
`t3wf-tick.timer`, so links already open stay open.

## Code mode

`t3wf exec` runs a Python snippet outside the model with these names bound:
`t3` (every T3 tool, as `t3.t3_thread_list(allProjects=True)` or
`t3.call(name, **args)`), `vars` (a dict kept per thread between runs),
`registry()` (the link registry), and `show(obj)`. Only what it prints comes
back, capped at 6000 characters. Use it for any question that would
otherwise pull a large T3 payload into context: filter and count in the
script, keep the bulk in `vars`, print the answer.

```bash
t3wf exec - <<'PY'
bad = [t for t in t3.t3_thread_list(allProjects=True, limit=200)["threads"]
       if t["status"] in ("failed", "interrupted")]
vars["failed"] = [t["threadId"] for t in bad]
show([(t["title"], t["link"]) for t in bad])
PY
```

## Action lines

A message that needs the user to do something opens with one action line per
action:

`**ACTION NEEDED:** <plain verb phrase>, here: <bare URL>`

For example:

`**ACTION NEEDED:** reply "merge" to merge #81, or review it first, here: https://github.com/acme/widgets/pull/81`

- The verb phrase names what the user clicks, runs, or answers, in plain
  words. The protocol's own terms, such as "gate", "link" and "nudge", stay
  out of it.
- The URL is bare text, with no markdown link label, and points at the thing
  to act on. When the action is a command, put the exact command in
  backticks after `here:`. When there is nothing to open or run, end the
  line on the verb phrase.
- Context, timings, and evidence come after the action lines.
- A message that needs nothing from the user carries no marker at all.

Only the root shows action lines to the user. A thread with a parent sends
each one up with `--kind gate`, and may also show it in its own thread.

**Thread links.** Name a thread to the user by the `link` field T3 returns
for it, verbatim; `t3wf board` prints each child's.
