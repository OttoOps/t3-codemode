---
status: accepted
date: 2026-11-21
---

# Keep config edits local and out of execute

## Context and problem statement

Issue #111 adds a board settings dialog over the whole config, and its plan
also proposed `wf.config.get` and `wf.config.set` for `execute` code. Whoever
can write that config can loosen `deny` immediately, add a stdio upstream that
the service runs as a command after a restart, and change who can reach the
remote board after a restart. We had to decide whether a signed-in remote
board and sandboxed code get that write.

## Decision drivers

- A remote sign-in to the board does not grant direct control of what the host
  runs.
- Sandboxed code cannot clear its own `deny` list through a global, because
  the project takes no change that weakens the sandbox.
- The local dialog and CLI can still edit every config key.
- The remote board keeps the Models control it already has.

## Considered options

- Settings on loopback only, with no config global
- Settings for an authenticated remote board
- Config globals for `execute` code

## Decision outcome

Chosen option: settings on loopback only, with no config global.

Every settings route of the board, reads included, answers only a direct
loopback request and refuses an authenticated remote request with 403. The
routes are `GET` and `PUT /api/settings` and `GET /api/settings/pools`, and a
later settings route goes behind the same check. `execute` gets no
`wf.config.get`, no `wf.config.set` and no other config global, so the
optional calls in the issue and its plan are dropped.

The `/api/models` routes stay as they are. An authenticated remote board can
still choose the ticket and ask models, and an ask save applies only the ask
keys.

Both restrictions block direct edits only:

- A remote board can still launch tickets and answer cards. Those reach agents
  that run in `board.runtimeMode`, `full-access` by default, and such an agent
  could edit the config file.
- `execute` can still start or message an agent with `t3_thread_launch`,
  `t3_thread_send` or `schedule_task` unless `deny` lists them, and that agent
  could edit the file or call the loopback routes. An upstream tool that
  writes files or sends HTTP to loopback could do the same.

### Consequences

- Good, because a remote session's only direct write to the config file stays
  the ask model, and a script has none through `wf`.
- Good, because a remote session cannot read the config file through the
  board, which matters when the file holds a credential written as a literal.
- Bad, because changing a setting from a phone or a second machine needs
  another way onto the host.
- Bad, because a script has no general way to read a config value through
  `wf`.
- Bad, because whoever operates t3cm still has to treat remote board access
  and `execute` as able to change the config through an agent. The two limits
  above remain open.
- Bad, because a raw TCP forwarder looks like loopback
  ([ADR 0008](0008-remote-board-trust-boundary.md)), so one in front of the
  board exposes the settings too.

Reversing the first restriction means removing the loopback check from the
settings routes. Reversing the second means adding a global. Either needs a
new ADR, because the config can change what the host runs.

## Pros and cons of the options

### Settings on loopback only, with no config global

- Good, because it follows the check that `POST /api/open` already uses.
- Good, because the CLI and the local board still cover every key.
- Bad, because it leaves the indirect paths through agents and upstream tools.

### Settings for an authenticated remote board

- Good, because the owner could change every key from any signed-in device.
- Bad, because a stolen session could add a command for the host to run, or
  add a login to the remote board.

### Config globals for `execute` code

- Good, because a script could read and change settings without a shell.
- Bad, because a script could clear the `deny` list that limits it.
