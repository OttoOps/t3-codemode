# Judging round 1

Candidate: the fixed panel board at `b36808e (pre-squash)`. Baseline: the board at `7e29c9d`. Captures: `harness/judge-stills.js` against the seeded demo board, one instance per subject started together; measurements: `harness/gates.js` (seeded and heavy), `harness/stability.js` (churn), `harness/holds.js` (seeded). The raw measurement output is in [`round-1-measurements.json`](round-1-measurements.json).

Judges: Claude Opus 5.5, GPT 6.2 Sol and Claude Sonnet 5.5, each with its own packet, letters and order. Each saw the judge brief (the judged criteria with their anchors, no Baseline column), the blinded stills and the sixteen references. The stability recordings were replaced by t0 and t30 stills of the same session, since the judges read images, not video.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| U1 | pass | scrollHeight = innerHeight at 1440x900, 1280x800, 1024x768, seeded and heavy (57 open tickets) |
| U2 | pass | at 1440 and 1280 both inputs in view after scrolling every panel; at 1024 the selected input (Launch) in view, and one activation switches to Ask keeping both drafts; Tab presses: Launch 2, Ask 3; `l` and `/` focus them |
<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:b93f0cddc36fd2bdcbe11af6a9a5ea53437f8d160c98ae043fb3bc81dac89a42 -->

| U3 | pass | focused Launch, Ask, card reply, Settled filter: 0 px moved, value and caret kept through a second-context answer and a stream cut; focused thread link, worker PR link, ask Settle, settled Restore stayed the same connected node for 30 s; hovered card option and ticket Settle did not move in the recorded samples |

<!-- /coderabbit-ai-generated:v1 -->
| U4 | pass | first visible item unchanged after 60 s in Decisions, Tickets and Activity; header slots read cards: 2 new, tickets: 2 new, feed: 15 new; tab switch restored scrollTop 61 |
| U5 | pass | 6 cards fully visible at 1440x900, median height 118 px |
| U6 | pass | 13 open rows visible at 1440 (13 on the heavy board); same-field rows share one height at 1024 and 1440 |
| U7 | pass | card order ['blocker', 'gate', 'gate', 'gate', 'gate', 'gate', 'report']; row to card and card to row in one activation each |
| U8 | pass | all options styled alike; no request before 5 s, one after; Undo sent nothing; a digit only focused its option |
| U9 | pass | skip links to all five targets; shortcuts inert inside fields and when turned off; `?` lists them; j, g t, g a move focus |
| U10 | pass | no horizontal scroll at 768 and 390; one panel shown; no target under 44 px; nothing clipped at 195 px (200% zoom); emulated keyboard keeps field and submit in view; a 250 px viewport switches the composer to sheet mode (height and restore on close not measured) |
| U11 | pass | header during the cut: "Disconnected. Reconnecting; showing data from 01:51."; panels dimmed; reply controls aria-disabled; panes and composer moved 0 px |
| P1 | pass | lowest status text light 6.37, dark 6.74; input border light 3.21, dark 4.32 |
| P2 | pass | sizes ['16px', '13px', '14px', '12px'], weights ['600', '400', '500'], no off-scale spacing |
| P3 | pass | accent on no status mark; amber only on needs-you items; running is neutral |
| N1 | pass | no gradients, shadows or emoji |
| N2 | pass | no Markdown; zone MST once; 47 truncated strings, all with full text in a tooltip or disclosure |
| F2 | pass | `cargo test --release --locked` green; tests/board.rs adds shell, held-arrival, placeholder, deferred-send, option, row, needs-you and stale-stream tests |
| F3 | pass | Ask error 1x (alert), launch result 1x (both issues failed, so a mixed result was not exercised), card error 1x (alert), 0 announcements matching the sampled Activity patterns |
| F4 | pass | every option, Send, Dismiss, Settle and Restore name carries its card id, ticket label or question |

F1 is covered by the gate scripts and tests above (a launch whose issues both fail, a failing Ask, a card reply that errors, a disconnect, Settle and Restore) and the existing thread-link fallback tests. The scripts do not exercise the hidden-tab notification: it calls the same `Notification` code as before, and permission now comes from the header's Notify me control instead of the first click.

The heavy board reached 57 open tickets but held at 12 decisions: in this harness a newer gate from a ticket supersedes its older gate card, so the decision count plateaus below the rubric's 15. The 15-decision load was not measured in this round.

## Judged scores

Means across judges, each judge's score in brackets (Opus/GPT/Sonnet). Holds means no more than 0.5 below the baseline and, for the still criteria, at least the median of the reference means.

| Criterion | Candidate | Baseline | Reference median | Holds |
| --- | --- | --- | --- | --- |
| U12 | 4.67 (5/4/5) | 2.33 (2/2/3) |  | yes |
| U13 | 4.33 (4/5/4) | 2.33 (2/3/2) |  | yes |
| P4 | 4.00 (4/4/4) | 3.00 (3/3/3) | 4.00 | yes |
| P5 | 3.33 (3/4/3) | 2.67 (3/2/3) | 4.00 | no |
| P6 | 4.33 (4/5/4) | 3.33 (3/4/3) | 4.67 | no |
| P7 | 4.00 (4/4/4) | 2.00 (2/2/2) |  | yes |
| N3 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.33 | no |
| N4 | 3.33 (3/4/3) | 2.33 (2/3/2) | 3.83 | no |
| N5 | 4.33 (4/5/4) | 2.00 (2/2/2) | 4.33 | yes |
| R1 | 5.00 (5/5/5) | 1.67 (1/2/2) |  | yes |
| R2 | 4.33 (4/4/5) | 3.67 (3/4/4) | 2.33 | yes |
| R3 | 4.33 (4/4/5) | 1.00 (1/1/1) |  | yes |
| R4 | 4.00 (4/4/4) | 2.00 (2/2/2) | 4.00 | yes |
| R5 | 3.67 (4/4/3) | 2.00 (2/2/2) |  | yes |
| R6 | 3.00 (3/3/3) | 3.00 (3/3/3) |  | yes |

| Axis | Candidate | Baseline | Beats |
| --- | --- | --- | --- |
| Usability | 4.50 | 2.33 | yes |
| Appearance and polish | 3.92 | 2.75 | yes |
| No slop | 3.89 | 2.11 | yes |
| Reference fit | 4.06 | 2.22 | yes |

**Decision rule: not met (P5, P6, N3, N4).**

## Evidence

### U12

- Candidate, Claude Opus 5.5: 5 - At 1440 the left Decisions panel lists the relay#11 blocker first with a red bar, then gates a1-a5 each with its reply buttons (merge, accept/fix, one/split), and the header says 7 decisions.
- Candidate, GPT 6.2 Sol: 4 - The 1440 queue leads with the relay#11 migration blocker, then shows widgets#41 merge or deploy #81, relay#9 keep or lower the retry budget, atlas#60 approve AWS sign-in, ledger#198 merge #203, atlas#57 choose one or two PRs, and a bottom-clipped gizmo#15 report about three hours with no PR.
- Candidate, Claude Sonnet 5.5: 5 - Left Decisions panel lists the relay#11 blocker first, then gates a1-a5 and report a7, each with its reply buttons, and tickets are tagged 'needs you - aN'; readable in seconds at 1440.
- Baseline, Claude Opus 5.5: 2 - The 1440 still shows only four large decision cards starting with widgets#41; the relay#11 blocker is not visible and appears only far down the tickets list as 'failed', so urgency order is lost.
- Baseline, GPT 6.2 Sol: 2 - The 1440 Decisions pane spells out widgets#41 merge or deploy #81, relay#9 keep or lower the retry budget, atlas#60 approve AWS sign-in, and ledger#198 merge #203, while the remaining three decisions are below the fold and the relay#11 blocker is absent from the first screen.
- Baseline, Claude Sonnet 5.5: 3 - Decisions column leads with gates a1-a4 and Merge/Accept buttons, but the blocker (relay#11) and report are not first and only appear as a 'failed / gate: sso' chip lower in Tickets; has to scan both columns.

### U13

- Candidate, Claude Opus 5.5: 4 - Three fixed columns (Decisions, Tickets, Launch/Ask/Activity) with titled headers at 1440; at 1024 Activity becomes a Tickets tab and Launch/Ask a bottom bar, at 390 a bottom tab bar; the right column's Answers block has a weak boundary.
- Candidate, GPT 6.2 Sol: 5 - At 1440 the bordered Decisions, Tickets and utility panes have clear homes, at 1024 Activity moves into the Tickets header and Launch/Ask into a dock, and at 390 the bottom tabs label each view.
- Candidate, Claude Sonnet 5.5: 4 - Decisions / Tickets / Launch+Ask+Answers+Activity each have a titled, bordered home at 1440, tabs at 390 and a bottom Launch bar at 1024; at 1280 the ticket name column collapses to one letter, weakening the Tickets panel.
- Baseline, Claude Opus 5.5: 2 - Centered two-column card stream with Activity and Settled as collapsed rows and Launch/Ask at the bottom of the page; their position depends on how many decisions and tickets exist, and at 390 everything is one long scroll.
- Baseline, GPT 6.2 Sol: 3 - Decisions and Tickets are clearly labeled at 1440 and 1024, but at 390 only the stacked Decisions cards are visible and the typing stills place Activity, Launch and Ask below the lists.
- Baseline, Claude Sonnet 5.5: 2 - Decisions, Tickets, Activity (collapsed), Settled (collapsed), Launch and Ask are separate blocks on one scrolling page; at 390 and 1024 they stack and Launch/Ask end up below everything, so positions depend on content length.

### P4

- Candidate, Claude Opus 5.5: 4 - Decisions panel and the red/orange-barred needs-you ticket group draw the eye first, but the orange 'needs you - aN' text on eight rows and many blue links make the tickets panel compete.
- Candidate, GPT 6.2 Sol: 4 - The red relay#11 blocker leads the left queue and amber needs-you labels draw attention to affected tickets, although the blue Launch and Ask buttons also pull the eye.
- Candidate, Claude Sonnet 5.5: 4 - Red blocker bar and bold ticket names lead, amber gate bars next, plain running tickets and activity recede; panel titles are bold, though 'needs you - aN' repeats the decisions list and competes slightly.
- Baseline, Claude Opus 5.5: 3 - Big blue reply buttons make decisions stand out, but ticket cards carry status pills, gate chips and long '**ACTION NEEDED:**' notes that compete, and the failed ticket has no priority.
- Baseline, GPT 6.2 Sol: 3 - Decisions gets the wider left column and prominent reply buttons, while ticket status pills compete for attention and widgets#44 and gizmo#12 precede several gated tickets.
- Baseline, Claude Sonnet 5.5: 3 - White decision cards with blue primary buttons stand out and the section titles are small grey labels; gate chips, status chips, sub-ticket rows and PR links all carry similar weight in the Tickets column.

### P5

- Candidate, Claude Opus 5.5: 3 - Status, name, PR, thread and age sit in fixed columns at 1440 and 1024, but at 1280 ticket names collapse to 'r.', 'w.', 'l.' to make room for 'needs you', and the PR sublines wrap differently by worker count.
- Candidate, GPT 6.2 Sol: 4 - Top-level status, PR, thread and time align at 1440, 1280 and 1024, though several flagged ticket names collapse to a single letter at 1280 and worker PRs are grouped inline.
- Candidate, Claude Sonnet 5.5: 3 - At 1440 and 1024 status, name, PR, thread and age sit in the same columns on every ticket row; at 1280 the names on needs-you rows shrink to 'r...', 'w...', 'a...' and the PR links get clipped.
- Baseline, Claude Opus 5.5: 3 - PR and thread links are right-aligned at 1440, but chip widths vary, child PR rows indent differently, and at 1024 'PR #204 open' wraps onto its own line under ledger#198.
- Baseline, GPT 6.2 Sol: 2 - PR and thread links align in the wider stills, but at 1024 those links wrap below the title for widgets#41 and ledger#198 while status pills follow the changing title width.
- Baseline, Claude Sonnet 5.5: 3 - Decision cards and ticket rows are internally consistent with status chips and PR/thread links right-aligned, but note text wraps to 1-3 lines, sub-PR rows differ per ticket, and link positions shift at 1024 (ledger#198 PR #204 wraps to its own line).

### P6

- Candidate, Claude Opus 5.5: 4 - Dark theme keeps panel borders, the red/amber left bars and status colors legible with a lighter-blue Launch button; looks designed rather than inverted.
- Candidate, GPT 6.2 Sol: 5 - At 1440 and 390 the charcoal surfaces, visible dividers, blue links and salmon and amber severity marks preserve clear panel and status roles.
- Candidate, Claude Sonnet 5.5: 4 - Dark 1440 keeps the same roles: red/amber left bars, tinted blocker, bordered buttons, pane dividers all still visible, readable text; 390 dark works too.
- Baseline, Claude Opus 5.5: 3 - Dark mode is correct and readable with lighter-blue buttons and visible card edges; it is a straightforward recolor rather than a tuned dark design.
- Baseline, GPT 6.2 Sol: 4 - On desktop and phone, bright text, blue controls and amber gate labels stay legible, although card edges and input borders are subtle against the dark background.
- Baseline, Claude Sonnet 5.5: 3 - Dark theme is correct and readable with tinted chips, but card and page backgrounds are close so boundaries between cards and the page are faint, especially at 390.

### P7

- Candidate, Claude Opus 5.5: 4 - Empty says 'Paste an issue into Launch to start one', loading keeps all panels with 'Loading tickets...', disconnected puts a red 'Reconnecting; showing data from 01:46' line in the header and dims reply buttons, all in the same layout.
- Candidate, GPT 6.2 Sol: 4 - Connecting names the lists being loaded, the empty Tickets pane points to Launch, and disconnect reports retrying and the last data time in place, though dimming makes cached rows harder to read.
- Candidate, Claude Sonnet 5.5: 4 - Loading shows 'Connecting' and 'Loading decisions...' with the full layout kept; empty says 'Nothing needs you...' and 'Paste an issue into Launch'; disconnected shows a red 'Disconnected. Reconnecting; showing data from 01:46.' and greys the stale items with dimmed buttons.
- Baseline, Claude Opus 5.5: 2 - Loading shows empty white blocks under Decisions and Tickets with only 'connecting' in the header; empty copy is positional ('Launch one below'); disconnected adds a pink banner that pushes the whole page down.
- Baseline, GPT 6.2 Sol: 2 - Connecting leaves two blank cards, the empty Tickets message says to launch one below, and the disconnect banner clearly says Retrying but shifts the panels down.
- Baseline, Claude Sonnet 5.5: 2 - Loading shows empty white blocks with no text and a bare 'connecting'; empty copy says 'Launch one below' (positional) and 'A short-lived thread reads the board...'; disconnected shows a pink banner that pushes the whole page down, though the message is clear.

### N3

- Candidate, Claude Opus 5.5: 4 - Strings are mostly data or instructions ('One issue per line. Ctrl+Enter launches.'); the empty Decisions copy explains where gates come from, a little wordy.
- Candidate, GPT 6.2 Sol: 4 - The empty Tickets pane names Launch and the textarea documents Ctrl+Enter, while the empty Answers line repeats what the adjacent Ask control already implies.
- Candidate, Claude Sonnet 5.5: 4 - Strings are mostly data or instruction ('One issue per line. Ctrl+Enter launches.', 'Working on it...'); empty states point to a control; a few lines such as 'gate sent; waiting on a6' only restate the badge.
- Baseline, Claude Opus 5.5: 2 - 'A short-lived thread reads the board and answers in a few lines.' describes the implementation, 'Full message m43' exposes internal ids, and raw '**ACTION NEEDED:**' markdown appears in ticket notes.
- Baseline, GPT 6.2 Sol: 2 - The Ask caption describes a short-lived implementation thread, the empty Tickets instruction says below, and the Tickets summaries expose raw ACTION NEEDED markers.
- Baseline, Claude Sonnet 5.5: 2 - Strings include 'A short-lived thread reads the board and answers in a few lines.', 'Full message m43' with internal ids, 'Reply in your own words' repeated on every card, and sub-rows that repeat the ticket name ('widgets#41 PR 1/2').

### N4

- Candidate, Claude Opus 5.5: 3 - The 'times in MST' label and the detail view that repeats the decision text verbatim in monospace add nothing; ticket rows show the PR link twice (right column and subline).
- Candidate, GPT 6.2 Sol: 4 - The thread and Full message links serve different jobs in the closed queue, but the expanded relay#11 text and its raw block repeat the same complete message.
- Candidate, Claude Sonnet 5.5: 3 - 'thread' link on every ticket row and decision, 'needs you - aN' duplicating the decision list, and 'times in MST' plus '?' in the header add little.
- Baseline, Claude Opus 5.5: 2 - Repeated 'Reply in your own words' input on every card, internal message ids, the 'not joined yet' text, duplicate PR/thread links on parent and child rows, and an 'a1' chip next to a 'gate' chip carry little.
- Baseline, GPT 6.2 Sol: 3 - The m43-style IDs beside Full message add little for scanning, and the headers could carry the ticket or decision totals without repeating them in the top strip.
- Baseline, Claude Sonnet 5.5: 2 - A status chip on every ticket and sub-ticket, 'gate: merge' chips duplicating the decision, a 'Full message mNN' disclosure per card and a reply box on every card carry little extra information.

### N5

- Candidate, Claude Opus 5.5: 4 - Dense three-pane tool layout with left status bars, aligned rows and plain text status marks; decision items are still bordered cards but it reads as a purpose-built ops console.
- Candidate, GPT 6.2 Sol: 5 - The severity-ordered decision queue, worker PR summaries and live event rail give the board the density and structure of a dedicated operations console.
- Candidate, Claude Sonnet 5.5: 4 - Three-pane inbox layout with left status bars and compact rows reads as a purpose-built ops tool and sits near Thunderbird/Linear; plain default fonts and underlined blue links keep it from a 5.
- Baseline, Claude Opus 5.5: 2 - Rounded white cards on a beige page, pill badges for every status and gate, centered max-width layout: reads as a generic web dashboard.
- Baseline, GPT 6.2 Sol: 2 - Shadowed decision cards, colored status pills and full-width reply fields give the board a familiar dashboard appearance despite the useful worker PR nesting.
- Baseline, Claude Sonnet 5.5: 2 - White rounded cards with soft shadows, pill chips in many hues and a centered column read as a generic dashboard template.

### R1

- Candidate, Claude Opus 5.5: 5 - Header, panel titles and Launch/Ask inputs stay put; in live-typing t0 to t30 the Launch textarea with 'acme/widgets#52' keeps its exact position while lists update.
- Candidate, GPT 6.2 Sol: 5 - The 1440 typing pair keeps the header, pane labels and focused Launch field stationary while ticket notes and activity update, and the phone stills show a bounded list between its header and the action dock.
- Candidate, Claude Sonnet 5.5: 5 - Header, panel titles, tab strip, Launch/Ask inputs and the Activity title stay put while only the lists scroll (decisions and tickets cut off at the viewport bottom); at 390 and 1024 the input bar and tabs are fixed.
- Baseline, Claude Opus 5.5: 1 - Only the thin status header sticks; panel titles scroll away and the Launch input moves from y~595 to y~760 between live-typing t0 and t30 as new decisions arrive.
- Baseline, GPT 6.2 Sol: 2 - The top connection strip stays visible in the Launch typing pair, while the Decisions and Tickets headings scroll away and the Launch field moves down about 166 pixels between captures.
- Baseline, Claude Sonnet 5.5: 2 - Only the top status bar is fixed; panel titles, tickets and Launch/Ask scroll away with the page (the typing stills show the page scrolled with titles gone).

### R2

- Candidate, Claude Opus 5.5: 4 - Decisions is its own left-hand queue with its count in the panel title and the header ('7 decisions', later '10 decisions' and a '1 new' marker); tab title cannot be judged from stills.
- Candidate, GPT 6.2 Sol: 4 - Decisions is the first pane, its count appears in desktop chrome and on the phone Decisions tab, and the browser tab title is outside the captures.
- Candidate, Claude Sonnet 5.5: 5 - Decisions is its own first-class left queue with its count in the panel title, in the header ('7 decisions', gold) and in the 390 tab ('Decisions 7'); a '1 new' marker appears when new ones arrive.
- Baseline, Claude Opus 5.5: 3 - Decisions is a separate section with its count in the header ('7 decisions'), but it is not sorted by urgency and has no new-item marker.
- Baseline, GPT 6.2 Sol: 4 - The separate Decisions column shows its count beside the heading and in the top strip, with no counted Decisions tab visible in the stills.
- Baseline, Claude Sonnet 5.5: 4 - Decisions is its own left column with its count in the header and title; but ordering is by id with the blocker and report not first, and there is no tab-style queue.

### R3

- Candidate, Claude Opus 5.5: 4 - Launch and Ask are fixed at the top of the right column next to the tickets list, with Answers appearing directly below Ask; at 1024 they move to a bottom bar, more detached.
- Candidate, GPT 6.2 Sol: 4 - Launch and Ask remain fixed with Answers immediately below Ask on desktop, and the typing pair preserves position, focus and text, although the narrow-screen dock spans the board.
- Candidate, Claude Sonnet 5.5: 5 - Launch and Ask sit fixed in the right pane, with the Answers list directly under them and Activity below; at narrower widths a fixed bottom Launch/Ask bar holds the input.
- Baseline, Claude Opus 5.5: 1 - Launch and Ask sit at the end of the page below Activity and Settled, far from the tickets list, and get pushed down as content grows.
- Baseline, GPT 6.2 Sol: 1 - Launch and Ask sit beneath the lists, Activity and Settled, and the new decision in the typing pair pushes the still-focused Launch field farther down while its draft survives.
- Baseline, Claude Sonnet 5.5: 1 - Launch and Ask are at the very bottom of the page below Activity and Settled, so the input is at the end of content; answers appear under Ask at the page bottom.

### R4

- Candidate, Claude Opus 5.5: 4 - Needs-you tickets are grouped at the top with a colored left bar; status is a small mark plus word (o running, failed square); palette is red, amber and blue links, though orange 'needs you' text repeats on many rows.
- Candidate, GPT 6.2 Sol: 4 - The blocker sits above amber gates and the gray report, and ticket rows use small shapes and status words with a restrained palette, though an unflagged waiting row interrupts the needs-you grouping.
- Candidate, Claude Sonnet 5.5: 4 - Position (blocker, then gates) and a 3px left bar carry status, plus a small mark and one word (failed / running / waiting / idle); palette is red, amber, blue, grey; 'needs you' text and bold ticket names add some extra emphasis.
- Baseline, Claude Opus 5.5: 2 - Every ticket and PR row has a colored pill (running blue, waiting amber, idle gray, failed red, done green) plus an outlined gate chip; position carries no status.
- Baseline, GPT 6.2 Sol: 2 - Decisions has its own column, but running, waiting, idle, failed and gate labels rely heavily on pills, with gated and routine tickets interleaved.
- Baseline, Claude Sonnet 5.5: 2 - A colored pill on every ticket and PR (blue running, grey idle, amber waiting, red failed, plus amber gate outline) with little position-based structure; many hues.

### R5

- Candidate, Claude Opus 5.5: 4 - Ages right-aligned at 01:47 etc., Reply/Dismiss in the same spot on every decision; status changes swap text without resizing, though rows are two or three lines depending on PR subline and atlas#62 reorders into the needs-you group between t0 and t30.
- Candidate, GPT 6.2 Sol: 4 - The relay#11 Full message link stays under the hover highlight as a 1 new indicator appears, and decision ages and actions line up, though ticket rows with workers are taller.
- Candidate, Claude Sonnet 5.5: 3 - Age is right-aligned and Reply/Dismiss/thread sit in the same spot, but rows are 1-3 lines tall depending on a PR sub-line or the number of PRs, and decisions vary by wrapped text, so a status change can still resize a row.
- Baseline, Claude Opus 5.5: 2 - Decision and ticket cards vary in height with note length and worker count; the reply button under the pointer stayed put only because new items appended below.
- Baseline, GPT 6.2 Sol: 2 - The widgets#41 merge-and-deploy hover stays on its button across the pair and ages are right-aligned, but decisions with quick replies are taller and ticket rows shrink when long notes are replaced.
- Baseline, Claude Sonnet 5.5: 2 - Ticket rows vary from 2 to 5 lines with wrapping notes and sub-rows, no age on ticket rows, and rows change height when notes or PR counts change; decision ages are right-aligned.

### R6

- Candidate, Claude Opus 5.5: 3 - Clicking Full message expands the relay#11 card inline with a monospace block, pushing the other decisions down; list scroll position is kept.
- Candidate, GPT 6.2 Sol: 3 - Full message opens inside relay#11 without a page jump, and the enlarged card pushes widgets#41 down.
- Candidate, Claude Sonnet 5.5: 3 - 'Full message' expands a monospace block inline inside the decision card, which grows that list while the panel keeps its scroll position and no page jump; it is not a side pane.
- Baseline, Claude Opus 5.5: 3 - Full message is an inline disclosure that grows the a1 card with a monospace block and pushes the rest of the list down.
- Baseline, GPT 6.2 Sol: 3 - Full message m43 opens within the widgets#41 card and keeps its top in place, while the relay#9 card shifts down.
- Baseline, Claude Sonnet 5.5: 3 - Full message expands inline under the decision card, pushing the cards below it down, with no page jump but a growing list.

## Reference means

| Reference | P4 | P5 | N3 | N4 | N5 | R2 | R4 | P6 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| buildkite-pipelines | 4.00 | 5.00 | 4.00 | 3.67 | 4.33 | 1.33 | 4.00 | - |
| claude-code-agent-view | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | - |
| datadog-dashboard | 3.33 | 4.00 | 4.00 | 3.33 | 3.33 | 1.00 | 2.33 | 4.00 |
| datadog-oncall-pages | 4.33 | 4.00 | 4.33 | 4.00 | 4.33 | 4.33 | 3.33 | 4.00 |
| github-actions-workflow-graph | 3.67 | 4.00 | 4.67 | 4.33 | 4.33 | 1.00 | 4.33 | - |
| github-projects-board | 3.00 | 3.00 | 3.33 | 2.67 | 3.00 | 2.33 | 2.33 | - |
| github-projects-table | 3.33 | 5.00 | 4.33 | 3.33 | 4.00 | 1.33 | 2.00 | - |
| grafana-alert-rules | 2.67 | 4.00 | 2.33 | 2.00 | 3.67 | 2.00 | 4.00 | 5.00 |
| grafana-alerting-dashboard | 3.00 | 4.00 | 2.33 | 2.67 | 3.33 | 2.67 | 4.00 | 4.67 |
| jira-board | 2.67 | 2.67 | 3.67 | 2.33 | 2.33 | 2.00 | 2.33 | - |
| linear-inbox | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 4.33 |
| linear-issue-agent-panel | 4.00 | 4.00 | 5.00 | 4.33 | 5.00 | 2.00 | 4.00 | 4.67 |
| linear-triage | 5.00 | 5.00 | 5.00 | 5.00 | 5.00 | 4.33 | 5.00 | 4.67 |
| slack-channel-thread | 4.00 | 4.00 | 4.33 | 4.33 | 4.33 | 2.33 | 4.00 | - |
| thunderbird-unified-inbox | 4.00 | 4.33 | 4.33 | 3.67 | 4.33 | 3.33 | 3.67 | - |
| zulip-stream-topics | 4.00 | 4.33 | 4.67 | 4.33 | 4.33 | 3.33 | 4.00 | - |

## Judges' notes

- **Claude Opus 5.5**: A is a fixed three-pane console that keeps inputs and chrome still under live updates and sorts the blocker first; its biggest weakness is width handling at 1280, where ticket names collapse to one letter to make room for the 'needs you - aN' label, plus a detail view that repeats the decision text verbatim. B is a page-scrolling card stream; its biggest weakness is that Launch/Ask sit at the end of content and move as decisions arrive, and the relay#11 blocker is not surfaced at the top of Decisions. B also leaks internal ids ('Full message m43') and raw markdown into the UI, and its loading state is blank white blocks.
- **GPT 6.2 Sol**: B’s biggest weakness is the Tickets pane at 1280, where several names lose nearly every character as the needs-you text and link columns retain their space. A’s biggest weakness is that the Launch and Ask controls sit below the lists and move as the board grows even while Launch has focus. Both hover pairs preserve the visible target and both full-message disclosures grow the queue inline. Browser tab titles were outside the supplied stills.
- **Claude Sonnet 5.5**: A biggest weakness: at 1280x800 the ticket name column on 'needs you' rows collapses to one letter (r..., w..., a...) and PR links clip, so the tickets pane is hard to scan at a common desktop width; ticket rows are also variable-height (1-3 lines). B biggest weakness: the page scrolls as one with Launch and Ask at the very bottom below Activity and Settled, the blocker and report are not first in the decisions list, and every ticket carries several pill chips, giving a generic card dashboard. Tab-title count could not be judged from stills; R2 for both is based on header and panel counts.

## Key

| Judge | Candidate | Baseline |
| --- | --- | --- |
| Claude Opus 5.5 | A | B |
| GPT 6.2 Sol | B | A |
| Claude Sonnet 5.5 | A | B |
