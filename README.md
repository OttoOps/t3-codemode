# t3-codemode

`t3cm` is a Code Mode MCP service for T3 Code. It puts T3's MCP tools, and the
tools of any other MCP servers you configure (upstreams), behind two tools,
`search` and `execute`, in the style of Cloudflare's Code Mode. Instead of 74
tool schemas (about 166k chars of JSON) in every session, a client sees two
tools (about 3.3k chars for the whole `tools/list`). The model finds what it
needs with code over a catalog, then runs code against it in a sandbox.

Beyond T3's tools, the sandbox carries:

- any other MCP server as an upstream, under its own namespace,
- a persistent memory store (`mem`, `vars`),
- a thread-workflow layer (`wf`): parent and child links between threads, gates,
  wakes and a watcher that nudges stalled threads,
- a local web board that acts as the root overseer for ticket threads.

Status: prototype. It builds and passes its tests on Linux, macOS and Windows,
and installs as a per-user service on each.

Documentation: https://ottoops.com/t3-codemode/

## Install

With T3 Code running, on Linux or macOS:

```bash
curl -fsSLO https://raw.githubusercontent.com/OttoOps/t3-codemode/main/deploy/install.sh
bash install.sh   # latest release: binary, user service, Claude Code MCP entry
t3cm status       # t3.connected should be true
```

On Windows, use `deploy/install.ps1`. Until the first release is published,
Linux and macOS build from a clone with `deploy/install.sh --from-source`. See
[Getting started](https://ottoops.com/t3-codemode/getting-started.html) for
every OS, the config, and the `t3cm` commands.

## Contributing

Every commit carries a DCO signoff (`git commit -s`), and pull requests merge
with rebase. The [contribution guide](.github/CONTRIBUTING.md) has the rest. Report
vulnerabilities privately through the [security policy](.github/SECURITY.md).
Participation follows the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

MIT, see `LICENSE`.
