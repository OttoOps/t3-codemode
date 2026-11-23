# The board

The service is the root of the link tree, in place of an LLM overseer thread. A
fixed virtual node, `board`, always exists, and ticket threads join it with
`wf.join({ parent: "board", label, role: "ticket" })`, or with
`wf.init({ role: "ticket" })` when a thread starts a ticket on its own. No
overseer thread runs, so no ticket's messages land in any model's context until
someone asks.

The page is `http://127.0.0.1:47731/` on the default port; use the port you
gave `--port` or `T3CM_PORT` if you changed it. It is one embedded HTML file with no
external requests, in light and dark, and it raises a browser notification for
each new card while the tab is hidden.

## What the page shows

The page has three panels under a header that counts the cards waiting on you
and the threads running, and shows where the next ticket launch goes, as
[Launch routing](#launch-routing) describes. Only the lists scroll.

Between the header and the filter bar, the pulse strip counts the open tickets
in five slots, as [The pulse strip](#the-pulse-strip) describes. Click a slot
to filter the board to its tickets, and click it again to clear that filter.

- **Needs you.** Gates, blockers, reports and the watcher's escalations each
  become a card, blockers first. A card opens with the ask itself ("Merge
  #64"), then the ticket, its issue title and how long the card has waited.
  Its answers are the filled buttons, as [Reply buttons](#reply-buttons)
  describes, followed by a link to the PR or issue the message names and the
  quieter Reply, Dismiss and full-message controls. The ticket label jumps to
  the ticket's row.
  Answering a card sends the ticket one reply and resolves the card. A ticket
  that clears its gate, or sends a newer `gate` message, resolves its older
  gate cards, and its handoff resolves all of its open cards.
  A pool thread that stops on a usage limit gets a `move`/`wait` card, unless
  its pool's [`moveOnLimit`](config.md#moving-a-limited-thread) is `auto`,
  which moves the thread with no card, or `never`, which leaves it. A failed
  automatic move falls back to the card.
- **New issues,** at the top of the Tickets column: the issues threads have
  filed and reported with `wf.issue` that have no ticket yet, newest first.
  Each row shows the issue's `repo#n` linked to GitHub, its title, how long ago
  it was filed and a link to the thread that filed it, with Launch and Dismiss.
  A count beside the heading says how many wait, and the section is gone
  while none do. Show dismissed, beside the heading when any are dismissed,
  swaps the list for the dismissed issues, each with Restore. [New issues](#new-issues) covers what each button
  does.
- **Up next,** under New issues when the config has a [`flow` block](config.md#flow):
  the flow's ready queue, in the order the board launches it.
  [The up-next panel](#the-up-next-panel) covers what each row shows.
- **Tickets,** in Open, Done and Settled tabs, grouped by state, and a
  [Graph](#the-graph-tab) tab that draws the open tickets and the flow's issues
  with the links between them. A ticket
  takes the most urgent state of its own thread and its workers, so a ticket
  whose worker runs reads running:

  | Mark | State |
  |---|---|
  | filled diamond | needs you: it has an open card |
  | cross | failed, or blocked |
  | half-filled ring | running |
  | dashed ring | waiting |
  | hollow ring | idle |
  | two bars, violet | paused ([pausing](#pausing)) |
  | check | done |

  Amber marks only the tickets with an open card. A row shows the ticket label,
  its issue title (or its gate in words when there is no title), and, for a
  ticket that needs you, a `Reply "merge"` link to its card. Other rows show
  the last message, and a gate already answered reads "Answered: ...", or "Dismissed: ..." when you dismissed its card. The
  workers sit at the end of the row, with finished ones folded into a count
  that expands. A worker whose link carries an issue identity
  ([`wf`](globals.md#wf)) shows it as `#115 · PR #136 · of #105`: its
  sub-issue, its pull request and the parent issue, each a chip that opens on
  GitHub. The ticket's row shows the same chips after its title for its
  active worker: of the workers with an identity, the one on the ticket's own
  pull request, else the last open one, else the last finished one. A ticket
  with no such worker shows its own issue and parent, and a row with no
  identity shows no chips.
  A pull request shows as an outlined pill with its number, and
  on the ticket's own row its state: open, queued, merged or closed. Once a
  [GitHub reconcile](#github-reconcile) has read an open pull request's
  readiness, the pill says where it stands instead:

  | Pill | Mark | Means |
  |---|---|---|
  | `ready`, green | ringed check | not a draft, checks passing or none, no review held, and GitHub can merge it; a queued pull request is green too |
  | `review`, `CI`, `CI failing`, `draft`, `changes`, `conflict` or `behind`, blue | ringed clock | still in review or CI: the word names what holds it |
  | `failing`, red | triangle | approved, but a check at the head failed |

  A worker's pill shows only the number, with the same mark and color, and
  its tooltip says the state.
- **Activity.** Notes and events from tickets, the latest `board.feed` (50)
  of them, under the ticket and worker that sent them. Runs from the same
  sender fold into one row that expands. The board drops kind prefixes and
  `Publishing:` tails, and shortens hashes to 7 characters and GitHub links to
  `#65`. When a launch picks from a pool in `models.ticket` or `board.pool`,
  each launch adds an entry that names the instance it went to and why.
- **Launch and Ask,** pinned above Activity. Launch takes GitHub issue URLs and
  makes each one a ticket thread. Before you launch, the board shows which model
  and source (`.t3-workflow.json`, `AGENTS.md`, `user config`, `board setting`,
  or `default`) the launch will use; when an issue's repository overrides
  `models.ticket` in its `.t3-workflow.json` or `AGENTS.md`, the board says so.
  The model pickers in [Settings](#settings), which the **Change** links
  beside Launch and Ask open, offer two modes for both ticket launches and the
  ask box: a **specific model** (provider
  instance, model and effort from T3's `orchestrator_capabilities`) or a **pool**
  (only pools configured in t3cm). Saving the ticket model writes the user-config
  layer of `models.ticket` (`~/.config/t3-workflow/config.json`); saving the ask
  model writes `board.askInstanceId`, `board.askModel`, `board.askEffort` or
  `board.askPool` in t3cm's `config.json`. Both persist and take effect on the
  next launch or question with no restart. **Ask your agent** takes a question
  about the board or a request for work, and starts a short-lived scratch
  thread on the ask model. The thread gets the list of T3 projects (id, title
  and `owner/repo`). A question, such as "which tickets wait on me?", gets a
  read-only answer and launches nothing. A request, such as "cut a new release
  for t3-codemode", is not done in the scratch thread: it picks the one
  project the request is about and calls [`wf.dispatch`](#from-a-thread),
  which launches a thread in that project. The launched thread joins the board
  and shows under Tickets, and the answer names it and its project. When no
  project fits, or more than one could, the answer asks which one and nothing
  is launched. The scratch thread writes its answer under Ask, and then gets
  archived.
- **Launch routing,** under Launch and Ask, collapsed until you open it: where
  ticket launches go, over a time range you pick.
  [The routing panel](#the-routing-panel) covers what it shows.

Times show as ages (`36m`, `1h 32m`); hover one for the local time.

The page follows your system's light or dark setting. The theme button in the
header, a half-filled circle, switches it to Light, then Dark, then back to
System, and this browser remembers the choice. A browser that blocks storage
still switches, but goes back to System on reload.

Single-key shortcuts work while no field has focus: `/` focuses search, `a`
focuses Ask, `l` focuses Launch, `j` and `k` move between cards and rows, and
`?` lists the rest. The list has a switch that turns them off. Enter on a
focused card or row opens it in [the inspector](#the-inspector).

The gear button in the header opens [Settings](#settings). What it shows
depends on who opened the board.

A `handoff` or `closed` message marks a ticket done.

## Settings

On a board you open at a loopback address, the gear button in the header opens
Settings, which edits all of t3cm's `config.json`. A remote board gets the
full dialog only when its own `GET /api/settings` answers 200, which needs all
of these:

- `board.remote.mode` is `"tailscale"`.
- The Tailscale login is in `board.remote.settingsUsers` or, when that list is
  empty, is the host's own Tailscale login.
- The request arrives over HTTPS with `X-Forwarded-Proto: https`, as
  Tailscale Serve sends it.

Any other remote board, including a pairing-mode session and an allowed
Tailscale user who is not a settings user, gets 403 from the
[settings API](#settings-api). Its gear opens Settings with only the ticket
and ask model pickers, which save as usual. See
[Board settings](security.md#board-settings) for why, and for what a remote
read returns.

| Section | Holds |
|---|---|
| Launches | The ticket and ask model pickers, and `board.runtimeMode` |
| Pools | Each [pool](config.md#pools) and its members, with each member's live state |
| Board | The other `board` keys, such as `feed`, the GitHub check intervals, `openLinks` and `remote` |
| Workflow links | `link`, `wake`, the body and reply caps, `digestLines`, and a JSON editor for `roles` |
| Runtime | Timeouts, run limits, the watcher interval, the hold limits and `updateCheck` |
| Upstreams and deny list | `deny`, one tool per line, and a JSON editor for `upstreams` |
| All keys (JSON) | An editor for the whole file, and a read-only view of every key with its default |

Edit a key that has no control of its own, such as the older
`board.ticketModel`, under All keys (JSON). Outside Pools, a field shows
**Default** when the file does not set its key, and **Reset** takes the key out
of the file so the default applies again. Settings leaves the keys you don't
edit as the file has them, and it writes a list whole. In Pools, choosing a
member can fill in the pool's driver, and turning **Mixed** on or off moves the
model and effort between the pool and its members. Each field shows the value
that Save will write.

The two model pickers save on their own. The ticket picker writes
`models.ticket` in the workflow config, which is a different file. The ask
picker writes `config.json`, so its Save is off while you have other unsaved
changes. The other fields wait while it saves.

**Save settings** writes every section at once and pauses editing until it
finishes. It then [reloads](config.md#reload) the service, which also applies
any hand edit that was waiting in the file. The dialog lists those waiting keys
before you save.
A save can end in these ways:

- An edit that fails t3cm's checks writes nothing. The error shows next to its
  field and at the top of the dialog, where a click takes you to the field.
  Validation reports the first error it finds, so the next save can show
  another. When T3 does not answer, the save goes on with pool members that
  t3cm could not check, and the dialog says so. That holds for a retry too. If
  T3 rejects a member and then stops answering, saving again without a change
  writes that member with the warning.
- Settings refuses to save when the file changed on disk since it read it. It
  finds the change when you save or when it reads the file again, lists the
  keys that changed and turns Save off until you discard your edits and load
  the file. You can copy your edits from All keys (JSON) first. An editor that
  saves the file while t3cm writes it can still lose its change, as
  [How t3cm writes the file](config.md#how-t3cm-writes-the-file) explains.
- A change to `maxConcurrentRuns`, `upstreams` or `board.remote` waits in the
  saved file for a [restart](config.md#restart-only-keys). The dialog marks
  those fields **Needs restart** and lists every key that still waits, earlier
  changes included.

In Pools, add a pool by name and then choose its driver, model and members.
The lists hold what T3 offers now. **Type another value** takes one that is
missing, such as when T3 is down or leaves a model off its short list. The
states beside the members describe the pool the service runs, so they change
after you save. They refresh every 20 seconds while Pools is open. Member order
matters to the `priority` strategy, so each member has Up and Down buttons.

Esc closes the dialog, and it asks before it drops unsaved changes to the
file. A model picker's unsaved choice doesn't count: opening Settings again
discards it. The arrow keys move between sections, and a
narrow window shows the sections as a menu.

The dialog holds an edit it can't save as shown:

- Save writes nothing while a field fails the dialog's own checks, and takes
  you to it. The service checks the whole file again on each save.
- While All keys (JSON) holds text that isn't valid JSON, the other sections
  take no edits.
- While a field elsewhere fails the dialog's own checks, All keys (JSON) is
  read-only. The dialog names the field to fix first.
- A number field refuses text it would change, such as `12abc`, or `1,5` in a
  browser that reads only `.` as a decimal point. Clear the field and type the
  number again.
- Save waits while the pool name box holds a name you haven't added.

The dialog cannot repair a file that is broken JSON. It shows the error, and
`t3cm config edit` repairs the file. The dialog also refuses an integer beyond
9007199254740991 in either direction, because a browser would round it. Edit a
file that holds one with `t3cm config edit`. A whole number written with a
decimal, such as `25.0`, becomes `25` when Settings saves the file, even when
you did not edit it, and a number with more digits than a browser keeps is
rounded, so `700000.00000000001` saves as `700000`. When All keys (JSON) repeats
a key, the last one wins and Settings drops the others. When `link`, `board` or
`board.remote` holds a list with entries, the dialog edits nothing until
`t3cm config edit` makes it an object. The JSON editors
show any literal credential in the file, so use an `${env:NAME}` or
`${file:path}` reference in `upstreams`.

## Updates

When a newer t3cm release is out, the header shows **Update to vX.Y.Z**. It
opens a panel with the `t3cm update` command, a button that copies it, and a
link to the release notes. **Hide until the next release** hides the button
until a later release comes out; the board remembers that per browser. After
the service restarts on a new version, the page reconnects by itself. Once the
service runs a newer version than the loaded page, the button reads **Reload
for vX.Y.Z**, and a click loads the new board.

When the T3 workflow skills are behind this release, edited, installed by
`npx skills`, or a setup step is missing, the header shows a warning button
such as **Skills out of date** or **Setup step missing**. Its panel says what
drifted and lists each command that fixes it, such as `t3cm skills update` or
`run setup-t3-workflow: new slot models`, each with a copy button. The button
goes away on the next check after the fix. A machine with no skills installed
shows nothing.

## Launch routing

To the right of the connection status, the header shows where board ticket
launches go. It follows the ticket model the board resolves: `models.ticket`
in the user config, else legacy `board.pool` or the board ticket keys, else
the default.

- **With a pool**, it lists each member with its share of the last 24 hours'
  picks, largest first, such as `→ claude-b 75% · claude-a 25%`. When the
  pool made no picks in that time, the share is the member's configured
  weight. A limited member also shows its reset time, as in
  `claude-a 25% until 21:00`.
- **Without a pool**, it names the one instance, model and effort launches
  run on, such as `→ claudeAgent · claude-opus-5-5 · high (no pool)`.

A dot before each member shows its state: filled green for ready, amber for
held at its reserve, a red square for limited, grey for disabled or unable to
run its model, and a grey ring when T3 did not answer. Hovering over or
focusing the indicator opens a tooltip with the strategy, what the shares
count, and each member's state, reason, running threads and limit reset time.
Escape or a click elsewhere closes it, and clicking or tapping the indicator
opens the [routing panel](#the-routing-panel). On a narrow screen the
indicator shrinks to the arrow and the top member, or the
instance alone without a pool. When the members do not fit in the header,
the line is cut off at the right, and the tooltip still lists every member.

The watcher refreshes the indicator on each pass and pushes it to the page
over `/api/events` when a share, state or running count changes, so it
updates without a reload. [`wf.usageStats`](globals.md#wf) and
`GET /api/usage-stats` return the same view for any window.

### The routing panel

The Launch routing panel shows the same routing over a time range you pick.
It starts collapsed, and clicking the header indicator opens it. On a wide
screen it sits under Launch and Ask and moves, collapses and resizes like the
other panels. On a narrower screen it sits under Up next in the Tickets column
and has its own Show and Hide button. This browser remembers whether the
panel is open and which range it shows.

- **Time range.** 1h, 24h, 7d or All, with 24h as the default. All covers
  every pick the store keeps.
- **Totals.** The picks in the range, any picks by former members, and the
  threads running now. With no picks in the range, the totals say the share is
  the configured weight.
- **Members.** One row per member, largest share first. Each row has the state
  dot, the share and its bar, the state, picks, running threads, and usage
  left once a usage source reports it. A limited member shows when its limit
  resets and why. Below that, the row counts the reasons picks chose the
  member, which are weight, priority, cheapest, last resort and moved, and the
  reasons picks passed it over, such as limited, at reserve and disabled.
- **Picks over time.** 24 bars across the range, each stacked by member, with
  a key.
- **Tokens over time.** The input and output tokens of the runs that ended in
  the range and have recorded counts, with the run count, then the same
  totals per provider. Below them, 24 bars cover the range, each stacked by
  instance in the picks chart's colors, with a key that gives each instance's
  total. When no token source reports counts, which is always the case against
  T3 today, the section says T3 reports no token counts yet and shows no
  numbers. When counts are reported but none fall in the range, it says so and
  shows no chart.
- **Pick and skip reasons.** The same counts for the whole pool.
- **Running now.** Running threads per instance, from the live board state.
- **Recent picks.** The last five, with the time, instance and reason.

Without a pool, the panel names the one instance with its model, effort and
running threads, and shows no picks chart. Its token section still shows.

The panel reads `GET /api/usage-stats?window=` when it opens, when you pick
another range, and when the routing in the board state changes, so a new
pick, a limit, a change in running threads or a newly counted run shows
without a reload. The running counts always come from the live board state,
even when a read fails. A collapsed panel reads nothing, and the panel stays
hidden until the watcher's first routing pass. A failed read shows under the
panel with a Retry button.

## Reply buttons

A `gate`, `blocker` or `report` sent with `options` gets exactly those
buttons, and the board does not read the body for them:

```js
async () => wf.send({
  kind: "gate", gate: "merge",
  body: '**ACTION NEEDED:** reply "merge" to merge #81, or "hold" to wait, here: https://github.com/acme/widgets/pull/81',
  options: ["merge", "hold"],
})
```

Each option must be a single line of 1 to 60 characters once trimmed, no two
may match ignoring case, and a send carries at most 8. A send that breaks any
of these fails, names each bad entry by its index, and leaves no card. A
`note`, `event`, `handoff` or `closed` send with `options` fails too. A send
with an empty list gets a card with no buttons.

Without `options`, the board reads the buttons from the body.

When a message has an `**ACTION NEEDED:**` line, its buttons come from that
line alone, or from each such line when it has several. A quoted phrase
elsewhere in the message never becomes a button. On an action line, each quoted
answer after the first `reply`, `answer` or `say` becomes a button. The first
line below gives two buttons and the second gives three:

```text
**ACTION NEEDED:** reply "merge and deploy" to merge #81, or "merge" alone, here: https://github.com/acme/widgets/pull/81
**ACTION NEEDED:** reply how to track it: "advisory", "issue", or "nothing", here: https://github.com/acme/widgets/pull/37
```

Double quotes count whether they are straight or curly. Single quotes count
only when they open after a space or punctuation mark and close before one or
at the end of the line. That rule keeps the apostrophe in `user's` from
starting or ending an answer.

A message with no action line gets one button per `reply "X"` phrase anywhere
in it.

The board trims each answer and drops one that runs past 60 characters or onto
a second line. Answers that differ only in case share one button, and a card
shows at most 8.

A send that opens a card returns that card's buttons as `options`, so the
sender sees what the user will see. A gate whose card has no buttons also
returns a `warning`: the user can still type an answer, but the sender should
send it again with `options`, or quote each answer after `reply` on its action
line.

### Delivery gates and chains

The board keeps no delivery wording of its own. `t3-ticket` reads ordered
stages from `wf.slot("delivery", { files })` and passes explicit `options` on
each `wf.send({ kind: "gate", gate, body, options })` call. The builtin merge
stage keeps `gate: "merge"` so GitHub reconcile tracks the PR, holds approval
to `head <sha>`, and clears the merge gate once the PR lands. Every later
gated stage uses `gate: "delivery:<id>"`, such as `gate: "delivery:deploy"` or
`gate: "delivery:publish"`.

When a gated stage is followed by a consecutive run of `gate: "chain"` stages,
`t3-ticket` passes one button per composable prefix, longest first, within the
board's 8-button and 60-character limits. For a pipeline with `merge`
(`always`), `deploy` (`chain`), and `verify` (`chain`), the merge gate sends:

```js
async () => wf.send({
  kind: "gate",
  gate: "merge",
  body: '**ACTION NEEDED:** reply "merge, deploy and verify", "merge and deploy", or "merge" to merge #81 at head d4db97c, here: https://github.com/acme/widgets/pull/81',
  options: ["merge, deploy and verify", "merge and deploy", "merge"],
})
```

Picking a shorter prefix runs that prefix and opens the next stage's own
`gate: "delivery:<id>"` card when reached, such as
`options: ["deploy and verify", "deploy", "skip deploy"]`. Stages with
`gate: "never"` run automatically once their predecessor succeeds. Stages with
`scope: "post-handoff"` run outside the ticket. Under the default
`closeAfter: "ticket"`, the ticket posts a note on the issue and includes the
remaining checklist in a final `report` and its `handoff` body without keeping
the ticket open. Under `closeAfter: "all"`, each remaining `post-handoff`
stage opens a `delivery:<id>` gate before the ticket closes.
[ADR 0007](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0007-composable-delivery.md)
records the composable delivery design.

## Filtering the page

The bar under the header narrows the page to the work you pick. Each filter
applies to the decision cards, the ticket tree, the activity feed, the
Settled section, the [Up next](#the-up-next-panel) rows and the
[Graph](#the-graph-tab) tab at once.

- **Project, Status and Card kind** are multi-select dropdowns. Type in one to
  narrow its list, and Enter toggles the first value left. A project shows as
  the ticket's `owner/repo`, or as its T3 project's title while the repo is
  not known. The filter keeps the T3 project id, so a pick still holds once
  the repo arrives, and
  [ADR 0004](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0004-context-projects-by-id.md)
  has the reasoning. Status is running, ready to launch, needs you, blocked,
  blocked: external, paused, ready to merge, in review or CI, alert or done.
  A ticket can hold several: one whose worker runs while its pull request is
  in CI is both running and in review or CI, and a pick matches a ticket that
  holds any picked status. Ready to launch matches the Up next queue rows.
  Blocked: external matches the tickets and Up next rows that wait on
  something outside the covered repos, as
  [the dependency graph](#the-dependency-graph) defines it.
  Card kind narrows only the cards.
- **Search** narrows the page while you type, with no Enter. Every word must
  appear somewhere in the ticket label, the issue title or number, the repo, or
  the card or feed text. `Esc` clears it, and `/` focuses it when single-key
  shortcuts are on. While [the inspector](#the-inspector) is open, the first
  `Esc` closes it and the next one clears the search.

While a filter is on, each count reads visible/total. A new card the filter
hides shows up as "N hidden", and clicking it clears the filter.

### The pulse strip

Each slot shows a count of open tickets and one line about them. The chevron
at the strip's end folds it, and this browser remembers that. Until you use the
chevron, the strip starts folded on a window under 1000 px tall. Folded, it shows
each slot's mark, a short name and its count, and from 1200 px it moves up
beside the filter bar, so the panels get its row back. Below 1200 px the strip
always shows one line per slot, and below 600 px only each slot's mark and
count. Hover a slot for its full line.

| Slot | Counts tickets that are | Status it filters by |
|---|---|---|
| Active | running: open, with no card, not blocked, and not paused itself. The line says how many threads run and, with a [flow](#the-flow), the open tickets against `maxTickets`, how many are over it, and how many issues are ready to launch | running |
| Ready to merge | holding a pull request that is ready to merge or queued | ready to merge |
| In review or CI | holding a pull request still in review or CI | in review or CI |
| Paused or blocked | paused, or with a paused worker, or failed or blocked, or blocked: external. A paused link counts while its thread runs, waits or fails too. The line also counts the Up next issues that are blocked: external | paused, blocked and blocked: external |
| Alerts | holding an approved pull request with a failing check | alert |

A slot sets its statuses in the Status filter, so it composes with the
project, card kind and search filters and with the context you are in, and
**Save filter** keeps it in a context. A slot reads pressed while all of its
statuses are picked. While a filter is on, each count reads visible/total.

### Contexts

A context is a named filter: a set of projects, statuses and card kinds, plus a
search if you want one. The service stores contexts, so every browser that
opens the board sees the same ones, and a change made in one browser reaches the
others as it happens.

The **Context** menu at the start of the filter bar switches between them.
Picking one applies its filter. **All** clears every filter, and so do **Clear
filters** and the "N hidden" button. The same menu edits contexts:

- **Save filter as a new context** names the filter you have on.
- **Save filter to** a context replaces its filter with the one you have on. A
  context whose filter you changed reads "(edited)" until you save or switch.
- **Rename** and **Delete** act on the context you are in. Delete asks for a
  second click.

Each browser remembers its own context and comes back to it after a reload. If
that context was deleted, the page falls back to All. A context changed in
another browser applies here too. A change that arrives while this browser is
saving a context applies when the save finishes, so the newer of the two edits
wins. A context you just saved stays on until the board sends a state that
includes the save, however long that takes.

While a context is on, a browser notification goes out only for a new card
inside it. A card outside it still counts toward "N hidden".

## The inspector

Click a ticket row, a worker, a pull request pill, a card's ask or ticket line,
a New issues row, a Settled ticket or an issue in the [Graph](#the-graph-tab)
tab, or press Enter on a focused card, row or issue,
and the inspector slides in from the right with that item's details. It covers
the right side of the board under the pulse strip and filter bar. The lists
keep their scroll and keep updating, and you can still answer the cards on the
left. Below 600 px the inspector takes the full width. The row or card it shows
gets the hover shade, so you can find it on the board.

- **Header.** The ticket or issue as `repo#N`, the PR part such as `PR 2/5`
  when a worker's label or `part` names one, the same issue, pull request and
  parent chips as the row (or the worker's own when opened from a worker), the
  title, and a close button.
- **Pills.** The live state, the pull request pill with the same readiness word
  as the [ticket list](#what-the-page-shows), `paused`, and the repository and
  project when `/api/state` knows them.
- **Actions.** Open issue on GitHub, Open PR on GitHub, Open T3 thread, and
  whichever of Pause or Resume, Settle and Restore apply to the ticket. A new
  issue offers Launch and Dismiss, or Restore once dismissed. A plain click on
  a row's pull request pill opens the inspector, and a Ctrl-click or
  middle-click goes straight to GitHub.
- **Needs you.** The ticket's open cards, each with Go to card, which focuses
  the card on the board.
- **Latest message.** The ticket's last gate, blocker, handoff or note. When
  the board holds only the start of it, the inspector loads the whole message
  from `GET /api/msg/{handle}`.
- **Workers.** Each worker's state, a link to its thread, its pull request and
  Pause or Resume. Opened from a worker, the inspector shades that worker.
- **Dependencies.** With a `flow` block, a ticket or issue in
  [the dependency graph](#the-dependency-graph) shows its lane and its
  Blocked by, Unlocks, Parent and Sub-issues as chips, each with its state.
  A chip moves the inspector to that issue, or to its ticket when it has one,
  and leaves the board where it was. `Esc` still returns focus to the item
  that first opened the inspector. The pills add the issue's priority and
  milestone. An issue with no ticket shows its flow standing, such as "In the
  ready queue", in place of the ticket sections.

The inspector follows its item while the board updates. If the item leaves the
board, the inspector says so, keeps the last details it had and disables its
buttons. Opening another item replaces what it shows. Enter on a row that has
left the board but still holds focus opens nothing and announces that the item
is gone. `Esc` or the close button closes the inspector and puts focus
back on the item that opened it. The next `Esc` clears an active search. While
the inspector covers Launch and Ask, `l` and `a` close it first, and `g d`,
`g t` and `g a` close it when it covers the item they focus.

## From a thread

| Call | Does |
|---|---|
| `wf.launchTicket({ issue, parentIssue?, part?, project?, title?, model?, effort? })` | Launch a ticket thread for a GitHub issue. Its link stores `issue`, `parentIssue` and `part` ([issue identity](globals.md#wf)) |
| `wf.dispatch({ project, request, title? })` | Launch a ticket thread in a T3 project to carry out a request |
| `wf.cards({ all?, context?, projects? })` | The open cards, oldest first, each with its `repo` and `project`; `all` includes resolved ones |
| `wf.answer(id, text)` | Answer a card for the user, the same as a button on the page |
| `wf.feed({ limit?, context?, projects? })` | The latest notes and events, each with its `ticket`, `ticketLabel`, `ticketTitle`, `ticketStatus`, `repo` and `project` |
| `wf.board({ parent: "board", all: true })` | Every ticket on the board, with live status |
| `wf.issue({ url, title?, note? })` | Put an issue this thread just filed on the [new issues](#new-issues) list |

```js
async () => (await wf.cards()).map(c => `${c.id} ${c.label}: ${c.options.join(" | ")}`)
```

`context` and `projects` narrow `wf.cards` and `wf.feed` to the same projects
the page would show. `context` takes a saved [context](#contexts)'s id or its
name, and uses only its projects. Its statuses, card kinds and search stay on
the page. `projects` takes a list of T3 project ids. A repo or title value
from an older context works there too. Both follow the page's rule from
[ADR 0004](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0004-context-projects-by-id.md),
so a card or row matches by its project id, by the repo or title the board
knows for that id, or, with no project id, by a repo the board ties to the
picked id. A card or row with no project and no repo matches nothing. An
empty list, or a context with no projects, keeps everything. Giving both, or
a context that does not exist, throws. With a filter, `wf.feed` scans the
latest 200 rows and returns up to `limit` of the ones that match.

```js
async () => (await wf.cards({ context: "Widget work" })).map(c => `${c.id} ${c.repo}: ${c.text}`)
```

`wf.launchTicket` finds the T3 project whose workspace `origin` remote is the
issue's GitHub repo, unless `project` names one, resolves `models.ticket` for
that project through the service's slot resolver, and launches a thread that
works the issue and reports to the board. The ticket is labeled `repo#n`, or
`owner/repo#n` when a ticket for another owner's issue of the same name and
number already holds `repo#n`. Launching an issue that already has a ticket,
or a launch that has not joined yet, fails. Precedence for the launch model is:

1. `model` or `effort` passed to `wf.launchTicket`, reported as `modelSource: "argument"`.
2. The repository's `.t3-workflow.json` or `AGENTS.md`, reported as `".t3-workflow.json"` or `"AGENTS.md"`.
3. The user config's `models.ticket`, reported as `"user"`.
4. Legacy `board.instanceId`, `ticketModel`, `ticketEffort` and `pool` in
   t3cm's config, reported as `"board"`.
5. The built-in default, reported as `"default"`.

The result always carries `instanceId`, `model`, `effort`, which is `null` when
none applies, and `modelSource`. When the resolved choice is a
[pool](config.md#pools) in `models.ticket` or legacy `board.pool`, the launch
picks a provider instance through `wf.pick`, adds `pool` and `reason` to the
result, and logs a feed entry with `instanceId` and `reason`. Every launch
checks its choice against T3's `orchestrator_capabilities`. An unmapped repo, a
missing or disabled instance, an unlisted model, an unsupported effort, an
unknown or exhausted pool, or a failed launch throws and leaves an error card,
never falling back to the built-in default. The launched thread is told to use
the `t3-ticket` skill, which lives in this repo's `skills/` folder and is
installed as an agent skill, apart from the service.
[The workflow skills](getting-started.md#the-workflow-skills) covers installing
them. Without the skills installed, launched threads do not know the workflow.

`wf.dispatch` launches a thread for work that has no GitHub issue. `project`
names a T3 project by its id or title, ignoring case. The launch resolves
`models.ticket` for that project in the order above, pools included, and
applies `board.runtimeMode` the way `wf.launchTicket` does. The thread's brief
is `request` word for word, followed by a board link, so the thread joins the
board as a ticket. Its label and T3 title are the project title and `title`,
as in `t3-codemode: release`. Without a `title`, the label uses the first
words of the request. When a ticket on the board, open or done, or a launch
still waiting to join already has that label, the label gets ` (2)`, ` (3)`
and so on. When the scratch thread of a question from Ask calls
`wf.dispatch`, the question records `dispatched: { thread, link, projectId,
project, label }`, where `project` is the project title. The result has
the same fields as `wf.launchTicket`'s. An unknown project or an empty request
throws and launches nothing. A model that does not resolve or a failed launch
throws and leaves an error card.

## New issues

A thread that files a GitHub issue reports it with `wf.issue`, and the board
keeps it on a list of new issues until someone acts on it. Threads know to
report because `setup-t3-workflow` installs a short rule that Claude Code and
Codex load in every session, so it reaches threads that run none of the
[workflow skills](getting-started.md#the-workflow-skills). Each entry records
the issue's URL, `repo` and `number`, and the `title` and `note` the thread
passed. It also records the filing `thread`, with its T3 `threadLink` and
`threadTitle`, and `at`, the time the thread reported it. An entry's id looks
like `n3`, and the list survives restarts. If T3 can't give the thread's link
and title when the issue is reported, the entry keeps them empty. The next
report of that issue fills them in.

An issue goes on the list once. Reporting it again, from any thread, changes
nothing. Neither does reporting an issue that already has a board ticket, open,
done or still waiting to join. The board matches tickets by their `repo#n`
label, ignoring case. The label leaves out the owner, so an issue in another
owner's repo of the same name counts as having that ticket.

- **Launch** runs `wf.launchTicket` for the entry's issue, with its title as
  the ticket's title. When the launch succeeds, the entry leaves the list and
  the ticket shows under Tickets. A launch that fails leaves the entry in
  place and an error card, as any failed launch does. Launching the same issue
  from the Launch tickets box also takes its entry off the list.
- **Dismiss** takes an entry off the list without touching GitHub. Dismissed
  entries stay in the store, and **Restore** puts one back.

The page gets a reported issue within seconds, on the same event stream as
everything else. While the pointer or focus is in the Tickets column, a new
entry waits behind an "N new" button in the section's heading, and an entry
that leaves stays as a placeholder ("widgets#43 launched") until you move
away, so nothing under your pointer shifts.

While no entry waits, nothing shows above Tickets, not even the heading. When
dismissed entries exist then, the Settled tab carries a Dismissed issues
button that opens the section on the dismissed list, and Show new closes it
again.

A new-issue row or ticket row that arrives on the event stream gets a green
tint that fades on its own over about four seconds. An entry held behind "N
new" gets it when you reveal it. Rows already there when the page loads, and
rows a filter change brings back, don't flash. With reduced motion set, the
tint holds still for a moment and then goes, with no fade.

## The flow

With a [`flow` block](config.md#flow) in the config, the board starts the next
ready issue when a ticket slot opens. A slot opens when a flow ticket sends its
handoff or `closed`, when its link closes because its thread is gone or
archived, and when its issue closes. The board also picks again on each
[GitHub reconcile](#github-reconcile) pass, so an issue whose last blocker
closed outside the board, or whose ticket's issue closed, is picked up within
one reconcile interval. With `board.reconcileIdleMinutes` at `0`, only a
ticket's handoff or close and `wf.flow.next()` pick.

Each pick reads the queue from GitHub, the same queue
[`wf.flow()`](globals.md#wfflow) returns, and fills the free slots in queue
order. What it does with each issue depends on `flow.launch`:

- **`ask`**, the default, raises a launch card such as "Launch widgets#60
  next? (priority: medium, unblocked by #115)", with **launch** and **skip**
  buttons and a link to the issue. **launch** starts the ticket through
  `wf.launchTicket`, with the same model choice as any board launch. The board
  reads the queue again first: when the flow is off, the issue is no longer
  ready, or no slot is free, the card closes and nothing launches. **skip**
  takes the issue out of the picks and offers the next ready issue in its
  place.
- **`auto`** launches the issue through `wf.launchTicket` and posts "Flow
  launched widgets#60 (priority: medium, unblocked by #115)" to the activity
  feed. `auto` treats a `priority: high` issue like any other. A launch that
  fails leaves its error card, posts a feed event, and skips the issue, so the
  board doesn't retry it on every pass.
- **`off`** launches nothing and reads nothing from GitHub.

An open launch card holds its slot, so a pick never raises more cards than
there are free slots. A pick resolves a launch card whose issue is no longer
ready, for example because it closed, gained an open blocker, or got a ticket
some other way. It also resolves the newest cards when tickets launched
elsewhere took their slots. Switching `flow.launch` to `off` or `auto` resolves every
open launch card on the next pick, and `auto` then launches into the freed
slots. Dismissing a launch card leaves the issue in
the queue, so the next pick offers it again.

A skipped issue stays out of the picks until its labels or blockers change:
adding or removing a label, adding or removing a blocked-by link, or a blocker
reopening puts it back in the queue once a pick sees the change. An issue that is still open after its own board ticket has
finished is skipped the same way, so a ticket that stopped short isn't
relaunched on its own. Skipped issues show in `wf.flow()`'s `skipped` list, and
the Launch tickets box still launches one by hand.

A ticket takes one slot, however many sub-issues or workers it runs. Picks,
launch-card answers and every other ticket launch run one at a time, and a pick
holds the board lock while it counts the open tickets and decides, so two
launches can't both fill the same slot. A launch from the Launch tickets box
waits for a pick in progress to finish.

### The up-next panel

With a `flow` block in the config, the Up next panel sits under New issues and
shows what the flow will launch next. It reads `GET /api/flow`, which returns
what [`wf.flow()`](globals.md#wfflow) returns. Reading the panel never records
a skip, raises a card or launches anything.

- The heading counts the ready issues and shows the free slots and the launch
  mode, such as "1 of 3 slots free · launch: ask".
- Each ready issue is a numbered row in queue order: its `repo#n` linked to
  GitHub, its title, its priority (`none` without a priority label), the
  blockers that closed (unblocked by #115), and its milestone and due date.
  A blocker in another repository shows as `repo#n`.
- **Blocked** folds the opted-in issues that still wait on something. Each row
  names its open blockers, adds "and more" when GitHub lists more than 100,
  or reads "blocked: external" for an issue with that label. Click Blocked to
  open or close the list.
- **Skipped** folds the issues the flow skipped ([skip memory](#the-flow)),
  each with the same details as a ready row.

A ready row carries a **ready** pill, and a blocked row that waits on
something outside the covered repos carries an **external** pill. The filter
bar narrows these rows like the rest of the page, and while it is on, each
heading count reads visible/total.

The panel reads the queue when the page loads, when a ticket opens or
finishes, when a launch card opens or closes, and once a minute while the tab
is open. Refresh reads it again now. A GitHub error shows under the panel. The
panel is gone while the config has no `flow` block, and the page then reads
nothing for it.

### The dependency graph

`GET /api/flow` also returns `graph`, the issues the flow and the board know
about and the links between them. The graph covers each open opt-in issue and
each active board ticket, including a ticket launched by hand outside the
flow. Each of these is a node with `tracked: true`. An issue they link to,
such as a closed blocker or a parent epic, is also a node, with
`tracked: false` and only what the link carries: `repo`, `number`, `label`,
`url`, `title` and `state`.

The graph makes no GitHub call per issue. The flow's read of each covered repo
asks for each issue's sub-issues next to its blockers and parent, and the
batched query that checks whether the open tickets' issues are closed reads
their blockers, parent and sub-issues too. That query also reads a ticket in
a repository the flow does not cover, once the board knows the ticket's
repository. Without a `flow` block, the graph holds only the active tickets
the board can place in a repository, with no GitHub data and no edges, and
the board reads nothing from GitHub.

`edges` lists each link once, from the issue that has it:

| `kind` | From, to | Read the other way |
|---|---|---|
| `blockedBy` | the blocked issue, its blocker | the blocker unlocks the issue |
| `parent` | the sub-issue, its parent | the parent's sub-issue |

Each node's `id` is `owner/name#N`, and its lists use those ids:

| Field | Holds |
|---|---|
| `blockedBy`, `unlocks` | Its blockers, open and closed, and the issues it blocks. `moreBlockers` is `true` when GitHub lists more than 100 blockers. |
| `parent`, `subIssues` | Its parent's id or `null`, and its sub-issues. `moreSubIssues` is `true` past 100. |
| `state` | `open` or `closed`, or `null` when nothing read it. |
| `labels`, `priority`, `milestone` | As GitHub gives them, with `priority` read from a `priority:` label. |
| `track`, `trackBy` | The lane it belongs to, by the rules below. |
| `flowList` | `queue`, `blocked` or `skipped` when it is in that list, else `null`. |
| `ready` | `true` while it is in the ready queue: opted in, no open blocker, no ticket. |
| `external` | `true` when it is labelled `blocked: external`, or when it has open blockers, GitHub listed all of them, and none is in a covered repo. |
| `ticket` | Its board ticket's `thread`, `link` and `status`, or `null`. |

`external` here is wider than a `blocked` issue's `external`, which reports
the label alone.

A node's track follows the first rule that applies:

1. Its `track:` label. A `track: board` label puts it in track `board`. When
   it has several, the first in alphabetical order counts. `trackBy` is
   `label`.
2. Its milestone's title. `trackBy` is `milestone`.
3. The track of its connected component, the nodes that any chain of edges
   joins it to. The component takes the track of its member with the lowest
   repo and number that has a label or milestone track, or else that member's
   label, such as `widgets#3`. `trackBy` is `component`.

Ticket rows on the board carry an **external** pill when their node is
external, and the inspector shows the same pill.

### The Graph tab

The Graph tab in the Tickets pane draws the dependency graph as an SVG
swimlane view, with no library and no request beyond `GET /api/flow`. This
browser remembers whether you left the Tickets pane on Graph, and opens it
there after a reload; `#tickets-graph` in the address opens it too.

- **Lanes.** Each lane is a track, by the rules in
  [the dependency graph](#the-dependency-graph): a `track:` label, else a
  milestone, else the issues linked to it, so an issue with no link, label or
  milestone has a lane of its own. Its heading names the track, what set it
  and how many issues it holds. Lanes holding a ticket come first, then label,
  milestone and linked tracks, each in name order.
- **Layout.** A blocker sits to the left of what it blocks, and a parent to
  the left of its sub-issues, so work flows left to right. The same graph
  always lays out the same way. A wide graph scrolls inside the tab, never
  the page. Each issue shows its `repo#N`, title and state: a ticket's live
  state, or for an issue with no ticket ready, blocked, external, skipped,
  closed or open. Amber still marks only a ticket with an open card.
- **Links.** A solid arrow runs from a blocker to the issue it blocks, and a
  dashed line from a parent to its sub-issue.
- **Tracing.** Hover or focus an issue, and the chain of issues that block it,
  however far back, turns red, and the chain it unlocks turns blue. Its
  parent and sub-issues keep their look, and everything else turns muted
  until the pointer leaves and focus moves out of the graph. The latest hover
  or focus picks the issue, and with the pointer off the graph, the focused
  issue keeps its trace.
- **Filters.** The [filter bar](#filtering-the-page), a saved
  [context](#contexts), search and the [pulse strip](#the-pulse-strip) apply
  to the graph too. An issue they leave out stays in place with a dashed
  outline and muted text, so the layout never jumps, and the tab's count
  reads visible/total. A ticket matches as its row would; an issue with no
  ticket matches by repo, its ready, blocked, external or closed standing,
  and its label, title, track and milestone.
- **Inspector.** Click an issue, or press Enter or Space on it, to open it in
  [the inspector](#the-inspector).
- **Keyboard.** The graph is one tab stop. The arrow keys move between
  issues lane by lane and left to right, as do `j` and `k` while single-key
  shortcuts are on, and Home and End jump to the first and last.
- **Live updates.** The graph follows each board update. While the pointer or
  focus is in the graph, a change that would move issues waits until it
  leaves, so nothing shifts under the pointer.

Without a `flow` block, or while `/api/flow` is loading or failing, the tab
draws the open tickets with no links, one lane per repository and a
**Tickets** lane for those the board has not placed, and says so above the
graph. When a refresh fails after a good read, the tab keeps the last graph
it read and says so.

## GitHub reconcile

The board checks what it believes against GitHub on a timer, so a pull request
merged, closed or dropped from a merge queue outside T3 still reaches the board
and its ticket. It tracks:

- each pull request of an open worker link,
- each pull request an open card names, read once when the card opens: from
  its action line, or else from its whole message, and only when it names
  exactly one,
- each pull request `wf.merge` saved as waiting to be queued, or sent to
  GitHub,
- each pull request it last saw in a merge queue, until GitHub records why it
  left or until the first check 30 minutes after it left, so that a removal
  GitHub records late, or never records, still gets a report.

It reads them with `gh` as the user the service runs as, in one GraphQL query
for every 100 pull requests it tracks, and makes no GitHub call when it tracks
nothing. A check runs every `board.reconcileActiveMinutes`, 2 by default, while
a tracked pull request is queued or waiting, and every
`board.reconcileIdleMinutes`, 10 by default, otherwise. Loading the page runs
one too, at most once every 30 seconds.
`docs/adr/0002-github-reconcile.md` in the repository has the reasoning.

The same query reads each pull request's readiness: whether it is a draft, its
merge state, its review decision and its checks at the head. The board keeps
the last read and shows it in `/api/state` as the pull request's `readiness`,
with no further GitHub call.

It also reads each pull request's closing issue references, the issues its
`Closes #N` lines name. An open worker link with no `issue` takes the first
closing reference of the pull request its row shows, so a worker launched
without an `issue` still shows the issue it closes. A link that already has an
`issue` keeps it.

Each check compares what GitHub shows now with the board's state now, and
makes them agree:

| GitHub shows | The board |
|---|---|
| Merged | Resolves the pull request's cards opened before the merge, with the note "merged on GitHub at `<sha>`, `<time>`". In the same transaction, clears the ticket's `merge` gate when no other gate card stays open for it (later delivery stages use `gate: "delivery:<id>"`, so reconcile leaves those untouched). Sends each ticket tracking the pull request one `event` to run its post-merge steps |
| Closed without merging | Resolves its cards opened before the close and sends each ticket one `event`. Reopening and closing it again sends another |
| Dropped from the merge queue for `merge_conflict`, `failed_checks` or `manual` removal | Opens a `report` card per ticket that gives the reason, with `requeue` and `hold` buttons, or one card with no buttons and the steps to take by hand when no ticket tracks it. An approval older than the removal is dropped, so the board never queues the pull request again on its own |
| Out of the merge queue for 30 minutes with no removal recorded, no saved approval and no earlier report of that exit | Opens a `report` card per ticket that says no reason was recorded, with `requeue` and `hold` buttons, or one card with no buttons and the steps to take by hand when no ticket tracks it. If GitHub records the removal while that card is open, a card with the reason replaces it. If a check sees the pull request back in the queue first, it resolves the card |
| Waiting to be queued, now mergeable at the approved head | Sends `enqueuePullRequest` with the approved head as `expectedHeadOid`. If the repository has turned its merge queue off since, sends `mergePullRequest` the same way, with the approval's method or else the repository's default |
| Waiting to be queued, with a check at the approved head that failed | Holds the approval and opens one `report` card per approval that names the failed checks, with a `fix` button. Queues the pull request and resolves the card once the check is re-run and passes. Resolves the card too when a check sees a new head, whatever that head's checks show |
| Approved at one head, now at another | Opens one new card per ticket: a `merge` gate for the new head, or a `report` when GitHub already has the new head in its queue. Resolves the old gate card, in one transaction with clearing the gate when the new card is a `report`, and drops the old approval |

A gate names its head with the word `head` and the SHA, such as
`head d4db97c`. The board holds a gate that names none to the head the first
check sees, so a push before that check goes unnoticed.

The board records each wake and each removal report per ticket, under the
merge commit, the close time or the removal time, so a later check never
repeats it. It saves a wake and its delivery in one transaction, so a T3 outage
delays a wake but never loses it. An event from before the board started tracking the pull
request, or from before this version of the service first started, stays
quiet. So does a removal dated before the last check that saw the pull request
in the queue, because it belongs to an earlier stay there.

When GitHub refuses the enqueue or the merge, or cannot find the pull request,
the approval stays for the next check. After 3 failures in a row the board
opens a `report` card with GitHub's last message and drops the approval. A pull
request sent to the queue that leaves it with no removal GitHub recorded gets
the same 3 checks. After that report the board keeps reading the pull request
for the rest of the 30 minutes, so that a reason GitHub records later still
gets a report. If no reason shows by then, it does not report that exit to the
same ticket again. The board never sends an approval whose number GitHub now
resolves to another pull request, such as after a repository rename. It drops
that approval and opens a `report` card. Every card the reconcile raises for an
approval carries that `approval` id, so `wf.cards()` callers can match it to
the `wf.merge` call that recorded it. Every fix goes on the activity feed
from `github`.

The tree shows a pull request in the merge queue as `queued`, next to `open`,
`merged` and `closed`. When `gh` cannot reach GitHub, the check logs a warning,
changes nothing, and runs again at the next interval. When one pull request
cannot be fixed, for example because the waiting list is full, the check still
fixes the others, and that pull request keeps its approval for the next check.
A read error for one pull request, other than GitHub's `NOT_FOUND`, changes
nothing for it either.

## Settling

The board settles finished tickets and answered asks and never deletes them. A ticket that
has been done, or an ask that has been closed, for `board.settleAfterHours`
leaves the main lists on the next watcher pass. The board also moves a settled
ticket off the thread list in T3 with `t3_thread_organize` and `settle`. That
step is best effort, so a T3 error goes to the log and the board state still
changes.

It works the other way too. A done ticket whose thread you settle or archive in
T3, or whose thread is gone, settles on the board at the next watcher pass. A
ticket you restored on the board ignores a T3 settle older than the restore.

The **Settled** tab keeps them, with a filter and a
**Restore** button on each one. Restoring a ticket sends `unsettle` and
restarts its settle clock. **Settle** on a row and **Settle all done** do it by
hand. An open ticket that is idle, paused or failed also shows **Close**, which
closes its link and moves it to **Done** so you can settle it without visiting
the thread.

## Pausing

**Pause** on a ticket row or a worker marks its link as parked on purpose, and
**Resume** undoes it. A thread can do the same with `wf.pause()`, and a parent
with `wf.pause({ child })`. While a link is paused, the watcher sends it no
nudges, does not escalate it, and reports neither a long silence nor an
unanswered question. It still closes the link when the thread is archived or
gone. The row shows a `paused` pill and sorts under **Paused**, and `/api/state` gives the ticket or
worker `paused: true` and `pausedAt`. Its `status` reads `paused` where it
would read `idle`; a paused thread that runs, waits or fails shows that
status instead. Resuming starts the nudge count again from zero, so a link
that has sat idle past its `down` minutes gets its first nudge on the next
watcher pass. A link stored before pausing existed loads unpaused.

## Remote access

The board is loopback-only by default. To open it from another device over
HTTPS, expose it with Tailscale Serve or put your own HTTPS reverse proxy in
front of `127.0.0.1:47731`, configure [`board.remote`](config.md#remote-board)
in `config.json`, and restart `t3cm`. `/mcp`, `/healthz` and `/reload` stay
loopback-only.

The `setup-t3-workflow` skill offers remote access as its last slot. It runs
the commands below, restarts the service and checks the result.

Five commands manage remote access from the host:

| Command | Does |
|---|---|
| `t3cm board expose --tailscale` | Serves the board over Tailscale Serve and writes `board.remote` |
| `t3cm board unexpose` | Removes the board's Tailscale Serve routes and sets `board.remote` to `{"mode": "off"}` |
| `t3cm board pair` | Mints a single-use pairing link and QR code for one browser |
| `t3cm board sessions` | Lists the paired browsers' active sessions |
| `t3cm board revoke <ID>` or `--all` | Ends one session, or every session |

`board.remote` is a [restart-only key](config.md#restart-only-keys), so the
service keeps its old remote settings until it restarts. `expose` and
`unexpose` print the restart command for your platform, such as
`systemctl --user restart t3-codemode` on Linux. `pair`, `sessions` and
`revoke` work on `store.db` in the state directory that `--state-dir` or
`T3CM_STATE_DIR` sets, and each takes `--json`.

Pick the front end by who opens the board:

- **Tailscale Serve,** when every device that opens it is on your tailnet.
  Tailscale issues the certificate and tells `t3cm` who each caller is, so no
  browser needs pairing.
- **Your own reverse proxy,** such as Caddy, nginx or Cloudflare Tunnel, when
  a device is not on your tailnet. You write `board.remote` yourself, with mode
  `"pairing"`, and pair each browser.

`t3cm board expose` drives Tailscale only. Without `--tailscale` it stops and
points you at `config.json`.

### Tailscale Serve

Tailscale Serve terminates HTTPS with an automatic certificate on your
tailnet's MagicDNS hostname (`https://box.acme.ts.net:47732/`) and injects the
caller's `Tailscale-User-Login` header. Serve keeps the board on your tailnet;
`t3cm` never turns on Funnel. `t3cm` defaults to HTTPS port `47732` so it does
not collide with T3 Code's own `:443` binding.

1. Make sure `tailscale status` shows Tailscale up on the host, and that
   MagicDNS and HTTPS Certificates are enabled in your Tailscale admin
   console. `t3cm board expose` checks all three and stops with the fix when
   one is missing.

   `t3cm` runs `tailscale serve` as your own user. Tailscale on Linux, and
   Tailscale installed inside a WSL distro, takes Serve changes only from root
   or its operator, so make your user the operator once:

   ```sh
   sudo tailscale set --operator=$USER
   ```

   Without it, `expose` fails with Tailscale's access-denied error and prints
   that command as a hint. `t3cm` finds the `tailscale` CLI on `PATH`, or at
   `T3CM_TAILSCALE_BIN`. Under WSL with no Linux `tailscale`, it uses Windows'
   `tailscale.exe` through interop, and the operator step does not apply.

   While any Serve handler on the node serves a local path or a Unix socket,
   `tailscaled` refuses every Serve change from anyone but root, operator
   included. `expose` then names that handler and prints the
   `sudo tailscale serve ...` command that adds the board route as root. Run
   it and rerun `expose`, which reuses the board route it finds and writes
   `board.remote` without another Serve change.

2. Expose the board and write `board.remote` in `config.json`:

   ```sh
   t3cm board expose --tailscale
   ```

   This runs `tailscale serve --bg --https=47732 http://127.0.0.1:47731` and
   writes `board.remote` with `mode: "tailscale"` and
   `origin: "https://box.acme.ts.net:47732"`. Restart the `t3cm` service so the
   new config takes effect.

   Before it writes `board.remote`, `expose` checks the Serve port on this
   machine's Tailscale IPv4 address. The listener there must belong to
   `tailscaled`, and a TLS handshake for the board's origin must succeed. When
   another process holds that address and port, `tailscaled` may serve the
   port on IPv6 only, and HTTPS over IPv4 fails. `expose` then names the
   process when it can, removes the board route it added, and stops. Stop that
   process and rerun `expose` with the same flags. The rerun applies the Serve
   route again, so `tailscaled` binds the IPv4 listener. When the Tailscale
   address is not on this machine, as with Windows' `tailscale.exe` under WSL
   or `tailscaled` in userspace networking, there is nothing to check locally
   and `expose` skips the check.

   Optional flags:

   - `--https-port <PORT>`: serve on a port other than `47732` (refuses to
     overwrite a port already serving a different target).
   - `--port <PORT>`: target a non-default local `t3cm` port (defaults to
     `T3CM_PORT` or `47731`). Pass the same `--port` to `t3cm status` and
     `t3cm board unexpose`.
   - `--allow-user <LOGIN>`: repeatable explicit Tailscale login allowlist,
     written to `allowUsers`. Without it, only the host's own login, as
     `tailscale status --json` reports it, gets in. A list replaces that default, so
     include your own login to keep your access.
   - `--require-pairing`: also require a per-browser pairing cookie on top of
     `Tailscale-User-Login`. With this enabled, run `t3cm board pair` for each
     remote browser.
   - `--json`: machine-readable output.

   This mode needs no pairing. Tailscale Serve authenticates each
   request and adds its `Tailscale-User-Login` header, and `t3cm` admits the
   request when that login is allowed. Until you set `requirePairing`,
   `t3cm board pair` refuses and says so.

3. Restart the service, then check the result:

   ```sh
   t3cm status
   ```

   Its `remote` object, which loopback `GET /healthz` also returns, shows what
   the running service enforces. `enabled` is `true`, and `origin` is the URL to
   open on the other device:

   ```json
   "remote": {
     "mode": "tailscale",
     "enabled": true,
     "origin": "https://box.acme.ts.net:47732",
     "requirePairing": false,
     "allowUsers": ["alice@acme.example"],
     "allowUsersSource": "tailscale",
     "hostLogin": "alice@acme.example",
     "tailscaleError": null,
     "activeSessions": 0
   }
   ```

   `enabled: false` after an `expose` means the service has not restarted
   onto the new config. `allowUsersSource` is `"config"` when `allowUsers`
   comes from `config.json`, as `--allow-user` writes it, and `"tailscale"`
   when it is the host's own login. It is `"unavailable"` when the service
   could not read the host's login, and then `tailscaleError` says why and the
   service refuses every Tailscale caller. In `pairing` mode the object
   carries only `mode`, `enabled`, `origin`, `requirePairing` and
   `activeSessions`.

   - `t3cm doctor` verifies that `config.json`, the running service, Tailscale
     status, MagicDNS, HTTPS certificates (`CertDomains`), and
     `tailscale serve status` agree, and flags any drift or accidental Funnel /
     raw-TCP exposure. It also runs the IPv4 listener and TLS check that
     `expose` runs.
   - `t3cm serve` never mutates Tailscale state on start or restart; only
     `t3cm board expose` and `t3cm board unexpose` change `tailscale serve`.

4. To turn remote access back off:

   ```sh
   t3cm board unexpose
   ```

   It removes the Serve routes that proxy to the board's local port on the
   port in `board.remote.origin`, or on `--https-port <PORT>`, and leaves
   other Serve handlers alone. When you exposed a non-default local port, pass
   the same `--port <PORT>`. Without it, `unexpose` does not recognize the
   board's Serve route, so it can turn `board.remote` off and leave the route
   in place, or refuse. It refuses, naming the command to run, when that port
   forwards the board over plain HTTP or raw TCP. Then it sets `board.remote`
   to `{"mode": "off"}` and prints the restart command. Restart the service to
   apply it.

### Pairing a browser

Use pairing mode behind Caddy, Cloudflare Tunnel, or nginx, or layer it on top
of Tailscale Serve with `requirePairing: true`:

1. Enable pairing mode with your public HTTPS origin in `config.json` (or run
   `t3cm board expose --tailscale --require-pairing`) and restart the service:

   ```json
   {
     "board": {
       "remote": {
         "mode": "pairing",
         "origin": "https://board.example.com"
       }
     }
   }
   ```

   After the restart, `t3cm status` shows `remote.enabled: true`, and
   `remote.origin` is the HTTPS origin you configured, such as
   `"https://board.example.com"` behind a proxy.

2. Mint a single-use 5-minute pairing URL and QR code on the host:

   ```sh
   t3cm board pair --label "Phone"
   ```

   Pass `--no-qr` to print only the URL, or `--json` for machine-readable output.
   `t3cm board pair` refuses while `board.remote` is off, and in `tailscale`
   mode without `requirePairing`.

3. Scan the QR code or open the `https://board.example.com/pair#<token>` link
   in the remote browser. The `/pair` page reads the token from the URL
   fragment, clears the fragment from browser history, and posts it to
   `POST /api/pair` so the raw token never appears in proxy logs or `Referer`
   headers. On success the service sets a 30-day
   `HttpOnly; Secure; SameSite=Strict; Path=/` session cookie and redirects to
   `/`.

4. List or revoke sessions from the host CLI:

   ```sh
   t3cm board sessions
   t3cm board revoke s1
   t3cm board revoke --all
   ```

   Revoking a session cuts off its API calls immediately with 401 and closes any
   open `/api/events` stream. Both pairing tokens and session tokens are stored
   only as SHA-256 hashes in `store.db`.

### Using the board from a phone or laptop

Once remote access is enabled over HTTPS, the board adapts to remote devices
without changing direct loopback behavior:

- **Thread links open on the device you clicked from.** On direct loopback
  (`127.0.0.1`, `localhost`, or `[::1]`), clicking a T3 thread link still calls
  `POST /api/open` to focus the desktop app on the host (unless
  `board.openLinks` is `"tab"`). On any remote host, the browser opens the
  `https://app.t3.codes/threads/...` URL directly on that device and never calls
  `POST /api/open`; server-side, authenticated remote `POST /api/open` requests
  receive 403 while unauthenticated or disallowed requests receive the usual
  401/403 guard refusal.
- **Settings open for the Tailscale owner only.** In `"tailscale"` mode, the
  host's own login, or each login in `board.remote.settingsUsers`, sees the
  full [Settings](#settings) dialog over HTTPS. Every other remote board sees
  only the model pickers there, and the [settings API](#settings-api) refuses
  its requests with 403, reads included. The board's guard refuses an
  unauthenticated remote request with its usual 401 or 403.
- **Add to Home Screen.** The board serves an embedded `/manifest.webmanifest`
  and icons (`/icon.svg`, `/icon-192.png`, `/icon-512.png`) behind the same
  authentication guards and zero-external-request CSP. On iOS Safari, open the
  HTTPS board URL and tap **Share** -> **Add to Home Screen**. On Android Chrome
  or desktop Chromium, use the browser's **Install app** or **Add to Home
  screen** menu.
- **Phone layout and notifications.** Narrow viewports (under 900 px) switch to
  a single-column view with bottom tabs (**Needs you**, **Tickets**,
  **Activity**), touch-sized controls, and a collapsible Launch/Ask dock.
  Because the remote board runs over HTTPS, browser notifications (**Notify
  me**) work on browsers that support direct page notifications (`new
  Notification()`, such as desktop browsers). Mobile browsers that require a
  service worker for notifications (such as Chrome on Android) mark
  notifications unavailable instead of failing silently.
- **Sign-in and expired sessions.** Opening `/` in a remote browser without an
  active session, or losing access while the tab is open (for example after
  `t3cm board revoke`, 30-day session expiry, or a Tailscale login change that
  causes the `/api/events` live auth recheck to close the stream), stops
  background reconnect retries and displays a sign-in or access-denied screen
  explaining whether to pair again with `t3cm board pair` or check Tailscale
  access.

### Reverse-proxy examples

Any proxy fronting the board must follow the rules in
[Security model](security.md#network): terminate TLS, refuse `/mcp`,
`/healthz` and `/reload`, strip client-supplied forwarding and identity headers, always inject
a trusted forwarding header, and reject or overwrite any `Host` that does not
match `board.remote.origin`. Do not configure the proxy to log `POST /api/pair`
request bodies, which carry the raw pairing token during redemption. Never front
the board with a raw TCP forwarder such as `ssh -L`, `socat`, or
`tailscale serve --tcp`.

#### Caddy

Caddy terminates TLS automatically, rejects requests whose `Host` does not match
the site address, preserves `Host`, and injects `X-Forwarded-For`,
`X-Forwarded-Proto`, and `X-Forwarded-Host` (replacing untrusted client values
by default). Strip `Tailscale-User-Login`, `Forwarded`, and `X-Forwarded-Port`,
block `/mcp`, `/healthz` and `/reload`, and set `flush_interval -1` for SSE:

```caddyfile
board.example.com {
    @blocked path /mcp /mcp/* /healthz /healthz/* /reload /reload/*
    respond @blocked 403

    reverse_proxy 127.0.0.1:47731 {
        flush_interval -1
        header_up -Tailscale-User-Login
        header_up -Forwarded
        header_up -X-Forwarded-Port
    }
}
```

#### Cloudflare Tunnel (`cloudflared`)

Place the `/mcp`, `/healthz` and `/reload` deny rule ahead of the board rule in your
tunnel's `config.yml`:

```yaml
ingress:
  - hostname: board.example.com
    path: ^/(mcp|healthz|reload)(/.*)?$
    service: http_status:403
  - hostname: board.example.com
    service: http://127.0.0.1:47731
  - service: http_status:404
```

Cloudflare terminates TLS, enforces `hostname`, preserves `Host`, and sets
`X-Forwarded-For` and `X-Forwarded-Proto: https`. To satisfy the proxy rules in
[Security model](security.md#network), also add a Cloudflare Request Header
Transform Rule on `board.example.com` that removes client-supplied
`Tailscale-User-Login`, `Forwarded`, `X-Forwarded-Host`, and `X-Forwarded-Port`
headers before requests reach `cloudflared`.

#### nginx

nginx's default `proxy_pass` rewrites `Host` to the upstream and sends no
forwarding header, so configure both a catch-all `default_server` that drops
unknown `Host` values and explicit `proxy_set_header` directives. At the edge,
set `X-Forwarded-For $remote_addr` so client-supplied headers are overwritten
rather than appended, and pass `Host $http_host` (or an explicit `host:port`
such as `board.example.com:47732` when serving on a non-443 port, since `$host`
drops the port):

```nginx
server {
    listen 443 ssl default_server;
    ssl_certificate     /path/to/fullchain.pem;
    ssl_certificate_key /path/to/privkey.pem;
    return 444;
}

server {
    listen 443 ssl;
    server_name board.example.com;

    ssl_certificate     /path/to/fullchain.pem;
    ssl_certificate_key /path/to/privkey.pem;

    location = /mcp { return 403; }
    location ^~ /mcp/ { return 403; }
    location = /healthz { return 403; }
    location ^~ /healthz/ { return 403; }
    location = /reload { return 403; }
    location ^~ /reload/ { return 403; }

    location / {
        proxy_pass http://127.0.0.1:47731;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_set_header Host $http_host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header X-Forwarded-Host "";
        proxy_set_header X-Forwarded-Port "";
        proxy_set_header Forwarded "";
        proxy_set_header Tailscale-User-Login "";
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 1h;
    }
}
```

## Settings API

`/api/settings` reads and saves all of t3cm's `config.json`. Its routes answer
direct loopback requests, and remote requests from the Tailscale owner as
[Settings](#settings) describes. Any other signed-in remote browser gets 403,
reads included. A `PUT` needs the `X-T3CM-CSRF` token either way. The service
logs each remote save with the Tailscale login and the keys it changed.

`GET /api/settings` returns these fields:

| Field | Holds |
|---|---|
| `path` | The config file |
| `revision` | An id for the file's content, which a save sends back |
| `file` | The file as parsed JSON, or `{}` when there is none. The file sets a key when the key is present here, even at its default. `null` when the file is invalid JSON. |
| `values` | The file merged over the defaults, as `t3cm config get` prints it. A `null` in a child role inherits the top-level value. `null` when the file is invalid. |
| `defaults` | The built-in defaults in the shape of `values`. A key returns to its default when a save omits it. |
| `running` | The config the service runs. It differs from `values` after a hand edit, and while a [restart-only key](config.md#restart-only-keys) waits for a restart. |
| `restart` | Every restart-only key whose saved value differs from the running one. `null` when the file is invalid. |
| `errors` | Why the file is invalid, as `{field, message}`, or an empty list |

A client saves by changing a copy of `file` and sending `PUT /api/settings`
with `{"revision": "<revision>", "file": {...}}`. It starts from `file`
because a save of `values` would write every default into the file, and it
omits a key to return that key to its default.

The route requires whole values and takes no edits by path, so a client adds
an item to a list by sending the whole list. When `file` lacks the list, the
client copies it from `values` first, because a list in the file replaces the
default one. A child role with no `wake` of its own copies the top-level
`wake`.

A save runs the checks of config load, keeps the layout of the values it does
not change and [reloads](config.md#reload) the service. It answers with one of
these:

| Status | Body | Means |
|---|---|---|
| 200 | `{revision, restart, warnings}` | The file is saved and `revision` is the one for the next save. `restart` lists every key that waits for a restart, earlier changes included. It is `null`, with a warning, when the save worked and the reload failed. |
| 400 | `{error}` | The body lacks a revision or a `file` object, or has another key. |
| 409 | `{error}` | The file changed after the client read that revision, so t3cm wrote nothing and the client reads it again. |
| 422 | `{error, errors}` | The result is invalid, so t3cm wrote nothing. Each of `errors` is `{field, message}`, and `field` is a dotted path such as `board.feed` or `pools."gpt-6.1".members.0.weight`. |
| 500 | `{error}` | t3cm could not read or write the file. |

Config validation reports the first error it finds, and the member check
reports one error for each bad pool. The member check asks T3 about the pools
that the save adds or changes. It skips a pool the save leaves alone, including
when the save repairs an error elsewhere in the file. When T3 does not answer,
the save goes on and `warnings` says that t3cm did not check the members. The
route refuses to replace broken JSON, which `t3cm config edit` can repair.

`GET /api/settings/pools` returns `{pools}`, which is the list `wf.pools()`
gives for the running config, with each member's live state. It is a separate
route so that a refresh leaves a form's revision alone and the settings still
load while T3 is down.

`GET /api/models` supplies the instance, model and effort choices. A client
changes the ticket launch model with `PUT /api/models/ticket`, which writes
`models.ticket` and leaves the legacy `board` ticket keys alone.

## HTTP routes

| Route | Does |
|---|---|
| `GET /` | The page, with a per-process CSRF token embedded (unauthenticated remote browser navigations with `Accept: text/html` receive a 401/403 sign-in or access-denied page with no CSRF token) |
| `GET /manifest.webmanifest` | Embedded web app manifest for Add to Home Screen (same board auth guards, zero external requests) |
| `GET /icon.svg`, `/icon-192.png`, `/icon-512.png` | Embedded board icons for browser tabs, notifications, and Add to Home Screen (same board auth guards) |
| `GET /pair` | Remote browser pairing page (enabled when `board.remote.mode` is `"pairing"`, or `"tailscale"` with `requirePairing: true`); reads the token from `#<token>`, clears the fragment, and posts to `POST /api/pair` |
| `POST /api/pair` `{token, label?}` | Redeem a single-use 5-minute pairing token (`Content-Type: application/json`) over the remote HTTPS origin and set a 30-day `HttpOnly; Secure; SameSite=Strict; Path=/` session cookie |
| `GET /api/state` | `{rev, cards, tree, feed, asks, settled, contexts, newIssues, dismissedIssues, flow, flowMaxTickets, counts, service, routing}`, with `counts.newIssues`. `flow` is `true` when the config has a `flow` block. `flowMaxTickets` is that block's `maxTickets`, or `null` without one. `routing` is the [launch routing](#launch-routing) view from the watcher's last pass, the `wf.usageStats()` result for the default 24-hour window, or `null` before the first pass. `rev` is the board revision: it goes up with every change, keeps going up across a restart, and a state whose `rev` is at least a write's `rev` already holds that write; tickets, cards and feed items carry `repo` (`owner/repo`) and `project` (`{id, title}`), and tickets their thread `title`, with `last.dismissed` when the last message is a gate whose card you dismissed. A ticket's or worker's open `pr` carries `readiness` once a [GitHub reconcile](#github-reconcile) has read it open: `isDraft`, `mergeStateStatus` and `reviewDecision` as GitHub gives them, and `checks`. `reviewDecision` is `null` when the repository requires no review. `checks` is `failing` when a check at the head failed, `pending` while one runs, `passing` when every check succeeded or was cancelled, skipped or neutral, and `null` when the head has no checks or has more than 100 and none of the first 100 failed. A pull request that no reconcile has read yet has no `readiness`. Tickets and workers carry `issue`, `parentIssue` and `part` (each `null` when unset; see [`wf`](globals.md#wf)), `paused`, `pausedAt` while paused, and `status: "paused"` in place of `idle` ([pausing](#pausing)). Times come as local clock strings with an ISO twin for ages: a ticket's `lastUp` and `upAt`, a settled ticket's `settledAt` and `settledIso`, and `service.watcher.lastPass` and `lastPassAt`. `service.update` is `null` while `updateCheck` is `false` or until a release check has answered, then `{current, latest, available, notes, command}`. `service.skills` is the skills check that `/healthz` also reports ([getting started](getting-started.md#install-the-skills-with-t3cm)) |
| `GET /api/events` | SSE: a `state` event on every change, carrying the same `rev` (closes when a remote pairing session is revoked or expires, or when a Tailscale login is no longer allowed) |
| `GET /api/msg/{handle}` | One stored message in full |
| `POST /api/cards/{id}/answer` `{text}` | Reply to the card's ticket and resolve it; a flow launch card takes `launch` or `skip` instead ([The flow](#the-flow)) |
| `POST /api/cards/{id}/dismiss` | Resolve without messaging |
| `POST /api/launch` `{issues: [...]}` | `wf.launchTicket` for each (at most 20) |
| `POST /api/ask` `{question}` | Ask a scratch thread about the board, or for work it launches with `wf.dispatch`, using `board.askPool` or `board.askInstanceId`/`askModel`/`askEffort`; returns the ask view with `instanceId`, `model`, `effort`, `modelSource` (and `pool`, `reason` for pools) |
| `GET /api/usage-stats?window=` | The `wf.usageStats({ window })` view of [launch routing](#launch-routing), computed fresh, which the [routing panel](#the-routing-panel) reads. `window` is optional: `all`, or a count of `m`, `h` or `d` such as `24h` (the default), and may be percent-encoded. Any other query parameter, a repeated `window`, or a bad duration is a 400 |
| `GET /api/models` | Current ticket and ask model resolution (`ticket`, `ask`), enabled providers/models/efforts from `orchestrator_capabilities` (`providers`), configured pools (`pools`), and per-project repo overrides (`repoOverrides`) |
| `PUT /api/models/ticket` `{mode, ...}` | Persist `models.ticket` to the user workflow config (`mode: "model"` with `provider`, `model`, `effort?`, or `mode: "pool"` with `pool`) after validating against live capabilities or configured pools; returns the updated `/api/models` view |
| `PUT /api/models/ask` `{mode, ...}` | Persist the ask box mode to t3cm's `config.json` (`board.askInstanceId`/`askModel`/`askEffort` or `board.askPool`) and apply it live with no restart; returns the updated `/api/models` view |
| `GET /api/settings` | Loopback or the Tailscale owner: the config file, its revision, the saved, default and running values, the keys that need a restart, and any errors. See [Settings API](#settings-api) |
| `PUT /api/settings` `{revision, file}` | Loopback or the Tailscale owner: check and save the whole config, then reload it; returns `{revision, restart, warnings}` (409 for a stale revision, 422 with per-field `errors` for an invalid result) |
| `GET /api/settings/pools` | Loopback or the Tailscale owner: `{pools}`, each pool's live member state as `wf.pools()` reports it |
| `POST /api/tickets/{thread}/close` `{notify?}` | Close an idle, paused or failed open ticket's link, resolve its open cards, and move it to Done; with `notify: true`, also message the thread (404 if not a board ticket, 400 if running, waiting or already done, 503 if T3 cannot verify the thread) |
| `POST /api/tickets/{thread}/settle`, `/restore` | Settle a done ticket, or bring a settled one back (400 if open, or not settled) |
| `POST /api/links/{thread}/pause`, `/resume` | [Pause](#pausing) an open ticket or worker link, or resume it; returns `{thread, paused}` (404 for a thread that is neither, 400 for a done one) |
| `POST /api/asks/{id}/settle`, `/restore` | The same for a closed ask; `id` is `q<n>` |
| `GET /api/new-issues` | `{issues, dismissed}`, newest first; an issue that has a board ticket is in neither |
| `GET /api/flow` | What `wf.flow({ graph: true })` returns: `{enabled, launch, maxTickets, free, running, queue, blocked, skipped, graph}`, read-only ([The up-next panel](#the-up-next-panel), [the dependency graph](#the-dependency-graph)); 502 with `error` when GitHub fails |
| `POST /api/new-issues/{id}/launch` | Launch a ticket for the entry, as `POST /api/launch` does for one issue; the result adds `id` (404 for no such entry) |
| `POST /api/new-issues/{id}/dismiss`, `/restore` | Dismiss the entry, or bring a dismissed one back (400 if already dismissed, or not dismissed) |
| `POST /api/settle-done` | Settle every done ticket and closed ask; returns `{tickets, asks}` counts |
| `GET /api/contexts` | `{contexts: [...]}`, sorted by name; each is `{id, name, spec, createdAt, updatedAt}`, and `spec` is `{projects, statuses, kinds, q?}`. Each project is a T3 project id, or, for an item with no project id, its lowercased `owner/repo` or title. A context saved before project ids holds an `owner/repo` or a project title instead, and an item matches it ignoring case |
| `PUT /api/contexts/{id}` `{name, spec}` | Create the context (201) or replace its name and filter (200), answering `{context, rev}`. Leave out `spec` to rename an existing context, or `name` to change only its filter; either form needs the context to exist (404 if it does not). `id` is 1 to 64 letters, digits, `-` or `_`. Names are unique ignoring case (409), at most 60 characters, and never "All". Statuses are `running`, `waiting`, `blocked`, `done`, `paused`, `pr-green`, `pr-review`, `alert`, `ready` and `external`, and kinds are the card kinds. The board keeps at most 100 contexts |
| `DELETE /api/contexts/{id}` | Delete the context (404 if there is none), answering `{deleted, rev}` |
| `POST /api/open` `{link}` | Loopback-only: open a T3 thread link in the running Windows desktop app (on Windows, or from WSL); returns `{opened}`, and the page opens a new tab when it is `false` (400 for a link that is not a T3 thread; 403 for authenticated remote requests, or the usual 401/403 guard refusal otherwise) |

Direct loopback requests require `Host` to be `127.0.0.1:<port>`,
`localhost:<port>`, or `[::1]:<port>` with no proxy headers and refuse any other
`Origin`. When `board.remote` is enabled, remote requests must match the
configured `https://` `origin` and pass remote authentication (except `GET /pair`
and `POST /api/pair` in pairing mode, which sit before session auth). Every
mutating board route (except `POST /api/pair`, which requires
`Content-Type: application/json`) also requires the `X-T3CM-CSRF` header, which
the page reads from its embedded token. See [Security model](security.md#network).
