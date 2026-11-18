# Judging round 4

Candidate: the fixed panel board at `e50b6d1 (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-4-measurements.json`](round-4-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | pass | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded and heavy (30 open tickets) |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | not run | the churn board had not started when `stability.js` ran, so this round has no U3 measurement; the next round measures it |
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 2 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 78 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 12 open rows visible at 1440 (12 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | pass | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; a 250 px viewport switches the composer to sheet mode (height and restore on close not measured) |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 02:33."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | pass | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], no off-scale spacing |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 44 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F3 | pass | Ask error 1x (alert), launch result 1x (both issues failed, so a mixed result was not exercised), card error 1x (alert), 0 announcements matching the sampled Activity patterns |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1 is covered by the gate scripts and tests above (a launch whose issues both fail, a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests. The scripts do not exercise the hidden-tab notification: it calls the same `Notification` code as before, and permission now comes from the header's Notify me control instead of the first click.

The heavy board reached 30 open tickets but held at 12 decisions: in this harness a newer gate from a ticket supersedes its older gate card, so the decision count plateaus below the rubric's 15. The 15-decision load was not measured in this round.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.33 (5/4/4) | 2.67 (3/2/3) |  | yes |
| U13 | 4.33 (4/5/4) | 2.33 (2/3/2) |  | yes |
| P4 | 4.33 (4/5/4) | 2.67 (2/3/3) | 3.67 | yes |
| P5 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.17 | no |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.00 | yes |
| P7 | 4.33 (4/5/4) | 2.00 (2/2/2) |  | yes |
| N3 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.17 | no |
| N4 | 4.00 (3/5/4) | 2.33 (2/3/2) | 3.83 | yes |
| N5 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.17 | yes |
| R1 | 5.00 (5/5/5) | 1.67 (2/2/1) |  | yes |
| R2 | 4.33 (4/5/4) | 4.00 (4/4/4) | 2.33 | yes |
| R3 | 4.33 (4/4/5) | 1.00 (1/1/1) |  | yes |
| R4 | 4.00 (4/4/4) | 2.00 (2/2/2) | 3.50 | yes |
| R5 | 4.33 (4/5/4) | 2.00 (2/3/1) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.33 | 2.50 | yes |
| Appearance and polish | 4.25 | 2.50 | yes |
| No slop | 4.11 | 2.11 | yes |
| Reference fit | 4.17 | 2.28 | yes |

**Decision rule: not met (P5, N3).**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - The Decisions panel leads with the a6 relay#11 blocker (red bar), then the gates with their reply buttons (merge / merge and deploy, accept / fix, one / split), each with its age. The Tickets panel repeats 'needs you - aN' at the top.
- Candidate, GPT 6.2 Sol: 4 - The first pane puts relay#11’s migration blocker first, then shows widgets#41’s merge or deploy choice, relay#9’s budget choice, atlas#60’s AWS sign-in, ledger#198’s merge, and atlas#57’s one-PR or two-PR choice, while the blocker’s exact question and gizmo#15’s full report require more reading.
- Candidate, Claude Sonnet 5.5: 4 - Left Decisions column at 1440 lists the red blocker (relay#11) first, then amber gates with their answer buttons (merge, accept/fix, one/split); only the blocker body is truncated mid-sentence.
- Baseline, Claude Opus 5.5: 3 - The left Decisions column lists a1 widgets#41 merge, a2 relay#9 accept/fix, a3 atlas#60 AWS sign-in and a4 ledger#198 merge, but cards are tall so only four fit, they run oldest-first, and the relay#11 blocker sits off-screen at the bottom of the ticket list.
- Baseline, GPT 6.2 Sol: 2 - The first 1440 still shows what widgets#41, relay#9, atlas#60 and ledger#198 want, while the remaining decisions, including the failed migration, sit outside the viewport.
- Baseline, Claude Sonnet 5.5: 3 - Decisions column starts with a1 gate cards with merge/accept buttons, but the blocker is not first (it is not visible in the first screen) and tickets with a gate repeat the same text, so you scroll and read more of the page to get the full list.

### U13

- Candidate, Claude Opus 5.5: 4 - Three titled, bordered panels (Decisions, Tickets with Open/Done/Settled tabs, a right column with Launch/Ask/Answers/Activity). At 1024 it folds to two panels with a docked Launch/Ask bar, and at 390 to a bottom tab bar. The right column holds four jobs, and Answers has a loose boundary.
- Candidate, GPT 6.2 Sol: 5 - At 1440, dividers give Decisions, Tickets and the Launch, Ask and Activity rail distinct homes, at 1024 Activity moves to a tab, and at 390 the counted bottom tabs preserve direct access to each list.
- Candidate, Claude Sonnet 5.5: 4 - Three titled columns (Decisions, Tickets with tabs, Launch/Ask/Answers over Activity) hold the same place at 1440, 1280 and 1024 (Activity folds into a tab), and 390 uses a bottom tab bar; Answers/Launch/Ask boundaries are a bit weak.
- Baseline, Claude Opus 5.5: 2 - Only Decisions and Tickets have fixed spots; Launch, Ask, Activity and Settled sit below the fold at a height that depends on content. At 390 everything stacks into one long page with no way to reach tickets.
- Baseline, GPT 6.2 Sol: 3 - Decisions and Tickets have clear columns at 1440 and 1024, while the 390 view starts a stacked card flow with no visible navigation to the other sections.
- Baseline, Claude Sonnet 5.5: 2 - Page flows as one document: Decisions and Tickets side by side, then collapsed Activity/Settled, then Launch and Ask at the bottom; Launch moved from y=557 to y=723 between typing t0 and t30 as the lists grew, and the empty state has a different arrangement.

### P4

- Candidate, Claude Opus 5.5: 4 - The eye goes to the red blocker and amber gate cards, then to the needs-you tickets marked by a left bar and bold 'needs you', then to plain rows. Bold panel titles anchor each area, and the sub-PR lines are a little noisy.
- Candidate, GPT 6.2 Sol: 5 - The red relay#11 blocker leads Decisions, amber gates follow, and the Tickets pane puts failed and marked needs-you work ahead of ordinary running rows as a group.
- Candidate, Claude Sonnet 5.5: 4 - Eye lands on the red-barred blocker then amber-barred gate cards with outlined action buttons, then 'needs you' amber lines in the ticket rows; panel titles are only modestly heavier than content.
- Baseline, Claude Opus 5.5: 2 - Section headings ('Decisions 7', 'Tickets 15 open') are small grey labels, weaker than the 16px decision text and the bold ticket names. Ticket rows repeat the gate text, so they compete with the decisions.
- Baseline, GPT 6.2 Sol: 3 - Decisions lead on the left and quick replies are prominent, while equally prominent ticket headers and repeated gate and running pills compete for attention.
- Baseline, Claude Sonnet 5.5: 3 - Decision cards with a filled blue primary button stand out, but the small grey 'Decisions 7' headings are weaker than the content and every ticket row competes with a status chip and gate chip.

### P5

- Candidate, Claude Opus 5.5: 4 - A status word, ticket name, PR link and time sit in the same columns on every ticket row at 1440, 1280 and 1024. Decision cards keep Reply/Dismiss right-aligned. On the phone the decision header splits awkwardly ('a6' far from 'relay#11').
- Candidate, GPT 6.2 Sol: 4 - Across all three desktop widths, ticket status, parent PR and time line up within their respective fields, while worker thread links run together in the lower text line.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows keep status word left, name, PR link and age right-aligned in the same columns on every row at 1440/1280/1024; decision actions Reply/Dismiss sit right on every card; sub-PR line and Activity truncation vary slightly.
- Baseline, Claude Opus 5.5: 2 - PR/thread links shift between rows: rows without a PR lose a column, rows with an idle sub-PR change shape, and at 1024 the links wrap onto their own line under widgets#41 and ledger#198. Row heights vary from 1 to 4 lines for the same fields.
- Baseline, GPT 6.2 Sol: 2 - PR and thread links largely share a right edge at 1440 and 1280, while at 1024 they wrap below widgets#41 and ledger#198 and status follows each name at a different offset.
- Baseline, Claude Sonnet 5.5: 2 - Chips sit right after each name so their x position drifts with name length, gate chips come and go, and at 1024 'PR #204 open' wraps onto its own line while other rows keep PR links inline.

### P6

- Candidate, Claude Opus 5.5: 4 - The dark theme keeps the red, amber and blue roles distinct and the panel borders and row separators visible. The disconnected dimming is not shown in dark.
- Candidate, GPT 6.2 Sol: 5 - Fine dividers separate the charcoal panes and rows, while coral blockers, amber gates, blue links and the active mobile tab remain easy to distinguish at both sizes.
- Candidate, Claude Sonnet 5.5: 4 - Dark 1440 and 390 keep the red/amber left bars, row dividers and panel boundaries; text contrast is good and roles read the same as light.
- Baseline, Claude Opus 5.5: 3 - The dark theme is correct and readable, with cards slightly lighter than the page, but the light-blue filled primary buttons and the pills read as a straight inversion rather than a designed palette.
- Baseline, GPT 6.2 Sol: 4 - The charcoal cards, lavender-blue links and amber gate outlines are legible on desktop and phone, with subtle borders around the response fields.
- Baseline, Claude Sonnet 5.5: 3 - Dark theme is correct and readable with a light-blue primary button, but cards barely separate from the background and chip colours get muddier than in light.

### P7

- Candidate, Claude Opus 5.5: 4 - All three keep the full layout. Loading says 'Loading decisions...' per panel. Empty says 'Nothing needs you right now.' and 'Paste an issue into Launch to start one.'. Disconnected says 'Disconnected. Reconnecting; showing data from 02:28.', dims the lists and greys out the reply buttons. Loading is plain text with no skeleton.
- Candidate, GPT 6.2 Sol: 5 - Connecting labels each loading list, empty Tickets and Answers name Launch and Ask, and the disconnected header gives both retry status and snapshot time while the panes stay in place.
- Candidate, Claude Sonnet 5.5: 4 - Empty keeps the layout with 'Nothing needs you right now.' and 'No open tickets. Paste an issue into Launch...'; loading shows 'Connecting' with 'Loading decisions...'; disconnected shows a red header line 'Disconnected. Reconnecting; showing data from 02:28.' and dims data and actions with no layout shift.
- Baseline, Claude Opus 5.5: 2 - Loading shows blank white blocks under 'connecting'. The empty state says 'Launch one below', which is positional. The disconnected state adds a red banner but leaves every button live and the header still claims '10 running'.
- Baseline, GPT 6.2 Sol: 2 - Connecting leaves Decisions and Tickets as blank cards, the empty state says ‘Launch one below,’ and the retry banner pushes still-active-looking controls down.
- Baseline, Claude Sonnet 5.5: 2 - Loading shows two blank white boxes with only 'connecting' in the header; empty is wordy and positional ('Launch one below'); disconnected adds a pink banner that pushes the content down, though the text itself is clear.

### N3

- Candidate, Claude Opus 5.5: 4 - The strings are data or instructions ('One issue per line. Ctrl+Enter launches.', 'No answers yet. Type a question in Ask.'). A stray 'Settle' word floats in the Answers card, and decision text is cut at 'here:...'.
- Candidate, GPT 6.2 Sol: 4 - Empty-state copy names Launch and Ask and the shortcut is actionable, while the connected header exposes the internal term ‘watcher’.
- Candidate, Claude Sonnet 5.5: 4 - Strings are data, labels or instructions ('One issue per line. Ctrl+Enter launches.'); empty copy mostly points to controls, with one 'progress here' positional phrase.
- Baseline, Claude Opus 5.5: 2 - Ticket rows reprint raw '**ACTION NEEDED:**' markdown truncated at 'https://githu'. 'Full message m43' exposes internal ids, the Ask box reads 'A short-lived thread reads the board and answers in a few lines.', and the empty state uses 'below'.
- Baseline, GPT 6.2 Sol: 2 - The Ask helper describes a ‘short-lived thread,’ the empty Decisions card explains its plumbing, and the empty Tickets card points ‘below’.
- Baseline, Claude Sonnet 5.5: 2 - Copy explains the implementation: 'A short-lived thread reads the board and answers in a few lines.', 'Gates, blockers and reports from ticket threads land here, with a button for each reply they offer.', plus raw 'Full message m43' ids.

### N4

- Candidate, Claude Opus 5.5: 3 - The Answers block takes a large fixed area for one running query with empty space under it. The sub-line 'o running PR 1/1' repeats the row's status, and the '?' in the header is unexplained.
- Candidate, GPT 6.2 Sol: 5 - The decision IDs tie back to needs-you rows, left-edge marks distinguish urgency, and counts, ages, links and controls each serve the queue workflow.
- Candidate, Claude Sonnet 5.5: 4 - Almost everything carries information; the 'needs you - a6' cross-reference repeats the decision queue and the '?' and 'Notify me' affordances are unexplained at a glance.
- Baseline, Claude Opus 5.5: 2 - The m43/m44 ids, the duplicated gate text in ticket rows, a 'running' pill on almost every parent and child row, and a separate 'thread' link per row add nothing a user needs.
- Baseline, GPT 6.2 Sol: 3 - The blue decision-ID chips and raw message-number suffixes such as m43 could be removed from the default cards without obscuring the ticket or the reply action.
- Baseline, Claude Sonnet 5.5: 2 - Header counts duplicate every panel title (7 decisions / Decisions 7), a status chip sits on every ticket and PR row, gate chips and truncated gate text repeat the decision cards, and 'Reply in your own words' inputs plus Send appear on every card.

### N5

- Candidate, Claude Opus 5.5: 4 - A dense three-pane layout with a left-edge colour bar, a status-word column and right-aligned times reads like a purpose-built ops tool. The outlined reply buttons are the only pill-like elements.
- Candidate, GPT 6.2 Sol: 5 - The persistent decisions queue, compact worker and PR lines, and live activity rail read as an operations console built around ticket handoffs.
- Candidate, Claude Sonnet 5.5: 4 - Dense three-pane inbox with left-edge status bars, plain rows and no card shadows or pill chips; looks like a purpose-built triage tool.
- Baseline, Claude Opus 5.5: 2 - Rounded white cards with soft shadows and pill badges on every row (running, idle, waiting, gate: merge) give a generic card-dashboard look.
- Baseline, GPT 6.2 Sol: 2 - Shadowed rounded cards and repeated status pills dominate the layout, although the worker tree and its PR links are specific to ticket operations.
- Baseline, Claude Sonnet 5.5: 2 - Rounded white cards, soft shadows, pill chips on every row and big blue buttons read as a generic AI dashboard.

### R1

- Candidate, Claude Opus 5.5: 5 - The header, panel titles, ticket tabs and the Launch/Ask inputs stay fixed across all live stills; only the decision, ticket and activity lists are cut off and scroll.
- Candidate, GPT 6.2 Sol: 5 - Both live pairs retain the top bar, pane headers and input coordinates while activity and counts change, and the 1024 and phone views dock the controls outside the list area.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tab strip, Launch/Ask inputs and Activity title stay put in the typing and hover stills while only the lists scroll; at 1024 and 390 the Launch bar and tab bar are fixed at the bottom.
- Baseline, Claude Opus 5.5: 2 - The header bar is sticky, but the page scrolls as one: in live-typing the Decisions cards and ticket list scroll under it and the panel titles scroll away.
- Baseline, GPT 6.2 Sol: 2 - The connection bar survives the scroll to Launch, but the Decisions and Tickets headings leave the viewport and Launch shifts down between the typing stills.
- Baseline, Claude Sonnet 5.5: 1 - In the live stills the whole page has scrolled, with only the header fixed; panel titles and lists scroll away together.

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is the first panel with its count in the panel header ('Decisions 7', and '1 new' when items arrive) plus needs-you rows in Tickets. The top header shows only '10 running', not the decision count.
- Candidate, GPT 6.2 Sol: 5 - Decisions is a dedicated first pane with a count in its header and a counted Decisions tab on the phone.
- Candidate, Claude Sonnet 5.5: 4 - Decisions is the first-class left queue with count in its title, a '1 new' marker and a blocker-first order; the header shows '10 running' rather than the decision count (the count appears in the 390 tab).
- Baseline, Claude Opus 5.5: 4 - Decisions is its own first column, and '7 decisions' sits in the top-right header next to the ticket and running counts.
- Baseline, GPT 6.2 Sol: 4 - Decisions has its own pane with a count, repeated in the global header, and the phone still shows that count above the cards.
- Baseline, Claude Sonnet 5.5: 4 - Decisions is its own left section with its count in the top bar ('7 decisions') and in its title, but it is part of a scrolling page rather than a fixed queue.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch stays put between typing-t0 and t30 in a fixed right column, and Ask's answers appear directly under it. Launched tickets land in the separate middle panel, and decision replies use Reply buttons inside each card.
- Candidate, GPT 6.2 Sol: 4 - The typed issue remains focused at the same coordinates in the desktop Launch rail and Answers sits directly beneath Ask, while the 1024 and phone views share a dock across panes.
- Candidate, Claude Sonnet 5.5: 5 - Launch and Ask inputs are pinned in the right column next to the Tickets list they act on, with Answers appearing directly under Ask; at 1024 and 390 the input is a fixed bottom bar.
- Baseline, Claude Opus 5.5: 1 - The Launch textarea sits at the end of the page under Activity and Settled. Between typing-t0 and t30 it moved from y~595 to y~760 as new decisions grew the page above it.
- Baseline, GPT 6.2 Sol: 1 - Launch and Ask appear after the lists and accordions, and the focused Launch box shifts from roughly y557 to y723 when a decision is added.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask live at the very bottom of the page below the Activity and Settled disclosures, so their position follows content length and they are at the end of content.

### R4

- Candidate, Claude Opus 5.5: 4 - Position carries most status: the decisions queue, needs-you tickets sorted to the top with a red or amber left bar, and the rest below. Each row has a small mark and a word ('o running', '■ failed', '● waiting') in a restrained red/amber/blue palette.
- Candidate, GPT 6.2 Sol: 4 - The blocker-first queue and the upper cluster of needs-you tickets do most of the sorting work, with a red square, amber dots and neutral rings reinforcing status.
- Candidate, Claude Sonnet 5.5: 4 - Position (Decisions queue, left bar, ticket order) carries urgency; status is a small dot/square plus one word (running, waiting, failed) in a restrained red/amber/green/blue set.
- Baseline, Claude Opus 5.5: 2 - Every ticket and sub-PR carries a coloured pill (blue running, amber waiting, grey idle, red failed, green done) plus outlined amber 'gate:' chips, so position carries almost nothing.
- Baseline, GPT 6.2 Sol: 2 - Decisions has a dedicated position, while ticket and worker rows use blue, green, gray, yellow and red pills with additional gate chips for status.
- Baseline, Claude Sonnet 5.5: 2 - A coloured pill (blue running, grey idle, amber waiting/gate, green done, red failed) sits on nearly every ticket and PR row, with no positional structure.

### R5

- Candidate, Claude Opus 5.5: 4 - Ticket rows are a constant ~65px with time right-aligned and the PR column fixed, and updates swap a time or note without resizing. New tickets such as atlas#62 still insert into the needs-you group and shift the rows below, but the hover target stays under the pointer and new decisions are held behind a '1 new' pill.
- Candidate, GPT 6.2 Sol: 5 - The hover pair keeps relay#11’s highlighted Full message target and adjacent Reply and Dismiss in place as the 1 new indicator appears, and decision ages hold their right edge.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows measure the same 65px for every state and age sits right-aligned, so t0 to t30 changes ('needs you' line appearing on ledger#201) swap text without resizing; decision cards still vary in height with their button rows.
- Baseline, Claude Opus 5.5: 2 - Ticket rows reflow on each update: widgets#41 shrinks from four lines to two when its gate text becomes a note, and gizmo#12 and ledger#201 change height too. Decision cards also vary with copy length.
- Baseline, GPT 6.2 Sol: 3 - Ages align right and widgets#41’s hovered ‘merge and deploy’ stays put through t30, while variable-length notes and worker counts give ticket rows different heights.
- Baseline, Claude Sonnet 5.5: 1 - Ticket cards reflow with every update: widgets#41's 3-line gate note became a 1-line note between t0 and t30 and the list shifted; no age column and rows have no fixed height.

### R6

- Candidate, Claude Opus 5.5: 3 - 'Hide message' expands the a6 card in place, which grows that card and pushes the rest of the list down. Reply/Dismiss jump above the body text, so the actions move.
- Candidate, GPT 6.2 Sol: 3 - Opening relay#11 reveals its full message inside the same card and keeps the headers in place, while the card grows and shifts widgets#41 downward.
- Candidate, Claude Sonnet 5.5: 3 - Full message expands inline inside the decision card, growing it and pushing the cards below, though the list stays in place with no page jump; hover target did not move between t0 and t30.
- Baseline, Claude Opus 5.5: 3 - 'Full message' is an inline disclosure that opens a monospace block inside the card and pushes the decisions below it down by about 85px. The list position is kept, but the list grows.
- Baseline, GPT 6.2 Sol: 3 - Full message expands a monospaced block inside widgets#41’s card and leaves Tickets alongside it, while pushing relay#9 and the following decisions down.
- Baseline, Claude Sonnet 5.5: 3 - Full message expands as an inline disclosure inside the card and pushes the cards below it down; no page jump and the hover target held position t0 to t30.

## Reference means

| Reference | P4 | P5 | N3 | N4 | N5 | R2 | R4 | P6 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 4.33 | 4.00 | 3.33 | 3.67 | 1.00 | 4.00 | - |
| claude-code-agent-view | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | - |
| datadog-dashboard | 3.33 | 3.67 | 3.67 | 3.00 | 3.00 | 1.00 | 2.00 | 3.67 |
| datadog-oncall-pages | 4.33 | 4.00 | 4.33 | 4.00 | 4.33 | 4.33 | 3.00 | 4.33 |
| github-actions-workflow-graph | 3.67 | 3.33 | 4.67 | 4.67 | 4.00 | 1.00 | 4.33 | - |
| github-projects-board | 3.33 | 3.00 | 3.00 | 2.33 | 2.67 | 2.67 | 2.33 | - |
| github-projects-table | 3.00 | 5.00 | 4.00 | 3.33 | 3.33 | 1.67 | 2.00 | - |
| grafana-alert-rules | 3.00 | 4.00 | 2.00 | 2.33 | 3.67 | 1.67 | 3.67 | 4.67 |
| grafana-alerting-dashboard | 2.67 | 3.33 | 2.33 | 2.67 | 3.33 | 2.67 | 3.00 | 4.00 |
| jira-board | 2.67 | 3.00 | 3.33 | 2.33 | 2.33 | 2.00 | 2.33 | - |
| linear-inbox | 4.67 | 4.67 | 5.00 | 4.33 | 5.00 | 4.67 | 4.33 | 4.00 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.00 | 4.00 | 5.00 | 2.00 | 4.00 | 4.67 |
| linear-triage | 4.67 | 5.00 | 5.00 | 4.67 | 5.00 | 4.00 | 4.33 | 4.00 |
| slack-channel-thread | 3.67 | 4.33 | 4.33 | 4.33 | 4.33 | 2.00 | 3.33 | - |
| thunderbird-unified-inbox | 4.00 | 4.33 | 4.33 | 3.67 | 4.33 | 3.33 | 3.33 | - |
| zulip-stream-topics | 4.00 | 4.33 | 4.67 | 4.33 | 4.33 | 3.00 | 4.00 | - |

## Judges' notes

- **Claude Opus 5.5**: Subject B is clearly stronger on structure and live behaviour: fixed three-pane chrome, inputs that never move, a blocker-first decision queue and needs-you tickets grouped at the top, and three state screens that keep the layout and are honest. B's biggest weakness: opening a decision's full message pushes the list down and moves Reply/Dismiss above the body, the right column crams four jobs together with a half-empty Answers area, and the header lacks the decision count. Subject A's biggest weakness: the page scrolls as one, so Launch/Ask sit at the end of the content and moved ~165px down in 30 s while the user typed. Ticket rows reflow on every update, the decisions are ordered oldest-first with the blocker missing from the queue view, and raw '**ACTION NEEDED:**' text plus pills on every row make it noisy. A's live-hover pair was stable (new decisions append at the end), which is its one behavioural plus. Note on references: several (buildkite, github-actions, jira, github-projects-table) have no person-queue concept, so their R2 score of 1 reflects absence, not a flaw in their own job.
- **GPT 6.2 Sol**: B’s biggest weakness is the two-line decision preview: relay#11’s exact drop-or-keep question is only visible after expanding the card, which moves subsequent decisions. A’s biggest weakness is placing Launch and Ask after the live lists: a single new decision moves the focused Launch field about 166 pixels down. A’s hovered quick reply and B’s hovered Full message link both stay in place in the supplied pairs.
- **Claude Sonnet 5.5**: Subject A: biggest weakness is that the header never shows the decision count at desktop widths (it is in the panel title) and decision cards still vary in height with their button rows; blocker text is truncated mid-sentence. Subject B: biggest weakness is the whole-page scroll with Launch/Ask at the end of content (its position moved 166px between typing t0 and t30), together with pill chips on every row and ticket text that duplicates the decision cards; the loading state is two blank boxes.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | B | A |
| GPT 6.2 Sol | B | A |
| Claude Sonnet 5.5 | A | B |
