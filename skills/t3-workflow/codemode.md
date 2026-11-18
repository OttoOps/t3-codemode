# t3-workflow over code mode (t3-codemode)

Read this when the `t3cm` MCP server is connected, so its two tools,
`mcp__t3cm__search` and `mcp__t3cm__execute`, are listed. It replaces every
`t3wf` command and most direct `mcp__t3-code__*` calls: the protocol state,
message bodies, notes, the waiting list and your own scratch state live in
the service, and only what your code returns enters your context.

## The two tools

- **search** `{code}`: an async arrow function over `catalog` (every T3 tool
  plus the `wf`, `mem` and `vars` API, each with a TypeScript `signature`).
  Return only what you need: a few names, or one signature.
- **execute** `{code, session?}`: an async arrow function run in a sandbox
  with `t3`, `wf`, `mem`, `vars` and `console`. It returns the function's
  return value and nothing else. TypeScript annotations are fine.

Return the smallest answer that settles the question: counts, ids, one line
per item. Keep anything bigger in `vars` (yours, per session) or `mem.ns(...)`
(shared) and return a handle to it.

```js
async () => {
  const { threads } = await t3.thread_list({ allProjects: true, limit: 200 });
  vars.failed = threads.filter(t => t.status === "failed").map(t => t.threadId);
  return { failed: vars.failed.length };
}
```

## The link protocol in code

The rules are the same as in SKILL.md: notes never wake, wakes carry a capped
body and a handle, wakes to a busy thread coalesce, and the service's watcher
nudges idle children and escalates. Only the calls change.

| Moment | In `execute` |
|---|---|
| Root thread starts | `await wf.init({ role: "overseer" })` |
| Ticket starts with no brief link | `await wf.init({ role: "ticket" })` joins the board under the thread's T3 title, or under `label` when you pass one. `root: true` keeps it off the board |
| Before launching a child | `const { brief } = await wf.openChild({ role: "ticket", label: "widgets#41" })`; paste `brief` at the end of the launch message |
| Child starts | the `wf.join(...)` line from its brief |
| Routine progress | `await wf.send({ kind: "note", body: "tests green at d4db97c" })` |
| The parent must act | `await wf.send({ kind: "event", body })`, or `blocker`, `report` |
| Waiting on a person | `await wf.send({ kind: "gate", gate: "merge", body: actionLine })`; later `await wf.gate({ clear: true })` |
| Done | `await wf.send({ kind: "handoff", body })` |
| Message a child | `await wf.reply({ to: "widgets#41", body })` |
| Children at a glance | `return (await wf.board()).map(c => `${c.label} ${c.status} ${c.gate ?? ""}`)` |
| A capped body in full | `return (await wf.show("m42")).body` |
| Unread notes | `return (await wf.inbox()).map(n => `${n.label}: ${n.head}`)` |

A wake arrives as a message whose first line is
`[t3cm <kind> from=<label> msg=<handle>]`. Act on it as your role says.

## The board is the overseer

The service is the root. Its page, `http://127.0.0.1:47731/`, shows every
ticket with its live status and PR, one card per decision the user owes (a
gate, blocker, report or escalation, with reply buttons parsed from its
action line), an activity feed, and an ask box. No overseer thread runs, so
no ticket message lands in any model's context.

- **Starting tickets.** From any thread or the page's launch form:
  `await wf.launchTicket({ issue: "https://github.com/o/r/issues/41" })`.
  The service picks the T3 project from the issue's repo, launches a
  t3-ticket thread, and the brief has it join with
  `wf.join({ parent: "board", label, role: "ticket" })`. A ticket started
  in a thread instead runs `wf.init({ role: "ticket" })`, which joins it to
  the board the same way, labeled from the thread's T3 title.
- **Answers.** The user clicks a card's button or types an answer, or
  answers in a thread with `wf.answer("a3", "merge")`; the ticket gets one
  `[t3cm reply]` and the card resolves. A ticket's own handoff, a cleared
  gate, or a newer gate resolves its older cards without the user.
- **Questions.** The ask box starts a throwaway thread that reads the board
  with `wf.board({ parent: "board", all: true })`, `wf.cards({ all: true })`
  and `wf.feed()`, writes the answer onto the page, and is archived.

Slots have no `wf` route: read them with the `t3wf slots` script, as
[slots.md](slots.md#reading-slots-in-a-role) says, in code mode too.

## Per role

**Ticket.** Join from the brief, or `wf.init({ role: "ticket" })` when the
brief has no link block, send gates, blockers and the handoff to the
board with `wf.send`, and progress as notes. Resume step: `wf.board()` for
this work's workers. Hand a worker its next PR's context through shared
memory instead of pasting it:

```js
async () => {
  await mem.ns("work:widgets#41").set("handoff:pr1", handoffText);
  return "stored";
}
```

and the next worker reads it with
`(await mem.ns("work:widgets#41").get("handoff:pr1"))`.

**Worker.** Join from the brief, send routine progress as notes, the PR as
an `event`, the merge request as a `gate`, and the handoff at the end. On a
`[t3cm nudge`, run the resume step, then send a note, open a gate, or hand
off.

**An overseer thread** (only when the user asks for one in chat) keeps its
waiting list in the service: on a `gate`, `wf.waiting.add({ text, from,
link })` and show the user the action line with its id; on the user's
answer, `wf.reply` to the ticket and `wf.waiting.resolve(id)`. Status
questions go through `wf.board()` and `wf.inbox()`, never `t3_thread_read`.

## When the service is down

`execute` fails to connect: fall back to `t3wf` as SKILL.md says, and tell
the user once: **ACTION NEEDED:** start the code-mode service, here:
`systemctl --user start t3-codemode` (macOS:
`launchctl kickstart gui/$(id -u)/com.ottoops.t3-codemode`, or when the agent
is not loaded,
`launchctl enable gui/$(id -u)/com.ottoops.t3-codemode && launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.ottoops.t3-codemode.plist`;
Windows:
`Enable-ScheduledTask t3-codemode; Start-ScheduledTask t3-codemode`).

## Installing t3cm

t3cm is optional. Without it, `t3wf` runs the whole protocol. Offer it at
most once per session, and install only after the user says yes: it adds a
per-user service (a systemd user unit, a launchd agent, or a Windows logon
task) and a user-scope MCP server.

1. **Release first.** The installer downloads the latest release, checks it
   against `SHA256SUMS`, installs `t3cm` and its service, and registers the
   MCP server:
   - Linux or macOS: fetch
     https://raw.githubusercontent.com/OttoOps/t3-codemode/main/deploy/install.sh
     and run `bash install.sh`.
   - Windows: fetch `deploy/install.ps1` from the same place and run
     `powershell -ExecutionPolicy Bypass -File install.ps1`.
   - On macOS, run it from a GUI login. Over SSH alone, add `--files-only`
     and have the user run the `launchctl` command it prints after logging in.
   - To update later, run it again.
2. **No release yet, or an unsupported machine** (the installer says so):
   build from source on Linux or macOS.
   - Check `rustc --version` reports 1.95 or newer. If Rust is missing or
     older, ask the user to install or update it through rustup, and stop
     until they have.
   - Clone https://github.com/OttoOps/t3-codemode, or `git pull` in an
     existing checkout.
   - Run `deploy/install.sh --from-source` there. The build takes a few
     minutes, so run it in the background.
3. **Verify.** `curl -s http://127.0.0.1:47731/healthz` must show
   `"connected": true` under `t3` (Windows:
   `(Invoke-RestMethod http://127.0.0.1:47731/healthz).t3.connected` must be
   `True`), and `claude mcp get t3cm` must show an HTTP entry for
   `http://127.0.0.1:47731/mcp` with the `X-T3-Thread` header. When the
   installer printed the `claude mcp add` command instead, run it first. When an
   older `t3cm` entry points elsewhere, ask the user before replacing it.
4. **Tell the user** the MCP tools load at session start, so new threads use
   t3cm and this one stays on `t3wf`. The board is at http://127.0.0.1:47731/.
