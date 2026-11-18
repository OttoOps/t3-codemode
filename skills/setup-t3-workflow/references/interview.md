# The interview, slot by slot

Each section says what the slot does, what to offer from `setup detect`,
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

**What it is.** What delivers a PR once it is approved.

- `merge` (default): the orchestrator merges the PR, and that is delivery.
- `tag-deploy`: after the merge, the deploy skill pushes a release tag and
  watches the pipeline to its end.

**Offer** `merge`, and `tag-deploy` with each skill in
`slots.delivery.found`. Delivery usually differs by repository: when it does,
leave the user config on `merge` and offer to set `Delivery:` and
`Deploy skill:` in that repository's `AGENTS.md` "Agent workflow" block
([slots.md](../../t3-workflow/slots.md#where-settings-live)).

**Records** `"delivery": {"mode": "merge"}`, or
`"delivery": {"mode": "tag-deploy", "skill": "<name>"}`.

## Models

**What it is.** The model and effort for ticket threads (they split the
issue, launch workers, merge) and for worker threads (they build each PR),
set separately. The default is `claude-opus-5-5` at `high` effort when T3
offers Claude, else the provider's default.

**Offer** the models in `providers.enabled`, grouped by provider, with their
effort values. Ask about the ticket model, then the worker model. The effort
must be one of the values listed for that model; a model with none listed
records no `effort`. When the worker's provider is the one the user named for
the reviewer, ask whether to change the reviewer or keep the pair.

**T3 did not answer** (`providers.error`): ask for the provider id, the model
id and the effort, and record what the user gives, leaving out `effort` when
they give none. Say `t3wf slots` shows them as unverified until T3 is
reachable.

**Records**
`"models": {"ticket": {"provider": "<id>", "model": "<id>", "effort": "<value>"}}`
(`effort` only when the model lists effort values, or the user gave one
while T3 did not answer); `worker` the same way.
