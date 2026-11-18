# Judging round 2

Candidate: the fixed panel board at `8c159f5 (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-2-measurements.json`](round-2-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | partial | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded and heavy (more than 41 open tickets); the heavy board held at 12 decisions, so the rubric's 15-decision load was not run |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | pass | focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept through a second-context answer and a stream cut; focused thread link, worker PR link, ask Settle, settled Restore stayed the same connected node for 30 s; hovered card option and ticket Settle did not move in the recorded samples |
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 3 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 110 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 12 open rows visible at 1440 (12 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | partial | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; a 250 px viewport switches the composer to sheet mode (height and restore on close not measured) |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 02:09."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | pass | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], no off-scale spacing |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 55 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F3 | partial | Ask error 1x (alert), launch result 1x (both issues failed, so a mixed result was not exercised), card error 1x (alert), 0 announcements matching the sampled Activity patterns; no completed Ask answer was recorded |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1 is partly covered by the gate scripts and tests above (a launch whose issues both fail, a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests. The hidden-tab notification is not verified in this round: it calls the same `Notification` code as before, and permission now comes from the header's Notify me control instead of the first click.

The heavy board reached more than 41 open tickets but held at 12 decisions: in this harness a newer gate from a ticket supersedes its older gate card, so the decision count plateaus below the rubric's 15. The 15-decision load was not measured in this round.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 3.00 (3/3/3) |  | yes |
| U13 | 4.67 (5/5/4) | 2.00 (2/2/2) |  | yes |
| P4 | 4.00 (4/4/4) | 2.67 (2/3/3) | 3.67 | yes |
| P5 | 4.33 (4/5/4) | 2.33 (3/2/2) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.33 | yes |
| P7 | 4.33 (4/4/5) | 2.00 (2/2/2) |  | yes |
| N3 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.33 | no |
| N4 | 3.33 (3/4/3) | 2.00 (2/2/2) | 3.67 | no |
| N5 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.17 | yes |
| R1 | 5.00 (5/5/5) | 1.67 (2/2/1) |  | yes |
| R2 | 4.67 (4/5/5) | 3.67 (3/4/4) | 2.33 | yes |
| R3 | 4.33 (4/4/5) | 1.00 (1/1/1) |  | yes |
| R4 | 4.33 (4/5/4) | 2.00 (2/2/2) | 3.50 | yes |
| R5 | 4.33 (4/5/4) | 2.33 (2/3/2) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.67 | 2.50 | yes |
| Appearance and polish | 4.25 | 2.58 | yes |
| No slop | 3.89 | 2.00 | yes |
| Reference fit | 4.28 | 2.28 | yes |

**Decision rule: not met (N3, N4; gates U1, U10 and F3 partly measured, F1 notification not verified).**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - The first panel leads with a red-bordered a6 relay#11 blocker, then the amber gate cards (widgets#41 merge, relay#9 accept/fix, atlas#60 AWS sign-in, ledger#198 merge, atlas#57 one/split), each with its reply buttons. The ticket rows repeat 'needs you - aN' in amber.
- Candidate, GPT 6.2 Sol: 4 - The queue leads with relay#11's failed migration, then shows widgets#41's merge or deploy, relay#9's budget choice, atlas#60's sign-in, ledger#198's merge, atlas#57's PR split and the start of gizmo#15's no-PR report, but truncates the blocker before the requested decision.
- Candidate, Claude Sonnet 5.5: 5 - First panel lists relay#11 blocker first with a red rail, then amber gate cards each with its reply buttons; the ticket list echoes 'needs you - a6' per ticket, so it reads in seconds.
- Baseline, Claude Opus 5.5: 3 - The left Decisions panel lists a1 widgets#41 merge, a2 relay#9 accept/fix, a3 atlas#60 AWS sign-in and a4 ledger#198 merge with clear asks, but they are sorted by age. The relay#11 blocker is not in the first viewport, so you can tell what is pending but not what is most urgent.
- Baseline, GPT 6.2 Sol: 3 - The first panel shows widgets#41 asking to merge or deploy PR #81, relay#9 asking to keep the budget or lower it to 3, atlas#60 asking for AWS sign-in approval, and ledger#198 asking to merge PR #203, with the blocker and remaining decisions below the fold.
- Baseline, Claude Sonnet 5.5: 3 - Decisions card list is on the left, but the blocker (relay#11) is not first, each card is ~220px tall so only 3-4 of 7 fit, and the ticket panel repeats the same gate text, so it takes a read of most of the page.

### U13

- Candidate, Claude Opus 5.5: 5 - Three fixed columns (Decisions | Tickets with Open/Done/Settled tabs | Launch, Ask, Answers, Activity) with ruled headers. At 1024, Activity folds into a tab and Launch/Ask become a fixed bottom bar. At 390, a bottom tab bar (Decisions 7, Tickets 15, Activity) sits under a fixed Launch bar.
- Candidate, GPT 6.2 Sol: 5 - At 1440, dividers give Decisions, Tickets and the command and activity sections clear homes, at 1024 Activity becomes a tab with a bottom command bar, and at 390 Decisions, Tickets and Activity remain named in bottom navigation.
- Candidate, Claude Sonnet 5.5: 4 - 1440 has three clearly titled columns (Decisions / Tickets with tabs / Launch+Answers+Activity), 1024 folds Activity into a tab with a bottom Launch bar, and 390 uses bottom tabs; the right column is cramped (Answers clipped) at 1440.
- Baseline, Claude Opus 5.5: 2 - Two floating card columns are centered with wide gutters. Launch and Ask sit below Activity/Settled disclosures at the end of the page and moved about 165px between typing t0 and t30. At 390 only the Decisions cards show, with Dismiss wrapped onto its own line.
- Baseline, GPT 6.2 Sol: 2 - Decisions and Tickets have recognizable desktop columns at 1440 and 1024, but the 390 view shows a stack of decision cards with other sections out of view and the desktop Launch position varies with list content.
- Baseline, Claude Sonnet 5.5: 2 - At 1440 the two panels sit side by side but Activity, Settled, Launch and Ask are stacked below the end of content, the 390 view is one long column with no Launch or Ask in view, and positions move with content (typing stills show the page scrolled down to reach Launch).

### P4

- Candidate, Claude Opus 5.5: 4 - Decisions take the left column with colored left borders, blocker first. Ticket rows put 'needs you' in amber bold and keep other rows plain. Panel titles are bold, though the Answers and Activity panels still compete a little for weight.
- Candidate, GPT 6.2 Sol: 4 - The red relay#11 blocker leads Decisions and marked needs-you tickets sit near the top, although blue Launch and Ask controls compete for attention and waiting gizmo#12 interrupts the marked rows.
- Candidate, Claude Sonnet 5.5: 4 - Blocker is red, gates carry amber rails, ticket names are bold with an amber 'needs you' line above plain notes; the eye goes to decisions then flagged tickets, though body text is dense.
- Baseline, Claude Opus 5.5: 2 - Section headings ('Decisions 7', 'Tickets 15 open') are small gray text, weaker than the large blue merge/accept buttons and the bold ticket names. Ticket notes with raw '**ACTION NEEDED:**' text compete with the decisions.
- Baseline, GPT 6.2 Sol: 3 - Large prompts and blue response buttons give Decisions weight, while the Tickets column mixes gated work with ordinary running and idle rows and shows competing status chips.
- Baseline, Claude Sonnet 5.5: 3 - Large blue primary buttons and tall cards make decisions the loudest thing, but running/idle/waiting/gate pill chips on every ticket compete and nothing orders urgent over routine.

### P5

- Candidate, Claude Opus 5.5: 4 - Ticket rows use fixed columns (status word at left, name, PR link, thread, time right-aligned) at 1440, 1280 and 1024. The PR link column shifts when it is empty or reads '#203 open', and the decision-card action rows differ by button count.
- Candidate, GPT 6.2 Sol: 5 - At 1440, 1280 and 1024 the parent ticket rows keep status, ticket name, PR, thread and time in consistent columns, with worker references in a separate subline.
- Candidate, Claude Sonnet 5.5: 4 - Status word sits in the same left column, PR/thread/age in right columns on each ticket row and sub-line across 1440 and 1280; right-aligned PR text ends at slightly different x, and the 1024 view drops to a different layout.
- Baseline, Claude Opus 5.5: 3 - PR and thread links are mostly right-aligned, but rows vary in height (3-line notes against 1-line ones), PR #203 starts at a different x than PR #81, and the 1024 layout wraps the PR links onto a separate line for some rows.
- Baseline, GPT 6.2 Sol: 2 - Status starts after ticket names of varying lengths, and at 1024 the PR and thread links for widgets#41 and the second ledger#198 worker drop to a separate line.
- Baseline, Claude Sonnet 5.5: 2 - Ticket rows vary from 1 to 3 note lines plus sub-rows, status chips follow the name at varying x, and PR/thread links start at different x per row (PR #81 open vs PR #203 open).

### P6

- Candidate, Claude Opus 5.5: 4 - In dark mode the panel boundaries, red and amber left borders, amber 'needs you' text and the dimmed meta all stay distinct, and the Answers panel keeps its tinted background, so roles are as clear as in light.
- Candidate, GPT 6.2 Sol: 5 - Desktop dividers and phone card borders remain visible, and red blockers, amber gates, muted routine status and blue actions keep distinct roles in both dark stills.
- Candidate, Claude Sonnet 5.5: 4 - 1440-dark keeps the red/amber rails, the divider lines between panels and the muted vs bold text roles as clear as light, with darker surfaces per panel.
- Baseline, Claude Opus 5.5: 3 - Dark mode is a correct inversion: the cards stay distinct from the background, and the pills and buttons are readable. The light blue primary buttons are somewhat washed out, and nothing appears designed specifically for dark.
- Baseline, GPT 6.2 Sol: 4 - Both dark stills distinguish decision cards and inputs from their backgrounds, and blue actions and amber gates remain legible, though ticket dividers are faint.
- Baseline, Claude Sonnet 5.5: 3 - 1440-dark is correct and readable with lighter cards on dark ground and recolored chips, but it is the light design inverted with soft low-contrast card edges and no distinct roles.

### P7

- Candidate, Claude Opus 5.5: 4 - Loading keeps the full layout with per-panel 'Loading decisions...' and 'Loading tickets...'. Empty points to a control ('Paste an issue into Launch to start one'). Disconnected turns the header red with 'Reconnecting; showing data from 02:04.' and dims the stale data. Loading is plain text with no skeleton.
- Candidate, GPT 6.2 Sol: 4 - Loading decisions, tickets and activity is explicit, the empty ticket pane names Launch, and reconnecting gives the cached timestamp without shifting panels, although the dimming reduces cached text legibility.
- Candidate, Claude Sonnet 5.5: 5 - Empty keeps all three panels and says 'Paste an issue into Launch to start one'; loading keeps the layout with 'Loading decisions...'; disconnected puts 'Disconnected. Reconnecting; showing data from 02:04.' in the header and dims the stale list and its buttons.
- Baseline, Claude Opus 5.5: 2 - Loading shows empty white blocks under 'connecting' with no text. Empty copy is positional ('Launch one below'). Disconnected adds a clear red banner, 'Disconnected from t3-codemode. Retrying.', but the stale data is not dimmed and its age is not given.
- Baseline, GPT 6.2 Sol: 2 - The empty state says to launch one below, loading leaves blank Decisions and Tickets blocks, and the retry banner shifts the populated board downward.
- Baseline, Claude Sonnet 5.5: 2 - Loading shows two blank white blocks with only 'connecting' in the header and the Launch section jumps up when data lands; empty copy says 'Launch one below' (positional); disconnected inserts a red banner that pushes the whole page down 35px and leaves data un-dimmed.

### N3

- Candidate, Claude Opus 5.5: 4 - The strings are data, labels or instructions ('One issue per line. Ctrl+Enter launches.', 'Nothing needs you right now'). The only padding is the empty-state clause 'it joins the board and reports here'.
- Candidate, GPT 6.2 Sol: 4 - The empty Tickets pane points to Launch and the shortcut helps users submit, but the one-issue-per-line instruction appears in both the Launch placeholder and helper.
- Candidate, Claude Sonnet 5.5: 4 - Strings are data or instruction ('One issue per line. Ctrl+Enter launches.', 'Working on it...', 'needs you - a1'); 'Nothing needs you right now. Gates, blockers and reports from tickets land here.' explains slightly, and '?' / 'Notify me' are terse.
- Baseline, Claude Opus 5.5: 2 - 'A short-lived thread reads the board and answers in a few lines' explains the implementation, 'Launch one below' is positional, 'Full message m43' exposes internal IDs, and the ticket notes leak raw markdown asterisks.
- Baseline, GPT 6.2 Sol: 2 - The Ask form explains the short-lived thread implementation, the ticket empty state says to launch one below, and the ticket summaries show raw ACTION NEEDED markers.
- Baseline, Claude Sonnet 5.5: 2 - 'A short-lived thread reads the board and answers in a few lines.' explains implementation, 'Full message m43' exposes message ids, header counts repeat panel headings, and '**ACTION NEEDED:**' raw markdown shows in ticket notes.

### N4

- Candidate, Claude Opus 5.5: 3 - Each ticket row repeats the decision text already shown in the Decisions panel, and 'thread' links repeat in every row and every decision card. Everything else carries information.
- Candidate, GPT 6.2 Sol: 4 - The one-issue-per-line phrase in the Launch placeholder can go because the helper repeats it, and the connected header could drop either Live or T3 connected without losing the immediate connection signal.
- Candidate, Claude Sonnet 5.5: 3 - Header counts duplicate the panel and tab counts, a 'thread' link on every ticket and decision repeats, and 'needs you - a1 ...' repeats the decision text beside it.
- Baseline, Claude Opus 5.5: 2 - The a1..a4 chips, 'Full message mNN' IDs, PR links duplicated between each ticket row and its PR sub-rows (PR #81 open twice), and a 'gate' pill repeated on every decision card all add nothing.
- Baseline, GPT 6.2 Sol: 2 - Removing the displayed Full message suffixes m43 and m44, the Ask implementation sentence, and the second identical PR #81 link would preserve the visible actions and their destinations.
- Baseline, Claude Sonnet 5.5: 2 - Several removable elements: header counts duplicate the panel headings, a1/a2 id badges, 'gate' chip on every decision, the ticket panel re-printing each decision's text, and per-PR idle/running chips.

### N5

- Candidate, Claude Opus 5.5: 4 - Dense ruled panes, small status words with marks, colored left borders instead of cards, and a tabbed tickets table: it reads like an operator tool, close to Linear and Thunderbird.
- Candidate, GPT 6.2 Sol: 5 - The blocker-first Decisions pane, fixed ticket columns with worker PRs, and adjacent Answers and Activity read as an operator workspace.
- Candidate, Claude Sonnet 5.5: 4 - Fixed three-pane panels with divider lines, rail marks and a tab strip look like a triage tool; some default blue button styling remains.
- Baseline, Claude Opus 5.5: 2 - White rounded cards with soft shadows on an off-white page, colored pill badges on every row (running, idle, waiting, gate: merge): a generic card-dashboard look.
- Baseline, GPT 6.2 Sol: 2 - Rounded shadowed cards and repeated colored pills dominate the board even though nested PR workers and direct decision replies are specific to the workflow.
- Baseline, Claude Sonnet 5.5: 2 - Rounded white cards with soft shadows, pill chips in four hues and big blue buttons read as a generic card dashboard.

### R1

- Candidate, Claude Opus 5.5: 5 - The header, panel titles, ticket tabs, Launch textarea and Ask input stay put across t0 and t30. Only the Decisions, Tickets and Activity lists scroll inside their panes.
- Candidate, GPT 6.2 Sol: 5 - In the typing pair the status bar, panel titles and focused Launch box stay at the same coordinates as counts and Activity change, and the 1024 and phone views retain a dedicated command bar.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tab strip, Launch and Ask are fixed in every still; only the lists scroll, and at 1024 and 390 the Launch bar and tabs are pinned at the bottom.
- Baseline, Claude Opus 5.5: 2 - Only the top status bar is fixed. The page scrolls as one: in the typing stills the panel titles are gone, and the Launch input moved down as content grew above it.
- Baseline, GPT 6.2 Sol: 2 - The connection bar stays at the top in the typing pair, but Decisions and Tickets headings have scrolled away and the focused Launch box moves down about 166 pixels.
- Baseline, Claude Sonnet 5.5: 1 - The page scrolls as one: panel titles, Activity, Settled and both inputs scroll away; only the thin header stays (typing stills show the page scrolled so Launch is reachable).

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is its own first panel, its count is in the header ('7 decisions' in amber), and a '1 new' pill appears when items arrive. The tab title is not visible in the stills.
- Candidate, GPT 6.2 Sol: 5 - Decisions is the first desktop pane with a count in its header and the status bar, and the phone Decisions tab also carries the count.
- Candidate, Claude Sonnet 5.5: 5 - Decisions is the first column with a count in its title and in the header ('7 decisions'), a '1 new' marker in the panel when items arrive, and 'needs you' rows on the ticket list.
- Baseline, Claude Opus 5.5: 3 - Decisions is a separate panel, and '7 decisions' appears in the header, but in small plain text. The relay#11 blocker sits outside the queue's visible top and shows up only as a 'failed' ticket further down.
- Baseline, GPT 6.2 Sol: 4 - Decisions is a dedicated first column with a count in both its heading and the desktop status bar, and the phone still shows the counted heading without a queue tab.
- Baseline, Claude Sonnet 5.5: 4 - Decisions is its own first panel with its count in the header ('7 decisions') and panel title, but it is not urgency sorted and the tab title is not shown.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask are fixed at the top of the right column, and answers appear directly below Ask. Launch is beside the Tickets pane rather than inside it, and at 1024/390 it moves to a fixed bottom bar.
- Candidate, GPT 6.2 Sol: 4 - Launch stays in the upper-right command area alongside Tickets and Ask has Answers directly beneath it, while the 1024 and phone layouts move the forms into a shared bottom bar.
- Candidate, Claude Sonnet 5.5: 5 - Launch and Ask sit fixed at the top of the right pane (bottom bar at 1024/390), focused in typing t0 and at the same position at t30, with Answers directly below.
- Baseline, Claude Opus 5.5: 1 - Launch and Ask sit at the bottom of the page after all the content. Launch moved about 165px between t0 and t30 as decisions were added above it.
- Baseline, GPT 6.2 Sol: 1 - Launch sits below the ticket and decision lists, Activity and Settled in both typing stills, and the focused box moves down when a decision is added.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask sit at the very end of the page below Activity and Settled; the typing t0 still had to scroll to the bottom to reach the input, and answers appear only down there.

### R4

- Candidate, Claude Opus 5.5: 4 - Status is a small mark plus a word in a fixed left column. Needs-you rows get an amber left border and a red border for failed, with only red, amber and neutral in use. Some marks are duplicated in the PR sub-line.
- Candidate, GPT 6.2 Sol: 5 - Queue order puts the red-square blocker ahead of amber-dot gates, while routine running and idle states stay gray and waiting uses a small amber mark and word.
- Candidate, Claude Sonnet 5.5: 4 - Left rails and a leading word (failed/running/waiting/idle) with a tiny dot carry status, with red and amber the only hues; 'running' repeats on many rows.
- Baseline, Claude Opus 5.5: 2 - Every ticket and PR row carries a colored pill (blue running, gray idle, amber waiting, red failed) plus an outlined gate pill. Status rides entirely on chips; position does not encode it.
- Baseline, GPT 6.2 Sol: 2 - The Decisions queue groups requests for the operator, but running, idle, waiting, done and failed rely on colored pills and additional outlined gate badges in the ticket list.
- Baseline, Claude Sonnet 5.5: 2 - A chip on nearly every ticket and sub-ticket (running blue, idle grey, waiting amber, gate amber outline, done green, failed red) and blue and grey buttons; position carries little status.

### R5

- Candidate, Claude Opus 5.5: 4 - Ticket rows are a fixed 65px, and the time is right-aligned and updates in place (01:20 to 02:05). New decisions raise a '1 new' pill instead of shifting the hovered item. A needs-you row (atlas#62) was inserted mid-list, which moved the rows below it.
- Candidate, GPT 6.2 Sol: 5 - Collapsed decision cards and ticket rows use consistent heights with right-aligned ages or times, and the underlined relay#11 Full message target stays in place as the hover pair shows a new-decision indicator and ticket updates.
- Candidate, Claude Sonnet 5.5: 4 - Age is right-aligned and thread/PR in fixed columns, row heights hold at ~65px between t0 and t30 as notes change; decision cards still vary in height by content.
- Baseline, Claude Opus 5.5: 2 - Ticket rows change height with note length: widgets#41 went from a 3-line note to a 1-line note between stills and every row below shifted. Ages in Decisions are right-aligned, and Send/Dismiss stay in a fixed place.
- Baseline, GPT 6.2 Sol: 3 - Ages align right and the hovered widgets#41 merge and deploy button holds its position from t0 to t30, but atlas#60 has a shorter card and Send sits higher within it.
- Baseline, Claude Sonnet 5.5: 2 - Decision cards and ticket rows differ in height by content (1-3 note lines, 0-2 sub-rows), new decisions and note changes resize rows between t0 and t30, and ticket rows show no age column.

### R6

- Candidate, Claude Opus 5.5: 3 - 'Full message' expands the a6 card in place, growing it about 20px and pushing the cards below it. Its Reply/Dismiss row jumps above the text, so the disclosure grows the list and changes the card layout.
- Candidate, GPT 6.2 Sol: 3 - The relay#11 message opens within the first card and reveals the partial-migration question, shifting the next decision down about 20 pixels while Tickets stays in place.
- Candidate, Claude Sonnet 5.5: 3 - 'Full message' expands the body inside the card and flips to 'Hide message', pushing the cards below down, in place with no page jump.
- Baseline, Claude Opus 5.5: 3 - 'Full message m43' expands inline as a monospace block that grows the card about 85px and pushes the rest of the list down. The list position is otherwise kept.
- Baseline, GPT 6.2 Sol: 3 - Full message expands inside the widgets#41 card and keeps its top in place, while relay#9 and the rest of the decision list move down about 84 pixels.
- Baseline, Claude Sonnet 5.5: 3 - 'Full message' is an inline disclosure that opens a mono block inside the card and pushes the list below it down; no page jump but it grows the list.

## Reference means

| Reference | P4 | P5 | P6 | N3 | N4 | N5 | R2 | R4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.33 | 4.33 | - | 3.67 | 3.67 | 3.67 | 1.33 | 4.00 |
| claude-code-agent-view | 5.00 | 5.00 | - | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 |
| datadog-dashboard | 3.33 | 3.67 | 3.67 | 4.33 | 3.33 | 3.00 | 1.00 | 2.00 |
| datadog-oncall-pages | 4.33 | 4.00 | 4.33 | 4.33 | 3.67 | 4.33 | 4.33 | 3.33 |
| github-actions-workflow-graph | 3.67 | 3.67 | - | 4.67 | 4.67 | 4.00 | 1.00 | 4.33 |
| github-projects-board | 2.67 | 3.00 | - | 3.00 | 2.00 | 2.33 | 2.33 | 2.00 |
| github-projects-table | 3.67 | 5.00 | - | 4.33 | 3.33 | 3.33 | 2.00 | 2.00 |
| grafana-alert-rules | 2.33 | 4.00 | 4.67 | 2.33 | 2.00 | 3.33 | 1.67 | 3.33 |
| grafana-alerting-dashboard | 2.33 | 3.33 | 4.67 | 2.67 | 2.67 | 3.33 | 2.33 | 3.67 |
| jira-board | 2.67 | 3.33 | - | 3.67 | 2.67 | 2.33 | 2.33 | 2.67 |
| linear-inbox | 4.67 | 5.00 | 4.33 | 5.00 | 4.67 | 5.00 | 5.00 | 5.00 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.33 | 4.33 | 4.33 | 5.00 | 1.67 | 4.00 |
| linear-triage | 4.67 | 5.00 | 4.00 | 4.33 | 4.33 | 5.00 | 3.67 | 4.67 |
| slack-channel-thread | 4.00 | 4.00 | - | 4.33 | 4.33 | 4.33 | 1.33 | 3.00 |
| thunderbird-unified-inbox | 3.67 | 4.00 | - | 4.33 | 3.67 | 4.33 | 3.67 | 3.33 |
| zulip-stream-topics | 3.67 | 4.00 | - | 4.67 | 4.33 | 4.33 | 2.67 | 3.67 |

## Judges' notes

- **Claude Opus 5.5**: B is a fixed three-pane operator tool, and A is a scrolling page of cards. A's biggest weakness: the page scrolls as one, so Launch/Ask sit at the end of the content and drift as decisions arrive. Its decisions are also age-sorted, which keeps the relay#11 blocker out of the first view. It puts pill chips on every row. B's biggest weakness: the 'Full message' disclosure grows the card and moves its Reply/Dismiss row, and new needs-you tickets are inserted mid-list, shifting the rows below. It also repeats each decision's text in the ticket rows. Both subjects kept the hovered reply control in place between t0 and t30.
- **GPT 6.2 Sol**: B's biggest weakness is that new decisions move Launch during typing and keep it below the work it acts on. A's biggest weakness is that the first decision truncates the requested action in the blocker preview, and opening it still shifts the decisions below. In A's hover pair the visible hovered target is Full message, and the stills show that target and the Reply row holding position while Reply itself appears idle.
- **Claude Sonnet 5.5**: Subject A's biggest weakness is that it is one scrolling page: Launch and Ask live at the page end (the typing stills show it scrolled to the bottom), panel titles scroll away, the blocker is not first, and loading/empty states shift the layout. Subject B's biggest weakness is density at 1440: the right column squeezes Answers and Activity (headings clipped), and the ticket rows truncate their text, so some detail only shows in the detail view. Both open 'Full message' as an in-place disclosure that grows the card. B showed a '1 new' marker rather than shifting the list under the pointer in the hover pair; A's list also held its place in both pairs, though its page layout shifts as items arrive.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | B | A |
| GPT 6.2 Sol | A | B |
| Claude Sonnet 5.5 | B | A |
