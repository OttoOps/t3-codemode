# Judging round 5

Candidate: the fixed panel board at `6f10d7d (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-5-measurements.json`](round-5-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | partial | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded and heavy (30 open tickets); the heavy board held at 12 decisions, so the rubric's 15-decision load was not run |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | pass | focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept through a second-context answer and a stream cut; focused thread link, worker PR link, ask Settle, settled Restore stayed the same connected node for 30 s; hovered card option and ticket Settle did not move in the recorded samples |
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 2 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 78 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 12 open rows visible at 1440 (12 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | partial | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; a 250 px viewport switches the composer to sheet mode (height and restore on close not measured) |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 02:41."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | FAIL | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], off-scale spacing: `.ch .cid` margin-left -12px |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 43 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F3 | partial | Ask error 1x (alert), launch result 1x (both issues failed, so a mixed result was not exercised), card error 1x (alert), 0 announcements matching the sampled Activity patterns; no completed Ask answer was recorded |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1 is partly covered by the gate scripts and tests above (a launch whose issues both fail, a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests. The hidden-tab notification is not verified in this round: it calls the same `Notification` code as before, and permission now comes from the header's Notify me control instead of the first click.

The heavy board reached 30 open tickets but held at 12 decisions: in this harness a newer gate from a ticket supersedes its older gate card, so the decision count plateaus below the rubric's 15. The 15-decision load was not measured in this round.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.33 (4/4/5) | 2.67 (3/2/3) |  | yes |
| U13 | 4.33 (4/5/4) | 2.33 (2/3/2) |  | yes |
| P4 | 4.00 (4/4/4) | 3.00 (3/3/3) | 3.83 | yes |
| P5 | 4.00 (4/4/4) | 2.33 (2/2/3) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.67 | no |
| P7 | 4.00 (4/4/4) | 2.00 (2/2/2) |  | yes |
| N3 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.33 | yes |
| N4 | 4.00 (4/5/3) | 2.33 (2/3/2) | 3.83 | yes |
| N5 | 4.33 (4/5/4) | 2.33 (2/3/2) | 4.33 | yes |
| R1 | 5.00 (5/5/5) | 1.33 (1/2/1) |  | yes |
| R2 | 4.33 (4/5/4) | 4.00 (4/4/4) | 2.17 | yes |
| R3 | 4.00 (4/4/4) | 1.00 (1/1/1) |  | yes |
| R4 | 4.00 (4/4/4) | 2.00 (2/2/2) | 3.67 | yes |
| R5 | 4.33 (4/5/4) | 2.00 (2/2/2) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.33 | 2.50 | yes |
| Appearance and polish | 4.08 | 2.67 | yes |
| No slop | 4.22 | 2.22 | yes |
| Reference fit | 4.11 | 2.22 | yes |

**Decision rule: not met (P6; gate P2 failed; gates U1, U10 and F3 partly measured, F1 notification not verified).**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 4 - The first column 'Decisions 7' leads with relay#11 as a red-ruled blocker, followed by amber gates (widgets#41 merge, relay#9 accept/fix, atlas#60 AWS sign-in, ledger#198 merge, atlas#57 one/split), each with its reply buttons; the Tickets column repeats 'needs you - aN'. Body text truncates with '...', which costs a click on the blocker.
- Candidate, GPT 6.2 Sol: 4 - The 1440 Decisions pane leads with relay#11’s failed migration and shows widgets#41’s merge or deploy choice, relay#9’s retry budget, atlas#60’s AWS sign-in, ledger#198’s PR #203 merge, and atlas#57’s one-or-two-PR choice, while the exact asks for relay#11 and gizmo#15’s stalled worker are clipped.
- Candidate, Claude Sonnet 5.5: 5 - First panel at 1440 is Decisions 7 with the red 'blocker' relay#11 on top, then gates for widgets#41, relay#9, atlas#60, ledger#198, atlas#57 and a report for gizmo#15, each with its ask and reply buttons; readable in seconds.
- Baseline, Claude Opus 5.5: 3 - The left column lists widgets#41 merge, relay#9 accept, atlas#60 sign-in and ledger#198 merge with buttons, and the header says '7 decisions'. The relay#11 blocker is not in view at the top: the order follows age, not urgency, and only about 4 cards fit, so the full set needs scrolling.
- Baseline, GPT 6.2 Sol: 2 - The 1440 Decisions column shows widgets#41’s merge or deploy choice, relay#9’s retry budget, atlas#60’s AWS sign-in and ledger#198’s PR #203 merge, while atlas#57’s PR scope, relay#11’s migration blocker and gizmo#15’s stalled-worker report are below the fold.
- Baseline, Claude Sonnet 5.5: 3 - Decisions column lists a1 widgets#41 first with large cards (~220px each) so only three or four fit; the blocker relay#11 is not first and shows only low in the ticket list, so urgency order is not readable.

### U13

- Candidate, Claude Opus 5.5: 4 - At 1440 and 1280 there are three fixed columns (Decisions, Tickets with Open/Done/Settled tabs, Launch/Ask/Answers/Activity) with ruled title bars. At 1024 Launch/Ask fold into a bottom bar and Activity becomes a tab, and 390 uses bottom tabs, so every panel keeps a clear home; the right column's sub-sections are separated only by thin rules and a grey band.
- Candidate, GPT 6.2 Sol: 5 - At 1440 Decisions, Tickets and the utility rail have clear dividers, at 1024 Activity becomes a tab and Launch/Ask a bottom bar, and at 390 the counted Decisions and Tickets tabs and the Activity tab make each destination visible.
- Candidate, Claude Sonnet 5.5: 4 - Three titled columns (Decisions, Tickets with tabs, Launch/Answers/Activity) with hairline dividers, collapsing to two panes at 1024 and to a tab bar plus fixed launch bar at 390; Activity and Answers share a right column which blurs the home slightly.
- Baseline, Claude Opus 5.5: 2 - At 1440 there are two centred columns with wide empty margins. Launch, Ask, Activity and Settled are stacked below the fold, and their position depends on how long the lists above are (Launch moves between t0 and t30). Section titles are small grey labels.
- Baseline, GPT 6.2 Sol: 3 - Decisions and Tickets are labeled columns at 1440 and 1024, but the 390 viewport shows only stacked decision cards with no visible navigation to Tickets or Activity.
- Baseline, Claude Sonnet 5.5: 2 - Two columns at 1440 then Activity, Settled, Launch and Ask stacked at the end of the page; at t0 versus t30 the whole layout moved ~200px as content grew, so positions depend on content.

### P4

- Candidate, Claude Opus 5.5: 4 - The eye goes to the red blocker card and the 'needs you' rows in brown/red; plain running/idle tickets are visibly quieter. The duplicated needs-you emphasis in two columns competes slightly.
- Candidate, GPT 6.2 Sol: 4 - The red relay#11 blocker leads the amber gates and matching needs-you notes stand out in Tickets, although the blue Launch and Ask buttons also draw the eye and gizmo#12 sits among tickets with decisions.
- Candidate, Claude Sonnet 5.5: 4 - Bold panel titles, coloured left rail and red blocker lead the eye to decisions, but the ticket list repeats 'needs you - a1' in equal weight and competes.
- Baseline, Claude Opus 5.5: 3 - Big blue primary reply buttons make decisions stand out, but section headings are small grey text, weaker than the content. In Tickets, status pills, gate pills, long notes and link pairs all compete equally.
- Baseline, GPT 6.2 Sol: 3 - The blue reply presets make decisions prominent, but the blocker follows several gates and the Tickets column gives ordinary running rows and gated rows similar weight.
- Baseline, Claude Sonnet 5.5: 3 - Solid blue primary buttons pull the eye in decision cards, but ticket rows carry a status pill, gate pill, PR link and thread link each at similar weight.

### P5

- Candidate, Claude Opus 5.5: 4 - Ticket rows keep status in a left column, name and needs-you line in the middle, PR link and time right-aligned at 1440, 1280 and 1024. The PR link column shifts a little with link length ('PR #203 open' vs 'PR #33 draft'), and decision cards have different heights.
- Candidate, GPT 6.2 Sol: 4 - Ticket status, issue name, parent PR and timestamp stay in consistent columns at 1440, 1280 and 1024, while worker links flow inline and lack a dedicated thread column.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows keep status word at left, PR link and time right-aligned on every row at 1440, 1280 and 1024; only the 2 vs 3 line content differs.
- Baseline, Claude Opus 5.5: 2 - Ticket rows wrap differently depending on note length (1-3 lines). Chip positions vary with the name width, and at 1024 'PR #204 open' falls onto its own line. PR/thread links are right-aligned but rows have no shared columns.
- Baseline, GPT 6.2 Sol: 2 - PR and thread links generally line up at 1440 and 1280, but at 1024 widgets#41’s parent links and ledger#198’s second worker links drop below their status lines.
- Baseline, Claude Sonnet 5.5: 3 - Aligned at 1440, but at 1024 PR #204 wraps to its own line under ledger#198 and notes wrap to two lines, so rows differ by content and width.

### P6

- Candidate, Claude Opus 5.5: 4 - Dark keeps panel borders, the red/amber left rules, the needs-you colours and link blue all distinct with no muddy surfaces. Answers and Activity separate a little less clearly than in light.
- Candidate, GPT 6.2 Sol: 5 - At 1440 and 390, panel rules and field outlines stay distinct on charcoal, and coral blockers, amber gates, blue links and gray metadata each retain a clear role.
- Candidate, Claude Sonnet 5.5: 4 - Dark 1440 and 390 keep rails, card outlines and the blocker red readable with muted accent colours; boundaries slightly lower contrast than light but intact.
- Baseline, Claude Opus 5.5: 3 - Dark is correct and readable, with card surfaces separated from the background. The pastel primary buttons and pills look like a straight token swap, and boundaries are softer than in light.
- Baseline, GPT 6.2 Sol: 4 - Blue and amber labels remain legible on charcoal at desktop and phone widths, with clear button roles and relatively subtle separation between cards and canvas.
- Baseline, Claude Sonnet 5.5: 3 - Dark is correct and readable, buttons and chips keep contrast, but cards and rows sit close in tone and pills lose their roles; adapted rather than designed.

### P7

- Candidate, Claude Opus 5.5: 4 - Loading keeps the full layout with 'Connecting' and per-panel 'Loading decisions...'. Empty states point to controls ('Paste an issue into Launch to start one', 'Type a question in Ask'). Disconnected shows a red header line 'Reconnecting; showing data from 02:37' and dims the reply actions. The dimming is subtle and the activity feed keeps changing while disconnected.
- Candidate, GPT 6.2 Sol: 4 - The three-pane layout stays put through loading, empty and reconnecting, with explicit loading labels, a named Launch action and a last-data timestamp, though Answers says “No answers yet” during loading.
- Candidate, Claude Sonnet 5.5: 4 - Loading says 'Loading decisions...' with 'Connecting', empty says 'Nothing needs you right now' and 'Paste an issue into Launch', disconnected keeps the data, dims buttons and shows a red 'Disconnected. Reconnecting; showing data from 02:37' with no layout shift; loading is plain text only.
- Baseline, Claude Opus 5.5: 2 - Loading shows blank white blocks with no copy beyond a 'connecting' dot. Empty copy is long and positional ('Launch one below'). Disconnected adds a pink banner that pushes the layout down, while reply buttons stay fully active-looking.
- Baseline, GPT 6.2 Sol: 2 - The empty state gives a launch direction and the disconnect banner says it is retrying, but loading leaves blank Decisions and Tickets cards and the banner shifts both columns down.
- Baseline, Claude Sonnet 5.5: 2 - Loading shows two blank white boxes and a duplicated Launch form with no status text; empty copy is positional ('Launch one below') and explanatory; disconnected inserts a red banner that pushes everything down ~35px.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are data, labels or instructions ('One issue per line. Ctrl+Enter launches.'). 'No activity yet. Launched tickets report their progress in this list.' is slightly padded.
- Candidate, GPT 6.2 Sol: 5 - Empty Tickets and Answers point by name to Launch and Ask, and the launch shortcut, reconnection timestamp and decision labels all convey actionable information.
- Candidate, Claude Sonnet 5.5: 4 - Strings are data, labels or instructions ('One issue per line. Ctrl+Enter launches.', 'No answers yet. Type a question in Ask.'); 'No answer yet.' says little.
- Baseline, Claude Opus 5.5: 2 - Several strings explain the implementation or pad the page: 'A short-lived thread reads the board and answers in a few lines.', 'Gates, blockers and reports from ticket threads land here, with a button for each reply they offer.', 'Full message m43' exposes internal ids, and ticket notes echo raw '**ACTION NEEDED:**' markdown.
- Baseline, GPT 6.2 Sol: 2 - The empty Decisions card explains which thread messages land there, Ask describes spinning up a short-lived thread, and Tickets uses the positional instruction “Launch one below”.
- Baseline, Claude Sonnet 5.5: 2 - Explains the implementation: 'A short-lived thread reads the board and answers in a few lines.', 'Working on it...', 'Gates, blockers and reports from ticket threads land here, with a button for each reply they offer.'

### N4

- Candidate, Claude Opus 5.5: 4 - Nearly everything works. The decision ids 'a1'...'a7' and the '?' button add little, and 'needs you' rows in Tickets partly duplicate the Decisions queue.
- Candidate, GPT 6.2 Sol: 5 - The compact decision IDs match ticket needs-you notes, Full message restores clipped text, and the new-item counter signals queued work while the hovered item stays put.
- Candidate, Claude Sonnet 5.5: 3 - 'needs you - a1' plus a gate chip and 'gate: merge' repeat what the Decisions panel says; both 'Reply' and per-card 'Full message' on every card; the '?' control is unlabeled.
- Baseline, Claude Opus 5.5: 2 - Removable or redundant: the 'm43' message ids, the 'a1' badges, the 'gate' chip repeated as 'gate: merge', 'thread' links on every parent and child row, PR links repeated on parent and sub-PR rows, and full decision text duplicated inside ticket notes.
- Baseline, GPT 6.2 Sol: 3 - The top-bar Decisions total repeats the adjacent Decisions count, and the full-width collapsed “Settled (0)” control has nothing to expose in the empty state.
- Baseline, Claude Sonnet 5.5: 2 - Message ids ('Full message m43'), a 'thread' link on every row and PR sub-rows that repeat the ticket name, status pills on every ticket and sub-row, 'Settle all done' and per-answer Settle.

### N5

- Candidate, Claude Opus 5.5: 4 - Flat, dense, ruled panels with a mark plus a word for status, no shadows or pill soup; it reads as a purpose-built ops console close to Thunderbird and Linear in spirit.
- Candidate, GPT 6.2 Sol: 5 - The blocker-first decision queue, fixed ticket columns and live activity rail read as a triage tool for supervising issue and PR work.
- Candidate, Claude Sonnet 5.5: 4 - Dense three-pane operations tool with rails, hairlines and a restrained palette, no card grid or soft shadows; reads like a purpose-built inbox.
- Baseline, Claude Opus 5.5: 2 - Rounded white cards with soft shadows on a beige ground, pill badges on every row, and a centred max-width layout; it reads as a generic dashboard template.
- Baseline, GPT 6.2 Sol: 3 - Ticket IDs, worker PR links and decision reply presets are specific to the work, while rounded shadowed cards and repeated status pills give it a familiar dashboard treatment.
- Baseline, Claude Sonnet 5.5: 2 - Rounded white cards on a tinted page with soft shadows and pill badges in blue, amber, grey, green and red; generic dashboard look.

### R1

- Candidate, Claude Opus 5.5: 5 - Header, panel titles, Launch/Ask inputs and the tabs stay fixed while the Decisions, Tickets and Activity lists clip and scroll independently. The Launch input sits at the same position at typing t0 and t30.
- Candidate, GPT 6.2 Sol: 5 - The header, panel titles and focused Launch field occupy the same coordinates at t0 and t30 as ticket and activity contents change, and the 390 view keeps its composer and tab bar on screen.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tab strip, Launch/Ask inputs and the 390 launch bar and tab bar stay fixed; only decision list, ticket list and activity scroll.
- Baseline, Claude Opus 5.5: 1 - Apart from a thin sticky status bar, the page scrolls as one document. Panel titles and the Launch/Ask inputs scroll away, and between the typing stills the Launch box moves about 210 px down the viewport.
- Baseline, GPT 6.2 Sol: 2 - The connection strip remains at the top in the typing stills, while the Decisions and Tickets titles scroll away and Launch moves down about 214 pixels between captures.
- Baseline, Claude Sonnet 5.5: 1 - Only the top status bar is fixed; scrolling moves panel titles, tickets and inputs together, so the page scrolls as one.

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is a first-class left queue with its count in the panel title and a '1 new' pill at t30. The global header shows '10 running', not the decision count.
- Candidate, GPT 6.2 Sol: 5 - Decisions has a dedicated counted desktop header and a counted mobile Decisions tab, and a “1 new” control appears in the queue header during the hover pair.
- Candidate, Claude Sonnet 5.5: 4 - Decisions is its own first column with count 7 (then 9, 10 live) and a '1 new' marker, and tab count at 390, but the header shows only '10 running', not the decision count.
- Baseline, Claude Opus 5.5: 4 - Decisions is a separate queue and the header shows '7 decisions' (then 9 and 10 live). The queue is ordered by age, though, and the failed relay#11 sits in Tickets instead of at the top of Decisions.
- Baseline, GPT 6.2 Sol: 4 - Decisions is a dedicated list with a count beside its heading and in the connection strip, with no visible Decisions tab to repeat that count.
- Baseline, Claude Sonnet 5.5: 4 - Decisions is its own section with its count both in the panel title and the header ('7 decisions'), but the blocker is not first and the queue shares the page scroll.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask are fixed at the top of the right column, and Ask answers appear directly beneath them. Launched tickets land in the middle column rather than beside the input.
- Candidate, GPT 6.2 Sol: 4 - Launch stays fixed beside Tickets through typing updates and Ask sits directly over Answers, while at 1024 and 390 both inputs share a bottom bar with less immediate context.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask are fixed in the right column with answers appearing directly beneath, and a fixed bottom bar at 1024 and 390; they sit apart from the ticket list they act on.
- Baseline, Claude Opus 5.5: 1 - Launch sits at the end of the page, below Decisions, Tickets, Activity and Settled, and it moves as content grows. Ask answers do appear under Ask.
- Baseline, GPT 6.2 Sol: 1 - Launch and Ask sit after the ticket list, Activity and Settled, and incoming decisions move the still-focused Launch field from around y=556 to y=770.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask sit at the bottom of the page after Activity and Settled, so reaching them means scrolling past everything, and results land below them.

### R4

- Candidate, Claude Opus 5.5: 4 - Status is a small glyph plus a word ('o running', '* waiting', red-square 'failed') with a coloured left rule only on rows that need you. The decisions queue carries most of the urgency; the palette stays to red, amber and blue links, though the needs-you lines add a second colour cue.
- Candidate, GPT 6.2 Sol: 4 - The blocker-first Decisions queue, red square and amber gate dot make urgency legible with little color, though gizmo#12’s waiting row still interrupts the needs-you tickets.
- Candidate, Claude Sonnet 5.5: 4 - Left colour rail and position (failed first, needs-you rows next) carry status with a small mark and a word; only red, amber and green appear.
- Baseline, Claude Opus 5.5: 2 - Every ticket and sub-PR row carries a coloured pill (running blue, waiting amber, idle grey, failed red, done green) plus a gate pill. Status is carried by chips, not by position.
- Baseline, GPT 6.2 Sol: 2 - Running, idle, waiting and gate pills carry much of the status in the ticket list, where widgets#44 and gizmo#12 sit ahead of gated tickets.
- Baseline, Claude Sonnet 5.5: 2 - A coloured pill on every ticket and PR sub-row (running, idle, waiting, done, failed) plus gate pills in several hues; position carries no status.

### R5

- Candidate, Claude Opus 5.5: 4 - Ticket rows are a constant height with time right-aligned, and status changes swap the mark. At t30, new needs-you tickets (ledger#201, atlas#62) are inserted mid-list and shift the rows below. In the hover pair the pointer target stays put because new decisions show as a '1 new' pill.
- Candidate, GPT 6.2 Sol: 5 - Decision ages retain their right edge and the first card’s controls and ticket row heights stay put as timestamps and ticket notes update, with the incoming decision announced in the header during hover.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows step a constant ~65px, time right-aligned, status word at left; Reply/Dismiss in the same right slot on every decision; decision cards vary in height with message length.
- Baseline, Claude Opus 5.5: 2 - Ticket rows resize as notes change: widgets#41 shrinks from a three-line gate note at t0 to one line at t30, so every row below shifts. Decision cards vary in height. The hovered 'merge and deploy' stays put only because new decisions append at the bottom.
- Baseline, GPT 6.2 Sol: 2 - The hovered widgets#41 quick reply stays in place and decision ages line up, but its ticket note shrinks from three lines to one and moves every following ticket up about 38 pixels.
- Baseline, Claude Sonnet 5.5: 2 - Ticket rows vary from one to six lines with wrapped note text and PR links moving to their own line, and live updates reorder content (t0 vs t30 shift), so rows reflow.

### R6

- Candidate, Claude Opus 5.5: 3 - 'Full message' expands the blocker card in place (link becomes 'Hide message'), growing the list and moving Reply/Dismiss above the text. The list position is kept, but this is inline growth, not a side pane.
- Candidate, GPT 6.2 Sol: 3 - Opening relay#11 replaces its preview with the full migration message in the same card and shifts widgets#41 down about 20 pixels while Tickets and Activity stay put.
- Candidate, Claude Sonnet 5.5: 3 - 'Full message' expands inline inside the card and grows it, but it stays within the scrolling Decisions panel and list position is kept (a6 stays at top).
- Baseline, Claude Opus 5.5: 3 - 'Full message m43' opens an inline disclosure inside the card with a monospace block, pushing the following cards down; the list position is kept.
- Baseline, GPT 6.2 Sol: 3 - Opening the widgets#41 message reveals it below the reply controls and pushes the relay#9 decision about 84 pixels down without navigating away.
- Baseline, Claude Sonnet 5.5: 3 - Full message opens as an inline monospace block under the card and pushes the remaining decisions down, list kept in place but grown.

## Reference means

| Reference | P4 | P5 | N3 | N4 | N5 | R2 | R4 | P6 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.33 | 4.67 | 4.00 | 3.67 | 4.00 | 1.00 | 3.67 | - |
| claude-code-agent-view | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | - |
| datadog-dashboard | 3.67 | 3.67 | 4.00 | 3.67 | 2.67 | 1.00 | 2.00 | 4.00 |
| datadog-oncall-pages | 4.33 | 4.33 | 4.33 | 4.00 | 4.33 | 4.00 | 3.33 | 4.33 |
| github-actions-workflow-graph | 3.67 | 4.00 | 5.00 | 4.67 | 4.33 | 1.00 | 5.00 | - |
| github-projects-board | 3.00 | 3.33 | 3.33 | 2.33 | 2.33 | 2.33 | 2.00 | - |
| github-projects-table | 3.67 | 5.00 | 4.33 | 3.00 | 3.33 | 1.67 | 2.33 | - |
| grafana-alert-rules | 3.00 | 4.00 | 2.33 | 2.00 | 3.67 | 2.00 | 4.00 | 5.00 |
| grafana-alerting-dashboard | 3.33 | 3.33 | 2.67 | 2.67 | 3.33 | 2.33 | 3.67 | 4.67 |
| jira-board | 2.67 | 3.33 | 3.67 | 2.33 | 2.33 | 1.67 | 2.33 | - |
| linear-inbox | 4.67 | 5.00 | 5.00 | 4.33 | 5.00 | 5.00 | 5.00 | 4.67 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.67 | 4.33 | 5.00 | 2.00 | 4.00 | 4.67 |
| linear-triage | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 | 4.33 | 5.00 | 4.33 |
| slack-channel-thread | 4.00 | 4.00 | 4.33 | 4.33 | 4.33 | 2.00 | 3.00 | - |
| thunderbird-unified-inbox | 4.00 | 4.00 | 4.33 | 3.67 | 4.33 | 3.33 | 3.33 | - |
| zulip-stream-topics | 4.00 | 4.00 | 4.67 | 4.33 | 4.33 | 3.00 | 4.00 | - |

## Judges' notes

- **Claude Opus 5.5**: A is a fixed three-pane console: lists scroll inside panes, the inputs stay put under live updates, and status is a mark plus a word. Its biggest weakness: new needs-you tickets are inserted mid-list and shift the rows below, and the duplicated needs-you emphasis in Decisions and Tickets dilutes priority. 'Full message' also grows the card inline and moves Reply/Dismiss. B is a single scrolling document of rounded cards and pills. Its biggest weakness is the layout: Launch/Ask/Activity sit at the end of the page and move as lists grow. Ticket rows resize as notes change, loading is blank boxes, and copy leaks implementation details (m-ids, '**ACTION NEEDED:**' markdown, 'short-lived thread'). B's large primary reply buttons and inline free-text reply field are a strength A lacks. In both subjects the blocker relay#11 is handled differently: A leads with it, B leaves it out of the decision queue's top.
- **GPT 6.2 Sol**: B’s biggest weakness is that its two-line preview cuts off the exact ask on the top migration blocker and the stalled-worker report, requiring Full message to finish triage. A’s biggest weakness is that the launch composer lives below the live lists and moves about 214 pixels during the typing pair. Both hover targets remain on the same item in the supplied captures: B shows an underline on relay#11’s Full message link at t0 and t30, and A shows the hover outline on widgets#41’s merge and deploy button at both times. The B screenshots therefore show target stability on that link, with no direct hover observation for its Reply button.
- **Claude Sonnet 5.5**: A biggest weakness: the Tickets list repeats the same 'needs you - aN gate: ...' text that the Decisions panel already shows, and decision card heights and truncated messages ('Needs a decision on...') vary; Full message grows the card inline. B biggest weakness: the page scrolls as one, so Launch and Ask end up at the bottom after Activity and Settled and everything moves as live updates add content (about 200px between typing t0 and t30), while the blocker is not first in the decisions list and every row carries status chips. B's loading state shows blank boxes and its disconnected banner shifts the layout. Dark mode for A is more deliberately differentiated than B. Subject rows in A change little between t0 and t30 and hover and typing focus stayed put in both.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | A | B |
| GPT 6.2 Sol | B | A |
| Claude Sonnet 5.5 | A | B |
