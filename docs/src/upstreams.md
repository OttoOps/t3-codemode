# Upstreams

An upstream is another MCP server, over stdio or Streamable HTTP (the legacy
HTTP+SSE transport is not supported), listed under `upstreams` in the
[config file](config.md). Its tools join the
catalog under its namespace. The service holds its credentials and redacts
them from what reaches the sandbox, tool output, `/healthz` and logs, within
the limits under [Secret references](#secret-references): tracked values of 4
or more bytes, in raw or JSON-escaped form. T3 itself stays built in as
the namespace `t3`, with its own OAuth client.

## Config

```json
{
  "upstreams": [
    {
      "name": "cbm",
      "transport": "stdio",
      "stdio": { "command": "codebase-memory-mcp" },
      "timeout_ms": 120000,
      "deny": ["delete_project", "index_repository"]
    },
    {
      "name": "linear",
      "transport": "http",
      "http": {
        "url": "https://mcp.linear.app/mcp",
        "headers": { "Authorization": "Bearer ${file:~/.config/t3-codemode/linear.token}" }
      },
      "allow": ["list_*", "get_*", "search_*"]
    }
  ]
}
```

| Key | Default | Rule |
|---|---|---|
| `name` | required | the JS namespace: `^[a-z][a-z0-9_]{0,31}$`, unique, and none of `t3`, `mem`, `vars`, `wf`, `console`, `tools`, `catalog` or a JS reserved word |
| `transport` | required | `"stdio"` or `"http"`; exactly the matching section is present |
| `stdio.command` | required | a bare name (searched on `PATH`, then `$HOME/.local/bin`) or a path; a relative path (`./bin/srv`) is resolved against `stdio.cwd`; on Windows a name with no extension also tries each `PATHEXT` extension (`npx` finds `npx.cmd`) |
| `stdio.args`, `stdio.cwd` | `[]`, service cwd | no references |
| `stdio.env` | `{}` | values may hold references |
| `http.url` | required | `http://` or `https://`; references allowed in the userinfo, path and query; the host takes none |
| `http.allow_insecure` | `false` | `true` allows credentials (any reference in the URL or headers) over `http://` to a non-loopback host |
| `http.headers` | `{}` | values may hold references |
| `enabled` | `true` | `false`: no global, no catalog entries, never connects |
| `timeout_ms` | `60000` | per request, 100 to 600000 |
| `allow`, `deny` | `["*"]`, `[]` | tool-name globs (`*`, `?`); deny wins. A denied tool is absent from the catalog and its calls throw |

`t3cm status` lists every configured upstream, disabled ones included, in config
order, with `name`, `transport`, `enabled`, `state` (`disabled`, `idle`,
`connecting`, `ready`, `backoff`), `tools`, `restarts`, `lastError` (redacted),
`retryAt` and `server` (the upstream's `serverInfo` `name`, `title` and
`version`, each cut to 100 characters).

## Namespacing

Each upstream's `name` is its namespace: one frozen global
in `execute` (`cbm.search_graph(input)`, `cbm.call(name, input)`), the
`server` of its catalog entries, and the prefix of their names
(`cbm.search_graph`). `tools.call("cbm.search_graph", input)` reaches the same
tool by its qualified name. T3 tools keep their bare names (`t3_thread_send`)
and the built-ins are `mem.*`, `wf.*` and `vars`; the reserved names keep the
three sets disjoint. A tool name that is not an identifier is reached as
`cbm["get-thing"]`, with a sanitized alias (`cbm.get_thing`) when no other tool
claims it.

## Secret references

A value in `stdio.env`, `http.headers` or `http.url` may mix literal text
with `${env:NAME}` (the service's environment; under systemd, the unit's) and
`${file:/abs/path}` or `${file:~/path}` (the file's content, trailing newlines
trimmed); `$${` is a literal `${`. Put each secret, whole, inside a reference.
An `Authorization`, `Proxy-Authorization` or `Cookie` header, or any header or
env name containing `TOKEN`, `SECRET`, `KEY`, `PASSWORD`, `PASSWD`, `AUTH`,
`COOKIE` or `CREDENTIAL`, whose value holds no reference is a config error. A
literal under any other name is accepted. The service resolves references at
every connect, so it picks up a rotated file on the next reconnect.

For redaction the service tracks each resolved reference, the whole resolved
value of any env var or header that holds a reference, a URL's password, and
the values of query parameters whose name looks like a secret (`?api_key=`).
Literal text beside a reference is redacted only as part of that whole value,
so it can appear unredacted on its own. Every tracked value of 4 or more bytes
is redacted (`[redacted]`) from tool results before they enter the sandbox, and
from errors and output, including its JSON-escaped form. Two fields are cut
before redaction runs: a tool error at 1,000 characters, and each `serverInfo`
field that `/healthz` reports at 100 characters. A secret that straddles either
cut keeps its leading part and no longer matches.

## Lifecycle

Nothing connects at start-up beyond a background warm-up; each
upstream connects on first use (`search`, `execute` or the 10-minute catalog
refresh). Connects run in the background: `search` and `execute` wait at most
2 s, and only for an upstream whose tools have never been listed; otherwise
they use the last tool list, and a call into a namespace waits for its own
connect. A connect (spawn or HTTP handshake plus the full `tools/list`) may
take `timeout_ms` or 15 s, whichever is longer. A failure puts only that
upstream in backoff: the first failure retries on next use, repeated failures
back off from 1 s doubling to 60 s. Its last tool list stays in the catalog. A
failed call is never retried, with one exception: when an HTTP upstream answers
404 to an established session, the client starts a new session and sends the
call once more, so a server that acted before answering 404 acts twice. A stdio server that exits between calls is
restarted on the next use without failing that call, and three timeouts in a
row restart a connection. A re-list is capped at 15 s, and the watcher
refreshes upstreams on their own task, so a hung upstream never delays the
workflow watcher. A server's `tools/list_changed` notification re-lists it,
and the next `search` or `execute` sees the new catalog. The service logs connects and
failures (`upstream connected`, `upstream unavailable: <redacted reason>`), and
logs an upstream's stderr at `debug` only, redacted.

## Security

- The service resolves `${env:..}` and `${file:..}` references in
  `stdio.env`, `http.headers` and `http.url` at connect time. It replaces each
  tracked value (see [Secret references](#secret-references)) of 4 or more
  bytes in results before they enter the sandbox, and filters errors, final
  output, `/healthz` `lastError` and log lines the same way. The filter matches
  exact values (raw and JSON-escaped), so a shorter secret, an untracked
  literal, or a secret an upstream returns in another encoding gets past it.
- A stdio upstream runs as a child of the service, in its own process group
  or, on Windows, its own Job Object, with only `PATH`, `HOME`, `USER`,
  `LOGNAME`, `SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `TMPDIR`, `TERM` and
  the `XDG_*` base directories from the service's environment, plus its own
  `env`. It inherits another upstream's `${env:..}` credential only when that
  variable's name is on the allowlist, such as an `LC_*` name, so keep
  credentials in variables outside it. The filter controls inheritance only.
  The child runs as the service's user, so it can read that user's files,
  other upstreams' credential files included, and, where the OS allows it, the
  service's environment. Run only stdio upstreams you trust with that access.
  Pass anything else it needs through its `env`
  (`"NODE_OPTIONS": "${env:NODE_OPTIONS}"`). Stopping it kills the whole
  group, so a server behind a wrapper (`npx`, `uvx`, `sh`) stops too. `args`
  are visible in `ps`, which is why they cannot hold references. On Windows the
  allowlist also passes the system variables programs need to start
  (`SYSTEMROOT`, `COMSPEC`, `PATHEXT`, `TEMP`, `USERPROFILE`, `APPDATA`,
  `LOCALAPPDATA` and the rest of `client_stdio::INHERITED_ENV_WINDOWS`), and
  matches names in any case.
- An HTTP upstream never follows redirects (a 3xx is an error), so its headers
  never reach another host. A reference in the URL or headers over `http://` to
  a non-loopback host needs `http.allow_insecure: true`. The check looks for
  references only, so a credential written literally into `http.url` goes out
  over `http://` without it. Use `https://` and references. Transport errors
  leave out the URL, but an HTTP error status adds up to 300 characters of the
  response body, redacted of tracked values only, so an upstream that echoes
  its URL can put an untracked literal from it into the error.
- One message from an upstream (a stdio line, an HTTP body or SSE event) is
  capped at 16 MiB; a larger one fails the connection.
- The host enforces `allow`/`deny`. A denied tool has no catalog entry and
  no global, and `tools.call` on it throws without contacting the upstream.
  Use `deny` for destructive tools you never want a script to reach.
- An upstream's tool descriptions and results are its own text. Treat them as
  data from that server, as you would its direct MCP tools.

The design and its acceptance tests are in the
[upstreams contract](upstreams-contract.md), a design record that has not kept
up with the code. Where the two differ, this page and the code govern.
