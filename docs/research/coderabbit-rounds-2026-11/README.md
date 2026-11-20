# What repeated CodeRabbit rounds did to PRs #78 and #80

November 2026. Two PRs ran through repeated CodeRabbit review rounds. After
each round the worker agent fixed every finding and force-pushed, which
started the next round. #78 ran 9 rounds and never converged. #80 ran 3
rounds before both PRs were closed and their branches reset to the head
after the first round. Two independent reviewers, GPT-6.2 Sol and Opus 5.5
(both at extra-high reasoning effort), then assessed each PR separately from
the same brief.

## Why the rounds never ended

- Every PR here is authored by the maintainer's account, so CodeRabbit was
  the only reviewer that could approve one. Its automatic approval needs a
  reviewed latest commit with no blocking findings. Minor findings blocked,
  and every push dismissed approval and started another review.
- `auto_pause_after_reviewed_commits: 0` had turned off CodeRabbit's pause.
- The worker procedure fixed every comment and pushed, with no severity
  triage and no round limit.
- Each incremental review re-reads every file the push touched, so each
  round is a fresh sample of edge cases, not a checklist that runs out. On
  #78, 22 of the 42 later-round findings were on lines already present in
  the round-1 commit. 12 were on code the previous fix had just written.

## Size of each change (added lines against its own merge-base)

| PR | Original | After round 1 | Final |
|---|---|---|---|
| #78 | 1372 (`2d7d2e7`) | 1511 (`8ad3a16`) | 2326 (`b366675`) |
| #80 | 1417 (`f2356e1`) | 1617 (`b962714`) | 1850 (`d8ae5c4`) |

## Where the four reviews agree

- Round 1 was a small net gain on both PRs.
- After round 1, quality declined. #78 added control fields to `wf.merge`'s
  result for races lasting 1-3 seconds, rewrote several fixes three or four
  times, and introduced bugs in rounds 7 and 9. #80's updater grew from 498
  to 783 lines, and its restart and rollback path has no tests at any head.
- Efficiency barely changed. GitHub request counts stayed flat.
- From about round 2, much of the work served the reviewer more than the
  product. Docs and tests restate the reviewer's exact scenarios. Opus
  counted 34 of #78's 54 final tests named after a specific finding.
- Resubmit each PR from the head after round 1, with a short list of later
  fixes ported or rewritten simply. Each report lists them.

## Files

| File | Contents |
|---|---|
| [brief.md](brief.md) | The brief all four reviewers worked from |
| [pr78-gpt-6.2-sol.md](pr78-gpt-6.2-sol.md) | #78, GPT-6.2 Sol |
| [pr78-gpt-6.2-sol-regression-tests.rs](pr78-gpt-6.2-sol-regression-tests.rs) | Three tests that reproduce two regressions and one tracking loss present since the original |
| [pr78-opus-5.5.md](pr78-opus-5.5.md) | #78, Opus 5.5 |
| [pr80-gpt-6.2-sol.md](pr80-gpt-6.2-sol.md) | #80, GPT-6.2 Sol |
| [pr80-opus-5.5.md](pr80-opus-5.5.md) | #80, Opus 5.5 |

The review data the brief points to (CodeRabbit's findings and the author's
replies) came from the GitHub API for each PR and is not stored here. No
branch points at most of the commits the reports name, but GitHub still
served each one by SHA when this was written.
