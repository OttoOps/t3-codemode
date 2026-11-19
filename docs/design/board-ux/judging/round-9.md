# Judging round 9

Candidate: round 8's build with the judges' evidence applied. Each ticket's PR link moves from after the title to a right-hand column beside the age, so it holds one x on every row (P5). The full-message control becomes a chevron in place of the bare card id, and the hidden-workers link reads "+1 worker" in place of "+1" (N4). Radii drop to 4 px and answer buttons to 28 px (N5). Baseline: the board at `9f8ad45`, `main` before this branch, with #66's filters and search. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together. From this round on, the harness gives each ticket an issue-style thread title (`relay#11: Move tenant settings into their own table`) in place of `Ticket relay#11`, as `wf.launchTicket` does, so the candidate's title column shows a title and not the label twice. The baseline does not show thread titles.

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column, and G5), the blinded stills, the sixteen references, and, for G5, two stills of the live replay blurred to 4 px. The stability recordings were replaced by t0 and t30 stills of the same session.

## Gates

Measured on the judged build with the same scripts as round 8, heavy load excepted: every gate passed. U5: 5 cards visible at 1440, median 128 px. U6: 12 rows at 1440. G2: no clipped text without an ellipsis or count at any view.

The G5 baseline stills are the same `49f9731` captures as round 8's.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 4.00 (4/4/4) |  | yes |
| U13 | 4.67 (4/5/5) | 3.67 (3/4/4) |  | yes |
| P4 | 4.67 (4/5/5) | 3.00 (3/3/3) | 3.67 | yes |
| P5 | 4.00 (4/4/4) | 4.00 (4/4/4) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 3.17 | yes |
| P7 | 4.00 (4/4/4) | 3.33 (3/4/3) |  | yes |
| N3 | 4.00 (4/4/4) | 3.00 (3/3/3) | 4.17 | no |
| N4 | 3.33 (3/4/3) | 3.00 (2/4/3) | 3.67 | no |
| N5 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.00 | yes |
| R1 | 4.00 (4/4/4) | 4.00 (4/4/4) |  | yes |
| R2 | 4.67 (4/5/5) | 3.00 (3/3/3) | 1.83 | yes |
| R3 | 4.33 (4/5/4) | 4.00 (-/4/4) |  | yes |
| R4 | 4.67 (5/5/4) | 3.00 (3/3/3) | 3.67 | yes |
| R5 | 4.00 (4/4/4) | 3.67 (4/4/3) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |
| G5 | 4.33 (4/5/4) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.67 | 3.83 | yes |
| Appearance and polish | 4.25 | 3.42 | yes |
| No slop | 3.89 | 3.11 | yes |
| Reference fit | 4.11 | 3.44 | yes |
| Glanceability | 4.33 | 3.00 | yes |

**Decision rule: not met** (N3, N4 short).

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - In 1440-light the left 'Needs you 7' panel leads with the red 'Blocked: relay#11 worker failed twice...' card, and each later card's bold title is the ask itself ('Reply "merge" to merge #81...', 'Approve the AWS sign-in tab...', 'Merge #203'), with the answer buttons right under it.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png, Needs you leads with relay#11's failed migration but clips the question, then names widgets#41's merge or deploy choice, relay#9's keep or lower budget choice, atlas#60's AWS sign-in approval and ledger#198's merge, with the adjacent group supplying atlas#57's one or two PRs and gizmo#15's stalled run.
- Candidate, Claude Sonnet 5.5: 5 - 1440-light: 'Needs you 7' panel leads with the red 'Blocked' card, then bold imperative cards ('Reply merge to merge #81') with merge/accept buttons; header also shows '7 need you' chip.
- Baseline, Claude Opus 5.5: 4 - In 1440-light the 'Decisions 7' panel leads with the red-railed 'a6 relay#11 blocker' card and each card's body holds the ask ('reply "merge" to merge #81...'), but the bold line is the id ('a1 widgets#41 gate'), so you have to read each body to learn what is wanted, and the header shows no need-you count.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png, Decisions puts relay#11's failed migration first but clips the question, then shows widgets#41's merge or deploy choice, relay#9's keep or lower budget choice, atlas#60's AWS sign-in approval, ledger#198's merge, and atlas#57's one or two PRs, with the Ask answer explaining gizmo#15's stalled run.
- Baseline, Claude Sonnet 5.5: 4 - 1440-light: the left Decisions panel lists the red relay#11 blocker first, then amber gate cards with quoted replies and buttons (merge, accept), readable in seconds, but cards use small body text and opaque ids (a1..a8).

### U13

- Candidate, Claude Opus 5.5: 4 - 1440/1024/390 each give every panel a strong title (warm-tinted 'Needs you 7', 'Tickets' with Open/Done/Settled tabs, 'Activity 46', with Launch/Ask in a bottom bar at 1024 and bottom tabs at 390), but the Answers block between Ask and Activity has no panel title and is clipped at 1280.
- Candidate, GPT 6.2 Sol: 5 - The 1440-light.png still frames the tinted Needs you queue, Tickets and the forms and feed with clear rules, 1024-light.png gives Activity a tab and Launch/Ask a dock, and 390-light.png gives the three lists labeled bottom tabs under a distinct panel title.
- Candidate, Claude Sonnet 5.5: 5 - 1440/1024/390: warm-tinted Needs you panel, Tickets with underlined tabs, and right Launch/Ask/Activity each have large titles and clear boundaries; 390 has Needs you / Tickets / Activity tabs with count.
- Baseline, Claude Opus 5.5: 3 - 1440/1024/390 keep three clear columns (and bottom tabs at 390), but the panel titles are small ('Decisions 7', 'Tickets' in a grey strip) and the Decisions panel is told apart only by a faint grey ground.
- Baseline, GPT 6.2 Sol: 4 - The 1440-light.png still gives Decisions, Tickets and Activity distinct homes, 1024-light.png moves Activity to a Tickets tab and Launch/Ask to a bottom bar, and 390-light.png gives each list a bottom tab, though the desktop panel headings carry little visual weight.
- Baseline, Claude Sonnet 5.5: 4 - 1440 and 1024 show three clearly titled panels (Decisions, Tickets, Launch/Ask/Activity) with a bottom Launch bar and tab bar at 390, but panel titles are light and Activity becomes a tab at 1024.

### P4

- Candidate, Claude Opus 5.5: 4 - In 1440-light the eye goes to the tinted Needs-you column and the red blocked card, then to the amber-diamond 'Needs you 7' group in Tickets, but the filled blue merge/accept/fix buttons on every card are the loudest thing on the page and compete with the ask titles.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png, the tinted Needs you column puts the red blocker and bold requests first, and the central Needs you group makes a second stop before Running and Waiting.
- Candidate, Claude Sonnet 5.5: 5 - 1440-light: large 'Needs you' title with amber count, bold imperative card titles and blue buttons dominate; then the 'Needs you' group of tickets, then Running with quieter text.
- Baseline, Claude Opus 5.5: 3 - In 1440-light the blocker's red rail and the amber 'needs you - aN' lines stand out, but every row's three lines of blue links ('PR #81 open', 'running PR 2/2 · #82 open') and every card's blue 'Full message' link compete at the same weight as the asks.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, the red-edged blocker leads Decisions, but gizmo#12's ordinary waiting row interrupts the tickets marked 'needs you' and the right-hand answers compete for attention.
- Baseline, Claude Sonnet 5.5: 3 - 1440-light: Decisions title is a small bold label and every card has the same weight; ticket rows show 'needs you - a1' in amber but they compete with running/waiting words and PR links.

### P5

- Candidate, Claude Opus 5.5: 4 - Ticket rows in 1440, 1280 and 1024 keep the status mark at left, the PR link and age right-aligned on line 1, and 'PR 1/1' / '+1 worker' right-aligned on line 2, but the PR link column moves with title length (e.g. '#33 draft' vs '#81 open'), and the reason text truncates at different points.
- Candidate, GPT 6.2 Sol: 4 - Across 1440-light.png, 1280-light.png and 1024-light.png, the left status marks and right-aligned PR links and ages hold their positions, while a '+1 worker' link shifts the worker mark left.
- Candidate, Claude Sonnet 5.5: 4 - 1440-light: ticket name, title, PR link and age right-aligned consistently across rows; PR marks and '+1 worker' vary slightly in position.
- Baseline, Claude Opus 5.5: 4 - Ticket rows keep a fixed status-word column, an id column, a right PR column and a right time column at 1440, 1280 and 1024, and Activity is a tidy time/id/text table, but the third sub-line varies (two items, one item or blank) from row to row.
- Baseline, GPT 6.2 Sol: 4 - Across 1440-light.png, 1280-light.png and 1024-light.png, ticket state, ID, primary PR and time keep their columns, while the second worker's PR rides inline after the first worker's status.
- Baseline, Claude Sonnet 5.5: 4 - 1440-light: status word, ticket name, PR link and time sit in consistent columns, though row height varies (atlas#57 has blank space, some rows 3 lines) and PR chips shift horizontally.

### P6

- Candidate, Claude Opus 5.5: 4 - 1440-dark keeps the warm tint on the Needs-you panel, the red border on the blocked card, amber diamonds and visible card and row dividers, so roles read as clearly as in light.
- Candidate, GPT 6.2 Sol: 5 - In 1440-dark.png and 390-dark.png, the warm queue surface, dark card fills and clear rules keep the pane boundaries visible, with amber and red retaining their roles from light mode.
- Candidate, Claude Sonnet 5.5: 4 - 1440-dark and blurred 1920-dark: purpose-built dark with amber chip, red outline on the blocker card, distinct card surface and clear section dividers.
- Baseline, Claude Opus 5.5: 3 - 1440-dark is correct and readable, with the amber/red rails and card borders kept. The Tickets header turns into a near-black strip, though, and the Decisions panel loses the ground that sets it apart in light.
- Baseline, GPT 6.2 Sol: 4 - In 1440-dark.png and 390-dark.png, the blocker and gate edges stay legible and fine rules separate the panels, though the queue cards and ticket rows use nearly the same charcoal.
- Baseline, Claude Sonnet 5.5: 3 - 1440-dark: readable with red/amber borders on cards, but surfaces are near-identical dark grays and the ticket list boundaries are faint; looks like an inverted light theme.

### P7

- Candidate, Claude Opus 5.5: 4 - state-loading keeps all three panels with 'Connecting' and 'Loading decisions/tickets/activity...'; state-empty says 'No open tickets. Paste an issue into Launch to start one.'; state-disconnected shows a red 'Disconnected. Reconnecting; showing data from 08:17.' with the action buttons greyed and data kept, though the loading state drops the Answers area and none of the three offers a retry.
- Candidate, GPT 6.2 Sol: 4 - State-loading.png labels the pending lists, state-empty.png directs users to Launch, and state-disconnected.png timestamps the cached data, though Activity sits directly under Ask when Answers is absent.
- Candidate, Claude Sonnet 5.5: 4 - state-loading shows 'Connecting' and Loading messages without contradictory Answers; state-empty says 'Nothing needs you right now' and 'Paste an issue into Launch'; disconnected dims buttons with a clear red banner and data time.
- Baseline, Claude Opus 5.5: 3 - state-loading keeps the layout and says 'Loading decisions...', but its Answers area already claims 'No answers yet. Type a question in Ask.' state-empty points to Launch, and state-disconnected shows a red 'Disconnected. Reconnecting; showing data from 08:17.' but fades the whole board, making the kept data hard to read, while Launch and Ask stay fully blue.
- Baseline, GPT 6.2 Sol: 4 - State-loading.png labels the pending lists, state-empty.png names Launch and Ask as next actions, and state-disconnected.png preserves the columns and gives the last data time, though the cached rows are heavily dimmed.
- Baseline, Claude Sonnet 5.5: 3 - state-loading shows 'Loading ...' placeholders but Answers says 'No answers yet' mid-load; state-empty points to Launch; state-disconnected is clear with dimmed data and a red banner.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are data, labels or instructions ('Ctrl+Enter launches.', 'Nothing needs you right now.', 'Launched, waiting for the thread to join'), and the empty state points to the Launch control. The only weak strings are a few terse ones like 'No answer yet.'
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png the prominent card text specifies decisions and state-empty.png names Launch, though the widgets#41 card repeats the exact reply choices on its buttons.
- Candidate, Claude Sonnet 5.5: 4 - Rows read as plain language: ticket title plus verb label (Choose/Reply/Report/Blocked) and the ask text; 'not joined yet' and 'Show all' are informative; no internal ids.
- Baseline, Claude Opus 5.5: 3 - Strings like 'needs you - a6', 'gate: scope 46 min', '06:26 note:' and the trailing 'here: acme/widgets#81' on each card are internal ids and padding rather than plain labels.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, short card IDs such as 'a6' and labels such as 'gate: sso' take space before the plain-language request, while state-empty.png directs the user to the named Launch control.
- Baseline, Claude Sonnet 5.5: 3 - Cards carry labels like 'a6', 'gate', 'needs you - a6 blocker 44 min' and 'note:' prefixes that expose internal ids and say little; the empty-state copy is good.

### N4

- Candidate, Claude Opus 5.5: 3 - In 1440-light the Tickets panel's 'Needs you 7' group repeats the same seven asks already shown as cards in the Needs-you panel. Each card's second line also repeats the ticket title shown in Tickets.
- Candidate, GPT 6.2 Sol: 4 - In 1440-light.png, relay#11 appears in both the blocker headline and its metadata, while the amber counts, Review links and expansion chevrons each serve a visible purpose.
- Candidate, Claude Sonnet 5.5: 3 - The 'Needs you 7' group in Tickets repeats all the cards in the left panel, and each ticket has a diamond icon plus verb label plus PR mark, duplicating information.
- Baseline, Claude Opus 5.5: 2 - Several elements carry little: the 'gate' chip that every decision card repeats, the a1-a7 cross-reference ids, the per-card 'Full message' links, and the left status word duplicated by the 'running PR 1/1' sub-line on the same ticket row.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png, the blocker card repeats relay#11 at the start of its message after naming it in the card header, while the remaining visible controls and counts convey actions or state.
- Baseline, Claude Sonnet 5.5: 3 - Every needs-you ticket row repeats the same decision already in the left panel, and each card has a 'Full message' link plus the a-ids that add little.

### N5

- Candidate, Claude Opus 5.5: 4 - The dense two-line ticket rows with small diamond/half-circle marks and verb labels (Choose, Reply, Report) look built for this job, though the Needs-you column of rounded bordered cards with filled blue buttons leans toward a generic card stack.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png, the action-first decision cards, direct PR review links and grouped ticket ledger make this read as a tool for supervising coding threads.
- Candidate, Claude Sonnet 5.5: 4 - Reads as a purpose-built triage tool with a warm queue panel, count chip and sectioned list, similar in spirit to Linear triage; still a few default outlined buttons.
- Baseline, Claude Opus 5.5: 3 - The tabular, text-dense rows with left colour rails look utilitarian rather than like a generic card dashboard, but there is no distinct point of view and the outlined chip buttons and repeated rails feel default.
- Baseline, GPT 6.2 Sol: 4 - In 1440-light.png, the tall decision stack, dense ticket rows and timestamped Activity feed make the page feel like a working queue, though the cards give each request the same basic treatment.
- Baseline, Claude Sonnet 5.5: 3 - Neutral, bordered, utilitarian layout with default-looking outlined buttons; competent but little point of view compared with Linear.

### R1

- Candidate, Claude Opus 5.5: 4 - Across 1440-light, live-hover-t0/t30 and live-typing-t0/t30 the header, filter row, panel titles and Launch/Ask inputs stay put while each list is clipped independently at the viewport edge. Whether every list scrolls on its own cannot be fully confirmed from stills.
- Candidate, GPT 6.2 Sol: 4 - Across live-hover-t0.png and live-hover-t30.png, search, panel titles and the right-side forms hold their positions while the queues and feed update and the hovered Reply stays put.
- Candidate, Claude Sonnet 5.5: 4 - 1440 and 1280: header with counts, tab bar and panel titles stay fixed while lists scroll; Answers scrolls in a clipped area at 1280; 1024 has a fixed bottom input.
- Baseline, Claude Opus 5.5: 4 - In the 1440 and live stills the header, filter row, panel titles and the Launch/Ask inputs stay fixed while the Decisions, Tickets and Activity lists are clipped independently, and Answers has its own fixed-height box.
- Baseline, GPT 6.2 Sol: 4 - Across live-hover-t0.png and live-hover-t30.png, the header, panel titles and inputs stay in the same places as rows change, with Activity updating below its title and the hovered Full message link staying in place.
- Baseline, Claude Sonnet 5.5: 4 - 1440 and 1280: header, panel titles and Launch/Ask stay fixed while lists scroll internally (Answers clips inside its own area); 1024 has a fixed bottom input bar.

### R2

- Candidate, Claude Opus 5.5: 4 - 'Needs you' is its own left-hand queue, and its count shows in the panel title and as an amber '7 need you' badge in the header (live-hover-t30: '10 need you' and a '1 new' pill). The tab title is not visible in the stills.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png, the dedicated Needs you queue has an amber count echoed in the global header, and 390-light.png carries that count into the Needs you tab label.
- Candidate, Claude Sonnet 5.5: 5 - Needs you is a first-class left panel with count chip, and the header shows '7 need you' beside '17 running'; the 390 tab also shows the count.
- Baseline, Claude Opus 5.5: 3 - Decisions is a separate left-hand queue with its count in the panel title ('Decisions 7', and a '1 new' pill in live-hover-t30), but the header shows only '10 running' and no need-you count.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, 'Decisions 7' heads a dedicated queue and 390-light.png repeats the count in the bottom tab, while the global header only counts running work.
- Baseline, Claude Sonnet 5.5: 3 - Decisions is its own first panel with count '7' in the panel title, but the header only shows '10 running' and has no needs-you count.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask sit fixed at the top of the right column, and Ask's results appear directly beneath it under 'Answers' ('answered thread', Settle). At 1024 and 390 the inputs move to a bottom bar, away from the content they act on.
- Candidate, GPT 6.2 Sol: 5 - In live-typing-t0.png and live-typing-t30.png, the focused Launch draft stays beside the updating ticket pane, and Ask stays directly above its running and completed answers.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask are fixed at the top of the right column with Answers directly below; at 1024/390 a fixed bottom bar toggles Launch/Ask.
- Baseline, Claude Opus 5.5: not scored
- Baseline, GPT 6.2 Sol: 4 - In live-typing-t0.png and live-typing-t30.png, the focused Launch draft stays alongside Tickets and Ask sits above Answers, with the Ask placeholder clipped in both frames.
- Baseline, Claude Sonnet 5.5: 4 - Launch and Ask are fixed at top of the right column with Answers below them at 1440; at 1024/390 a fixed bottom bar switches Launch/Ask, detached from Activity.

### R4

- Candidate, Claude Opus 5.5: 5 - Status is carried by grouping ('Needs you 7', 'Running 5', 'Waiting 1') plus one small glyph per row (red x, amber diamond, half-circle) and one coloured verb ('Blocked', 'Choose', 'Reply'). The palette is limited to red, amber and link blue.
- Candidate, GPT 6.2 Sol: 5 - In 1440-light.png, Needs you, Running and Waiting are separate groups, with an amber diamond for requests, a red cross and 'Blocked' for relay#11, and quiet monochrome marks for ordinary work.
- Candidate, Claude Sonnet 5.5: 4 - Position carries status (Needs you / Running / Waiting groups) with a small icon and one word per row; palette limited to amber, red and blue, though every needs row repeats an amber diamond and blue buttons.
- Baseline, Claude Opus 5.5: 3 - The palette is restrained (red, amber, link blue), but every ticket row carries a status word ('running', 'waiting', 'idle', 'failed') plus a second status sub-line, and the needs-you rows are told apart by a rail rather than grouped by position.
- Baseline, GPT 6.2 Sol: 3 - In 1440-light.png, the red blocker leads the Decisions queue, but the ticket list mixes ordinary waiting work among requests and repeats status through edge stripes, marks and words.
- Baseline, Claude Sonnet 5.5: 3 - Each row has a colored status word plus dot (failed, running, waiting, idle), amber gate chips and amber/red card borders and green 'answered'; restrained but status is not structured by position.

### R5

- Candidate, Claude Opus 5.5: 4 - From live-hover-t0 to t30 the hovered card stays put (a '1 new' pill appears instead of shifting it), ticket rows stay two lines with age right-aligned and the Reply/Dismiss actions stay in one spot, but new needs-you rows are inserted mid-list and push the Running group down.
- Candidate, GPT 6.2 Sol: 4 - Across live-hover-t0.png and live-hover-t30.png, ages end at the same right edge and relay#11's hovered Reply stays put when atlas#62 appears, though shortcut actions occupy different lines on the widgets#41 and relay#9 cards.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows are a consistent two-line height with age right-aligned and PR mark in the same column; live-hover t0 to t30 swaps ages and rows without reflow, though left cards vary in height.
- Baseline, Claude Opus 5.5: 4 - Rows keep a fixed height even when a line is empty (the atlas#57 row is as tall as the three-line rows) and Reply/Dismiss sit in the same place on every card, but the right column is a clock time rather than an age, and new needs-you rows are inserted mid-list between live-hover-t0 and t30.
- Baseline, GPT 6.2 Sol: 4 - Between live-hover-t0.png and live-hover-t30.png, the first card's age and actions retain their alignment and ticket rows retain their height as atlas#62 is inserted, though Tickets shows clock times on the right.
- Baseline, Claude Sonnet 5.5: 3 - Ticket rows have time right-aligned but heights vary between 2 and 3 lines and some rows carry empty space (atlas#57); t0 vs t30 rows mostly keep layout.

### R6

- Candidate, Claude Opus 5.5: 3 - In detail-open the relay#11 card expands in place to show the full message and 'Open the thread', growing the card and pushing the cards below it down the column.
- Candidate, GPT 6.2 Sol: 3 - Compared with live-hover-t30.png, detail-open.png keeps relay#11 in the first card and adds its full message and thread link there, shifting the later cards down while the Tickets pane stays beside it.
- Candidate, Claude Sonnet 5.5: 3 - detail-open: the card expands in place with the full message and 'Open the thread', pushing following cards down; an inline disclosure.
- Baseline, Claude Opus 5.5: 3 - In detail-open the a6 card expands in place with a 'Hide message' link, growing the card and pushing every decision card below it down.
- Baseline, GPT 6.2 Sol: 3 - Compared with live-hover-t30.png, detail-open.png reveals relay#11's full message inside the first card and keeps that card at the top, while shifting widgets#41 and the following cards down.
- Baseline, Claude Sonnet 5.5: 3 - detail-open: the full message expands inside the card and pushes the cards below down (Hide message), an inline disclosure that grows the list.

### G5

- Candidate, Claude Opus 5.5: 4 - Blurred, the warm-tinted left column with its blue button blocks is the first landing spot in both 1366-light and 1920-dark, the amber-marked rows at the top of Tickets read next, and Activity is a distinct text column, though the dark tint is subtle.
- Candidate, GPT 6.2 Sol: 5 - In live-1920-dark-blurred.png and live-1366-light-blurred.png, the warm left surface and repeating blue action blocks lead into the amber-marked top ticket group, while form shapes and a stacked feed distinguish the right pane.
- Candidate, Claude Sonnet 5.5: 4 - Blurred stills show a warm tinted Needs you column with amber chip and blue buttons that lead, then a flat ticket list with grouped rows and a separate right column.
- Baseline, Claude Opus 5.5: 3 - Blurred 1366-light and 1920-dark show separable columns with amber rails on the decision cards and on the matching ticket rows, but the Decisions column has no stronger weight or ground than Tickets, so nothing leads.
- Baseline, GPT 6.2 Sol: 3 - In live-1920-dark-blurred.png and live-1366-light-blurred.png, the left card stack, middle rows and right feed remain separable, but the bright blue form buttons command more attention than the queue's thin amber edges.
- Baseline, Claude Sonnet 5.5: 3 - Both blurred stills show Decisions as a tinted left column with amber edge cards and a separable ticket list and right column, but nothing makes Decisions lead over the tickets.

## Reference means

| Reference | P4 | P5 | P6 | N3 | N4 | N5 | R2 | R4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 4.33 | 3.00 | 3.67 | 3.33 | 3.67 | 1.00 | 4.33 |
| claude-code-agent-view | 5.00 | 5.00 | 3.33 | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 |
| datadog-dashboard | 3.00 | 3.67 | 4.00 | 3.67 | 3.33 | 2.67 | 1.00 | 1.67 |
| datadog-oncall-pages | 4.00 | 4.00 | 4.33 | 4.00 | 3.67 | 4.33 | 3.67 | 3.00 |
| github-actions-workflow-graph | 3.33 | 4.00 | 3.00 | 5.00 | 4.67 | 4.00 | 1.00 | 4.33 |
| github-projects-board | 2.67 | 3.00 | 3.00 | 3.00 | 2.33 | 2.67 | 2.00 | 2.00 |
| github-projects-table | 3.00 | 5.00 | 3.00 | 4.00 | 3.33 | 3.33 | 1.33 | 2.00 |
| grafana-alert-rules | 3.00 | 4.00 | 5.00 | 2.33 | 2.00 | 3.33 | 1.33 | 3.67 |
| grafana-alerting-dashboard | 2.67 | 3.33 | 4.67 | 2.33 | 3.00 | 3.00 | 1.67 | 3.67 |
| jira-board | 2.67 | 3.00 | 3.00 | 3.67 | 2.67 | 2.33 | 1.33 | 2.00 |
| linear-inbox | 5.00 | 5.00 | 4.67 | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.67 | 4.67 | 4.33 | 5.00 | 2.00 | 4.00 |
| linear-triage | 5.00 | 5.00 | 4.67 | 4.67 | 4.67 | 5.00 | 3.67 | 5.00 |
| slack-channel-thread | 3.67 | 4.00 | 3.00 | 4.67 | 4.33 | 4.00 | 1.00 | 3.00 |
| thunderbird-unified-inbox | 3.67 | 4.33 | 3.00 | 4.33 | 3.67 | 4.33 | 3.00 | 3.67 |
| zulip-stream-topics | 4.00 | 4.00 | 3.00 | 4.67 | 4.00 | 4.33 | 2.33 | 3.67 |

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | F | W |
| GPT 6.2 Sol | R | H |
| Claude Sonnet 5.5 | X | J |
