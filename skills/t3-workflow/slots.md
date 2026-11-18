# Slots

The role skills name capabilities, not skills. Each **slot** holds one, and
`t3wf` resolves it to whatever this machine has installed.

| Slot | Holds | Default |
|---|---|---|
| `review` | the independent review a worker runs before it opens a PR | a found `sol-loop`, else none |
| `passes` | cleanup and templating skills a worker runs, in order | none |
| `credentials` | renewals: a failure pattern and the skill that renews it | none |
| `delivery` | `merge`, or `tag-deploy` with a deploy skill | `merge` |
| `models` | `ticket` and `worker` thread models, set separately | `claude-opus-5-5`, effort `high`, when T3 confirms it offers Claude; else the provider's default |

A slot with no setting uses its default, so a machine with no config at all
works.

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
- Worker model: claude-opus-5-5, high effort
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

A full name matches only itself, so a recorded full name always wins. A bare
name that matches several full names uses the first and lists the others. A
skill whose frontmatter sets `disable-model-invocation` resolves as
**inline**: read its `SKILL.md` at the printed path and apply it step by
step, because the Skill tool refuses it.

## Commands

| Command | Use |
|---|---|
| `t3wf slots [--skills <names>]` | at run time: each slot's effective setting, its source layer, and how to invoke its skill. A skill that no longer resolves prints as `SKIP <name>`: skip that slot, tell the parent in one note, and carry on. |
| `t3wf resolve <name>` | every installed skill a name matches, in search order |
| `t3wf doctor` | every configured skill still resolves, models exist in T3, delivery is complete; exits 1 on a problem |
| `t3wf check-config` | the config file's and the `AGENTS.md` block's shape, offline |

`slots`, `resolve` and `doctor` take `--repo <dir>` (default: the current
directory's repository) and `--json`. `slots` and `doctor` check models
through T3 when it is reachable; `--no-t3` skips that.
