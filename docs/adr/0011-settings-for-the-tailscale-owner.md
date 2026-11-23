---
status: accepted
date: 2026-11-22
---

# Open board settings to the Tailscale owner

Supersedes [ADR 0010](0010-keep-config-edits-local.md) for the board settings
routes only. Its second decision, that `execute` gets no config global, stands.

## Context and problem statement

ADR 0010 made every settings route loopback-only, so the owner of a board
exposed over Tailscale Serve cannot change a setting from a phone or a second
machine at all. Issue #193 asks for that.

The ADR's own limits weaken its case. A remote session can already launch
tickets that run as full-access agents, and any of them could edit the config
file. A raw TCP forwarder in front of the board looks like loopback, so it
exposes the settings with no sign-in at all. The loopback rule keeps the
remote owner out without closing either path.

We had to decide who, beyond loopback, may read and save the whole config,
and whether a remote read hides the credentials the file holds.

## Decision drivers

- The person who owns the host can change its settings from any of their
  devices.
- Any other remote user keeps only the board, its Models control included.
- The identity check is one the board already trusts: a Tailscale login that
  Tailscale Serve sets and the client cannot forge.
- A remote save leaves a record of who changed what.
- Configs written before this change keep working unchanged.

## Considered options

- Keep settings on loopback only
- Settings for every authenticated remote session
- Settings for the Tailscale owner only

## Decision outcome

Chosen option: settings for the Tailscale owner only.

`GET` and `PUT /api/settings` and `GET /api/settings/pools`, and any later
settings route behind the same check, answer a remote request only when all
of these hold:

- `board.remote.mode` is `"tailscale"`.
- The request passed the board's own remote checks, so its
  `Tailscale-User-Login` is an allowed board user, and it carries a pairing
  cookie too when `requirePairing` is set.
- That login is in the new `board.remote.settingsUsers` list or, when the
  list is empty, matches the host's own Tailscale login.
- The request carries `X-Forwarded-Proto: https`. The board's guard lets a
  remote request through without that header. The settings routes do not.
- A `PUT` carries the board's CSRF token, as every mutating board route does.

Every other remote request gets 403. That covers a pairing-mode session, an
allowed board user who is not a settings user, and an owner request with no
`X-Forwarded-Proto: https`. Loopback requests are unchanged.

`settingsUsers` replaces the host-login default when it is not empty, as
`allowUsers` does. Each entry must also appear in `allowUsers` when that list
is set, because a login the board refuses can never reach Settings.
Validation rejects the config otherwise. The key is optional and ignored
outside `"tailscale"` mode, so an existing config keeps its meaning.

The service logs each remote save at info level with the Tailscale login and
the dotted paths of the keys it changed. The log names keys only, never their
values.

The board page probes `GET /api/settings` when it loads on a remote host, and
shows the gear only when that answers 200.

**A remote read does not redact credentials.** `GET /api/settings` returns
the file as written, literal upstream `env` and `headers` values included,
because:

- Whoever may save the file can add a stdio upstream that runs any command on
  the host. Hiding a token from that person protects nothing.
- A save sends the whole file back. A redacted read would write the
  placeholders over the real values, unless the service merged them back,
  which is a second path into the file for little gain.
- `${env:..}` and `${file:..}` references keep a credential out of the file,
  and the read returns the reference unresolved.

### Consequences

- Good, because the owner can change every setting from a phone or a second
  machine on the tailnet.
- Good, because other allowed board users and pairing sessions keep exactly
  the access they had.
- Good, because each remote save is logged with who made it and what it
  changed.
- Bad, because a stolen device signed in to the owner's tailnet account can
  now rewrite the config directly, add a command for the host to run, or add
  a login to the board. Before this change it could only do that through an
  agent.
- Bad, because a literal credential in the file reaches the owner's remote
  device. Moving it to an `${env:..}` or `${file:..}` reference avoids that.
- Bad, because a remote board makes one more request at load to decide
  whether to show the gear.
- Neutral, because the open limits ADR 0010 lists remain. A raw TCP forwarder
  still looks like loopback, and agents and upstream tools can still edit the
  file.

Reverting means restoring the loopback-only check on the settings routes and
dropping `settingsUsers`. A config that sets the key would then fail to load,
so a revert also has to accept and ignore it.

## Pros and cons of the options

### Keep settings on loopback only

- Good, because no remote device can write the config directly.
- Bad, because the owner cannot change a setting without another way onto
  the host, while agents and raw forwarders still can.

### Settings for every authenticated remote session

- Good, because one rule covers both remote modes.
- Bad, because a pairing cookie identifies a browser and says nothing about
  the person using it, so a leaked or forgotten session could rewrite what the
  host runs.
- Bad, because every allowed Tailscale user would get the owner's control.

### Settings for the Tailscale owner only

- Good, because it reuses the identity Tailscale Serve already vouches for.
- Good, because `settingsUsers` lets the owner name a second account without
  handing Settings to every board user.
- Bad, because pairing mode gets no remote Settings at all.
