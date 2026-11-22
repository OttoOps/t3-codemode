---
status: accepted
date: 2026-11-21
---

# Reload the config in place, with three restart-only keys

## Context and problem statement

The service read `config.json` once at start, so each change needed a restart.
A restart ends runs in flight and drops every upstream connection, which
affects every thread on the machine when someone only wanted a longer board
feed. Issue #111 adds a CLI and a board settings dialog that edit the config,
and both need to know whether a saved change is in effect.

## Decision drivers

- Whoever saves a change learns whether the service runs it.
- An invalid or half-written file never replaces a working config.
- A reload leaves upstream processes, runs in flight and the remote board's
  trust boundary as they were.
- `execute` code gets no new way to change what the service runs.

## Considered options

- Restart the service after each save
- Watch the file and reload on each change
- Reload on request, with a short list of restart-only keys
- Reload every key on request

## Decision outcome

Chosen option: reload on request, with a short list of restart-only keys.

`Wf::reload_config` reads the file with the checks config load runs, then
swaps the whole config at once. A failed read or check keeps the running
config. These keys keep the value the service started with, and each reload
returns the ones whose saved value differs:

| Key | Why it waits for a restart |
|---|---|
| `maxConcurrentRuns` | It sizes the run semaphore and with it the service's memory bound. Raising it also needs new unit memory limits, which need a restart. |
| `upstreams` | It owns child processes, connections, catalog namespaces and the secrets the redactor tracks. Replacing them under a running script would break its calls or stop redacting a secret that the script holds. |
| `board.remote` | It is the trust boundary of [ADR 0008](0008-remote-board-trust-boundary.md). The service admitted open event streams and signed-in browsers under the value at start. |

A new key applies on reload unless it joins this list.

`POST /reload` sits behind the same loopback checks as `/mcp`. The caller
names the file it saved, and a service that reads another file refuses. Code
in the service calls `Wf::reload_config` directly. `execute` has no reload
call.

### Consequences

- Good, because a change to any other key takes effect with no dropped runs or
  connections, and each save names any pending restart.
- Good, because the running config is always valid. The restart-only keys
  validate independently of the rest, so keeping their old values cannot
  invalidate it.
- Bad, because the file and the running config can differ. A hand edit waits
  for the next reload or restart, and a `t3cm stdio` process keeps the config
  it started with.
- Bad, because revoking a login through `board.remote.allowUsers` still needs
  a restart.
- Bad, because work that already read a value finishes with it. An open link
  keeps its numbers and a run keeps its time limits.

Reversing this means removing the route and the reload call and restoring the
restart hint in the CLI. The file format is unchanged.

## Pros and cons of the options

### Restart the service after each save

- Good, because every key behaves the same.
- Bad, because every edit interrupts every thread that uses the service.

### Watch the file and reload on each change

- Good, because hand edits apply too.
- Bad, because the watcher sees editors' partial writes, and nobody is there
  to receive an error or the list of keys that need a restart.

### Reload on request, with a short list of restart-only keys

- Good, because the caller gets the result of its own save.
- Bad, because three keys still need a restart.

### Reload every key on request

- Good, because no key needs a restart.
- Bad, because it needs a resizable run limit, an upstream manager that swaps
  connections under running calls, and revocation of open remote sessions.
