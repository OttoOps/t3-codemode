# Judging round 8

Issue #67's first round. Candidate: the readable board, `0c85189` (pre-squash) with its phone, state-mark and worker-toggle fixes. Baseline: the board at `9f8ad45`, `main` before this branch, with #66's filters and search. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together. From this round on, the harness gives each ticket an issue-style thread title (`relay#11: Move tenant settings into their own table`) in place of `Ticket relay#11`, as `wf.launchTicket` does, so the candidate's title column shows a title and not the label twice. The baseline does not show thread titles.

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column, and G5), the blinded stills, the sixteen references, and, for G5, two stills of the live replay blurred to 4 px. The stability recordings were replaced by t0 and t30 stills of the same session.

## Gates

Measured on the judged build: `harness/gates.js` seeded and heavy (27 decisions and 30 open tickets at 1440, 1280 and 1024), `harness/stability.js` with churn, `harness/holds.js`, and `live/capture.js` and `live/glance.js` on the live replay. Every gate passed, with the same evidence as [round 10](round-10.md) apart from U5 (5 cards visible at 1440, median 128 px). G1 ran on this build: all three readers named exactly the four asks (Haiku 11.6 s, Sonnet 4.7 s, Opus 6.6 s).

The G5 baseline stills in this round's packets came from `live/before/`, captured from `main` at `49f9731`, before #66 added the filter bar. [Round 10](round-10.md) re-judges G5 against `9f8ad45`.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 3.67 (4/3/4) |  | yes |
| U13 | 4.33 (4/5/4) | 4.00 (3/5/4) |  | yes |
| P4 | 4.33 (4/5/4) | 3.00 (3/3/3) | 3.83 | yes |
| P5 | 3.33 (4/3/3) | 4.00 (4/4/4) | 4.00 | no |
| P6 | 4.33 (4/5/4) | 3.67 (3/4/4) | 3.00 | yes |
| P7 | 4.33 (5/4/4) | 3.67 (4/4/3) |  | yes |
| N3 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.33 | yes |
| N4 | 3.67 (4/4/3) | 3.33 (3/4/3) | 4.00 | no |
| N5 | 4.00 (4/5/3) | 3.67 (3/4/4) | 4.17 | no |
| R1 | 4.00 (4/4/4) | 4.00 (4/4/4) |  | yes |
| R2 | 5.00 (5/5/5) | 3.00 (3/3/3) | 2.17 | yes |
| R3 | 4.33 (4/5/4) | 4.33 (4/5/4) |  | yes |
| R4 | 4.67 (5/5/4) | 3.00 (3/3/3) | 3.33 | yes |
| R5 | 4.33 (5/4/4) | 4.00 (4/4/4) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |
| G5 | 4.33 (4/5/4) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.50 | 3.83 | yes |
| Appearance and polish | 4.08 | 3.58 | yes |
| No slop | 4.00 | 3.44 | yes |
| Reference fit | 4.22 | 3.56 | yes |
| Glanceability | 4.33 | 3.00 | yes |

**Decision rule: not met** (P5, N4, N5 short).

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - 1440-light: the first panel 'Needs you 7' leads with the red 'Blocked: relay#11...' card, and each later card's bold title is the ask itself ('Reply "merge" to merge #81...', 'Approve the AWS sign-in tab...'), with the answer buttons right under it.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light, Needs you leads with relay#11's migration failure, then shows widgets#41's merge or deploy choice, relay#9's budget choice, atlas#60's AWS sign-in and ledger#198's PR merge, while the Tickets group supplies atlas#57's one-or-two-PR choice and gizmo#15's stuck-worker report.
- Candidate, Claude Sonnet 5.5: 5 - 1440-light: the 'Needs you 7' panel leads with the red Blocked relay#11 card, then widgets#41 merge / merge and deploy, relay#9 accept / fix, each with the wanted action as a button, urgent first.
- Baseline, Claude Opus 5.5: 4 - 1440-light: the left 'Decisions 7' panel leads with the red-railed relay#11 'blocker' card, and each card's body states the ask ('reply "merge" to merge #81...'), but the asks are small lowercase prose under id/kind metadata and the header shows no needs-you count.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light, Decisions leads with relay#11's migration failure and spells out widgets#41's merge or deploy choice, relay#9's budget choice, atlas#60's AWS sign-in, ledger#198's merge and atlas#57's PR split, but gizmo#15's stuck-worker report must be pieced together from its clipped card and the Answers pane.
- Baseline, Claude Sonnet 5.5: 4 - 1440-light: 'Decisions 7' lists the blocker first then gates with action buttons, but the header shows no count and the wording ('here: acme/widgets#81') takes more reading.

### U13

- Candidate, Claude Opus 5.5: 4 - 1440/1024/390: three titled columns (tinted 'Needs you', 'Tickets' with tabs, Launch/Ask/Activity) collapse cleanly into an 'Activity' tab plus a fixed bottom Launch/Ask bar at 1024 and a bottom tab bar at 390, but the Answers block inside the right column has a weak boundary and no strong title.
- Candidate, GPT 6.2 Sol: 5 - 1440-light gives Needs you, Tickets and the command and activity rail distinct homes, 1024-light places Activity in the ticket tabs and commands in the bottom bar, and 390-light gives each list a named bottom tab.
- Candidate, Claude Sonnet 5.5: 4 - 1440 stills show three titled columns (tinted 'Needs you', 'Tickets' with tabs, Launch/Ask/Activity) and 390/1024 use a fixed bottom Launch bar and tabs; section titles are modest.
- Baseline, Claude Opus 5.5: 3 - 1440/1024/390: the three columns and the 390 bottom tabs are clear, but the panel titles ('Decisions 7', 'Tickets', 'Activity 46') are body-sized and the boundaries are thin rules.
- Baseline, GPT 6.2 Sol: 5 - 1440-light places Decisions, Tickets and commands with activity in three labeled columns, 1024-light locates Activity in the tab strip and commands in the footer, and 390-light gives the three lists explicit bottom tabs.
- Baseline, Claude Sonnet 5.5: 4 - Three columns with titled headers (Decisions, Tickets, Launch/Ask/Answers/Activity) and a bottom tab bar at 390; headings are small so boundaries are carried by borders.

### P4

- Candidate, Claude Opus 5.5: 4 - 1440-light: the Needs-you queue, the red blocked card and the grouped 'Needs you 7 / Running 5 / Waiting 1' sections lead the eye, but the large filled blue merge/accept buttons on every card are the loudest marks on the page and compete with the asks.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light, the warm Needs you pane and red blocker draw the eye first, then the amber Needs you ticket group leads into the quieter Running and Waiting groups.
- Candidate, Claude Sonnet 5.5: 4 - 1440-dark: bold 'Needs you' with count badge and large blue action buttons dominate, the Tickets list is grouped under Needs you / Running / Waiting; secondary text still competes a little in the Answers pane.
- Baseline, Claude Opus 5.5: 3 - 1440-light: the red blocker rail and the orange 'needs you - aN' text stand out, but the panel headings are no heavier than the content, and every ticket row repeats status words and PR lines at the same weight.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light, the red blocker and amber card edges identify Decisions, but waiting gizmo#12 interrupts the needs-you tickets and the blue Launch and Ask buttons compete for attention.
- Baseline, Claude Sonnet 5.5: 3 - 1440-light: Decisions cards have colored left borders and tags, but the heading is small, tickets are one flat list with 'running' on most rows, and secondary text competes.

### P5

- Candidate, Claude Opus 5.5: 4 - 1440/1280/1024: every ticket row keeps the status glyph at left, age at top-right and the PR mark ('PR 1/1') right-aligned on line two, but the inline '#81 open' PR link drifts with title length.
- Candidate, GPT 6.2 Sol: 3 - Across 1440-light, 1280-light and 1024-light, ticket marks and ages line up, while PR links follow variable-length titles and Review #81 moves to a different line at 1280.
- Candidate, Claude Sonnet 5.5: 3 - 1440-light: age and 'PR 1/1' are right-aligned, but '#81 open' trails the title at varying x and '+1' appears on some rows only, so PR info does not sit in one column; card heights in the left panel vary.
- Baseline, Claude Opus 5.5: 4 - 1440/1280/1024: a fixed status-word column, a right-aligned 'PR #81 open' column and a clock column hold on every row, though the secondary PR lines on line three vary in content and length.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light, 1280-light and 1024-light, ticket status, issue IDs, primary PR links and timestamps retain aligned columns, while individual PR entries run inline below the issue.
- Baseline, Claude Sonnet 5.5: 4 - Status word sits in the left column, PR 'PR #81 open' right-aligned at one x, and time at the far right on every ticket row across 1440 and 1280.

### P6

- Candidate, Claude Opus 5.5: 4 - 1440-dark: card borders, the red-outlined blocked card, the warm tinted Needs-you panel and the orange diamonds all hold, so roles stay as clear as in light.
- Candidate, GPT 6.2 Sol: 5 - In 1440-dark and 390-dark, the warm queue background, slate cards, visible dividers and legible red and amber marks preserve the panel and status distinctions visible in light mode.
- Candidate, Claude Sonnet 5.5: 4 - 1440-dark and 390-dark keep the warm-tinted queue panel, red blocked border, and row dividers legible; blue buttons and orange marks hold contrast.
- Baseline, Claude Opus 5.5: 3 - 1440-dark: correct and readable, with colored rails visible, but the card and panel surfaces sit close to the background, and the near-black Tickets header bar is inconsistent with the other panel headers.
- Baseline, GPT 6.2 Sol: 4 - 1440-dark keeps the salmon blocker, amber gate marks and blue links legible, though queue cards and ticket rows sit on very similar charcoal surfaces.
- Baseline, Claude Sonnet 5.5: 4 - 1440-dark keeps colored left borders, outlined buttons and blue PR links readable with clear panel separation.

### P7

- Candidate, Claude Opus 5.5: 5 - state-loading keeps all three panels with 'Connecting' and per-panel 'Loading decisions...'; state-empty says 'No open tickets. Paste an issue into Launch to start one.'; state-disconnected shows a red 'Disconnected. Reconnecting; showing data from 08:09.' and dims the action buttons while the data stays readable.
- Candidate, GPT 6.2 Sol: 4 - state-loading labels each loading list, state-empty points to Launch, and state-disconnected timestamps cached data and mutes card actions, though Activity shifts upward when Answers is absent.
- Candidate, Claude Sonnet 5.5: 4 - state-loading says 'Loading decisions...' with stable layout; state-empty says 'Nothing needs you right now' and 'Paste an issue into Launch'; state-disconnected shows a red banner with the data time and dims the action buttons.
- Baseline, Claude Opus 5.5: 4 - state-loading keeps the layout with 'Loading decisions...' but shows the Answers empty copy ('No answers yet. Type a question in Ask.') mid-load; state-empty points to Launch; state-disconnected shows a red banner and fades the whole board, which hurts legibility.
- Baseline, GPT 6.2 Sol: 4 - state-loading and state-empty preserve every panel and name Ask and Launch as next actions, while state-disconnected timestamps cached data and fades decision actions but leaves Launch and Ask looking active.
- Baseline, Claude Sonnet 5.5: 3 - state-loading shows 'Loading decisions...' but also 'No answers yet. Type a question in Ask.'; state-empty says 'Nothing needs you right now' and 'Paste an issue into Launch'; disconnected dims cards with a red banner.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are data or instructions ('Ctrl+Enter launches.', 'Launched, waiting for the thread to join'), and the empty state points to Launch; the bare 'a1'..'a6' ids on the cards say little.
- Candidate, GPT 6.2 Sol: 5 - 1440-light uses labels such as Launch tickets and instructions such as Ctrl+Enter launches, and state-empty names Launch as the control for starting a ticket.
- Candidate, Claude Sonnet 5.5: 4 - Strings are data or instructions ('Ctrl+Enter launches.', 'Reply "merge" Merge #203'); a few like 'checked just now' and 'Show all' say little.
- Baseline, Claude Opus 5.5: 3 - Rows repeat implementation-flavored labels ('needs you - a6', 'gate: merge 58 min', '06:18 note:'), and the cards append 'here: acme/widgets#81'; the copy is plain otherwise.
- Baseline, GPT 6.2 Sol: 4 - state-empty directs users to Launch and Ask by name, while 1440-light includes internal-looking strings such as 'needs you - a1' and 'gate: sso' in Tickets.
- Baseline, Claude Sonnet 5.5: 3 - Rows read 'needs you - a6 gate: merge 58 min' and '06:18 note: ...', cards add 'here: acme/widgets#81'; labels reflect internal ids and fields.

### N4

- Candidate, Claude Opus 5.5: 4 - Nearly everything carries data; the unexplained 'a6'/'a1' ids and the '+1' next to some PR marks are the only elements whose loss would go unnoticed.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light, the bare a6 and a1 tokens could go because the queue and ticket rows already connect those cards through relay#11 and widgets#41.
- Candidate, Claude Sonnet 5.5: 3 - The Needs you panel and the 'Needs you 7' group in Tickets list the same seven items twice, and the a1..a6 ids and a diamond on every needs-you row add little.
- Baseline, Claude Opus 5.5: 3 - 1440-light: the per-row 'needs you - aN' line duplicates the Decisions panel, and a 'running' status word plus a 'running PR 1/1' line on most rows repeat each other; 'Full message' appears on every card.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light, the trailing acme/widgets#81 reference in the widgets#41 decision preview could go because the same sentence already names PR #81 and the ticket row links it.
- Baseline, Claude Sonnet 5.5: 3 - 'running' is repeated as a word on most ticket rows, 'Full message' repeats on every card and a1..a8 ids show on both panels.

### N5

- Candidate, Claude Opus 5.5: 4 - 1440-light: the grouped ticket list with glyphs, kind words ('Choose', 'Reply', 'Report') and right-aligned ages reads as a purpose-built tool, though the rounded Needs-you cards with big blue buttons lean toward generic.
- Candidate, GPT 6.2 Sol: 5 - 1440-light pairs an actionable decision inbox with grouped ticket rows and a compact command rail, including controls that send the exact replies requested by agents.
- Candidate, Claude Sonnet 5.5: 3 - Rounded cards, a pill count badge and bright blue buttons are fairly generic, though the sectioned ticket list and text-first rows feel more tool-like.
- Baseline, Claude Opus 5.5: 3 - 1440-light: a dense, neutral, utilitarian layout of colored left rails, outlined chip buttons and text everywhere; competent, but without a clear point of view.
- Baseline, GPT 6.2 Sol: 4 - 1440-light uses a purpose-built decision inbox and a dense ticket table with per-PR status, while the answer area and launch form use standard form styling.
- Baseline, Claude Sonnet 5.5: 4 - Dense, text-first rows with an outlined-button style and no card shadows look like an ops tool, though the colored left borders and tag dots are plentiful.

### R1

- Candidate, Claude Opus 5.5: 4 - 1440 and the live stills: the header and filter bar stay put and each column clips its own list at the bottom (Needs you, Tickets, Activity) with the panel titles and the Launch/Ask inputs at their tops; true scroll behavior is not visible in stills.
- Candidate, GPT 6.2 Sol: 4 - Across live-hover-t0 and live-hover-t30, the header, panel titles and inputs stay at the same coordinates as Tickets and Activity update, with scrolling behavior beyond what the stills show.
- Candidate, Claude Sonnet 5.5: 4 - All stills keep header, filter bar and panel titles in place while the three columns scroll independently; the Answers pane at 1280 is clipped inside its box.
- Baseline, Claude Opus 5.5: 4 - 1440 and the live stills: the header and filter bar stay fixed, and each column clips independently with its title (Decisions, Tickets tabs, Launch/Ask) at the top; the Activity header holds at a fixed y below the Answers region.
- Baseline, GPT 6.2 Sol: 4 - live-hover-t0 and live-hover-t30 keep the header, panel titles and input rail in the same places as the lists update, with scrolling behavior beyond what the stills show.
- Baseline, Claude Sonnet 5.5: 4 - Header and filter bar stay fixed and each column scrolls; at 1280 the Answers pane is cut off mid-text.

### R2

- Candidate, Claude Opus 5.5: 5 - 1440-light: 'Needs you' is its own first panel with a '7' badge, and the header shows '7 need you' beside '17 running'; the count updates to 9/10 in the live stills.
- Candidate, GPT 6.2 Sol: 5 - 1440-light shows a first-class Needs you queue whose count appears again in the global header, and 390-light also puts the count in the Needs you tab label.
- Candidate, Claude Sonnet 5.5: 5 - 'Needs you 7' is a first-class left queue and the header shows an orange '7 need you' beside '17 running'; the count updates to 10 in the live stills.
- Baseline, Claude Opus 5.5: 3 - 1440-light: Decisions is a separate first panel with a count, but the header shows only '10 running', with no needs-you count.
- Baseline, GPT 6.2 Sol: 3 - 1440-light gives Decisions a dedicated queue and local count of seven, while the global header shows only the running count and 390-light puts the seven in the bottom tab.
- Baseline, Claude Sonnet 5.5: 3 - Decisions is a separate left panel with its count in its title, but the header only says '10 running' and tickets needing the user are mixed into the full list.

### R3

- Candidate, Claude Opus 5.5: 4 - 1440: Launch and Ask sit fixed at the top of the right column with Answers directly beneath Ask; at 1024 and 390 they become a fixed bottom bar, a bit detached from Answers.
- Candidate, GPT 6.2 Sol: 5 - live-typing-t0 and live-typing-t30 keep acme/widgets#52 focused at the top of the rail above Activity, with Ask directly above its Answers.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask sit fixed at the top of the right column with Answers directly under them (1440), and a fixed bottom bar at 1024/390; typing stills show the field keeping focus while lists update.
- Baseline, Claude Opus 5.5: 4 - 1440: the Launch textarea and Ask input are fixed at the top of the right column with Answers directly below; at 1024 and 390 they move to a fixed bottom bar; typed text survives updates (live-typing t0/t30).
- Baseline, GPT 6.2 Sol: 5 - live-typing-t0 and live-typing-t30 show the focused Launch input holding acme/widgets#52 at the top of the rail, with Activity in that rail and Ask directly above Answers.
- Baseline, Claude Sonnet 5.5: 4 - Launch and Ask are fixed in the right column with Answers below; at 1024 and 390 a bottom bar holds Launch/Ask.

### R4

- Candidate, Claude Opus 5.5: 5 - 1440-light: status comes from the queue and the group headers (Needs you / Running / Waiting), then one small glyph (X, diamond, half-circle) and one colored word ('Blocked', 'Choose'), in a restrained red/amber/blue palette.
- Candidate, GPT 6.2 Sol: 5 - 1440-light puts human work in Needs you and groups Tickets by Needs you, Running and Waiting, using small marks and short labels for the remaining status cues.
- Candidate, Claude Sonnet 5.5: 4 - Position carries status (Needs you / Running / Waiting groups) with a small diamond or half-circle mark and a word, but orange, red, blue and green all appear and the PR glyphs add more marks.
- Baseline, Claude Opus 5.5: 3 - 1440-light: the palette is restrained, but tickets are not grouped by status (failed, running, waiting and idle rows interleave), so every row needs a status word, a colored rail and a PR status dot.
- Baseline, GPT 6.2 Sol: 3 - 1440-light uses small red, amber and gray marks with words and a separate Decisions queue, but the middle list mixes waiting gizmo#12 with needs-you rows and repeats status on individual PR entries.
- Baseline, Claude Sonnet 5.5: 3 - Every row carries a status word plus colored dot, 'needs you' text, colored left border and PR marks in red, orange, green, blue; restrained but not position-driven.

### R5

- Candidate, Claude Opus 5.5: 5 - live-hover t0 vs t30: ticket rows stay two lines with ages right-aligned and PR marks in place; the hovered relay#11 card does not move, and new items show as a '1 new' pill rather than shifting the queue.
- Candidate, GPT 6.2 Sol: 4 - Between live-hover-t0 and live-hover-t30 the top card's Reply stays under the pointer and ticket ages stay right-aligned as rows are inserted, while decision card heights vary with their controls.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows are a consistent two-line height with age right-aligned and PR on the right; hover/typing t0 vs t30 stills show text swapping without row resize, though cards in the queue vary in height.
- Baseline, Claude Opus 5.5: 4 - live-hover t0 vs t30: ticket rows hold a fixed three-line height and the hovered relay#11 card stays put behind a '1 new' pill, but the right column shows clock times (07:11 -> 08:10) rather than ages, and new rows insert mid-list.
- Baseline, GPT 6.2 Sol: 4 - Across live-hover-t0 and live-hover-t30, ticket rows retain their height and right-edge timestamps when atlas#62 enters the list, while decision cards use different heights for their messages and actions.
- Baseline, Claude Sonnet 5.5: 4 - Ticket rows share a fixed height with time right-aligned and PR in the same column; t0/t30 stills show time and note text changing in place.

### R6

- Candidate, Claude Opus 5.5: 3 - detail-open: the relay#11 card expands in place to show the full message and an 'Open the thread' link, pushing the cards below down and reordering the card's own meta/actions rows.
- Candidate, GPT 6.2 Sol: 3 - In detail-open, the full relay#11 message and thread link open inside its card and push widgets#41 lower than in live-hover-t30.
- Candidate, Claude Sonnet 5.5: 3 - detail-open expands the relay#11 card in place and shows the full message and an 'Open the thread' link, but the card grows and pushes the cards below it down.
- Baseline, Claude Opus 5.5: 3 - detail-open: 'Full message' expands the relay#11 card inline ('Hide message'), growing it and pushing the decisions below down.
- Baseline, GPT 6.2 Sol: 3 - In detail-open, relay#11 expands inside the Decisions pane and moves the widgets#41 card down from its position in live-hover-t30.
- Baseline, Claude Sonnet 5.5: 3 - detail-open expands the relay#11 card in place with the full message below its actions, growing the card and shifting the cards below.

### G5

- Candidate, Claude Opus 5.5: 4 - 1920-dark and 1366-light blurred: the tinted Needs-you column with its blue button blocks lands first, the ticket rows with amber marks second and the feed third, though the dark tint on Needs you is faint.
- Candidate, GPT 6.2 Sol: 5 - In both live-1920-dark-blurred and live-1366-light-blurred, the tinted left pane and stacked action buttons lead into the amber-headed ticket rows, with form shapes and a separate feed visible at right.
- Candidate, Claude Sonnet 5.5: 4 - Blurred 1366 and 1920: the warm-tinted left panel with blue buttons reads first, the ticket list is a separate neutral surface with section bands, and the right column is a distinct block.
- Baseline, Claude Opus 5.5: 3 - 1920-dark and 1366-light blurred: the panels are separable by their amber rails and the gray Answers block, but Decisions and the ticket rows carry the same rail weight, and the blue Launch/Ask buttons draw the eye as much as Decisions does.
- Baseline, GPT 6.2 Sol: 3 - In live-1920-dark-blurred and live-1366-light-blurred, edge stripes separate the decision cards from the ticket rows, but the bright buttons at right and colored metadata in the center draw the eye away from Decisions.
- Baseline, Claude Sonnet 5.5: 3 - Blurred 1366/1920: Decisions cards are separable by orange borders but the left panel and tickets list share similar weight and the large gray Answers block draws attention.

## Reference means

| Reference | P4 | P5 | P6 | N3 | N4 | N5 | R2 | R4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 4.33 | 3.00 | 3.67 | 4.00 | 4.00 | 1.33 | 4.00 |
| claude-code-agent-view | 5.00 | 4.67 | 3.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 |
| datadog-dashboard | 3.67 | 4.00 | 4.00 | 3.67 | 3.33 | 2.67 | 1.00 | 1.67 |
| datadog-oncall-pages | 4.33 | 4.33 | 4.00 | 4.33 | 4.00 | 4.33 | 3.33 | 3.33 |
| github-actions-workflow-graph | 4.00 | 4.00 | 3.00 | 4.67 | 4.67 | 4.00 | 1.33 | 4.33 |
| github-projects-board | 3.00 | 3.33 | 3.00 | 3.00 | 3.00 | 2.67 | 2.00 | 2.33 |
| github-projects-table | 3.33 | 5.00 | 3.00 | 4.33 | 3.33 | 3.33 | 1.67 | 2.00 |
| grafana-alert-rules | 3.33 | 4.00 | 5.00 | 2.33 | 2.67 | 3.67 | 2.00 | 3.67 |
| grafana-alerting-dashboard | 3.00 | 3.33 | 4.67 | 2.33 | 3.00 | 3.33 | 2.33 | 3.33 |
| jira-board | 2.67 | 3.33 | 3.00 | 3.67 | 3.00 | 2.33 | 1.67 | 1.67 |
| linear-inbox | 4.67 | 5.00 | 4.33 | 5.00 | 5.00 | 5.00 | 4.00 | 4.67 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.67 | 4.67 | 4.33 | 5.00 | 2.33 | 4.00 |
| linear-triage | 5.00 | 5.00 | 4.67 | 5.00 | 5.00 | 5.00 | 3.67 | 4.67 |
| slack-channel-thread | 4.00 | 4.00 | 3.00 | 4.33 | 4.33 | 4.33 | 1.67 | 3.00 |
| thunderbird-unified-inbox | 3.67 | 4.00 | 3.00 | 4.00 | 3.67 | 4.33 | 3.67 | 3.33 |
| zulip-stream-topics | 4.00 | 4.00 | 3.00 | 4.67 | 4.33 | 4.33 | 2.33 | 3.33 |

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | D | W |
| GPT 6.2 Sol | R | P |
| Claude Sonnet 5.5 | W | P |
