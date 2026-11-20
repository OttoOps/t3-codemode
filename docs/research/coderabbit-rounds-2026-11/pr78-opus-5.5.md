# PR #78 "feat: reconcile tracked PRs with GitHub" - what the review rounds did

Reviewer: independent (Opus). Scope: PR #78 only. Times are MST.

## Verdict in one paragraph

The original submission (2d7d2e7) was a coherent, working design with 20 product-level
tests. Round 1 (fixes in 8ad3a16) was a small net positive. After that, the review
loop never converged: CodeRabbit posted 15, 9, 7, 5, 2, 6, 4, 4 and 6 findings and
requested changes on every one of the 9 heads. Most later fixes harden races that
take seconds to happen, or edge cases outside how this system is actually used. The
clearest cluster comes from one design choice: the background path calls `wf.merge`,
which re-reads GitHub and decides again. That gave CodeRabbit an endless supply of
"batch read says X, fresh read says Y" scenarios, and the agent answered each one by
adding a new control field to `wf.merge`'s public result. The production code grew
moderately (reconcile.rs 572 -> 750 lines, `wf.merge` 124 -> 152 lines). The tests grew
2.7x (20 -> 54), and 34 of those tests are named after a specific CodeRabbit finding.
There are no new tables or columns after the original. Efficiency did not change.
**Resubmit from 8ad3a16 (the round-2 head) plus 3-4 short rewrites listed in Q6.**

## Method

- Diffed every reviewed head against the next. Rounds 1-8 share merge-base 8e179c3. The
  brief says round 9 shares it too, but 5d91438 is already rebased onto c29bc71
  (`git merge-base 5d91438 c29bc71` = c29bc71). For 829a732 -> 5d91438 I diffed only the
  PR-owned files and used `git range-diff` for the files main also touched.
- I mapped each finding to the agent's reply ("Fixed in <sha>" or a pushback) from
  cmt78.json. The fix SHAs are intermediate commits, so round N's findings land in the
  diff from head N to head N+1. One exception: the round-4 findings (a3e06fd) landed in
  a42ec46, together with round 5's. The a3e06fd -> 08cdcf4 diff answers CodeRabbit's
  follow-up replies on earlier threads.
- Built detached worktrees (since removed) and ran `cargo test --test reconcile`:
  - each head's own suite passes: 2d7d2e7 20/20, 8ad3a16 24/24, b366675 54/54.
  - **I ran the final head's 54 tests against older code.** The two test-only fake
    hooks from a86bd34 (`omit_from_reconcile`, `update_pr_after_reconcile`) were patched
    in. 8ad3a16 fails 30/54 and 2d7d2e7 fails 32/54. So rounds 1-9 changed 30-32
    observable behaviors, and only 2 of them separate the original from the round-2
    head. The 20 original tests are byte-identical in the final head, so no round
    weakened them.

## Q1. Round-by-round character

Classes: (a) real defect, realistic trigger; (b) improbable race/edge hardening;
(c) reviewer-satisfaction (docs hedging, scenario special-casing, scenario tests);
(d) new defect or regression. The percentages are rough shares of the changes in each
fix diff.

| Fix diff (answers) | a | b | c | d | Clearest examples |
|---|---|---|---|---|---|
| 2d7d2e7 -> 8ad3a16 (R1) | ~30% | ~50% | ~20% | 0 | (a) `wf.merge` stored `MERGED` before refusing an older head, which swallowed the later ticket event (merge.rs:110-116@2d7d2e7, fixed by moving `note_seen` past the head check). (a) Read `pr.url` when T3 omits `pullRequests` (reconcile.rs:230-236@8ad3a16). (a, hacky) A ticketless report no longer shows dead reply buttons, but `unanswerable()` re-parses the prose the code itself just formatted (reconcile.rs:63@8ad3a16). (b) `at_host_start`, the GraphQL `Int` bound, `done_waiting` conditional delete. (c) Docs on the 100-PR batch and partial-failure semantics. The cooldown-under-lock fix merged three duplicated entry points into `reconcile_after`, a net simplification. |
| 8ad3a16 -> d9a26ef (R2) | ~25% | ~50% | ~12% | 0 | (a) **Per-PR error isolation.** In 8ad3a16 a `?` (reconcile.rs:212) aborts the whole pass on one PR's error, and BTreeMap order makes that same PR fail first on every later pass, so it blocks every PR after it. (a/b) A moved-head gate supersedes an unrelated gate on the same ticket. That is a real board rule (`board_card`, board.rs:627-633), but the fix silently changes `raise(kind="gate")` into a report (reconcile.rs:589@d9a26ef) and was reworked twice afterwards. (b) Null timeline, `answerable()`, restoring the wait when the board is full (36 slots, board full is implausible here). |
| d9a26ef -> a3e06fd (R3) | 0 | ~25% | ~50% | 0 | (c) `fire = p.state != state \|\| (!merged && waited)` (reconcile.rs:355@a3e06fd) plus a docs row that spells out the reviewer's exact sequence: "one more if the pull request was reopened, approved again and closed again between two checks" (board.md:211@a3e06fd). (b) "Queued on GitHub at another head" report. |
| a3e06fd -> 08cdcf4 (follow-ups) | 0 | ~30% | ~70% | 0 | Second rewrite of host matching, and the `merging` refinement of the gate downgrade. This is churn on code written one round earlier. |
| 08cdcf4 -> a42ec46 (R4+R5) | 0 | ~70% | ~15% | design debt | (b) Rate-limit throws to keep the approval, the full action line rather than the 400-char card text, a null removal reason, saving the link after the card. **First control field added to `wf.merge`'s public result:** `state: "MERGED"` (merge.rs@a42ec46), so `advance_wait` can tell one fresh-read outcome apart. |
| a42ec46 -> 829a732 (R6) | ~10% | ~70% | ~20% | design debt | (b, real-ish consequence) Skip pruning when any worker's T3 PR list is stale (reconcile.rs:228-231@a86bd34). (b) Fresh read finds CLOSED or a moved head, which adds the `state: "CLOSED"` and `now` fields. (b) Omitted batch alias, empty action marker. (c) `rfind` -> `find` in host matching, the third iteration. |
| 829a732 -> 5d91438 (R7) | ~20% | ~40% | ~25% | ~15% | (a, minor) Clear the ticket's `merge` gate when a GitHub merge resolves its gate card. That is realistic: it is the feature's main use case, a merge done by hand on GitHub. (d) The same change clears the gate when *any* resolved card belongs to the ticket, including a report card (reconcile.rs:378@5d91438). Flagged in R9 and fixed only in the unreviewed final. (c) Case-insensitive `GitHub.com`, although every URL here is generated lowercase by T3 or gh. (b) `DRAFT` state field. |
| 5d91438 -> b366675 (R8+R9, never reviewed) | ~10% | ~60% | ~20% | ~10% | (a) Fixes the R7 over-reach (gate cards only, reconcile.rs:393@b366675). (b) Every action line, removal before the first check, a gate and a wait at one moved head, "briefly blocked after queue turned off" (adds the `mergeState` field), queued-elsewhere from the fresh read (adds the `queued` field). (c) Docs: "Those are minimum gaps: the real gap can be up to one watcher interval longer" (board.md:204@b366675). (d, minor) Any read error other than NOT_FOUND/FORBIDDEN now throws (merge.rs:100@b366675). In the background path a persistent read error therefore turns a visible report card into a silent retry every 2 minutes. This contract change to `wf.merge`, which every agent calls, went out in a head nobody reviewed. |

Across all rounds, roughly 15% of the changes fix realistic defects, about half are
improbable-race hardening, about a quarter is reviewer-satisfaction, and a few percent
are new defects. Several fixes needed fixes of their own:

- Host matching was written 4 times: 8ad3a16, 08cdcf4, 829a732, then 5d91438 and the final.
- The gate-to-report downgrade was written 3 times: d9a26ef, 08cdcf4, a42ec46.
- The action-line extraction was written 3 times: a42ec46, a86bd34, b366675.
- The gate-clearing change from R7 needed a fix in the final head.

## Q2. Code quality: original -> after round 1 -> final

| Metric | 2d7d2e7 | 8ad3a16 | b366675 |
|---|---|---|---|
| reconcile.rs lines / fns | 572 / 24 | 605 / 27 | 750 / 32 |
| `fix` lines / branch tokens* | 54 / 18 | 54 / 18 | 71 / 33 |
| `finished` | 46 / 18 | 46 / 18 | 62 / 30 |
| `advance_wait` (+ `wait_moved`, `wait_queued_elsewhere`) | 87 / 22 | 90 / 23 | 87+22+30 / 35 |
| `raise` | 45 / 16 | 48 / 18 | 61 / 24 |
| `Wf::merge` | 124 / 48 | 131 / 52 | 152 / 57 |
| store API added | 6 fns | 8 fns | 9 fns |
| tables / PrSeen fields | +1 / 9 | same | same |
| `wf.merge` refusal fields | `reason` | `reason` | `reason`, `state` (MERGED/CLOSED/DRAFT), `now`, `queued`, `mergeState` |
| tests | 20 | 24 | 54 |

*Branch tokens = a crude count of `if`/`match`/`=>`/`?`/`&&`/`||`. Use it only to compare heads.

- **Original.** Clean structure: one batch read, and each PR's outcome dispatched
  through `fix` -> `finished` / `dropped` / `check_gate` / `advance_wait`. It has two
  weak points that drove most of the later churn:
  1. `pr_seen` is both the observation cache and the "event already sent" ledger, and
     two writers update it without a shared lock: the pass and `wf.merge`
     (merge.rs:110-178@2d7d2e7).
  2. `advance_wait` hands the decision to `wf.merge`, which reads GitHub again
     (reconcile.rs:454@2d7d2e7).

  Compare-and-swap by whole-row JSON equality (`pr_seen_swap`, `merge_wait_remove_if`)
  works, but it is an awkward identity scheme.
- **After round 1.** Same shape, +33 lines. `reconcile_after` is better than the three
  duplicated cooldown checks. `unanswerable()` is a hack. Net mildly positive.
- **Final.** No schema growth, but coherence got worse:
  - `fix`'s OPEN branch now coordinates a drop report, a gate replacement and the wait
    through returned flags (`reported`, `replaced`) and a guard
    (`reported || (replaced && head != w.head)`, reconcile.rs:361-367@b366675).
  - `raise` quietly changes the card kind it was asked for (reconcile.rs:692-703@b366675).
  - `advance_wait` reads stringly-typed control fields off `wf.merge`'s user-facing JSON
    (reconcile.rs:543-551@b366675), so a public API now doubles as an internal signal
    channel.
  - Each new fresh-read scenario needs a matching new field in merge.rs and a new arm
    in reconcile.rs, which is the coupling that drove the whack-a-mole.
- **Tests.** Good style throughout: real integration through the fake GitHub, no
  mocking of internals. But 34 of the 54 final tests restage a reviewer scenario, and
  the fake gained hooks only to stage races between the batch read and the fresh read.
  They pin behavior nobody will hit and make the design more expensive to change.
- **Overall.** Better through round 1, then worse: more special cases and more
  coupling, with only a handful of real gains.

## Q3. Efficiency

No meaningful change.

- **GitHub calls.** Identical in every head: `ceil(N/100)` batch queries per pass, plus
  one `wf.merge` (1-3 reads + 1 mutation) per waiting PR that turned mergeable at its
  approved head.
- **Cadence.** Unchanged: 2 minutes while anything waits or is queued, 10 otherwise.
- **Retries.** From R5 on, transient errors keep the wait and retry at that cadence. In
  the final head a persistent non-NOT_FOUND read error retries forever, about 30
  GraphQL calls an hour. That is harmless against the 5,000-point budget, but silent.
- **DB work.**
  - Two queries got cheaper: `pr_seen_any_queued` instead of a full scan
    (store.rs@a3e06fd), and `merge_wait_get` by primary key (store.rs@d9a26ef).
  - Some work went up: `card_prs` now runs `msg_get` for every open card on every pass,
    and `raise` and `finished` re-read `cards()`.
  - With at most 36 cards and a handful of PRs, that is tens of local SQLite point
    reads per pass. Negligible.
- **Locks.** The global wf lock was taken at 5 sites in the original and 6 in the final.
  Both hold it across `send_wake` (T3 delivery) in `finished`, `raise` and
  `advance_wait`, which matches the existing pattern elsewhere in the codebase
  (board.rs:684-699@8e179c3).
- **Watcher pass.** Reconcile still runs inline in the watcher pass in every head.
- **Verdict.** Flat, so efficiency is not an argument for or against any head.

## Q4. What reverting to 8ad3a16 loses

Real defects that later rounds fixed, in rough order of value:

1. **One failing PR blocks every PR after it, on every pass**: the `?` at
   reconcile.rs:212@8ad3a16. Severity medium, likelihood low: it needs a persistent
   per-PR failure, such as a store error or a full board. Fixed in d9a26ef with about
   10 lines. **Keep.**
2. **The ticket's `merge` gate stays set after a GitHub merge resolves its gate card.**
   That suppresses stall reports and nudges until the ticket sets another gate or
   closes. Severity low, likelihood moderate, because "merged by hand on GitHub" is the
   feature's purpose. Fixed in 5d91438 with a regression, and corrected in b366675
   (reconcile.rs:391-405). **Keep the final version.**
3. **A stale T3 worker PR list prunes the OPEN row, so a merge on the next good pass is
   a "quiet first observation" and the ticket never gets its post-merge event.**
   Severity medium (a lost event), likelihood low. The a86bd34 fix (skip all pruning if
   any worker is stale) plus the 5d91438 snapshot fix works, but it is roundabout.
   **Rewrite simply** (Q6).
4. **The background path turns any fresh-read disagreement into a lost approval plus a
   generic report.** The disagreements are a rate limit, a transient error, or the PR
   merged, closed, moved, drafted or blocked within the 1-3 s window. Severity low:
   the user sees a report and can re-approve. Likelihood low, except rate limits on a
   gh token every agent shares. **Rewrite simply** (Q6) rather than taking the five
   signal fields.
5. **Unrelated-gate supersede on multi-PR tickets** (d9a26ef, 08cdcf4, a42ec46).
   Severity low-moderate, likelihood low: it needs one PR waiting and another PR's
   gate open on the same ticket. The root cause is the board's one-gate-per-ticket
   rule, not reconcile. Optional; if kept, fix it in `board_card` (supersede only gates
   for the same PR), not by changing the card kind inside `raise`.

Everything else after round 2 is (b) or (c) and can go: host case, every action line,
empty marker, 400-char truncation, null reason or timeline, re-close after
re-approval, queued-elsewhere, removal before the first check, a gate and a wait at one
head, blocked after the queue was turned off, the draft ordering, and the board-full
restores.

**Defects in the original that no round fixed:**
- **The agent's deferred "known limits"** (PR body items 1-4): same-second
  re-approval, unlocked `pr_seen` writes in `wf.merge`, crash windows between the card
  resolve, the event and the row write, and per-PR (not per-ticket) delivery. All are
  (b) for one maintainer and one instance. They are also exactly what the design
  changes in Q6 remove.
- **A queued PR that later returns NOT_FOUND** (repo deleted or access lost). Its
  `queued` row is never updated or pruned (prune skips queued rows,
  reconcile.rs:231-235@b366675), so the PR is queried forever and keeps the cadence at
  2 minutes forever. Low severity, rare.
- **The batch fragment lacks `id` and `isMergeQueueEnabled`.** The background path
  therefore needs `wf.merge`'s extra read at all, which is the source of the whole
  fresh-read class (PR_FIELDS, reconcile.rs:17-20@2d7d2e7). `MergeWait` already stores
  `pull_request_id`.

## Q5. Gate-chasing

Yes, though not dishonestly.

- **Pushback.** The agent pushed back or deferred on 15 of 57 findings, and those calls
  were mostly right:
  - kept the digit rule for SHAs;
  - kept `wf.merge` for queue-disabled repos;
  - deferred the crash windows and multi-ticket delivery as design work.
- **What it fixed.** It fixed everything else that could be fixed locally, including
  findings with no realistic trigger here. Its line was not "does this matter for this
  system" but "can this be fixed without a schema or transaction change". So it
  deferred some races between the batch read and `wf.merge`'s write as known limits,
  while fixing the same kind of race between the batch read and the fresh read. That
  is an inconsistent bar.
- **Signs of writing for the reviewer:**
  - Docs rows that restate the comment's scenario (board.md:211@a3e06fd;
    board.md:213@b366675 "including a removal after the approval of a waiting record
    the board had not checked yet").
  - One test per finding, named after it (34 tests).
  - The PR body's Validation section reads like a CodeRabbit checklist.
  - One public-API field per scenario on `wf.merge`.
  - Replies to every thread, down to "Noted. The follow-up issue will say...".
- **Pace.** Fixes landed about every 15-20 minutes from 14:43 to 17:23 MST, the last
  push 2 minutes before the PR closed (17:25 MST). That pace leaves no room for design
  reflection.
- **Should it have pushed back more?** Yes, on most (b) and (c) items. More
  importantly, by round 3 or 4 it should have named the repeating batch-vs-fresh-read
  pattern as a design problem and changed the design. Instead it added another field.

## Q6. Recommendation

**Base: 8ad3a16.** Round 1 is a small net gain, and the final head's tests show that
the original and 8ad3a16 differ in only 2 behaviors, both improvements. Not the final
head: it carries the signal-field coupling, the hidden kind downgrade and an
unreviewed `wf.merge` contract change, for mostly improbable cases.

Cherry-pick or rewrite on top of it, each small:

1. **Per-PR error isolation in the reconcile loop.** Take reconcile.rs:209-230 from
   d9a26ef as is, plus the 08cdcf4 line that logs partial actions.
2. **Clear the ticket's `merge` gate when reconcile resolves that ticket's merge gate
   card.** Take the b366675 version (reconcile.rs:391-405, gate cards only), not 5d91438.
3. **Stop pruning OPEN `pr_seen` rows from the per-pass tracking snapshot.** Delete the
   prune loop (reconcile.rs:216-220@8ad3a16), or prune only rows in a terminal state or
   older than N days. That removes the stale-T3 lost event (a86bd34), the
   snapshot-mismatch race (5d91438) and the deferred "queue exit loses tracking" limit,
   with fewer lines than any of those fixes.
4. **Background advancement without a second decider.**
   - Add `id isMergeQueueEnabled` to `PR_FIELDS`.
   - When the batch shows the PR open, mergeable, not draft, unqueued and at `w.head`,
     send `ENQUEUE_PR`, or `MERGE_PR` when the queue is off, using `expectedHeadOid =
     w.head`. GitHub then enforces the head match atomically.
   - On a transport or rate-limit error, or any mutation error, keep the wait; report
     only a definitive mutation refusal.
   - Every other state (merged, closed, moved, draft, blocked) is already handled by the
     next pass's existing branches.

   This drops every fresh-read finding (R4-R9, about 10) and all five refusal fields,
   saves 1-3 reads per advancement, and leaves `wf.merge` unchanged for agents.

**The simpler design that removes the race class**, for the follow-up rather than this
PR:

- **One decider, one writer.** Only the reconcile pass writes `pr_seen`. `wf.merge`
  records intent in `merge_waits` (for example a `queued` status) instead of writing
  `pr_seen`. The pass does its GitHub reads outside the lock. It then applies each PR
  under the wf lock in one SQLite transaction: resolve cards, insert outbox events,
  update the row. That closes known limits 2 and 3.
- **A separate notification ledger** keyed `(repo, pr, terminal_state, ticket)` with a
  unique constraint. That gives idempotent per-ticket delivery and closes known limit 4.
  `pr_seen` stays a pure observation cache.
- **An integer revision on `merge_waits`** instead of JSON-equality CAS. That closes
  known limit 1.
- **Structured PR refs on cards.** Store `repo/pr/head` in the card data when the gate
  is raised, instead of regexing prose. That removes the whole class of host, case,
  action-line and truncation findings (R1, R4, R6, R7, R8). It touches the gate
  protocol, so treat it as separate work.

## Q7. Other notes

- CodeRabbit never approved: all 9 reviews were CHANGES_REQUESTED, and the findings
  per round were not trending to zero (round 9 had 6). The loop had no natural end.
  Given this design, more rounds would have kept producing adjacent scenarios.
- The PR already went through 5 Sol rounds before CodeRabbit (PR body), and the
  original still had 15 CodeRabbit findings. Most were (b). Reviewer volume says little
  about defect density here.
- Live-path validation was thin and never grew: the PR body says the enqueue and merge
  mutations never ran against live GitHub, and the service never ran against a live
  T3. Not one round asked for that, though it is the riskiest unverified part.
- Uncertainty:
  - How often T3 sets `pullRequestsStale`, and how often the gh token hits rate
    limits, drive the likelihood of Q4 items 3 and 4. I could not measure either.
  - The board-full likelihood assumes the 36-slot waiting list (`waiting_ids`,
    wf.rs:25-27).
  - The per-round percentages are judgment calls on small numbers of changes.
