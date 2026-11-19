# Skill evals

The eval sets for the skills in `skills/`, in the format Anthropic's
`skill-creator` uses, and a runner for the live part.

| File | What it holds |
|---|---|
| `<skill>/triggers.json` | Trigger evals: `{"query", "should_trigger"}` items, plus `form` (`plain` or `brief`) on each should-trigger item and `env: "plain"` on a prompt that runs outside T3 Code. |
| `<skill>/evals.json` | Behavior evals for the roles and setup: skill-creator's `evals.json` shape, with a `checks` list that matches `expectations` one to one, and the `setup` the sandbox needs. |
| `live.py` | The live runner. |
| `stub_t3_mcp.py`, `fake_gh.py` | The stub T3 MCP server and the fake `gh` a sandbox runs with. |

The sets live here rather than in each skill's folder because
`npx skills add` copies a skill's whole folder onto the user's machine, and
nothing at run time reads them.

## What CI runs

`python3 -m unittest discover -s tests/skills -v` runs `test_evals.py`. It
checks the shape of every eval set, that every skill has trigger evals with
both kinds and a plain-language trigger, that no prompt starts with a slash,
that `t3-ticket` and `t3-worker` trigger from a launch brief, and that a
worker eval configures a slash-only pass. It also checks the sandbox, the
stub server, the fake `gh` and the check matchers. None of it calls a model.

## Live runs

Live runs call Claude, cost money, and run on demand, never in CI. Each run
gets its own sandbox under `--out` (a temporary folder by default):

- its own `HOME`, XDG config and state, and T3 home, so the real
  `~/.claude`, `~/.agents`, `~/.codex` and T3 Code are never read or written;
- `PATH` set to the sandbox's `bin` (a fake `gh`, `python3` and `claude`)
  then `/usr/bin:/bin`;
- a stub `t3-code` MCP server that logs each call, `t3wf` on its file-backed
  fake T3, and a T3 Code runtime note in the sandbox's `CLAUDE.md`, so a
  prompt runs as it would in a T3 Code thread (`env: "plain"` drops all three);
- a git repo for `acme/widgets` whose pushes land in a local bare repo.

Auth comes from `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` in the
environment. `claude setup-token` prints a token for the second.

### Trigger evals

```sh
python3 tests/skills/evals/live.py triggers [--skill t3-ticket] [--runs 3] [--model claude-opus-5-5]
```

The runner needs skill-creator installed (the `skill-creator` plugin), or
`--skill-creator <dir>`. Each run puts the skill's description in front of
the model the way skill-creator's `run_eval.py` does, with the other skills
from `skills/` installed beside it, and counts it as triggered when the
model's first tool call invokes it. A prompt with a `## t3-workflow link`
block is the exception: the block tells the thread to invoke `t3-workflow`
first, so it counts when the skill is invoked within the first 8 tool calls.
A query passes when it fires in at least half its runs (or under half, for a
should-not-trigger query).

To try a new description before editing `SKILL.md`, pass `--skill <name>
--description '<text>'`.

### Behavior evals

```sh
python3 tests/skills/evals/live.py behavior [--skill t3-worker] [--eval 2]
```

Each eval runs twice: with every skill from `skills/` installed and with
none. The `checks` read the run's tool calls, its final text, the stub
server's and fake `gh`'s logs, and the repo's files. An eval passes when every
check holds with the skill and at least one fails without it.

After changing a check, score the transcripts a past run saved again, with
no model calls: `live.py behavior --rescore <out>`.

Both commands print a summary and write the full results, each run's
transcript and its sandbox to `--out`. Remove the folder when you are done.
