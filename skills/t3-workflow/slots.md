# Slots

The role skills name capabilities, not skills. Each **slot** holds one, and
`t3wf` resolves it to whatever this machine has installed.

| Slot | Holds | Default |
|---|---|---|
| `review` | the independent review a worker runs before it opens a PR | a found `sol-loop`, else none |
| `passes` | cleanup and templating skills a worker runs, in order | none |
| `credentials` | renewals: a failure pattern and the skill that renews it | none |
| `delivery` | `merge`, or `tag-deploy` with a deploy skill | `merge` |
| `models` | `ticket` and `worker` thread models, set separately, each a model or a t3cm pool | `claude-opus-5-5`, effort `high`, when T3 confirms it offers Claude; else the provider's default |

A slot with no setting uses its default, so a machine with no config at all
works.

## Reading slots in a role

A role reads its slots when it starts and again on every resume, because a
skill can be installed or removed mid-ticket:

```bash
t3wf slots --skills '<every skill this session lists, comma separated>'
```

Run it through the `t3wf` script in code mode too: t3cm has no `wf` route
for slots. Each line is one slot, `<slot>: <setting> [<source layer>]`; add
`--json` for the same data as fields. Act on each skill entry by its form:

| Entry | Do |
|---|---|
| `<name> (Skill tool)` | invoke `<name>` with the Skill tool |
| `<name> (inline: read <path> and apply it)` | read that `SKILL.md` and apply it step by step, as if invoked; the Skill tool refuses it |
| `SKIP <name>: no longer resolves` | **skip** it (below) |
| `none` | the slot is off |

A `PROBLEM:` on a line (a tag-deploy with no deploy skill, a model T3 does
not offer) is a skip too, and that slot falls back to its default. A
`config problem (ignored)` line is one note: the file or block it names is
ignored, and the layers below it decide.

A model line reads `<model> effort <effort> on <provider>`, each part present
only when set, `provider default`, or `pool <name>`. A pool line needs t3cm at
launch: [A pool on a model line](#a-pool-on-a-model-line).

**Skip** means carry on without that skill, and say so once per skill per
run, in one note to the parent:
`t3wf send --kind note --body 'slot <slot>: <name> skipped, <reason>'`. At
the root, tell the user that line. A skipped slot never opens a gate,
raises a blocker, or stops the ticket; `t3wf doctor` is where the user
repairs it.

**No config.** When the first line says `config: none (defaults)`, every
slot runs on its default. The overseer and the ticket each tell the user
once per run: "No T3 workflow config yet, so every slot uses its default.
Run the `setup-t3-workflow` skill to pick a reviewer, passes, delivery and
models." A ticket with a parent sends that line up as a note.

### A pool on a model line

`model worker: pool <name>` (or `model ticket`) means t3cm picks the T3
provider instance for each launch from the pool `<name>` in the t3cm config,
so threads spread across several accounts of one provider. The pool sets the
model and effort. `t3wf` cannot read t3cm's pools, so it prints the line
without checking it; the pick checks it.

- **t3cm listed** (`mcp__t3cm__execute`): just before the launch, run one
  `execute` with `async () => wf.pick({ pool: "<name>" })`. Pass its
  `modelSelection` to `t3_thread_launch` as it is, and compare the launched
  thread's model against it. Pick once per launch, and never reuse an
  earlier pick: each pick moves the pool's balance. Tell the parent (at the
  root, the user) the picked `instanceId` and its `reason` in the line that
  reports the launch.
- **The pick throws** (no member can run; the message names the pool and
  why each member is out): launch nothing. Send the message as a gate with
  the action line `reply "retry" once a member is back, or "default" to
  launch on the provider default`. On "default", launch without
  `modelSelection`.
- **No t3cm:** skip the pool, as for a `PROBLEM:`: send
  `slot model <role>: pool <name> skipped, needs t3cm`, and launch without
  `modelSelection`, on the provider default. Never guess a member from the
  pool's name or from `orchestrator_capabilities`.

## Where settings live

The repository's setting wins, slot by slot. `delivery` and `models` merge
field by field, so a repo can change the deploy skill alone.

1. The defaults above.
2. The user config file: `$T3WF_CONFIG`, else
   `$XDG_CONFIG_HOME/t3-workflow/config.json` (by default
   `~/.config/t3-workflow/config.json`).
3. The repository's `AGENTS.md` "Agent workflow" block.

The config file:

```json
{
  "version": 1,
  "review": "cbrip:sol-loop",
  "passes": ["unslop", "pr"],
  "credentials": [{"match": "ExpiredToken|Token has expired", "skill": "aws-sso-login"}],
  "delivery": {"mode": "merge"},
  "models": {
    "ticket": {"provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"},
    "worker": {"model": "claude-opus-5-5", "effort": "high"}
  }
}
```

A role set to a pool holds `pool` alone, such as
`"worker": {"pool": "opus"}`. The pool's model and effort come from the t3cm
config.

`"review": null` turns review off; leaving `review` out keeps the default.
`match` is a Python regular expression tried against the failing command's
output.

The `AGENTS.md` block reads these bullets and ignores the rest:

```markdown
## Agent workflow

- Review: `cbrip:sol-loop` (or none)
- Passes: `unslop`, `pr` (or none)
- Credential renewal: `ExpiredToken` -> `aws-sso-login`; `HTTP 401` -> `gh-login`
- Delivery: tag-deploy
- Deploy skill: `deploy`
- Ticket model: claude-opus-5-5, high effort
- Worker model: claude-opus-5-5, high effort (or pool `opus`)
```

## Finding skills

A slot names a skill by its bare name (`sol-loop`) or its full name
(`cbrip:sol-loop`). A bare name matches in any namespace. The search order:

1. the skills listed in this session (`--skills`);
2. `~/.claude/skills` (or `$CLAUDE_CONFIG_DIR/skills`);
3. `~/.agents/skills`;
4. the project's `.claude/skills`;
5. `~/.codex/skills` (or `$CODEX_HOME/skills`);
6. the Claude plugin cache, then the Codex plugin cache, as `plugin:name`.

A full name matches only itself, so a recorded full name always wins. A skill
installed with no namespace has its bare name as its full name: `sol-loop`
picks tstack's bare `sol-loop` even when `cbrip:sol-loop` is installed too.
A bare name that matches only namespaced skills uses the first and lists the
others. A
skill whose frontmatter sets `disable-model-invocation` resolves as
**inline**: read its `SKILL.md` at the printed path and apply it step by
step, because the Skill tool refuses it.

## Commands

| Command | Use |
|---|---|
| `t3wf slots [--skills <names>]` | at run time: each slot's effective setting, its source layer, and how to invoke its skill ([Reading slots in a role](#reading-slots-in-a-role)) |
| `t3wf resolve <name>` | every installed skill a name matches, in search order |
| `t3wf doctor` | every configured skill still resolves, models exist in T3, delivery is complete; exits 1 on a problem |
| `t3wf check-config` | the config file's and the `AGENTS.md` block's shape, offline |

`slots`, `resolve` and `doctor` take `--repo <dir>` (default: the current
directory's repository) and `--json`. `slots` and `doctor` check models
through T3 when it is reachable; `--no-t3` skips that.
