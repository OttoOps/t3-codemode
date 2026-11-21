---
status: accepted
date: 2026-11-20
---

# Compose delivery from ordered steps in `.t3-workflow.json`

## Context and problem statement

The `delivery` slot previously supported only two fixed modes: `merge` alone,
or `tag-deploy` paired with one deploy skill. Every other slot (`passes`,
`credentials`, `models`) already accepted structured configuration, while
`delivery` stayed a binary switch.

Repositories with multi-stage releases broke that assumption in four ways:

- Different changed paths trigger different stages, such as a content
  publish step that applies only when files under `content/**` change, while
  a deploy skill still selects its own tag family internally.
- Several merged PRs often go out together as one batched deploy on `main`,
  so tying worker thread ownership and worktree lifetime to a per-PR deploy
  held worktrees open across unrelated PRs or forced premature deploys.
- Deployment pipelines finish asynchronously and need a scheduled watch
  before later stages run.
- Owner-gated stages after deploy, such as publishing live catalog data or
  cross-ticket batched releases, run on a person's schedule and would wedge
  a ticket thread and GitHub issue open indefinitely if the ticket waited
  for them.

Issue #101 records the requirements and settled design across PRs #156, #158
and this change.

## Decision drivers

- Express multi-stage delivery (merge, deploy, verify, manual approval,
  post-handoff publishing) in one ordered pipeline without hardcoding
  delivery wording in the board.
- Keep `gate: "merge"` intact so GitHub reconcile continues to track PRs,
  hold approvals to an approved head SHA, and clear the merge gate on merge.
- Filter path-conditional stages deterministically in Rust rather than
  asking an LLM to evaluate glob rules in prose.
- Separate PR worker ownership (freeing worktrees once `pr`-scoped stages
  finish) from ticket-level work on `main` and post-handoff tasks, and let
  each repository pick when its issue closes.
- Keep existing `merge`, `tag-deploy`, and `AGENTS.md` `Delivery:` /
  `Deploy skill:` configurations working with zero migration.
- Validate repository delivery configs in `t3cm doctor` and write them
  through `t3cm setup write --repo-config`.

## Considered options

- Add more fixed `mode` values and `AGENTS.md` bullet keys
- Move every multi-stage concern into one repository-specific deploy skill
- Replace binary `mode` with ordered `delivery.steps` and `closeAfter`,
  configured in repo-root `.t3-workflow.json`, filtered in Rust via
  `wf.slot("delivery", { files })`, and gated with explicit `options` on
  `merge` and `delivery:<id>` gates

## Decision outcome

Chosen: compose delivery from ordered `delivery.steps` and `closeAfter`,
layered through `.t3-workflow.json`, resolved and path-filtered in
`src/slots.rs`, and executed by `t3-ticket` and `t3-worker`.

- **Ordered step schema.** `delivery.steps` is a non-empty list of stages,
  each with a unique `id`, a runner (`kind: "builtin"` for `merge` at most
  once and first with `scope: "pr"`, `skill: "<name>"` resolved by
  `src/slots.rs`, or `kind: "manual"` with a non-empty `prompt`), a `gate`
  policy (`always`, `chain`, or `never`, where `chain` requires a preceding
  `always`), a non-decreasing `scope` (`pr`, `ticket`, or `post-handoff`,
  default `pr`), optional `paths` globs (`*`, `**`, `?`, `[...]`, `{a,b}`),
  and optional `watch: true`. `closeAfter` (`pr`, `ticket`, or `all`,
  default `ticket`) controls when the ticket closes its GitHub issue.
- **Four-layer configuration with `.t3-workflow.json`.** Structured
  pipelines live in a repo-root `.t3-workflow.json` (the same JSON schema as
  `~/.config/t3-workflow/config.json`, without per-machine `steps` or
  `writtenBy`), rather than inventing multi-line bullet syntax in
  `AGENTS.md`. Precedence runs slot by slot: defaults, then user
  `config.json`, then `AGENTS.md`, then `.t3-workflow.json`. `Snapshot`
  fingerprints `.t3-workflow.json` so cached SQLite resolutions rebuild when
  the file changes, and `t3cm setup write --repo-config` validates and
  writes it.
- **Deterministic `paths` filtering.** `wf.slot("delivery", { files })`
  matches each step's `paths` globs in Rust using a bounded-depth glob
  compiler, drops unmatched steps from `steps`, and returns them in
  `skipped` with their `id`, `reason`, and `paths`. A single deploy skill
  can still choose internal tag families from changed paths.
- **Composable gates and explicit board `options`.** The builtin merge
  stage keeps `gate: "merge"` so GitHub reconcile clears only the merge
  gate. Every subsequent stage uses `gate: "delivery:<id>"`. Consecutive
  `gate: "chain"` stages compose arbitrarily. `t3-ticket` passes one
  explicit option per prefix via
  `wf.send({ kind: "gate", gate, body, options })`, such as
  `["merge, deploy and verify", "merge and deploy", "merge"]`, and picking
  a shorter prefix opens `gate: "delivery:<id>"` with `"skip <id>"` when
  the next stage is reached. Stages with `gate: "never"` run automatically
  once their predecessor succeeds.
- **Scope ownership and failure paths.** A PR worker owns its PR until the
  last `pr`-scoped stage finishes, fixing any `pr`-scoped failure in a
  follow-up PR before handing off and freeing its worktree. `ticket`-scoped
  stages run once on `main` after the last PR hands off, launching a
  targeted fix worker on `main` if one fails. `post-handoff` stages run
  outside the ticket. Under `closeAfter: "ticket"` or `"pr"`, `t3-ticket`
  posts a note on the issue, emits a final report checklist that it also
  includes in the `handoff` body because `handoff` resolves open board
  cards, and hands off. Under `closeAfter: "all"`, each `post-handoff`
  stage opens a `delivery:<id>` gate before close. Stages with `watch: true`
  use the existing scheduled deploy watch.
- **Backwards compatibility.** Legacy `{"mode": "merge"}`,
  `{"mode": "tag-deploy", "skill": "<s>"}`, and `AGENTS.md` `Delivery:` /
  `Deploy skill:` bullets normalize to equivalent `steps` (`[merge]` or
  `[merge, deploy (chain/pr/watch)]`) with `closeAfter: "ticket"`, and
  legacy-equivalent resolved outputs keep `mode` and `skill` alongside
  `steps`.

### Consequences

- Good: multi-stage pipelines (per-PR merge, batched ticket deploy with
  watch, path-gated post-handoff publish) run through one uniform mechanism
  with `t3cm doctor` validation.
- Good: PR worktrees are released after `pr`-scoped stages instead of
  staying alive across batched ticket deploys or manual publishing.
- Good: the board needs zero delivery-specific button parser rules because
  `t3-ticket` supplies explicit `options` per gate.
- Good: existing repositories and user configs keep resolving with no edits.
- Bad: `t3-ticket` now tracks approved chain prefixes across merges and
  scheduled watches, and launches a fresh fix worker on `main` when a
  `ticket`-scoped stage fails.
- Bad: arbitrary chaining can exceed the board's 8-option or 60-character
  button limits on very long chains, which `t3-ticket` must keep within
  bounds until the board adds dedicated multi-step card UI.
- Neutral: reversing this decision would mean removing `delivery.steps`
  handling from `src/slots.rs`, `src/wf_slots.rs`, `src/setup.rs`, and the
  role skills while leaving legacy `mode`/`skill` configs intact.

## Pros and cons of the options

### Add more fixed `mode` values and `AGENTS.md` bullet keys

- Good: small config change for each single new workflow.
- Bad: every combination of deploy, verify, manual sign-off, path filter,
  and scope requires another mode name and bespoke parsing in `AGENTS.md`
  and `t3-ticket`.

### Put all multi-stage logic in one repository-specific deploy skill

- Good: keeps `src/slots.rs` unchanged.
- Bad: hides gates, watches, scope boundaries, and issue-close rules inside
  per-repo skill prose, leaving `t3cm doctor` unable to validate stages and
  forcing each repo to reimplement chaining, worker handoff, and recovery.

### Composable `delivery.steps` in `.t3-workflow.json`

- Good: separates stage orchestration, path filtering, and validation
  (in `t3cm` and the role skills) from stage execution (in each skill),
  keeps GitHub reconcile on `gate: "merge"`, and preserves legacy configs.
- Bad: introduces a fourth config layer (`.t3-workflow.json`) and a richer
  schema for `t3cm setup` and `t3-ticket` to maintain.

## More information

Issue #101 defines the requirements and settled design. Implementation spans
PR #156 (bundled #91 merge-watch fixes), PR #158 (`src/slots.rs`,
`src/wf_slots.rs`, `src/setup.rs`, `src/main.rs`, `src/catalog.rs`), and the
role-skill, setup-interview, and documentation updates in
`skills/t3-ticket/SKILL.md`, `skills/t3-worker/procedure.md`,
`skills/setup-t3-workflow/`, `skills/t3-workflow/slots.md`, and
`docs/src/board.md`.
