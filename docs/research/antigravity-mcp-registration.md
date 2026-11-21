# Antigravity MCP registration for t3cm

Research for [#151](https://github.com/OttoOps/t3-codemode/issues/151). It tests whether Antigravity threads inside T3 Code can carry their own `T3CODE_THREAD_ID` on `t3cm` MCP calls, selects the transport and configuration file, and runs the two-thread gate against a shared temporary store.

## Decision

- **Gate result.** The gate passed. Two concurrent Antigravity threads on version `1.5.13` sharing one temporary `--gemini-dir` profile each spawned their own `t3cm stdio` child on version `0.6.0` against a shared temporary `store.db` alongside a running test `t3cm serve`. Every `execute` call arrived under its own thread id, and the test service read both threads' writes immediately over HTTP.
- **Chosen route.** Register `t3cm stdio` in `mcp_config.json`, with `search` and `execute` marked `"eager": true`. Antigravity `1.5.13` does not expand environment variables in HTTP `headers`, so braced forms such as `${T3CODE_THREAD_ID}` leave `caller` unset and the bare `$T3CODE_THREAD_ID` form assigns every thread the same literal caller `"$T3CODE_THREAD_ID"`.
- **Target configuration file.** Write the entry to T3's per-instance `~/.t3/userdata/providers/antigravity/<hash>/config/mcp_config.json`, where `<hash>` is the lowercase hex SHA-256 of T3's `providerInstanceId`. T3 starts Antigravity with `--gemini-dir <profile>` and `GEMINI_HOME=<profile>`, so Antigravity ignores `~/.gemini/config/mcp_config.json`. T3 keeps the per-instance `mcp_config.json` across restarts.
- **Config folder reporting.** HTTP header expansion cannot carry `X-T3-Config-Dir` with `${GEMINI_HOME}`, because Antigravity sends `${GEMINI_HOME}` verbatim. Over `t3cm stdio`, Antigravity passes `GEMINI_HOME` into the child environment automatically, so no header is needed. Current `t3cm` reads only `CLAUDE_CONFIG_DIR` and `CODEX_HOME` in `Reported::from_env()` at `src/wf_slots.rs:44-47`, so PR 2 will need to wire `GEMINI_HOME` if Antigravity slots should report their config directory.

## Environment and isolation

All probes ran on Linux against Antigravity `1.5.13` and `t3cm 0.6.0` on 2026-11-21. Every command used temporary `--gemini-dir` folders, a temporary `T3CM_STATE_DIR`, an ephemeral loopback port for the test `t3cm serve`, and a local fake T3 MCP server via `T3CM_T3_ORIGIN` with a pre-seeded dummy row in the `token` table. Neither `~/.gemini/config/mcp_config.json`, T3's live `~/.t3/userdata/providers/antigravity/<hash>/` directory, the live T3 server, nor the live `t3cm` service on port 47731 were modified or restarted.

## Question 1. HTTP header expansion

Antigravity `1.5.13` does not expand environment variables in `mcp_config.json` HTTP `headers`. Every tested syntax arrived verbatim on the wire.

### Test setup

I wrote a temporary `<tmp>/config/mcp_config.json` pointing at a local HTTP capture server on `127.0.0.1:<port>/mcp` with five candidate syntaxes plus a literal control header:

```json
{
  "mcpServers": {
    "cap_http": {
      "url": "http://127.0.0.1:<port>/mcp",
      "headers": {
        "X-Test-Braces": "${T3CODE_THREAD_ID}",
        "X-Test-Default": "${T3CODE_THREAD_ID:-fallback}",
        "X-Test-Bare": "$T3CODE_THREAD_ID",
        "X-Test-EnvColon": "${env:T3CODE_THREAD_ID}",
        "X-Test-ConfigDir": "${GEMINI_HOME}",
        "X-Test-Literal": "literal-ok"
      }
    }
  }
}
```

I launched `antigravity acp` with the same flags and environment T3 uses:

```sh
GEMINI_HOME="<tmp>" \
T3CODE_THREAD_ID="probe-thread-http-001" \
AGY_ACP_FORCE_FILE_STORAGE=1 \
AGY_CLI_DISABLE_AUTO_UPDATE=1 \
SSH_CONNECTION="127.0.0.1 0 127.0.0.1 0" \
antigravity acp --gemini-dir "<tmp>" --app-data-dir antigravity-acp
```

Sending ACP `initialize`, `authenticate` with `oauth-personal`, and `session/new` with an empty `mcpServers` list caused Antigravity to connect to `cap_http` and issue `server/discover`, `initialize`, `notifications/initialized`, `tools/list`, and `notifications/roots/list_changed`.

### Captured HTTP headers

Every one of those five POST requests carried the same unexpanded header values:

```json
{
  "X-Test-Bare": "$T3CODE_THREAD_ID",
  "X-Test-Braces": "${T3CODE_THREAD_ID}",
  "X-Test-Configdir": "${GEMINI_HOME}",
  "X-Test-Default": "${T3CODE_THREAD_ID:-fallback}",
  "X-Test-Envcolon": "${env:T3CODE_THREAD_ID}",
  "X-Test-Literal": "literal-ok"
}
```

`t3cm` rejects any `X-T3-Thread` or `X-T3-Config-Dir` header containing `${` at `src/mcp.rs:105-108` and `src/wf_slots.rs:26-29`, and the bare `$T3CODE_THREAD_ID` form would register every thread under the literal string `"$T3CODE_THREAD_ID"`. HTTP registration therefore cannot distinguish Antigravity threads.

## Question 2. Stdio transport and shared store concurrency

Antigravity's stdio transport passes the parent process environment straight into each spawned MCP server, including `T3CODE_THREAD_ID` and `GEMINI_HOME`, and multiple `t3cm stdio` processes share `store.db` alongside `t3cm serve` without SQLite lock errors.

### Stdio environment inheritance

I repeated the isolated `antigravity acp` run with a temporary capture script registered as a stdio MCP server in `<tmp>/config/mcp_config.json`, setting `T3CODE_THREAD_ID=probe-thread-stdio-001` and `GEMINI_HOME=<tmp>` in the parent environment and adding three `env` entries for `${T3CODE_THREAD_ID}`, `$T3CODE_THREAD_ID`, and a literal value inside `mcp_config.json`.

The spawned stdio child recorded these values at startup before answering `server/discover`, `initialize`, `notifications/initialized`, `tools/list`, and `notifications/roots/list_changed`:

```json
{
  "T3CODE_THREAD_ID": "probe-thread-stdio-001",
  "GEMINI_HOME": "<tmp>",
  "PROBE_EXPAND_BRACES": "${T3CODE_THREAD_ID}",
  "PROBE_EXPAND_BARE": "$T3CODE_THREAD_ID",
  "PROBE_LITERAL": "from-config-env"
}
```

Values written inside the `env` object of `mcp_config.json` are sent literally without expansion, while `T3CODE_THREAD_ID` and `GEMINI_HOME` are inherited directly from `antigravity acp`. `t3cm stdio` reads `T3CODE_THREAD_ID` once at startup in `src/main.rs:830-841` and binds it as `caller` for the lifetime of that stdio connection in `src/server.rs:133-168`. The watcher stays off unless `--watcher` is passed, so `t3cm stdio` does not compete with `t3cm serve` over watcher ticks.

### Concurrent SQLite store test

`Store::open` configures SQLite with `busy_timeout` set to 5 seconds, `journal_mode = WAL`, and `synchronous = NORMAL` in `src/store.rs:345-348`. To verify multi-process access alongside a live service, I started one `t3cm serve` on an ephemeral loopback port with `--no-watcher` and four `t3cm stdio` processes named `stress-stdio-0` through `stress-stdio-3`, all pointed at one temporary `store.db`. After synchronizing at a barrier, each `t3cm stdio` process ran `wf.init({ role: "worker" })` followed by 20 `mem.set` and `mem.get` cycles while a fifth worker sent 20 `mem.set` and `mem.get` cycles over HTTP with `X-T3-Thread: stress-http-0` to `t3cm serve`.

| Metric | Observed result |
|---|---|
| Processes sharing `store.db` | 4 `t3cm stdio` plus 1 `t3cm serve` |
| Total operations | 4 `wf.init`, 100 `mem.set`, 100 `mem.get` |
| Wall time | 0.05 s |
| SQLite `journal_mode` | `wal` |
| Errors or busy failures | 0 |
| `nodes` rows written | `board`, `stress-stdio-0` through `stress-stdio-3` |
| `kv` rows per thread namespace | 20 each across all 5 `thread:<id>` namespaces |

### Operational caveats for `t3cm stdio`

Reading the source turned up four multi-process behaviors that PR 2 and operators should keep in view:

1. **Proactive OAuth token refresh window.** `HttpT3::rpc` handles a `401` by checking `store.token_get()` before acquiring a new token at `src/t3_http.rs:364-374`, which lets processes adopt a token another process already refreshed. `HttpT3::ensure_token` at `src/t3_http.rs:164-180` checks only its in-memory copy before calling `acquire()`, which revokes older `t3-codemode` sessions at `src/t3_http.rs:262`. When several long-lived `t3cm stdio` processes cross the three-day proactive refresh threshold at once, they can trigger competing refreshes and session revocations.
2. **Board live push delay.** `Wf.changes` at `src/wf.rs:235` is an in-process broadcast channel inside `t3cm serve`. Writes from `t3cm stdio` commit to `store.db` immediately, so the web board sees them on its next refresh or watcher pass instead of getting an instant SSE event.
3. **Upstream process fan-out.** Every `t3cm stdio` process creates its own `UpstreamManager` at `src/app.rs:175`. If a user configures stdio upstream MCP servers in `t3cm`, each Antigravity thread starts its own copy of those upstreams.
4. **Eager tool loading.** Antigravity lazy-loads MCP tools by default, writing schemas to `<app-data-dir>/mcp/<server>/` and requiring the model to read them before invoking `call_mcp_tool`. Adding `"tools": { "search": { "eager": true }, "execute": { "eager": true } }` to the `t3cm` entry registers `mcp_t3cm_search` and `mcp_t3cm_execute` upfront on turn one. Because `t3cm` exposes only two tools, eager loading avoids an extra tool turn without bloating the prompt.

## Question 3. Config file location, restart persistence, and `X-T3-Config-Dir`

The registration belongs in T3's per-instance `~/.t3/userdata/providers/antigravity/<hash>/config/mcp_config.json`.

### Why `~/.gemini/config/mcp_config.json` does not load in T3 threads

T3 Code isolates each Antigravity provider instance into its own profile directory. Inspecting T3's bundled Antigravity support module `antigravityAuthSupport.ts` and live process arguments shows the exact layout:

- `resolveAntigravityInstanceDirectories(stateDir, instanceId)` sets the profile folder to `<stateDir>/providers/antigravity/<sha256(instanceId)>`.
- `buildAntigravityAcpSpawnInput` launches `antigravity acp --gemini-dir <profile> --app-data-dir antigravity-acp` with `GEMINI_HOME=<profile>` in the environment.
- `linkAntigravityUserConfig` symlinks only `config/skills`, `antigravity-cli/skills`, `AGENTS.md`, and `GEMINI.md` from `~/.gemini` into `<profile>`. On Windows, `copyAntigravityUserRuleFiles` copies `AGENTS.md` and `GEMINI.md` instead of symlinking them. Neither function links `config/mcp_config.json`.

A directory listing of the live T3 profile confirms this layout:

```text
~/.t3/userdata/providers/antigravity/<hash>/
  GEMINI.md -> ~/.gemini/GEMINI.md
  antigravity-acp/
  config/
    .migrated
    config.json
    mcp_config.json          (0 bytes)
    projects/
    skills -> ~/.gemini/config/skills
```

Two direct observations confirm that `~/.gemini/config/mcp_config.json` is ignored by T3 Antigravity threads:

1. On the test machine, `~/.gemini/config/mcp_config.json` defined two enabled stdio MCP servers, yet neither appeared in a live T3 Antigravity thread. Only the `t3-code` server passed by T3 over ACP `session/new` appeared in `<profile>/antigravity-acp/mcp/`.
2. In the isolated stdio probe, `<tmp>/antigravity-acp/mcp/` contained only `['cap_stdio']` from `<tmp>/config/mcp_config.json` and none of the servers from `~/.gemini/config/mcp_config.json`.

### Persistence across restarts

T3 does not overwrite `<profile>/config/mcp_config.json`. On every session start, `prepareAntigravityProfile` creates the profile directories with mode `0700` on Unix, writes `<profile>/antigravity-acp/settings.json`, and refreshes the skill and rule links listed above. It never writes, truncates, or removes `config/mcp_config.json`.

The 0-byte `mcp_config.json` in the live profile was created once by Antigravity's first-run migration when `config/.migrated` was absent, as logged by `migrate.go:49` in the profile's first ACP server log. In the isolated stdio probe, comparing `<tmp>/config/mcp_config.json` byte-for-byte before and after running `antigravity acp` returned `unchanged: True`.

### Two constraints for PR 2

1. **Edit JSON directly instead of calling `antigravity mcp add`.** Testing `GEMINI_HOME="<tmp>" antigravity mcp add ...` showed that the `antigravity mcp` subcommand ignores `GEMINI_HOME` and always reads and writes `$HOME/.gemini/config/mcp_config.json`. PR 2's installer and setup step must update `<profile>/config/mcp_config.json` directly as JSON, and should handle every Antigravity profile directory present under `~/.t3/userdata/providers/antigravity/` if the user configured more than one Antigravity instance.
2. **T3 system text-generation model guard.** In T3's `AntigravityTextGeneration.ts`, `isAntigravityTextGenerationAvailable(profileDirectory)` inspects `<profile>/config/hooks.json` and `<profile>/config/mcp_config.json`. Whenever either file contains a non-empty `hooks` or `mcpServers` object, T3 refuses to use that Antigravity profile as its system text-generation model, which handles helper tasks such as title generation, and returns:
   `Antigravity text generation is unavailable for profiles with global hooks or MCP configuration. Select another system model.`
   Normal Antigravity agent threads continue to run without issue, and T3's system model continues to work when set to another provider. PR 2's docs should call this out for users who selected Antigravity as their T3 system model.

### Passing `X-T3-Config-Dir`

Because HTTP header expansion does not run in Antigravity, `X-T3-Config-Dir: ${GEMINI_HOME}` cannot be sent over HTTP. Over `t3cm stdio`, `GEMINI_HOME` is already present in `t3cm`'s process environment with the exact `<profile>` path.

Right now, `Reported::from_env()` at `src/wf_slots.rs:44-47` reads only `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, `Folders` at `src/slots.rs:529-574` stores only `claude` and `codex` directories, and `Wf::slot_ctx` at `src/wf_slots.rs:183-240` picks the Codex folder for `codex` and falls back to the Claude folder for every other driver, including `antigravity`, with no Antigravity-specific folder mapping. If PR 2 wants Antigravity threads to record their instance config folder in `instance_folders` and resolve skills from `<profile>/config/skills`, which symlinks to `~/.gemini/config/skills`, `t3cm stdio` can read `GEMINI_HOME` directly from the environment without any HTTP header.

## Two-thread gate run

### Evidence that T3 sets distinct `T3CODE_THREAD_ID` values on one shared profile

To confirm how T3 launches multiple Antigravity threads on the same provider instance without editing live T3 settings or creating a second OAuth profile, I inspected both T3's bundled Antigravity driver and the live `antigravity acp` processes running under T3 on the test machine:

- In T3's bundled runtime, `acpMcpActivation` sets `processEnvironment: { T3CODE_THREAD_ID: session.threadId, ... }`, and `AntigravityDriver.makeRuntime` merges that per-thread environment into `prepareSpawn` and `buildAntigravityAcpSpawnInput`. `antigravityEnvironment` preserves `T3CODE_THREAD_ID`, which is absent from `removedEnvironmentKeys`, and adds `GEMINI_HOME: profile.geminiHome`.
- Inspecting `/proc/<pid>/environ` across three live concurrent T3-launched `antigravity acp` processes showed all three sharing the same `--gemini-dir ~/.t3/userdata/providers/antigravity/<hash>` and `GEMINI_HOME`, while each process held its own distinct `T3CODE_THREAD_ID` value, with `sort -u | wc -l` returning `3`:

```text
process 1: antigravity acp --gemini-dir ~/.t3/userdata/providers/antigravity/<hash> --app-data-dir antigravity-acp
           GEMINI_HOME=~/.t3/userdata/providers/antigravity/<hash>
           T3CODE_THREAD_ID=<live-thread-1>
process 2: antigravity acp --gemini-dir ~/.t3/userdata/providers/antigravity/<hash> --app-data-dir antigravity-acp
           GEMINI_HOME=~/.t3/userdata/providers/antigravity/<hash>
           T3CODE_THREAD_ID=<live-thread-2>
process 3: antigravity acp --gemini-dir ~/.t3/userdata/providers/antigravity/<hash> --app-data-dir antigravity-acp
           GEMINI_HOME=~/.t3/userdata/providers/antigravity/<hash>
           T3CODE_THREAD_ID=<live-thread-3>
```

Adding a disposable Antigravity provider instance directly inside T3 would require editing live `~/.t3/userdata/settings.json` and completing a separate Google OAuth sign-in, which would violate the non-destructive gate constraint. Instead, the gate run below reproduces T3's exact spawn command and shared per-instance profile directory in `/tmp`.

### Setup and configuration

I ran two Antigravity ACP processes concurrently against one shared temporary `--gemini-dir` profile at `<tmp>/shared-profile` with one `<tmp>/shared-profile/config/mcp_config.json`, matching T3's per-instance layout, and gave each process its own `T3CODE_THREAD_ID` value `<thread-a>` or `<thread-b>`. Both processes shared one temporary `T3CM_STATE_DIR` at `<tmp>/t3cm-state` with a running test `t3cm serve` bound to an ephemeral loopback port with `--no-watcher` and pointed at a local fake T3 MCP server via `T3CM_T3_ORIGIN=http://127.0.0.1:<fake-t3-port>`.

The shared profile used this `config/mcp_config.json` with no thread id in the file:

```json
{
  "mcpServers": {
    "t3cm": {
      "command": "<path-to-t3cm>",
      "args": ["stdio", "--state-dir", "<tmp>/t3cm-state"],
      "env": {
        "T3CM_T3_ORIGIN": "http://127.0.0.1:<fake-t3-port>",
        "T3CM_NO_UPDATE_CHECK": "1",
        "XDG_CONFIG_HOME": "<tmp>/xdg-config"
      },
      "tools": {
        "search": { "eager": true },
        "execute": { "eager": true }
      }
    }
  }
}
```

Both processes completed ACP `initialize`, `authenticate`, and `session/new`, synchronized on a two-thread barrier, and received `session/prompt` simultaneously instructing each thread to call `mcp_t3cm_execute` once with `wf.init({ role: "worker" })`, `mem.set("shared_gate", "<marker>")`, and `vars.marker = "<marker>"`.

### Per-thread observed results

| Thread | `T3CODE_THREAD_ID` env | Child process | Tool call status | Elapsed | `nodes` row in `store.db` | `kv` rows in `store.db` |
|---|---|---|---|---|---|---|
| Thread A | `<thread-a>` | own `t3cm stdio` child | `mcp_t3cm_execute` completed | 15.90 s | `<thread-a>`, `worker`, `2026-11-21T04:31:51Z` | `thread:<thread-a>` `shared_gate="alpha"`, `__vars__={"marker":"alpha"}` |
| Thread B | `<thread-b>` | own `t3cm stdio` child | `mcp_t3cm_execute` completed | 8.26 s | `<thread-b>`, `worker`, `2026-11-21T04:31:50Z` | `thread:<thread-b>` `shared_gate="beta"`, `__vars__={"marker":"beta"}` |

An earlier run using two separate temporary `--gemini-dir` folders at `<tmp>/profile-a` and `<tmp>/profile-b` produced the same isolated per-thread `nodes` and `kv` records in 11.71 s and 10.83 s.

### Concurrent service readback

Immediately after both turns completed, an HTTP `tools/call` for `execute` sent to the concurrently running test `t3cm serve` on `127.0.0.1:<port>/mcp` returned both threads' writes from the shared SQLite store:

```json
{
  "result": {
    "a": "alpha",
    "b": "beta"
  }
}
```

### Verdict

The gate passed. Live T3 process inspection shows T3 gives each concurrent Antigravity thread its own `antigravity acp` process and distinct `T3CODE_THREAD_ID` while sharing one per-instance `--gemini-dir` profile, and the shared-profile gate run shows each concurrent `antigravity acp` process launches its own `t3cm stdio` child, carries its own `T3CODE_THREAD_ID` on `execute`, writes into its own `nodes` row and `thread:<id>` namespace, and shares `store.db` cleanly with a running `t3cm serve` instance. PR 2 on `feat/151-antigravity-mcp` can proceed on the `t3cm stdio` route targeting `~/.t3/userdata/providers/antigravity/<hash>/config/mcp_config.json`.
