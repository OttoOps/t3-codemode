# t3-codemode

`t3cm` is a Code Mode MCP service for T3 Code. A client sees two tools,
`search` and `execute`, in place of every T3 tool schema. The model looks up
what it needs with code over a catalog, then runs code against that catalog in
a sandbox. The tools of any other MCP server you configure join the same
catalog under their own namespace.

CI builds this site from the `docs/` directory of the
[repository](https://github.com/OttoOps/t3-codemode) and redeploys it on every
push to `main`. For now the [README](https://github.com/OttoOps/t3-codemode#readme)
covers install, config and the board. These pages are here today:

- [API reference](api.md): the TypeScript declarations for `t3`, `tools`,
  `mem`, `vars` and `wf`, which the site build generates from `t3cm types`.
- [Upstreams contract](upstreams-contract.md): how other MCP servers plug in as
  upstreams.

To contribute, read the
[contribution guide](https://github.com/OttoOps/t3-codemode/blob/main/.github/CONTRIBUTING.md).
Report vulnerabilities through the
[security policy](https://github.com/OttoOps/t3-codemode/blob/main/.github/SECURITY.md).
