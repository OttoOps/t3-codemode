# Judging round 10

Candidate: round 9's build with one more change from its evidence: an ask that opens with its own ticket's label drops it, since the label sits on the line below, so the blocker card reads "Blocked: Worker failed twice..." above "relay#11" (N4). Baseline: the board at `9f8ad45`, `main` before this branch, with #66's filters and search. Captures: `harness/judge-stills.js` against the seeded demo board with issue-style thread titles, one instance per subject started together; `live/capture.js` and `live/glance.js` against the live replay. The raw measurement output is in [`round-10-measurements.json`](round-10-measurements.json), and the live replay's stills and reports are in [`../live/after/`](../live/after/).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order, as in rounds 8 and 9. G5 was then judged again on its own: rounds 8 to 10 had used `live/before/` for the baseline's blurred stills, and those show `main` at `49f9731`, before #66 added the filter bar. The re-judge used `9f8ad45` replayed against the same snapshot, fresh letters, and only the four blurred stills. Its scores replace the round's G5 scores below.

## Gates

The judges saw the build before code review. The shipped board, `59b51ca`, adds the fixes from sol-loop and CodeRabbit:

- an ask keeps a "then reply" clause and a command after `here:`;
- a Review link drops a trailing period;
- a hash made only of digits is shortened only after "head", "commit", "from", "at", "sha" or an opening parenthesis, so a job number stays whole;
- Activity folds a run only when the sender and the ticket both match;
- the folded worker line shows the most urgent live worker, and focus moves to the visible toggle when a worker toggle hides itself;
- a blocked ticket with an open card also matches the "needs you" filter;
- the unused `last.iso` field is gone;
- a link written in backticks drops the closing backtick;
- a ticket whose last gate card was dismissed reads "Dismissed:", from a new `last.dismissed` field, where it used to read "Answered:";
- a ticket row leads with its most urgent card, the same one that leads the card panel;
- the ticket list keeps its group headings in place while it holds arrivals;
- an expanded Activity group stays open when its oldest message leaves the feed;
- an ask drops a `here:` clause only when a URL follows it, so a plain command after `here:` stays in the ask;
- an age's tooltip names the browser's time zone, the one its clock time is read in, where it used to take the zone from the server's clock strings;
- an ask keeps its inline code whole, so a hex argument in a command is never shortened;
- the header's "checked ... ago" age refreshes with the other ages every 30 seconds.

It also sits on #55, which adds a Context button to the filter bar. At 1920 and 1366 the button sits before the search box, and at 390 it takes a row of its own, so the panels there start 48 px lower and Activity shows seven rows in place of eight. The `live/after/` stills show it.

The heavy gates and G3 ran on `8839cf1`, before the digit-only hash rule and the CodeRabbit fixes, none of which change layout. G1 ran on `7298bf1`, before #55, on a 1920 dark still that differs from the shipped one only in the Context button. The seeded, holds and U3 gates and the live replay rows ran on `59b51ca`, with U3's worker-link probe pointed at the worker's PR link and `glance.js` fading each element's background through the same opacity groups as its text before G11 compares them.

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | pass | scrollHeight = innerHeight at 1440x900, 1280x800 and 1024x768, seeded, and heavy with 30 open tickets and 27 decisions at each size |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input in view, one activation switches and keeps both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | pass | focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept through a second-context answer and a stream cut; thread link, worker PR link, ask Settle, settled Restore kept the same connected node for 30 s; hovered card option and ticket Settle kept their position for 60 s (at least 3003 samples each) |
| U4 | pass | first visible item unchanged after 60 s in all three panels; slots read cards: 2 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 86 |
| U5 | pass | 5 cards fully visible at 1440x900, median height 128 px |
| U6 | pass | 12 open rows visible at 1440 (13 on the heavy board); same-field rows 57 px at 1024 and 1440 |
| U7 | pass | blocker first, then gates, then the report; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to Needs you, Launch, Ask, Tickets and Activity; shortcuts inert in fields and when off |
| U10 | pass | no horizontal scroll at 768 and 390; no target under 44 px; nothing clipped at 200% zoom; emulated keyboard keeps field and submit in view; closing restores panel and scroll |
| U11 | pass | header "Disconnected. Reconnecting; showing data from 09:15."; content stale; reply controls disabled; nothing moved |
| P1 | pass | lowest status text light 5.5, dark 6.74; input border light 3.92, dark 4.57 |
| P2 | pass | sizes 16, 14, 13 and 12 px; weights 600, 500 and 400; no off-scale spacing |
| P3 | pass | accent on no status; amber only on needs-you items; running is ink |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; no time zone outside the header tooltip; every truncated string has its full text in a tooltip |
| F1 | pass | launch results, Ask, card errors, Settle and Restore as in round 7; a new decision raised a browser notification while the tab was hidden |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds ISO times, worker-derived ticket state and grouping, row titles and answered gates, card structure, Activity grouping, and folded workers |
| F3 | pass | each error, answer and launch result announced once; no Activity line announced |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

Glanceability, on the live replay at 1920x1080, 1366x768 and 390x844 in both themes unless noted.

| Gate | Result | Evidence |
| --- | --- | --- |
| G1 | pass | from the 1920 dark still, Claude Haiku 5.5, Sonnet 5.5 and Opus 5.5 each named exactly t3-codemode#2 ("windows ready") and #17, #59 and #56 ("merge"), with no unsure ticket, in 9.7 s, 4.3 s and 5.7 s (`main`: 17.0 s, 9.2 s and 14.3 s, with an extra or unsure #67) |
| G2 | pass | 0 clipped elements without an ellipsis or a count at every view; each clipped element keeps its full text in a tooltip |
| G3 | pass | in grayscale stills of the seeded board (Open and Done tabs) and the live board, Claude Sonnet 5.5 told all six states apart by shape: diamond, half-filled ring, dashed ring, hollow ring, cross and check. Weakest pair: waiting and idle |
| G4 | pass | amber only inside needs-you marks, links and counts; red only on failed and blocker items; the accent only on links, focus and primary controls |
| G5 | judged | see below |
| G6 | pass | answer options are the filled controls on a card, all in one style; Reply, Dismiss and the disclosure are quiet |
| G7 | pass | no raw slug, `gate:` prefix or long hex id in any label or title |
| G8 | pass | every time is an age with the local time in a tooltip; the only clock time on screen is inside one worker's message text ("pushed 17:07 MST") |
| G9 | pass | no Activity row starts with a kind or shows a hash over 7 characters; no two neighbouring rows share a sender; runs fold into one row with a count |
| G10 | pass | the empty Answers area is 0 px; Activity takes the rest of the column |
| G11 | pass | lowest text 4.95 (dark) and 5.2 (light), placeholders included; lowest line 3.08 (dark) and 3.25 (light) |

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 3.67 (4/3/4) |  | yes |
| U13 | 5.00 (5/5/5) | 4.00 (4/4/4) |  | yes |
| P4 | 4.00 (4/4/4) | 3.00 (3/3/3) | 3.83 | yes |
| P5 | 4.00 (4/4/4) | 4.00 (4/4/4) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 4.00 (4/4/4) | 3.00 | yes |
| P7 | 3.67 (4/3/4) | 4.00 (4/4/4) |  | yes |
| N3 | 4.00 (4/4/4) | 3.00 (3/3/3) | 4.17 | no |
| N4 | 3.67 (4/4/3) | 3.33 (3/4/3) | 4.00 | no |
| N5 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.33 | yes |
| R1 | 4.00 (4/4/4) | 4.00 (4/4/4) |  | yes |
| R2 | 5.00 (5/5/5) | 3.00 (3/3/3) | 2.00 | yes |
| R3 | 4.00 (4/4/4) | 4.00 (4/4/4) |  | yes |
| R4 | 4.67 (5/5/4) | 3.00 (3/3/3) | 3.50 | yes |
| R5 | 4.00 (4/4/4) | 3.33 (4/3/3) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |
| G5 | 4.67 (4/5/5) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.83 | 3.83 | yes |
| Appearance and polish | 4.00 | 3.75 | yes |
| No slop | 4.00 | 3.22 | yes |
| Reference fit | 4.11 | 3.39 | yes |
| Glanceability | 4.67 | 3.00 | yes |

**Decision rule: not met** (N3, N4 short).

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - In 1440-light the first panel 'Needs you 7' leads with a red-bordered 'Blocked: Worker failed twice...' card for relay#11, then bold ask titles (Reply "merge" to merge #81, Reply "accept"..., Approve the AWS sign-in tab, Merge #203), each with its ticket and age, so the asks read off in seconds.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png, Needs you puts relay#11's migration blocker first and foregrounds widgets#41's merge/deploy choice, relay#9's budget choice, atlas#60's AWS approval and ledger#198's merge, while the center reveals atlas#57's PR split choice and gizmo#15's stalled-worker report below the visible cards.
- Candidate, Claude Sonnet 5.5: 5 - 1440-light: 'Needs you 7' panel with an orange badge and a '7 need you' header count; each card leads with a bold ask and its action buttons (merge, accept, fix), the red Blocked card first.
- Baseline, Claude Opus 5.5: 4 - In 1440-light the 'Decisions 7' panel lists a6 relay#11 'blocker' first, then 'gate' cards with lowercase reply instructions ('reply "merge" to merge #81...here: acme/widgets#81'), so the asks are readable but in body-weight text padded with codes and repo paths.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, Decisions puts relay#11's migration blocker first but cuts off the decision itself, then shows widgets#41's merge/deploy choice, relay#9's budget choice, atlas#60's AWS approval, ledger#198's merge and atlas#57's PR split choice, with only gizmo#15's report heading visible.
- Baseline, Claude Sonnet 5.5: 4 - 1440-light: Decisions panel leads with relay#11 blocker in red then gate cards with quoted asks, but messages are truncated and the header shows only '10 running', so the count must be read from the panel title.

### U13

- Candidate, Claude Opus 5.5: 5 - At 1440 there are three titled columns (warm-tinted 'Needs you', 'Tickets' with Open/Done/Settled tabs, a Launch/Ask/Answers/Activity column); at 1024 the right column folds into a fixed Launch|Ask bar plus an Activity tab, and at 390 a bottom tab bar (Needs you 7 / Tickets 15 / Activity) gives each panel a clear home.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png the tinted queue, ruled Tickets list and right-hand controls and Activity rail have clear boundaries, with Activity tabbed under Tickets at 1024 and all three destinations named in the 390-light.png bottom navigation.
- Candidate, Claude Sonnet 5.5: 5 - 1440, 1024 and 390 stills: a tinted Needs you column, a grouped Tickets list with sub-headers and a Launch/Ask/Activity column each with a bold title; mobile bottom tabs 'Needs you 7 / Tickets 15 / Activity' mirror them.
- Baseline, Claude Opus 5.5: 4 - At 1440 and 1024 the three panels (Decisions, Tickets, Launch/Ask/Answers/Activity) have fixed homes and 390 has a bottom tab bar, but panel titles are small and weak ('Decisions 7' at body size), and at 390 the decision cards spread a6, relay#11, Full message and age across loose lines.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png and 1024-light.png, a vertical rule separates Decisions from Tickets, with the right rail replaced by an Activity tab and a bottom Launch/Ask bar at 1024, while 390-light.png gives each list its own bottom tab.
- Baseline, Claude Sonnet 5.5: 4 - 1440 shows three titled columns (Decisions, Tickets, Launch/Ask/Activity) with clear dividers; titles are small and weak, and at 1024 Launch collapses into a bottom bar with Activity folded into a tab.

### P4

- Candidate, Claude Opus 5.5: 4 - In 1440-light the large 'Needs you' heading with an orange count, the red blocked card and the diamond-marked 'Needs you 7' group atop Tickets lead the eye, but the many solid blue merge/accept/fix buttons repeated down the left column compete with each other.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png the tinted Needs you pane leads with a red blocker and bold requests, and the center lists all seven attention tickets above Running, with the blue Launch and Ask controls still drawing some attention.
- Candidate, Claude Sonnet 5.5: 4 - 1440-light: tinted Needs you panel, blue action buttons and an orange count badge pull the eye first, then the 'Needs you' group in Tickets, then 'Running'; blue buttons are somewhat loud.
- Baseline, Claude Opus 5.5: 3 - In 1440-light the colored left bars and 'needs you - aN' text mark priority, but decision cards, ticket rows and the status-word column all carry similar weight, so the Decisions panel does not clearly lead.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png the red-edged blocker leads the left queue, but gizmo#12 interrupts the center's needs-you rows and the blue Launch and Ask buttons draw attention on the right.
- Baseline, Claude Sonnet 5.5: 3 - 1440-light: decision cards have coloured left rails, but the Tickets list bolds every id and repeats orange 'needs you - aN' on most rows, so secondary rows compete with the Decisions queue.

### P5

- Candidate, Claude Opus 5.5: 4 - In 1440 and 1280 ticket rows keep id+title left, age right, the PR link (#81 open) and PR progress (PR 1/2, +1 worker) in a right-hand column on every row, though decision cards vary in height and button rows.
- Candidate, GPT 6.2 Sol: 4 - Across 1440-light.png, 1280-light.png and 1024-light.png, ticket glyphs, titles, main PR links and ages hold predictable positions, but the lower PR/worker indicator shifts left when '+1 worker' appears.
- Candidate, Claude Sonnet 5.5: 4 - 1440/1280: every row has id plus title, a lead word, age right-aligned and PR mark in the same place; some rows add '+1 worker' or truncated text, and at 1024 the PR marks shift.
- Baseline, Claude Opus 5.5: 4 - In 1440 and 1024 a status-word column sits left, the PR link column and clock time sit right, and rows are fixed height, though the third-line PR status runs vary in length and some rows leave an empty third line (atlas#57).
- Baseline, GPT 6.2 Sol: 4 - Across 1440-light.png, 1280-light.png and 1024-light.png, ticket status, ticket ID, primary PR and timestamp hold their columns, while extra worker PRs share a line under the ID and start after variable-length labels.
- Baseline, Claude Sonnet 5.5: 4 - 1440/1280-light: status word, ticket id, PR link and age sit in consistent columns on every row; sub-lines vary (one PR vs two PRs plus '#82 open') and card heights in Decisions differ.

### P6

- Candidate, Claude Opus 5.5: 4 - 1440-dark keeps card borders, the warm-tinted Needs you panel, the red blocked border and the orange/brown status words legible, with roles as clear as in light; nothing feels inverted, though it is a straight port rather than distinctly designed.
- Candidate, GPT 6.2 Sol: 5 - In 1440-dark.png and 390-dark.png, the warm queue ground, charcoal cards, neutral lists and dark input surfaces keep the same visual roles as in light, with red and amber signals legible on both widths.
- Candidate, Claude Sonnet 5.5: 4 - 1440-dark: tinted left panel, outlined red Blocked card and orange badge keep roles clear and legible; the dark panel boundaries are subtler than in light.
- Baseline, Claude Opus 5.5: 4 - 1440-dark keeps card borders, left accent bars and status colors legible with a darker Tickets header band, readable and correct, though the large dark Answers block in the blurred 1920 still is a heavy empty mass.
- Baseline, GPT 6.2 Sol: 4 - In 1440-dark.png and 390-dark.png, the red blocker, amber gates and blue controls are legible, although decision card surfaces and pane edges are subtler than in the light stills.
- Baseline, Claude Sonnet 5.5: 4 - 1440-dark: readable with clear card borders and amber/red rails and a distinct right-hand surface; looks like a light theme inverted with no extra role differentiation.

### P7

- Candidate, Claude Opus 5.5: 4 - state-loading shows 'Connecting' and per-panel 'Loading decisions...' text, state-empty says 'No open tickets. Paste an issue into Launch to start one.', and state-disconnected shows a red 'Disconnected. Reconnecting; showing data from 08:29.' with action buttons greyed; the Answers block is absent in loading so Activity sits higher than in the loaded layout.
- Candidate, GPT 6.2 Sol: 3 - state-loading.png labels the loading lists, state-empty.png directs the user to Launch and state-disconnected.png timestamps the cached data, but the Answers panel disappears and Activity jumps up in the loading and empty stills.
- Candidate, Claude Sonnet 5.5: 4 - state-loading keeps the layout with 'Connecting' and per-panel Loading; state-empty gives '0 need you' and 'Paste an issue into Launch'; state-disconnected has a red banner, dimmed buttons and a last-data time.
- Baseline, Claude Opus 5.5: 4 - state-loading shows 'Connecting' and per-panel loading text with the Answers block held in place, state-empty says 'No open tickets. Paste an issue into Launch to start one.' and 'No answers yet. Type a question in Ask.', and state-disconnected shows a red 'Disconnected. Reconnecting; showing data from 08:29.' and dims all content and actions.
- Baseline, GPT 6.2 Sol: 4 - state-loading.png labels the loading lists, state-empty.png points to Launch and Ask, and state-disconnected.png retains the populated layout and timestamps the stale data, though Answers says there are no answers during loading.
- Baseline, Claude Sonnet 5.5: 4 - state-loading shows 'Connecting' with per-panel 'Loading...'; state-empty says 'Paste an issue into Launch to start one'; state-disconnected has a red banner with data dimmed and a timestamp; layout is kept in all three.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are mostly data or instructions (card titles are the asks, 'Ctrl+Enter launches.', the empty state points to Launch), with a little repetition where each Needs-you ticket row repeats the ask text from the card.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png the queue leads with concrete requests and state-empty.png names Launch as the next step, although the widgets#41 card spells out the exact reply options that its buttons already display.
- Candidate, Claude Sonnet 5.5: 4 - Cards lead with the ask in plain words and rows use verbs (Choose, Reply, Report, Blocked); 'not joined yet' and 'Launched, waiting for the thread to join' are clear; no card ids.
- Baseline, Claude Opus 5.5: 3 - Rows read 'needs you - a6', 'gate: sso 52 min', 'gate: scope', '06:39 note: ...', and cards end with 'here: acme/widgets#81', so codes and implementation terms pad the copy.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, strings such as 'a6' and 'needs you - a6 blocker' make the reader decode card identifiers, while state-empty.png directly names Launch and Ask.
- Baseline, Claude Sonnet 5.5: 3 - Card ids like 'a6'/'a1' and 'needs you - a6' leak implementation, 'Full message' repeats on every card, and 'thread' links say little; most other strings are data.

### N4

- Candidate, Claude Opus 5.5: 4 - Nearly every element carries information, but 'PR 1/1' appears on most rows with little signal and the Tickets 'Needs you' group duplicates the left panel's content.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png the widgets#41 card could drop the repeated literal reply choices from its heading and retain them on the buttons, while Review #81 and the disclosure control open distinct information.
- Candidate, Claude Sonnet 5.5: 3 - Every card repeats a Reply/Dismiss/chevron row and the same item appears as a card and as a 'Needs you' row, and the diamond marker adds little beside the group header.
- Baseline, Claude Opus 5.5: 3 - The aN card codes, the left status-word column repeated by a third-line 'running PR 1/1' mark, and the colored left bar all restate the same status, so several elements could go with no loss.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png, the 'reply "merge" to merge #203' instruction duplicates the effect of the merge button on the same card, while Full message, Reply and Dismiss each provide a distinct action.
- Baseline, Claude Sonnet 5.5: 3 - The aN card ids, repeated 'running' word on most rows and the 'Full message' link on each card carry little; 'needs you' repeats in both panels.

### N5

- Candidate, Claude Opus 5.5: 4 - Grouped list with small status glyphs, a tinted triage column and a dense activity feed reads as a purpose-built tool, though the stacked bordered cards with solid blue buttons in the left column lean toward generic.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png the decision-first queue, named worker tasks, direct PR review links and adjacent board answers make the workspace specific to supervising coding agents.
- Candidate, Claude Sonnet 5.5: 4 - Grouped inbox-style list with sub-headers, count badge and sparse marks reads as a purpose-built tool close to Linear triage, though filled blue buttons are generic.
- Baseline, Claude Opus 5.5: 3 - Dense and utilitarian with a log-style timestamped Activity feed and outlined buttons, but the mix of status columns, colored bars and code labels feels neutral rather than designed for this job.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png, the decision replies, worker-level PR lines and timestamp-first activity feed give the three-pane layout a recognizable coding-operations purpose.
- Baseline, Claude Sonnet 5.5: 3 - Neutral, dense list-and-column tool with no card grid or shadows, but no strong point of view; looks competent rather than distinctive.

### R1

- Candidate, Claude Opus 5.5: 4 - Across the 1440 stills the header, filter row, panel titles and Launch/Ask inputs stay put while each list is clipped independently at the viewport, and the Answers block in 1280-light clips internally above a fixed Activity header; full scroll behavior cannot be confirmed from stills.
- Candidate, GPT 6.2 Sol: 4 - In live-hover-t0.png and live-hover-t30.png the connection bar, pane headings, Launch and Ask stay in place while the row contents and queue count update, though the stills do not show a scroll gesture.
- Candidate, Claude Sonnet 5.5: 4 - 1440 stills: header, filter row, panel titles and tab bar stay fixed while the Needs you cards, Tickets list and Activity scroll within their columns.
- Baseline, Claude Opus 5.5: 4 - Header, filters, panel titles and Launch/Ask stay fixed across the 1440 stills while lists clip independently, and the Answers block in 1280-light clips above a fixed Activity heading; full scroll behavior cannot be confirmed from stills.
- Baseline, GPT 6.2 Sol: 4 - In live-hover-t0.png and live-hover-t30.png, the header, pane titles and inputs occupy the same positions as ticket and activity rows update, though the stills do not show a scroll gesture.
- Baseline, Claude Sonnet 5.5: 4 - 1440 stills: header, filter bar, panel titles and tabs stay put while Decisions, Tickets and Activity scroll independently; Launch/Ask are fixed at the top of the right column.

### R2

- Candidate, Claude Opus 5.5: 5 - Needs you is its own first column with a '7' badge, and the header shows an orange '7 need you' next to '17 running', updating to 9/10 in the live stills; the tab title is not visible.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png the global header shows '7 need you' and a dedicated pane and ticket group repeat the count, while 390-light.png carries it into the Needs you tab.
- Candidate, Claude Sonnet 5.5: 5 - Needs you is a first-class left queue with its count in the header ('7 need you' badge) and at 390 in the tab bar 'Needs you 7' and the panel badge.
- Baseline, Claude Opus 5.5: 3 - Decisions is a separate first panel with a small '7' after its title, but the header shows only '10 running' with no needs-you count.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png the separate Decisions pane carries the count of seven while the global header shows only the running count, and 390-light.png repeats seven in the Decisions tab.
- Baseline, Claude Sonnet 5.5: 3 - Decisions is its own first column with a count in its title, but the header only shows '10 running' with no needs-you count.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask inputs sit fixed at the top of the right column with Answers directly beneath Ask (live-typing keeps 'acme/widgets#52' in place across updates), and at 1024/390 they move to a fixed bottom bar; launch results land in the Tickets list rather than beside the input.
- Candidate, GPT 6.2 Sol: 4 - In both live-typing stills, the focused Launch draft stays beside the Tickets list and Ask sits directly over Answers, although the stills show no new launch result.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask sit fixed at the top of the right column with Answers directly below them; at 1024 and 390 they collapse into a fixed bottom bar.
- Baseline, Claude Opus 5.5: 4 - Launch and Ask sit fixed in the right column with Answers directly below Ask, and typed text ('acme/widgets#52') survives live updates in live-typing-t30; at 1024/390 they move to a fixed bottom bar, while launch results go to the Tickets list.
- Baseline, GPT 6.2 Sol: 4 - In both live-typing stills, Launch keeps the focused acme/widgets#52 draft alongside Tickets and Ask sits directly above its Answers pane, although the stills show no new launch result.
- Baseline, Claude Sonnet 5.5: 4 - Launch and Ask are pinned at the top of the right column with Answers directly beneath; at 1024 and 390 they become a fixed bottom bar detached from the Answers.

### R4

- Candidate, Claude Opus 5.5: 5 - Tickets are grouped by position (Needs you / Running / Waiting) with a small diamond, half-circle or X and a single colored verb (Blocked, Choose, Reply, Report), and the palette stays at orange, red and link blue.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png, a ticket's place under Needs you, Running or Waiting carries its primary state, then amber diamonds and short action words distinguish requests, with red reserved for the blocker.
- Candidate, Claude Sonnet 5.5: 4 - Status is mostly position (Needs you, Running, Waiting groups) with small diamond, half-circle and x marks plus a lead word; blue filled action buttons add some noise.
- Baseline, Claude Opus 5.5: 3 - Every ticket row carries a status word (running/waiting/failed/idle), a colored left bar and a second status mark on its third line, and needs-you rows interleave with waiting rows (gizmo#12) rather than being grouped by position.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png the tray uses restrained red and amber marks, but the center interleaves gizmo#12's waiting row with needs-you rows and relies on repeated row labels to convey urgency.
- Baseline, Claude Sonnet 5.5: 3 - Palette is restrained (red, amber, grey, blue) but a status word and dot lead every ticket row, plus gate/blocker chips on cards, so mark-per-row carries most status rather than position.

### R5

- Candidate, Claude Opus 5.5: 4 - Between live-hover-t0 and t30 the decision cards hold still with a '1 new' marker instead of inserting, ages are right-aligned, and Reply/Dismiss stay in the same spot; ticket rows keep a fixed two-line height, though decision cards differ in height by content.
- Candidate, GPT 6.2 Sol: 4 - From live-hover-t0.png to live-hover-t30.png the blocker Reply target stays put and ticket rows retain their two-line height and right-aligned ages, although decision cards with different actions have different heights.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows are uniform two-line rows with age right-aligned and PR mark in a fixed right slot, and t0/t30 only swap text and counts; decision cards still vary in height.
- Baseline, Claude Opus 5.5: 4 - Ticket rows keep a fixed three-line height between live-typing-t0 and t30 even as ledger#201 changes to idle/needs you, times are right-aligned, and the Decisions header shows '1 new' in hover-t30; the right column shows clock times rather than ages, and decision cards vary in height.
- Baseline, GPT 6.2 Sol: 3 - Between the live-typing stills, ticket rows keep their height and right-edge timestamps, but relative age sits within the needs-you text and the ledger#198 decision card is shorter than the adjacent cards.
- Baseline, Claude Sonnet 5.5: 3 - Ticket rows are a near-constant height with age right-aligned, but a row such as atlas#57 reserves blank space and PR sub-lines vary; Decisions cards change height with content and the live t0/t30 stills show rows reflowing.

### R6

- Candidate, Claude Opus 5.5: 3 - detail-open expands the relay#11 card inline in the Needs you list with the full message and 'Open the thread', pushing the cards below down; the list position is kept but the detail does not open beside the list.
- Candidate, GPT 6.2 Sol: 3 - In detail-open.png the relay#11 card exposes the full request and thread link in place, holding the center list alongside it but pushing widgets#41 lower than in live-hover-t30.png.
- Candidate, Claude Sonnet 5.5: 3 - detail-open: the chevron expands the card inline showing the full message and 'Open the thread', growing the card and pushing the next cards down.
- Baseline, Claude Opus 5.5: 3 - detail-open expands the relay#11 card inline ('Hide message') with the full text under Reply/Dismiss, growing the Decisions list while keeping its position.
- Baseline, GPT 6.2 Sol: 3 - In detail-open.png relay#11's full message expands inside its card and moves the widgets#41 card down relative to live-hover-t30.png, where the same decision queue stays anchored with a '1 new' indicator.
- Baseline, Claude Sonnet 5.5: 3 - live-hover-t0: 'Full message' expands the card inline into 'Hide message' with the full body, growing the card and pushing the other cards down.

### G5

- Candidate, Claude Opus 5.5: 4 - In both stills the left Needs-you panel leads: it has its own warm tint (clear beige in 1366-light, faint warm cast in 1920-dark) and a stack of cards each with a solid blue primary button. Next, the ticket list opens with an amber-dotted Needs-you group above Running. Launch/Ask and the Activity feed read as a separate form-plus-feed column. It falls short of 5 because the solid blue Launch and Ask buttons in the top right compete with the Decisions buttons.
- Candidate, GPT 6.2 Sol: 5 - In L’s 1920 dark and 1366 light stills, the warm-backed stack of Decisions cards with blue action buttons draws the eye first, followed by the amber-marked ticket group in the center, while the right-side fields and feed have distinct silhouettes.
- Candidate, Claude Sonnet 5.5: 5 - In the 1366 light still the Needs you column has a warm tint, a bold header with an amber count badge, and raised white cards with solid blue action buttons, so it leads. In the 1920 still the tickets list opens with a Needs you group marked by amber dots before Running, and the Launch/Ask and Activity column is plainer, so each panel's role reads from its shape.
- Baseline, Claude Opus 5.5: 3 - Panels are separable in both stills: Decisions cards have amber left bars, and the first four ticket rows have an amber edge. But the Decisions actions are ghost buttons, so the only solid fills are the blue Launch and Ask buttons in the top right. In 1920-dark the large empty Answers box also pulls weight, so the eye lands on the right column, not on Decisions.
- Baseline, GPT 6.2 Sol: 3 - In R’s 1920 dark and 1366 light stills, the left card stack and center ticket rows have distinct shapes, but their matching amber rails and the bright blue buttons on the right compete for the first glance.
- Baseline, Claude Sonnet 5.5: 3 - In both stills the Decisions, Tickets and Activity panels are separable by column and by the amber left bars on the decision cards and the first four ticket rows. The Decisions header is small and shares the page surface with the other panels, and the grey Answers box draws the eye about as much, so Decisions does not clearly lead.

## Reference means

| Reference | P4 | P5 | P6 | N3 | N4 | N5 | R2 | R4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 4.33 | 3.00 | 4.00 | 4.00 | 4.00 | 1.33 | 4.00 |
| claude-code-agent-view | 5.00 | 5.00 | 3.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 |
| datadog-dashboard | 3.33 | 3.67 | 4.33 | 4.00 | 3.33 | 2.67 | 1.33 | 1.67 |
| datadog-oncall-pages | 4.00 | 4.00 | 4.00 | 4.00 | 4.00 | 4.33 | 3.67 | 3.00 |
| github-actions-workflow-graph | 3.67 | 4.00 | 3.00 | 5.00 | 4.67 | 4.33 | 1.00 | 4.33 |
| github-projects-board | 3.00 | 3.33 | 3.00 | 3.33 | 2.33 | 2.67 | 2.00 | 2.33 |
| github-projects-table | 3.33 | 5.00 | 3.00 | 4.00 | 3.33 | 3.33 | 1.67 | 2.00 |
| grafana-alert-rules | 3.00 | 4.00 | 5.00 | 2.33 | 2.33 | 3.67 | 1.67 | 3.67 |
| grafana-alerting-dashboard | 3.33 | 3.67 | 4.67 | 2.67 | 3.00 | 3.33 | 2.33 | 3.00 |
| jira-board | 2.67 | 3.00 | 3.00 | 3.67 | 2.67 | 2.33 | 1.33 | 2.33 |
| linear-inbox | 5.00 | 5.00 | 4.67 | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.67 | 4.33 | 4.00 | 5.00 | 2.00 | 4.00 |
| linear-triage | 5.00 | 4.67 | 4.67 | 5.00 | 5.00 | 5.00 | 4.00 | 4.67 |
| slack-channel-thread | 4.00 | 4.00 | 3.00 | 4.33 | 4.00 | 4.33 | 1.67 | 2.67 |
| thunderbird-unified-inbox | 4.00 | 4.33 | 3.00 | 4.33 | 3.67 | 4.33 | 3.00 | 3.33 |
| zulip-stream-topics | 4.00 | 4.00 | 3.00 | 4.67 | 4.00 | 4.33 | 2.67 | 4.00 |

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | X | Z |
| GPT 6.2 Sol | C | S |
| Claude Sonnet 5.5 | H | L |
