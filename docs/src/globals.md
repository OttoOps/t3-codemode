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

- `wf.init({role, label?, root?})` starts a thread with no parent link. A role
  that `roles.overseer.children` lists, which is only `ticket` by default, joins
  the board the way `wf.join({ parent: "board", label, role: "ticket" })` does,
  under its T3 title or under `label`. The result names `parent: "board"` and
  the label. `root: true` keeps the thread off the board as a standalone root.
  Every other role starts as a root. `wf.init` is for a ticket started on its
  own. A ticket launched from the board runs its brief's `wf.join` call
  instead, which clears the launch from the board's pending list.
- `wf.openChild({role, label})` returns a `brief` block the parent pastes into
  the child's launch brief; the child runs the
  `wf.join({parent, label, role, down, miss, stall})` call it contains.
- `wf.send({kind, body, gate?, wake?})`, kinds `note | event | gate | blocker |
  report | handoff | closed`, to the caller's parent. Returns
  `{handle, delivery}`: `noted`, `sent`, `queued` (the parent is mid-turn, or
  delivery failed; the watcher retries), or `board` when the parent is the
  board, with a `card` id when the message opened a decision card. `gate`
  also opens a gate, which stops nudges until the sender clears it. `handoff`
  and `closed` close the link.
- `wf.reply`, `wf.gate`, `wf.close`, `wf.inbox`, `wf.show`, `wf.board`,
  `wf.adopt` and `wf.sweep` cover the rest of a link's life.
- `wf.launchTicket`, `wf.cards`, `wf.answer` and `wf.feed` work with the
  [board](board.md).
- `wf.waiting.add`, `wf.waiting.resolve` and `wf.waiting.list` keep an overseer
  thread's waiting list.

```js
async () => wf.send({ kind: "note", body: "tests green at d4db97c" })
```

The service stores a note without waking anyone, and the note reaches the
parent as a digest line on its next wake. The kinds in the config's `wake` list
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
pending requests once, escalates after repeated misses, and closes links whose
thread was deleted or archived.

## console

```js
async () => {
  console.log("starting");
  console.warn("this goes to logs too");
  return "done";
}
```

returns `{"result": "done", "logs": [...]}` with both lines.
