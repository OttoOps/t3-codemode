# t3-codemode

`t3cm` is a Code Mode MCP service for T3 Code. It puts T3's MCP tools, and the
tools of any other MCP servers you configure (upstreams), behind two tools, `search` and `execute`, in the style of Cloudflare's
Code Mode. Instead of 74 tool schemas (about 166k chars of JSON) in every
session, a client sees two tools (about 3.3k chars for the whole `tools/list`).
The model finds what it needs with code over a catalog, then runs code against
it in a sandbox.

Beyond T3's tools, the sandbox carries:

- any other MCP server as an upstream, under its own namespace,
- a persistent memory store (`mem`, `vars`),
- a thread-workflow layer (`wf`): parent and child links between threads, gates,
  wakes and a watcher that nudges stalled threads,
- a local web board that acts as the root overseer for ticket threads.

Status: prototype. It builds and passes its tests on Linux, macOS and Windows.
The installer and the service unit are Linux and systemd only for now.

Documentation: https://ottoops.com/t3-codemode/

## Requirements

- Linux with systemd user services, for `deploy/install.sh`. On macOS and
  Windows, run `t3cm serve` yourself for now.
- Rust 1.95 or newer (`oxc` is pinned to `=0.146.0` because later releases need a newer compiler)
- A running T3 Code server and its `t3` CLI
- Claude Code, if you want the installer to register the MCP server for you

## Install

```bash
git clone https://github.com/OttoOps/t3-codemode
cd t3-codemode
deploy/install.sh              # build, install ~/.local/bin/t3cm, enable the user unit, register with Claude Code
deploy/install.sh --uninstall  # reverse it (the store is kept)
```

The unit (`deploy/t3-codemode.service`) runs `t3cm serve` on `127.0.0.1:47731`
with `Restart=on-failure`, `LimitNOFILE=1048576`, `MemoryHigh=1G` and
`MemoryMax=1536M`. The
installer registers it with:

```bash
claude mcp add --transport http --scope user t3cm http://127.0.0.1:47731/mcp \
  --header 'X-T3-Thread: ${T3CODE_THREAD_ID}'
```

Logs: `journalctl --user -u t3-codemode` (`RUST_LOG` sets the level).

`/healthz` returns `version`, `uptimeSecs`, `t3` (connection and token expiry),
`tools` (the T3 tool count), `watcher`, `store`, and `upstreams`: one object per
configured upstream, disabled ones included, in config order, with `name`,
`transport`, `enabled`, `state` (`disabled`, `idle`, `connecting`, `ready`,
`backoff`), `tools`, `restarts`, `lastError` (redacted), `retryAt` and `server`
(the upstream's `serverInfo` `name`, `title` and `version`, each cut to 100
characters). Every route answers only requests whose `Host` is `127.0.0.1`,
`localhost` or `[::1]` (any port), and refuses others with 403, which shuts out
DNS rebinding.

| Command | Does |
|---|---|
| `t3cm serve [--port N] [--state-dir D] [--no-watcher]` | MCP Streamable HTTP at `/mcp`, `/healthz`, the board at `/` |
| `t3cm stdio [--state-dir D] [--watcher]` | MCP over stdio; the caller is `$T3CODE_THREAD_ID` |
| `t3cm status` | The running service's `/healthz`: version, uptime, T3 connection, token expiry, watcher last pass, store path, upstreams |
| `t3cm exec <file\|->`, `t3cm search <file\|->` | Call the running service's tools, for humans and scripts |
| `t3cm auth` | Acquire a fresh T3 session now; prints only its expiry |
| `t3cm types [--from tools.json]` | Print the generated TypeScript declarations and their size |

## Config

Optional JSON at `config.json` in the config directory (`T3CM_CONFIG` points
elsewhere), deep-merged over the built-in defaults:

| OS | Config and state directory |
|---|---|
| Linux | `$XDG_CONFIG_HOME/t3-codemode/` (default `~/.config/t3-codemode/`) for config, `$XDG_STATE_HOME/t3-codemode/` (default `~/.local/state/t3-codemode/`) for state |
| macOS | `~/Library/Application Support/t3-codemode/` for both |
| Windows | `%LOCALAPPDATA%\t3-codemode\` for both |

It sets:

- `deny`: T3 tools scripts may not call (default `t3_project_delete`, `t3_environment_unlink`)
- `upstreams`: other MCP servers to wrap (see Upstreams below)
- `watcherIntervalSecs` (60), `cpuBudgetMs` (2000), `defaultTimeoutMs` (60000), `maxTimeoutMs` (600000), `maxHoldMinutes` (30)
- `maxConcurrentRuns` (16): `execute` and `search` runs allowed at once; more wait up to `runQueueMs` (10000) for a slot, then fail with a retryable busy error. Each run can use up to 64 MB, so the default bounds sandbox memory at 1 GB, under the unit's `MemoryHigh`
- `link`, `wake`, `bodyCap`, `replyCap`, `digestLines` and per-role `roles` policy for the workflow layer
- `board`: `ticketModel` (`claude-opus-5-5`), `ticketEffort` (`high`), `askModel` (`claude-haiku-5-5`), `askEffort`, `instanceId` (`claudeAgent`), `runtimeMode` (`full-access`, for launches from the service, which has no thread to inherit a mode from), `askPollSecs`, `askTimeoutMinutes`, `feed` (50), `settleAfterHours` (24; `0` turns auto-settle off), `openLinks` (`desktop`: thread links open in the running T3 Code desktop app, or in a new tab when it is not running; `tab`: always a new tab)

State lives in SQLite at `store.db` in the state directory above
(`--state-dir` or `T3CM_STATE_DIR` override). On Linux and macOS the directory
is `0700` and the database `0600`. On Windows t3cm sets no ACL of its own: the
store relies on the per-user ACL that `%LOCALAPPDATA%` already carries, which
grants access to you, SYSTEM and Administrators, so a `--state-dir` outside
your profile gets whatever its parent grants. Everything survives restarts.

## The two tools

**search** `{code}` runs an async arrow function whose only globals are
`catalog` and `console`. `catalog` is an array of
`{name, kind, server, description, signature, inputSchema}`, in this order: one
entry per T3 tool (`kind: "t3"`, `server: "t3"`, the bare T3 name; `signature`
is a generated TypeScript declaration with input and result types), one per
upstream tool (`kind: "mcp"`, `server` is the upstream's namespace, `name` is
`ns.tool`; the signature reads `ns.tool(...)`, or `ns["raw-name"](...)` when the
tool name is not an identifier), then the `wf`, `mem` and `vars` API (`kind` and
`server` are `"wf"` / `"mem"`).

```js
async () => catalog.find(t => t.name === "t3_thread_send").signature
async () => catalog.filter(t => t.server === "cbm").map(t => t.name)
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
| `<ns>` | One per enabled upstream, named by its `name`, present even before its tool list is known: each allowed tool by exact name (`cbm.search_graph(input)`, `cbm["get-thing"](input)`), a sanitized alias when it is free (`cbm.get_thing`), and `cbm.call(name, input)`. A tool named `call` is reached with `cbm.call("call", input)`. |
| `tools` | `tools.call("ns.tool", input)` for any namespace, `t3` included (which keeps the `t3_` alias and the deny list). The name splits at the first dot, so `tools.call("cbm.a.b")` calls tool `a.b`. |
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

## Upstreams: wrap any MCP server

An upstream is another MCP server, over stdio or Streamable HTTP (the legacy
HTTP+SSE transport is not supported), listed under `upstreams` in
`~/.config/t3-codemode/config.json` (or `$T3CM_CONFIG`). Its tools join the
catalog under its namespace; the service holds its credentials, which never
reach the sandbox, tool output, `/healthz` or logs. T3 itself stays built in as
the namespace `t3`, with its own OAuth client.

```json
{
  "upstreams": [
    {
      "name": "cbm",
      "transport": "stdio",
      "stdio": { "command": "codebase-memory-mcp" },
      "timeout_ms": 120000,
      "deny": ["delete_project", "index_repository"]
    },
    {
      "name": "linear",
      "transport": "http",
      "http": {
        "url": "https://mcp.linear.app/mcp",
        "headers": { "Authorization": "Bearer ${file:~/.config/t3-codemode/linear.token}" }
      },
      "allow": ["list_*", "get_*", "search_*"]
    }
  ]
}
```

| Key | Default | Rule |
|---|---|---|
| `name` | required | the JS namespace: `^[a-z][a-z0-9_]{0,31}$`, unique, not `t3`, `mem`, `vars`, `wf`, `console`, `tools`, `catalog` or a JS reserved word |
| `transport` | required | `"stdio"` or `"http"`; exactly the matching section is present |
| `stdio.command` | required | a bare name (searched on `PATH`, then `$HOME/.local/bin`) or a path; a relative path (`./bin/srv`) is resolved against `stdio.cwd`; on Windows a name with no extension also tries each `PATHEXT` extension (`npx` finds `npx.cmd`) |
| `stdio.args`, `stdio.cwd` | `[]`, service cwd | no references |
| `stdio.env` | `{}` | values may hold references |
| `http.url` | required | `http://` or `https://`; references allowed in the userinfo, path and query, not in the host |
| `http.allow_insecure` | `false` | `true` allows credentials (any reference in the URL or headers) over `http://` to a non-loopback host |
| `http.headers` | `{}` | values may hold references |
| `enabled` | `true` | `false`: no global, no catalog entries, never connects |
| `timeout_ms` | `60000` | per request, 100 to 600000 |
| `allow`, `deny` | `["*"]`, `[]` | tool-name globs (`*`, `?`); deny wins. A denied tool is absent from the catalog and its calls throw |

**Namespacing.** Each upstream's `name` is its namespace: one frozen global
in `execute` (`cbm.search_graph(input)`, `cbm.call(name, input)`), the
`server` of its catalog entries, and the prefix of their names
(`cbm.search_graph`). `tools.call("cbm.search_graph", input)` reaches the same
tool by its qualified name. T3 tools keep their bare names (`t3_thread_send`)
and the built-ins are `mem.*`, `wf.*` and `vars`; the reserved names keep the
three sets disjoint. A tool name that is not an identifier is reached as
`cbm["get-thing"]`, with a sanitized alias (`cbm.get_thing`) when no other tool
claims it.

**References.** A value in `stdio.env`, `http.headers` or `http.url` may mix literal text
with `${env:NAME}` (the service's environment; under systemd, the unit's) and
`${file:/abs/path}` or `${file:~/path}` (the file's content, trailing newlines
trimmed); `$${` is a literal `${`. Secrets go in only by reference: an
`Authorization`, `Proxy-Authorization` or `Cookie` header, or any header or env
name containing `TOKEN`, `SECRET`, `KEY`, `PASSWORD`, `PASSWD`, `AUTH`, `COOKIE`
or `CREDENTIAL`, with a literal value is a config error. References are resolved
at every connect, so a rotated file is picked up on the next reconnect. Every
resolved value is redacted (`[redacted]`) from tool results before they enter
the sandbox, and from errors and output, including its JSON-escaped form. A
URL's password and the values of query parameters whose name looks like a
secret (`?api_key=`) are redacted the same way.

**Lifecycle.** Nothing connects at start-up beyond a background warm-up; each
upstream connects on first use (`search`, `execute` or the 10-minute catalog
refresh). Connects run in the background: `search` and `execute` wait at most
2 s, and only for an upstream whose tools have never been listed; otherwise
they use the last tool list, and a call into a namespace waits for its own
connect. A connect (spawn or HTTP handshake plus the full `tools/list`) may
take `timeout_ms` or 15 s, whichever is longer. A failure puts only that
upstream in backoff: the first failure retries on next use, repeated failures
back off from 1 s doubling to 60 s. Its last tool list stays in the catalog. A
failed call is never retried. A stdio server that exits between calls is
restarted on the next use without failing that call, and three timeouts in a
row restart a connection. A re-list is capped at 15 s, and the watcher
refreshes upstreams on their own task, so a hung upstream never delays the
workflow watcher. A server's `tools/list_changed` notification re-lists it,
and the next `search` or `execute` sees the new catalog. Connects and failures are logged
(`upstream connected`, `upstream unavailable: <redacted reason>`); an
upstream's stderr is logged at `debug` only, redacted.

**Security.**

- Credentials enter only through `${env:..}` and `${file:..}` references in
  `stdio.env`, `http.headers` and `http.url`, resolved by the service at
  connect time. The
  sandbox never sees them: results are redacted before they enter it, so code
  cannot rebuild a secret from a result, and errors, final output, `/healthz`
  `lastError` and log lines are redacted too.
- A stdio upstream runs as a child of the service, in its own process group
  or, on Windows, its own Job Object, with only `PATH`, `HOME`, `USER`,
  `LOGNAME`, `SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `TMPDIR`, `TERM` and
  the `XDG_*` base directories from the service's environment, plus its own
  `env`, so no upstream sees another's `${env:..}` credentials.
  Pass anything else it needs through its `env`
  (`"NODE_OPTIONS": "${env:NODE_OPTIONS}"`). Stopping it kills the whole
  group, so a server behind a wrapper (`npx`, `uvx`, `sh`) stops too. `args`
  are visible in `ps`, which is why they cannot hold references. On Windows the
  allowlist also passes the system variables programs need to start
  (`SYSTEMROOT`, `COMSPEC`, `PATHEXT`, `TEMP`, `USERPROFILE`, `APPDATA`,
  `LOCALAPPDATA` and the rest of `client_stdio::INHERITED_ENV_WINDOWS`), and
  matches names in any case.
- An HTTP upstream never follows redirects (a 3xx is an error), so its headers
  never reach another host. Credentials over `http://` to a non-loopback host
  need `http.allow_insecure: true`. Error text never carries the URL.
- One message from an upstream (a stdio line, an HTTP body or SSE event) is
  capped at 16 MiB; a larger one fails the connection.
- `allow`/`deny` are enforced host-side: a denied tool has no catalog entry and
  no global, and `tools.call` on it throws without contacting the upstream.
  Use `deny` for destructive tools you never want a script to reach.
- An upstream's tool descriptions and results are its own text. Treat them as
  data from that server, as you would its direct MCP tools.

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

Finished tickets and answered asks are settled, never deleted. A ticket that
has been done, or an ask that has been closed, for `board.settleAfterHours`
leaves the main lists on the next watcher pass. A ticket is also moved off the
thread list in T3 (`t3_thread_organize` with `settle`, best effort: a T3 error
is logged and the board state still changes). The page's collapsed **Settled**
section keeps them, with a filter and a **Restore** button on each one;
restoring a ticket sends `unsettle` and restarts its settle clock. **Settle**
on a row and **Settle all done** do it by hand.

| Route | Does |
|---|---|
| `GET /` | The page, with a per-process CSRF token embedded |
| `GET /api/state` | `{cards, tree, feed, asks, settled, counts, service}` |
| `GET /api/events` | SSE: a `state` event on every change |
| `GET /api/msg/{handle}` | One stored message in full |
| `POST /api/cards/{id}/answer` `{text}` | Reply to the card's ticket and resolve it |
| `POST /api/cards/{id}/dismiss` | Resolve without messaging |
| `POST /api/launch` `{issues: [...]}` | `wf.launchTicket` for each (at most 20) |
| `POST /api/ask` `{question}` | Ask a scratch thread about the board |
| `POST /api/tickets/{thread}/settle`, `/restore` | Settle a done ticket, or bring a settled one back (400 if open, or not settled) |
| `POST /api/asks/{id}/settle`, `/restore` | The same for a closed ask; `id` is `q<n>` |
| `POST /api/settle-done` | Settle every done ticket and closed ask; returns `{tickets, asks}` counts |
| `POST /api/open` `{link}` | Open a T3 thread link in the running desktop app; returns `{opened}`, and the page opens a new tab when it is `false` (400 for a link that is not a T3 thread) |

Every board route requires `Host` to be `127.0.0.1:<port>` or `localhost:<port>`
and refuses any other `Origin`; POSTs also need the `X-T3CM-CSRF` header.

### Ticket workflow and the `t3-ticket` skill

`wf.launchTicket({ issue, project?, title?, model?, effort? })` finds the T3
project whose workspace `origin` remote is the issue's GitHub repo and launches
a thread that works the issue and reports to the board. That thread is started
with the `t3-ticket` skill, which lives in this repo's `skills/` folder and is
installed as an agent skill, apart from the service. Without that skill
installed, launched threads do not know the workflow, so the board and `wf` are
the part of this project that works standalone.

## Auth

On start, and when the stored token has under 3 days left or T3 answers 401, the
service runs T3's MCP OAuth flow against the local server: it registers a
client, mints a 5-minute pairing code with the `t3` CLI, approves it as
`full-access`, and exchanges the code with PKCE for a 30-day bearer token. The T3
origin comes from `~/.t3/userdata/server-runtime.json`; the `t3` binary is
`$T3CM_T3_BIN`, else the running server's executable when it is named `t3`
(`t3.exe` on Windows), else the newest `~/.t3/wsl-runtime/sha256-*/t3`, else
the newest `~/.t3/runtime/versions/*/t3` that T3's installer unpacks.

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
- Upstream secrets live only in the config's references and in memory; the same
  redaction applies to them, and to upstream results before the sandbox sees
  them. `allow`/`deny` per upstream are enforced in the host, not in the prompt.
  See [Upstreams](#upstreams-wrap-any-mcp-server) and `docs/src/upstreams-contract.md`.
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

`docs/build.sh` generates the API reference page from `t3cm types`, then builds
the docs site into `docs/book/` with mdBook 0.5.4. CI builds it on every pull
request and deploys it to GitHub Pages from `main`.

## Contributing

See the [contribution guide](.github/CONTRIBUTING.md): every commit carries a DCO
signoff (`git commit -s`), and pull requests merge with rebase. Report
vulnerabilities privately through the [security policy](.github/SECURITY.md).
Participation follows the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

MIT, see `LICENSE`.
