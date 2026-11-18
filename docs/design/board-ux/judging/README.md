# Blind judging

Each round scores one build of the fixed panel board against the board at
`7e29c9d`, following the protocol in [`../rubric.md`](../rubric.md). Three
judges (Claude Opus 5.5, GPT 6.2 Sol, Claude Sonnet 5.5) each scored a
separately blinded packet; the gate scripts in [`../harness/`](../harness/README.md)
measured the rest. Commits are named as they were before the branch was
squashed.

| Round | Judged build | Result | Judged criteria short of the rule | Gates failed or not measured |
| --- | --- | --- | --- | --- |
| [1](round-1.md) | `b36808e` | not met | P5, P6, N3, N4 | F1 notification, 15-decision load |
| [2](round-2.md) | `8c159f5` | not met | N3, N4 | F1 notification, 15-decision load |
| [3](round-3.md) | `7b0c8c2` | not met | P6, N3 | F1 notification, 15-decision load |
| [4](round-4.md) | `e50b6d1` | not met | P5, N3 | U3 not measured; F1 notification, 15-decision load |
| [5](round-5.md) | `6f10d7d` | not met | P6 | P2 failed; F1 notification, 15-decision load |
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
