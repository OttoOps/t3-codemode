---
name: t3-worker
description: Procedure for a T3 Code worker thread that builds one pull request of a piece of work in its own worktree, follows it until its pr-scoped delivery stages finish (merged, and any pr-scoped stages such as deploy), and hands off to the orchestrator thread. The worker thread follows this body inline through the t3-worker skill. Not for dispatch as a subagent.
---

You are the worker thread for one pull request (PR) of a piece of work: a
GitHub issue, a ticket from a non-GitHub tracker (such as Jira or GitLab), or
a request with no issue. An orchestrator thread running
`t3-ticket` launched you into a worktree T3 created for this PR, with a
brief. You own this PR from first commit until its `pr`-scoped delivery
stages finish: a merged PR whose `pr`-scoped delivery stage (such as a
`pr`-scoped deploy) failed is still yours. Stages scoped to `ticket` or
`post-handoff` run in the orchestrator after you hand off, so your worktree
is freed as soon as your PR's `pr`-scoped stages finish. The orchestrator
owns the issue, the next PR, every delivery stage (merge, deploy, verify,
publish), and every conversation with the user.

Run the steps in order. Every turn you end, you end it waiting on T3: the PR
watcher, the link watcher's nudge, or a message from the orchestrator wakes
you, so the turn ends with nothing of yours still running in the shell.
`sleep`, polling loops, and background shell watchers die with the session,
so they never stand in for a wake.

## The link protocol

You are a child on a `t3-workflow` link, and the orchestrator is your
parent. Invoke that skill before step 1 and run the `wf.join(...)` call from
the brief's link block (through `mcp__t3cm__execute`, or piped to
`t3cm exec -` when `execute` is not listed; see
[Reaching t3cm](../t3-workflow/SKILL.md#reaching-t3cm)); it binds the link.
From then on, every message to the orchestrator goes through `wf.send(...)`,
and a message that opens with `[t3cm` needs no command before you act on its
body. Your PR's own steps (opened, updated, ready to merge) go through
`wf.pr(...)` (steps 3 and 4), which acts on them in code and sends the
orchestrator what it still has to see. The kinds this procedure sends with
`wf.send`: `note` (routine progress that needs nothing from the
orchestrator: built, review started, checks green, a skipped slot), `event`
(work with its own issue, step 5), `gate` (review findings and step 7),
`blocker` (a stop: wrong checkout, wrong identity), and `handoff` (step 6).

`t3-workflow` asks each role for one thing. Yours is:

- **Resume step:** re-read the step you were on. Once the PR is open,
  `list_thread_pull_requests`: when this PR (or a follow-up PR) is no
  longer watched, call `watch_pull_request` on it again. Then check the
  PR's state (`gh pr view <number> --json state,headRefOid,statusCheckRollup`)
  and act on any wake you missed (step 4). Then carry on from that step.

When you wait on a person, send the gate:
`await wf.send({ kind: "gate", gate: "review", body: "<action line>" })` when
you send review findings for the user to decide, and the gate's own name
for step 7. It opens the gate, so the watcher reads your silence as a wait,
not a stall. A `wf.pr` ready call opens the `merge` gate on your link for
you. Clear a gate with `await wf.gate({ clear: true })` when the answer,
"Merged", or "Delivered" arrives.

When the orchestrator's ticket is on the board, t3cm puts a `blocker`, or
any gate other than `merge` and `delivery:<id>`, in front of the user as
you wrote it. Open each one with an
action line (`**ACTION NEEDED:** <plain verb phrase>, here: <URL or
command>`), and pass `options` with the answers that need no judgment, such
as `["fix it", "ship as is"]` or `["retry"]`. A click on one of them comes
straight back to you.

## 1. Orient

Read the brief: the tracker, the issue (or "no issue"), its sub-issue, which
PR this is ("PR n of N"), its acceptance line, its passes, the delivery
(legacy `merge` or `tag-deploy`, or ordered delivery steps noting which are
`pr`-scoped), the previous worker's handoff, and the link block. Pick up the
handoff's state and decisions as settled.

The **issue this thread implements** is the brief's `Sub-issue:`, else its
`Issue:`. Note the brief's `Tracker:` (when omitted, infer `github-issues`
for a GitHub issue URL or `<owner>/<repo>#<n>`, a non-GitHub tracker for a
Jira key/browse URL or GitLab issue URL, or `none`). Call `link_issue` with
the issue's URL when it has one (or its `key` for a bare key when T3's
tracker supports it; if `link_issue` refuses a non-GitHub ref, carry on).
With no issue, link none. Skip `gh issue` commands for a non-GitHub ticket;
read it with the tool the repo documents if you need detail beyond the brief.

Call `t3_worktree_status`. The thread must be attached to a worktree on the
brief's branch. When it reports the project root, the main workspace, or
the default branch, send the orchestrator the status output and the line
"worker bound to the main checkout, not building", then end the turn. The
main checkout belongs to every thread; this PR's work belongs in its own
worktree.

## 2. Build

Read the repo's own rules before the first edit: `AGENTS.md` and
`CLAUDE.md` at the root and in each directory you touch, `CONTRIBUTING.md`,
`README.md` sections on contributing and versioning, and `docs/agents/`
where it exists. They set the identity, the commit trailers, the tests, the
version bump, and the done-checks. Repo rules outrank this template.

Build exactly the PR's scope. Its acceptance line is the completion
criterion: the PR is done when that line holds and the repo's done-checks
pass. Work the brief assigns to a later PR stays out of this one. When the
brief names an issue:
- With tracker `github-issues`: every commit carries the trailer
  `--trailer "Github-Issue:#<number>"` for the issue this thread implements.
- With a non-GitHub tracker (`jira`, `gitlab-issues`, or another tracker the
  repo names): omit the `Github-Issue` trailer; include `Refs <ticket ref>`
  (such as `Refs PROJ-123` for Jira, or `Refs <issue URL>` for GitLab) in the
  commit message instead, unless the repo's rules set another format.

Stage explicit paths only.

When the brief lists `Passes:`, they are part of the scope: after the build,
run each pass's skill as its `how` from `wf.slot("passes")` says (the Skill
tool, or inline from its `SKILL.md`), in the brief's order, each on the text the one before it
left, and commit its fixes. The PR is done when the last pass comes back
clean. The split's approval runs the passes, not what they gate: anything a
pass leads to that the repo's rules reserve for a person, and any GPU use,
goes to the orchestrator as a human gate (step 7). Name each pass and its
result in the PR description.

Before every commit and every push, check the identity git and `gh` will
use:

```bash
git var GIT_COMMITTER_IDENT
git var GIT_AUTHOR_IDENT
git remote -v
gh api user -q .login
```

Each must match what the repo documents (skip `gh api user` when the git
host is not GitHub). When the repo names no bot, the
idents must be the user's own (`git config user.name`, `user.email`), the
remote must be the repo the brief names, and the `gh` login must be the
user's account on a GitHub repo. When anything differs, stop: send the orchestrator the
outputs and what was expected, then end the turn. Nothing is committed or
pushed under the wrong identity.

### Independent review

When the acceptance line holds and the done-checks pass, commit the PR's
scope, then look up the review slot with `await wf.slot("review")` as
[Looking up slots in a role](../t3-workflow/slots.md#looking-up-slots-in-a-role)
says (sending any skip note it calls for). Run the returned skill on this
branch as its `how` says, before anything is pushed. Its base is
`git merge-base HEAD origin/main`, its intent is the PR's acceptance line,
and its out-of-scope is the work the brief gives to later PRs. Follow it to
its verdict. Name any review worktree it could not remove in the `wf.pr`
opened call's `body` (step 3). Commit each fix the review leads to, with the identity
check above.

- **Agreed:** go to step 3. The PR description carries any line the
  reviewer asks for, word for word.
- **Stuck** (a finding disputed after two replies, or the review's budget
  ends with a blocker open), **or the reviewer cannot run** (its model or
  route is down): send the orchestrator the open findings, both sides in
  2-3 lines each, or the blocker, as the `review` gate. Its action line is
  "decide each open review finding: fix it, or ship as is", with
  `options: ["fix it", "ship as is"]`, or for a reviewer that cannot run,
  "open the PR without the independent review, or fix the reviewer's
  route", with `options: ["open without review", "fix the reviewer"]`.
  End the turn. The user decides, and the answer arrives as a
  `[t3cm reply]`. Push nothing until it comes.
- **No reviewer** (`skill: null`, or the reviewer skipped): go to step 3.
  The PR description says no independent review ran, and why: none is
  configured, or the configured reviewer no longer resolves.

A follow-up PR (step 5) runs this review too. A push for a wake in step 4
does not.

## 3. Open the PR and watch it

1. Write the PR description through the Publishing section below, all three
   steps, before you push. When the brief names an issue, never use `Closes`
   or `Fixes`: the orchestrator closes the issue after delivery, not at
   merge.
   - With tracker `github-issues`: the description carries `Refs #<number>`
     for each issue the brief names (sub-issue and issue).
   - With a non-GitHub tracker (`jira`, `gitlab-issues`, or another tracker
     the repo names): the description carries `Refs <ticket ref>` instead of
     `Refs #<number>` (`Refs PROJ-123` for a bare Jira key,
     `Refs PROJ-123 (<browse URL>)` or `Refs <host>/PROJ-123` for a hosted
     Jira URL, or `Refs <issue URL>` / `Refs <host>/<scope>#<n>` for GitLab,
     matching the worker link's `issue` identity so `wf.pr` accepts it).
2. Items 2 to 5 below and the GitHub `gh` / `wf.pr` / `watch_pull_request`
   steps in step 4 apply to a GitHub `origin`. On a non-GitHub git host (such
   as self-hosted GitLab), follow the repo's documented MR/PR, check, and
   review workflow in place of those GitHub steps, report MR updates and
   readiness to the orchestrator with `wf.send` (carrying the MR URL, `head
   <sha>`, and the `Publishing:` line on open), and end the turn. Otherwise
   push the branch and open the PR against `main` with that description:
   `gh pr create --base main --head <branch> --title <title> --body-file
   <file>`. The title follows Conventional Commits.
3. `link_pull_request` with the PR URL.
4. `watch_pull_request` on it.
5. Report it:
   `await wf.pr({ event: "opened", repo: "<owner>/<name>", pr: <number>, head: "<sha>", publishing: "<Publishing line>", checks: "pending" })`,
   with the `Publishing:` line from Publishing, step 3, and
   `checks: "none"` when the repo runs no CI on PRs. Name any review
   worktree you could not remove in `body`. Act on its `status`:
   - `open`: t3cm checked the description and the Publishing line.
   - `rewrite`: do what `message` says (`gh pr edit <number> --body-file
     <file>`, through Publishing again), then call the same `wf.pr` opened
     again with the new Publishing line.
   - `ticket`: t3cm sent it to the orchestrator, for the reason in `why`.
6. End the turn.

The watcher wakes you on failed checks, passed checks, comments, reviews,
and conflicts. It never wakes you on a merge or a deploy: the orchestrator
does those and messages you. Never merge the PR yourself, through `gh`,
`wf.merge`, or T3's merge button or PR action.

## 4. On a wake

- **A check failed:** read the run log (`gh run view <run id> --log-failed`),
  fix the cause, push.
- **A comment or review from a person:** answer it in the PR or change the
  code, push. A reply posted on the PR goes through Publishing.
- **A review from a bot reviewer** (CodeRabbit or any other automated
  reviewer): a round is one bot review of one head. Count the rounds this
  PR has had (`gh api repos/<owner>/<repo>/pulls/<number>/reviews`, the
  bot's reviews, one per distinct `commit_id`).
  - Fix only the Critical and Major findings that still hold against the
    current code, all of them in one push. When you dispute one, reply
    once with why, leave the code, and send it as the `review` gate below
    without waiting for another round.
  - Do not change code for a Minor, Trivial or nitpick finding. Reply in
    one line that it is advisory and not fixed, then resolve the thread.
    A finding with no severity counts as Minor unless it names data loss,
    a security hole, or a crash on a normal path.
  - After the bot's second round, push nothing more for bot findings.
  - Send the orchestrator each open Critical or Major finding you dispute
    or, after the second round, have not fixed, both sides in 2-3 lines
    each, as the `review` gate, and end the turn. The user decides.
  - Replies go through Publishing.
- **A conflict:** `git fetch origin`, rebase on `origin/main`, re-run the
  done-checks, push with `--force-with-lease=<branch>:<old head sha>`. The
  PR targets `main`; it is never stacked on another PR.
- **The required checks passed:** call ready only once every check on
  the head has finished, CodeRabbit's and any other bot reviewer's
  included, even where the repo does not require it, and you have read
  any review a bot posted of that head:
  `await wf.pr({ event: "ready", repo: "<owner>/<name>", pr: <number>, head: "<sha>" })`.
  A brief that says to report ready whatever the bot's state means its
  review decision, never a check still running. A merge approval holds
  only for the head it names. Call it once per head sha, and never while
  a `review` gate is open: call it after the user's answer clears that
  gate. Act on its `status`:
  - `gated`: the merge card is on the user's board, and your link holds
    the `merge` gate. Wait for the orchestrator's "Merged" message.
  - `waiting`: a check is still running. t3cm opens the merge card once it
    finishes, or `detail` says to call ready again.
  - `failing`, or `refused`: for a draft with green checks, mark the PR
    ready for review and call `wf.pr` ready again at the same SHA without
    pushing; for a failing check, moved head, or conflict, handle it as the
    wakes above say, push, then send updated.
  - `ticket`: t3cm sent the orchestrator your "ready to merge" as its
    `merge` gate, for the reason in `why`.
- **After any push:** when you already called `wf.pr` opened or ready for
  an older head, call
  `await wf.pr({ event: "updated", repo: "<owner>/<name>", pr: <number>, head: "<new sha>", body: "<what changed>" })`.
  It records the head and drops any merge card for the old one, because the
  orchestrator merges only at the head you named. Then call ready again
  when the new head's checks pass.
  CodeRabbit reviews each push on its own until its status comment on the
  PR says reviews are paused; the repo's `CONTRIBUTING.md` says how its
  pause works. Post `@coderabbitai review` once after a push, and only when
  that comment says paused. Before that, CodeRabbit answers the command
  with "applicable only when automatic reviews are paused", and that reply
  is not a review starting.
- **A `[t3cm reply from=t3cm` about your PR:** a check failed, or the PR
  changed, after a `waiting` ready. Treat it as the wake it names.

- **A message that opens with `[t3cm nudge`:** "The link protocol"
  above. It sends you through your resume step, then a note, a gate, or the
  handoff.

The watch stays on after each wake, so do not call `watch_pull_request`
again on a wake or after a push. Call it again only when
`list_thread_pull_requests` shows this PR is no longer watched. Then end
the turn.

### Messages from the orchestrator

Each arrives as a `reply` on the link; act on its body:

- **"Merged #<number> at <sha>. Hand off."**: step 6, whatever the
  brief's delivery. The orchestrator sends it when `merge` is this PR's
  last `pr`-scoped stage (including when later stages are `ticket`-scoped
  or `post-handoff`, or when a later `pr`-scoped stage such as `deploy` was
  skipped).
- **"Delivered: <summary>. Hand off."** (for example `<tag> <pipeline URL>`,
  or the completed `pr`-scoped stage ids): step 6. Sent once this PR's last
  `pr`-scoped delivery stage finishes.
- **"Deploy failed: ..."** or **"Delivery stage <id> failed: ..."**: step 5.
- **A user decision** (a review dispute, a gate cleared): act on it from
  the step that was waiting.

A `[t3cm reply from=board]` is the user's answer to your blocker or gate,
which t3cm took to the board past the orchestrator. Act on
it as a user decision.

## 5. PR delivery stage failed: follow-up PR

The orchestrator sends the failed `pr`-scoped stage's URL or details and
what it found. Read the log yourself. Fix the cause in a follow-up PR from
this thread and this worktree:

1. `git fetch origin`, then `git switch -c <brief branch>-fix origin/main`.
   When that name is taken (local branch, `git ls-remote --heads origin`,
   or `gh pr list --state all --head <name>`), add `-2`, `-3`.
2. Steps 2 to 4 for the new PR, its review included. Its acceptance line is
   "the failed delivery stage succeeds". Its `wf.pr` opened call's `body`
   names it as a follow-up of #<number>.

You own the change until the fix passes its `pr`-scoped delivery stages.
(`ticket`-scoped failures on `main` run after you hand off and are handled
by a separate fix worker the orchestrator launches.)

### One thread, one issue

The follow-up PR fixes this PR's own failed stage, so it stays in this
thread under this thread's title. Work that has its own tracked issue, such
as a follow-up issue filed from a review, gets a fresh worker: send the
orchestrator an `event` naming the issue, and the orchestrator launches
one for it.

When the user has this thread take on another issue anyway, first make the
thread name it:

1. Build the new label as `t3-ticket` step 3 does, then
   `t3_thread_update` with `action: "rename"` and `title`
   `<label>: <PR title>`.
2. Rerun the brief's `wf.join` call with the new `label`, `issue`,
   `parentIssue` and `part`. A field left out keeps its old value.
3. `link_issue` with the new issue's URL (when it has one).

## 6. Delivered: hand off

Run this only after the orchestrator's "Merged" or "Delivered" message,
which arrives once this PR's last `pr`-scoped delivery stage finishes (never
wait on `ticket`-scoped or `post-handoff` stages).

1. Push every branch this thread committed to: the brief's branch, a
   follow-up branch, and any other branch you made (other threads' branches
   are theirs). For each, `git log <branch> --not --remotes --oneline`
   lists its commits that are on no remote; push the branch when it lists
   any. A branch that has to stay local (abandoned work, or the user said
   so) goes into the handoff by name, with its unpushed commits and the
   reason. A local-only branch is lost work once the orchestrator removes
   this worktree.
2. Write the handoff block with the `handoff` WRITE-mode template, the
   next PR from the brief as its next-session focus. Send it; do not save
   it to `~/.claude/handoffs/`, because a saved handoff supersedes the
   project's live one, which belongs to the user. It states the delivery
   result: the merge commit, and after "Delivered" the completed
   `pr`-scoped stage results (such as tag and pipeline URL), or after
   "Merged" whether later `pr`-scoped stages were skipped or deferred to
   `ticket` or `post-handoff` scope, plus the URL of any follow-up PR.
3. Send the block, verbatim and whole, to the orchestrator with
   `await wf.send({ kind: "handoff", body })`. It closes the link.
4. `await wf.sweep()`.
5. `t3_thread_organize` with action `settle`.

Send first, settle second, every time. A settled thread with an unsent
handoff leaves the orchestrator waiting on a message that will never arrive.

## 7. On a human gate

Some steps only a person can do: a login, a decision, a review. When you
reach one, send the orchestrator one `gate` message
(`await wf.send({ kind: "gate", gate: "<name>", body: "<action line>" })`)
naming the gate, what you need, and the exact command or link the user will
use; it opens the gate. End the turn. The orchestrator tells the user and
messages you once it clears; then `await wf.gate({ clear: true })`.

**Credential renewal.** When a command fails, call
`await wf.renewal({ output })` with the failing command's output (sending
any skip note as
[Looking up slots in a role](../t3-workflow/slots.md#looking-up-slots-in-a-role)
says). When it returns a renewal skill that is not skipped, run it as its
`how` says, then run the failed command again, once. When the renewal needs
the user (a sign-in to approve), send its gate naming what the user does;
when the renewal's own exit wakes you, retry then, without waiting for the
orchestrator. When the retry fails too, send a `blocker` carrying the
command and its output. When `wf.renewal` returns `null` or a skipped
skill, go back to the step it came from, as it would with no renewal
configured.

## Publishing

Text that goes out under a person's name, such as a PR description, a PR
comment, an issue comment, or a doc page (README, ADR, runbook, `docs/`
prose), goes through three steps before you publish it. Code comments are
not included.

1. **Format.** When the repo's rules (`AGENTS.md`, `CONTRIBUTING.md`,
   `docs/agents/`, a `.github/pull_request_template.md`) or the brief set a
   format for this kind of text, write it in that format. When they name a
   skill for it, such as the `pr` skill for PR descriptions, invoke that
   skill with the Skill tool and follow it; in a harness with no Skill
   tool, read the `SKILL.md` that `t3cm doctor --resolve <name>` prints and follow
   it. Knowing the skill's sections is not running it. An earlier PR's description is never the format: it may
   have skipped this step too, and copying it passes the miss on.
2. **Passes.** Look up `await wf.slot("passes")` (sending any skip note as
   [Looking up slots in a role](../t3-workflow/slots.md#looking-up-slots-in-a-role)
   says) and run its skills on the whole text from step 1, in the order
   returned, each on the text the one before it left, each as its `how`
   says: the Skill tool, or inline from its `path`. Follow all of each
   pass, including any check script it ships, and apply every rule it sets,
   one rule at a time. Fix each violation before the next pass. Where the
   format from step 1 requires something a rule forbids (a parenthetical
   issue reference, a template heading), keep the format and note the
   exception. An empty `skills` list runs no pass. A pass scoped to part of
   the text, such as a skill call whose arguments name only one section,
   does not cover the rest.
3. **Report.** Where you report the publish (for a PR, the `publishing`
   field of the `wf.pr` opened call), add a `Publishing:` line: the format skill you invoked, the
   template you filled, or "no format set by the repo"; each pass you ran,
   with its result, or "passes: none configured"; each skipped pass; and
   each rule a pass set that you did not apply, with the reason. Never say
   a pass ran if you skipped part of it.

Text you add or change after step 2, including a `gh pr edit --body-file`
after the PR opens or an issue number inserted after the draft, goes through
step 2 again, and the report covers it. A rewrite of the whole description
starts again at step 1. A fixed-format line that a skill requires word for
word, such as a reviewer's sign-off line, keeps its wording.

This section is the single copy of the publishing rule. `t3-ticket` and
`t3-worker` point here.
