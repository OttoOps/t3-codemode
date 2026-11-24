---
name: setup-t3-workflow
description: Set up or change the T3 workflow's slots - the reviewer that checks a PR before it opens, the worker's passes, credential renewal, delivery (merge, tag-deploy, or multi-step pipelines in .t3-workflow.json), the ticket and worker models, project flow, and remote board access - by detecting what is installed and interviewing the user one slot at a time, then writing the config. Use when the user asks to set up the T3 workflow, change the reviewer, use a different model for workers or tickets, add a cleanup or voice pass, renew a credential, deploy with tags, or configure multi-stage delivery or .t3-workflow.json, configure project flow or auto-launching issues, open the board from a phone or another device, and when a role reports no T3 workflow config yet. Safe to run again to change one slot.
---

The T3 workflow's roles name capabilities, not skills. Each **slot** holds
one; this skill fills them for this user. What each slot means, the config
shape and how settings layer:
[slots.md](../t3-workflow/slots.md).

Before step 1, make sure `t3cm` is installed and has `setup` (`t3cm setup --help`,
or `~/.local/bin/t3cm setup --help` when `~/.local/bin` is not on `PATH`):

- **No `t3cm` binary:** install it first:
  [Installing t3cm](../t3-workflow/codemode.md#installing-t3cm).
- **Older `t3cm` without `setup`:** run `t3cm update` first, then continue.

## 1. Detect

```bash
t3cm setup detect
```

Add `--repo <dir>` when the user means a repository other than the current
one. The JSON it prints is the interview's only source of facts:

- `summary`: the current settings, one line per slot, with the layer each
  comes from in brackets.
- `slots.<slot>.found`: installed candidates by the name searched, each with
  its full `name`, `path` and `invoke` (`skill`, or `inline` for a slash-only
  skill). `missing`: names searched and not found; `installedInstead` lists
  what is installed under the same bare name. `slots.review.suggest` appears
  when no `sol-loop` is installed.
- `providers.enabled`: each T3 provider and its models with their effort
  values; `providers.error` when T3 did not answer.
- `t3cm`: the binary, and whether the service runs and reaches T3.
- `rule`: the filed-issue rule for `claude` and `codex`, each with its
  `path` and a `status` of `current`, `stale`, `missing` or `no agent`;
  `rule.error` in their place when the rule could not be checked, which you
  show the user.
- `steps`: the setup steps `recorded` in the config, and those `missing`,
  each with its `id`, its `kind` (`interview`, naming a `slot`, or `auto`)
  and, for an interview step, the slot setup has not asked about yet.
- `platform`, and `config`: its `path`, whether it `exists`, its `repoPath`
  (`.t3-workflow.json`) and `repoExists`, its `problems`, and `repoProblems`
  from the repository's `AGENTS.md` block and `.t3-workflow.json`, which
  the user fixes in those files.

Also read t3cm's pools (the groups of provider instances a launch can be
spread across), through `mcp__t3cm__execute` or `t3cm exec -`:

```js
async () => (await wf.pools()).map(p => ({ pool: p.pool, model: p.model, effort: p.effort,
  error: p.error, members: p.members.map(m => `${m.instanceId} ${m.state}`) }))
```

When the service is not running yet, note that pools will be available once
it starts.

Show the user the `summary` lines, then one line each for the providers,
the pools and t3cm. When `config.problems` is not empty, the roles ignore the whole file
until it is fixed. Show the user the problems and the file. Fix the entry each
problem names when the user agrees. A fresh file (step 3, `--replace`) drops
every setting in the old one, so before using it, list what it drops and
carry into the write each setting the user wants to keep.

## 2. Interview

Ask about one slot per question, in this order: review, passes, credentials,
delivery, model ticket, model worker, flow, remote board. Use the session's question tool when
it has one; otherwise ask in chat. Wait for each answer before the next
question. When the user asked to change one slot ("change the reviewer"),
ask about that slot alone. When the config exists and `steps.missing` names
interview steps, a later release added those slots: ask about each of them,
as well as any slot the user asked to change.

What to offer, suggest and explain for each slot, and the JSON each answer
records: [interview.md](references/interview.md). Three rules hold for every
slot:

- **Several skills for one name** (`found.<name>` lists more than one
  `name`): ask which one, and record its full `name`.
- **Slash-only skill** (`invoke: inline`): record it like any other. Tell
  the user the roles read its `SKILL.md` and apply it inline, because the
  Skill tool refuses it.
- **Keep current**: a slot the user keeps stays out of the JSON, and goes
  in `--asked` (step 3).

Flow and remote board live in t3cm's own `config.json` and are carried out
during the interview: `flow` writes and reloads through `t3cm config` with no
restart, `remote board` runs t3cm's board commands, restarts the service and
checks the result, and the step 3 write records only `--asked flow` and
`--asked remote`.

## 3. Write

Gather the answers into one JSON object with only the slots that changed,
and write it, with the same `--repo` as step 1 when you passed one:

```bash
t3cm setup write <<'JSON'
{"review": "cbrip:sol-loop", "passes": ["unslop", "pr"]}
JSON
```

Pass `--asked <slot>` for each slot you asked about, the kept ones too, such
as `--asked review,passes,credentials,delivery,models,flow,remote` after a full
interview, and write `{}` when every slot was kept. The write records those
slots as done setup steps, so a later release names only the slots it adds.

It merges into the user config at `config.path` (`models` by role), checks
the result, and prints the new `t3cm slots` lines. `--unset <slot>` (or
`--unset models.worker`) returns a slot to its default. `--replace` starts
from an empty file.

To write repository-level settings (such as a multi-step `delivery.steps`
pipeline) to the repository's `.t3-workflow.json` instead of the user
config, pass `--repo-config` (with the same `--repo <dir>` when targeting
another repository, and without `--asked`, which applies only to the user
config):

```bash
t3cm setup write --repo-config <<'JSON'
{"delivery": {"steps": [{"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"}, {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "ticket", "watch": true}], "closeAfter": "ticket"}}
JSON
```

`--repo-config` validates the file, merges into `config.repoPath`, omits
per-machine `steps` and `writtenBy`, and prints the new `t3cm slots` lines.
When the user sets a repo-level delivery pipeline in `.t3-workflow.json`
during a full interview, still write the user config (passing `--asked` for
the interviewed slots) and run the `--repo-config` write as a second
command.

- **`not written`**: fix what each line names and run it again. A bare name
  that matches several skills means asking the user which one.
- **`warn: ... not installed yet`**: written anyway; tell the user where to
  get the skill, and that the roles skip it until it is installed.

Show the user the printed slot lines as the summary. A slot whose line ends
in `[.t3-workflow.json]` or `[AGENTS.md]` is set by that repository, which
wins over the user config there (`.t3-workflow.json` wins over `AGENTS.md`);
tell the user, and offer to update `.t3-workflow.json` (with
`--repo-config`) or the `AGENTS.md` block when they want the change in that
repository.

## 4. Install the filed-issue rule

Every thread on the machine reports the GitHub issues it files to the t3cm
board, whether or not it runs a T3 role skill
([Filed issues](../t3-workflow/SKILL.md#filed-issues)). Step 1's `rule`
says where that rule stands for each agent. When either agent's `status`
is `missing` or `stale`, install it, on a one-slot change too:

```bash
t3cm setup rule
```

It writes the rule to `rules/t3-workflow-filed-issues.md` in the Claude
Code config folder and as a marked block in Codex's global `AGENTS.md`
(`AGENTS.override.md` when that file exists), and leaves the rest of that
file as it was. An agent with no config folder is skipped. It records the
step in the config when the config exists. Tell the user in one line what it
wrote; new sessions load it. `t3cm skills install` and `t3cm skills update`
install the same rule the same way.

## 5. Check t3cm

`t3cm` is required: it runs the link protocol, resolves every slot, and
serves the board. Act on `t3cm` from step 1:

- **`running` and `connected`**: nothing to do. When this Claude Code session
  lists no `mcp__t3cm__execute`, say new sessions pick it up (and this
  session, like Codex, reaches it through `t3cm exec -`).
- **`binary` but not `running`**: show the user `t3cm.error`. A bad
  `T3CM_PORT` needs that variable fixed, and "did not answer like t3cm" means
  another service holds the port. Otherwise, give the start command from
  [When the service is down](../t3-workflow/codemode.md#when-the-service-is-down).
- **`running`, not `connected`**: the service cannot reach T3 Code; point
  the user at the board at `t3cm.url` for its last error.
- **No `binary`**: install it with the user's go-ahead:
  [Installing t3cm](../t3-workflow/codemode.md#installing-t3cm).

Setup is done when the config holds every answer and records every slot
asked about, the user has seen the summary, the filed-issue rule is current for
each installed agent, any flow change has saved and reloaded (or the user has
the restart command when the service did not reload), a remote board the user
turned on or off has passed its check, and `t3cm` is running and connected (or
the user has the command to start or install it).
