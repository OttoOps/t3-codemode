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

## Install the service

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

## Install the workflow skills

The T3 workflow runs on five agent skills in [`skills/`](skills). They install
apart from the service and work in Claude Code and Codex. Without `t3cm`, the
`t3-workflow` skill's own script runs the whole protocol.

| Skill | Say | What it does |
|---|---|---|
| `t3-workflow` | "how does the T3 workflow work" | The link protocol under every workflow thread, and the router to the other skills. |
| `setup-t3-workflow` | "set up the T3 workflow" | Picks the reviewer, passes, credential renewal, delivery and models, and writes the config. |
| `t3-overseer` | "work 41, 42 and 43 in parallel" | One root thread over several issues, with one list of the decisions you owe. |
| `t3-ticket` | "work on issue 42" | Splits an issue into PRs, runs one worker thread per PR, merges and deploys on your word. |
| `t3-worker` | nothing, `t3-ticket` launches it | Builds one PR, has it reviewed, follows it to merged, hands off. |

### 1. Get the skills

With [skills.sh](https://skills.sh):

```bash
npx skills@latest add OttoOps/t3-codemode
```

It asks which skills and which agents to install for. Pick all five skills,
then Claude Code, Codex or both. For the scope, pick Global, or pass `-g`.
The skills then go in `~/.agents/skills`, where Codex reads them. For Claude
Code it offers symlinks from `~/.claude/skills` into that folder; take them,
so both agents share one copy. Update with `npx skills@latest update`, and run
`add` again to pick up skills added since.

Or copy them by hand:

```bash
git clone https://github.com/OttoOps/t3-codemode
mkdir -p ~/.claude/skills ~/.codex/skills
cp -R t3-codemode/skills/* ~/.claude/skills/   # Claude Code
cp -R t3-codemode/skills/* ~/.codex/skills/    # Codex
```

`~/.agents/skills` works as the target too. Symlink the folders instead of
copying them if you want `git pull` to update them. Each skill is a folder,
so keep the folders whole.

### 2. Run the setup skill once

Ask your agent to "set up the T3 workflow", or run `/setup-t3-workflow`. It
finds what you have installed, asks about one slot at a time, and writes the
config. Run it again whenever you want to change a slot. A slot that no
config sets keeps its default: review by a `sol-loop` skill if one is
installed, no passes, delivery by merge.

### Which skills need which

Some skills call others. Install the ones they need too, in the same skills
folder, because a few read their neighbors' files by relative path:

- Every skill needs `t3-workflow`, whose scripts run the protocol and read
  the slots.
- `t3-overseer` launches `t3-ticket` threads.
- `t3-ticket` launches `t3-worker` threads and reads `t3-worker`'s procedure.
  For tag-deploy delivery it runs the deploy skill you name in setup.
- `setup-t3-workflow` uses `t3-workflow`'s scripts and docs.
- `t3-worker` uses the skills you put in its slots: a reviewer such as
  `sol-loop` from [t3dotgg/tstack](https://github.com/t3dotgg/tstack), passes
  such as `pr` from [mattpocock/skills](https://github.com/mattpocock/skills),
  and a credential renewal skill. Each is optional, and the worker skips a
  slot with no skill.

`t3cm` is optional, and the setup skill offers to install it when it is
missing.

## Contributing

Every commit carries a DCO signoff (`git commit -s`), and pull requests merge
with rebase. The [contribution guide](.github/CONTRIBUTING.md) has the rest. Report
vulnerabilities privately through the [security policy](.github/SECURITY.md).
Participation follows the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

MIT, see `LICENSE`.
