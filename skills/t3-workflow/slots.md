# Slots

The role skills name capabilities, not skills. Each **slot** holds one, and
the `t3cm` service resolves it to whatever the calling agent has installed
and enabled.

| Slot | Holds | Default |
|---|---|---|
| `review` | the independent review a worker runs before it opens a PR | a found `sol-loop`, else none |
| `passes` | cleanup and templating skills a worker runs, in order | none |
| `credentials` | renewals: a failure pattern and the skill that renews it | none |
| `delivery` | `merge`, or `tag-deploy` with a deploy skill | `merge` |
| `models` | `ticket` and `worker` thread models, set separately, each a model or a t3cm pool | `claude-opus-5-5`, effort `high`, when T3 confirms it offers Claude; else the provider's default |

A slot with no setting uses its default, so a machine with no config at all
works.

## Looking up slots in a role

Roles do not bulk-read all slots at start or on resume. Each step looks up
only the slot it needs, right before it uses it, through `execute` (or
`t3cm exec -` when `mcp__t3cm__execute` is not listed):

| Step | Call |
|---|---|
| ticket, choosing delivery and gate wording | `await wf.slot("delivery")` |
| ticket or overseer, launching a thread | `await wf.slot("models", { repo? })` |
| worker, before opening the PR | `await wf.slot("review")` |
| worker, publishing | `await wf.slot("passes")` |
| any role, a command fails | `await wf.renewal({ output, repo? })` |

Pass `{ repo: "<workspace root>" }` when the repository differs from this
thread's project root (for example, an overseer checking another repo, or a
worker reading its worktree's `AGENTS.md`).

`wf.slot(name)` returns the slot's resolved fields, its `source` layer, a
one-line `line`, and `warnings`. Each skill object carries `status` (`"ok"`,
`"ambiguous"`, or `"missing"`), `name`, `path`, `invoke` (`"skill"` or
`"inline"`), `others` (when `"ambiguous"`), and `how`. `"ambiguous"` still
returns a usable first match; act on each skill object by its fields:

| Fields | Do |
|---|---|
| `status: "ok"` or `"ambiguous"`, `invoke: "skill"` (`how: "Skill tool"`) | invoke `name` with the Skill tool |
| `status: "ok"` or `"ambiguous"`, `invoke: "inline"` (`how: "inline: read <path> and apply it"`) | read `path` and apply it step by step, as if invoked; the Skill tool refuses it |
| `status: "missing"` | **skip** it (below) |
| `skill: null` (or `skills: []`) | the slot is off |

A `problem` field on a slot (a tag-deploy with no deploy skill, a model T3
does not offer) is a skip too, and that slot falls back to its default. Any
item in `warnings` (such as an ignored config problem or a missing
`X-T3-Config-Dir` header) goes in one note to the parent (at the root, to
the user).

**Skip** means carry on without that skill, and say so once per skill per
run, in one note to the parent:
`await wf.send({ kind: "note", body: "slot <slot>: <name> skipped, <reason>" })`.
At the root, tell the user that line. A skipped slot never opens a gate,
raises a blocker, or stops the ticket; `t3cm doctor` is where the user
repairs it.

**No config.** When a slot's `source` is `"default"` and `t3cm` reports no
user config (for example in `wf.slots().config`), every slot runs on its
default. Tell the user once per run at the root: "No T3 workflow config yet,
so every slot uses its default. Run the `setup-t3-workflow` skill to pick a
reviewer, passes, delivery and models." A ticket with a parent sends that
line up as a note.

### A pool on a model slot

When `wf.slot("models")` returns a role whose `spec.pool` is set (and whose
`modelSelection` is `{ pool: "<name>", pick: ... }`), t3cm picks the T3
provider instance for each launch from the pool `<name>` in the t3cm config,
so threads spread across several accounts. The pool sets the model and
effort; in a mixed pool, each member sets its own. `t3cm doctor` checks that
every configured pool exists in t3cm's config.

- Just before the launch, call `await wf.pick({ pool: "<name>" })`. Pass its
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
  },
  "writtenBy": "0.7.0",
  "steps": ["slot:review", "slot:passes", "slot:credentials", "slot:delivery", "slot:models", "rule:filed-issues"]
}
```

A role set to a pool holds `pool` alone, such as
`"worker": {"pool": "opus"}`. The pool's model and effort come from the t3cm
config.

`"review": null` turns review off; leaving `review` out keeps the default.
`match` is a regular expression (with lookaround and backreferences, matching
Python's syntax) tried against the failing command's output by `wf.renewal`;
`t3cm doctor` flags any pattern t3cm cannot compile.

Setup and t3cm write the last two keys themselves. `writtenBy` is the release
that last wrote the file. `steps` lists the setup steps done on this machine,
by the ids in `setup-t3-workflow/steps.json`. That file lists each release's
steps: an `interview` step is a slot setup asks about, and an `auto` step
needs no answers, so `t3cm skills install` and `update` apply it. A file with
no `steps` was written before setup recorded them, so every step marked
`legacy` counts as done.

The `AGENTS.md` block reads these bullets and ignores the rest:

```markdown
## Agent workflow

- Review: `cbrip:sol-loop` (or none)
- Passes: `unslop`, `pr` (or none)
- Credential renewal: `ExpiredToken` -> `aws-sso-login`; `HTTP 401` -> `gh-login`
- Delivery: tag-deploy
- Deploy skill: `deploy`
- Ticket model: claude-opus-5-5, high effort
- Worker model: pool `opus`
```

## Finding skills

A slot names a skill by its bare name (`sol-loop`) or its full name
(`cbrip:sol-loop`). A bare name matches in any namespace. The service reads
each agent's reported config folder (`X-T3-Config-Dir`, or `t3cm exec`'s
environment) and applies its `settings.json` `enabledPlugins`, so disabled
plugins never resolve. The search order:

1. `~/.claude/skills` (or `$CLAUDE_CONFIG_DIR/skills`);
2. `~/.agents/skills`;
3. the project's `.claude/skills`;
4. `~/.codex/skills` (or `$CODEX_HOME/skills`);
5. `~/.gemini/config/skills` (or `$GEMINI_HOME/config/skills`), where
   Antigravity reads skills;
6. the Claude plugin cache (enabled plugins only), then the Codex plugin
   cache, as `plugin:name`.

A full name matches only itself, so a recorded full name always wins. A skill
installed with no namespace has its bare name as its full name: `sol-loop`
picks tstack's bare `sol-loop` even when `cbrip:sol-loop` is installed too.
A bare name that matches only namespaced skills uses the first and lists the
others. A skill whose frontmatter sets `disable-model-invocation` resolves
as **inline**: read its `SKILL.md` at the returned `path` and apply it step
by step, because the Skill tool refuses it.

Antigravity has no Skill tool. For an Antigravity thread, the service
resolves every skill found on disk as **inline** (`invoke: "inline"`, with
its `path`), plugin skills included; `t3cm slots --no-t3` does the same in a
shell where `ANTIGRAVITY_AGENT` is set. Only a skill the session lists that
no install folder holds keeps `invoke: "skill"`; use it the way the session
offers its skills.

## Commands

| Command | Use |
|---|---|
| `t3cm slots [--repo <dir>] [--json] [--no-t3]` | print the service's slot resolution for this shell's agent and repository |
| `t3cm doctor [--repo <dir>] [--json] [--no-t3]` | check the whole workflow install (config shape, skills, models, pools, service, T3 connection, skills drift, setup steps); exits 1 on a problem |
| `t3cm doctor --resolve <name>` | list every installed skill a name matches, in search order |
