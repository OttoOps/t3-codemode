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

The service reads the file at start, and again when
[`t3cm config`](#change-the-config) or the board's
[Settings](board.md#settings) saves a change. It does not watch the file, so
restart it after you edit the file another way or change a
[restart-only key](#restart-only-keys):

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

## Change the config

On a local board, [Settings](board.md#settings) edits every key with the same
checks and reload as the commands below, apart from the few files it leaves to
`t3cm config edit`. It uses the
[settings API](board.md#settings-api), which takes whole values in place of
paths, so it always saves a list whole.

`t3cm config` reads and changes the config file. Before it writes, it checks
the whole result as config load does, so a rejected change prints the field
that failed and leaves the file as it was. After `set`, `unset` or a saved
`edit`, it asks the running service to reload the file and prints the keys
that still need a restart.

| Command | Does |
|---|---|
| `t3cm config get [<path>]` | Prints the value in effect as JSON, defaults included. Without a path, it prints the whole config. |
| `t3cm config set <path> <value>` | Changes one key. The value is parsed as JSON, and a value that is not JSON is taken as a string. |
| `t3cm config unset <path>` | Removes one key from the file so that its default applies, or removes one item from a list. |
| `t3cm config edit` | Opens a copy of the file in `$VISUAL` or `$EDITOR` and replaces the file once the copy is valid. |
| `t3cm config validate` | Checks the file and exits 1 when it is invalid. |

```sh
t3cm config get board.feed
t3cm config set board.feed 80
t3cm config set deny '["t3_project_delete", "t3_environment_unlink", "t3_thread_interrupt"]'
t3cm config set pools.opus '{"driver": "claudeAgent", "model": "claude-opus-5-5", "members": [{"instanceId": "claude-a"}]}'
t3cm config set pools.opus.members.0.weight 3
t3cm config set 'pools."gpt-6.1".effort' high
t3cm config unset board.feed
```

A path joins key names with dots and names a list item by its index from 0,
and setting the index one past the end appends an item. A name that holds a dot goes in
double quotes, with single quotes around the whole path so that the shell
passes them through. To save a string that also reads as JSON, such as `5` or
`true`, quote it twice, as in `'"5"'`.

Each `set` has to leave a valid config, so keys that depend on each other need
an order or one value that holds them all. A new pool is set whole, and
`board.remote.origin` is set before `board.remote.mode`. An edit inside a list
that the key inherits, such as `deny.2` from the defaults or a child role's
`wake.0` from `wake`, copies that list into the file first. A copied list no
longer follows the one it came from.

`get` needs a valid file, and `set` and `unset` write only when their one
change leaves the file valid. Use `edit` to repair a file with several errors
or broken JSON.

When a change adds or alters a pool, t3cm asks the running service whether
each member of that pool is a T3 provider instance and has the pool's
`driver`. A change that repairs another key does not ask about a pool it
leaves alone. When t3cm cannot read a pool's earlier entry, it counts that
pool as changed, so an `edit` that repairs broken JSON asks about every pool.
`validate` asks about every pool. If the service or T3 does not answer, or you
pass `--no-t3`, t3cm warns that it did not check the members and goes on,
since each pick checks them again. `--port` names the service's port when it
is not the default.

### Reload

A reload applies the whole file, so it also picks up what you edited by hand
since the service last read it. The service checks the file first and keeps
the config it was running when the file is invalid. A `set` that changes
nothing still reloads.

The reload goes to the service on `--port` or `T3CM_PORT`, and that service
reloads only when it reads the file you changed. If it reads another file,
t3cm names that file. If no service answers, or the service is older than this
t3cm, t3cm says that it did not reload and prints the restart command. t3cm
saves the file in each case. When the service rejects the saved file, t3cm
prints its error and exits 1.

Work already under way can finish with the value it started with. An open
link keeps its `link` numbers, a run keeps its CPU and time limits, and an
open ask keeps its poll interval. A `t3cm stdio` process reads the file only
at start.

### Restart-only keys

| Key | Why it needs a restart |
|---|---|
| `maxConcurrentRuns` | It sets the service's run slots and memory bound at start. |
| `upstreams` | Every key under it, `allow` and `deny` included, belongs to connections and a catalog built at start. |
| `board.remote` | It sets who can reach the board from another device. To revoke a login, change it and restart. |

Every other key takes effect on reload. The service keeps the starting value
of these three until it restarts, and each reload names the ones whose saved
value differs from that.
[ADR 0009](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0009-reload-config-in-place.md)
records why.

### How t3cm writes the file

A write replaces the file in one step. It keeps the file's permissions, a
symlink to it, and the formatting of the values it does not change. t3cm
refuses to write when the file changed after it read it, and you run the
command again. `t3cm config`, the board, `t3cm board expose` and
`t3cm board unexpose` take turns through a lock file beside the config,
`.config.json.t3cm-lock` for the default name. Your editor does not take that
lock, so its save during a write can still be lost.

When the copy from `edit` is invalid and t3cm runs in a terminal, it offers to
open the copy again. If t3cm does not save a copy that you changed, it keeps
that copy and prints its path.

## Keys

### Sandbox and runs

| Key | Default | Does |
|---|---|---|
| `deny` | `["t3_project_delete", "t3_environment_unlink"]` | T3 tools scripts may not call. The host refuses them; see [Security model](security.md). |
| `cpuBudgetMs` | `2000` | JavaScript CPU time per run. Time spent awaiting host calls does not count. |
| `defaultTimeoutMs` | `60000` | Wall-clock limit when a call gives no `timeout_ms` |
| `maxTimeoutMs` | `600000` | The largest `timeout_ms` a call may ask for, at least 100 |
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
| `ticketModel`, `ticketEffort` | `claude-opus-5-5`, `high` | Legacy fallback for ticket launches when `models.ticket` is unset in repo and user workflow configs. Move these into `models.ticket`; `t3cm doctor` warns when they are set. |
| `askInstanceId` | unset | Provider instance for the ask box when `askPool` is unset. When unset, defaults to an enabled `claudeAgent` instance for the default ask model. |
| `askModel`, `askEffort` | `claude-haiku-5-5`, unset | Model and effort for the ask box's scratch threads when `askPool` is unset. |
| `askPool` | unset | Pool from `pools` for the ask box. Takes precedence over `askInstanceId`, `askModel` and `askEffort`, running the picked member's model. See [Pools](#pools). |
| `instanceId` | `claudeAgent` | Legacy fallback T3 provider instance for ticket launches when `models.ticket` is unset. No longer applies to the ask box; use `models.ticket` for tickets and `board.askInstanceId` for the ask box. `t3cm doctor` warns when it is set. |
| `pool` | unset | Legacy fallback pool from `pools` for ticket launches when `models.ticket` is unset. No longer applies to the ask box; use `models.ticket` for tickets and `board.askPool` for the ask box. `t3cm doctor` warns when it is set. See [Pools](#pools). |
| `runtimeMode` | `full-access` | Runtime mode for launches. The service has no thread to inherit a mode from. |
| `askPollSecs` | `4` | How often an open ask is checked for its answer |
| `askTimeoutMinutes` | `15` | How long an ask may run |
| `feed` | `50` | Activity feed entries kept on the page |
| `settleAfterHours` | `24` | Hours a done ticket or closed ask stays in the main lists; `0` turns auto-settle off |
| `reconcileActiveMinutes` | `2` | Minutes between GitHub checks while any tracked pull request is queued or waiting to be queued. From 1 to `reconcileIdleMinutes`. |
| `reconcileIdleMinutes` | `10` | Minutes between GitHub checks otherwise; `0` turns the check off, page loads included. See [The board](board.md#github-reconcile). |
| `openLinks` | `desktop` | `desktop`: thread links open in the running T3 Code desktop app, or in a new tab when it is not running. The app check needs Windows, natively or through WSL, so macOS and Linux without WSL always get a tab. `tab`: always a new tab. |
| `remote` | `null` | Opt-in HTTPS remote board access behind a reverse proxy. `null` or `{"mode": "off"}` keeps the board loopback-only. See [Remote board](#remote-board). |

#### Remote board

`board.remote` is off by default (`null`). To serve the board over Tailscale
Serve, run `t3cm board expose --tailscale` (which sets `tailscale serve` on
HTTPS port `47732` and writes `board.remote` for you) and restart the service,
or edit `config.json` directly:

```json
{
  "board": {
    "remote": {
      "mode": "tailscale",
      "origin": "https://box.acme.ts.net:47732",
      "allowUsers": ["alice@acme.example"],
      "requirePairing": false,
      "settingsUsers": []
    }
  }
}
```

For Caddy, nginx, or `cloudflared`, use `"mode": "pairing"` with your public
HTTPS origin:

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

| Key | Default | Does |
|---|---|---|
| `mode` | `"off"` | `"off"`: refuse all remote and proxied requests (403). `"pairing"`: require a session cookie minted via `t3cm board pair` and redeemed through `/pair#<token>`. `"tailscale"`: verify `Tailscale-User-Login` (injected by Tailscale Serve) against `allowUsers` or the host's own Tailscale login. |
| `origin` | unset | Required when `mode` is not `"off"`. Canonical `https://` origin with no path, query, fragment, or credentials, such as `"https://box.acme.ts.net:47732"` or `"https://board.example.com"`. |
| `allowUsers` | `[]` | Tailscale logins allowed in `"tailscale"` mode (matched case-insensitively). When omitted or empty, defaults to the host's own login from `tailscale status --json`. When non-empty, it replaces that default, so include your own login if you want to keep access. Ignored outside `"tailscale"` mode. |
| `requirePairing` | `false` | When `true` in `"tailscale"` mode, requires both an allowed `Tailscale-User-Login` and a valid `__Host-t3cm_session` pairing cookie minted with `t3cm board pair`. Pairing is always required in `"pairing"` mode. |
| `settingsUsers` | `[]` | Tailscale logins that may open [Settings](board.md#settings) from a remote board in `"tailscale"` mode (matched case-insensitively). When omitted or empty, only the host's own login may. When non-empty, it replaces that default. Each entry must also be in `allowUsers` when `allowUsers` is set. Ignored outside `"tailscale"` mode, where remote Settings stay closed. |

`/mcp`, `/healthz` and `/reload` remain strictly loopback-only regardless of `board.remote`.
See [Remote access](board.md#remote-access) for Tailscale Serve, pairing
commands, and reverse-proxy examples, and [Security model](security.md#network)
for the proxy trust rules.

### Pools

`pools` names groups of T3 provider instances, such as two Claude accounts,
each one its own T3 provider instance. A launch from a pool picks one member,
so work spreads across the accounts instead of filling one until it hits its
usage limit. A pool holds instances of one driver and runs one model, unless
it is [mixed](#mixed-pools).

```json
{
  "pools": {
    "opus": {
      "driver": "claudeAgent",
      "model": "claude-opus-5-5",
      "effort": "high",
      "strategy": "weighted",
      "members": [
        { "instanceId": "claude-a", "weight": 1, "reserve": 30 },
        { "instanceId": "claude-b", "weight": 3 }
      ]
    }
  },
  "board": { "pool": "opus" }
}
```

| Key | Default | Does |
|---|---|---|
| `mixed` | `false` | `true` lets members be instances of any driver, each with its own model and effort. See [Mixed pools](#mixed-pools). |
| `driver` | required | The driver every member must be, as `orchestrator_capabilities` reports it in `driverKind`. A mixed pool has none. |
| `model`, `effort` | required, unset | Model and effort for every launch from the pool, so a thread gets the same model whichever account runs it. A mixed pool has none. |
| `strategy` | `weighted` | `weighted` spreads launches by weight. `priority` fills members in order and moves to the next only when one is out. `cheapest` sends launches to the lowest-cost members that can run, by weight among them. `headroom` needs a usage source, so config load rejects it for now. |
| `moveOnLimit` | `ask` | What happens when a thread the pool launched stops on a usage limit: `ask` puts a move card on the board, `auto` moves the thread to a ready member without one, and `never` leaves the thread where it is. See [Moving a limited thread](#moving-a-limited-thread). |
| `members[].instanceId` | required | A T3 provider instance |
| `members[].weight` | `1` | The member's share of `weighted` launches. With weights 3 and 1, three of every four launches go to the first member. |
| `members[].reserve` | `0` | A share of the account's usage window, 0-100, held for work that names the instance directly |
| `members[].cost` | `1` | What a launch on the member costs, a number of 0 or more. Only its order against the other members counts, and only `cheapest` reads it. |
| `members[].model`, `members[].effort` | required, unset | Only in a mixed pool: the model and effort for launches on this member |

`weighted` is a smooth weighted round robin. Each member's running credit is
in `store.db`, so a run of launches from the same state picks the same
members, and a restart carries on where it stopped. When two members are
level, the one with fewer running threads wins, then the one listed first.

`cheapest` looks at the members a pick has not skipped, takes those with the
lowest `cost`, and spreads launches across them by the same weighted round
robin. It moves to the next cost only when every cheaper member is out. Give a
flat-rate subscription account cost 0 and metered accounts their relative
prices. The pool then fills the subscription until limit marking takes it out,
and the metered accounts take over after that, cheapest first. Cost never overrides a reserve
or a limit. The pick reason names the cost, as in
`cheapest: cost 0, weighted: weight 1 of 2`. Costs come from config only, since
t3cm cannot see provider pricing;
[ADR 0004](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0004-cost-for-pool-picks.md)
records the choice.

A pick skips a member that T3 reports as disabled or constrained, a member
that does not list its model or does not offer its effort, and a member
marked limited until its reset time, as [Limits](#limits) describes. A member
with a `reserve` is a last resort. t3cm has no usage source yet, so it cannot
see how close an account is to its reserve, and the pool picks that member only
when every member without a reserve is out. A launch that names the instance
directly ignores the reserve. When every member is out, the pick fails, nothing
launches, and a board launch leaves an error card that names the pool.

At load, a pool must have at least one member, no member twice, weights of 1
or more, reserves from 0 to 100 and costs of 0 or more, and `board.pool` and
`board.askPool` must name configured pools. A pool that is not mixed must not set `model` or `effort` on a member.
A bad pool fails config load with a message that names it. Only T3 knows
whether each member is a real instance and, in a pool that is not mixed,
whether it is an instance of the pool's `driver`. Every pick checks that
against T3's live `orchestrator_capabilities`. A pick from a pool with an
unknown instance fails, and so does a pick from a pool that is not mixed with
a member of another driver. Either failure names the pool and the member. When
T3 answers at start, the service also logs a bad pool then.

The pick's reason starts with the driver, model and effort it chose, such as
`claudeAgent claude-opus-5-5, effort high; weighted: weight 1 of 4`, and
`wf.pick` returns them as `driver`, `model` and `effort`.

#### Sole members

A pool can depend on one account without saying so: when only one member can
run the pool's model, every launch from the pool waits on that member, and the
pool launches nothing while it is limited. `wf.pools()` flags that member in
the pool's `warnings` and suggests a reserve on it:

```text
only claude-b can run claude-opus-5-5, so the pool launches nothing while claude-b is out. Set a reserve on claude-b to keep room for the work only it can run.
```

With a usage source, the reserve is a floor that keeps part of the account's
usage for launches that name it directly. Until t3cm has one, a reserve only
makes the member a last resort, as the pick rules above say, so in this pool,
where nothing else can run the model, it changes no pick yet.

A member can run the model when T3 lists the model for it and offers the
member's effort. Limits and usage do not count, so the warning does not come
and go as members are benched. A mixed pool's members run different models by
design, so there the check groups members by driver: a member that is the only
one of its driver that can run its model is flagged. Once the member has a reserve, the warning drops the suggestion and
stays, since the pool still has one member for that model. A pool that is not
mixed gets no warning when two members can run its model, and a mixed pool
gets none for a driver with two members that can run their models. A pool
whose `error` is set gets no warning until the error is fixed.

#### Mixed pools

A pool with `"mixed": true` takes members of any driver, and each member names
its own model and, when it wants one, its effort. Use one for work where it
matters less which vendor runs a launch than that the launches spread out,
such as a research swarm on cheap models. A capability tier is a mixed pool
named for its class:

```json
{
  "pools": {
    "frontier": {
      "mixed": true,
      "members": [
        { "instanceId": "claude-a", "model": "claude-opus-5-5", "effort": "high" },
        { "instanceId": "codex-a", "model": "gpt-6-astra", "effort": "xhigh" }
      ]
    },
    "cheap": {
      "mixed": true,
      "strategy": "priority",
      "members": [
        { "instanceId": "claude-a", "model": "claude-haiku-5-5" },
        { "instanceId": "codex-a", "model": "gpt-6-astra-mini", "effort": "low" }
      ]
    }
  }
}
```

A mixed pool has no pool-level `driver`, `model` or `effort`, and config load
rejects one that sets them. Every member needs a `model`. t3cm does not group
models into tiers on its own, so a new model joins a tier when you add it to
the pool. The same instance can be a member of several pools, each with its
own model. Weights and credits are per pool, and a usage limit benches the
instance in every pool it belongs to. With a mixed pool active for a board
launch, passing `model` to `wf.launchTicket` overrides the pool, while passing
`effort` alone fails since each member runs its own model. [ADR 0005](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0005-tiers-are-mixed-pools.md)
records why tiers work this way.

#### Limits

The watcher pass marks a member limited when a thread on it stops on a usage
limit, and picks skip the member until the reset. T3 shows no reason for a
snooze, so a snooze alone never marks a member. A thread counts as stopped on
a limit when its status is `failed` and its latest run ends with T3's
`Usage limit reached` error item. The pass reads each failed run once, and it
looks only at runs on pool members from the last 24 hours.

The reset time comes from T3's limit snooze. With T3's "snooze limited
threads" setting on, T3 snoozes a thread that stops on a limit until its
account resets, and t3cm takes that time when it first sees the stop. With the
setting off, t3cm cannot see the reset, so it holds the member for
`limitHoldMinutes` from when it saw the stop. Turn the setting on in T3 to get
exact reset times.

| Key | Default | Does |
|---|---|---|
| `limitHoldMinutes` | `60` | How long a member stays limited after a limit stop whose reset time T3 does not show, from 1 to 10080 (a week) |

A stop with a later reset extends a member's mark, and one with an earlier
reset leaves it as it is. The
pass clears a mark once its reset passes, and the member is picked again.
Marks live in `store.db`, so they survive a restart. `wf.pools()` shows each
mark's reset time and the thread that set it. [ADR 0001](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0001-usage-signal-for-pools.md)
records why t3cm reads limits this way.

Board ticket launches and `wf.launchTicket` resolve their pool or model from
the `models.ticket` slot first, checking repo `.t3-workflow.json` and
`AGENTS.md` before user `~/.config/t3-workflow/config.json`, and fall back to
legacy `board.pool` only when `models.ticket` is unset. `model` or `effort`
passed to `wf.launchTicket` takes precedence over the slot and legacy board
keys; passing `effort` alone over a mixed pool fails because members run
different models. `board.askPool` covers the ask box, running the picked
member's model in place of `askInstanceId`, `askModel` and `askEffort`. Changes
to ticket or ask model modes made from the board UI persist to their config
files and take effect without a restart. Skills pick from a pool with
[`wf.pick`](globals.md#wf).
The workflow skills do that for each launch when their `model worker` or
`model ticket` slot names a pool, as `"worker": {"pool": "opus"}`; see
[A pool on a model slot](https://github.com/OttoOps/t3-codemode/blob/main/skills/t3-workflow/slots.md#a-pool-on-a-model-slot).
Pools are per T3 environment, since instance ids belong to one T3 server.

#### Moving a limited thread

When a thread that a pool launched stops on a usage limit, the pool's
`moveOnLimit` sets what the watcher pass does:

- `ask`, the default, puts one card on the board: reply `move` to move the
  thread to a ready member of its pool, or `wait` to leave it until the reset.
- `auto` moves the thread at once, with no card, and logs the move to the
  feed. When the move fails, for example because no other member is ready,
  the pass puts up the `ask` card instead. The card says the automatic move
  failed, and its full message gives the reason. When the move goes through
  but the fork cannot be resumed or the old thread settled, the pass puts a
  report card on the fork that says what to finish by hand.
- `never` leaves the thread where it is. The member is still marked limited,
  so new picks skip it until the reset.

Config load rejects any other value. `wf.pools()` shows each pool's
`moveOnLimit`.

In a mixed pool, a move considers only the members of the thread's own
driver. A move forks the thread with its context and worktree, switches the
fork to the new member, moves the thread's workflow links to the fork, resumes
the fork, and settles the old thread. The pick history and the feed record the move. The links, the pick,
the feed entry and the answer are saved together, so if saving them fails,
nothing moves and the card stays open. Answering `move` again then reuses the
same fork. A failed automatic move keeps its fork the same way, and answering
`move` on its card reuses it.

Only a thread with a recorded pick is moved or gets a card: a ticket the
board launched, or a launch reported with [`wf.picked`](globals.md#wf). A
thread launched straight on an instance never is, and neither is a board
question. A thread that runs again before the answer
loses its card, and a move refuses a thread that is running.

### Flow

`flow` turns on the project flow, the board's queue of issues that are ready
to work next. Without a `flow` block the flow is off. For now the flow only
computes the queue, which [`wf.flow()`](globals.md#wfflow) returns, and
nothing launches from it yet.

```json
{
  "flow": {
    "repos": ["acme/widgets", "acme/tools"],
    "optIn": { "label": "flow" },
    "maxTickets": 2,
    "launch": "ask"
  }
}
```

| Key | Default | Does |
|---|---|---|
| `repos` | required | The `owner/name` repositories the flow covers. Two repos can't share a name, because a board ticket's label, such as `widgets#41`, doesn't name the owner. |
| `optIn.label` | required | The label that puts an issue in the flow. Issues without it are never queued. |
| `maxTickets` | required | How many flow tickets can be open on the board at once, at least 1. |
| `launch` | `ask` | `ask`, `auto` or `off`, the way a free slot will launch the next ready issue. Config load checks it, but nothing reads it yet. |

An issue is ready when all of these hold:

- It is open, in a covered repo, and carries the opt-in label.
- No open issue blocks it through GitHub's blocked-by links.
- It is not labelled `blocked: external`.
- No board ticket is open for it.
- It is not a sub-issue whose parent has an open board ticket. That ticket
  works its sub-issues itself.

The queue runs by `priority:` label first, from `priority: high` through
`medium` and `low` to issues with no priority label. Ties go to the earlier
milestone due date, with issues that have no due date last, and then to the
oldest issue.

A flow ticket is an open board ticket, or a launch still waiting to join, for
an issue in a covered repo. Tickets are matched by label, which carries no
owner, so a ticket for `other/widgets#41` counts as `acme/widgets#41` when
`acme/widgets` is covered. A ticket takes one slot however many sub-issues or
workers it runs. The free slots are `maxTickets` minus the flow tickets, and
never fewer than zero.

### Updates

| Key | Default | Does |
|---|---|---|
| `updateCheck` | `true` | Check GitHub for a newer release: the service every 12 hours, the CLI at most once a day. `false` turns off the CLI notice, the board's update button and the service's checks; `t3cm update` still works. See [Update](getting-started.md#update). |

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
| `T3CM_TAILSCALE_BIN` | Override path to the `tailscale` CLI used by `board expose`, `board unexpose`, `doctor`, and Tailscale identity mode (otherwise discovered on `PATH`, in `/Applications/Tailscale.app` on macOS, or under `Program Files\Tailscale\tailscale.exe` on Windows/WSL) |
| `T3CM_T3_ORIGIN` | T3's origin, when `server-runtime.json` has the wrong one |
| `T3CODE_HOME` | T3's home directory (default `~/.t3`) |
| `T3CM_TZ` | Time zone for times the board and `wf` show (default `America/Denver`; an unknown zone falls back to UTC) |
| `T3CM_NO_UPDATE_CHECK` | Any value but `0` or `false` turns the release checks off, like `updateCheck: false`. `CI` does the same. |
| `T3CM_RELEASES_URL` | Where `t3cm update` and the checks look for releases (default `https://github.com/OttoOps/t3-codemode/releases`); https, or plain http on `127.0.0.1` or `localhost` only, and with no user name or password in it. The installers read it too. |
| `RUST_LOG` | Log level |

## State

State lives in SQLite at `store.db` in the state directory above
(`--state-dir` or `T3CM_STATE_DIR` override it). It holds the T3 session, `mem`
and `vars`, the workflow links and messages, the board's cards and feed, hashed
remote pairing tokens and browser sessions, pool member credits, limits and pick
history, and the cached T3 catalog for when T3 is down. Everything survives
restarts. Next to it, `update-check.json` holds the latest release the last
check found and when.

On Linux and macOS the directory is `0700` and the database `0600`. On Windows
t3cm sets no ACL of its own: the store relies on the per-user ACL that
`%LOCALAPPDATA%` already carries, which grants access to you, SYSTEM and
Administrators, so a `--state-dir` outside your profile gets whatever its parent
grants.

Uninstalling (`install.sh --uninstall` or `install.ps1 -Uninstall`) keeps the
store. Delete the state directory to drop memory and links.
