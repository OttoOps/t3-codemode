# Judging round 7

Candidate: the fixed panel board at `edf2ff3 (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-7-measurements.json`](round-7-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

The judges saw `edf2ff3`. Its measurement failed P2 on one negative margin in the phone card header (`margin-left: -12px`, also present in round 5). The final build, `be148ee` on this branch, replaces it with on-scale padding that renders the same phone stills apart from live data, and adds the fixes from code review: held worker changes, placeholder heights, a reply that loses its card, its handle or its connection during Undo, replies and dismissals the server checks against the decision they were made on, full messages bound to the handle that loaded them, and error Close returning focus. The table below is measured on that build with the harness as committed here.

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | pass | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded, and heavy with at least 30 open tickets and 26 decisions on the board at each size, counted by the same check |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | pass | a page script sampled every 20 ms for the whole of each case. Focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept in all 3358 samples, through a second-context answer and a stream cut; focused thread link, worker PR link, ask Settle, settled Restore stayed the same connected node in every sample for 30 s (at least 1500 each); hovered card option (answered from a second context and replaced by its same-height placeholder) and ticket Settle (through a stream cut) kept their position in every sample for 60 s (at least 3005 each) |
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 2 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 78 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 12 open rows visible at 1440 (12 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | pass | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; at a 250 px viewport the composer becomes a 250 px sheet; closing the keyboard restores panel decisions at scrollTop 200 |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 13:51."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | pass | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], no off-scale spacing |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 44 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green on `be148ee`; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F1 | pass | launch of one good and one unknown issue shows both results; with notifications granted (Notifications on) and the tab hidden, a new decision raised 1 notification(s), 1 confirmed shown by the browser: atlas#518 gate (b3); the rest of the inventory is covered as described below |
| F3 | pass | Ask error 1x (alert), completed Ask answer 1x (live), launch success 1x and failure 1x, card error 1x (alert), 0 announcements matching the sampled Activity patterns |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1's other items are covered by the gate scripts and tests above (a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests.

The U1 check counts the heavy board's open tickets and decisions at each window size. The harness sends each new gate to a ticket that arrived during the run and has none, so decisions pass 15.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 3.00 (3/3/3) |  | yes |
| U13 | 4.33 (4/5/4) | 2.00 (2/2/2) |  | yes |
| P4 | 4.33 (4/5/4) | 2.33 (2/3/2) | 3.83 | yes |
| P5 | 4.00 (4/4/4) | 2.33 (2/2/3) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.33 | yes |
| P7 | 4.33 (5/4/4) | 2.00 (2/2/2) |  | yes |
| N3 | 4.67 (5/5/4) | 2.00 (2/2/2) | 4.33 | yes |
| N4 | 4.00 (4/4/4) | 2.33 (2/3/2) | 3.83 | yes |
| N5 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.33 | yes |
| R1 | 5.00 (5/5/5) | 1.67 (2/1/2) |  | yes |
| R2 | 4.33 (4/5/4) | 3.67 (4/4/3) | 2.83 | yes |
| R3 | 4.00 (4/4/4) | 1.00 (1/1/1) |  | yes |
| R4 | 4.33 (4/5/4) | 2.00 (2/2/2) | 3.67 | yes |
| R5 | 4.00 (4/4/4) | 2.33 (2/3/2) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.50 | 2.50 | yes |
| Appearance and polish | 4.25 | 2.50 | yes |
| No slop | 4.33 | 2.11 | yes |
| Reference fit | 4.11 | 2.28 | yes |

**Decision rule: met.**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - At 1440 the left Decisions panel lists the a6 relay#11 blocker first in red, then the gates in amber, each with its ask in one or two lines and reply buttons (merge, accept/fix, one/split), so the queue reads off in seconds.
- Candidate, GPT 6.2 Sol: 4 - The first panel leads with relay#11's migration blocker, then shows widgets#41's merge or deploy choice, relay#9's retry budget, atlas#60's AWS sign-in, ledger#198's merge, atlas#57's one-or-two-PR choice, and gizmo#15's stalled worker, with the blocker's exact ask clipped.
- Candidate, Claude Sonnet 5.5: 5 - The first panel lists a6 relay#11 blocker (red stripe) first, then a1-a7 gates with their reply buttons and ages, and the ticket list repeats 'needs you - a6/a1/a4...' on the matching rows; readable in seconds.
- Baseline, Claude Opus 5.5: 3 - Decisions panel shows gate cards with large primary buttons, but the relay#11 blocker is not first (a1 gate leads) and only about four of seven decisions fit at 1440, so urgent items need scrolling to find.
- Baseline, GPT 6.2 Sol: 3 - The first panel shows widgets#41 choosing merge or merge and deploy for #81, relay#9 choosing the retry budget, atlas#60 needing AWS sign-in approval, and ledger#198 needing #203 merged, while three of the seven decisions sit below the fold.
- Baseline, Claude Sonnet 5.5: 3 - The Decisions column lists a1-a4 first with equal-weight cards; the a6 relay#11 blocker is not on the first screen at 1440 and the matching tickets are mixed into a long right column, so the answer takes most of the page.

### U13

- Candidate, Claude Opus 5.5: 4 - Three titled fixed columns at 1440 (Decisions, Tickets with Open/Done/Settled tabs, Launch/Ask/Activity); 1024 folds Activity into a tab and Launch/Ask into a bottom bar; 390 uses a bottom tab bar with counts. Panel titles are small but every panel has a stable place.
- Candidate, GPT 6.2 Sol: 5 - At 1440 the queue, tickets, and right rail have distinct boundaries, at 1024 Activity becomes a tab and the composers have a bottom bar, and at 390 Decisions, Tickets, and Activity have labeled tabs.
- Candidate, Claude Sonnet 5.5: 4 - Three fixed columns (Decisions, Tickets with Open/Done/Settled tabs, Launch/Answers/Activity rail) with bold titles and 1px dividers; at 1024 Activity becomes a tab and at 390 a bottom nav with a docked Launch bar; the rail's Launch/Ask split has weak separation.
- Baseline, Claude Opus 5.5: 2 - Decisions and Tickets sit side by side, but Activity, Settled, Launch and Ask stack below them and move down as content grows; at 390 everything is one long stacked column with no navigation.
- Baseline, GPT 6.2 Sol: 2 - Decisions and Tickets have labeled columns at 1440 and 1024, while at 390 only Decisions is visible and the Launch, Ask, and Activity sections sit after the lists in the desktop flow.
- Baseline, Claude Sonnet 5.5: 2 - Launch and Ask sit at the page end, Activity and Settled are collapsed disclosures, and the disconnected banner and empty state shift every section down, so positions depend on content and viewport.

### P4

- Candidate, Claude Opus 5.5: 4 - Red blocker rail and amber gate rails lead; in Tickets the needs-you rows carry a colored left rail and 'needs you - aN' text while idle/plain rows recede, though decision cards are all similar in weight.
- Candidate, GPT 6.2 Sol: 5 - The red relay#11 blocker leads directly into amber gates, while 'needs you' identifies the priority ticket rows ahead of routine tickets and the Activity feed.
- Candidate, Claude Sonnet 5.5: 4 - Bold panel titles, the red blocker card and amber stripes on needs-you cards pull the eye first, then needs-you tickets (amber 'needs you' text); secondary notes and the activity feed stay quiet.
- Baseline, Claude Opus 5.5: 2 - Panel headings ('Decisions 7', 'Tickets 15 open') are small gray text weaker than card content; big blue reply buttons stand out, but ticket rows with pills, gate chips and raw '**ACTION NEEDED:**' notes compete for attention.
- Baseline, GPT 6.2 Sol: 3 - Blue reply buttons draw the eye to Decisions, while bold ticket names and repeated badges compete on the right and the relay#11 blocker sits below earlier gates.
- Baseline, Claude Sonnet 5.5: 2 - Panel titles ('Decisions 7', 'Tickets 15 open') are small grey 13px, weaker than the card text under them; every card, ticket row and badge has similar weight and the blocker is not distinguished.

### P5

- Candidate, Claude Opus 5.5: 4 - Ticket rows keep status, name, PR link and time in the same columns at 1440, 1280 and 1024; the worker sub-line packs one or two PRs inline, and decision cards vary in height with or without reply buttons.
- Candidate, GPT 6.2 Sol: 4 - Status, title, main PR, and timestamp hold their columns at 1440, 1280, and 1024, while worker PR links run inline beneath the title.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows keep status, name, PR link and age in the same columns on every row at 1440, 1280 and 1024; decision cards share a header line with name, gate, Full message and age; minor variance in PR sub-line spacing and wrapping of text.
- Baseline, Claude Opus 5.5: 2 - Ticket rows wrap differently: notes run one to three lines, and at 1024 PR links drop to their own lines (ledger#198 PR #204 on a separate row, widgets#41 PR link under the title).
- Baseline, GPT 6.2 Sol: 2 - PR and thread links line up at 1440 and 1280, while widgets#41 and ledger#198 links wrap to separate lines at 1024 and status pills follow the variable-width names.
- Baseline, Claude Sonnet 5.5: 3 - Ticket rows align name, chip, PR link and thread link in rough columns, but row heights vary with note wrapping and PR sub-rows, the PR link shifts left when there is a long gate chip (ledger#198), and 1024 wraps PR links to a separate line.

### P6

- Candidate, Claude Opus 5.5: 4 - Dark theme is a designed near-black with visible panel borders; red, amber and blue are lightened for contrast and the left rails stay as clear as in light.
- Candidate, GPT 6.2 Sol: 5 - Dark desktop and phone retain distinct pane and field borders, with coral blockers, amber gates, and blue links separating the same roles as in light.
- Candidate, Claude Sonnet 5.5: 4 - Dark theme has distinct surface levels, readable red/amber stripes and borders on cards, dimmed disconnected state, and clear text hierarchy; boundaries are as clear as in light.
- Baseline, Claude Opus 5.5: 3 - Dark mode is correct and readable with cards slightly lighter than the page and pills recolored; the primary button turns light blue with dark text, and card boundaries are faint.
- Baseline, GPT 6.2 Sol: 4 - Periwinkle links, amber gates, and light message text remain legible on both dark stills, though some card and input edges rely on subtle charcoal differences.
- Baseline, Claude Sonnet 5.5: 3 - Dark theme is correct and readable with lighter blue primary buttons and amber gate chips, but cards sit on a near-identical dark surface so the boundaries are soft and it is a straight inversion of the light card look.

### P7

- Candidate, Claude Opus 5.5: 5 - Loading keeps the full layout with 'Loading decisions...'/'Loading tickets...' and 'Connecting' in the header; empty says 'Paste an issue into Launch to start one' and 'Type a question in Ask'; disconnected puts 'Reconnecting; showing data from 02:52' in the header and dims stale content in place.
- Candidate, GPT 6.2 Sol: 4 - Loading labels the lists, empty names the Launch and Ask controls, and reconnecting timestamps and dims retained data in place, though Answers says 'No answers yet' during loading.
- Candidate, Claude Sonnet 5.5: 4 - Empty says 'Nothing needs you right now.' and 'No open tickets. Paste an issue into Launch to start one.'; loading shows 'Connecting' with 'Loading decisions...' in the same layout; disconnected shows a header notice 'Reconnecting; showing data from 02:52.' with content dimmed in place.
- Baseline, Claude Opus 5.5: 2 - Loading shows blank white blocks under Decisions and Tickets with no text; the empty state uses positional copy ('Launch one below') and explains mechanics; disconnected has a clear 'Disconnected... Retrying.' banner but no data age.
- Baseline, GPT 6.2 Sol: 2 - Loading leaves blank Decisions and Tickets blocks, empty says 'Launch one below,' and the reconnect banner shifts the populated board down while retaining the ticket data.
- Baseline, Claude Sonnet 5.5: 2 - Loading shows blank rounded boxes with a 'connecting' dot; empty shows explanatory paragraphs and different layout; disconnected pushes content down with a banner naming 't3-codemode' while counts still look live.

### N3

- Candidate, Claude Opus 5.5: 5 - Strings are data, labels or instructions ('One issue per line. Ctrl+Enter launches.', 'Nothing needs you right now.'); empty states name controls, not positions.
- Candidate, GPT 6.2 Sol: 5 - Labels such as 'needs you,' the one-issue-per-line hint, and empty prompts naming Launch and Ask all describe data or an action.
- Candidate, Claude Sonnet 5.5: 4 - Strings are data or instructions ('One issue per line. Ctrl+Enter launches.', 'needs you - a6', 'checked 02:52 MST'); a few say little ('Answers', 'No answer yet.') and the empty answers copy 'Type a question in Ask.' is slightly positional.
- Baseline, Claude Opus 5.5: 2 - Copy explains implementation ('A short-lived thread reads the board and answers in a few lines.', 'Gates, blockers and reports from ticket threads land here...'), exposes ids ('Full message m43'), and shows raw markdown ('**ACTION NEEDED:**').
- Baseline, GPT 6.2 Sol: 2 - The Ask card explains that 'a short-lived thread reads the board,' the empty Decisions card describes what lands there, and 'Launch one below' describes a position.
- Baseline, Claude Sonnet 5.5: 2 - Strings like 'A short-lived thread reads the board and answers in a few lines.', 'Gates, blockers and reports from ticket threads land here...', 'Full message m43' (message id), 'Working on it...' explain the implementation or pad.

### N4

- Candidate, Claude Opus 5.5: 4 - Almost everything carries data; the worker sub-line repeats the parent status word ('running PR 1/1' under a 'running' row) and the 'Answers' subheading adds little.
- Candidate, GPT 6.2 Sol: 4 - The compact Launch/Ask bar repeats 'Launch' on the mode switch and submit button, where the second label adds little once Launch mode is selected.
- Candidate, Claude Sonnet 5.5: 4 - Almost every element carries data; the duplicate 'needs you - a1' and gate name repeats the decision card, and the 'Open 15 / Done 4 / Settled 26' count plus 'Activity 50' are marginal.
- Baseline, Claude Opus 5.5: 2 - Removable without loss: the internal 'm43' ids, ticket notes that repeat the decision text, PR links repeated on parent and worker rows, a 'thread' link on every row, and pills on every row and sub-row.
- Baseline, GPT 6.2 Sol: 3 - The empty Settled (0) disclosure and the Ask implementation blurb take space without adding a useful action in that state.
- Baseline, Claude Sonnet 5.5: 2 - Status chip on every ticket and PR row, 'gate' chip plus 'gate: merge' chip duplicating each other, header counts repeating the panel counts, 'PR 1/2' rows repeating the ticket name, and a 'Full message mNN' id carry little information.

### N5

- Candidate, Claude Opus 5.5: 4 - Dense bordered panes, no shadows or card grid, status in a left column and a fixed three-pane mail-client layout; it reads as a purpose-built operations tool, if a little plain.
- Candidate, GPT 6.2 Sol: 5 - The blocker-first decision queue, linked decision IDs in tickets, inline worker PRs, and separate activity rail make the layout specific to coordinating agents.
- Candidate, Claude Sonnet 5.5: 4 - Flat three-pane layout with left-edge status stripes, dense rows, text-first status and no pills reads like Linear or Thunderbird for this job, though the type is system-default.
- Baseline, Claude Opus 5.5: 2 - Centered max-width rounded white cards on a beige page with wide empty margins and pill badges everywhere; reads as a generic card dashboard.
- Baseline, GPT 6.2 Sol: 2 - Soft-shadowed decision cards and repeated pills give the board a familiar dashboard appearance, while ticket IDs and concrete reply choices provide some operational character.
- Baseline, Claude Sonnet 5.5: 2 - Rounded white cards with soft shadows, pill chips in blue/amber/green/grey on nearly every row, and a centered column grid read as a generic dashboard.

### R1

- Candidate, Claude Opus 5.5: 5 - Header, panel titles, tabs, Launch and Ask inputs stay fixed while Decisions, Tickets and Activity scroll inside their panes; t0/t30 pairs show nothing in the chrome moving.
- Candidate, GPT 6.2 Sol: 5 - Decision and ticket titles, the right rail, and the focused Launch field hold position throughout the typing pair, and the narrow layouts reserve bottom chrome for actions.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tabs and the Launch/Ask inputs stay fixed while each list scrolls independently; at 390 and 1024 the Launch bar and tab bar are docked at the bottom.
- Baseline, Claude Opus 5.5: 2 - Only the top status bar stays fixed; the page scrolls as one, so panel titles, Launch and Ask scroll away (typing stills show Decisions/Tickets scrolled under the header).
- Baseline, GPT 6.2 Sol: 1 - The connection bar stays visible in the typing pair while Decisions and Tickets titles leave the viewport and the Launch input moves down about 166 pixels.
- Baseline, Claude Sonnet 5.5: 2 - Header bar is sticky, but the page itself scrolls as one (typing stills show page scrolled to Launch at the bottom with panel titles gone); panels have no independent scroll.

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is its own leftmost queue with its count ('Decisions 7' rising to 10) and a '1 new' marker, but the header bar shows only '10 running', not the decision count.
- Candidate, GPT 6.2 Sol: 5 - The desktop Decisions header carries the live total and a '1 new' affordance, and the phone Decisions tab repeats the queue count.
- Candidate, Claude Sonnet 5.5: 4 - Decisions is its own first column with its count in the panel title and urgency ordering (blocker first), with a '1 new' marker; the header does not show the decision count, only '10 running'.
- Baseline, Claude Opus 5.5: 4 - Decisions is its own first section and its count shows in the header bar ('7 decisions'), though the blocker is not ranked first.
- Baseline, GPT 6.2 Sol: 4 - Decisions is a dedicated left-hand queue with a count in its own heading and in the top summary, and the phone shows the same count as static text.
- Baseline, Claude Sonnet 5.5: 3 - Decisions is its own column with a count in the header ('7 decisions'), but it is one of two equal columns, is not ordered by urgency (blocker a6 is not first), and tickets needing the user are not separated from the rest.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask are fixed at the top of the right pane with Answers directly under Ask, and the Launch input did not move between typing-t0 and t30; Launch sits apart from the Tickets list it feeds, and at 1024 it is a detached bottom bar.
- Candidate, GPT 6.2 Sol: 4 - The desktop Launch field stays fixed as updates arrive and Ask sits above Answers, while the narrow composer is shared across views and desktop Launch sits across the divider from Tickets.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask are fixed in the right rail beside the ticket list and answers appear directly beneath Ask; in typing t0 and t30 the input stayed at the same position while the lists updated, though it is not inside the ticket pane.
- Baseline, Claude Opus 5.5: 1 - Launch and Ask sit at the end of page content below Activity and Settled, and the focused Launch box moved about 165 px down between typing-t0 and t30 as new decisions arrived.
- Baseline, GPT 6.2 Sol: 1 - Launch and Ask sit after Decisions, Tickets, Activity, and Settled, and an incoming decision moves the focused Launch field down about 166 pixels.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask are at the very bottom of the page after Activity and Settled; in the typing stills the input moved ~166px between t0 and t30 as content grew and results appear far from the tickets they affect.

### R4

- Candidate, Claude Opus 5.5: 4 - Status is a small mark plus word in a fixed left column, the decisions queue and sorted needs-you rows carry most status, and the palette is red/amber/blue only.
- Candidate, GPT 6.2 Sol: 5 - The blocker leads the decision queue, attention-needed tickets are grouped high, and small squares, dots, and status words distinguish failed, waiting, and routine work.
- Candidate, Claude Sonnet 5.5: 4 - Status by position and a left stripe (red blocker, amber needs-you), then a small mark and a word (failed, waiting, running) with only red and amber plus muted grey; blue is limited to links.
- Baseline, Claude Opus 5.5: 2 - A colored pill on every ticket and worker row (running blue, idle gray, waiting amber, failed red) plus outlined gate chips; status is carried by chips, not position.
- Baseline, GPT 6.2 Sol: 2 - The Decisions column provides some positional grouping, while parent and worker ticket rows rely on repeated running, waiting, idle, and gate pills.
- Baseline, Claude Sonnet 5.5: 2 - Every ticket and PR row carries a coloured pill (running blue, idle grey, waiting amber, done green, failed red) plus gate pills, so many hues and chips; position carries no status.

### R5

- Candidate, Claude Opus 5.5: 4 - Ticket rows hold a fixed height with time right-aligned, and status changes swap text ('needs you - a6 +1', 'gate: review 1 min') without reflow; hover-t30 shows the pointed card unmoved with new items announced by a '1 new' pill. Decision cards differ in height.
- Candidate, GPT 6.2 Sol: 4 - The visible ticket rows share a fixed grid, decision ages stay right-aligned, and the same relay#11 Full message target remains highlighted in the hover pair, though decision cards use several heights.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows are a fixed grid with status left, age right, 'needs you' line and PR line in the same place, and decision actions sit right-aligned; a changed note swaps text on the same row, though some rows differ in height (with or without PR line) and the expanded decision card moves its Reply row.
- Baseline, Claude Opus 5.5: 2 - Ticket rows resize as notes change (widgets#41 shrinks from a three-line gate note to a one-line note between stills); decision cards are tall and vary, though ages are right-aligned.
- Baseline, GPT 6.2 Sol: 3 - The widgets#41 'merge and deploy' target and its right-aligned age stay put across the hover pair, while longer ticket summaries and varying worker counts change ticket row heights across the desktop stills.
- Baseline, Claude Sonnet 5.5: 2 - Ticket rows reflow with note length and sub-rows (a status change adds or removes lines, e.g. gizmo#12 note text changed height at t30), age is a top-right 'min' only on decisions, and decision cards vary with wrapping.

### R6

- Candidate, Claude Opus 5.5: 3 - 'Full message' expands inside the decision card (becoming 'Hide message'), growing it and moving Reply/Dismiss above the text; the list keeps position but grows.
- Candidate, GPT 6.2 Sol: 3 - Opening relay#11 keeps the card anchored at the top of Decisions and grows it about 20 pixels, moving the following cards down.
- Candidate, Claude Sonnet 5.5: 3 - 'Full message' expands inside the decision card, growing the card and pushing the following decisions down (Reply/Dismiss row moves above the body), the list scroll position kept but not detail beside the list.
- Baseline, Claude Opus 5.5: 3 - 'Full message m43' is an inline disclosure that grows the card by a monospace block and pushes the rest of the list down; no page jump.
- Baseline, GPT 6.2 Sol: 3 - Opening widgets#41 adds the full message under its controls in the card and shifts relay#9 down about 84 pixels.
- Baseline, Claude Sonnet 5.5: 3 - 'Full message' is an inline disclosure that grows the card and pushes later decisions down, with no page jump but the list shifts.

## Reference means

| Reference | P4 | P5 | N3 | N4 | N5 | R2 | R4 | P6 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 4.33 | 4.00 | 3.33 | 3.67 | 1.00 | 3.67 | - |
| claude-code-agent-view | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 | - |
| datadog-dashboard | 3.00 | 4.00 | 3.67 | 3.67 | 3.33 | 1.33 | 1.67 | 4.00 |
| datadog-oncall-pages | 4.00 | 4.00 | 4.33 | 4.00 | 4.33 | 4.00 | 3.00 | 4.00 |
| github-actions-workflow-graph | 3.67 | 4.00 | 4.67 | 4.67 | 4.33 | 1.00 | 4.33 | - |
| github-projects-board | 3.00 | 3.67 | 3.33 | 2.33 | 2.67 | 3.00 | 2.33 | - |
| github-projects-table | 3.33 | 5.00 | 4.33 | 3.33 | 3.67 | 1.67 | 2.00 | - |
| grafana-alert-rules | 2.33 | 4.00 | 2.00 | 2.00 | 3.33 | 2.00 | 4.00 | 4.67 |
| grafana-alerting-dashboard | 2.67 | 4.00 | 2.67 | 3.00 | 3.33 | 3.00 | 3.67 | 4.33 |
| jira-board | 2.67 | 3.33 | 3.67 | 2.67 | 2.33 | 2.00 | 2.33 | - |
| linear-inbox | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 4.67 | 3.67 |
| linear-issue-agent-panel | 4.00 | 4.00 | 5.00 | 4.33 | 5.00 | 2.67 | 4.33 | 4.33 |
| linear-triage | 5.00 | 5.00 | 4.67 | 4.67 | 5.00 | 4.33 | 5.00 | 4.33 |
| slack-channel-thread | 4.00 | 4.00 | 4.67 | 4.33 | 4.33 | 2.00 | 3.67 | - |
| thunderbird-unified-inbox | 4.00 | 4.00 | 4.00 | 3.33 | 4.33 | 3.67 | 3.67 | - |
| zulip-stream-topics | 4.00 | 4.33 | 4.67 | 4.33 | 4.33 | 3.00 | 4.33 | - |

## Judges' notes

- **Claude Opus 5.5**: A is a fixed three-pane tool: decisions ranked blocker-first, stable under live updates, good states; its biggest weakness is that decision cards vary in height and the header lacks the decision count, and at 390 the cards waste space with 'Full message' on its own line. B is a scrolling page of rounded cards: its biggest weakness is that Launch/Ask sit at the end of content and move while the user types (R3), with the page scrolling as one; it also leaks raw markdown and internal ids ('m43'), puts a pill on every row, and does not rank the relay#11 blocker first.
- **GPT 6.2 Sol**: B's biggest weakness is that new content physically moves the focused Launch control and pushes key tools below the working view. A's biggest weakness is that the highest-priority decision clips the actual ask in its preview, and the phone layout gives fewer decisions room on screen. B retains the same hovered merge-and-deploy target across its two stills, while A retains the same highlighted Full message link and surfaces a new-decision counter.
- **Claude Sonnet 5.5**: Biggest weakness in A: it is one scrolling page of soft cards with the inputs at the bottom of the page and no urgency ordering (the a6 blocker is not first), so Launch moved ~166px between typing t0 and t30 and sections reflow in the empty and disconnected states. Biggest weakness in B: the inline 'Full message' disclosure still grows the decision card and pushes the list (R6), the header omits the decision count, and the Answers/Activity rail truncates text at 1280; otherwise it is a fixed three-pane layout with stable rows in every still.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | A | B |
| GPT 6.2 Sol | A | B |
| Claude Sonnet 5.5 | B | A |
