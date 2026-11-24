# Execute globals

These are the only globals inside `execute`. There is no fs, require, process,
fetch, import or timers.

| Global | What it is |
|---|---|
| `t3` | Every T3 tool by exact name (`t3.t3_thread_list(input)`), an alias without the `t3_` prefix, and `t3.call(name, input)`. A failed call throws an `Error` with T3's message. |
| `<ns>` | One per enabled upstream, named by its `name`, present even before its tool list is known: each allowed tool by exact name (`cbm.search_graph(input)`, `cbm["get-thing"](input)`), a sanitized alias when it is free (`cbm.get_thing`), and `cbm.call(name, input)`. A tool named `call` is reached with `cbm.call("call", input)`. |
| `tools` | `tools.call("ns.tool", input)` for any namespace, `t3` included (which keeps the `t3_` alias and the deny list). The name splits at the first dot, so `tools.call("cbm.a.b")` calls tool `a.b`. |
| `mem` | `get(key)`, `set(key, value)`, `del(key)`, `list(prefix?)` on the session namespace; `mem.ns(name)` opens a shared one (for example `work:widgets#41`). JSON values, 1 MB each. |
| `vars` | A plain object restored before the run and saved after it (also after a thrown error; a CPU or wall kill loses that run's changes). REPL state across calls and restarts. |
| `wf` | The workflow layer, acting as the calling thread. |
| `console` | `log`/`info`/`debug`/`warn`/`error`, captured to `logs` (50 lines / 4,000 chars). |

The full signatures are on the [API reference](api.md) page.

## t3

```js
async () => {
  const { threads } = await t3.thread_list({ allProjects: true, limit: 20 });
  return threads.filter(t => t.status === "running").map(t => t.link);
}
```

`t3.thread_list` and `t3.t3_thread_list` are the same tool. A tool on the
config's `deny` list throws without reaching T3.

## Upstream namespaces

Each enabled upstream adds one frozen global named after it. With the `cbm`
upstream from the [Upstreams](upstreams.md) example:

```js
async () => {
  try {
    return await cbm.list_projects({});
  } catch (e) {
    return `cbm failed: ${e.message}`;
  }
}
```

`tools.call("cbm.list_projects", {})` reaches the same tool by its qualified
name. Use `search` to find an upstream's tool names; `catalog` is not a global
inside `execute`.

## mem and vars

`mem` is a key-value store on the session namespace. The session defaults to
`thread:<caller>`, so each thread gets its own, and `mem.ns(name)` opens a
namespace any thread can share:

```js
async () => {
  await mem.ns("work:widgets#41").set("handoff:pr1", "PR 1 merged at d4db97c");
  return await mem.ns("work:widgets#41").get("handoff:pr1");
}
```

`vars` is a plain object. Whatever a run leaves in it is there at the start of
the next run in the same session, across service restarts:

```js
async () => {
  vars.runs = (vars.runs ?? 0) + 1;
  return vars.runs;
}
```

## wf

`wf` is the thread-workflow layer. Threads form a tree of links, each between a
parent and a child, and the service keeps every link and message in its store.

- `wf.init({role, label?, root?, issue?, parentIssue?, part?})` starts a thread with no parent link. A role
  that `roles.overseer.children` lists, which is only `ticket` by default, joins
  the board the way `wf.join({ parent: "board", label, role: "ticket" })` does,
  under the board launch's label, `label`, or its T3 title, in that order. The
  result names `parent: "board"` and the label. `root: true` keeps the thread
  off the board as a standalone root. Every other role starts as a root.
  `wf.init` is for a ticket started on its own, but a ticket launched from the
  board that calls it instead of its brief's `wf.join` still clears its launch
  from the board's pending list. The service matches the launch by the thread
  T3 created for it, never by label, so a ticket started by hand under the
  same title leaves another launch on the pending list.
- `wf.openChild({role, label})` returns a `brief` block the parent pastes into
  the child's launch brief; the child runs the
  `wf.join({parent, label, role, down, miss, stall})` call it contains. When
  the parent launches the child with `t3.t3_thread_launch` in `execute`, the
  service records the new thread on its launch in the pending list, and the
  watcher's never-joined report links to it.
- `wf.openChild`, `wf.join`, `wf.init` and `wf.launchTicket` take an optional
  `issue`, `parentIssue` and `part`. `issue` is the GitHub issue the link
  works, `parentIssue` the epic it belongs to, each as `<owner>/<repo>#<n>` or
  the issue's URL, and `part` says which of the epic's PRs it is, as `n/N`
  with `1 <= n <= N`, such as `"1/4"`. The link stores them as
  `<owner>/<repo>#<n>` and `n/N`, and a malformed value fails the call.
  `wf.openChild` writes them into the `wf.join` call in its brief. A rejoin
  that leaves one out keeps the value the link already has. `wf.board()` and
  the board's `/api/state` return all three on each ticket and worker, `null`
  when unset. A worker link with no `issue` takes one from its pull request's
  closing references (`Closes #N`) at the next
  [GitHub reconcile](board.md#github-reconcile).
- `wf.openChild` also takes `pr: { format?, sections?, passes?, mergeOnly?, method? }`,
  the PR description format a worker's [`wf.pr`](#wfpr) `opened` event is
  checked against. It is stored on the child's link when the child joins.
  `method` is the repo's merge method, `merge`, `squash` or `rebase`, and a
  brief may give it with no `format`. With it,
  the service merges the PR on the board's answer to its merge card
  ([The board's merge answer](#the-boards-merge-answer)).
- `wf.send({kind, body, gate?, wake?, options?})`, kinds `note | event | gate |
  blocker | report | handoff | closed`, to the caller's parent. Returns
  `{handle, delivery}`: `noted`, `sent`, `queued` (the parent is mid-turn, or
  delivery failed; the watcher retries), or `board` when the parent is the
  board. When the message opened a decision card, the result also carries the
  `card` id and the card's reply buttons as `options`, plus a `warning` when a
  gate's card has no buttons. `gate` also opens a gate, which stops nudges
  until the sender clears it. `handoff` and `closed` close the link. A
  worker's `blocker`, or any gate it opens other than `merge` and
  `delivery:<id>`, goes to the board past a board ticket, with `delivery: "board"`, as
  [`relay`](config.md#workflow-layer) describes.
- `options`, a list of answers on a `gate`, `blocker` or `report`, sets the
  board card's buttons in place of the ones parsed from the body. The limits
  and the parsing rules are in [Reply buttons](board.md#reply-buttons). A bad
  entry fails the send. A thread parent gets only the body.
- `wf.reply`, `wf.gate`, `wf.close`, `wf.inbox`, `wf.show`, `wf.board`,
  `wf.adopt` and `wf.sweep` cover the rest of a link's life.
- `wf.pause({ child?, paused? })` pauses a link that is parked on purpose:
  the caller's own parent link, or with `child` one of its child links.
  `paused: false` resumes it. The [board](board.md#pausing) shows it as
  paused, not idle.
- `wf.launchTicket`, `wf.dispatch`, `wf.cards`, `wf.answer` and `wf.feed` work
  with the [board](board.md).
- `wf.issue({ url, title?, note?, blockedBy?, parent?, track?, priority?, sync? })`
  reports a GitHub issue the calling thread just filed. The board lists it under
  [new issues](board.md#new-issues) with the filing thread and the time. The call
  needs the caller's thread id but no parent link, so any thread can make it. It
  returns `{ added, message, issue }`. When the issue is already on the list,
  dismissed or not, or already has a board ticket, the call adds nothing. Its
  result then has `added: false`, a `reason` of `reported` or `ticket`, the
  existing `issue` or `ticket`, and a `message` that says which. Passing any of
  `blockedBy`, `parent`, `track`, or `priority` enables GitHub wiring by default
  unless `sync` is `false`. Each entry in `blockedBy` or `parent` accepts `#N`,
  `owner/repo#N`, or a GitHub issue URL, `track` takes a flow track slug, and
  `priority` takes `"high"`, `"medium"`, or `"low"`. The service wires GitHub's
  native `addBlockedBy` and `addSubIssue` links and applies the `track: <track>`
  and `priority: <priority>` labels idempotently, creating missing labels when
  the repo lacks them. Existing links and labels succeed as `alreadyLinked` or
  `alreadyApplied`. GitHub and permission errors land in `wiring.errors` and in
  `message`. You can also sync body-parsed links to GitHub from the board's
  [Graph tab](board.md#the-graph-tab) through `POST /api/flow/sync-links`.
- `wf.fileIssues({ repo?, parent?, track?, priority?, labels?, queue?, report?, sync?, issues })`
  creates a batch of GitHub issues forming a dependency DAG in one call. See
  [wf.fileIssues](#wffileissues).
- `wf.pick({ pool })` picks a member of a configured [pool](config.md#pools)
  for one launch and returns `{ pool, pick, instanceId, driver, model, effort,
  options, reason, skipped, modelSelection }`. Pass `modelSelection` to
  `t3_thread_launch`. `reason` names the driver, model and effort chosen and
  says why that member, and `skipped` lists each member left out with
  its reason, such as `limited until 14:00 MST`. Under the `cheapest`
  strategy, `reason` names the cost, as in
  `claudeAgent claude-opus-5-5, effort high; cheapest: cost 0, weighted: weight 1 of 2`.
  The service records each pick, and `pick` is its id. When no member can
  run, it throws and names the pool.
- `wf.picked({ pick, thread })` records the thread a pick launched. The board
  offers to move a thread to another member when it stops on a usage limit,
  as [Moving a limited thread](config.md#moving-a-limited-thread) describes,
  and only for a thread with a recorded pick. Board launches record theirs on
  their own. It throws when the pick is unknown or already launched another
  thread.
- `wf.pools()` lists each pool, with `mixed` and the pool's `driver`, `model`
  and `effort` (`null` in a mixed pool), and its members with their `driver`,
  `model`, `effort`, `weight`, `reserve`, `cost`, `state` (`ready`, `limited`,
  `disabled`, `constrained`, `missing-model`, `missing-effort`, or `unknown`
  when T3 did not answer), `reason`, `limitedUntil`, `limitedUntilLocal`,
  `limitReason`, `usageLeft`, `running` threads, `picks` and `lastPick`.
  `limitReason` names the thread whose limit stop set the mark and where its
  reset time came from. `usageLeft` stays `null` until t3cm has a usage
  source. The watcher sets and clears the marks, as
  [Limits](config.md#limits) describes. Each pool also has `warnings`, which
  flags a member that is the only one that can run the pool's model, or in a
  mixed pool the only one of its driver that can run its model, as
  [Sole members](config.md#sole-members) describes.
- `wf.usageStats({ window })` reports how board ticket launches are routed,
  the view the board header shows as
  [Launch routing](board.md#launch-routing). `window` is `"all"` or a count
  of `m`, `h` or `d`, such as `30m`, `24h` or `7d`, and defaults to `24h`.
  The result has `window`, `since`, `at`, `mode`, `source`, `pool`,
  `strategy`, `error`, `warnings` and `members`. `since` is `null` for
  `all`, and `source` names where the ticket model came from.
  - **`mode: "pool"`.** Each member carries what `wf.pools()` reports, plus
    `status`: `ready`, `reserve` when it is held at its reserve, `limited`,
    `disabled` when it cannot run, or `unknown` when T3 did not answer. It
    also carries `picks` in the window, `share` and `weightShare`.
    `shareBasis` says what `share` counts: `picks`, the fraction of every
    pick in the window, former members' picks included, or `weight`, the
    configured weight when the window has no picks. A member's `reasons`
    counts its picks by kind, which is `weighted`, `priority`, `cheapest`,
    `last-resort` or `moved`. Its `skips` counts the window's picks that
    passed it over, by kind: `limited`, `reserve`, `disabled`,
    `constrained`, `missing-model`, `missing-effort` or `other`. At the top
    level, `picks` and `reasons` cover every pick in the window, `series`
    splits the window into 24 buckets of picks per member, `recent` lists
    the latest 20 picks with their reasons, and `former` counts picks of
    instances no longer in the pool.
  - **`mode: "single"`.** `members` holds the one instance, model and effort
    that launches use, with `share` 1 and no pick history.
  - **Tokens.** `tokens` totals the input and output tokens of the runs that
    ended in the window, as `{input, output, runs}`. It also holds
    `byInstance`, and `byProvider`, which groups the totals by driver. Each
    member carries its own `tokens`, and `tokenSeries` splits the window
    into the same 24 buckets as `series`, each with `input` and `output` per
    instance. These
    fields are `null` while no token source reports counts, and T3 sends
    none over MCP today, so they stay `null` against a real T3. The
    watcher reads each run's counts once, within a day of the run ending,
    counts them on the instance that ran it, and keeps them for 400 days.
    [ADR 0012](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0012-token-counts-per-run.md)
    has the reasoning.

  `running` lists the active threads of every provider instance, counted the
  way `wf.pools()` and the pool picker count them, so queued and starting
  threads count too. It is `null`, with `runningError`, when T3 did not
  answer. The pick history keeps the latest 2000 picks, and a bad window
  throws.
- `wf.status()` returns the service's state as the board header shows it:
  `version`, `t3`, `tokenExpiresAt`, `watcher`, `openLinks`, `update` and
  `skills`, the check of the installed T3 workflow skills and setup steps
  described in [Install the skills with t3cm](getting-started.md#install-the-skills-with-t3cm).
  It needs no caller and reads the skills fresh.
- `wf.slot(name, { repo?, instance?, skills?, files? })` returns one
  [slot](getting-started.md#slots), `review`, `passes`, `delivery`, `models` or
  `credentials`, resolved for the caller's T3 instance and project: its
  `source` layer, each skill with `how` to run it (`Skill tool`, or
  `inline: read <path> and apply it`, or why it is skipped), a one-line `line`,
  and `warnings`. For `delivery`, `files` filters steps with `paths` globs
  against changed repo-relative paths and returns unmatched steps in `skipped`.
  For an Antigravity thread, which has no Skill tool, every
  skill found on disk is `inline`. For `models`, each role carries a
  `modelSelection` for `t3_thread_launch`, or the pool to pass to `wf.pick`. `repo` overrides the
  project's workspace root, for example with a worktree.
- `wf.renewal({ output, repo? })` matches a failed command's output against
  the `credentials` patterns and returns `{ match, skill, how, skipped }`, or
  `null` when none matches.
- `wf.slots({ repo?, instance? })` returns every slot at once, as
  `t3cm slots --json` prints it, with the caller `context` and `warnings`.
- `wf.waiting.add`, `wf.waiting.resolve` and `wf.waiting.list` keep an overseer
  thread's waiting list.
- `wf.flow()` returns the [flow](config.md#flow)'s ready queue, the flow
  tickets running, and the free slots. `wf.flow.next()` runs one pick by hand.
  See [wf.flow](#wfflow).
- `wf.pr({event, repo, pr, head, publishing?, checks?, body?})` is how a
  worker reports its pull request: `opened`, `updated` or `ready`. Under a
  board ticket the service checks the description, records the head, or
  opens the merge card itself, with no ticket turn. See [wf.pr](#wfpr).
- `wf.merge({repo, pr, head, method?})` merges a GitHub pull request at the
  approved head commit. `repo` is `owner/name`, `head` the full 40-character
  SHA the user approved, and `method` one of `merge`, `squash` or `rebase`.
  See [wf.merge](#wfmerge).

```js
async () => wf.send({ kind: "note", body: "tests green at d4db97c" })
```

The service stores a note without waking anyone, and the note reaches the
parent as a digest line on its next wake. Each message the service relayed
past the parent, and each answer it routed back, reaches it the same way, as a
"handled for you" line. The kinds in the config's `wake` list
wake the receiver. `wake: true` on a send forces a wake for any kind, notes
included, and `wake: false` suppresses one, except that `handoff` and `closed`
always wake a thread parent. A send to the board ignores `wake`, since the board
turns messages into cards and feed entries. A wake is one compact
`t3_thread_send` (mode `queue`) of the form
`[t3cm <kind> from=<label> msg=<handle>]`, with the body capped at `bodyCap`.
`wf.show(handle)` returns the rest.

Wakes to a busy thread wait in an outbox and go out as one combined message
when it is idle, held at most `maxHoldMinutes` (30). A watcher task runs every
`watcherIntervalSecs` (60 s): it nudges idle children, reports stalls and
pending requests once, escalates after repeated misses, closes links whose
thread was deleted or archived, and marks [pool](config.md#limits) members
limited when one of their threads stops on a usage limit.

```js
async () => {
  const p = await wf.pick({ pool: "opus" });
  const r = await t3.thread_launch({ projectId, title, message, modelSelection: p.modelSelection });
  return wf.picked({ pick: p.pick, thread: r.threadId });
}
```

### wf.pr

A worker calls `wf.pr` for each step of its pull request. `repo` is
`owner/name`, `pr` the number, and `head` the head commit SHA, 7 to 40
characters. When the worker's ticket is on the board and the link's
[`relay`](config.md#workflow-layer) is on, the service handles the event
itself. Each step lands in the ticket's digest as a "handled for you" line.

| `event` | What the service does | `status` |
|---|---|---|
| `opened`, with `publishing` | Checks that the Publishing line names the format and each pass, and that the description on GitHub has every heading, element and `Refs #<n>` line the format sets | `open`, or `rewrite` with the rewrite request in `message` and the gaps in `missing` |
| `updated` | Records the new head and drops any merge card for an older head | `recorded`, with the `dropped` card ids |
| `ready` | Reads the PR on GitHub, resolves the [delivery slot](config.md#workflow-layer) for its changed files, and opens the ticket's merge card with the answers the slot calls for | `gated` with the `card` and its `options` |
| `ready`, any check still running, required or not, such as CodeRabbit's | Marks the PR; GitHub reconcile opens the card once every check finishes | `waiting` |
| `ready`, a check failed | Nothing on the board; a failure reconcile finds later comes back as a reply | `failing` with the `checks` |
| `ready`, draft, moved head or merge conflict | Nothing | `refused` with a `reason` |

The format comes from the ticket's `wf.openChild({..., pr})`. `format: "pr"`
requires the `pr` skill's Summary, Evidence and Merge Danger sections with
their elements, and `sections` maps more headings to the text each must
hold. A PR whose merge state is `BLOCKED` or `BEHIND` with every check passed
still gets its card, and the card names the block. For each reviewer whose
latest review requests changes, it names the reviewer and the commit that
review is on, as the current head or an earlier head.

Where the service cannot decide, it sends the ticket the message a worker
sent before `wf.pr`, with a line saying why, and returns `status: "ticket"`
and `why`. That covers a ticket under an overseer thread, `relay` off, a
worker's second PR, a `mergeOnly` brief, `checks: "none"`, no stored format, a
delivery slot without a builtin `merge` stage, and a PR it cannot read or
verify. [ADR 0015](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0015-structured-pr-events-from-the-worker.md)
lists every case.

```js
async () => wf.pr({ event: "ready", repo: "acme/widgets", pr: 7, head: "d4db97c" })
```

### wf.merge

`wf.merge` reads the pull request in one GraphQL query, then does one of these.
GitHub computes the merge state lazily, so while an open pull request reports
`UNKNOWN`, the query runs again, at most three times, one second apart.

| The pull request | What `wf.merge` does | Returns |
|---|---|---|
| Is at a head other than `head` | Nothing | `refused` |
| Is a draft or closed | Nothing | `refused` |
| Is already merged | Nothing | `merged` with its merge commit |
| Is already in the merge queue | Nothing | `queued` with its position and `approval` id |
| Has a merge queue and is mergeable | `enqueuePullRequest` with `expectedHeadOid = head` | `queued` with its position and `approval` id |
| Has a merge queue and is not mergeable yet | Records it as waiting to be queued | `waiting` with its `mergeState` and `approval` id |
| Is `CLEAN` or `HAS_HOOKS` with a check on its head still running | Records it as waiting | `waiting` with its `mergeState` and `approval` id |
| Has no merge queue and is mergeable | `mergePullRequest` with `method`, else the repo's default, and `expectedHeadOid = head` | `merged` with the commit |
| Has no merge queue, and is `BLOCKED`, `UNKNOWN`, or `UNSTABLE` with a check still running | Records it as waiting to be merged | `waiting` with its `mergeState` and `approval` id |
| Has no merge queue and is `BEHIND`, `DIRTY` or otherwise not mergeable | Nothing | `refused` |
| Has a check on its head that failed | Nothing | `refused`, naming the check |

Mergeable means a `mergeStateStatus` of `CLEAN` or `HAS_HOOKS` with no check
on the head running or failed, or `UNSTABLE` when every check on the head that did not pass ended `CANCELLED`, `SKIPPED` or
`NEUTRAL`. GitHub's own merge button accepts any `UNSTABLE` pull request, but
`UNSTABLE` also covers a failed optional check, so `wf.merge` reads the
checks. A check that ended `FAILURE`, `ERROR`, `TIMED_OUT`, `ACTION_REQUIRED`,
`STALE` or any other outcome makes it refuse, and a check still running makes
the pull request not mergeable yet, so `wf.merge` records it as waiting.
[ADR 0016](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0016-merge-on-the-boards-answer.md)
moved that case from `refused` to `waiting` on a repo with no merge queue. [ADR 0003](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0003-unstable-merge-state.md)
has the reasoning. `wf.merge` never turns on
auto-merge: on a merge-queue repo, auto-merge set on an already mergeable pull
request never joins the queue. Every result carries `status`, `repo`, `pr` and
`head`; `refused` adds a `reason`. When `gh` cannot reach GitHub at all, the
call throws.

A waiting record holds the repo, the pull request, the approved head, who
asked (`thread` and `label`), and the merge state it saw. A `queued` result
keeps the record too, marked as sent, so the board follows the pull request
until it merges or leaves the queue. The board's
[GitHub reconcile](board.md#github-reconcile) queues a waiting pull request
once GitHub shows it mergeable at the approved head, or merges it on a repo
with no merge queue, and opens a new merge gate if the head moves first. It
never merges over a review that requests changes: GitHub reports such a pull
request `BLOCKED`, and the record waits until the block clears or an admin
merges the pull request on GitHub. A merge, here or on GitHub, drops the record. A
repeat call replaces the record, and the board acts only on the latest one.

The service runs `gh api graphql` with the `gh` login of the user it runs as,
so `gh` must be on the service's `PATH` and signed in. `execute` code never
reaches `gh` or the network itself; see [Security](security.md#github).

```js
async () => wf.merge({ repo: "acme/widgets", pr: 81, head: "d4db97c0e1f2a3b4c5d6e7f8091a2b3c4d5e6f70" })
```

### The board's merge answer

When the user answers a merge card that [`wf.pr`](#wfpr) `ready` opened
with one of its buttons, such as `merge` or `merge and deploy`, the service
runs the merge itself, with no ticket turn:

1. It calls `wf.merge` for the ticket at the card's head, with the `method`
   from the worker's `wf.openChild` brief. A `queued` or `waiting` result
   holds the approval in the merge wait, and GitHub reconcile follows it.
2. Once GitHub shows the pull request merged, it deletes the head branch
   through the API, unless GitHub already did, the branch is on a fork, or
   the branch's tip is no longer the merged head.
3. It resolves the ticket's merge cards for the pull request.
4. When no `pr`-scoped delivery stage remains after `merge`, it sends the
   worker `Merged #<n> at <merge commit>. Hand off.`
5. It wakes the ticket only when stages remain to run: a `pr`-scoped stage,
   or on the ticket's last PR a `ticket`-scoped one. The wake names the
   remaining stages and the ones the answer approved. Otherwise the ticket
   reads the merge in its digest.

A pull request merged on GitHub while its merge card is still open counts as
`merge` alone. On a card GitHub reconcile opened after the approval, `requeue`
and `merge` run `wf.merge` again at the card's head, and `ok` accepts the
queued head and keeps waiting. Any other answer goes to the ticket, and
drops the service's approval and its merge wait.

The ticket gets the answer as before when the service cannot decide: the
brief gave no `method`, the reconcile is off, no open worker link holds the
pull request, or the card predates this behavior. A `refused` result, or a
`wf.merge` call that throws, also goes to the ticket, with the outcome.
[ADR 0016](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0016-merge-on-the-boards-answer.md)
records the rules.

### wf.flow

`wf.flow()` reads each covered repo's open issues through `gh api graphql`,
then applies the [ready rules](config.md#flow) when `flow.maxTickets` is
configured. Covered repos come from `flow.repos`, or from active T3 projects'
GitHub remotes when `flow.repos` is omitted, with `flow.includeRepos` and
`flow.excludeRepos` applied. It needs no caller and changes nothing. It returns:

| Field | Holds |
|---|---|
| `enabled` | `false` when `flow.maxTickets` is unset, whether the config has no `flow` block or sets repo overrides only. Without `{ graph: true }`, every list is empty and `wf.flow()` reads nothing from GitHub. |
| `launch`, `maxTickets` | The config's values, or `null` and `0` when the launch queue is off. |
| `free` | `maxTickets` minus the flow tickets running, never below zero. |
| `running` | The flow tickets: `label`, `repo`, `number`, `url`, and the ticket's `thread` and `link`. `thread` is `null` for a launch that hasn't joined yet. A ticket whose issue has closed isn't listed. |
| `queue` | The ready issues in launch order. |
| `blocked` | The matching issues held back by an open native or body-parsed blocker, an open `prStack` dependency, a `fileConflict` with a running ticket when `flow.serializeFileConflicts` is enabled, an unresolved blocker reference, a plain `blocked` label, or `blocked: external`, in the same order. |
| `skipped` | Ready issues kept out of the picks, in the same order: skipped on a launch card, or still open after their own ticket finished. Each comes back once its labels or blockers change. |

Each issue in `queue`, `blocked` and `skipped` has `repo`, `number`, `label`, `url`,
`title`, `priority`, `milestone`, `createdAt`, `parent` and `unblockedBy`.
`priority` is `high`, `medium`, `low` or `null`, `milestone` has `title` and
`dueOn` or is `null`, and `unblockedBy` lists the closed issues that used to
block it. A `blocked` issue adds `blockers`, the open issues that block it,
and `external`, which is `true` when it carries `blocked: external`. It also
adds `blockedLabel: true` when held by a plain `blocked` label, `unresolved`
for unresolved blocker references from its body, `prStack` for stacked PR or
sequential `part: n/N` holds (each with `repo`, `number`, `label`, `url`,
and optional `branch`, `baseBranch`, `pr`, `basePr`, `part`), and
`fileConflicts` when `flow.serializeFileConflicts` holds it behind a running
ticket touching overlapping files (each with `repo`, `number`, `label`, `url`,
`files`). An issue with more blockers than GitHub returned in one read
stays in `blocked` with `moreBlockers: true`. When `gh` can't read an
explicitly configured repo, or cannot read any discovered repo, the call throws
and names the repo.

`wf.flow({ graph: true })` also returns `graph`, the dependency graph the
board reads from `GET /api/flow`, even when `config.json` has no `flow` block.
It reads every open issue, its body and open pull requests across covered
repos, inspects local worktree branch ancestry and modified files when a local
checkout is attached, keeps transitive blocker chains intact through unlabelled
issues, and uses the batched tickets query for any active ticket or
body-referenced issue not already in that read. Each edge carries `kind` as
`blockedBy`, `parent`, `prStack` or `fileConflict` and `source` as `github`,
`body`, `git` or `code`. `prStack` edges include optional `branch`,
`baseBranch`, `pr`, `basePr` and `part` fields; `fileConflict` edges carry
`files` listing the overlapping repo-relative paths. `graph.warnings` and each
affected node's `warnings` report cycles, unresolved references, and plain
`blocked` labels with no linked open blocker. `wf.flow()` leaves `graph` out by
default because it can be large.
[The dependency graph](board.md#the-dependency-graph) describes its nodes,
edges, diagnostics and smart lane rules.

```js
async () => {
  const f = await wf.flow();
  return { free: f.free, next: f.queue.slice(0, 3).map(i => `${i.label} ${i.priority ?? "-"}`) };
}
```

`wf.flow.next()` runs one pick by hand, under the rules the board applies when
a slot opens ([The flow](board.md#the-flow)). It takes the first ready issue
that has no launch card yet. With `launch: "ask"` it raises that issue's
launch card, with `"auto"` it launches the ticket, and with `"off"` it reads
nothing and launches nothing. It returns:

| Field | Holds |
|---|---|
| `launch`, `free` | The config's mode and the free slots the pick saw. |
| `picked` | The issue it took, if any: `label`, `url`, `title`, `priority` and `why` (such as `priority: high, unblocked by #9`), with `card` for the card it raised, `launched` for the `wf.launchTicket` result, or `error` when the launch threw. |
| `offered` | The launch cards still open, each `{ label, card }`. Each holds a slot. |
| `retired` | The launch cards this pick resolved, because their issue is no longer ready or no slot is free. |
| `reason` | Why `picked` is empty: `no free ticket slot`, `no ready issue to launch`, or the flow being off. |

```js
async () => (await wf.flow.next()).picked
```

### wf.fileIssues

`wf.fileIssues` creates a multi-issue dependency plan on GitHub in topological
order. Each entry in `issues` requires `title` and accepts optional fields:
`id` for a symbolic local id such as `"E"`, `"P0"`, or `"P1"`, `repo`, `body`,
`blockedBy`, `parent`, `track`, `priority` as `"high"`, `"medium"`, or `"low"`,
`touches` for repo-relative paths, `labels`, `queue`, and `note`. Top-level
`repo`, `parent`, `track`, `priority`, `labels`, and `queue` act as defaults
for items that omit them, except that a top-level `parent` matching an item's
own local `id` does not attach that item to itself. When `queue` is `true` and
`flow.optIn.label` is configured, the service adds that opt-in label unless
`sync` is `false`. Entries
in `blockedBy` and `parent` can name another item's local `id` or an external
GitHub issue as `#N`, `owner/repo#N`, or a URL.

Before writing anything to GitHub, `wf.fileIssues` validates every item and
reference, checks that every target repository is reachable, and topologically
sorts the batch across both `parent` and `blockedBy` edges. Any cycle,
including a self-reference or a mixed `parent` and `blockedBy` cycle, or any
unknown local id throws with zero partial writes.

For each item in topological order, `wf.fileIssues`:

1. Substitutes real `#N` references within the same repository, or
   `owner/repo#N` across repositories, for every resolved `parent` and
   `blockedBy` target, and writes the canonical `## Flow` block with `Parent:`,
   `Blocked by:`, `Track:`, `Priority:`, and `Touches:` into the issue body. If
   the body already has a `## Flow` section outside fenced code, the service
   replaces that section; otherwise it appends the block.
2. Creates the issue on GitHub through `createIssue`.
3. Unless `sync` is `false`, wires native GitHub `addSubIssue` and
   `addBlockedBy` relationships, creates any missing labels, and applies
   `track: <track>`, `priority: <priority>`, `flow.optIn.label` when `queue` is
   `true`, and any custom labels. The service records permission and wiring
   errors in each item's `wiring` object and in top-level `errors`.
4. When called from a thread with `report` omitted or `true`, records each
   created issue on the board's new-issues list.

The result holds `{ ok, order, issues, byId, errors }`. The `issues` array
follows creation order and gives each issue's `id`, `inputIndex`, `repo`,
`number`, `label`, `url`, `title`, `body`, resolved `parent` and `blockedBy`,
applied `labels`, `reported`, and `wiring`. The `byId` map indexes each
symbolic local `id` to `{ repo, number, label, url }`.

```js
async () => wf.fileIssues({
  repo: "acme/widgets",
  track: "auth",
  priority: "high",
  issues: [
    { id: "E", title: "Epic: auth refresh", queue: false },
    { id: "P0", title: "Phase 0: token store", parent: "E", touches: ["src/tokens.rs"], queue: true },
    { id: "P1", title: "Phase 1: refresh flow", parent: "E", blockedBy: ["P0"], touches: ["src/auth.rs"], queue: true },
  ],
})
```

## console

```js
async () => {
  console.log("starting");
  console.warn("this goes to logs too");
  return "done";
}
```

returns `{"result": "done", "logs": [...]}` with both lines.
