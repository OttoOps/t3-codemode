# Security model

`t3cm` is a local, single-user service. It holds a T3 session with
`full-access` rights and runs code that a model wrote. The model below keeps
that code inside a sandbox and keeps the service off the network.

## Trust boundary

Any local process that can reach the port acts with the service's T3 session.
That is the same trust boundary as the `t3` CLI for the same user. The service
does not authenticate callers beyond that, and it does not try to.

## Network

- It binds `127.0.0.1` only.
- Every route answers only requests whose `Host` is `127.0.0.1`, `localhost` or
  `[::1]` (any port), and refuses others with 403, which shuts out DNS
  rebinding.
- `POST /mcp` and the board routes also refuse a request whose `Origin` is not
  a loopback host.
- Board routes are stricter: `Host` must name the service's own port, and every
  POST needs the page's per-process CSRF token in `X-T3CM-CSRF`. See
  [The board](board.md#http-routes).

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
