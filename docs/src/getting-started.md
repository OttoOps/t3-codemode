# Getting started

## Requirements

- Linux with systemd user services, macOS, or Windows (x86_64; Arm64 runs it
  under emulation).
- A running T3 Code server and its `t3` CLI.
- `curl` and `tar` for `deploy/install.sh`; Windows PowerShell 5.1 or newer for
  `deploy/install.ps1`.
- Claude Code, if you want the installer to register the MCP server for you.
- `gh`, signed in, if you want the installer to check build provenance.
- Rust 1.95 or newer, only to build from source. `oxc` is pinned to
  `=0.146.0` because later releases need a newer compiler.

## Install

The installers fetch the latest release from
[Releases](https://github.com/OttoOps/t3-codemode/releases), check the archive
against the release's `SHA256SUMS`, and, when `gh` is signed in, check its
build provenance with `gh attestation verify`. Then they install the binary and
a per-user service, start it, and register the MCP server with Claude Code.
Last, they ask `Use the T3 workflow? [Y/n]`, and on yes run
`t3cm skills install`, described [below](#install-the-skills-with-t3cm).
Nothing needs administrator rights. Release builds came in with
[#2](https://github.com/OttoOps/t3-codemode/issues/2).

### Linux and macOS

```bash
curl -fsSLO https://raw.githubusercontent.com/OttoOps/t3-codemode/main/deploy/install.sh
bash install.sh                     # latest release
bash install.sh --version v0.4.0    # a given release
```

On Linux the installer picks the static musl build, which runs on any distro.
`--target x86_64-unknown-linux-gnu` (or `aarch64-unknown-linux-gnu`) picks the
glibc build, which needs glibc 2.39 or newer. On a Mac it picks the Mac's own
architecture, under Rosetta too.

Releases sign the macOS binaries with a Developer ID and notarize them with
Apple, so Gatekeeper lets them run when a browser download quarantines the
archive. A bare binary can't carry a stapled ticket, so Gatekeeper fetches the
notarization ticket online the first time it checks one.
To check a binary yourself:

```bash
codesign --verify --strict --verbose=2 ~/.local/bin/t3cm
spctl --assess --type open --context context:primary-signature --verbose=2 ~/.local/bin/t3cm
```

The second command should print `source=Notarized Developer ID`.

On macOS, install from a GUI login: launchd loads the agent into your login
session. Over SSH alone, use `--files-only` and run the `launchctl bootstrap`
command it prints once you log in.

When no `v*` release is published yet, the default install stops and says so.
Build from a clone instead:

```bash
git clone https://github.com/OttoOps/t3-codemode && cd t3-codemode
deploy/install.sh --from-source
```

### Windows

In PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/OttoOps/t3-codemode/main/deploy/install.ps1 -OutFile install.ps1 -UseBasicParsing
powershell -ExecutionPolicy Bypass -File install.ps1                   # latest release
powershell -ExecutionPolicy Bypass -File install.ps1 -Version v0.4.0   # a given release
```

The Windows installer does not add `t3cm` to `PATH`, so run it by its full
path: `& "$env:LOCALAPPDATA\Programs\t3-codemode\t3cm.exe" status`. Windows has
no source install; it needs a published release.

### Other ways in

- `install.sh --archive t3cm-<version>-<target>.tar.gz --sums SHA256SUMS`
  (`install.ps1 -Archive t3cm-<version>-x86_64-pc-windows-msvc.zip -Sums SHA256SUMS`)
  installs an archive you already downloaded. It checks the checksum, not the
  provenance; run `gh attestation verify <archive> -R OttoOps/t3-codemode` for
  that.
- `deploy/install.sh --from-source`, from a clone, builds with
  `cargo build --release --locked` instead (Linux and macOS).
- `--files-only` (`-FilesOnly`) installs or removes the files without touching
  systemd, launchd or Task Scheduler, and prints the command that starts the
  service.
- `--workflow` and `--no-workflow` (`-Workflow`, `-NoWorkflow`) answer the
  workflow question for an unattended run. With no terminal to ask on, the
  answer is yes. When t3cm installed the skills before, the installer brings
  them to the new release without asking. When `npx skills` or a copy by hand
  installed them, it asks whether to replace them, yes by default, and on yes
  runs `t3cm skills install --take-over`.

### What it installs

| OS | Binary | Service | Logs |
|---|---|---|---|
| Linux | `~/.local/bin/t3cm` | systemd user unit `t3-codemode` (`deploy/t3-codemode.service`) | `journalctl --user -u t3-codemode` |
| macOS | `~/.local/bin/t3cm` | launchd agent `com.ottoops.t3-codemode` in `~/Library/LaunchAgents/` | `~/Library/Logs/t3-codemode/t3cm.log` |
| Windows | `%LOCALAPPDATA%\Programs\t3-codemode\t3cm.exe` | scheduled task `t3-codemode`, at logon | `%LOCALAPPDATA%\t3-codemode\logs\t3cm.log` |

Each runs `t3cm serve` on `127.0.0.1:47731` and restarts it when it fails. The
systemd unit also sets `LimitNOFILE=1048576`, `MemoryHigh=1G` and
`MemoryMax=1536M`. The Windows task runs as you, not elevated, starts at logon
and then every five minutes, and does nothing while t3cm is still running, so a
dead server comes back within five minutes. To stop it for good, run
`Disable-ScheduledTask t3-codemode` and then `Stop-ScheduledTask t3-codemode`.
When `~/.local/bin` is not on your `PATH`, the Linux and macOS installer says
so.

To update, run the installer again, or run `t3cm update`. Either one also
brings the skills t3cm installed to the new release. To uninstall, run
`bash install.sh --uninstall` or `install.ps1 -Uninstall`. It stops and removes
the service and the binary, and keeps the store and the skills.

Uninstalling also removes Claude Code's user-scope `t3cm` MCP entry, even one
you added yourself before installing.

## Register the MCP server

The installers run this for you. When `claude` is not on `PATH`, they print it
for you to run. When Claude Code already has a server named `t3cm`, they leave
that entry alone.

```bash
claude mcp add --transport http --scope user t3cm http://127.0.0.1:47731/mcp \
  --header 'X-T3-Thread: ${T3CODE_THREAD_ID}'
```

In PowerShell, run it on one line:

```powershell
claude mcp add --transport http --scope user t3cm http://127.0.0.1:47731/mcp --header 'X-T3-Thread: ${T3CODE_THREAD_ID}'
```

Keep the header in single quotes in either shell. Claude Code expands `${T3CODE_THREAD_ID}`
when it connects, so each T3 thread reaches the service as itself. That header
is how `mem`, `vars` and `wf` know which thread is calling.

## Check it

```bash
t3cm status
```

`t3cm status` prints the running service's `/healthz`. `t3.connected` is `true`
once the service holds a T3 session, and `tools` counts the T3 tools in the
catalog. Claude Code loads MCP servers at session start, so threads that were
already open keep their old tool list until they restart.

## Update

```bash
t3cm update          # install the latest release, and on Linux restart the service on it
t3cm update --check  # only say whether a newer release is out
```

`t3cm update` (or `t3cm upgrade`) finds the latest release, downloads this
machine's archive, and checks it against the release's `SHA256SUMS` and, with
`gh` signed in, its build provenance, as the installers do. It replaces the
binary in place and keeps the old one next to it as `t3cm.old`.

On Linux, when the systemd service is running, `t3cm update` restarts it.
Runs in flight finish first, up to systemd's stop timeout, and the board
reconnects by itself. It then waits for `/healthz` to report the new version.
If the service stops or does not report it within 45 seconds, `t3cm update`
puts the old binary back, restarts the service on it, and exits non-zero. Once
the new version serves, it deletes `t3cm.old`.

When t3cm installed the workflow skills, `t3cm update` then runs the new
binary's `t3cm skills update`, so the skills and their setup steps move to the
new release with the binary. An update that finds t3cm already current still
runs it when the skills or a setup step have drifted. When that sync fails, the
new binary stays, `t3cm update` exits non-zero, and it names the command to run
once you fix the cause. It leaves skills that `npx skills` installed alone and
prints the `t3cm skills install --take-over` command that replaces them.

`t3cm update` never edits the systemd unit. When a release ships a changed
unit, it saves the new one as `t3-codemode.service` in the state directory and
prints the command that installs it. Keep local settings in a drop-in
(`systemctl --user edit t3-codemode`) so that you can copy a new unit over
without losing them.

On macOS and Windows, `t3cm update` replaces the binary and prints the command
that restarts the service on it, along with the one that goes back to
`t3cm.old`. On macOS that is `launchctl kickstart -k`, and on Windows the
scheduled task's `/End` and `/Run`; both end runs in flight. Rerun the
installer to pick up a changed launchd agent or task.

Only one `t3cm update` runs at a time, and it changes nothing while a service
answers with a version other than the binary on disk, for example after an
update with `--no-restart` that the service has not restarted onto yet.

`--version vX.Y.Z` installs that release instead, older ones included, and
`--no-restart` replaces the binary but leaves the running service alone, and
keeps `t3cm.old`. Run
it as the installed binary (`~/.local/bin/t3cm`, or
`%LOCALAPPDATA%\Programs\t3-codemode\t3cm.exe`). A build from a checkout
updates with `deploy/install.sh --from-source` instead.

`t3cm --version`, `t3cm --help` and a bare `t3cm` end with a notice when a
newer release is out. The service checks every 12 hours, the CLI at most once
a day, and both share the answer in `update-check.json` in the state directory.
The notice goes to stderr, and only when stderr is a terminal, so scripts that
read `t3cm --version` see no change. The board shows the same news in its
header; see [The board](board.md#updates). Set `updateCheck` to `false` in the
config, or `T3CM_NO_UPDATE_CHECK=1`, to turn the checks off. They are also off
when `CI` is set.

## Commands

| Command | Does |
|---|---|
| `t3cm serve [--port N] [--state-dir D] [--no-watcher] [--log-file F]` | MCP Streamable HTTP at `/mcp`, `/healthz`, the board at `/` |
| `t3cm stdio [--state-dir D] [--watcher]` | MCP over stdio; the caller is `$T3CODE_THREAD_ID` |
| `t3cm status` | The running service's `/healthz` |
| `t3cm exec <file\|->`, `t3cm search <file\|->` | Call the running service's tools, for humans and scripts |
| `t3cm auth` | Acquire a fresh T3 session now; prints only its expiry |
| `t3cm update [--check] [--version vX.Y.Z] [--no-restart]` | Install the latest (or a given) release in place, and on Linux restart the service on it; see [Update](#update) |
| `t3cm types [--from tools.json]` | Print the generated TypeScript declarations and their size |
| `t3cm skills install\|update\|status` | Install the workflow skills this binary carries, bring them to its release, or check them ([below](#install-the-skills-with-t3cm)) |

`--port` also reads `T3CM_PORT` (default `47731`), and `--state-dir` reads
`T3CM_STATE_DIR`. `--log-file` appends the log to a file instead of stderr and
rolls it to `<file>.1` at start once it passes 10 MB; the macOS and Windows
services use it. `t3cm exec` takes `--session`, `--timeout-ms` and `--thread`
(default `$T3CODE_THREAD_ID`), and exits non-zero when the script fails.

```bash
echo 'async () => (await t3.thread_list({ allProjects: true, limit: 5 })).threads.map(t => t.title)' | t3cm exec -
```

## The workflow skills

Agent skills drive the thread-workflow layer. This repo carries five under
`skills/`: `t3-workflow`, `setup-t3-workflow`, `t3-overseer`, `t3-ticket` and
`t3-worker`. `t3-workflow` holds the link protocol and routes to the other
four. The installers are the only way to get them. They put them in place
when you answer yes to the workflow question, and `t3cm update` keeps them
current. On a machine where t3cm is already installed, `t3cm skills install`
adds them.

### Install the skills with t3cm

Each t3cm binary carries the skills from the commit it was built from, so a
release build and a `--from-source` build both have them:

```bash
t3cm skills install
t3cm skills status
```

`install` writes them to `~/.agents/skills`. It stamps the release into each
`SKILL.md` under `metadata.t3-codemode-release`, and records the release,
every file and its SHA-256 in `~/.config/t3-workflow/installed.json` (under
`$XDG_CONFIG_HOME` when that is set). `t3cm skills update` brings the skills
to the binary's release. It writes the files that changed, removes files the
release dropped, and keeps any file you edited since t3cm wrote it, with a
warning; `--force` overwrites those too. `status` lists each skill as current,
out of date, edited locally or missing, names the command that fixes it, and
exits 1 unless every skill is current. `--json` prints either result as JSON.

Codex reads `~/.agents/skills`. Claude Code reads `~/.claude/skills`, or
`$CLAUDE_CONFIG_DIR/skills` when that is set, so when that config folder
exists, `install` and `update` also link each skill there to its folder in
`~/.agents/skills`. They never replace a folder or a link already in a skill's
place, and `status` mentions one that is in an `info:` line, and in
`claudeLinks` with `--json`, without counting it against the skills being
current. Where the account cannot make links, as on Windows without Developer
Mode, they print the copy commands instead.

When `npx skills` (or a copy by hand) installed the skills on an older
machine, t3cm has no manifest for them. `status` reports that install, the
commit in `~/.agents/.skill-lock.json`, and how many files differ from this
release. `install` and `update` leave those skills alone until you run
`t3cm skills install --take-over`, which the installers offer, yes by default.
That replaces their files with this release's and removes them from the
`npx skills` lock file, so `npx skills update` no longer overwrites them. t3cm
never writes through a skill folder that is a link, such as one into a git
clone.

`install` and `update` also apply the setup steps that need no answers,
which this release lists in `setup-t3-workflow/steps.json`. The first is the
filed-issue rule: t3cm writes it for Claude Code
(`rules/t3-workflow-filed-issues.md` in `$CLAUDE_CONFIG_DIR`, else
`~/.claude`) and as a marked block in Codex's global `AGENTS.md`
(`$CODEX_HOME`, else `~/.codex`), skipping an agent with no config folder.
When `~/.config/t3-workflow/config.json` exists, t3cm records each step it
applied there under `steps`, with its release under `writtenBy`. A step that
needs answers, such as a slot a later release added, prints as
`run setup-t3-workflow: new slot <slot>`; ask your agent to run that skill.
`status` lists the missing steps too, and exits 1 while a step t3cm can
apply is missing.

The running service checks the same things. `/healthz`, `wf.status()` and the
board's `/api/state` (under `service`) carry a `skills` object:

| Field | Meaning |
| --- | --- |
| `release` | The release this t3cm binary carries |
| `installed` | The release `installed.json` records, or `null` when t3cm did not install the skills |
| `present` | `false` when no skills are installed at all; the board then shows nothing |
| `current` | `true` when `t3cm skills status` would exit 0 |
| `fix`, `foreign` | As in `t3cm skills status --json` |
| `missingSteps` | The setup steps not applied or recorded, each with its `id`, `kind`, `since`, `slot` and `fix` |
| `configState`, `problem` | Whether `config.json` is absent, legacy (no `steps`), recorded, or ignored, and why |
| `fixes` | The commands to run, in order |
| `error` | Set when the check itself failed |

The service reads the files at startup and on each watcher pass and keeps
the result, so board updates and `/healthz` cost no file reads.
`wf.status()` reads them fresh.

### Set up the workflow

After installing the skills, ask your agent once to "set up the
T3 workflow". Setup also installs the rule that has every thread report the
issues it files to the [board](board.md#new-issues). The
[README](https://github.com/OttoOps/t3-codemode#install-the-workflow-skills)
covers which skills need which. Without the skills,
launched threads do not know the workflow, so the board and `wf` are the part
of this project that works standalone.
