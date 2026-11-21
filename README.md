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

Status: prototype. It builds and passes its tests on Linux and macOS, and
installs as a per-user service on Linux, macOS and Windows. Windows CI and
Windows release binaries are paused until we have a code-signing certificate,
so v0.6.0 is the last release with a Windows archive
([#123](https://github.com/OttoOps/t3-codemode/issues/123)).

Documentation: https://ottoops.com/t3-codemode/

## Install the service

With T3 Code running, on Linux or macOS:

```bash
curl -fsSLO https://raw.githubusercontent.com/OttoOps/t3-codemode/main/deploy/install.sh
bash install.sh   # latest release: binary, user service, MCP entries, workflow skills
t3cm status       # t3.connected should be true
```

The installer registers `t3cm` over HTTP for Claude Code and adds a `t3cm stdio`
entry to every T3 Code Antigravity profile's `config/mcp_config.json`. A
non-empty `mcpServers` object in that file stops T3 Code from using Antigravity
as its system text-generation model. Antigravity agent threads continue to work;
see [Antigravity registration](https://ottoops.com/t3-codemode/getting-started.html#antigravity).

The installer asks `Use the T3 workflow? [Y/n]`. On yes, the default, it
installs the workflow skills below. Update it in place with `t3cm update`,
which brings the skills along; `t3cm --version` says when a newer release is
out. On Windows, use `deploy/install.ps1 -Version v0.6.0`. Linux and macOS can
also build from a clone with `deploy/install.sh --from-source`. See
[Getting started](https://ottoops.com/t3-codemode/getting-started.html) for
every OS, the config, and the `t3cm` commands.

## Install the workflow skills

The T3 workflow runs on five agent skills in [`skills/`](skills). They need
T3 Code, whose `t3_thread_launch` tool starts the ticket and worker threads,
and the `t3cm` service, which runs the link protocol and resolves the slots.
They load in T3 Code's Claude Code, Codex and Antigravity threads.

| Skill | Say | What it does |
|---|---|---|
| `t3-workflow` | "how does the T3 workflow work" | The link protocol under every workflow thread, and the router to the other skills. |
| `setup-t3-workflow` | "set up the T3 workflow" | Picks the reviewer, passes, credential renewal, delivery and models, and writes the config. |
| `t3-overseer` | "work 41, 42 and 43 in parallel" | One root thread over several issues, with one list of the decisions you owe. |
| `t3-ticket` | "work on issue 42" | Splits an issue into PRs, runs one worker thread per PR, merges and deploys on your word. |
| `t3-worker` | nothing, `t3-ticket` launches it | Builds one PR, has it reviewed, follows it to merged, hands off. |

### 1. Get the skills

Install the [service](#install-the-service) and answer yes to the workflow
question. The t3cm installer is the only way to get the skills. On a machine
that already has t3cm, run:

```bash
t3cm skills install
```

Each t3cm binary carries the skills of its own release. `t3cm skills install`
writes all five to `~/.agents/skills`, where Codex reads them, and links each
into Claude Code's `~/.claude/skills` and Antigravity's
`~/.gemini/config/skills` when those agents are installed. It also applies
the setup steps that need no answers, such as the filed-issue rule. Rerunning
the installer or running `t3cm update` brings them to the next release, and
keeps any file you edited. `t3cm skills status` says whether they are current.
See [Getting
started](https://ottoops.com/t3-codemode/getting-started.html#install-the-skills-with-t3cm)
for the details.

When `npx skills` or a copy by hand installed the skills before, the installer
offers to replace them, and yes is the default. It runs
`t3cm skills install --take-over`, which writes this release's files and drops
the skills from the `npx skills` lock file, so `npx skills update` no longer
overwrites them. Run that command yourself if you answered no.

### 2. Run the setup skill once

Ask your agent to "set up the T3 workflow", or run `/setup-t3-workflow`. It
finds what you have installed, asks about one slot at a time, and writes the
config. Run it again whenever you want to change a slot. A slot that no
config sets keeps its default: review by a `sol-loop` skill if one is
installed, no passes, delivery by merge.

### Which skills need which

Some skills call others, and a few read their neighbors' files by relative
path, so t3cm installs all five together in one folder:

- Every skill needs `t3-workflow`, which documents the `wf.*` link protocol
  and slot lookups, and a running `t3cm` service (via `mcp__t3cm__execute`
  in Claude Code, or `t3cm exec -` in Codex and other shell-only agents).
- `t3-overseer` launches `t3-ticket` threads.
- `t3-ticket` launches `t3-worker` threads and reads `t3-worker`'s procedure.
  For tag-deploy delivery it runs the deploy skill you name in setup.
- `setup-t3-workflow` runs `t3cm setup` and uses `t3-workflow`'s docs.
- `t3-worker` uses the skills you put in its slots: a reviewer such as
  `sol-loop` from [t3dotgg/tstack](https://github.com/t3dotgg/tstack), passes
  such as `pr` from [mattpocock/skills](https://github.com/mattpocock/skills),
  and a credential renewal skill. Each is optional, and the worker skips a
  slot with no skill.

## Contributing

Every commit carries a DCO signoff (`git commit -s`), and pull requests merge
with rebase. The [contribution guide](.github/CONTRIBUTING.md) has the rest. Report
vulnerabilities privately through the [security policy](.github/SECURITY.md).
Participation follows the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

MIT, see `LICENSE`.
