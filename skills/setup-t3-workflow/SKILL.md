---
name: setup-t3-workflow
description: Set up or change the T3 workflow's slots - the reviewer that checks a PR before it opens, the worker's passes, credential renewal, delivery (merge or tag-deploy) and the ticket and worker models - by detecting what is installed and interviewing the user one slot at a time, then writing the config. Use when the user asks to set up the T3 workflow, change the reviewer, use a different model for workers or tickets, add a cleanup or voice pass, renew a credential, or deploy with tags, and when a role reports no T3 workflow config yet. Safe to run again to change one slot.
---

The T3 workflow's roles name capabilities, not skills. Each **slot** holds
one; this skill fills them for this user. What each slot means, the config
shape and how settings layer:
[slots.md](../t3-workflow/slots.md).

`setup` below means
`python3 <this skill's base directory>/scripts/setup_workflow.py`. It needs
the `t3-workflow` skill installed beside this one.

## 1. Detect

```bash
setup detect --skills '<every skill this session lists, comma separated>'
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
- `platform`, and `config`: its path, whether it exists, its `problems`, and
  `repoProblems` from the repository's `AGENTS.md` block, which the user
  fixes in that file.

When `mcp__t3cm__execute` is listed, also read t3cm's pools, the groups of
provider instances a launch can be spread across:

```js
async () => (await wf.pools()).map(p => ({ pool: p.pool, model: p.model, effort: p.effort,
  error: p.error, members: p.members.map(m => `${m.instanceId} ${m.state}`) }))
```

Without it, there are no pools to offer.

Show the user the `summary` lines, then one line each for the providers,
the pools and t3cm. When `config.problems` is not empty, the roles ignore the whole file
until it is fixed. Show the user the problems and the file. Fix the entry each
problem names when the user agrees. A fresh file (step 3, `--replace`) drops
every setting in the old one, so before using it, list what it drops and
carry into the write each setting the user wants to keep.

## 2. Interview

Ask about one slot per question, in this order: review, passes, credentials,
delivery, model ticket, model worker. Use the session's question tool when
it has one; otherwise ask in chat. Wait for each answer before the next
question. When the user asked to change one slot ("change the reviewer"),
ask about that slot alone.

What to offer, suggest and explain for each slot, and the JSON each answer
records: [interview.md](references/interview.md). Three rules hold for every
slot:

- **Several skills for one name** (`found.<name>` lists more than one
  `name`): ask which one, and record its full `name`.
- **Slash-only skill** (`invoke: inline`): record it like any other. Tell
  the user the roles read its `SKILL.md` and apply it inline, because the
  Skill tool refuses it.
- **Keep current**: a slot the user keeps stays out of the write.

## 3. Write

Gather the answers into one JSON object with only the slots that changed,
and write it, with the same `--repo` as step 1 when you passed one:

```bash
setup write --skills '<same list as step 1>' <<'JSON'
{"review": "cbrip:sol-loop", "passes": ["unslop", "pr"]}
JSON
```

It merges into the user config at `config.path` (`models` by role), checks
the result, and prints the new `t3wf slots` lines. `--unset <slot>` (or
`--unset models.worker`) returns a slot to its default. `--replace` starts
from an empty file.

- **`not written`**: fix what each line names and run it again. A bare name
  that matches several skills means asking the user which one.
- **`warn: ... not installed yet`**: written anyway; tell the user where to
  get the skill, and that the roles skip it until it is installed.

Show the user the printed slot lines as the summary. A slot whose line ends
in `[AGENTS.md]` is set by that repository's "Agent workflow" block, which
wins over the user config there; tell the user, and offer to edit the block
when they want the change in that repository.

## 4. Install the filed-issue rule

Every thread on the machine reports the GitHub issues it files to the t3cm
board, whether or not it runs a T3 role skill
([Filed issues](../t3-workflow/SKILL.md#filed-issues)). Step 1's `rule`
says where that rule stands for each agent. When either agent's `status`
is `missing` or `stale`, install it, on a one-slot change too:

```bash
setup rule
```

It writes the rule to `rules/t3-workflow-filed-issues.md` in the Claude
Code config folder and as a marked block in Codex's global `AGENTS.md`
(`AGENTS.override.md` when that file exists), and leaves the rest of that
file as it was. An agent with no config folder is skipped. Tell the user in
one line what it wrote; new sessions load it.

## 5. Offer t3cm

t3cm runs the same protocol as `t3wf` through code mode, with fewer tokens
and a board in the browser. Act on `t3cm` from step 1:

- **`running` and `connected`**: nothing to do. When this session lists no
  `mcp__t3cm__execute`, say new sessions pick it up.
- **`binary` but not `running`**: show the user `t3cm.error`. A bad
  `T3CM_PORT` needs that variable fixed, and "did not answer like t3cm" means
  another service holds the port. Otherwise, when `platform.systemdUser` is
  true, offer `systemctl --user start t3-codemode`; without it, say t3cm needs
  systemd user services.
- **`running`, not `connected`**: the service cannot reach T3 Code; point
  the user at the board at `t3cm.url` for its last error.
- **No `binary`**: offer to install it, once. On yes, follow
  [Installing t3cm](../t3-workflow/codemode.md#installing-t3cm).

Setup is done when the config holds every answer (or the user kept every
slot), the user has seen the summary, the filed-issue rule is current for
each installed agent, and t3cm is running or was offered.
