# Configuration

## The config file

The config is optional JSON at `config.json` in the config directory.
`T3CM_CONFIG` points at a different file. Whatever it holds is deep-merged over
the built-in defaults, so a file only needs the keys you change:

| OS | Config and state directory |
|---|---|
| Linux | `$XDG_CONFIG_HOME/t3-codemode/` (default `~/.config/t3-codemode/`) for config, `$XDG_STATE_HOME/t3-codemode/` (default `~/.local/state/t3-codemode/`) for state |
| macOS | `~/Library/Application Support/t3-codemode/` for both |
| Windows | `%LOCALAPPDATA%\t3-codemode\` for both |

The service reads the file at start. Restart it after an edit:

| OS | Restart |
|---|---|
| Linux | `systemctl --user restart t3-codemode` |
| macOS | `launchctl kickstart -k gui/$(id -u)/com.ottoops.t3-codemode` |
| Windows | `Stop-ScheduledTask t3-codemode; Start-ScheduledTask t3-codemode` |

```json
{
  "deny": ["t3_project_delete", "t3_environment_unlink", "t3_thread_interrupt"],
  "defaultTimeoutMs": 120000,
  "board": { "settleAfterHours": 48 }
}
```

A deep merge replaces arrays whole, so this `deny` replaces the default list,
and it has to repeat the two default entries to keep them.

## Keys

### Sandbox and runs

| Key | Default | Does |
|---|---|---|
| `deny` | `["t3_project_delete", "t3_environment_unlink"]` | T3 tools scripts may not call. The host refuses them; see [Security model](security.md). |
| `cpuBudgetMs` | `2000` | JavaScript CPU time per run. Time spent awaiting host calls does not count. |
| `defaultTimeoutMs` | `60000` | Wall-clock limit when a call gives no `timeout_ms` |
| `maxTimeoutMs` | `600000` | The largest `timeout_ms` a call may ask for |
| `maxConcurrentRuns` | `16` | `execute` and `search` runs allowed at once |
| `runQueueMs` | `10000` | How long a run waits for a free slot before it fails with a retryable busy error |

Each run can use up to 64 MB, so the default `maxConcurrentRuns` bounds sandbox
memory at 1 GB, which equals the Linux unit's `MemoryHigh` before the rest of
the service is counted. On Linux, raise it only with `MemoryHigh` and
`MemoryMax`. The launchd agent and the Windows task set no memory limit.

### Workflow layer

| Key | Default | Does |
|---|---|---|
| `watcherIntervalSecs` | `60` | How often the watcher runs a pass |
| `maxHoldMinutes` | `30` | How long a wake to a busy thread waits in the outbox |
| `link` | `{"down": 45, "miss": 2, "stall": 240}` | Default link policy: idle minutes before a nudge, unanswered nudges before an escalation, busy minutes with nothing sent before the parent hears of it |
| `wake` | `["event", "gate", "blocker", "handoff", "closed", "report", "escalate", "reply"]` | Message kinds that wake the receiver. Other kinds are stored as notes. |
| `bodyCap`, `replyCap` | `1500`, `1500` | Characters of a wake or reply body sent inline; the rest stays behind its handle |
| `digestLines` | `6` | Note lines in the digest that rides on the next wake |
| `roles` | see below | Per-role overrides for each child role |

The built-in `roles` give an overseer's `ticket` children `down` 75, `stall`
480, `bodyCap` 600, and a `wake` list without `event` and `reply`, so a
ticket's routine events reach an overseer as notes. `ticket` has `worker`
children with the default policy, and `worker` has none.

### Board

All under `board`:

| Key | Default | Does |
|---|---|---|
| `ticketModel`, `ticketEffort` | `claude-opus-5-5`, `high` | Model and effort for ticket threads the board launches |
| `askModel`, `askEffort` | `claude-haiku-5-5`, unset | Model and effort for the ask box's scratch threads |
| `instanceId` | `claudeAgent` | The T3 provider instance launches use |
| `runtimeMode` | `full-access` | Runtime mode for launches. The service has no thread to inherit a mode from. |
| `askPollSecs` | `4` | How often an open ask is checked for its answer |
| `askTimeoutMinutes` | `15` | How long an ask may run |
| `feed` | `50` | Activity feed entries kept on the page |
| `settleAfterHours` | `24` | Hours a done ticket or closed ask stays in the main lists; `0` turns auto-settle off |
| `openLinks` | `desktop` | `desktop`: thread links open in the running T3 Code desktop app, or in a new tab when it is not running. The app check needs Windows, natively or through WSL, so macOS and Linux without WSL always get a tab. `tab`: always a new tab. |

### Upstreams

`upstreams` lists other MCP servers to wrap. It has its own page:
[Upstreams](upstreams.md).

## Environment

| Variable | Does |
|---|---|
| `T3CM_CONFIG` | Path of the config file |
| `T3CM_STATE_DIR` | State directory, same as `--state-dir` |
| `T3CM_PORT` | Port for `serve`, `status`, `exec` and `search` (default `47731`) |
| `T3CM_T3_BIN` | The `t3` binary used to mint pairing codes; see [Security model](security.md#t3-sign-in) |
| `T3CM_T3_ORIGIN` | T3's origin, when `server-runtime.json` has the wrong one |
| `T3CODE_HOME` | T3's home directory (default `~/.t3`) |
| `T3CM_TZ` | Time zone for times the board and `wf` show (default `America/Denver`; an unknown zone falls back to UTC) |
| `RUST_LOG` | Log level |

## State

State lives in SQLite at `store.db` in the state directory above
(`--state-dir` or `T3CM_STATE_DIR` override it). It holds the T3 session, `mem`
and `vars`, the workflow links and messages, the board's cards and feed, and the
cached T3 catalog for when T3 is down. Everything survives restarts.

On Linux and macOS the directory is `0700` and the database `0600`. On Windows
t3cm sets no ACL of its own: the store relies on the per-user ACL that
`%LOCALAPPDATA%` already carries, which grants access to you, SYSTEM and
Administrators, so a `--state-dir` outside your profile gets whatever its parent
grants.

Uninstalling (`install.sh --uninstall` or `install.ps1 -Uninstall`) keeps the
store. Delete the state directory to drop memory and links.
