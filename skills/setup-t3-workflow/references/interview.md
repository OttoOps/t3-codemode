# The interview, slot by slot

Each section says what the slot does, what to offer from `t3cm setup detect`,
what to suggest when nothing is found, and the JSON the answer records. Every
question also offers "keep the current setting" (from `summary`).

## Review

**What it is.** A review loop the worker runs on its branch before it opens
the PR: the reviewer reports findings, the worker fixes them, and the loop
ends when the reviewer agrees. With no reviewer, the worker opens the PR and
says no independent review ran.

**The recommendation.** Use a reviewer that runs on a different AI provider
from the worker model. A model tends to miss the same things in a review
that it missed while writing the code. A reviewer from another provider
brings different blind spots, so the change gets an independent second
opinion before the PR exists, and the user reviews work that has already
been checked once. Name the enabled providers from `providers.enabled` that
differ from the worker model's provider, and ask the user to confirm the
reviewer skill they pick runs on one of them.

**Offer** every full `name` under `slots.review.found`, then "none", then
"another skill" (the user names it).

**Nothing found.** Suggest `slots.review.suggest`: tstack's `sol-loop`. The
user can record `sol-loop` now and install it later (the roles skip it until
then), or choose none.

**Records** `"review": "<full name>"`, or `"review": null` for none.

## Passes

**What it is.** Skills the worker runs, in order, on text that goes out
under the user's name: the PR description, PR and issue comments, and docs.

**Offer** what `slots.passes.found` holds, with what each does:

- a voice skill (any name with `voice` in it): writes in the author's voice;
- `pr` (mattpocock/skills): the PR body format;
- `unslop` (cursor/plugins): tightens prose and code; usually slash-only,
  so it runs inline.

Suggest the ones in `slots.passes.missing` with where they come from. Ask
which to run and in what order; a format skill first and `unslop` last
keeps each pass working on the one before's output. Any skill the user
names can go in the list.

**Records** `"passes": ["<name>", ...]` in run order, or `"passes": []`.

## Credentials

**What it is.** Renewals for credentials that expire mid-ticket. Each pairs
a failure pattern, a Python regular expression tried against the failing
command's output, with the skill that renews the credential. The worker runs
the skill, then retries the command once.

**Offer** each skill in `slots.credentials.found` with a pattern to match,
for example `ExpiredToken|Token has expired|SSO session` for an AWS SSO
login skill. Ask for any other credential the user's work hits, and its
error text.

**Records** `"credentials": [{"match": "<regex>", "skill": "<name>"}, ...]`,
or `[]`.

## Delivery

**What it is.** Ordered stages that deliver approved work once a PR is
ready, plus `closeAfter` for when the ticket closes its GitHub issue.

- `merge` (default): single stage; the orchestrator merges the PR.
- `tag-deploy`: legacy two-stage shorthand; after the merge, the deploy
  skill pushes a release tag and watches the pipeline (`[merge, deploy
  (chain/pr/watch)]`).
- Multi-step pipeline (`steps` + optional `closeAfter`): an ordered list of
  stages, where each stage has:
  - `id`: unique stage name (`merge`, `deploy`, `verify`, `publish`, ...);
  - runner: `kind: "builtin"` for `merge` (at most once, first, `scope:
    "pr"`), `skill: "<name>"` (resolved by `t3cm`), or `kind: "manual"`
    with a short `prompt` for human-only stages;
  - `gate`: `"always"` (always opens a gate), `"chain"` (can be
    pre-approved with a preceding `"always"` stage; consecutive `"chain"`
    stages compose into prefix options), or `"never"` (runs automatically
    after the preceding stage succeeds);
  - `scope`: `"pr"` (default; runs per PR before the worker hands off),
    `"ticket"` (runs once on `main` after the last PR merges), or
    `"post-handoff"` (final report card and issue note outside the ticket);
    scopes must be non-decreasing (`pr` then `ticket` then `post-handoff`);
  - optional `paths`: list of repo-relative glob patterns (`*`, `**`, `?`,
    `[...]`, `{a,b}`) so `wf.slot("delivery", { files })` drops stages that
    match no changed file;
  - optional `watch`: `true` when the stage hands off to a scheduled
    pipeline watch;
  - `closeAfter`: `"pr"`, `"ticket"` (default), or `"all"` (whether the
    issue closes after the last PR's `pr` stages, after `ticket` stages, or
    only after `post-handoff` stages too).

**Offer** `merge`, `tag-deploy` with each skill in `slots.delivery.found`,
and a multi-step pipeline (ask for each stage's `id`, runner, `gate`,
`scope`, optional `paths`, optional `watch`, and `closeAfter`). Delivery
usually differs by repository: when it does, leave the user config on
`merge` and write the repository's multi-step pipeline to its repo-root
`.t3-workflow.json` using `t3cm setup write --repo-config` (step 3; do not
pass `--asked` with `--repo-config`). Existing `Delivery:` and `Deploy
skill:` bullets in `AGENTS.md` still work for simple `tag-deploy`, and
`.t3-workflow.json` wins over `AGENTS.md` when present
([slots.md](../../t3-workflow/slots.md#where-settings-live)).

**Records**

- Single merge: `"delivery": {"mode": "merge"}`
- Legacy tag-deploy: `"delivery": {"mode": "tag-deploy", "skill": "<name>"}`
- Multi-step pipeline (usually written to `.t3-workflow.json` via
  `t3cm setup write --repo-config`):

```json
{
  "delivery": {
    "steps": [
      {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
      {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "ticket", "watch": true},
      {"id": "publish", "skill": "widgets-publish", "gate": "always", "scope": "post-handoff", "paths": ["content/**"]}
    ],
    "closeAfter": "ticket"
  }
}
```

## Models

**What it is.** The model and effort for ticket threads (they split the
issue, launch workers, merge) and for worker threads (they build each PR),
set separately. The default is `claude-opus-5-5` at `high` effort when T3
offers Claude, else the provider's default.

**Offer** the models in `providers.enabled`, grouped by provider, with their
effort values, and each t3cm pool from step 1 as `pool <name>` with its model,
effort and members. A pool spreads launches across the accounts in it, so the
roles pick a member for each launch with t3cm's `wf.pick`. Show a pool's
`error` beside it when it has one. Ask about the ticket model, then the
worker model. The effort must be one of the values listed for that model; a
model with none listed records no `effort`. When the worker's provider is the one the user named for
the reviewer, ask whether to change the reviewer or keep the pair.

**T3 did not answer** (`providers.error`): ask for the provider id, the model
id and the effort, and record what the user gives, leaving out `effort` when
they give none. Say the summary from `t3cm setup detect` shows them as
unverified until T3 is reachable.

**A pool chosen:** record `pool` alone; the pool's model and effort come from
the t3cm config. For the ticket model, also tell the user that tickets the
board launches use the t3cm config's `board.pool`, not this slot.

**Records**
`"models": {"ticket": {"provider": "<id>", "model": "<id>", "effort": "<value>"}}`
(`effort` only when the model lists effort values, or the user gave one
while T3 did not answer), or `"models": {"ticket": {"pool": "<name>"}}` for a
pool; `worker` the same way.

## Flow

**What it is.** Project flow controls which GitHub repositories the board's
dependency graph and Up next queue cover, which open issues enter the queue,
how many flow tickets run at once, how ready issues launch, and whether
file-conflict edges serialize launches. This step also sets the ticket cap,
`maxTicketsTotal`: the most tickets open on the board at once, from every
launch, flow or not. Both live in t3cm's own `config.json`, the flow keys
under `flow` and the cap as a top-level key beside it, not in the workflow
slot config, so this section writes them through
`t3cm config` (which validates and reloads the running service with no
restart) and step 3 records only `--asked flow`. Reference:
[Flow config](https://ottoops.com/t3-codemode/config.html#flow),
[The ticket cap](https://ottoops.com/t3-codemode/config.html#the-ticket-cap) and
[The flow](https://ottoops.com/t3-codemode/board.html#the-flow).

**The current setting.** Run `t3cm config get flow` to display the effective
values (`null` when no `flow` block is set), and `t3cm config get
maxTicketsTotal` for the cap (`8` when the file does not set it). Pass `--port <port>` to
`t3cm config` commands when the service runs on a non-default port
(`T3CM_PORT`). Effective values materialize defaults such as `launch`, so use
them for display only; keep omitted keys omitted when writing changes below.

Ask one question at a time in this order, showing the current value on each
key and waiting for each answer. When the user asked to change one key (such
as `maxTickets`, `maxTicketsTotal` or `excludeRepos`), ask about that key
alone. When `steps.missing` names `board:ticket-cap` but not `board:flow`,
the flow questions were answered under an earlier release: ask about the
ticket cap (question 3) alone, or about the cap and the one key the user
asked to change. Never pass `--asked flow` while `board:ticket-cap` is
missing and the cap question went unasked.

1. **Launch mode (`flow.launch`).** Offer:
   - **off / graph only** (default when `flow` is unset): the Graph tab
     auto-discovers active T3 projects' GitHub repos, and nothing queues or
     launches on its own. If the user wants no repo overrides either, ask
     only the ticket cap (question 3) of the remaining questions, and unset
     `flow` (or keep it unset).
   - **ask** (recommended when enabling the queue): raise a launch card on the
     board (`launch` / `skip`) when a ticket slot opens.
   - **auto**: launch the next ready issue automatically when a slot opens.
   - **queue only (`off` with a cap)**: show the Up next queue on the board
     without raising launch cards or launching automatically.
2. **Max open tickets (`flow.maxTickets`).** Ask when the user chose `ask`,
   `auto`, or queue-only `off`. Default: `2` (must be at least `1` and at most
   the ticket cap; required when `launch` is `ask` or `auto`). Omit for
   graph-only repository overrides.
3. **Ticket cap (`maxTicketsTotal`).** Always ask, with or without a flow
   queue. Default: `8` (must be at least `1`). Every open board ticket and
   every launch still waiting to join counts against it, whatever started
   the launch, and a launch at the cap is refused. `flow.maxTickets` must not
   exceed it: when the user wants more open flow tickets than the cap, raise
   the cap too.
4. **Repository discovery (`flow.repos`).** Default: **auto-discover from
   active T3 projects** (omit `repos`). Offer an explicit list of `owner/name`
   repositories to replace auto-discovery (at least one entry; no two entries
   may share a short repository name).
5. **Include repositories (`flow.includeRepos`).** Default: none (`[]`). Extra
   `owner/name` repositories added on top of discovered or explicit repos.
6. **Exclude repositories (`flow.excludeRepos`).** Default: none (`[]`).
   `owner/name` repositories removed from the covered list; takes precedence
   over discovery, `repos`, and `includeRepos`.
7. **Opt-in label (`flow.optIn.label`).** Ask when `maxTickets` is set (or the
   user asks to change opt-in). Default: none (every open issue in covered
   repos enters the queue). When set, only open issues with this GitHub label
   enter `queue`, `blocked`, or `skipped`; other open issues still appear in
   the dependency graph.
8. **Opt-in milestone (`flow.optIn.milestone`).** Ask when `maxTickets` is set
   (or the user asks to change opt-in). Default: none (any milestone). Matched
   ignoring case; when both `optIn.label` and `optIn.milestone` are set, an
   issue must match both.
9. **Serialize file conflicts (`flow.serializeFileConflicts`).** Ask when
   `maxTickets` is set (or the user asks to change it). Default: `false`
   (`fileConflict` edges stay advisory). When `true`, holds an issue out of
   `queue` while a conflicting ticket is running and avoids launching
   conflicting ready issues at the same time.

**Apply.** Keep omitted keys omitted rather than copying materialized defaults
from `t3cm config get flow`:

- **The ticket cap** is never part of the `flow` JSON. Write it with
  `t3cm config set maxTicketsTotal <n>`, or `t3cm config unset
  maxTicketsTotal` to return to the default `8`. A write that leaves
  `flow.maxTickets` above the cap fails with `is unchanged`, so order the
  commands: when raising both, set `maxTicketsTotal` first; when lowering
  both, set `flow.maxTickets` first.

- **Full interview (all keys answered) or creating `flow` from `null`:**
  assemble one JSON object containing only the keys the user chose (omit
  `repos` for auto-discovery, omit empty `includeRepos`/`excludeRepos`, omit
  `optIn` when both `label` and `milestone` are unset, omit `maxTickets` for
  graph-only overrides, omit `launch` when leaving it at its default, omit
  `serializeFileConflicts` when `false`) and write it in one command:

  ```bash
  t3cm config set flow '<json>'
  ```

- **Updating an existing `flow` block without replacing unrelated keys:** use
  per-key `t3cm config set flow.<key> <value>` and `t3cm config unset
  flow.<key>` in a valid order so stored keys stay untouched and omitted keys
  stay omitted:
  - Enable the queue: `t3cm config set flow.maxTickets <n>`, then check
    `t3cm config get flow.launch` and run `t3cm config set flow.launch
    <ask|auto|off>` whenever it differs from the chosen mode (or `t3cm config
    unset flow.launch` for the default `ask`).
  - Turn the queue off while keeping repo overrides:
    `t3cm config set flow.launch off`, then `t3cm config unset flow.maxTickets`,
    then `t3cm config unset flow.launch`.
  - Clear one opt-in filter when the other stays set:
    `t3cm config unset flow.optIn.label` (or `flow.optIn.milestone`).
  - Clear the last or only opt-in filter: `t3cm config unset flow.optIn` (an
    empty `optIn: {}` fails validation).
  - Return to auto-discovered repos: `t3cm config unset flow.repos`.
- **Turn all of flow back to defaults** (no queue and no repo overrides):

  ```bash
  t3cm config unset flow
  ```

Check the command output:

- **Exit 0 with `Reloaded the running service`:** the change is saved and live
  with no restart. Run `t3cm config get flow` and `t3cm config get
  maxTicketsTotal` to confirm the effective settings and show them to the
  user.
- **Exit 0 with `warn: not reloaded`:** the file is saved on disk, not live in
  a running service. Tell the user the change is saved but not yet live, give
  them the printed restart command (or config-path/port warning), and confirm
  the saved values with `t3cm config get flow`.
- **Non-zero exit with `kept the config it was running`:** the file was saved
  on disk, but the running service rejected the reload and kept its previous
  config. Show the user that error, clarify that the change is saved on disk
  but not live, and fix the issue before treating it as live.
- **Non-zero exit with `is unchanged`:** nothing was written. Fix the reported
  field error and run the command again.

**Records** nothing in the workflow config: the step 3 write passes
`--asked flow`.

## Remote board

**What it is.** Whether the board opens from another device, such as a
phone, over HTTPS. It is set up here but is not a workflow slot: it lives in
t3cm's own `config.json` under `board.remote`, which the commands below
write. The book is the reference for every detail this section leaves out:
[Remote access](https://ottoops.com/t3-codemode/board.html#remote-access)
and [the `board.remote` keys](https://ottoops.com/t3-codemode/config.html#remote-board).

**The current setting** is the `remote` object that `t3cm status` prints:
`mode`, `enabled`, `origin`. When the service is not running,
`t3cm config get board.remote` shows what it will start with. When the
service runs on a port other than the default (`T3CM_PORT`), pass the same
`--port <port>` to every `t3cm status`, `t3cm board expose` and
`t3cm board unexpose` below.

**Offer** three settings, with who each one suits:

- **off** (the default): the board answers on this machine only.
- **Tailscale Serve**: every device that opens the board is on the user's
  tailnet. Tailscale issues the certificate and names each caller, so no
  browser needs pairing.
- **a reverse proxy** (Caddy, nginx, or Cloudflare Tunnel with
  `cloudflared`): a device that opens the board is off the tailnet. Each
  browser is paired.

**Records** nothing in the workflow config: the write is `{}` with
`--asked remote`. The setup is done when its check passes.

### Tailscale Serve

1. Ask who gets in. The default is the host's own Tailscale login. A list
   of logins replaces that default, one `--allow-user <login>` each, so the
   list includes the user's own login.
2. Ask whether to also require pairing. Explain that it is not needed here:
   Tailscale Serve authenticates each request and adds its
   `Tailscale-User-Login` header, and the board admits each allowed login.
   `--require-pairing` adds a per-browser pairing cookie on top of that.
   Suggest no.
3. Run the expose:

   ```bash
   t3cm board expose --tailscale --json [--allow-user <login>]... [--require-pairing]
   ```

   It checks that Tailscale is up and that MagicDNS and HTTPS certificates
   are on. When one is missing it stops and names the fix; show the user,
   and run the same command again once they have made it.
4. **Sudo** (Linux, and WSL with a Linux `tailscale`): when the expose
   fails, read its error in this order.
   - **Root needed to serve a path or Unix socket:** another Serve handler
     on this machine makes every Serve change root-only, `unexpose`
     included, and the operator setting does not help. When the error names
     a root `tailscale serve` command, show the user what it does, run it
     the sudo way below once they agree, then run the same expose again.
     When it names none, show the user the error and
     `tailscale serve status`, and let them decide.
   - **Access denied, naming `sudo tailscale set --operator=$USER`:** make
     the user the operator the sudo way below, then run the same expose
     again.

   The sudo way: run the command with `sudo -n` first, which succeeds where
   sudo needs no password. When sudo wants a password, call
   `request_secret` with `delivery: "file"` for the user's sudo password,
   then run `sudo -S -p '' <command> < "$file"`, for example
   `sudo -S -p '' tailscale set --operator="$USER" < "$file"`. With no
   `request_secret` tool, give the user the command to run in their own
   terminal, and go on when they say it ran.

   When the expose fails again, show the user its error and whether
   `t3cm status` still shows the earlier remote access as enabled, leave
   `board.remote` as it is, and let them decide.

   Under WSL with no Linux `tailscale`, t3cm drives Windows'
   `tailscale.exe`, and the operator step does not apply.
5. Restart the service with the expose output's `restartHint`, because
   `board.remote` is restart-only. This session's t3cm calls fail until the
   service is back; `t3cm status` answers once it is.
6. With `--require-pairing`, pair each browser ([Pair](#pair)). Then
   [check](#check).

To change the logins or the pairing choice later, run the expose again with
the new flags and restart. Pass `--https-port` with the port in
`remote.origin` when it is not `47732`, or the expose adds a second route.
The expose rewrites `board.remote` whole, which drops `settingsUsers`.
Before it runs, read `t3cm config get board.remote`. When `settingsUsers`
is a non-empty list, the list to keep is that list without each login the
new allowlist leaves out (with no `--allow-user`, it admits the host login
alone; compare ignoring case). When that drops a login or leaves the list
empty, which gives Settings to the host login, tell the user and wait for
their go-ahead before the expose. After the expose, and before the restart,
set a non-empty list to keep with
`t3cm config set board.remote.settingsUsers '<list>'`.

### A reverse proxy

1. Ask which proxy, and the board's public HTTPS origin with no path, such
   as `https://board.example.com`. Point the user at that proxy's example in
   [Reverse-proxy examples](https://ottoops.com/t3-codemode/board.html#reverse-proxy-examples):
   the board's protection depends on the proxy following the rules there,
   and the user sets the proxy up. When `remote.mode` is `tailscale` now,
   [turn it off](#turn-it-off) first, so its Serve route goes too.
2. Write the setting as one value, which sets `origin` and `mode` together:

   ```bash
   t3cm config set board.remote '{"mode": "pairing", "origin": "<origin>"}'
   ```

3. Restart the service with the command `t3cm config set` prints.
4. [Pair](#pair) each browser, then [check](#check).

### Pair

Run `t3cm board pair --label "<device>"`, once per browser. It prints a
single-use link and a QR code that work for 5 minutes. Show the user both,
whole, in a code block so the QR code keeps its shape (`send_to_user` when
the session has it), and have them scan the code or open the link on the
device. The link lets one browser in, so it goes to the user alone: keep it
out of PRs, issues, commits and workflow messages.

### Check

1. `t3cm status`: `remote.enabled` is `true`, `remote.mode` and
   `requirePairing` are what the user chose, and `remote.origin` is the URL
   to open. `enabled: false` means the service has not restarted onto
   the new config; restart it. In Tailscale mode, `allowUsersSource:
   "unavailable"` means the service refuses every caller; show the user its
   `tailscaleError`.
2. Request the origin over HTTPS and read the page title:

   ```bash
   curl -sS -H 'Accept: text/html' "<origin>/" | grep -o '<title>[^<]*</title>'
   ```

   The board answers with a title that ends in `t3 board`:
   - `t3 board`: the board let this caller in. Expected in Tailscale mode
     without pairing.
   - `Sign in to t3 board - t3 board`: the board wants a pairing session.
     Expected behind a proxy and with `requirePairing`; anywhere else it
     means the config is not the one the user chose.
   - a title that starts with `Access denied` or
     `Tailscale sign-in required`: the board refused
     this machine's Tailscale identity, or got none. Show the user the
     title; the other device's check below settles it.

   A certificate or connection error, or no `t3 board` title, means the
   route or proxy does not reach the board; `t3cm doctor` names what
   disagrees. When this machine cannot resolve the origin's name (WSL with
   Tailscale on Windows may not), go to the next step.
3. Give the user the origin to open on the other device, and have them say
   what it shows: the board, or the pairing page until they pair.

### Turn it off

- **Tailscale mode:** run `t3cm board unexpose --json`, with the same
  `--port` the expose used. With a different port, it misses the board's
  Serve route, and can turn the config off while the route stays up.
- **Pairing mode:** run `t3cm config set board.remote '{"mode": "off"}'`,
  then offer `t3cm board revoke --all` to end the paired sessions.

Restart the service with the printed restart command, then check that
`t3cm status` shows `remote.enabled: false`.
