# t3-workflow over code mode (t3-codemode)

The `t3cm` service runs the T3 workflow. Every thread reaches it through its
two MCP tools, `mcp__t3cm__search` and `mcp__t3cm__execute`, or, when those
tools are not listed (Codex, other non-Claude agents, or a session started
before t3cm connected), by piping the same async arrow function to
`t3cm exec -` (`~/.local/bin/t3cm` when `t3cm` is not on `PATH`). The
protocol state, message bodies, notes, slots, the waiting list and your own
scratch state live in the service, and only what your code returns enters
your context.

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

### Peek a thread

Read a child or retiring thread in a few lines without pulling its whole
transcript into context:

```js
async () => {
  const { thread } = await t3.thread_read({ threadId: "<id>", limit: 1, maxCharsPerItem: 1 });
  const { items } = await t3.thread_read({
    threadId: "<id>",
    limit: 3,
    maxCharsPerItem: 300,
    beforePosition: Math.max(thread.itemCount ?? 0, 1) + 1,
  });
  const tail = (items ?? []).map(m => `${m.role ?? m.type ?? "item"}: ${(m.text ?? "").slice(0, 300)}`);
  return { title: thread.title, status: thread.status, link: thread.link, tail };
}
```

## The link protocol in code

Notes never wake, wakes carry a capped body and a handle, wakes to a busy
thread coalesce, and the service's watcher nudges idle children and
escalates.

| Moment | In `execute` (or `t3cm exec -`) |
|---|---|
| Root thread starts | `await wf.init({ role: "overseer" })` |
| Ticket starts with no brief link | `await wf.init({ role: "ticket" })` joins the board under the thread's T3 title, or under `label` when you pass one. `root: true` keeps it off the board |
| Before launching a child | `const { brief } = await wf.openChild({ role: "ticket", label: "widgets#41" })`; paste `brief` at the end of the launch message |
| Child starts | the `wf.join(...)` line from its brief |
| Routine progress | `await wf.send({ kind: "note", body: "tests green at d4db97c" })` |
| The parent must act | `await wf.send({ kind: "event", body })`, or `blocker`, `report` |
| Waiting on a person | `await wf.send({ kind: "gate", gate: "merge", body: actionLine })`; later `await wf.gate({ clear: true })` |
| Merging a PR on the user's word | `await wf.merge({ repo: "acme/widgets", pr: 41, head: sha, method: "rebase" })` returns `queued`, `merged`, `waiting` or `refused` |
| Done | `await wf.send({ kind: "handoff", body })` |
| Message a child | `await wf.reply({ to: "widgets#41", body })` |
| Children at a glance | `return (await wf.board()).children.map(c => `${c.label} ${c.status} ${c.gate ?? ""}`)` |
| A capped body in full | `return (await wf.show("m42")).body` |
| Unread notes | `return (await wf.inbox()).map(n => `${n.label}: ${n.head}`)` |
| One slot at point of use | `await wf.slot("review")` (or `"passes"`, `"delivery"`, `"models"`) |
| Match a failed command against renewals | `await wf.renewal({ output })` |

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
- **Buttons.** A `gate`, `blocker` or `report` with
  `options: ["merge", "hold"]` gets exactly those buttons, each one line of
  1 to 60 characters, no two the same ignoring case, at most 8; a bad entry
  fails the send. Without `options`, the buttons are the quoted answers after
  the first `reply`, `answer` or `say` on each `**ACTION NEEDED:**` line,
  and a body with no action line gets one button per `reply "X"` anywhere
  in it. A send that opens a card returns its buttons as `options`, and a gate with
  none also returns a `warning`: send it again with `options`.
- **Answers.** The user clicks a card's button or types an answer, or
  answers in a thread with `wf.answer("a3", "merge")`; the ticket gets one
  `[t3cm reply]` and the card resolves. A ticket's own handoff, a cleared
  gate, or a newer gate resolves its older cards without the user.
- **Questions.** The ask box starts a throwaway thread that reads the board
  with `wf.board({ parent: "board", all: true })`, `wf.cards({ all: true })`
  and `wf.feed()`, writes the answer onto the page, and is archived.

## Per role

**Ticket.** Join from the brief, or `wf.init({ role: "ticket" })` when the
brief has no link block, send gates, blockers and the handoff to the
board with `wf.send`, and progress as notes. Resume step: `wf.board()` for
this work's workers. Look up `wf.slot("delivery")` and `wf.slot("models")`
when each step needs them, and `wf.renewal({ output })` when a deploy-watch
command fails. Hand a worker its next PR's context through shared memory
instead of pasting it:

```js
async () => {
  await mem.ns("work:widgets#41").set("handoff:pr1", handoffText);
  return "stored";
}
```

and the next worker reads it with
`(await mem.ns("work:widgets#41").get("handoff:pr1"))`.

A ticket merges every PR with `wf.merge`, never with `gh pr merge` or T3's
own merge button or PR action: on a repo with a merge queue, T3 turns on
auto-merge, and a PR whose checks have already passed never joins the
queue. `wf.merge` reads per PR whether the repo has a queue, so no skill or
config lists those repos. Its merge gate names the approved head
(`head <sha>`), and the board holds the approval to that head. After a
`queued` or `waiting` outcome the ticket polls nothing: the board's GitHub
reconcile wakes it when the PR merges, closes or leaves the queue, as
t3-ticket's "The merge wait" says.

**Worker.** Join from the brief, send routine progress as notes, the PR as
an `event`, the merge request as a `gate` that names its head
(`head <sha>`), and the handoff at the end. Look up `wf.slot("review")`
before opening the PR, `wf.slot("passes")` in Publishing, and
`wf.renewal({ output })` when a command fails. It never merges. On a
`[t3cm nudge`, run the resume step, then send a note, open a gate, or hand
off.

**An overseer thread** (only when the user asks for one in chat) keeps its
waiting list in the service: on a `gate`, `wf.waiting.add({ text, from,
link })` and show the user the action line with its id; on the user's
answer, `wf.reply` to the ticket and `wf.waiting.resolve(id)`. Status
questions go through `wf.board()` and `wf.inbox()`, never `t3_thread_read`.

## When the service is down

When `execute` or `t3cm exec -` cannot reach the service, stop and tell the
user once: **ACTION NEEDED:** start the code-mode service, here:
`systemctl --user start t3-codemode` (macOS:
`launchctl kickstart gui/$(id -u)/com.ottoops.t3-codemode`, or when the agent
is not loaded,
`launchctl enable gui/$(id -u)/com.ottoops.t3-codemode && launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.ottoops.t3-codemode.plist`;
Windows:
`Enable-ScheduledTask t3-codemode; Start-ScheduledTask t3-codemode`).

When `wf.slot` or `wf.renewal` fails with an unknown-method error, the
running binary is older than these skills. Stop and tell the user once:
**ACTION NEEDED:** update t3cm so the service matches these skills, here:
`t3cm update`.

## Installing t3cm

`t3cm` is required for the T3 workflow. Install only after the user says
yes: it adds a per-user service (a systemd user unit, a launchd agent, or a
Windows logon task) and a user-scope MCP server for Claude Code.

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
   - With no terminal to ask on, it also installs the workflow skills with
     `t3cm skills install`, and replaces an `npx skills` or copied install
     with `t3cm skills install --take-over`. Pass `--no-workflow`
     (`-NoWorkflow`) only when the user says to leave the skills alone.
   - To update later, run `t3cm update` or the installer again. Both bring
     the skills t3cm installed to the new release.
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
   `True`). Run `claude mcp get t3cm` from the project directory where the
   next thread will run, because project-scope MCP entries override
   user-scope ones: it must show an HTTP entry for
   `http://127.0.0.1:47731/mcp` with the `X-T3-Thread` and
   `X-T3-Config-Dir` headers. When the installer printed the
   `claude mcp add` command instead, run it first. When an older `t3cm`
   entry points elsewhere, ask the user before replacing it.
4. **Tell the user** Claude Code loads MCP tools at session start, so new
   Claude sessions use `mcp__t3cm__execute` directly, and this session (and
   non-Claude agents such as Codex) reaches the same service through
   `t3cm exec -`. The board is at http://127.0.0.1:47731/.
