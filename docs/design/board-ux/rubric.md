# Board redesign rubric

This rubric scores the board redesign in PR 2 of issue #28. It turns two
things into criteria a judge can check: the shortcomings from the
[paired review](review.md), and what the [reference UIs](references/README.md)
do well. The redesign ships only when it passes every gate and beats the
current board on every axis.

[Issue #67](https://github.com/OttoOps/t3-codemode/issues/67) extends it.
#28's judging passed a board that is hard to read under real use, so the
[Glanceability](#glanceability) axis adds a live load level, a timed task
test, and gates for clipped text, state encoding, color meaning, titles,
times and Activity text. The [critique](critique.md) gives the reasons and
the current board's values.

## How judging works

**Fixture.** Every capture and measurement uses the seeded demo board from
[`harness/`](harness/README.md), with live updates on (`BOARD_DEMO_EVERY=4`)
unless a test says otherwise. Three load levels:

- *Seeded*: the harness as committed (15 open tickets, 7 decisions, about 45
  Activity lines, 26 settled).
- *Heavy*: the seeded board left running until it has at least 30 tickets
  and 15 decisions (about nine minutes of live updates, since a ticket arrives every 36 s and a decision every 24 s).
- *Live*: a snapshot of the real board, anonymized and replayed read-only
  by [`live/`](live/README.md): 4 decisions, 8 open tickets with 18
  workers, 50 Activity rows and 19 settled tickets. Glanceability is judged
  on this load, at 1920x1080, 1366x768 and 390x844.

Viewports: 1440x900, 1280x800 and 1024x768 for desktop; 768x1024 and
390x844 for narrow. Light and dark.

**Subjects.** The current board at `7e29c9d` (the baseline), each candidate,
and, for the judged appearance criteria, the reference screenshots.

**Blind protocol.**

1. A script captures every subject with the same fixture, viewports and
   themes, and records the measured criteria.
2. The script blanks the page title, the "t3 board" heading, and any file
   or branch name that reveals the subject. Subjects get random
   letters, and each judge sees them in a different order.
3. At least three judges score independently: one Claude model, one GPT
   model, and one more of either. They get a judge copy of this rubric with
   the Baseline column removed, the stills, short
   screen recordings of the stability tests, and the reference images. They
   do not see the review, the code, or which subject is the baseline.
4. Judges score each judged criterion from 1 to 5 against the anchors
   below and give one sentence of evidence per score. Measured criteria are
   computed by the script, and judges do not score them.
5. PR 2 commits the scores, the evidence and the measurement output under
   `docs/design/board-ux/judging/` as `round-<n>.md`, one file per round,
   with the letter-to-subject key at the bottom.

**Decision rule.** A candidate wins a round when:

- it passes every **gate**;
- on each of the six axes (Usability, Appearance and polish,
  Functionality, No slop, Reference fit, Glanceability), its mean judged score across
  judges is higher than the baseline's, and no judged criterion falls below
  the baseline by more than 0.5;
- on each criterion a still can show (P4, P5, P6, N3, N4, N5, R2 and R4),
  its mean is at least the median of the reference subjects' means on that
  same criterion. Judges score the references on those criteria only. R1,
  R3, R5 and R6 depend on behavior (what stays fixed, what moves when
  detail opens), so judges score them from the recordings of the candidate
  and the baseline, and no reference median applies.

If it does not win, the judges' evidence goes into the next iteration and a
new round runs. The Functionality axis has no judged criteria, so for it
"beats" means it passes every Functionality gate.

For #67, a candidate also has to pass the task test (G1) on the live load,
and the issue's owner signs off on the after-screenshots. Agent judging
alone does not close it.

## Judged scale

Each judged criterion lists its own anchors. In general:

| Score | Meaning |
| --- | --- |
| 1 | Fails the criterion's job; a user would notice and be slowed or misled. |
| 2 | Works with visible friction or inconsistency. |
| 3 | Adequate; no problem a user would notice, nothing notably good. |
| 4 | Good; matches the better references. |
| 5 | Exemplary; a judge would cite it as a reference. |

## Usability

| ID | Criterion | Kind | Test | Baseline |
| --- | --- | --- | --- | --- |
| U1 | **The page does not scroll at desktop widths.** | Gate | At 1440x900, 1280x800 and 1024x768, seeded and heavy: `document.scrollingElement.scrollHeight <= innerHeight` and `scrollWidth <= innerWidth`. | Fails: 2,345 px at 1440x900 (`current/report.json`). |
| U2 | **Launch and Ask are always reachable.** | Gate | At 1440x900 and 1280x800, after scrolling each panel to its end, both inputs' bounding boxes are fully inside the viewport. Where a layout shows one composer with a Launch and Ask switch (1024 px and narrower), the selected input is fully inside the viewport after scrolling each panel, and switching to the other takes one activation and keeps both drafts. From page load each input takes at most one shortcut or three Tab presses (skip link included). At 390 px each is one tap away. | Fails: 2.22 screens down; tab stop 218 of 227. |
| U3 | **Nothing the user touches moves.** | Gate | For 60 s of live updates from the harness started with `BOARD_DEMO_EVERY=1 BOARD_DEMO_CHURN=1`, which every 5 s also dismisses the oldest card and, when a ticket has arrived during the run, moves the oldest such ticket to Done, plus a card answered from a second browser context (`POST /api/cards/<id>/answer`) and a reconnect without navigation (the method in `harness/cut-stream.js`: an init script records each `EventSource`; the test routes `/api/events` to abort, closes the open stream and fires its `error` event, unroutes after 5 s, and waits for a fresh `state` event): (a) focus Launch, Ask, a card reply and the Settled filter in turn; the focused element's `top` and `left` change by 0 px and its value and caret are unchanged; (b) focus a ticket thread link, a worker PR link, an ask Settle and a settled Restore; after 30 s `document.activeElement` is the same connected node, never `BODY`; (c) hover a card option and a ticket Settle mid-panel; `elementFromPoint` at that spot returns the same element, or its same-height placeholder, for the whole run. | Fails: 214 px drift; focus fell to `BODY`; a row moved 80 px. |
| U4 | **Nothing inserts above the reader.** | Gate | Scroll Decisions, Tickets and Activity to mid-list. After 60 s the first visible item in each is the same item at the same offset, and the arrivals show as a count in the panel header's reserved slot. Switching Tickets tabs and back restores each tab's scroll offset. | Fails: the Activity feed and ticket list shift. |
| U5 | **Decisions are dense enough to triage.** | Gate | At 1440x900, seeded: at least 5 decision cards fully visible without scrolling the panel, and the median card height is at most 144 px. At 390x844: at least 2.5 cards visible. | Fails: about 4 visible, 156-221 px each. |
| U6 | **Tickets are dense enough to scan.** | Gate | At 1440x900, seeded: at least 12 open ticket rows, with their note lines, visible in the Tickets panel. Two rows with the same fields have the same height at 1024 and 1440. | Fails: rows vary from 43 to 118 px. |
| U7 | **Urgent work comes first, and tickets link to their decisions.** | Gate | Blockers and errors sort above gates and reports. In a grayscale capture a judge can still tell a blocker from a gate. Every needs-you ticket row reaches its card in one activation, and the card reaches its row. | Fails: age order only; no link. |
| U8 | **Replies are safe.** | Gate | With the options reordered in a fixture, none is styled primary by its position (G6 styles the answer options above Reply and Dismiss as a group, never one option over another). Clicking an option sends no request for at least 5 s; Undo cancels it with no request; otherwise exactly one request is sent. Digit shortcuts never send. | Fails: index-based primary; immediate send. |
| U9 | **The keyboard reaches everything.** | Gate | Every item in the functionality inventory works by keyboard alone. Skip links reach each panel and both inputs. Shortcuts do nothing while a field, contenteditable or IME composition has focus, and can be turned off. | Fails: no skip links; Launch at tab stop 218. |
| U10 | **Narrow widths fall back cleanly.** | Gate | At 768x1024 and 390x844: no horizontal page scroll; one panel at a time; tabs and composer pinned; touch targets at least 44 px; no clipped control at 200% zoom. With the on-screen keyboard open (device or emulated `visualViewport`), the focused input and its submit control stay visible, and closing restores the previous panel and scroll. | Fails: one 6,263 px column. |
| U11 | **The connection state is honest.** | Gate | Cut the event stream as `harness/cut-stream.js` does, while `/api/state` still succeeds. The header never shows healthy, content is marked stale, reply controls are disabled, and no panel or composer moves (0 px). | Fails: green "T3 connected" under the red banner; content pushed down 35 px. |
| U12 | **A still answers "what needs me".** | Judged | From one 1440x900 still, the judge writes which tickets need the user and what each wants. 1: cannot tell. 3: can tell after reading most of the page. 5: reads it off the first panel in seconds, urgent items first. | - |
| U13 | **Each panel has an obvious home.** | Judged | From the 1440, 1024 and 390 stills. 1: sections float, positions depend on content. 3: panels are clear but their boundaries or titles are weak. 5: each panel's place and job are obvious, as in `thunderbird-unified-inbox.webp` or `slack-channel-thread.webp`. | - |

## Appearance and polish

| ID | Criterion | Kind | Test | Baseline |
| --- | --- | --- | --- | --- |
| P1 | **Status and controls are legible.** | Gate | In both themes, every status word and chip is at least 4.5:1 against its background, and input borders and the focus ring are at least 3:1, computed from the rendered colors. | Fails: waiting 4.22, idle 4.29, done 4.49; input border 1.38. |
| P2 | **The type and spacing scales are small.** | Gate | Computed styles of visible text use at most 4 font sizes and 3 weights, monospace excluded. Every margin, padding and gap is 0 or from {4, 8, 12, 16, 24} px (control heights and borders excluded). | Fails: 7 sizes and weights in 600 px. |
| P3 | **Color means one thing.** | Gate | The accent color is not used for any status. Amber appears only on items that wait on the user. Running is neutral. | Fails: blue means running and primary; amber means waiting and gate. |
| P4 | **Hierarchy.** | Judged | 1: headings are weaker than content; everything has the same weight. 3: decisions stand out, but secondary items still compete. 5: the eye goes to decisions, then tickets that need the user, then the rest, as in `linear-triage.webp`. | - |
| P5 | **Alignment and consistency.** | Judged | 1: columns drift, rows wrap differently for the same fields. 3: mostly aligned, a few variances. 5: status, PR and thread sit in the same columns on every row at every desktop width, as in `github-projects-table.webp`. | - |
| P6 | **Dark mode.** | Judged | 1: inverted or muddy; boundaries vanish. 3: correct and readable. 5: designed for dark, with roles and boundaries as clear as in light, as in `grafana-alert-rules.webp`. | - |
| P7 | **Loading, empty and disconnected states.** | Judged | From the three state stills. 1: blank blocks, contradictory status, positional copy. 3: each state is clear on its own. 5: each state says what is happening and what to do, keeps the layout, and matches the rest of the design. | - |

## Functionality

| ID | Criterion | Kind | Test | Baseline |
| --- | --- | --- | --- | --- |
| F1 | **Nothing the current board does is lost.** | Gate | A scripted pass over every item of the functionality inventory in [`review.md`](review.md#functionality-inventory) against the seeded harness, including failure paths: a launch with mixed results, an Ask that fails, a card reply that errors, a disconnect, a hidden-tab notification, Restore from Settled, and thread links falling back to a browser tab. | Passes by definition. |
| F2 | **Tests cover the new structure.** | Gate | `cargo test --release --locked` passes, `tests/board.rs` covers the panels, tabs, reserved slots and stability rules the candidate adds, and the existing board tests still pass. | - |
| F3 | **The board announces each result once.** | Gate | Launch results, Ask answers, card errors and "N new decisions" each reach a live region exactly once. Errors are alerts. Activity lines are never announced. | Fails: Launch results and Ask errors are not announced. |
| F4 | **Repeated controls have context.** | Gate | The accessible name of every option, Send, Dismiss, Settle and Restore control includes its card id or ticket label (check with the accessibility tree or axe). | Fails: twelve identical "Send" and "Dismiss". |

## No slop

| ID | Criterion | Kind | Test | Baseline |
| --- | --- | --- | --- | --- |
| N1 | **No decoration.** | Gate | A CSS and DOM audit finds no gradients, no emoji, no drop shadows on panels or cards, and no icon that is not paired with a function or a status. | Passes, except the shadows on every card. |
| N2 | **No raw or broken text.** | Gate | No `**` or other Markdown syntax reaches the page. Every truncated string ends in an ellipsis and keeps its full text in a tooltip or disclosure. The time zone appears once, in the header. | Fails: `**ACTION NEEDED:**`, "https://githu", "MST" on every row. |
| N3 | **No filler copy.** | Judged | 1: labels explain the implementation or pad the page. 3: plain, a few strings say little. 5: every string is data, a label, or an instruction, and empty states point to a control rather than a position. | - |
| N4 | **Every element has a job.** | Judged | The judge lists any element whose removal would lose nothing. 1: several such elements, or badges and chips that carry no information. 3: one or two. 5: none. | - |
| N5 | **It does not look like a generic AI dashboard.** | Judged | 1: card grid, soft shadows, pill badges everywhere, no point of view. 3: neutral and competent. 5: looks like a tool built for this job, comparable to the Linear and Thunderbird references. | - |

## Reference fit

Each criterion names the references that show it. Judges score the
candidate and the baseline against the references, and score the
references themselves where a still can show the criterion.

| ID | Criterion | References | Anchors |
| --- | --- | --- | --- |
| R1 | **Fixed chrome, scrolling middle.** Header, panel titles and inputs are fixed; only lists scroll. | `slack-channel-thread.webp`, `thunderbird-unified-inbox.webp`, `grafana-alert-rules.webp` | 1: the page scrolls as one. 3: panels scroll, but some chrome scrolls away. 5: as in the references. |
| R2 | **What needs a person is its own queue, with its count in the header.** | `linear-inbox.webp`, `claude-code-agent-view.webp`, `linear-triage.webp` | 1: mixed into everything else. 3: a separate section, count buried. 5: a first-class queue whose count shows in the header and the tab title. |
| R3 | **The input sits with the pane it acts on and is never at the end of content.** | `slack-channel-thread.webp`, `claude-code-agent-view.webp`, `linear-issue-agent-panel.webp` | 1: at the bottom of the page. 3: fixed but detached from its context. 5: fixed, next to what it acts on, and its results appear beside it. |
| R4 | **Status by position first, then a small mark and a word, with a restrained palette.** | `linear-triage.webp`, `buildkite-pipelines.webp`, against the noisier `jira-board.webp` | 1: a chip on everything, many hues. 3: restrained but not structured. 5: queue and position carry most status; a few quiet marks carry the rest. |
| R5 | **Rows are fixed-height, with age right-aligned and actions in the same place.** | `linear-inbox.webp`, `thunderbird-unified-inbox.webp`, `datadog-oncall-pages.webp` | 1: rows reflow on every change. 3: mostly fixed. 5: a status change swaps a mark, never resizes a row. |
| R6 | **Detail opens beside or within the item, without a page jump.** | `thunderbird-unified-inbox.webp`, `datadog-oncall-pages.webp`, `slack-channel-thread.webp` | 1: detail pushes the list away. 3: inline disclosure that grows the list. 5: detail in place or beside, with the list position kept. |

## Glanceability

Added for issue #67. Every test runs on the *Live* load at 1920x1080,
1366x768 and 390x844, dark and light, unless it says otherwise.
`live/capture.js` records the measured values; the baseline column comes
from [`live/before/report.json`](live/before/report.json) and the
[critique](critique.md).

| ID | Criterion | Kind | Test | Baseline |
| --- | --- | --- | --- | --- |
| G1 | **What waits on me, and the reply, at a glance.** | Gate | The task test. Three cold readers (Claude Haiku, Sonnet and Opus), each allowed one look at the 1920 dark still and nothing else, name every ticket that waits on the user and the reply each one wants. Pass: each reader names exactly the tickets with an open decision card, with the right reply, and no "maybe" or extra ticket. Record each reader's wall time. The issue's owner also times one cold look at the candidate and at the current board. | Fails: all three found the four asks, but Haiku added a fifth (#67), and Sonnet and Opus could not tell whether #67 waits. 9.2 s, 17.0 s and 14.3 s. |
| G2 | **No clipped text without an indicator.** | Gate | Every visible element whose text overflows its box ends in an ellipsis or an overflow count (`+3 more`), and its full text is reachable in a tooltip or a disclosure. No text is cut mid-word by a parent's edge. Screen-reader-only text is exempt. | Fails: the worker line (`.wl`) is cut by the row edge with no ellipsis or tooltip at 1366 and 390 (`#647 PR 2/2 gate: transcript-sour`). |
| G3 | **Every state is distinct without color.** | Gate | `needs you`, `running`, `waiting`, `idle`, `failed` and `done` each have their own shape and word. In grayscale captures of the *Seeded* load, which has failed and waiting workers that the *Live* load lacks, and of the *Live* load, a judge tells all six apart. A ticket's state is the most urgent of its own and its workers' (needs you, then failed, running, waiting, idle, done). | Fails: all 8 open tickets read `idle` while 4 of their 18 workers run; `running` and `idle` share one hollow ring at the same weight. |
| G4 | **Each color has one meaning.** | Gate | A script lists every rendered text and mark color with the elements that use it. Amber appears only on items with an open decision card and on the Decisions count. Red appears only on failed and blocker items. The accent appears only on links, focus and primary controls. No status hue is used for a heading, a stripe on every card, or a gate that has no open card. Extends P3. | Fails: amber marks `needs you`, every card's stripe and kind dot, and `gate:` text on workers whose gate was already answered. |
| G5 | **Panels are told apart before they are read.** | Judged | From the 1920 and 1366 stills, blurred to 4 px. 1: every panel has the same surface and weight; the eye has nowhere to land. 3: panels are separable but Decisions does not lead. 5: Decisions reads first, then the tickets that need the user; each panel's job is clear from its shape alone, as in `linear-triage.webp`. | - |
| G6 | **The answer is the most prominent control on a card.** | Gate | On each decision card that offers answer options, while they are available (not during the Undo window), the options carry the most visual weight of any control (filled, or the highest contrast and area), and Reply, Dismiss and the full-message link are quieter. With several options, all share one style. | Fails: `merge` is an outlined 28 px button with the same weight as the text buttons beside it. |
| G7 | **Every ticket has a readable title.** | Gate | No ticket, worker or card label shows a raw slug: no `gate:` prefix as a title, no run of `_` between words, no 40- or 64-character hex id. A ticket with no issue title shows its humanized gate or label. | Fails on the board the issue describes (`gate: web#646_owns_the_next_deploy/...`, anonymized); the live snapshot shows raw `gate: windows-signing` and `gate: transcript-source` slugs as row text. |
| G8 | **One time convention.** | Gate | Every visible time uses one form: relative age (`36m`, `1h 32m`), with the absolute local time in a tooltip. The time zone shows at most once, in the header. | Fails: absolute `15:54` sits beside `1 h 32 min` in the same row; Activity shows clock times only. |
| G9 | **Activity shows the substance.** | Gate | No Activity row's visible text starts with `event:` or `note:` or shows a hash longer than 7 characters. Consecutive rows for the same ticket or PR collapse into one group with a count, which expands in place. At 1366 the first 40 visible characters of every row's message line are message text. | Fails: 19 of the 50 rows start with `event:`, 23 show a hash of 8 or more characters, five consecutive rows are `#56 PR 1/4`, and the label column clips to `t3-codem...`. |
| G10 | **Space goes to content.** | Gate | An empty Answers area is at most one line (40 px) under Ask; Activity takes the rest of the column. A ticket list shorter than its panel leaves no reserved empty block above Activity or Launch. | Fails: at 1920 the empty Answers area takes about 290 px, so Activity gets 461 of the column's 1,040 px; Decisions and Tickets each leave about 480 px empty below their last item. |
| G11 | **All text meets AA.** | Gate | Every visible text, secondary and placeholder text included, is at least 4.5:1 against its rendered background in both themes, and 3:1 for 18 px+ bold text and for borders and focus rings. Extends P1 to all text. | Text passes (lowest 5.38:1). Fails for lines: `--line` dividers and borders are 1.27 to 1.38:1 in light and 1.89 to 2.13:1 in dark, and stale text at 0.55 opacity would be 2.50:1. |


## Dynamic layout

Added for [issue #93](https://github.com/OttoOps/t3-codemode/issues/93):
moving, collapsing and resizing panels at desktop widths (1200 px and up).
The gates are measured by [`harness/layout-gates.js`](harness/README.md) on
the *Live* replay at 1440x900 unless a test says otherwise. The judged
criteria are scored blind against the panel references in
[`references/panels/`](references/panels/README.md): VS Code for the Web,
Dockview, Golden Layout, Grafana and JetBrains IDEs.

**Subjects and packets.** Each judged subject is a set of stills: the
candidate's from [`harness/layout-stills.js`](harness/README.md), and each
reference product's images. [`harness/layout-packets.py`](harness/README.md)
builds one packet per judge: every subject under a random letter, in a
random order, with images renamed and captioned neutrally (state only, no
product names), and the candidate's page title and heading blanked. Judges
are not told which subject is under test. Headless captures show no mouse
pointer, in the candidate's stills and the live references alike.

**Decision rule.** The candidate passes every L gate, and on every judged
criterion its mean across judges is at least the median of the reference
subjects' means. A subject whose images cannot show a criterion is scored
`n/a` and left out of that criterion's median. Where fewer than two
reference subjects are scored on a criterion, the bar is 4 ("good; matches
the better references"). The issue's owner also
reviews the stills and the scores before merge.

| ID | Criterion | Kind | Test |
| --- | --- | --- | --- |
| L1 | **Every panel moves anywhere.** | Gate | Each of the five panels reaches the top and the bottom of each of the three columns by pointer drag of its header, and each column by keyboard alone (its grip and the arrow keys). After a keyboard move, focus stays on the moved panel's grip. |
| L2 | **Every panel collapses and expands.** | Gate | The chevron collapses and expands by click and by Enter, with `aria-expanded` kept in step; a double-click on the header's empty area collapses; a shortcut that jumps to a collapsed panel (`g a`) expands it. |
| L3 | **Columns and stacked panels resize, within limits.** | Gate | A 100 px pointer drag of a column divider widens the column by 100 px, and a 60 px drag of a panel divider grows the panel by 60 px; the arrow keys move a focused divider by 24 px. At 1200x700, 1280x800, 1440x900 and 1920x1080, with both dividers dragged to either extreme, and with all five panels in one column: no column is narrower than 320 px, the page does not scroll, and no panel heading or layout control is cut by its column's edge. |
| L4 | **The layout persists and resets.** | Gate | A moved, collapsed layout survives a reload. Reset layout shows only when the layout differs from the default, restores the default and clears the saved value. A saved value that is not JSON, lists a panel twice, has the wrong number of columns, or holds a negative width loads the default. |
| L5 | **Narrow widths are untouched.** | Gate | At 1024x768, 768x1024 and 390x844, in both themes, every rendered element's box matches `main` exactly, with or without a saved custom layout. Going narrow and back to 1440 restores the saved layout. |
| L6 | **Rearranging keeps the reader's place.** | Gate | With Activity scrolled to 300 px and an item in it focused: dragging another panel into Activity's column, and dragging Activity itself to another column, keep its scroll offset and its focus. After a column resize and a collapse, the first visible Activity item is unchanged. |
| L7 | **The controls are accessible.** | Gate | Every grip, chevron and divider has an accessible name; chevrons carry `aria-expanded`; dividers are `role="separator"` with `aria-orientation`. A keyboard move, a collapse and a cancelled drag (Esc) are each announced once. Mid-drag shows a drag preview, a drop marker and a landing area, and Esc cancels the drag with nothing moved. Grip and chevron icons and divider lines are at least 3:1 against their background in both themes. |
| L8 | **Discoverable at rest.** | Judged | From the resting stills. 1: nothing says panels can move, collapse or resize. 3: some affordances show, or only on hover, and a user would have to guess at the rest. 5: move, collapse and resize controls are visible and legible at rest without adding clutter. |
| L9 | **Drag feedback.** | Judged | From the mid-drag stills. 1: nothing shows what moves or where it will land. 3: either the moving item or the landing spot is shown, or the landing spot is ambiguous. 5: what is moving, the exact landing area, and the source's state are all unmistakable. |
| L10 | **Resize affordance and feedback.** | Judged | From the divider stills. 1: no visible divider or handle. 3: a divider is visible, but its hover or active state is weak, or the layout does not visibly follow. 5: a clear divider with a distinct hover and active state, and the layout follows the drag live. |
| L11 | **Collapsed state.** | Judged | From the collapsed stills. 1: collapsed panels vanish, or collapsed and expanded look alike. 3: the collapsed state is clear, but the way back or the reclaimed space is not. 5: a collapsed panel keeps its name (and its count where it has one), the way to expand it is obvious, and its space goes to the rest. |
| L12 | **A rearranged layout still reads.** | Judged | From stills of a non-default arrangement. 1: rearranging leaves the screen cramped, misaligned or broken. 3: usable, with visible seams. 5: the rearranged layout looks designed, in both themes where shown. |
| L13 | **Layout chrome is polished and restrained.** | Judged | From all stills. 1: handles, chevrons, dividers and drop targets are noisy or inconsistent. 3: consistent but plain or slightly heavy. 5: cohesive and restrained; the chrome stays out of the content's way until it is needed. |
