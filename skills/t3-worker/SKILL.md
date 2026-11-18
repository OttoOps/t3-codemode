---
name: t3-worker
description: Worker side of t3-ticket - build one PR in this T3 thread's worktree, review it with the configured reviewer, follow it to merged and deployed, hand off to the orchestrator. Use only from a t3-ticket launch brief or when told to be an issue's PR worker. A user handing over an issue to work is t3-ticket.
---

This role runs on the `t3-workflow` link protocol, as a child of the
orchestrator. Invoke that skill first. This skill's `t3-role.json` says a
worker opens no links of its own.

The worker procedure lives in one place: [procedure.md](procedure.md), next
to this file. Read it and follow its body inline, in this thread, from "The
link protocol". Its Publishing section governs the PR description and any
PR or issue comment you write.

Follow it here rather than handing it to a subagent. A subagent's tool calls
happen out of sight and only its summary comes back, and a subagent cannot
own a PR watch. Run inline, every edit, push, and watcher wake stays on this
thread's timeline, where the user and the orchestrator can review it.
