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
and the threads running. Only the lists scroll.

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
- **Tickets,** in Open, Done and Settled tabs, grouped by state. A ticket
  takes the most urgent state of its own thread and its workers, so a ticket
  whose worker runs reads running:

  | Mark | State |
  |---|---|
  | filled diamond | needs you: it has an open card |
  | cross | failed, or blocked |
  | half-filled ring | running |
  | dashed ring | waiting |
  | hollow ring | idle |
  | check | done |

  Amber marks only the tickets with an open card. A row shows the ticket label,
  its issue title (or its gate in words when there is no title), and, for a
  ticket that needs you, a `Reply "merge"` link to its card. Other rows show
  the last message, and a gate already answered reads "Answered: ...", or "Dismissed: ..." when you dismissed its card. The
  workers sit at the end of the row, with finished ones folded into a count
  that expands.
- **Activity.** Notes and events from tickets, the latest `board.feed` (50)
  of them, under the ticket and worker that sent them. Runs from the same
  sender fold into one row that expands. The board drops kind prefixes and
  `Publishing:` tails, and shortens hashes to 7 characters and GitHub links to
  `#65`. With `board.pool` set, each launch adds an entry that names the
  instance it went to and why.
- **Launch and Ask,** pinned above Activity. Launch takes GitHub issue URLs and
  makes each one a ticket thread. A question in Ask starts a short-lived Haiku
  thread that reads the board, writes its answer under Ask, and then gets
  archived. With `board.pool` set, the thread runs on a pool member with the
  pool's model.

Times show as ages (`36m`, `1h 32m`); hover one for the local time.

A `handoff` or `closed` message marks a ticket done.

## Updates

When a newer t3cm release is out, the header shows **Update to vX.Y.Z**. It
opens a panel with the `t3cm update` command, a button that copies it, and a
link to the release notes. **Hide until the next release** hides the button
until a later release comes out; the board remembers that per browser. After
`t3cm update` restarts the service, the page reconnects by itself. Once the
service runs a newer version than the loaded page, the button reads **Reload
for vX.Y.Z**, and a click loads the new board.

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

## Filtering the page

The bar under the header narrows the page to the work you pick. Each filter
applies to the decision cards, the ticket tree, the activity feed and the
Settled section at once.

- **Project, Status and Card kind** are multi-select dropdowns. Type in one to
  narrow its list, and Enter toggles the first value left. A project is the
  ticket's `owner/repo`, or its T3 project's title when the repo is not known.
  Status is running, needs you, blocked or done. Card kind narrows only the
  cards.
- **Search** narrows the page while you type, with no Enter. Every word must
  appear somewhere in the ticket label, the issue title or number, the repo, or
  the card or feed text. `Esc` clears it, and `f` focuses it when single-key
  shortcuts are on.

While a filter is on, each count reads visible/total. A new card the filter
hides shows up as "N hidden", and clicking it clears the filter.

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
another browser applies here too.

While a context is on, a browser notification goes out only for a new card
inside it. A card outside it still counts toward "N hidden".

## From a thread

| Call | Does |
|---|---|
| `wf.launchTicket({ issue, project?, title?, model?, effort? })` | Launch a ticket thread for a GitHub issue |
| `wf.cards({ all? })` | The open cards, oldest first; `all` includes resolved ones |
| `wf.answer(id, text)` | Answer a card for the user, the same as a button on the page |
| `wf.feed({ limit? })` | The latest notes and events, each with its `ticket`, `ticketLabel`, `ticketTitle`, `ticketStatus`, `repo` and `project` |
| `wf.board({ parent: "board", all: true })` | Every ticket on the board, with live status |

```js
async () => (await wf.cards()).map(c => `${c.id} ${c.label}: ${c.options.join(" | ")}`)
```

`wf.launchTicket` finds the T3 project whose workspace `origin` remote is the
issue's GitHub repo, unless `project` names one, and launches a thread that
works the issue and reports to the board. An unmapped repo or a failed launch
throws and leaves an error card. The launched thread is told to use the
`t3-ticket` skill, which lives in this repo's `skills/` folder and is installed
as an agent skill, apart from the service.
[The workflow skills](getting-started.md#the-workflow-skills) covers installing
them. Without the skills installed, launched threads do not know the workflow.

With [`board.pool`](config.md#pools) set, each launch from the board picks its
provider instance from that pool in place of `board.instanceId`. The result
carries `instanceId` and `reason`, and the feed entry for the launch carries
the same two fields. When no member of the pool can run, the launch fails,
leaves an error card that names the pool, and launches nothing.

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
hand.

## HTTP routes

| Route | Does |
|---|---|
| `GET /` | The page, with a per-process CSRF token embedded |
| `GET /api/state` | `{cards, tree, feed, asks, settled, contexts, counts, service}`; tickets, cards and feed items carry `repo` (`owner/repo`) and `project` (`{id, title}`), and tickets their thread `title`, with `last.dismissed` when the last message is a gate whose card you dismissed. Times come as local clock strings with an ISO twin for ages: a ticket's `lastUp` and `upAt`, a settled ticket's `settledAt` and `settledIso`, and `service.watcher.lastPass` and `lastPassAt`. `service.update` is `null` until a release check has answered, then `{current, latest, available, notes, command}` |
| `GET /api/events` | SSE: a `state` event on every change |
| `GET /api/msg/{handle}` | One stored message in full |
| `POST /api/cards/{id}/answer` `{text}` | Reply to the card's ticket and resolve it |
| `POST /api/cards/{id}/dismiss` | Resolve without messaging |
| `POST /api/launch` `{issues: [...]}` | `wf.launchTicket` for each (at most 20) |
| `POST /api/ask` `{question}` | Ask a scratch thread about the board |
| `POST /api/tickets/{thread}/settle`, `/restore` | Settle a done ticket, or bring a settled one back (400 if open, or not settled) |
| `POST /api/asks/{id}/settle`, `/restore` | The same for a closed ask; `id` is `q<n>` |
| `POST /api/settle-done` | Settle every done ticket and closed ask; returns `{tickets, asks}` counts |
| `GET /api/contexts` | `{contexts: [...]}`, sorted by name; each is `{id, name, spec, createdAt, updatedAt}`, and `spec` is `{projects, statuses, kinds, q?}` |
| `PUT /api/contexts/{id}` `{name, spec}` | Create the context (201) or replace its name and filter (200). Leave out `spec` to rename an existing context, or `name` to change only its filter; either form needs the context to exist (404 if it does not). `id` is 1 to 64 letters, digits, `-` or `_`. Names are unique ignoring case (409), at most 60 characters, and never "All". Statuses are `running`, `waiting`, `blocked` and `done`, and kinds are the card kinds. The board keeps at most 100 contexts |
| `DELETE /api/contexts/{id}` | Delete the context (404 if there is none) |
| `POST /api/open` `{link}` | Open a T3 thread link in the running Windows desktop app (on Windows, or from WSL); returns `{opened}`, and the page opens a new tab when it is `false` (400 for a link that is not a T3 thread) |

Every board route requires `Host` to be `127.0.0.1:<port>` or
`localhost:<port>` and refuses any other `Origin`. POST, PUT and DELETE also
need the `X-T3CM-CSRF` header, which the page reads from its embedded token.
