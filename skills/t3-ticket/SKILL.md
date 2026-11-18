---
name: t3-ticket
description: Use when the t3_thread_launch tool is available (T3 Code) and the user hands over GitHub issue work, even without naming T3 or threads - "work on issue 533", "pick up 42 and ship it", "start on the issues we filed", "have a new thread work on that issue", an issue URL, or "work" plus a request. Splits it into PRs, one worker thread each, merges and deploys on the user's word. Not without t3_thread_launch, nor for status or summary asks or one comment, label, or close.
argument-hint: "<#issue | issue URL | work: request>"
---

This thread is the work's orchestrator. It reads the issue, splits it into
pull requests (PRs), and launches one worker thread per PR, one at a time,
each into a worktree T3 creates. Workers build; this thread coordinates,
merges, and deploys.

This role runs on the `t3-workflow` link protocol. Invoke that skill
before step 1 and keep its table at hand: each worker is this thread's
child, and a `t3-overseer` thread, when one launched this thread, is
its parent. Every message between threads and every escalation goes through
`t3wf`, and the link watcher does the timekeeping. With no parent, this
thread is the root and talks to the user itself.

## Hard rules

- This thread edits no code and makes no commits. All repo changes happen in
  a worker's worktree. Merging a PR and pushing a deploy tag are this
  thread's, and only on the user's word (Gates).
- Invoking this skill, or a repo rule that routes issue work to it, is the
  user's request to launch one new thread per PR with `t3_thread_launch`.
  Do not stop to ask for a new thread at step 3.
- A worker is launched only through `t3_thread_launch` with a `worktree`
  workspace strategy, so T3 owns the checkout and the thread's binding.
- A worker owns its PR until the change is delivered, not only merged. A
  failed deploy goes back to it as a follow-up PR.
- Workers run one at a time. Each PR branches from `main` after the
  previous PR has been delivered, and targets `main`. PRs are never stacked
  on one another.
- Handed several issues at once, this thread works them one after another,
  in the order the user gave, each through steps 2 to 5 before the next
  issue's split. Step 5's unpin and settle wait for the last issue. Issues
  the user wants worked in parallel, or a ticket boss, are
  `t3-overseer`.
- This thread stays pinned and active while any worker is open. It settles
  only after the last worker has settled.
- Every human gate surfaces here, as an action line (below). With a
  parent, this thread also sends each one up (`t3wf send --kind gate
  --gate <phase>`).
- Text published under the user's name (the split comment, issue comments)
  follows the Publishing section of the `t3-worker` procedure, at
  `../t3-worker/procedure.md` relative to this skill's base directory.
- Check a worker's claims on GitHub before acting on them.
- Never `sleep` or poll in the shell. Waiting is ending the turn; a
  worker's message, the link watcher, or a scheduled run wakes this thread.
- Every message to a worker goes through `t3wf reply --to <worker id or
  label> --body -`.

## Action lines

A message that needs the user to do something opens with one action line
per action:

`**ACTION NEEDED:** <plain verb phrase>, here: <bare URL>`

For example:

`**ACTION NEEDED:** reply "merge and deploy" to merge #81 and push content/v1.4.2, or review it first, here: https://github.com/acme/widgets/pull/81`

- The verb phrase names what the user clicks, runs, or answers, in plain
  words. This skill's own terms, such as "gate" and "slot", stay out of it.
- The URL is bare text, with no markdown link label, and points at the
  thing to act on: the PR, or the run itself. When the action is a command,
  put the exact command in backticks after `here:`. When the action is a
  reply in this thread, put the URL of what the reply is about, or end the
  line on the verb phrase.
- Context, timings, and evidence come after the action lines.
- A message that needs nothing from the user carries no marker at all.

## On the link protocol

`t3-workflow` asks each role for one thing. This role's is:

- **Resume step:** read the slots again (step 1), `t3wf board` for this
  work's worker threads, and
  re-show any action line the user has not acted on. While a deploy runs,
  check `list_scheduled_tasks` for its deploy watch; when it is missing,
  create it again as "The deploy watch" says, with the original
  `Started` time.

Progress to a parent that needs no action goes up as `--kind note`. With a
parent (an overseer), three more rules hold:

- While an action line waits on the user, send it up with `t3wf send --kind
  gate --gate <phase> --body '<action line>'`, which opens the gate on the
  parent link, and `t3wf gate --clear` once the user acts.
- Send each action line up as one line, with no steps, commands or logs.
  The detail stays in this thread, where the user opens it from the
  overseer's waiting list.
- The user may answer here or through the overseer. A `reply` from the
  parent that relays the user's answer is the user's word, "merge" and
  "merge and deploy" included.

## 1. Start

1. Join the link protocol: the `t3wf join` line when the message that
   started this thread has a "t3-workflow link" block, or `t3wf init --role
   ticket` when it has none. `t3wf whoami` prints this thread's id.
2. `t3_thread_organize` with action `pin`.
3. `t3_worktree_status`: note the main workspace root and the repo
   (`gh repo view --json nameWithOwner,url`). Worktree removal and branch
   cleanup run there.
4. Read the slots with `t3wf slots` from the main workspace root, as
   [Reading slots in a role](../t3-workflow/slots.md#reading-slots-in-a-role)
   says, and send the skip and no-config lines it calls for. The `delivery`
   line is the delivery (`merge`, or `tag-deploy` via a deploy skill) and
   the `model worker` line the worker's model (step 3). A `tag-deploy`
   whose deploy skill is skipped delivers by `merge`.
5. Read the repo's `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `README.md`
   (contributing and versioning), and `docs/agents/`. Settle the
   conventions below, from the repo's `## Agent workflow` section when it
   has one, else by inference. Show every inferred value under the split
   (step 2), with the delivery, so the user's approval confirms it.

   | Setting | Values | Inferred when the repo does not say |
   |---|---|---|
   | Tracker | `github-issues`, `none` | `github-issues` when the user named an issue, else `none` |
   | Merge method | `merge`, `squash`, `rebase` | what the repo documents (`gh pr merge --merge`), else the method its recent merged PRs show |
   | PR format | a skill or template | the `pr` skill when it is available, else `.github/pull_request_template.md`, else none |
   | Checks | commands | the test commands the repo documents |
   | In-progress label | a label | the label the repo documents for work under way, else none |

   Note the identity too: the bot the repo names, or the user's own. And
   whether the repo runs CI on PRs (`ls .github/workflows`, triggers on
   `pull_request`).

## 2. Read the work and split it

With tracker `github-issues`, read the issue:
`gh issue view <n> -R <repo> --json number,title,body,labels,assignees,milestone,comments,url`.
With `none`, the user's request is the work.

Write the split: a numbered list of N PRs, in merge order. Each entry
carries a title (Conventional Commits), the scope, one acceptance line (a
checkable condition the PR makes true), and a branch name: `<type>/<n>-<slug>`
with an issue, `<type>/<slug>` without, where `<type>` is the Conventional
Commits type. Keep N small: a PR is a unit someone can review and revert
alone.

A PR's entry carries a `Passes:` line only when the repo's rules
(`AGENTS.md`, `docs/agents/`) name passes for its kind of change, such as a
content audit for changed factual claims. The line names the change that
earns them and lists the passes in the order the rules give; name any skill
the repo's rules call for that is not installed as skipped. These are this
PR's own passes; the `passes` slot, which every worker runs on what it
publishes, stays out of the line.

Each branch name must be unused. T3 creates the worker's branch and fails
the launch with `branch_already_exists` when the name is taken, and work
that runs more than once reuses slugs easily. A name is taken when any of
these finds it:

- `git -C <main workspace root> branch --list <name>`
- `git -C <main workspace root> ls-remote --heads origin <name>`
- `gh pr list -R <repo> --state all --head <name>`

Give a taken name a numeric suffix (`-2`, `-3`) until all three come back
empty.

Show the split under the action line "approve this PR split, or reply with
changes", and wait for approval. Under the split, list the settings from
step 1 (inferred ones marked), the delivery each PR gets (merge only, or
merge then deploy with the tag family the `delivery` slot's deploy skill
would pick for its paths), each `Passes:` line (approval runs the passes,
but anything they lead to that the repo's rules reserve for a person, and
any GPU use, still comes back as a gate), and, with an issue, what approval also does to it: the
split posted as a comment, the issue assigned to the user when it has no
assignee, and the in-progress label added when the repo has one. The
approval is the user's confirmation of those writes. Then, with an issue:

1. Post the split as an issue comment through the Publishing section
   (`gh issue comment <n> -R <repo> --body-file <file>`).
2. When the issue has no assignee: `gh issue edit <n> -R <repo>
   --add-assignee @me`. Leave an existing assignee alone.
3. When the repo has an in-progress label: `gh issue edit <n> -R <repo>
   --add-label <label>`, and remove any not-started label the repo's
   conventions replace it with.

Workers never write to the issue except through their commit trailers and
`Refs #<n>` lines.

## 3. Launch a worker

For PR n:

1. Run the branch-name check from step 2 again for PR n's branch, because
   other work can take the name between the split and the launch. When it
   is taken now, give it the next free suffix and tell the user. Then fill
   in the launch brief below with the final name.
2. Read the slots again (step 1). The `model worker` line is the
   worker's model: `<model> effort <effort> on <provider>`, each part
   present only when set, or `provider default`. A `PROBLEM:` on it is a
   skip, and the worker gets the provider default.
3. `t3wf open-child --role worker --label '<#n or slug> PR n/N'`. It prints
   the "t3-workflow link" block for the brief. The worker binds the link
   when it joins.
4. Call `t3_thread_launch` with:
   - `title`: `<#n or slug> PR n/N: <PR title>`
   - `modelSelection`: `instanceId` the line's provider (`claudeAgent`
     when it names none), its `model`, and, when it names an effort,
     `options` carrying it in the shape `orchestrator_capabilities`
     reports for that model (today `[{"id": "effort", "value":
     "<effort>"}]`). Leave `modelSelection` out for `provider default`.
   - `workspaceStrategy`: `{type: "worktree", baseRef: "main", branch:
     "<branch>", startFromOrigin: true}`
   - `message`: the filled brief.
5. Read the worker's model and effort with `t3_thread_configure`, passing
   the `threadId` and no `modelSelection`. When they differ from the
   slot's model, call `t3_thread_configure` again with the `modelSelection`
   from item 4 and tell the user.
6. Tell the user the worker is running, by its `link`, verbatim, and end
   the turn.

### Launch brief

```markdown
# <#n: issue title, or the request in one line>

Issue: <issue URL, or "none">
PR <n> of <N>: <PR title>
Acceptance: <the PR's acceptance line>
Branch: <branch>
Delivery: <merge | tag-deploy>. The orchestrator merges and deploys; you
never merge, tag, or deploy.
Later PRs (out of scope): <titles of PRs n+1..N, or "none">
Passes: <the PR's `Passes:` line from the split, in order, or "none">

## Previous handoff
<the previous worker's handoff block, verbatim, or "none">

<the "t3-workflow link" block from t3wf open-child, verbatim>

## Repo rules
Follow AGENTS.md, CLAUDE.md, CONTRIBUTING.md, README.md and docs/agents/ in
this repo. They set the identity, commit trailers, tests, version bump, and
done-checks, and they outrank this brief.
Identity: <the bot the repo names, or "the user's own: <name> <email>,
gh login <login>">.
Checks: <the commands from step 1>.
PR format: <the format from step 1, for example "the `pr` skill">. Write
the description with it, through the Publishing section of the procedure.

## Procedure
Invoke the t3-worker skill and follow the t3-worker procedure
inline, in this thread, from step 1.
```

## 4. Between workers

Every worker message opens with a `[t3wf2` header and needs no command
before you act. A message about a closed link no longer arrives. The body
says which part below handles it: a blocker or gate is the next two
paragraphs, "PR opened", "ready to merge" and "updated" are their sections,
and the `handoff` is "The handoff". A `note` never wakes this thread; it
shows in the digest.

When a worker reports a blocker, relay it to the user as an action line
with the command or link the worker named. When the user reports it
cleared, tell the worker, then end the turn.

Before it opens a PR, the worker runs the `review` slot's reviewer. When
that review gets stuck, or the reviewer cannot run, the worker sends the
open findings with both sides. Relay them under the action line "decide
each open review finding: fix it, or ship as is", or for a reviewer that
cannot run, "open the PR without the independent review, or fix the
reviewer's route". Send the user's answer
to the worker, then end the turn.

### PR opened

First check two things. (a) The worker's message carries a `Publishing:`
line that names the brief's format and each pass. (b) The published
description (`gh pr view <number> -R <repo> --json body`) has every
section and required element the format sets (for the `pr` skill: the
Summary, Evidence and Merge Danger headings, a fenced block under Summary,
`Before:` and `After:` under Evidence, and `Door:` and `Blast Radius:` under
Merge Danger), plus `Refs #<n>` when there is an issue. When either fails,
send the worker "Rewrite #<number>'s description through the Publishing
section of the t3-worker procedure: <what is missing>."

Then tell the user the PR is open, with its URL, so they can review it
while its checks run. When the worker reported "Checks: none in this repo",
go straight to "Ready to merge" below. Otherwise end the turn.

### Ready to merge

A worker's "ready to merge" message, or "opened" in a repo with no CI,
starts the merge gate. Verify on GitHub first:

`gh pr view <number> -R <repo> --json state,isDraft,headRefOid,mergeStateStatus,statusCheckRollup,files`

The PR is open and not a draft, `headRefOid` equals the head the worker
named, every check passed, and `mergeStateStatus` is `CLEAN`. A pending
check is waited out (end the turn; the worker reports again). A failing one
goes back to the worker. In a repo with no CI, run the brief's checks on the
PR head yourself, in a detached temporary worktree that you remove
afterwards (`git -C <main workspace root> worktree add --detach <tmp path>
<head sha>`, run, `git -C <main workspace root> worktree remove <tmp
path>`), and name the result.

Then send the gate:

- Delivery `merge`: "reply \"merge\" to merge #<number>, or review it
  first, here: <PR URL>".
- Delivery `tag-deploy`: "reply \"merge and deploy\" to merge #<number>
  and push <family>/v<next version>, or \"merge\" alone, or review it
  first, here: <PR URL>". Take the family and the next version from the
  deploy skill (its rules for choosing the pipeline and the version,
  applied to the PR's `files`), and say they are recomputed at deploy time.

End the turn.

### On the user's word

**"merge"** (or "merge and deploy"):

1. Run the "Ready to merge" checks again. A head that moved since the gate
   gets a new gate, not a merge.
2. `gh pr merge <number> -R <repo> --<merge method> --match-head-commit
   <sha>`. Never pass `--delete-branch`: it checks out the default branch
   wherever it runs. On a branch with a merge queue this queues the PR, or
   sets it to auto-merge, and the PR merges later.
3. Read where the PR stands, in one query so a merge cannot land between
   two reads: `gh api graphql -f query='query{repository(owner:"<owner>",
   name:"<name>"){pullRequest(number:<number>){state mergeCommit{oid}
   isInMergeQueue autoMergeRequest{enabledAt}}}}'`.
   - `MERGED`: go on to item 4.
   - `OPEN`, and in the queue or with an `autoMergeRequest`: start the
     merge watch (below) and end the turn.
   - `OPEN` with neither: nothing will merge it. Tell the user what
     `gh pr merge` printed, and end the turn.
   - `CLOSED`: send the gate "reopen #<number>, then reply \"merge\" to
     queue it again, here: <PR URL>", and end the turn.
4. Only now that GitHub shows it merged, delete its branch:
   `git -C <main workspace root> push origin --delete <branch>`. Deleting
   the head branch of an open PR closes the PR. The worker's worktree still
   has the branch checked out, so the local branch waits for The handoff.
5. Delivery `merge`: send the worker "Merged #<number> at <merge commit
   sha>. Hand off." and end the turn.
6. Delivery `tag-deploy` with "merge and deploy": go to Deploy. With
   "merge" alone: send the gate "reply \"deploy\" to push
   <family>/v<version> from main at <sha>, or \"skip deploy\", here: <PR
   URL>", and end the turn. "skip deploy" makes this PR's delivery `merge`:
   send the worker the item 5 message, and say in the issue comment that
   it is merged but not deployed.

A user who merged the PR on GitHub themselves counts as "merge": verify
the merged state and continue from item 4.

#### The merge watch

Create it as items 1 and 2 of "The deploy watch" say, with `title` `<#n or
slug> PR n/N merge watch`, and a prompt that names the task id, the PR
number and URL, its head SHA and branch, the main workspace root, the
user's reply ("merge" or "merge and deploy"), and ends with "Follow 'Each
merge-watch run' in the t3-ticket skill."

Each merge-watch run reads the item 3 fields:

- **`MERGED`:** delete the task with `delete_scheduled_task`, then go on
  from item 4.
- **Still queued, or waiting on auto-merge:** end the turn quietly.
- **`OPEN`, out of the queue, with no `autoMergeRequest`:** delete the
  task and send the gate "reply \"merge\" to queue #<number> again, or
  check why it left the queue, here: <PR URL>". End the turn.
- **`CLOSED`:** delete the task and send item 3's `CLOSED` gate. End the
  turn.

### Deploy

Run the `delivery` slot's deploy skill as its entry says and follow it,
with these overrides:

- The user's "deploy" or "merge and deploy" reply in this thread is the
  skill's go-ahead. Show the family, version and SHA it chose; when they
  differ from the gate, say so before tagging.
- The skill's "watch the pipeline to a terminal state" runs as the deploy
  watch below, never as a poll in the shell.

#### The deploy watch

1. Create the task with `schedule_task`: `title` `<#n or slug> PR n/N
   deploy watch`, `schedule` `{type: "interval", everyMs: 300000}`,
   `bindToCurrentThread` `true`, `enabled` `false`, and a placeholder
   `prompt`. The prompt needs the task's own id, which exists only after
   this call.
2. Call `schedule_task` again with the returned `scheduledTaskId`,
   `enabled` `true`, and a prompt that names: the task id, the tag and SHA,
   the worker's thread id, the exact status commands the deploy skill uses
   for this family, what success means (every terminal condition the skill
   names, such as pipeline `Succeeded` and the service roll `COMPLETED`),
   the main workspace root, and the flags "Auth gate sent: no. Slowed:
   no. Started: <UTC time>." End
   it with "Follow 'Each deploy-watch run' in the t3-ticket skill." A
   scheduled run must not look itself up in `list_scheduled_tasks`, so the
   prompt carries everything the next run needs.

#### Each deploy-watch run

- **Status command fails:** read the `credentials` line fresh with `t3wf
  slots --repo <main workspace root> --skills '<this session's skills>'`,
  sending a skip note as
  [Reading slots in a role](../t3-workflow/slots.md#reading-slots-in-a-role)
  says. When the output matches a pattern on it and the prompt says "Auth
  gate sent: no",
  run that renewal skill as its entry says, then run the status command
  again, once. When the renewal needs the user, the retry fails too, or
  the failure is credentials or the network with no pattern (or a skipped
  skill) to renew it, send the action line with the sign-in the renewal
  skill or the repo documents, and update the flag to the UTC time. Once
  the flag is set, renew and send nothing more while the failure lasts.
  End the turn.
- **Still running:** end the turn quietly, after setting "Auth gate sent"
  back to "no" when it holds a time. When it has run 2 hours since
  "Started" and the prompt says "Slowed: no", tell the user once, set
  `everyMs` to 1800000, and update the flag.
- **Succeeded:** delete the task with `delete_scheduled_task`, then send
  the worker "Delivered: <tag> <run URL>. Hand off." and tell the user.
- **Failed:** delete the task, read the failed step's log, tell the user
  what failed with its URL, and send the worker "Deploy failed: <tag> <run
  URL>: <what the log shows>. Fix it in a follow-up PR (step 5)." Nothing
  else moves until the worker hands off. The follow-up PR runs through
  "PR opened" and the gates again. Rollback is the user's call; name the
  skill's rollback path in the message, and do not run it unasked.

### The handoff

The worker hands off only after "Merged" or "Delivered". Check its result;
do not detect it here for the first time. The handoff has already closed
the worker's link.

1. Confirm the PR is merged on GitHub: `gh pr view <number> -R <repo>
   --json state,mergeCommit`, and the same for any follow-up PR.
2. For `tag-deploy`: the tag contains the merge commit (`git -C <main
   workspace root> fetch origin --tags`, then `git merge-base --is-ancestor
   <merge sha> <tag>`), and the run the handoff names succeeded, as the
   deploy watch saw it. When the check disagrees with the handoff, tell the
   user and the worker before anything else moves.
3. With an issue: comment on it with the PR, any follow-up PR, what it
   changed, and the delivery result. Publish through the Publishing section.
4. Remove the worker's worktree from the main workspace root once GitHub
   shows the PR merged: `git -C <main workspace root> worktree remove
   <worktree path>`. The path is in `t3_worktree_list`. When git refuses
   because the worktree has changes, leave it and tell the user what is in
   it. Then delete the PR's local branches (the follow-up's too):
   `git -C <main workspace root> branch -D <branch>`. `-D` is safe because
   GitHub's merged state, checked in item 1, says the work landed; a squash
   merge would make `-d` refuse. Delete a branch only after its worktree is
   gone. Then sweep the review clones the reviewer left behind. They sit on
   `t3/*` branches, so find them by thread, not branch name:
   `t3_thread_list` with `allProjects` and `includeSubagents` both `true`
   (page with `cursor`), keeping the threads whose `parentThreadId` is the
   worker's thread id and whose `relationshipToParent` is `subagent`.
   `t3_thread_read` each one (`limit: 1`) for its `worktreePath` and
   `branch`. For each path still in `git -C <main workspace root> worktree
   list`, run `git -C <main workspace root> worktree remove <path>` (no
   `--force`). Then, after `git -C <main workspace root> fetch origin`,
   delete its branch only when `git -C <main workspace root> merge-base
   --is-ancestor <branch> origin/main` succeeds: `git -C <main workspace
   root> branch -D <branch>`. The ancestor check stands in for `-d`, which
   judges against the main checkout's local `main` and refuses while that
   lags the merge. When git refuses the removal or the check fails (a
   squash merge), leave it and tell the user the path or branch and what
   git said.
5. Launch the next worker (step 3) with this handoff as its previous
   handoff.

## 5. Close

After the last PR is delivered:

1. Check `t3wf board`: every worker thread for this work is settled. A
   message to a worker after its handoff wakes it and leaves it unsettled.
   Settle each such worker with `t3_thread_organize`, action `settle`, once
   its turn has ended (`t3_thread_wait`).
2. `list_scheduled_tasks`: none of this work's deploy watches remain.
   Delete any that do. Then `t3wf sweep`: it names any worker link still
   open.
3. Run `git -C <main workspace root> worktree list` and look for a
   worktree on any of this work's branches, or bound to any worker's
   subagent child thread (found as "The handoff" item 4 says). A message to
   a worker after its handoff makes T3 recreate its worktree, so a worktree
   removed at the handoff can come back. Remove each one as "The handoff"
   item 4 says.
4. With an issue: remove the in-progress label, then close it with the
   outcome as the comment, through Publishing: `gh issue close <n> -R
   <repo> --comment "<text>"`.
5. With a parent link: `t3wf send --kind handoff --body -` with the
   outcome and the PR URLs.
6. Unpin this thread and settle it.
