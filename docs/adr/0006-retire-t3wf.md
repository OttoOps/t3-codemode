---
status: accepted
date: 2026-11-20
---

# t3cm is the only workflow engine; slots resolve in the service, at the point of use

## Context and problem statement

`t3cm` runs the T3 workflow (`wf.*`, the watcher, the board and GitHub
reconcile), but the skills in `skills/` continued to ship `t3wf.py`,
`slots.py`, `t3api.py`, `defaults.json` and per-role `t3-role.json` files
as a parallel Python implementation of the link protocol and slot resolver.
Threads without `mcp__t3cm__execute` (such as Codex threads, or Claude
sessions started before `t3cm` connected) fell back to `t3wf`, leaving
their links and outbox messages invisible to the board and to the service's
watcher.

Slots compounded the split: even in code mode, every role bulk-read all
five slots through `t3wf.py slots` at start and on every resume, passing
`--skills` to work around `slots.py` ignoring Claude Code's
`enabledPlugins`. That paid tokens for unused slots (especially
`credentials`) on every run and kept Python scripts embedded in the `t3cm`
binary.

Issue #105 records the real-run evidence and the full migration plan across
#115, #116, #117 and #118.

## Decision drivers

- One workflow engine and one store per machine, so no thread silently
  falls back to a file-backed registry the board cannot see.
- No Python runtime or `fcntl` dependency inside shipped skills or the
  embedded binary bundle.
- Slots resolve accurately for each calling provider instance (including
  instances with a custom `CLAUDE_CONFIG_DIR` or `CODEX_HOME`, and
  respecting `enabledPlugins`) without reading T3 Code's private settings
  files.
- Roles pay only for the slot a step actually uses, with credential renewal
  regex matching performed in code rather than by the model.
- Existing `~/.config/t3-workflow/config.json` files and repository
  `AGENTS.md` "Agent workflow" blocks keep working unchanged.
- Updating an existing machine removes `t3wf` scripts and timers cleanly
  while warning and nudging any threads with open links.

## Considered options

- Keep `t3wf.py` as a no-daemon fallback alongside `t3cm`
- Move slot resolution to a `t3cm slots` shell command called at start and
  resume, and retire `t3wf.py`
- Resolve slots inside the `t3cm` service with caller-reported config
  folders, expose point-of-use `wf.slot` and `wf.renewal`, and retire
  `t3wf` completely

## Decision outcome

Chosen: resolve slots inside the `t3cm` service at the point of use and
retire `t3wf` completely.

- **Single engine.** `t3wf.py`, `slots.py`, `t3api.py`, `defaults.json` and
  the three `t3-role.json` files are deleted. The `t3cm` binary embeds no
  Python files. Threads with `mcp__t3cm__execute` call it directly; threads
  without it (including Codex, where all threads share one `codex
  app-server` process without per-thread `T3CODE_THREAD_ID`, per #118)
  pipe the same `async () => ...` function to `t3cm exec -`. When the
  service is down, roles surface the start-the-service action line and stop
  rather than falling back.
- **Caller-reported config folders.** Claude Code's MCP registration sends
  `X-T3-Config-Dir: ${CLAUDE_CONFIG_DIR:-}`, and `t3cm exec` sends its
  `CLAUDE_CONFIG_DIR` and `CODEX_HOME` environment. The service records
  `instance -> folder` in its store for the board and reads
  `enabledPlugins` from the agent's `settings.json`, removing the need for
  `--skills` on the normal path.
- **Point-of-use slot lookups.** Roles drop the bulk slot read at start and
  resume and call `wf.slot("delivery")`, `wf.slot("models")`,
  `wf.slot("review")`, `wf.slot("passes")` and `wf.renewal({ output })`
  only at the step that needs them. `t3cm slots`, `t3cm doctor` and
  `t3cm setup detect|write|rule` share the same Rust resolver.
- **Upgrade cleanup (`retire:t3wf`).** `t3cm skills update` warns (without
  blocking) when `t3wf` `v2/links.json` or the `t3cm` store has open links,
  syncs the skills (removing the dropped Python files), disables and
  removes `t3wf-tick.*` and `cbrip-t3wf-tick.*` systemd units, removes the
  `v2/bin/t3wf-tick` shim and `scripts/__pycache__`, and nudges each open
  thread to re-invoke `t3-workflow`. Registry data (`v2/links.json`) is
  left untouched until `t3cm doctor` reports it is safe to delete.

### Consequences

- Good: every workflow thread, Claude or Codex, shares one store, one
  watcher and the web board.
- Good: zero Python files under `skills/` or embedded in `t3cm`; the
  `fcntl`/WSL restriction on the skills is gone.
- Good: unused slots (such as `credentials` when no command fails) cost no
  tokens, and `wf.renewal` matches patterns in Rust.
- Good: the board and `wf.launchTicket` can share the exact same slot
  resolution as role threads (#106, #101).
- Bad: the workflow skills now require a running `t3cm` service; there is
  no standalone script-only mode.
- Neutral: Codex continues to call `t3cm exec -` from a shell until
  upstream T3 Code exposes per-thread identity to Codex MCP servers (#118).

## Pros and cons of the options

### Keep `t3wf.py` as a fallback

- Good: works without running a background service.
- Bad: maintains two implementations of the protocol and slot resolver,
  silently splits active threads off the board, and keeps `fcntl` and
  Python dependencies in shipped skills.

### Retire `t3wf.py`, keep bulk `t3cm slots` reads

- Good: removes Python and unifies the engine.
- Bad: still pays tokens for all five slots at start and on every resume,
  leaves regex matching to the model, and still requires shell calls from
  MCP-connected threads.

### Retire `t3wf.py` and resolve slots in the service at point of use

- Good: removes Python, eliminates bulk slot reads and `--skills`, keeps
  all calls on `wf.*`, and gives the board the same resolver.
- Bad: requires callers to report their config folder (handled via
  `X-T3-Config-Dir` and `t3cm exec`) and requires a one-time upgrade step
  (`retire:t3wf`) for existing machines.

## More information

Issue #105 and its sub-issues #115 (service slot resolver, PR #136 and
#143), #116 (`t3cm setup`, PR #149), #117 (retire `t3wf`), and #118 (Codex
MCP gate). Implementation lives in `src/slots.rs`, `src/wf_slots.rs`,
`src/setup.rs`, `src/doctor.rs` and `src/steps.rs`.
