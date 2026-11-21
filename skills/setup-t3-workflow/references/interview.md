# The interview, slot by slot

Each section says what the slot does, what to offer from `t3cm setup detect`,
what to suggest when nothing is found, and the JSON the answer records. Every
question also offers "keep the current setting" (from `summary`).

## Review

**What it is.** A review loop the worker runs on its branch before it opens
the PR: the reviewer reports findings, the worker fixes them, and the loop
ends when the reviewer agrees. With no reviewer, the worker opens the PR and
says no independent review ran.

**The recommendation.** Use a reviewer that runs on a different AI provider
from the worker model. A model tends to miss the same things in a review
that it missed while writing the code. A reviewer from another provider
brings different blind spots, so the change gets an independent second
opinion before the PR exists, and the user reviews work that has already
been checked once. Name the enabled providers from `providers.enabled` that
differ from the worker model's provider, and ask the user to confirm the
reviewer skill they pick runs on one of them.

**Offer** every full `name` under `slots.review.found`, then "none", then
"another skill" (the user names it).

**Nothing found.** Suggest `slots.review.suggest`: tstack's `sol-loop`. The
user can record `sol-loop` now and install it later (the roles skip it until
then), or choose none.

**Records** `"review": "<full name>"`, or `"review": null` for none.

## Passes

**What it is.** Skills the worker runs, in order, on text that goes out
under the user's name: the PR description, PR and issue comments, and docs.

**Offer** what `slots.passes.found` holds, with what each does:

- a voice skill (any name with `voice` in it): writes in the author's voice;
- `pr` (mattpocock/skills): the PR body format;
- `unslop` (cursor/plugins): tightens prose and code; usually slash-only,
  so it runs inline.

Suggest the ones in `slots.passes.missing` with where they come from. Ask
which to run and in what order; a format skill first and `unslop` last
keeps each pass working on the one before's output. Any skill the user
names can go in the list.

**Records** `"passes": ["<name>", ...]` in run order, or `"passes": []`.

## Credentials

**What it is.** Renewals for credentials that expire mid-ticket. Each pairs
a failure pattern, a Python regular expression tried against the failing
command's output, with the skill that renews the credential. The worker runs
the skill, then retries the command once.

**Offer** each skill in `slots.credentials.found` with a pattern to match,
for example `ExpiredToken|Token has expired|SSO session` for an AWS SSO
login skill. Ask for any other credential the user's work hits, and its
error text.

**Records** `"credentials": [{"match": "<regex>", "skill": "<name>"}, ...]`,
or `[]`.

## Delivery

**What it is.** Ordered stages that deliver approved work once a PR is
ready, plus `closeAfter` for when the ticket closes its GitHub issue.

- `merge` (default): single stage; the orchestrator merges the PR.
- `tag-deploy`: legacy two-stage shorthand; after the merge, the deploy
  skill pushes a release tag and watches the pipeline (`[merge, deploy
  (chain/pr/watch)]`).
- Multi-step pipeline (`steps` + optional `closeAfter`): an ordered list of
  stages, where each stage has:
  - `id`: unique stage name (`merge`, `deploy`, `verify`, `publish`, ...);
  - runner: `kind: "builtin"` for `merge` (at most once, first, `scope:
    "pr"`), `skill: "<name>"` (resolved by `t3cm`), or `kind: "manual"`
    with a short `prompt` for human-only stages;
  - `gate`: `"always"` (always opens a gate), `"chain"` (can be
    pre-approved with a preceding `"always"` stage; consecutive `"chain"`
    stages compose into prefix options), or `"never"` (runs automatically
    after the preceding stage succeeds);
  - `scope`: `"pr"` (default; runs per PR before the worker hands off),
    `"ticket"` (runs once on `main` after the last PR merges), or
    `"post-handoff"` (final report card and issue note outside the ticket);
    scopes must be non-decreasing (`pr` then `ticket` then `post-handoff`);
  - optional `paths`: list of repo-relative glob patterns (`*`, `**`, `?`,
    `[...]`, `{a,b}`) so `wf.slot("delivery", { files })` drops stages that
    match no changed file;
  - optional `watch`: `true` when the stage hands off to a scheduled
    pipeline watch;
  - `closeAfter`: `"pr"`, `"ticket"` (default), or `"all"` (whether the
    issue closes after the last PR's `pr` stages, after `ticket` stages, or
    only after `post-handoff` stages too).

**Offer** `merge`, `tag-deploy` with each skill in `slots.delivery.found`,
and a multi-step pipeline (ask for each stage's `id`, runner, `gate`,
`scope`, optional `paths`, optional `watch`, and `closeAfter`). Delivery
usually differs by repository: when it does, leave the user config on
`merge` and write the repository's multi-step pipeline to its repo-root
`.t3-workflow.json` using `t3cm setup write --repo-config` (step 3; do not
pass `--asked` with `--repo-config`). Existing `Delivery:` and `Deploy
skill:` bullets in `AGENTS.md` still work for simple `tag-deploy`, and
`.t3-workflow.json` wins over `AGENTS.md` when present
([slots.md](../../t3-workflow/slots.md#where-settings-live)).

**Records**

- Single merge: `"delivery": {"mode": "merge"}`
- Legacy tag-deploy: `"delivery": {"mode": "tag-deploy", "skill": "<name>"}`
- Multi-step pipeline (usually written to `.t3-workflow.json` via
  `t3cm setup write --repo-config`):

```json
{
  "delivery": {
    "steps": [
      {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
      {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "ticket", "watch": true},
      {"id": "publish", "skill": "widgets-publish", "gate": "always", "scope": "post-handoff", "paths": ["content/**"]}
    ],
    "closeAfter": "ticket"
  }
}
```

## Models

**What it is.** The model and effort for ticket threads (they split the
issue, launch workers, merge) and for worker threads (they build each PR),
set separately. The default is `claude-opus-5-5` at `high` effort when T3
offers Claude, else the provider's default.

**Offer** the models in `providers.enabled`, grouped by provider, with their
effort values, and each t3cm pool from step 1 as `pool <name>` with its model,
effort and members. A pool spreads launches across the accounts in it, so the
roles pick a member for each launch with t3cm's `wf.pick`. Show a pool's
`error` beside it when it has one. Ask about the ticket model, then the
worker model. The effort must be one of the values listed for that model; a
model with none listed records no `effort`. When the worker's provider is the one the user named for
the reviewer, ask whether to change the reviewer or keep the pair.

**T3 did not answer** (`providers.error`): ask for the provider id, the model
id and the effort, and record what the user gives, leaving out `effort` when
they give none. Say the summary from `t3cm setup detect` shows them as
unverified until T3 is reachable.

**A pool chosen:** record `pool` alone; the pool's model and effort come from
the t3cm config. For the ticket model, also tell the user that tickets the
board launches use the t3cm config's `board.pool`, not this slot.

**Records**
`"models": {"ticket": {"provider": "<id>", "model": "<id>", "effort": "<value>"}}`
(`effort` only when the model lists effort values, or the user gave one
while T3 did not answer), or `"models": {"ticket": {"pool": "<name>"}}` for a
pool; `worker` the same way.
