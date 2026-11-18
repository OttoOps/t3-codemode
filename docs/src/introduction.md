# t3-codemode

`t3cm` is a Code Mode MCP service for T3 Code, in the style of Cloudflare's Code
Mode. It puts T3's MCP tools, and the tools of any other MCP servers you
configure (upstreams), behind two tools, `search` and `execute`. Instead of 74
tool schemas (about 166k chars of JSON) in every session, a client sees two
tools (about 3.3k chars for the whole `tools/list`). The model finds what it
needs with code over a catalog, then runs code against that catalog in a
sandbox.

Beyond T3's tools, the sandbox carries:

- any other MCP server as an upstream, under its own namespace,
- a persistent memory store (`mem`, `vars`),
- a thread-workflow layer (`wf`): parent and child links between threads,
  gates, wakes and a watcher that nudges stalled threads,
- a local web board that acts as the root overseer for ticket threads.

Status: prototype. It builds and passes its tests on Linux, macOS and Windows,
and installs as a per-user service on each: a systemd user unit, a launchd
agent, or a logon task.

## Where to go next

- [Getting started](getting-started.md): requirements, install on each OS, and
  the `t3cm` commands.
- [Configuration](config.md): the config file, its defaults, and where state
  lives.
- [The two tools](tools.md): `search` and `execute`, with examples.
- [Execute globals](globals.md): `t3`, upstream namespaces, `tools`, `mem`,
  `vars`, `wf` and `console`.
- [The board](board.md): decision cards, the feed, the ask box and the HTTP
  routes.
- [Upstreams](upstreams.md): wrap any MCP server over stdio or HTTP.
- [Security model](security.md): what the service trusts and what it shuts out.
- [Troubleshooting](troubleshooting.md): the usual failures and how to read
  them.
- [API reference](api.md): the TypeScript declarations the site build generates
  from `t3cm types`.
- [Contributing](contributing.md): the contribution guide, the security policy
  and the development checks.

CI builds this site from the `docs/` directory of the
[repository](https://github.com/OttoOps/t3-codemode) and redeploys it on every
push to `main`.
