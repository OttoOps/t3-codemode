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

To update, run the installer again. To uninstall, run `bash install.sh
--uninstall` or `install.ps1 -Uninstall`. It stops and removes the service and
the binary, and keeps the store.

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

## Commands

| Command | Does |
|---|---|
| `t3cm serve [--port N] [--state-dir D] [--no-watcher] [--log-file F]` | MCP Streamable HTTP at `/mcp`, `/healthz`, the board at `/` |
| `t3cm stdio [--state-dir D] [--watcher]` | MCP over stdio; the caller is `$T3CODE_THREAD_ID` |
| `t3cm status` | The running service's `/healthz` |
| `t3cm exec <file\|->`, `t3cm search <file\|->` | Call the running service's tools, for humans and scripts |
| `t3cm auth` | Acquire a fresh T3 session now; prints only its expiry |
| `t3cm types [--from tools.json]` | Print the generated TypeScript declarations and their size |

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
four. They install as agent skills, apart from the service, for Claude Code
and Codex:

```bash
npx skills@latest add OttoOps/t3-codemode -g
```

Then ask your agent once to "set up the T3 workflow". Setup also installs
the rule that has every thread report the issues it files to the
[board](board.md#new-issues). The
[README](https://github.com/OttoOps/t3-codemode#install-the-workflow-skills)
covers the manual copy and which skills need which. Without the skills,
launched threads do not know the workflow, so the board and `wf` are the part
of this project that works standalone.
