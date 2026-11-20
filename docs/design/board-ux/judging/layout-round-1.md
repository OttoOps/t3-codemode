# Dynamic layout, round 1

Issue [#93](https://github.com/OttoOps/t3-codemode/issues/93): moving,
collapsing and resizing the board's panels at desktop widths. Judged build:
`1d9e785`. Criteria: L1 to L13 in the [rubric](../rubric.md#dynamic-layout).

## How the round ran

- **Gates.** [`harness/layout-gates.js`](../harness/layout-gates.js) on the
  live replay ([`../live/`](../live/README.md)), against a second replay of
  `main`'s page for L5. Raw output: [`layout-1/gates.json`](layout-1/gates.json).
- **Stills.** [`harness/layout-stills.js`](../harness/layout-stills.js) on the
  same replay: [`layout-1/stills/`](layout-1/stills/).
- **References.** The sixteen images in
  [`../references/panels/`](../references/panels/README.md), grouped into five
  subjects: VS Code for the Web, Dockview, Golden Layout, Grafana and
  JetBrains IDEs.
- **Judges.** Claude Opus 5.5, Claude Sonnet 5.5 and GPT 6.2 Sol (high
  effort), each with its own packet from
  [`harness/layout-packets.py`](../harness/layout-packets.py): six subjects
  under random letters in a random order, images renamed and captioned by
  state only, the candidate's title and heading blanked. No judge was told
  which subject was under test. Their replies are in
  [`layout-1/`](layout-1/), and the letter key is
  [`layout-1/key.json`](layout-1/key.json).

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| L1 | pass | 30 of 30 pointer drags (5 panels, top and bottom of 3 columns) landed where dropped; by keyboard alone every panel reached all three columns, and focus stayed on its grip |
| L2 | pass | the chevron collapsed by click and expanded by Enter, `aria-expanded` in step; a header double-click collapsed Activity; `g a` expanded it |
| L3 | pass | a 100 px column-divider drag gave +100 px, a 60 px panel-divider drag +60 px, arrow keys 24 px; at 1200x700, 1280x800, 1440x900 and 1920x1080, both dividers dragged to either extreme kept every column at 320 px or more, and with all five panels in one column the page did not scroll and no heading or control was cut |
| L4 | pass | a moved and collapsed layout survived reload; Reset layout showed only when needed, restored the default and cleared the saved value; malformed JSON, a non-object, a duplicated panel and a negative width all loaded the default |
| L5 | pass | at 1024x768, 768x1024 and 390x844, light and dark, every rendered element's box matched `main` (324, 124 and 124 elements), with and without a saved custom layout; narrow and back restored the saved layout |
| L6 | pass | with Activity scrolled to 300 px and an item focused, dragging Needs you into Activity's column and dragging Activity to another column both kept the offset and the focus; after a column resize and a collapse the first visible Activity item was unchanged |
| L7 | pass | 14 grips, chevrons and dividers named, chevrons with `aria-expanded`, dividers `role="separator"` with `aria-orientation`; one announcement each for a keyboard move, a collapse and a cancelled drag; mid-drag showed the preview, marker and landing area, and Esc moved nothing; icons 6.40:1 light and 7.74:1 dark, dividers 3.25:1 and 3.66:1 |

The board's existing gates also ran on the seeded demo
([`../harness/gates.js`](../harness/gates.js)), in the default layout:
U1, U2, U5 to U10, P1, P3, N1, N2 and F4 match round 10. P2 lists one
off-scale value, `.nrow` `row-gap: 2px`, which comes from `main`'s New
issues rows, not from this change.

## Judged scores

Means across judges; the candidate's per-judge scores in brackets. Each
reference column is that subject's mean; `n/a` means none of its images
can show the criterion. The bar is the median of the scored references.

| Criterion | Candidate (opus/sonnet/sol) | dockview | goldenlayout | grafana | jetbrains | vscode | Bar | Holds |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| L8 | 4.33 (4/5/4) | 3.00 | n/a | 2.67 | 3.00 | 2.67 | 2.83 | yes |
| L9 | 5.00 (5/5/5) | 3.33 | 3.33 | 4.00 | 4.00 | 3.33 | 3.33 | yes |
| L10 | 4.33 (4/5/4) | 3.33 | n/a | 3.33 | n/a | 3.67 | 3.33 | yes |
| L11 | 4.67 (4/5/5) | 3.33 | n/a | 4.33 | 3.33 | 4.00 | 3.67 | yes |
| L12 | 4.33 (4/5/4) | 3.33 | n/a | 3.00 | 4.00 | 4.00 | 3.67 | yes |
| L13 | 4.33 (4/5/4) | 3.00 | 3.33 | 4.00 | 4.33 | 4.33 | 4.00 | yes |

Result: met

## The candidate's evidence

- **L8**: Opus 4, "a grip and a collapse chevron on every panel header, but
  the column dividers do not show until hover"; Sonnet 5; Sol 4, "resize
  boundaries look like ordinary separators until hovered".
- **L9**: 5 from every judge: the dimmed source, the named chip, and the
  dashed landing area with its insertion line.
- **L10**: Opus 4 and Sol 4, "hover and active look the same" and "the two
  states are demonstrated on different dividers"; Sonnet 5.
- **L11**: Opus 4, "the small chevron is the only thing that tells collapsed
  from expanded"; Sonnet 5; Sol 5.
- **L12**: Opus 4 and Sol 4, "a large empty area under the right column";
  Sonnet 5.
- **L13**: Opus 4 and Sol 4, "the drag view repeats the label in both the
  chip and the placeholder, which is slightly busy"; Sonnet 5.

## Not changed after judging

The evidence names three things the round did not change, because the
candidate already holds on every criterion and a change would put the
shipped build out of step with the judged one:

- The divider's hover and active states share one treatment, as in VS
  Code.
- The drag chip and the landing area both carry the panel's name.
- The rearranged stills leave space under the right column, because that
  arrangement puts Launch and Ask alone under a collapsed New issues. That
  is the arrangement, not a layout fault: a column of only fixed-height
  panels gives the last one the spare height.

Changes after judging, none of which changes a still:

- The drop marker's `-2px` margin became `top: -2px`, so P2's spacing scale
  holds. It renders the same.
- Code review (sol-loop) fixes: the layout handlers do nothing below
  1200 px, and the layout rows in the shortcut list show only above it; a
  column that gets a panel back after the others were resized takes their
  mean width, and every filled column keeps a 320 px minimum; new issues
  hold while the reader is in whichever column holds New issues; a jump
  from a ticket row to its card, or back, expands a collapsed panel first;
  double-clicking a divider resets the sizes in place and keeps focus on
  it. Round 2 added: the move hint for screen readers exists only above
  1200 px; a panel header wraps rather than clipping its grip and chevron
  in a 320 px column; the new-issues hold follows the hovered column after a
  keyboard move; collapsing keeps each list's scroll position for when it
  opens again and moves focus to the chevron when the focused item is
  hidden; a drop counts only inside a column, not over the toolbar above
  it; going narrow keeps the focused element; the layout no longer syncs
  live between tabs. Round 3 added: going narrow from a focused Ask field
  switches the narrow composer to Ask; a collapsed New issues panel that
  was scrolled holds new rows until it opens; Launch and Ask take at most
  70% of a column and every panel keeps its header in view. L1 to L7 pass
  again on the fixed build.
