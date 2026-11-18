# Paired design review of the current board

Two reviewers looked at the board as it stands at commit `7e29c9d`: a Claude
Opus agent and GPT 6.1 Sol, the second briefed to weight taste and visual
style. Each worked alone first, from the live demo board (see
[`harness/`](harness/README.md)) and the captures in [`current/`](current/),
then read the other's review and settled the open questions. This page merges
the four documents. The originals are in [`review/`](review/):
[Claude](review/claude.md), [Sol](review/sol.md), and the cross-reads
[Claude on Sol](review/claude-cross.md) and [Sol on Claude](review/sol-cross.md). They are
kept as the reviewers wrote them, except that local file paths became
repository links and curly quotes became straight ones.

Line numbers refer to `src/board.html` at `7e29c9d`.

## Verdict

1. **Nothing has a fixed position.** The page is 2,345 px tall at 1440x900,
   5,323 px with Activity and Settled open, and 6,263 px on a 390 px phone.
   The Launch textarea starts 2.2 screens down on desktop and 6.6 on the
   phone. It is element 218 of 227 in tab order.
2. **Live updates move or destroy what the user is touching.** A focused
   Launch textarea moved 214 px in 36 s as cards and tickets arrived above it.
   The ticket tree, asks, feed and settled list are rebuilt from scratch on
   every update, so a focused ticket link is detached and focus falls to
   `<body>` within 9 s. A row under the pointer moved 80 px.
3. **Decisions are hard to triage and risky to click.** Cards are 156 to
   221 px tall, so about four fit at 1440x900. The board sorts them by age only. A
   blocker differs from a gate by one 13 px red word. The first parsed reply
   is always the filled primary button, so on a7 that is "stop", which ends a
   three-hour run, and every option posts on one click.
4. **Color carries too many meanings.** One blue means running, link, id
   badge and primary action. Amber means both "waiting on CI" and "needs
   you". Light-mode chip text for waiting, idle and done is below 4.5:1.

## Findings

Severity is the pair's settled rating. "Both" means each reviewer found it
on their own.

### Layout

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| L1 | High | One document flow; only the header is sticky, so every section's position depends on the content above it. | `desktop-1440-light-full.webp`; lines 67-68, 85-93 | Both |
| L2 | High | Launch and Ask are the last grid items, after Activity and Settled. | `report.json` `launchBelowFold`: 2.22 screens (desktop), 6.55 (phone) | Both |
| L3 | High | Below 900 px every decision comes before every ticket, and the inputs come last. | `phone-390-light-full.webp`, 6,263 px | Sol |
| L4 | Medium | `max-width: 1280px` leaves 96 px dead gutters at 1440 while content is cramped vertically. | `desktop-1440-light-top.webp`; line 88 | Claude |
| L5 | Medium | At 1024 px the PR and thread links wrap onto their own line on gated tickets, so row height varies from 66 to 118 px for the same fields. | `tablet-1024-light-top.webp`, widgets#41, ledger#198 | Both |

### Hierarchy

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| H1 | High | The first parsed reply is styled primary only because it comes first in the list; nothing in the source recommends it. | line 367 (`i === 0`); a7 "stop", a1 "merge" | Both |
| H2 | High | The board orders decisions by age only and gives them one visual weight; one 13 px red word marks a blocker, and a report has no color at all. | `desktop-1440-light-full.webp`, a6 and a7 | Claude |
| H3 | Medium | A ticket that needs the user is not marked as such, and nothing links the ticket row to its card. | widgets#41 row vs card a1 | Claude, accepted by Sol |
| H4 | Medium | Section headings are 13 px grey, smaller and lighter than the 15 px content they label. | lines 75, 94-95 | Both |
| H5 | Medium | "Settle all done" sits inside the Tickets heading, offsets it 8 px from Decisions, and becomes part of its accessible name. | lines 204-205 | Claude |

### Scroll behavior

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| S1 | High | One scroll container: reading history or a long ticket list moves every control away with it. | page height 2,345 to 5,323 px when history opens | Both |
| S2 | High | Activity is newest-first and re-rendered whole; with it open, each event pushes the row being read down one row. | lines 530-531 | Claude |
| S3 | Medium | Settled has no height limit, and its filter scrolls away with the list. | `desktop-1440-light-expanded-full.webp` | Claude |
| S4 | Medium | Only the global header sticks; scrolled down, the user loses which queue they are in. | `desktop-1440-light-typing-launch.webp` | Sol |

### Layout shift

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| X1 | High | Focused Launch textarea moved 556, 557, 604, 770 px (top) in 36 s of live updates. | `report.json` `focusedInputDrift`, 214 px | Both |
| X2 | High | Re-rendering the ticket tree, asks and settled list detaches focused controls; focus falls to `<body>`. | live probes by both reviewers; lines 528-534 | Both |
| X3 | High | The element under the pointer moves: rows shift as tickets insert above them, and an answered card's removal slides the next card's buttons under the cursor. | gizmo#91 row 1834, 1914, 1877 px; line 500 | Claude |
| X4 | Medium | The disconnect banner enters the document flow and pushes every panel down 35 px. | `desktop-1440-light-disconnected.webp`; line 83 | Both |

### Density

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| D1 | High | Each decision spends about 110 px on always-visible chrome (reply input, Send, Dismiss, "Full message mNN"); cards are 156-221 px, about 285 px on a phone. | `desktop-1440-light-top.webp`, `phone-390-light-top.webp` | Both |
| D2 | Medium | Ticket notes repeat the card text verbatim, two to three lines per gated ticket. | widgets#41, ledger#198 | Claude |
| D3 | Medium | Done rows are taller (57 px) than running rows (43 px) only because of the Settle button's 34 px minimum height. | line 119 | Claude |

### Typography

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| T1 | Medium | Raw Markdown leaks (`**ACTION NEEDED:**`) and notes are cut mid-word with no ellipsis ("https://githu"). | `desktop-1440-light-top.webp`; line 417 | Both |
| T2 | Medium | Seven sizes and weights within 600 px; service line, headings, notes and links all share 13 px, so weight and color do all the grouping. | lines 59, 76, 95, 150, 163 | Both |
| T3 | Low | Every Activity row repeats "MST". | `desktop-1440-light-expanded-full.webp` | Claude |

### Color

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| C1 | High | Chip text below 4.5:1 in light mode: waiting 4.22, idle 4.29, done 4.49. | tokens, lines 12-20, 153 | Both |
| C2 | High | Blue means running, primary, link and id badge; running rows form blue islands that compete with decisions. | `desktop-1440-light-top.webp` | Both |
| C3 | Medium | Amber means waiting, launching, gate kind and gate tag, so "waiting on CI" looks like "needs you". | lines 113, 159, 162 | Claude |
| C4 | Medium | Input borders `#dcdcd5` on white are 1.38:1, below the 3:1 non-text contrast minimum. | `desktop-1440-light-top.webp` reply inputs | Claude; Sol disputes only the claim that dark panels depend on shadow |

### Affordances

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| A1 | High | Option buttons send immediately, with no confirm or undo; "merge and deploy" and "stop" are one click next to a reply box, and subject to X3. | lines 355, 367 | Claude, accepted by Sol |
| A2 | Medium | Twelve identical Send and Dismiss buttons; reply inputs are named only "Reply to a1". | lines 356, 375 | Sol, accepted by Claude |
| A3 | Medium | Cards with no ticket still render a full disabled input and Send. | lines 356, 375 | Claude |
| A4 | Medium | On the phone, Dismiss wraps onto its own line, 44 px below Send. | `phone-390-light-top.webp` | Both |
| A5 | Medium | The first click anywhere, including a click on "merge", raises the notification permission prompt. | lines 577-579 | Claude, accepted by Sol |
| A6 | Low | Every container gets the same rounded, shadowed card, so passive history weighs as much as decision work. | lines 100, 165, 171 | Sol |

### Empty and loading states

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| E1 | Medium | Loading is one blank 64 px block per section, no text, no `aria-busy`, while Launch and Ask already look ready. | `desktop-1440-light-loading.webp`; lines 187, 201 | Both |
| E2 | Medium | The header shows a green "T3 connected" under the red Disconnected banner: the first `/api/state` fetch rewrites the status after `setConnected(false)`. | `desktop-1440-light-disconnected.webp`; lines 541-548, 556, 620 | Sol, confirmed in source by Claude |
| E3 | Low | Empty panels differ in height (77 vs 113 px), and "Launch one below" is positional copy that breaks once Launch moves. | `desktop-1440-light-empty.webp`; line 529 | Both |
| E4 | Low | Disconnected content stays fully interactive and undimmed; nothing marks it stale. | `desktop-1440-light-disconnected.webp` | Claude |

### Accessibility

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| Y1 | High | Re-renders destroy keyboard focus (X2). | live probes | Both |
| Y2 | High | No skip links or shortcuts; Launch is tab stop 218 of 227. | live probe, 12 cards | Claude |
| Y3 | Medium | Launch results and Ask errors reach no live region; nothing announces "a1 answered" or "2 new decisions". | lines 231, 239 | Both |

### Copy

| # | Sev | Finding | Evidence | Found by |
| --- | --- | --- | --- | --- |
| W1 | Medium | Cards show the raw agent instruction ("reply "merge" to merge #81, or reply ...") although the buttons already carry the replies; the card should lead with the question. | `desktop-1440-light-top.webp` | Claude |
| W2 | Low | The Ask label describes the implementation ("A short-lived thread reads the board") instead of the task. | `desktop-1440-light-full.webp` | Sol |

## What works and must survive

- Card diffing by JSON signature, per-card drafts, and focus and caret
  restore. A focused card reply moved 0 px over 25 s of updates.
- One button per parsed reply, plus a free-text reply on every decision.
- Status words on every chip, so color is never the only signal.
- Thread links open the T3 desktop app, with modifier-click and
  `openLinks: "tab"` falling back to the browser.
- The "(n) t3 board" title badge, and notifications only while the tab is
  hidden.
- Tabular numerals for counts and ages, and the local time in each age's
  tooltip.
- Tuned dark-mode tokens, reduced motion respected, a visible focus ring.
- No icons, gradients or emoji, and plain, specific empty-state copy.
- `safeUrl` on every external link and the CSRF header on every POST.
- Per-card busy state and inline errors that keep the input on failure.
- Batch launch with per-issue results, and settling the user can reverse.

## Functionality inventory

A redesign must keep every item. PR 2 checks each one against the seeded
harness.

- [ ] Header: title; service dot (ok, warn, bad, idle) and text (connecting,
      T3 connected, watcher last pass, T3 not connected with its error,
      watcher stale); token expiry tooltip; counts of decisions, open
      tickets, running.
- [ ] A disconnect indication that stays visible until the stream
      reconnects (a banner today); live updates over `/api/events` with reconnect
      backoff to 30 s; initial `/api/state` fetch.
- [ ] Document title carries the decision count.
- [ ] Browser notification for each new card while the tab is hidden;
      permission request.
- [ ] Decisions: count; empty state; card id; ticket label as a thread
      link; kind (gate, blocker, report, escalate, error); age that refreshes
      every 30 s with the local time as tooltip; card text.
- [ ] Card actions: one button per parsed option; free-text reply with Send
      and Enter, blank replies ignored; Dismiss; reply disabled when the card
      has no ticket; Full message, fetched on first open, with loading and
      error text; busy state; inline error; draft kept across updates; focus
      and caret restored.
- [ ] Tickets: open count; "Settle all done", shown when any done ticket or
      closed ask exists; error line.
- [ ] Ticket row: label; status (running, waiting, failed, idle, done,
      launching, unknown, gone); gate tag; "not joined yet"; PR link with
      number and state; thread link; last message (time, kind, text), hidden
      once done; Settle on done tickets.
- [ ] Worker rows under their ticket: label, status, gate tag, PR link,
      thread link.
- [ ] Tickets empty state.
- [ ] Activity: count; time with ISO tooltip; ticket label; one line.
- [ ] Settled: count; filter over ticket label, link, question and answer;
      settled tickets (label, settled time, PR, thread, Restore); settled
      asks (question, thread, Restore, answer); "Nothing matches the filter"
      and "Nothing settled yet".
- [ ] Launch: URLs or `owner/repo#n`, split on whitespace or commas;
      "Launching..." state; per-issue result (thread link, or the error and
      its card id); the textarea clears only when every issue launched.
- [ ] Ask: input, Ask button, error; asks list with question, status
      (running, waiting, answered, failed and the other closed states),
      thread link, answer or "Working on it...", Settle on closed asks.
- [ ] Thread links open in the T3 desktop app, falling back to a browser
      tab; PR links open externally.
- [ ] Light and dark from the system setting; reduced motion; responsive
      wrapping with no horizontal scroll.

## How the pair settled its disagreements

The cross-reads closed most of these. Two stayed split after them, card
height (Claude at most 128 px, Sol 128 to 152 px) and confirm versus undo, and
for those the row gives the answer this synthesis picked.

| Question | Claude first | Sol first | Settled |
| --- | --- | --- | --- |
| Where Launch and Ask live at 1440 | Right rail | 176 px bottom strip | Right rail. A bottom strip takes 20% of the height from both queues. Sol changed position. |
| Single-key shortcuts | A full set | None inside fields | A small set, only when no field has focus, listed in a `?` overlay and possible to turn off. Digits only focus an option. |
| Decision card height | 88-120 px | 144-200 px | Median at most 144 px for a two-line question, with the reply input and Full message disclosed on demand; at least 5 cards fully visible at 1440x900. |
| Running status | Distinct calm hue | Neutral and quiet | Neutral, with its status word. "Needs you" gets an amber rule and a link to its card. |
| Irreversible replies | 5 s undo | Confirm | Deferred send with Undo: the request leaves only when the window ends, and the card says "Sending", never "done". Sol accepts undo when sending really waits. |
| Settled, Done, Activity | Tickets tabs; Activity in the rail | Utility tabs in a third column | Open, Done and Settled tabs inside Tickets; Activity under Launch and Ask in the rail. |
| Arrivals while reading | Hold, with a 10 s cap | Hold, anchor, no cap | Hold behind a reserved "N new" slot with no time cap; update visible rows in place at fixed height; removed items keep a full-height placeholder until the pointer and focus leave. |
| Phone | Bottom tabs | Top tabs | Bottom tabs, with the composer above them; it docks on the keyboard and turns into a full-height sheet when little room is left. |
| Headings | 12 px uppercase | 14/20, 650 | Sentence case, 14/20, weight 600, primary ink, with a muted count. |
| Panel separation | 1 px borders | Background steps | 1 px borders plus one background step; no shadows. |

The settled answers are the basis of [`layouts.md`](layouts.md) and
[`rubric.md`](rubric.md).
