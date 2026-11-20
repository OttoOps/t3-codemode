---
name: t3-worker
description: Procedure for a T3 Code worker thread that builds one pull request of a piece of work in its own worktree, follows it until it is delivered (merged, and deployed where the repo deploys), and hands off to the orchestrator thread. The worker thread follows this body inline through the t3-worker skill. Not for dispatch as a subagent.
---

You are the worker thread for one pull request (PR) of a piece of work: a
GitHub issue, or a request with no issue. An orchestrator thread running
`t3-ticket` launched you into a worktree T3 created for this PR, with a
brief. You own this PR from first commit until its change is delivered: a
merged PR whose deploy failed is still yours. The orchestrator owns the
issue, the next PR, every merge and deploy, and every conversation with the
user.

Run the steps in order. Every turn you end, you end it waiting on T3: the PR
watcher, the link watcher's nudge, or a message from the orchestrator wakes
you, so the turn ends with nothing of yours still running in the shell.
`sleep`, polling loops, and background shell watchers die with the session,
so they never stand in for a wake.

## The link protocol

You are a child on a `t3-workflow` link, and the orchestrator is your
parent. Invoke that skill before step 1 and run the `t3wf join` line from
the brief's "t3-workflow link" block; it binds the link. From then on,
every message to the orchestrator goes through `t3wf send`, and a message
that opens with `[t3wf2` needs no command before you act on its body. The
kinds this procedure sends: `event` (PR opened, ready to merge, updated),
`note` (routine progress that needs nothing from the orchestrator: built,
review started, checks green, a skipped slot), `gate` (step 7), `blocker` (a
stop: wrong checkout, wrong identity), and `handoff` (step 6).

`t3-workflow` asks each role for one thing. Yours is:

- **Resume step:** re-read the step you were on, and read the slots again
  (step 1). Once the PR is open,
  `list_thread_pull_requests`: when this PR (or a follow-up PR) is no
  longer watched, call `watch_pull_request` on it again. Then check the
  PR's state (`gh pr view <number> --json state,headRefOid,statusCheckRollup`)
  and act on any wake you missed (step 4). Then carry on from that step.

When you wait on a person, send the gate: `t3wf send --kind gate --gate
merge --body '<action line>'` when you send "ready to merge", `--gate
review` when you send review findings for the user to decide, and the
gate's own name for step 7. It opens the gate, so the watcher reads
your silence as a wait, not a stall. Clear it with `t3wf gate --clear` when
the answer, "Merged", or "Delivered" arrives.

## 1. Orient

Read the brief: the issue (or "no issue"), which PR this is ("PR n of N"),
its acceptance line, its passes, the delivery (`merge` or `tag-deploy`),
the previous worker's handoff, and the "t3-workflow link" block. Pick up
the handoff's state and decisions as settled.

Call `t3_worktree_status`. The thread must be attached to a worktree on the
brief's branch. When it reports the project root, the main workspace, or
the default branch, send the orchestrator the status output and the line
"worker bound to the main checkout, not building", then end the turn. The
main checkout belongs to every thread; this PR's work belongs in its own
worktree.

Read the slots with `t3wf slots`, as
[Reading slots in a role](../t3-workflow/slots.md#reading-slots-in-a-role)
says. Three lines steer this procedure: `review` (the review in step 2),
`passes` (Publishing, step 2) and `credentials` (step 7). Send the skip
notes it calls for now.

## 2. Build

Read the repo's own rules before the first edit: `AGENTS.md` and
`CLAUDE.md` at the root and in each directory you touch, `CONTRIBUTING.md`,
`README.md` sections on contributing and versioning, and `docs/agents/`
where it exists. They set the identity, the commit trailers, the tests, the
version bump, and the done-checks. Repo rules outrank this template.

Build exactly the PR's scope. Its acceptance line is the completion
criterion: the PR is done when that line holds and the repo's done-checks
pass. Work the brief assigns to a later PR stays out of this one. When the
brief names an issue, every commit carries the trailer
`--trailer "Github-Issue:#<number>"`. Stage explicit paths only.

When the brief lists `Passes:`, they are part of the scope: after the
build, invoke each pass's skill with the Skill tool, in the brief's order,
each on the text the one before it left, and commit its fixes. The PR is
done when the last pass comes back clean. The split's approval runs the
passes, not what they gate: anything a pass leads to that the repo's rules
reserve for a person, and any GPU use, goes to the orchestrator as a human
gate (step 7). Name each pass and its result in the PR description.

Before every commit and every push, check the identity git and `gh` will
use:

```bash
git var GIT_COMMITTER_IDENT
git var GIT_AUTHOR_IDENT
git remote -v
gh api user -q .login
```

Each must match what the repo documents. When the repo names no bot, the
idents must be the user's own (`git config user.name`, `user.email`), the
remote must be the repo the brief names, and the `gh` login must be the
user's account. When anything differs, stop: send the orchestrator the
outputs and what was expected, then end the turn. Nothing is committed or
pushed under the wrong identity.

### Independent review

When the acceptance line holds and the done-checks pass, commit the PR's
scope, read the slots again, then run the `review` slot's skill on this
branch, as its entry says, before anything is pushed. Its base is
`git merge-base HEAD origin/main`, its intent is the PR's acceptance line,
and its out-of-scope is the work the brief gives to later PRs. Follow it to
its verdict. Name any review worktree it could not remove in the "opened"
message (step 3). Commit each fix the review leads to, with the identity
check above.

- **Agreed:** go to step 3. The PR description carries any line the
  reviewer asks for, word for word.
- **Stuck** (a finding disputed after two replies, or the review's budget
  ends with a blocker open), **or the reviewer cannot run** (its model or
  route is down): send the orchestrator the open findings, both sides in
  2-3 lines each, or the blocker, as the `review` gate. End the turn. The
  user decides, and the orchestrator relays the answer. Push nothing until
  it comes.
- **No reviewer** (`review: none`, or the reviewer skipped): go to step 3.
  The PR description says no independent review ran, and why: none is
  configured, or the configured reviewer no longer resolves.

A follow-up PR (step 5) runs this review too. A push for a wake in step 4
does not.

## 3. Open the PR and watch it

1. Write the PR description through the Publishing section below, all three
   steps, before you push. When the brief names an issue, the description
   carries `Refs #<number>`, never `Closes` or `Fixes`: the orchestrator
   closes the issue after delivery, not at merge.
2. Push the branch and open the PR against `main` with that description:
   `gh pr create --base main --head <branch> --title <title> --body-file
   <file>`. The title follows Conventional Commits.
3. `link_pull_request` with the PR URL.
4. `watch_pull_request` on it.
5. Send the orchestrator "PR n/N opened: #<number> <PR URL>, head <sha>",
   followed by the `Publishing:` line (Publishing, step 3), and one of:
   - "Checks: none in this repo" when the repo runs no CI on PRs;
   - "Checks: pending" otherwise.
6. End the turn.

The watcher wakes you on failed checks, passed checks, comments, reviews,
and conflicts. It never wakes you on a merge or a deploy: the orchestrator
does those and messages you.

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
- **The required checks passed:** send the orchestrator "PR n/N ready to
  merge: #<number>, head <sha>, checks passed" as the `merge` gate (`t3wf
  send --kind gate --gate merge --body '<action line>'`). Send it once per
  head sha, and never while a `review` gate is open: send it after the
  user's answer clears that gate.
- **After any push:** when you already sent "opened" or "ready" for an older
  head, send "PR n/N updated: #<number>, head <sha>" with what changed. The
  orchestrator merges with `--match-head-commit`, so it must know the head.

- **A message that opens with `[t3wf2 nudge`:** "The link protocol"
  above. It sends you through your resume step, then a note, a gate, or the
  handoff.

The watch stays on after each wake, so do not call `watch_pull_request`
again on a wake or after a push. Call it again only when
`list_thread_pull_requests` shows this PR is no longer watched. Then end
the turn.

### Messages from the orchestrator

Each arrives as a `reply` on the link; act on its body:

- **"Merged #<number> at <sha>. Hand off."**: step 6, whatever the
  brief's delivery. The orchestrator sends it for a `tag-deploy` PR when
  the deploy was skipped.
- **"Delivered: <tag> <pipeline URL>. Hand off."** (delivery
  `tag-deploy`): step 6.
- **"Deploy failed: ..."**: step 5.
- **A user decision** (a review dispute, a gate cleared): act on it from
  the step that was waiting.

## 5. Deploy failed: follow-up PR

The orchestrator sends the failed run's URL and what it found. Read the log
yourself. Fix the cause in a follow-up PR from this thread and this
worktree:

1. `git fetch origin`, then `git switch -c <brief branch>-fix origin/main`.
   When that name is taken (local branch, `git ls-remote --heads origin`,
   or `gh pr list --state all --head <name>`), add `-2`, `-3`.
2. Steps 2 to 4 for the new PR, its review included. Its
   acceptance line is "the deploy that failed succeeds". Its "opened"
   message names it as a follow-up of #<number>.

You own the change until the fix is delivered.

## 6. Delivered: hand off

Run this only after the orchestrator's "Merged" or "Delivered" message.

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
   result: the merge commit, and after "Delivered" the tag and the
   pipeline URL (after "Merged" on a `tag-deploy` brief, that the deploy
   was skipped), plus the URL of any follow-up PR.
3. Send the block, verbatim and whole, to the orchestrator with `t3wf send
   --kind handoff --body -` through a quoted heredoc. It closes the link.
4. `t3wf sweep`.
5. `t3_thread_organize` with action `settle`.

Send first, settle second, every time. A settled thread with an unsent
handoff leaves the orchestrator waiting on a message that will never arrive.

## 7. On a human gate

Some steps only a person can do: a login, a decision, a review. When you
reach one, send the orchestrator one `gate` message (`t3wf send --kind gate
--gate <name> --body '<action line>'`) naming the gate, what you need, and
the exact command or link the user will use; it opens the gate. End the
turn. The orchestrator
tells the user and messages you once it clears; then `t3wf gate --clear`.

**Credential renewal.** When a command fails and its output matches a
pattern on the `credentials` slot's line (`/<pattern>/ -> <skill>`, a
Python regular expression), run that renewal skill as its entry says, then
run the failed command again, once. When the renewal needs the user (a
sign-in to approve), send its gate naming what the user does; when the
renewal's own exit wakes you, retry then, without waiting for the
orchestrator. When the retry fails too, send a `blocker` carrying the
command and its output. A failure no pattern matches, or one whose renewal
skill is skipped, goes back to the step it came from, as it would with no
renewal configured.

## Publishing

Text that goes out under a person's name, such as a PR description, a PR
comment, an issue comment, or a doc page (README, ADR, runbook, `docs/`
prose), goes through three steps before you publish it. Code comments are
not included.

1. **Format.** When the repo's rules (`AGENTS.md`, `CONTRIBUTING.md`,
   `docs/agents/`, a `.github/pull_request_template.md`) or the brief set a
   format for this kind of text, write it in that format. When they name a
   skill for it, such as the `pr` skill for PR descriptions, invoke that
   skill with the Skill tool and follow it. Knowing the skill's sections is
   not running it. An earlier PR's description is never the format: it may
   have skipped this step too, and copying it passes the miss on.
2. **Passes.** Run the `passes` slot's skills on the whole text from
   step 1, in the order the slot lists them, each on the text the one
   before it left, each as its entry says: the Skill tool, or inline from
   its `SKILL.md`. Follow all of each pass, including any check script it
   ships, and apply every rule it sets, one rule at a time. Fix each
   violation before the next pass. Where the format from step 1 requires
   something a rule forbids (a parenthetical issue reference, a template
   heading), keep the format and note the exception. A slot of `none` runs
   no pass. A pass scoped to part of the text, such as a skill call whose
   arguments name only one section, does not cover the rest.
3. **Report.** Where you report the publish (for a PR, the "opened"
   message), add a `Publishing:` line: the format skill you invoked, the
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
