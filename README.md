# t3-codemode

`t3cm` is a Code Mode MCP service for T3 Code. It puts T3's
MCP tools behind two tools, `search` and `execute`, in the style of Cloudflare's
Code Mode. Instead of 74 tool schemas (about 166k chars of JSON) in every
session, a client sees two tools (about 3.3k chars for the whole `tools/list`).
The model finds what it needs with code over a catalog, then runs code against
it in a sandbox.

Beyond T3's tools, the sandbox carries:

- a persistent memory store (`mem`, `vars`),
- a thread-workflow layer (`wf`): parent and child links between threads, gates,
  wakes and a watcher that nudges stalled threads,
- a local web board that acts as the root overseer for ticket threads.

Status: prototype. Linux only (systemd user unit, T3 runtime discovery via
`/proc`).

## Requirements

- Linux with systemd user services
- Rust 1.95 or newer (`oxc` is pinned to `=0.146.0` because later releases need a newer compiler)
- A running T3 Code server and its `t3` CLI
- Claude Code, if you want the installer to register the MCP server for you

## Install

```bash
git clone https://github.com/potto007/t3-codemode
cd t3-codemode
deploy/install.sh              # build, install ~/.local/bin/t3cm, enable the user unit, register with Claude Code
deploy/install.sh --uninstall  # reverse it (the store is kept)
```

The unit (`deploy/t3-codemode.service`) runs `t3cm serve` on `127.0.0.1:47731`
with `Restart=on-failure`, `LimitNOFILE=1048576` and `MemoryMax=512M`. The
installer registers it with:

```bash
claude mcp add --transport http --scope user t3cm http://127.0.0.1:47731/mcp \
  --header 'X-T3-Thread: ${T3CODE_THREAD_ID}'
```

Logs: `journalctl --user -u t3-codemode` (`RUST_LOG` sets the level).

| Command | Does |
|---|---|
| `t3cm serve [--port N] [--state-dir D] [--no-watcher]` | MCP Streamable HTTP at `/mcp`, `/healthz`, the board at `/` |
| `t3cm stdio [--state-dir D] [--watcher]` | MCP over stdio; the caller is `$T3CODE_THREAD_ID` |
| `t3cm status` | The running service's `/healthz`: version, uptime, T3 connection, token expiry, watcher last pass, store path |
| `t3cm exec <file\|->`, `t3cm search <file\|->` | Call the running service's tools, for humans and scripts |
| `t3cm auth` | Acquire a fresh T3 session now; prints only its expiry |
| `t3cm types [--from tools.json]` | Print the generated TypeScript declarations and their size |

## Config

Optional JSON at `~/.config/t3-codemode/config.json` (`T3CM_CONFIG` points
elsewhere), deep-merged over the built-in defaults. It sets:

- `deny`: T3 tools scripts may not call (default `t3_project_delete`, `t3_environment_unlink`)
- `watcherIntervalSecs` (60), `cpuBudgetMs` (2000), `defaultTimeoutMs` (60000), `maxTimeoutMs` (600000), `maxHoldMinutes` (30)
- `link`, `wake`, `bodyCap`, `replyCap`, `digestLines` and per-role `roles` policy for the workflow layer
- `board`: `ticketModel` (`claude-opus-5-5`), `ticketEffort` (`high`), `askModel` (`claude-haiku-5-5`), `askEffort`, `instanceId` (`claudeAgent`), `runtimeMode` (`full-access`, for launches from the service, which has no thread to inherit a mode from), `askPollSecs`, `askTimeoutMinutes`, `feed` (50)

State lives in SQLite at `$XDG_STATE_HOME/t3-codemode/store.db` (default
`~/.local/state/t3-codemode/`; `--state-dir` or `T3CM_STATE_DIR` override). The
directory is `0700` and the database `0600`. Everything survives restarts.

## The two tools

**search** `{code}` runs an async arrow function whose only globals are
`catalog` and `console`. `catalog` is an array of
`{name, kind, description, signature, inputSchema}`: one entry per T3 tool
(`kind: "t3"`, `signature` is a generated TypeScript declaration with input and
result types) plus the `wf`, `mem` and `vars` API (`kind: "wf"` / `"mem"`).

```js
async () => catalog.find(t => t.name === "t3_thread_send").signature
```

**execute** `{code, session?, timeout_ms?, thread?}` runs an async arrow
function and returns `{"result": ..., "logs": [...]}`. TypeScript annotations
are stripped. Loose input is accepted: markdown fences, `export default`, a
named function, or a bare body whose last expression is returned.

Globals inside `execute`, and nothing else (no fs, require, process, fetch,
import or timers):

| Global | What it is |
|---|---|
| `t3` | Every T3 tool by exact name (`t3.t3_thread_list(input)`), an alias without the `t3_` prefix, and `t3.call(name, input)`. A failed call throws an `Error` with T3's message. |
| `mem` | `get(key)`, `set(key, value)`, `del(key)`, `list(prefix?)` on the session namespace; `mem.ns(name)` opens a shared one (for example `work:widgets#41`). JSON values, 1 MB each. |
| `vars` | A plain object restored before the run and saved after it (also after a thrown error, not after a CPU or wall kill). REPL state across calls and restarts. |
| `wf` | The workflow layer, acting as the calling thread. |
| `console` | `log`/`info`/`debug`/`warn`/`error`, captured to `logs` (50 lines / 4,000 chars). |

`session` names the `mem`/`vars` namespace; it defaults to `thread:<caller>`, or
`default` with no caller. The caller is the `X-T3-Thread` header, else the
`thread` input, else none.

Results over 24,000 chars are cut structurally: the output stays valid JSON, the
largest values are cut first, cuts are marked `--- TRUNCATED ---`, and a
`truncated` note says so. Errors come back as `isError` with `Error: <message>`
and any logs; the code is never echoed.

## The workflow layer (`wf`)

- `wf.init({role})`, `wf.openChild({role, label})` returns a `brief` block the
  parent pastes into the child's launch brief; the child runs the
  `wf.join({parent, label, role, down, miss, stall})` call it contains.
- `wf.send({kind, body, gate?, wake?})`, kinds `note | event | gate | blocker |
  report | handoff | closed`. Returns `{handle, delivery}`: `noted`, `sent`, or
  `queued` (the parent is mid-turn, or delivery failed; the watcher retries).
- `wf.reply`, `wf.gate`, `wf.close`, `wf.inbox`, `wf.show`, `wf.board`,
  `wf.adopt`, `wf.sweep`, and the overseer waiting list `wf.waiting.add /
  resolve / list`.

A wake is one compact `t3_thread_send` (mode `queue`) of the form
`[t3cm <kind> from=<label> msg=<handle>]`. Wakes to a busy thread wait in an
outbox and go out as one combined message when it is idle (held at most 30
minutes). A watcher task runs every 60 s: it nudges idle children, reports
stalls and pending requests once, escalates after repeated misses, and closes
links whose thread was deleted or archived.

## The board

The service is the root of the link tree, in place of an LLM overseer thread. A
fixed virtual node `board` always exists; ticket threads join it with
`wf.join({ parent: "board", label, role: "ticket" })`. Gates, blockers, reports
and the watcher's escalations become **decision cards** with one button per
`reply "X"` phrase in the body; notes and events go to an activity feed;
`handoff` / `closed` mark the ticket done. Answering a card sends the ticket one
reply and resolves the card.

The page is `http://127.0.0.1:47731/`: decisions with reply buttons, the ticket
tree with live status, an activity feed, a launch box for issue URLs and an ask
box (a short-lived Haiku thread answers from the board). It is one embedded HTML
file with no external requests, light and dark, and raises a browser
notification for each new card while the tab is hidden.

| Route | Does |
|---|---|
| `GET /` | The page, with a per-process CSRF token embedded |
| `GET /api/state` | `{cards, tree, feed, asks, counts, service}` |
| `GET /api/events` | SSE: a `state` event on every change |
| `GET /api/msg/{handle}` | One stored message in full |
| `POST /api/cards/{id}/answer` `{text}` | Reply to the card's ticket and resolve it |
| `POST /api/cards/{id}/dismiss` | Resolve without messaging |
| `POST /api/launch` `{issues: [...]}` | `wf.launchTicket` for each (at most 20) |
| `POST /api/ask` `{question}` | Ask a scratch thread about the board |

Every board route requires `Host` to be `127.0.0.1:<port>` or `localhost:<port>`
and refuses any other `Origin`; POSTs also need the `X-T3CM-CSRF` header.

### Ticket workflow and the `cbrip:t3-ticket` skill

`wf.launchTicket({ issue, project?, title?, model?, effort? })` finds the T3
project whose workspace `origin` remote is the issue's GitHub repo and launches
a thread that works the issue and reports to the board. Today that thread is
started with the `cbrip:t3-ticket` skill, which lives in a separate plugin and
is not part of this repo. Without that skill installed, launched threads do not
know the workflow, so the board and `wf` are the part of this project that works
standalone.

## Auth

On start, and when the stored token has under 3 days left or T3 answers 401, the
service runs T3's MCP OAuth flow against the local server: it registers a
client, mints a 5-minute pairing code with the `t3` CLI, approves it as
`full-access`, and exchanges the code with PKCE for a 30-day bearer token. The T3
origin comes from `~/.t3/userdata/server-runtime.json`; the `t3` binary is
`$T3CM_T3_BIN`, else the running server's `/proc/<pid>/exe`, else the newest
`~/.t3/wsl-runtime/sha256-*/t3`.

## Security

- Binds 127.0.0.1 only; requests with a non-local `Origin` are refused (DNS
  rebinding).
- The token lives only in the store and in memory. It never enters the sandbox,
  tool output or logs; any error text that contains it is redacted.
- Each run gets a fresh QuickJS runtime: 64 MB memory, 512 KB stack, a 2 s
  JavaScript CPU budget (time awaiting host calls does not count), and a wall
  timeout (`timeout_ms`, default 60 s, max 600 s). No fs, network, timers,
  modules, `SharedArrayBuffer` or `Atomics`. The deny list is enforced in the
  host functions, not in the prompt.
- Any local process that can reach the port acts with the service's T3 session.
  That is the same trust boundary as the `t3` CLI for the same user.

## Development

```bash
CARGO_BUILD_JOBS=8 cargo test --release --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
t3cm bench -n 1000 --tools tests/fixtures/t3-tools.json   # hidden: latency and RSS against the in-process FakeT3
```

`tests/fixtures/t3-tools.json` is a snapshot of T3's 74 tools; at runtime the
catalog comes live from `tools/list` (refreshed every 10 minutes, cached in the
store for when T3 is down).

## License

MIT, see `LICENSE`.
