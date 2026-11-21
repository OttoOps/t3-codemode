# Security model

`t3cm` is a local, single-user service. It holds a T3 session with
`full-access` rights and runs code that a model wrote. The model below keeps
that code inside a sandbox and keeps the service off the network.

## Trust boundary

Any local process that can reach the loopback port directly acts with the
service's T3 session. That is the same trust boundary as the `t3` CLI for the
same user. Direct loopback callers are not authenticated beyond that.

The HTTP server splits `/mcp` and `/healthz` from the web board (`/`,
`/api/*`) into separate middleware boundaries:

- `/mcp` and `/healthz` are strictly loopback-only. When fronted by a proxy
  that adds trusted forwarding headers and rejects or overwrites non-public
  `Host` values, they refuse remote hosts, remote origins, and proxied requests
  even when remote board access is enabled.
- The board stays loopback-only by default (`board.remote` is off). When
  explicitly enabled behind an HTTPS reverse proxy such as Tailscale Serve,
  Caddy, nginx, or `cloudflared`, only the board routes accept requests for the
  configured `https://` origin, and every remote read and write route requires
  remote authentication.

See [ADR 0008](https://github.com/OttoOps/t3-codemode/blob/main/docs/adr/0008-remote-board-trust-boundary.md)
for the split trust boundary and reverse-proxy trust assumptions.

## Network

- The service binds `127.0.0.1` only.
- `/mcp` and `/healthz` require a single `Host` naming `127.0.0.1`, `localhost`,
  or `[::1]` (any valid port), refuse any non-loopback `Origin` (including a
  configured remote board origin), and refuse any request carrying proxy
  headers (`Tailscale-User-Login`, `X-Forwarded-Host`, `X-Forwarded-For`,
  `X-Forwarded-Proto`, `X-Forwarded-Port`, `Forwarded`) with 403.
- Board routes (`/`, `/api/*`) classify each request before running any handler:
  - A request is **local** only when it carries none of the proxy headers above
    and has a single `Host` naming `127.0.0.1:<port>`, `localhost:<port>`, or
    `[::1]:<port>` on the service's own port. Local requests require any
    `Origin` header to match the service's own `http://` loopback origin and
    require `X-T3CM-CSRF` on every mutating method (`POST`, `PUT`, `DELETE`).
  - Any other request (non-loopback `Host`, wrong port, missing or duplicate
    `Host`, or any proxy header) is **remote**. Because reverse proxies connect
    over `127.0.0.1`, proxy headers prevent a proxied request from falling into
    local zero-auth access.
- With `board.remote` off (the default), every remote request to every board
  read and write route is refused with 403.
- With `board.remote` enabled (`mode` and an `https://` `origin`), a remote
  board request must satisfy all of the following on both read routes (`/`,
  `/api/state`, `/api/events`, `/api/msg/*`, `/api/contexts`,
  `/api/new-issues`) and mutating routes:
  - `X-Forwarded-Proto`, when present, must be `https`.
  - `Host` must match the configured `origin`, or `Host` must be the service's
    own loopback host while `X-Forwarded-Host` matches the configured `origin`.
    If `X-Forwarded-Host` is present alongside a non-loopback `Host`, both must
    match the configured `origin`.
  - `Origin`, when present, must match the configured `https://` `origin`.
  - Remote authentication must succeed:
    - In `pairing` mode, the request must carry a valid `__Host-t3cm_session`
      cookie (`HttpOnly; Secure; SameSite=Strict; Path=/`) backed by a
      SHA-256-hashed 30-day session in `store.db`, minted from a single-use
      5-minute `/pair#<token>` token via `t3cm board pair` and `POST /api/pair`
      (`GET /pair` and `POST /api/pair` enforce the same remote `Host`,
      `X-Forwarded-Proto`, and `Origin` checks before session auth; `/pair`
      never embeds the board CSRF token, and `POST /api/pair` requires
      `Content-Type: application/json`). Revoking or expiring a session cuts
      off API calls with 401 and closes open `/api/events` streams.
    - In `tailscale` mode, the request must carry a single
      `Tailscale-User-Login` header matching `board.remote.allowUsers` (or,
      when `allowUsers` is empty, the host's own login from
      `tailscale status --json`). Any other login is refused with 403, and a
      missing header or unavailable host login is refused with 401. When
      `requirePairing: true` is also set, the request must also carry a
      valid `__Host-t3cm_session` cookie (and `/pair` and `POST /api/pair`
      require an allowed `Tailscale-User-Login` before redeeming a token).
      `Tailscale-User-Login` grants no access when `board.remote.mode` is
      `"off"` or `"pairing"`.
  - Every mutating method (`POST`, `PUT`, `DELETE`) on authenticated board
    routes still requires the per-process `X-T3CM-CSRF` header.
- **Reverse-proxy trust assumption.** Because `t3cm` listens on `127.0.0.1`,
  any forwarder that sends a loopback `Host` with no forwarding headers looks
  identical on the wire to a direct loopback client. Any proxy fronting the
  board must terminate TLS, forward only board routes (never `/mcp` or
  `/healthz`), strip client-supplied forwarding and identity headers
  (`X-Forwarded-*`, `Forwarded`, `Tailscale-User-Login`), always add a trusted
  forwarding header (`X-Forwarded-For`, `X-Forwarded-Host`,
  `X-Forwarded-Proto`, or `Forwarded`; Tailscale Serve adds its own), and
  reject or overwrite any client `Host` that does not match the configured
  public name. nginx's default `proxy_pass` rewrites `Host` to the upstream and
  adds no `X-Forwarded-*` header, so nginx requires explicit `proxy_set_header`
  lines (`Host $http_host` or an explicit public `host:port` when non-443,
  `X-Forwarded-For $remote_addr` at the edge so client-supplied headers are
  overwritten rather than appended, and `X-Forwarded-Proto https`). Raw TCP
  forwarders such as `ssh -L`, `socat`, and `tailscale serve --tcp` are
  indistinguishable from direct loopback and hand out zero-auth board access,
  so they must never front the board. See [Remote access](board.md#remote-access).

## The sandbox

Each run gets a fresh QuickJS runtime:

- 64 MB memory and a 512 KB stack,
- a JavaScript CPU budget, `cpuBudgetMs`, 2 s by default (time awaiting host
  calls does not count),
- a wall timeout, `timeout_ms`, 60 s by default and capped at `maxTimeoutMs`
  (600 s by default),
- no fs, network, timers, modules, `SharedArrayBuffer` or `Atomics`.

The only way out of the sandbox is the host functions behind `t3`, the upstream
namespaces, `tools`, `mem`, `vars` and `wf`. The host functions enforce the
`deny` list themselves, so a denied T3 tool throws however the script reaches
it, whatever the prompt says. At most `maxConcurrentRuns` (16) runs execute at once.

## GitHub

`wf.merge` is the one host function that reaches GitHub. The service runs
`gh api graphql` itself, as the user it runs under, with that user's existing
`gh` login. Sandboxed code cannot run `gh` or send GraphQL of its own: it passes
a repo, a pull request number, a head SHA and a merge method, the service
checks their shape, and the service sends one of three fixed operations (read
the pull request, `enqueuePullRequest`, `mergePullRequest`). The service never
reads the `gh` token.

The board's [GitHub reconcile](board.md#github-reconcile) uses the same `gh`
login. It sends one fixed read query, with each tracked pull request's owner,
name and number as GraphQL variables, and the same two fixed mutations, only
for a pull request someone approved through `wf.merge` and only at the head
they approved.

## The T3 token

The token lives only in the store and in memory. It never enters the sandbox,
tool output or logs, and the service redacts any error text that contains it.

## T3 sign-in

On start, and when the stored token has under 3 days left or T3 answers 401,
the service runs T3's MCP OAuth flow against the local server:

1. It registers a client.
2. It mints a 5-minute pairing code with the `t3` CLI.
3. It approves the code as `full-access`.
4. It exchanges the code with PKCE for a 30-day bearer token.

The T3 origin comes from `~/.t3/userdata/server-runtime.json` (`T3CODE_HOME`
moves `~/.t3`, and `T3CM_T3_ORIGIN` overrides the origin). The `t3` binary is
`$T3CM_T3_BIN`, else the running server's executable when it is named `t3`
(`t3.exe` on Windows), else the newest `~/.t3/wsl-runtime/sha256-*/t3`, else the
newest `~/.t3/runtime/versions/*/t3` that T3's installer unpacks.

`t3cm auth` runs the flow on demand and prints only the new token's expiry.

## Upstream credentials

- Put every credential, whole, in an `${env:..}` or `${file:..}` reference in
  `stdio.env`, `http.headers` or `http.url`. The service resolves references at
  connect time and tracks each resolved reference, the whole value of any env
  var or header that holds a reference, a URL's password, and the values of
  secret-named query parameters. It rejects a literal value only under a
  secret-looking name. A literal under another name is accepted and not
  tracked, and literal text beside a reference is redacted only as part of the
  whole value, never on its own.
- The service replaces each tracked value of 4 or more bytes with
  `[redacted]` in upstream results before they enter the sandbox, and filters
  errors, final output, `/healthz` `lastError` and log lines the same way. The
  filter matches exact values in raw and JSON-escaped forms, so a credential
  shorter than 4 bytes, or one an upstream returns in another encoding, gets
  past it. Tool errors (1,000 characters) and `serverInfo` fields (100
  characters) are cut before redaction, so a secret that straddles the cut can
  leak its leading part.
- Each stdio upstream inherits only an allowlisted environment plus its own
  `env`, so it does not inherit another upstream's credential unless the
  variable holding it has an allowlisted name, such as an `LC_*` name. That
  filter is not an isolation boundary. A stdio upstream runs as the service's
  user and can read whatever that user can, including other upstreams'
  credential files and, where the OS allows it, the service's own environment.
  Run only stdio upstreams you trust with that access.
- An HTTP upstream never follows redirects, so its headers never reach another
  host. A credential written literally into `http.url` goes out over `http://`
  without `http.allow_insecure`, so use `https://` and references.
- The host enforces `allow` and `deny` on every call.

[Upstreams](upstreams.md#security) has the details.

## On disk

The store is `0700` and `0600` on Linux and macOS. On Windows it relies on the
per-user ACL of `%LOCALAPPDATA%`, and t3cm sets no ACL of its own, so a
`--state-dir` elsewhere gets whatever its parent grants. Protect a custom state
directory yourself, because the store holds the T3 token. See
[Configuration](config.md#state).

## Reporting a vulnerability

Report suspected vulnerabilities privately through the
[security policy](https://github.com/OttoOps/t3-codemode/blob/main/.github/SECURITY.md),
not in a public issue or pull request.
