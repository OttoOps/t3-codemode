# Troubleshooting

Start with `t3cm status`. It prints the running service's `/healthz`, and most
problems show up there:

| Field | Tells you |
|---|---|
| `version`, `uptimeSecs` | Which build is running, and whether it restarted recently |
| `t3` | `connected`, `origin`, `tokenExpiresAt` and `lastError` for the T3 session |
| `tools` | How many T3 tools the catalog holds |
| `watcher` | `lastPass`, `alive` and `intervalSecs` for the workflow watcher |
| `runs` | `running`, `queued` and `max` for sandbox runs |
| `store` | The path of `store.db` |
| `upstreams` | One object per configured upstream, with its `state` and `lastError` |

The service logs to:

| OS | Logs | Service state |
|---|---|---|
| Linux | `journalctl --user -u t3-codemode` | `systemctl --user status t3-codemode` |
| macOS | `~/Library/Logs/t3-codemode/t3cm.log`, and `launchd.log` beside it for what the service writes to stderr | `launchctl print gui/$(id -u)/com.ottoops.t3-codemode` |
| Windows | `%LOCALAPPDATA%\t3-codemode\logs\t3cm.log` | `Get-ScheduledTask t3-codemode \| Get-ScheduledTaskInfo` |

A log file rolls to `t3cm.log.1` at start once it passes 10 MB. Set
`RUST_LOG=debug` in the service's environment for more.

## `is t3cm serving on 127.0.0.1:47731?`

`t3cm status`, `exec` or `search` could not reach the service. The message
goes on with the command that starts the service on your OS and where its logs
are.

- The service-state command in the table above shows whether it runs, and the
  log shows why it stopped. On Windows a stopped task starts again within five
  minutes unless it is disabled; `Enable-ScheduledTask t3-codemode` and
  `Start-ScheduledTask t3-codemode` bring it back now.
- `serve` fails with `binding 127.0.0.1:47731` when something else holds the
  port. Pick another with `--port` or `T3CM_PORT`, and register the MCP server
  with the same port.
- A service on another port needs `--port` or `T3CM_PORT` on the CLI too.

## `t3.connected` is false

`t3.lastError` says why:

- `T3 Code is not running here: no readable .../server-runtime.json`: start T3
  Code. If T3 keeps its home somewhere other than `~/.t3`, set `T3CODE_HOME`
  for the service.
- `no t3 binary found; set T3CM_T3_BIN`: the service could not find the `t3`
  CLI it uses to mint pairing codes. Set `T3CM_T3_BIN` to its path.
- `` `t3 ...` failed: ``: the `t3` CLI ran and failed. The rest of the message
  is its stderr.

If `lastError` is empty, the service log has the reason: a failed sign-in at
start goes to the log as `T3 auth not ready yet`. After fixing the cause, run
`t3cm auth` to sign in from your shell, with the service's `--state-dir` or
`T3CM_STATE_DIR` if it uses one, then restart the service. It loads the saved
token from that state directory, or signs in again on its own. See
[T3 sign-in](security.md#t3-sign-in) for the whole flow.

## A tool is missing from the catalog

- **A new T3 tool.** The catalog comes from T3's `tools/list`, refreshed every
  10 minutes. Restart the service to pick it up sooner.
- **An upstream's tools.** Check the upstream's `state` in `t3cm status`.
  `backoff` means it failed to connect; `lastError` says why and `retryAt` says
  when it tries again. `disabled` means `enabled: false` in the config. A tool
  that `allow` leaves out, or `deny` names, has no catalog entry at all.
- **The client's tool list.** Claude Code loads MCP servers at session start.
  A thread opened before you installed or re-registered `t3cm` has the old list
  until it restarts.

## `t3cm is busy`

`t3cm is busy: N runs are in progress and none finished within M ms; retry
shortly` means other runs held every one of the `maxConcurrentRuns` slots for
`runQueueMs`. It is safe to retry. If it happens often, `runs` in
`t3cm status` shows the load; raise `maxConcurrentRuns` only with the memory
math on [Configuration](config.md#sandbox-and-runs) in mind.

## A script is cut off

- `CPU budget exceeded`: the script spent more than `cpuBudgetMs` (2 s) running
  JavaScript. Time waiting on `t3`, `mem` or `wf` calls does not count, so the
  cause is a loop over a large result. Filter earlier, or split the work across
  runs and keep the progress in `vars`.
- `timed out after N ms (timeout_ms)`: the wall clock ran out, usually waiting
  on a slow tool. Pass a larger `timeout_ms`, up to `maxTimeoutMs`.
- `vars` keeps what a script set before a thrown error, but not what it set
  before a CPU or wall kill.

## A call says `target_required` or "not running inside a T3 thread"

`t3` calls reach T3 as the service's own MCP client, so T3 cannot tell which
thread ran the script. The `X-T3-Thread` header tells `t3cm` who is calling, for
`mem`, `vars` and `wf`, but T3 never sees it. A T3 tool that defaults to "this
thread" or "this project" therefore needs an explicit target in every script:
pass `allProjects: true`, a `projectId` or a `threadId`.

## `... is denied by the t3-codemode deny list`

The tool is on the config's `deny` list, and the host refuses it however the
script calls it. Retrying will not help. Remove it from `deny` if scripts
should reach it.

## The board answers 403

The board answers only on its own host and port: `http://127.0.0.1:<port>/` or
`http://localhost:<port>/`, where `<port>` is `47731` unless you changed it. A proxy, another hostname, or a page on another
origin gets 403. POSTs from your own scripts also need the `X-T3CM-CSRF` token
embedded in the page.

## Still stuck

Search the [issues](https://github.com/OttoOps/t3-codemode/issues), and open one
with the `t3cm status` output (secrets removed) and the steps that led there.
Report anything that looks like a security problem through the
[security policy](https://github.com/OttoOps/t3-codemode/blob/main/.github/SECURITY.md)
instead.
