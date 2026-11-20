# Blind judging

Each round scores one build of the fixed panel board against the board at
`7e29c9d`, following the protocol in [`../rubric.md`](../rubric.md). Three
judges (Claude Opus 5.5, GPT 6.2 Sol, Claude Sonnet 5.5) each scored a
separately blinded packet; the gate scripts in [`../harness/`](../harness/README.md)
measured the rest. Commits are named as they were before the branch was
squashed.

| Round | Judged build | Result | Judged criteria short of the rule | Gates failed or not measured |
| --- | --- | --- | --- | --- |
| [1](round-1.md) | `b36808e` | not met | P5, P6, N3, N4 | partly measured: U1 (15-decision load), U10 (restore on close), F3 (completed Ask answer), F1 (notification) |
| [2](round-2.md) | `8c159f5` | not met | N3, N4 | partly measured: U1, U10, F3, F1 as in round 1 |
| [3](round-3.md) | `7b0c8c2` | not met | P6, N3 | partly measured: U1, U10, F3, F1 as in round 1 |
| [4](round-4.md) | `e50b6d1` | not met | P5, N3 | U3 not measured; partly measured: U1, U10, F3, F1 as in round 1 |
| [5](round-5.md) | `6f10d7d` | not met | P6 | P2 failed; partly measured: U1, U10, F3, F1 as in round 1 |
| [6](round-6.md) | `862f206` | not met | N3 | no gates measured |
| [7](round-7.md) | `edf2ff3` | met | none | none on the final build |

The candidate beat the baseline on every axis in every round. From round 4
on, each judged miss was one criterion falling under the reference median by
one judge's point, and the reference medians themselves moved between rounds
(P6 ran from 4.00 to 4.67 on the same reference images), so a round's judged
misses say more about where the bar sat than about the build. Gate failures
and missing measurements are listed separately in the last column.

The judges saw `edf2ff3`. The shipped board adds two sets of changes to it:
the phone card header drops a `-12px` margin that failed P2, for on-scale
padding that renders the same, and the fixes from code review, which change
behavior under live updates but not the stills. Round 7's gate table is
measured on that final build, named in the report, and `final/` holds stills
of it.

## Issue #67

Rounds 8 to 10 score the readable board against `main` at `9f8ad45`, with
the same judges and protocol, plus G5 on the live replay. The judged
criteria are #28's and G5. The task test (G1), the grayscale check (G3)
and the other Glanceability gates are measured.

| Round | Judged build | Result | Judged criteria short of the rule | Gates failed or not measured |
| --- | --- | --- | --- | --- |
| [8](round-8.md) | `0c85189` with fixes | not met | P5, N4, N5 | none |
| [9](round-9.md) | round 8 with its evidence applied | not met | N3, N4 | heavy U1 not measured |
| [10](round-10.md) | round 9 with the label strip | not met | N3, N4 | none on the shipped build, `59b51ca` |

The candidate beat the baseline on every axis in every round. Rounds 9
and 10 missed only on N3 and N4, each under the reference median: N3 by
0.17, one judge's point, and N4 by 0.33. The judges' evidence for both
comes down to two choices the issue's owner settled before this PR:
tickets that need the user stay in both the Needs you panel and the
Tickets list, and the board rewrites no ask beyond stripping the action
line's scaffolding, so a card whose ask names its replies repeats them on
its buttons.

## Issue #93

The Dynamic layout criteria (L1 to L13) judge moving, collapsing and
resizing panels against best-in-class dynamic-panel apps, not against a
baseline board, since `main` cannot move panels at all.

| Round | Judged build | Result | Judged criteria short of the rule | Gates failed or not measured |
| --- | --- | --- | --- | --- |
| [1](layout-round-1.md) | `1d9e785` | met | none | none |
