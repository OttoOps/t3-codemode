# Security policy

## Supported code

t3-codemode is a prototype with no tagged releases yet. We investigate
suspected vulnerabilities in the current `main` branch. Include the commit you
tested so we can identify the affected code.

There is no maintained release series or backport commitment. When releases
exist, this section will name the versions that receive security fixes.

## Report a vulnerability privately

Use [GitHub private vulnerability reporting](https://github.com/OttoOps/t3-codemode/security/advisories/new).
Do not open a public issue or pull request with exploit details or live
credentials. The maintainer, `potto007`, reviews reports through the private
advisory and may invite the people needed to investigate or fix the problem.

Include what you know:

- The affected component and the commit you tested.
- The configuration, access and other prerequisites: your OS, the T3 Code
  version, the `t3cm` config (with secrets removed), and any upstream MCP
  servers involved.
- The security impact and the steps to reproduce.
- A minimal proof that uses made-up tokens and a small script or request.
- Any mitigation you have found.

Remove tokens, pairing codes, OAuth material, machine-specific absolute paths,
and personal data from attachments and from any script or log you paste. Test
only on systems you control or have permission to assess.

If details have already been posted publicly, link that post in the private
report and avoid adding further exploit detail there.

## Scope

Reports about these components belong here:

- The `t3cm` service: the MCP endpoints (`/mcp`, stdio), the board page and its
  `/api` routes, and `/healthz`.
- The QuickJS sandbox that runs `search` and `execute` code, and the host
  functions it exposes (`t3`, `wf`, `mem`, `vars`, upstream namespaces).
- How the service gets, stores, refreshes and revokes its T3 session.
- The upstream MCP clients (stdio and Streamable HTTP), and the handling of
  `${env:}` and `${file:}` secret references.
- `deploy/install.sh` and the service units it installs.

In scope, for example:

- Code in `execute` that escapes the sandbox, or calls a T3 tool on the deny
  list.
- The T3 token, an upstream credential, or a file's contents leaking through
  tool output, logs, errors or the board.
- A cross-site request, a DNS-rebinding page or a non-local origin that drives
  the board or `/mcp`.
- One upstream reading another upstream's secrets.
- Resource exhaustion that gets past the configured CPU, memory, timeout and
  output limits.

Out of scope here:

- Faults in T3 Code itself. Report those to
  [pingdotgg/t3code](https://github.com/pingdotgg/t3code/security). Report
  them here too if this project's use of T3 creates or worsens the exposure.
- Faults in an upstream MCP server you configured, unless `t3cm` makes them
  worse.
- Other local processes running as the same user. Any local process that can
  reach the port acts with the service's T3 session, the same trust boundary
  as the `t3` CLI.
- Exposing the service beyond loopback. It binds `127.0.0.1` by design, and
  deploying it otherwise is an operator decision.

Ordinary bugs, usage questions, and feature requests belong in
[GitHub Issues](https://github.com/OttoOps/t3-codemode/issues).
Conduct concerns follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Triage and disclosure

The maintainer reviews reports, asks for missing details, and coordinates
fixes and disclosure with the reporter through the private advisory. Timing
depends on severity and maintainer availability. There is no guaranteed
acknowledgement or fix deadline, and no bug bounty.

For a confirmed issue, we aim to publish an advisory alongside a fix or a
useful mitigation, naming the affected commits or versions and the fix.
Reporters are credited when they want to be. Keep exploit details and
proposed fixes in the private advisory while we coordinate publication.
