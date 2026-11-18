# Judging round 3

Candidate: the fixed panel board at `7b0c8c2 (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-3-measurements.json`](round-3-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | pass | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded and heavy (30 open tickets) |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
| U3 | pass | focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept through a second-context answer and a stream cut; focused thread link, worker PR link, ask Settle, settled Restore stayed the same connected node for 30 s; hovered card option and ticket Settle never moved |
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 2 new, tickets: 1 new, feed: 15 new; tab switch restored scrollTop 78 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 12 open rows visible at 1440 (12 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | pass | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; a 250 px viewport switches the composer to sheet mode (height and restore on close not measured) |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 02:25."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | pass | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], no off-scale spacing |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 46 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F3 | pass | Ask error 1x (alert), launch result 1x (both issues failed, so a mixed result was not exercised), card error 1x (alert), 0 announcements matching the sampled Activity patterns |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1 is covered by the gate scripts and tests above (a launch whose issues both fail, a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests. The scripts do not exercise the hidden-tab notification: it calls the same `Notification` code as before, and permission now comes from the header's Notify me control instead of the first click.

The heavy board reached 30 open tickets but held at 12 decisions: in this harness a newer gate from a ticket supersedes its older gate card, so the decision count plateaus below the rubric's 15. The 15-decision load was not measured in this round.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 2.67 (3/2/3) |  | yes |
| U13 | 4.33 (4/5/4) | 2.00 (2/2/2) |  | yes |
| P4 | 4.00 (4/4/4) | 2.67 (2/3/3) | 3.83 | yes |
| P5 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.00 | yes |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.67 | no |
| P7 | 4.00 (4/4/4) | 2.00 (2/2/2) |  | yes |
| N3 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.33 | no |
| N4 | 3.67 (4/4/3) | 2.00 (2/2/2) | 3.67 | yes |
| N5 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.17 | yes |
| R1 | 5.00 (5/5/5) | 1.33 (1/2/1) |  | yes |
| R2 | 4.33 (4/5/4) | 4.00 (4/4/4) | 2.33 | yes |
| R3 | 4.00 (4/4/4) | 1.00 (1/1/1) |  | yes |
| R4 | 4.00 (4/4/4) | 2.00 (2/2/2) | 3.50 | yes |
| R5 | 4.33 (4/5/4) | 2.00 (2/2/2) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.50 | 2.33 | yes |
| Appearance and polish | 4.08 | 2.50 | yes |
| No slop | 4.00 | 2.00 | yes |
| Reference fit | 4.11 | 2.22 | yes |

**Decision rule: not met (P6, N3).**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - The left Decisions 7 panel opens with the red-barred relay#11 blocker, then widgets#41 merge, relay#9 accept/fix, atlas#60 approve AWS sign-in, ledger#198 merge, atlas#57 one/split, gizmo#15 report, each with ticket, kind and reply buttons; the ticket list repeats this as 'needs you - aN' rows sorted to the top.
- Candidate, GPT 6.2 Sol: 4 - The first pane leads with relay#11's failed migration, then shows widgets#41's merge choice, relay#9's budget choice, atlas#60's AWS approval, ledger#198's merge, atlas#57's one-PR-or-two choice and gizmo#15's stalled run at the fold, with the exact blocker decision cut off.
- Candidate, Claude Sonnet 5.5: 5 - First panel (Decisions 7) lists the red relay#11 blocker first, then widgets#41 merge, relay#9 accept/fix, atlas#60 SSO tab, ledger#198 merge, atlas#57 one/split, gizmo#15 report, each with its quick-reply buttons and age; a few bodies are ellipsized.
- Baseline, Claude Opus 5.5: 3 - The Decisions 7 column is first and each card names ticket, gate and buttons, but only four cards fit at 1440, the relay#11 blocker is not first, and the user must scroll to see the rest.
- Baseline, GPT 6.2 Sol: 2 - The first panel shows widgets#41's merge choice, relay#9's budget choice, atlas#60's AWS approval and ledger#198's merge, while three decisions including the migration blocker sit outside the 1440 viewport.
- Baseline, Claude Sonnet 5.5: 3 - Decisions column starts with widgets#41 merge, relay#9, atlas#60, ledger#198 with their buttons, but there is no urgency ordering (the blocker is not first at 1440), the header repeats counts, and 'needs you' is mixed with pills in the ticket list.

### U13

- Candidate, Claude Opus 5.5: 4 - At 1440 three bordered columns (Decisions, Tickets with Open/Done/Settled tabs, Launch/Ask/Answers/Activity) with bold titles; 1024 folds Activity into a tab and Launch/Ask into a fixed bottom bar, 390 uses a bottom tab bar; the right column's Answers block is weakly bounded and its last question is clipped by Activity.
- Candidate, GPT 6.2 Sol: 5 - At 1440 the three columns have explicit boundaries, at 1024 Activity joins the Tickets tabs and Launch/Ask docks below, and at 390 bottom tabs give Decisions, Tickets and Activity a predictable home.
- Candidate, Claude Sonnet 5.5: 4 - Three fixed panels (Decisions left, Tickets centre, Launch/Answers/Activity right) at 1440; at 1024 Launch/Ask moves to a bottom bar and Activity becomes a tab, at 390 a bottom tab bar (Decisions/Tickets/Activity); panel boundaries are only thin rules and a faint background tint.
- Baseline, Claude Opus 5.5: 2 - Sections are floating rounded cards in a centered page; Launch, Ask, Activity and Settled sit below the content and their position depends on how many decisions and tickets exist (Launch moved down ~165 px between typing t0 and t30); section titles are small grey labels.
- Baseline, GPT 6.2 Sol: 2 - Decisions and Tickets have recognizable headings at 1440 and 1024, while the phone still shows only the Decisions stack and the typing pair places Activity and the forms after content of variable length.
- Baseline, Claude Sonnet 5.5: 2 - Page is two columns of cards over a free-flowing page with Activity and Settled as collapsed bars and Launch/Ask at the very bottom; positions depend on content (Launch moved from y=557 to y=723 between t0 and t30 when a decision was added).

### P4

- Candidate, Claude Opus 5.5: 4 - Blocker first with a red bar, gate cards with amber bars, ticket rows that need the user carry amber 'needs you' text and a left bar while plain running rows are quieter; the blue 'Full message' links on every card compete slightly.
- Candidate, GPT 6.2 Sol: 4 - The red relay#11 blocker leads the Decisions queue and the central list uses bold 'needs you' labels before routine notes, though gizmo#12 interrupts that priority group.
- Candidate, Claude Sonnet 5.5: 4 - Decisions column has red/amber left stripes, bold ids and bordered quick-reply buttons so the eye lands there, then ticket rows with bold amber 'needs you - a6' lines; plain notes rows recede, though the whole ticket list is fairly dense.
- Baseline, Claude Opus 5.5: 2 - Small grey 'Decisions 7'/'Tickets 15 open' headings are weaker than the large blue primary buttons and full-width reply inputs; ticket pills (running, gate: merge, idle) give every row equal weight.
- Baseline, GPT 6.2 Sol: 3 - Decision cards and blue reply buttons draw attention first, while routine ticket notes and bright running pills compete with the tickets that carry gates.
- Baseline, Claude Sonnet 5.5: 3 - Blue filled primary buttons make decisions prominent, but ticket rows are crowded with blue/amber/grey pills, tag chips and blue links that compete with them; panel titles are small grey caps.

### P5

- Candidate, Claude Opus 5.5: 4 - Ticket rows keep status word in a fixed left column, title, PR link and right-aligned time in the same columns at 1440, 1280 and 1024; decision card heights still vary with/without reply buttons.
- Candidate, GPT 6.2 Sol: 4 - Status, issue, top-level PR and timestamp retain their columns at 1440, 1280 and 1024, while worker status and links share an inline line under each issue.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows keep status word left, name, PR link and age right-aligned in the same columns at 1440/1280/1024; only the sub-PR line text varies, and decision cards differ in height depending on whether they have buttons.
- Baseline, Claude Opus 5.5: 2 - At 1440 PR/thread links are right-aligned, but status pills float after variable-width titles, and at 1024 links wrap to new lines (ledger#198 PR #204 on its own line, PR #81 under the title), so rows differ.
- Baseline, GPT 6.2 Sol: 2 - Status pills follow issue names of different lengths and the PR and thread links for widgets#41 and ledger#198 drop to a second line at 1024.
- Baseline, Claude Sonnet 5.5: 2 - PR and thread links shift left/right per row (PR #81 vs #203 vs thread-only), sub-PR rows repeat the ticket name and at 1024 ledger#198's second PR link wraps to its own line; note text wraps to 1-3 lines per row.

### P6

- Candidate, Claude Opus 5.5: 4 - Dark theme keeps panel borders, card outlines and the red/amber/blue roles (lightened for contrast) as legible as light; primary buttons shift to a softer blue but stay clear.
- Candidate, GPT 6.2 Sol: 5 - Thin dividers keep the queue, tickets and right rail distinct, and the same restrained red, amber and blue roles remain clear across desktop and phone.
- Candidate, Claude Sonnet 5.5: 4 - Dark theme is a real redesign: dark panels with visible borders, pastel red/amber stripes and status words, light-periwinkle Launch button, links stay legible; phone dark matches light structure.
- Baseline, Claude Opus 5.5: 3 - Dark mode is correct and readable with card boundaries and amber/blue pills intact; the light-blue primary buttons with dark text read as a different style from light mode.
- Baseline, GPT 6.2 Sol: 4 - Blue links, amber gates and the reply field remain legible on desktop and phone, though dark card edges and muted metadata recede against the canvas.
- Baseline, Claude Sonnet 5.5: 3 - Dark theme is correct and readable with cards on a near-black ground, but pills and card edges are low-contrast and it is mostly a colour swap of the light page.

### P7

- Candidate, Claude Opus 5.5: 4 - Loading keeps all three panels with 'Loading decisions...'/'Loading tickets...'/'Loading activity...' and header 'Connecting'; empty says 'Nothing needs you right now.' and 'Paste an issue into Launch to start one.'; disconnected shows a red 'Disconnected. Reconnecting; showing data from 02:21.' and dims stale content, though Launch/Ask buttons stay fully enabled.
- Candidate, GPT 6.2 Sol: 4 - Loading labels, an empty-state prompt naming Launch, and a reconnecting message with the last data time preserve the panes, though Answers says 'No questions yet' while the board is still loading.
- Candidate, Claude Sonnet 5.5: 4 - Empty: 'Nothing needs you right now.' / 'No open tickets. Paste an issue into Launch to start one.' with the layout intact; Loading: 'Connecting' plus 'Loading decisions...' in the same panels; Disconnected: red banner 'Disconnected. Reconnecting; showing data from 02:21.' and stale data dimmed with reply buttons greyed.
- Baseline, Claude Opus 5.5: 2 - Loading shows empty white blocks with no text and lowercase 'connecting'; empty copy uses positional 'Launch one below'; disconnected adds a pink 'Disconnected from t3-codemode. Retrying.' banner that shifts the page and does not say data is stale.
- Baseline, GPT 6.2 Sol: 2 - The loading view leaves two unlabeled blank blocks under a generic connecting status, the empty state directs users 'below', and a retry banner shifts the populated board down.
- Baseline, Claude Sonnet 5.5: 2 - Empty copy is explanatory ('A short-lived thread reads the board...', 'Launch one below') and the layout collapses; loading shows blank white boxes with only 'connecting'; disconnected adds a red banner that pushes everything down about 35px.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are data or labels ('needs you - a6', 'gate: merge 58 min', 'One issue per line. Ctrl+Enter launches.'); empty states point to the Launch control by name; only the truncated Answers headings say little.
- Candidate, GPT 6.2 Sol: 4 - The empty ticket state names Launch and the helper gives a submission shortcut, while the Activity empty state adds an explanation of which events appear there.
- Candidate, Claude Sonnet 5.5: 4 - Strings are labels or instructions ('One issue per line. Ctrl+Enter launches.', 'Paste an issue into Launch'); only 'Working on it...' and 'No questions yet.' say little.
- Baseline, Claude Opus 5.5: 2 - 'A short-lived thread reads the board and answers in a few lines.' explains implementation, 'Full message m43' exposes an internal id, ticket notes show raw '**ACTION NEEDED:**' markdown, and empty states say 'below'.
- Baseline, GPT 6.2 Sol: 2 - The Ask helper explains that a 'short-lived thread' reads the board, the empty state says 'Launch one below', and ticket excerpts include raw 'ACTION NEEDED' markup.
- Baseline, Claude Sonnet 5.5: 2 - Strings explain the implementation or point by position: 'A short-lived thread reads the board and answers in a few lines.', 'Gates, blockers and reports from ticket threads land here...', 'Launch one below', 'not joined yet', 'Full message m43'.

### N4

- Candidate, Claude Opus 5.5: 4 - Nearly everything carries information; the colored left bar on ticket rows duplicates the colored 'needs you' text, and the 'PR 1/1' sub-line adds little on single-PR tickets.
- Candidate, GPT 6.2 Sol: 4 - The decision ID connects queue cards to 'needs you' ticket rows, while the tiny unlabeled artifact before the connection dot adds no readable information.
- Candidate, Claude Sonnet 5.5: 3 - Nearly everything works, but 'needs you - a6' repeats information from the Decisions panel on each ticket row, the bare '?' header button is unexplained, and ids like a6 add little.
- Baseline, Claude Opus 5.5: 2 - Removable without loss: 'Full message mNN' ids, the a1-style badge next to the ticket link, the per-PR status pill that repeats the ticket pill on single-PR tickets, and duplicate 'thread' links on ticket and PR rows.
- Baseline, GPT 6.2 Sol: 2 - The Ask implementation sentence, the message IDs appended to Full message, and the repeated gate excerpt on widgets#41 could go without losing the action or its context.
- Baseline, Claude Sonnet 5.5: 2 - Header duplicates the panel counts (7 decisions / 15 tickets / 10 running), a status pill on every ticket and PR, repeated ticket-name prefix on PR sub-rows, 'm43' message ids, and a 'thread' link on every row carry little.

### N5

- Candidate, Claude Opus 5.5: 4 - Dense three-pane mail-client layout, hairline rules, no shadows or pill soup, status as a mark plus word; reads as a purpose-built ops tool close to Thunderbird/Linear.
- Candidate, GPT 6.2 Sol: 5 - The blocker-first queue, terse ticket rows with worker PR links, and live activity rail read as a console for triaging agent work.
- Candidate, Claude Sonnet 5.5: 4 - Dense, ruled three-pane tool with left-edge urgency stripes, status words and a ticket list with an age column; reads as a purpose-built operator console rather than a card grid.
- Baseline, Claude Opus 5.5: 2 - Centered card grid with rounded white cards and soft shadows, pill badges on every ticket and PR, and big filled buttons; reads as a generic dashboard template.
- Baseline, GPT 6.2 Sol: 2 - Rounded shadowed cards, badge-heavy ticket rows and repeated primary buttons give the board a familiar dashboard look despite job-specific PR and thread links.
- Baseline, Claude Sonnet 5.5: 2 - White rounded cards with soft shadows, pill badges for running/idle/waiting/gate/done, and a collapsed-bar stack: reads as a generic AI dashboard.

### R1

- Candidate, Claude Opus 5.5: 5 - Header, panel titles, tabs, Launch textarea and Ask input sit at identical positions in live-typing t0/t30 and hover t0/t30 while counts and lists update underneath; each list scrolls inside its panel.
- Candidate, GPT 6.2 Sol: 5 - Across the typing stills the header, pane titles and focused Launch field hold their positions as lists update, and the smaller layouts visibly dock the controls outside the lists.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tab strip, Launch/Ask pane, Answers and Activity headers stay put at every size; only the decision list, ticket list and activity list scroll (t0 vs t30 show the chrome unmoved).
- Baseline, Claude Opus 5.5: 1 - Only the thin top bar is sticky; in live-typing the whole page is scrolled, panel titles scroll away, and the Launch input itself moves as content above it grows.
- Baseline, GPT 6.2 Sol: 2 - The connection bar remains visible in the typing pair, while panel titles scroll away and the Launch field moves down about 166 pixels.
- Baseline, Claude Sonnet 5.5: 1 - Only the top bar is fixed; the whole page scrolls as one (live-typing stills show the page scrolled with panel titles gone and the Launch input mid-page).

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is its own first panel with its count in the title and a '1 new' pill at hover-t30, and ticket rows mirror it; the global header shows only '10 running', not the decision count.
- Candidate, GPT 6.2 Sol: 5 - Decisions has its own desktop pane with a header count and an explicit 390-wide navigation tab carrying the same count.
- Candidate, Claude Sonnet 5.5: 4 - Decisions is a first-class left queue with its count in the panel title (and on the Decisions tab at 390), urgency-ordered; the header shows '10 running' rather than the decision count.
- Baseline, Claude Opus 5.5: 4 - Decisions is a separate first section and the header reads '7 decisions 15 tickets 10 running', updating to 10 decisions at t30; the queue itself scrolls away with the page.
- Baseline, GPT 6.2 Sol: 4 - The dedicated Decisions pane shows seven both in its title and in the global status bar, while the phone shows no dedicated Decisions tab.
- Baseline, Claude Sonnet 5.5: 4 - Decisions is its own left column and its count appears in the header ('7 decisions') and panel title, but it is not urgency-ordered and sits in a scrolling page.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask are fixed at the top of the right column with Answers directly under Ask, and the typed 'acme/widgets#52' stays put from t0 to t30; launched tickets land in the middle column rather than beside the input, and at 1024 the inputs move to a bottom bar.
- Candidate, GPT 6.2 Sol: 4 - The desktop Launch field stays at the same coordinates with its draft intact at t30 and Ask sits above Answers, while the compact Launch/Ask dock is shared across views.
- Candidate, Claude Sonnet 5.5: 4 - Launch and Ask sit in a fixed right pane (bottom bar at 1024/390) and answers appear directly beneath Ask; typing focus is held in the Launch box at t0 and t30 with nothing moving; the Launch pane is separate from the Tickets list it feeds.
- Baseline, Claude Opus 5.5: 1 - Launch and Ask sit at the end of the page after decisions, tickets, Activity and Settled; the focused Launch textarea moved from y~575 to y~740 between t0 and t30.
- Baseline, GPT 6.2 Sol: 1 - Launch and Ask appear after Activity and Settled, and the focused Launch field retains its text but drops about 166 pixels when a decision arrives.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask are at the end of the page below the lists and Activity/Settled; their position moves as the content above grows (557 to 723 px), and answers appear below them far from the tickets.

### R4

- Candidate, Claude Opus 5.5: 4 - Status is a small mark plus word in a fixed column ('o running', '* waiting', 'square failed'), and needs-you tickets sort to the top; restrained red/amber/blue palette, though left bars plus colored text plus marks layer three signals.
- Candidate, GPT 6.2 Sol: 4 - The blocker is pinned first and statuses use compact marks and words with red and amber reserved for exceptions, though colored row edges repeat some of the same information.
- Candidate, Claude Sonnet 5.5: 4 - Position carries most status (decisions column, left stripes, needs-you rows) with a small mark plus a word per row (failed, running, waiting, idle); only red, amber and blue links appear.
- Baseline, Claude Opus 5.5: 2 - A colored pill on everything: ticket status, each PR status, gate type, decision id, in blue, amber, grey, red and green; position carries little status.
- Baseline, GPT 6.2 Sol: 2 - The separate Decisions list provides some positional meaning, while the ticket column mixes routine and gated work and repeats filled worker and ticket status pills.
- Baseline, Claude Sonnet 5.5: 2 - A coloured chip on every ticket and PR (running blue, idle grey, waiting amber, gate amber, failed red, done green), so status is carried by chips rather than position.

### R5

- Candidate, Claude Opus 5.5: 4 - Ticket rows are a fixed height whether they hold two or three lines, with time right-aligned and PR links in one column; ledger#201 turning from idle note to 'needs you - a8' kept its row size, but decision cards differ in height.
- Candidate, GPT 6.2 Sol: 5 - Collapsed decision rows retain their height and right-aligned ages, and the visibly hovered Full message link on relay#11 stays in place as the '1 new' indicator appears at t30.
- Candidate, Claude Sonnet 5.5: 4 - Ticket rows keep roughly fixed height with age right-aligned and reply/dismiss in the same spot; at t30 times changed and a 'needs you +1' appeared without resizing any row, and a new decision was buffered ('1 new') instead of shifting the list under the pointer.
- Baseline, Claude Opus 5.5: 2 - Ticket rows vary from one to three lines with note length (widgets#41 shrank when its gate note was replaced), decision cards vary by button count; age is right-aligned in decisions and the hovered button did hold its place.
- Baseline, GPT 6.2 Sol: 2 - The hovered widgets#41 'merge and deploy' button stays put between stills, but decision cards vary by reply controls and ticket rows shrink as new notes replace wrapped text.
- Baseline, Claude Sonnet 5.5: 2 - Row heights change with content (widgets#41 went from a 3-line note to a 1-line note at t30 and the following rows moved up); links sit in different x positions per row; the disconnected banner shifts the whole page.

### R6

- Candidate, Claude Opus 5.5: 3 - Opening the relay#11 full message expands the card in place (list position kept) but grows it and jumps Reply/Dismiss above the body text.
- Candidate, GPT 6.2 Sol: 3 - Opening relay#11 replaces the preview with the full migration question inline, growing the card and nudging subsequent decisions down.
- Candidate, Claude Sonnet 5.5: 3 - 'Full message' expands the body inline in the card (becomes 'Hide message') and pushes the cards below down; the list position is kept and no page jump occurs, but it is a disclosure that grows the list.
- Baseline, Claude Opus 5.5: 3 - 'Full message m43' opens an inline monospace block inside the card, pushing every later card down ~85 px; reply controls stay put.
- Baseline, GPT 6.2 Sol: 3 - Full message expands inside the widgets#41 decision and adds roughly 84 pixels above the next card, with the ticket pane remaining alongside.
- Baseline, Claude Sonnet 5.5: 3 - 'Full message' opens a monospace block inside the card and pushes the cards below down; inline disclosure with no page jump, but the list grows.

## Reference means

| Reference | P4 | P5 | N3 | N4 | N5 | R2 | R4 | P6 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 3.67 | 5.00 | 3.33 | 3.67 | 4.00 | 1.67 | 4.00 | - |
| claude-code-agent-view | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 | - |
| datadog-dashboard | 3.00 | 4.00 | 3.67 | 3.33 | 3.00 | 1.00 | 1.67 | 3.67 |
| datadog-oncall-pages | 4.33 | 4.00 | 4.33 | 3.67 | 4.33 | 4.33 | 3.00 | 4.00 |
| github-actions-workflow-graph | 4.00 | 4.33 | 4.67 | 4.67 | 4.00 | 1.00 | 4.33 | - |
| github-projects-board | 3.00 | 3.33 | 3.33 | 2.33 | 2.67 | 2.33 | 2.00 | - |
| github-projects-table | 3.00 | 5.00 | 4.33 | 3.33 | 3.33 | 1.33 | 2.00 | - |
| grafana-alert-rules | 3.00 | 4.00 | 2.33 | 2.00 | 3.67 | 2.00 | 4.00 | 4.67 |
| grafana-alerting-dashboard | 2.67 | 3.67 | 2.33 | 2.33 | 3.33 | 2.67 | 3.67 | 4.67 |
| jira-board | 2.33 | 3.00 | 3.67 | 2.33 | 2.33 | 1.67 | 2.33 | - |
| linear-inbox | 5.00 | 5.00 | 5.00 | 4.67 | 5.00 | 5.00 | 5.00 | 4.33 |
| linear-issue-agent-panel | 4.00 | 4.00 | 4.33 | 4.33 | 5.00 | 2.00 | 4.00 | 4.67 |
| linear-triage | 5.00 | 5.00 | 4.67 | 4.67 | 5.00 | 4.33 | 4.67 | 4.67 |
| slack-channel-thread | 4.00 | 4.00 | 4.33 | 4.33 | 4.33 | 2.33 | 3.00 | - |
| thunderbird-unified-inbox | 3.67 | 4.00 | 4.33 | 3.67 | 4.33 | 3.33 | 3.33 | - |
| zulip-stream-topics | 4.00 | 4.00 | 4.67 | 4.33 | 4.33 | 3.33 | 3.33 | - |

## Judges' notes

- **Claude Opus 5.5**: A is a fixed three-pane tool: chrome and inputs held still across both t0/t30 pairs, rows are fixed height, blocker sorted first. Biggest A weakness: the right column crams Launch, Ask, Answers and Activity together (an Answers heading is clipped), detail disclosure jumps Reply/Dismiss above the body, and Launch/Ask stay fully enabled while disconnected. B is a scrolling page of rounded cards: Launch/Ask sit at the end of content and moved ~165 px while the user was typing, ticket rows reflow with note length and wrap links at 1024, pills are on everything, the loading state is blank blocks, and the relay#11 blocker is not first in Decisions. Biggest B weakness: no fixed layout, so the input the user is typing in drifts as live data arrives. B's one clear advantage is the header decision count.
- **GPT 6.2 Sol**: B’s biggest weakness is clipping: the first blocker preview hides the actual choice, long answer and activity lines are truncated, and opening the full decision shifts later cards. A’s biggest weakness is that Launch follows the growing lists: the 30-second pair shows the focused field moving down by about 166 pixels even though its draft survives.
- **Claude Sonnet 5.5**: Biggest weakness A: truncation and clipping - Answers questions are ellipsized and the third answer is cut by the Activity header, ticket and decision bodies end in '...', and the mobile decision card puts 'Full message' on its own row under the id, which wastes space. Biggest weakness B: the whole page scrolls as one with Launch/Ask at the bottom (Launch moved ~166px down between typing t0 and t30 as decisions were added), no urgency ordering in decisions, blank loading boxes, and explanatory empty-state copy. B's ticket list shows a failed relay#11 with 'gate: sso' while its decision list is unordered, so the blocker is not obvious. Reference scoring: linear-triage, linear-inbox and claude-code-agent-view are exemplars; the kanban and card references score low on N4/N5/R4.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | A | B |
| GPT 6.2 Sol | B | A |
| Claude Sonnet 5.5 | A | B |
