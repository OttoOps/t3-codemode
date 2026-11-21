---
status: accepted
date: 2026-11-21
---

# Split the board trust boundary from loopback /mcp

## Context and problem statement

`t3cm` binds `127.0.0.1:<port>` and previously wrapped every HTTP route behind
one loopback-only `Host` middleware. Opening the web board from a phone,
laptop, or second machine required an out-of-band proxy that rewrote `Host` and
`Origin` over plain HTTP on a raw network IP. That created four problems:

- Browsers treated plain HTTP on a non-loopback IP as an insecure context,
  blocking browser notifications, clipboard access, and home-screen install.
- Rewriting `Host` and `Origin` bypassed `t3cm`'s own boundary and exposed
  board mutations (answering gates, launching tickets, merging pull requests)
  and `/mcp` to anyone able to reach the proxy.
- Because reverse proxies connect to `127.0.0.1:<port>` over loopback, a
  server that trusts loopback peer addresses cannot tell a proxied remote
  request from a direct local browser without inspecting `Host` and proxy
  headers.
- Local-only UI assumptions leaked remotely, such as `/api/open` attempting to
  open the host's desktop app instead of opening the T3 thread URL on the
  client device.

Issue #164 records the problem, design, and settled decisions across a four-PR
rollout.

## Decision drivers

- Keep `t3cm` off the network by default. Remote board access must be opt-in
  and disabled out of the box.
- Keep `POST /mcp` (Code Mode `search` and `execute`) and `GET /healthz`
  strictly loopback-only, never reachable through a reverse proxy or from a
  remote origin.
- Keep direct loopback access to the board zero-auth and identical to today,
  while ensuring no proxied or remote request can slip into the local
  zero-auth path.
- Avoid raw non-loopback binds and custom TLS certificate management inside
  `t3cm`; rely on Tailscale Serve or a user-managed HTTPS reverse proxy.
- Support both frictionless single-owner access over Tailscale Serve and
  pairing-cookie authentication for generic reverse proxies and opt-in
  per-browser pairing.
- Protect every board read route as well as every write route, so ticket
  titles, pull requests, and message bodies never leak unauthenticated.

## Considered options

- Expose the whole `t3cm` HTTP server behind a reverse proxy with one shared
  middleware
- Bind `t3cm` directly on a non-loopback interface and manage TLS certificates
  in-process
- Split `/mcp` and `/healthz` from board routes into separate middleware
  layers, classify every request as local or remote from `Host` and proxy
  headers, and authenticate remote board traffic behind an opt-in HTTPS origin

## Decision outcome

Chosen: split the HTTP router into a strict loopback boundary for `/mcp` and
`/healthz` and a separate board boundary for `/` and `/api/*`, backed by an
opt-in `board.remote` configuration.

- **Single-board v1 scope.** V1 covers multi-device HTTPS access to one host's
  board from phones, laptops, and desktops. Multi-host board aggregation is out
  of scope while keeping URLs, headers, and API shapes clean for later work.
- **Loopback bind with external TLS.** `t3cm` continues to bind
  `127.0.0.1:<port>` only. First-class Tailscale Serve support terminates TLS
  on `https://<host>.<tailnet>.ts.net:<port>` with automatic certificates, and
  the same trusted-origin model works behind Caddy, nginx, or `cloudflared`.
- **Strict `/mcp` and `/healthz` isolation.** `/mcp` and `/healthz` sit behind
  their own route middleware that requires a single loopback `Host`
  (`127.0.0.1`, `localhost`, or `[::1]`), refuses any non-loopback `Origin`
  (including a configured remote board origin), and refuses any request
  carrying proxy headers (`Tailscale-User-Login`, `X-Forwarded-Host`,
  `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Port`, `Forwarded`)
  with 403.
- **Fail-closed request classification on board routes.** Every board route
  (`/`, `/api/state`, `/api/events`, `/api/msg/*`, `/api/contexts`,
  `/api/new-issues`, and all mutating routes) runs behind one shared board
  middleware. A request is classified as **local** only when it carries none of
  the proxy headers above and has a single `Host` matching `127.0.0.1:<port>`,
  `localhost:<port>`, or `[::1]:<port>` on `t3cm`'s own port. Every other
  request is classified as **remote**.
- **Opt-in `board.remote` config and origin validation.** `board.remote`
  defaults to off (`null` or `mode: "off"`). When off, every remote request is
  refused with 403. When enabled (`mode: "pairing"` or `"tailscale"` with an
  `https://` `origin`), remote requests must present `X-Forwarded-Proto: https`
  when that header is set, a `Host` (or own loopback `Host` paired with
  `X-Forwarded-Host`) matching the configured `origin`, a matching `Origin`
  when present, valid remote authentication, and `X-T3CM-CSRF` on every
  mutating method (`POST`, `PUT`, `DELETE`).
- **Dual-mode remote authentication and CLI surface.**
  - Pairing mode stores SHA-256-hashed 5-minute single-use pairing tokens and
    30-day sessions in `store.db`, redeemed from `/pair#<token>` to
    `POST /api/pair` for an `HttpOnly; Secure; SameSite=Strict` cookie.
  - Tailscale Serve mode honors `Tailscale-User-Login` only when enabled,
    checked against `allowUsers` (defaulting to the host's own login from
    `tailscale status --json`), with optional `requirePairing: true` to layer
    pairing cookies on top.
  - Remote management lives under `t3cm board expose|unexpose|pair|sessions|revoke`.
    Tailscale Serve defaults to HTTPS port `47732` so it never collides with
    T3 Code's own `:443`. Only `t3cm board expose` and `unexpose` mutate
    Tailscale state; `t3cm serve` never does, and `t3cm doctor` reports drift.
    Session management stays CLI-only in v1.
- **Remote client UX.** `board.html` calls `POST /api/open` only on loopback
  hosts; remote devices open `https://app.t3.codes/threads/...` directly.

### Consequences

- Good: `/mcp` and `/healthz` refuse requests from any proxy that adds trusted
  forwarding headers and enforces the configured public `Host`, even when
  remote board access is turned on.
- Good: direct loopback users keep zero-auth access with no workflow change,
  while remote access stays completely off until opted in.
- Good: one board middleware guards every read and write route, so new routes
  cannot drift out of auth checks.
- Good: proxy-header spoofing and host-confusion attempts cannot downgrade a
  proxied request into local zero-auth access.
- Bad: route isolation and remote classification depend on the reverse proxy
  exposing only board routes, stripping client-supplied `Tailscale-User-Login`,
  `Forwarded`, and `X-Forwarded-*` headers, always adding a trusted forwarding
  header (`X-Forwarded-For`, `X-Forwarded-Host`, `X-Forwarded-Proto`, or
  `Forwarded`; Tailscale Serve adds its own), and rejecting or overwriting any
  client `Host` that does not match the configured public name. nginx's default
  `proxy_pass` sets `Host` to the upstream and adds no `X-Forwarded-*` header,
  so it needs explicit `proxy_set_header` lines (`Host $host`,
  `X-Forwarded-For $proxy_add_x_forwarded_for`, `X-Forwarded-Proto $scheme`).
  Raw TCP forwarders (`ssh -L`, `socat`, `tailscale serve --tcp`) are
  indistinguishable from direct loopback and hand out zero-auth board access,
  so they must never front the board.
- Bad: splitting the rollout across four PRs leaves remote requests refused
  with 401 in PR 1 until pairing and Tailscale auth land in PRs 2 and 3.
- Neutral: reversing this decision means restoring one loopback middleware in
  `src/server.rs`, removing `board.remote` from `src/config.rs`, and removing
  the remote auth and CLI code added by the follow-up PRs.

## Pros and cons of the options

### Expose the whole `t3cm` HTTP server behind one shared middleware

- Good: smallest router change.
- Bad: exposing `/mcp` remotely gives any remote session or misconfigured
  proxy direct access to Code Mode `execute` and every configured upstream,
  extending remote reach far beyond board operations.

### Bind non-loopback and terminate TLS in `t3cm`

- Good: no external proxy required.
- Bad: forces `t3cm` to manage ACME/Let's Encrypt or self-signed certificates
  across Linux, macOS, and Windows, conflicts with the loopback-only default,
  and duplicates Tailscale Serve's managed TLS and identity injection.

### Split `/mcp` from board routes with external TLS and opt-in remote auth

- Good: preserves loopback isolation for `/mcp` and `/healthz`, keeps `t3cm`
  bound to `127.0.0.1`, fits Tailscale Serve and generic HTTPS reverse proxies
  alike, and fails closed when unconfigured or unauthenticated.
- Bad: requires strict request classification around `Host` and proxy headers
  because proxied traffic arrives over the loopback socket.

## More information

Issue #164 defines the requirements and settled design. Implementation lands in
four ordered PRs:

1. Split `/mcp` and `/healthz` middleware from board routes, add request
   classification and `board.remote` config, refuse all remote board requests
   until auth modes exist, add this ADR, and update `docs/src/security.md`.
2. Add pairing-cookie auth, `store.db` token/session tables, CLI pairing and
   session management, and reverse-proxy docs.
3. Add Tailscale Serve integration (`expose`/`unexpose`, `Tailscale-User-Login`
   enforcement, status and doctor drift checks).
4. Add remote client UX (`board.html` remote link behavior, web manifest and
   icon, and expired-session sign-in view).
