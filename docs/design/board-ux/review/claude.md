# t3 board design review (Claude)

Evidence sources: screenshots in `../current/` (file names below), `../current/report.json`, `src/board.html` line numbers, and my own live probes on the running board (1440x900 and 390x844, session `review-claude`, 2026-11-17 about 23:38 MST).

## 1. Verdict

1. **Nothing has a stable position.** The page is 2,345 px tall at 1440 (5,323 px with Activity and Settled open, 6,263 px on a phone); the Launch textarea starts 2.22 screens down (6.55 on a phone), and while focused it drifted 214 px in 36 s of live updates (`report.json`).
2. **Live re-rendering destroys what the user is touching.** The ticket tree, asks, feed and settled list are rebuilt with `replaceChildren` on every update (board.html:528-534); a focused ticket link was detached and focus fell to `<body>` within 9 s in my probe. Removing an answered card slides the next card's buttons under the pointer.
3. **Decisions are the job, but they are not prioritized or dense.** Every card is 156-221 px of equal-weight chrome, sorted by age only; a blocker looks like a gate except for one 13 px red word; only about 3.5 cards fit above the fold.
4. **Status color is overloaded.** The same blue means running, primary action, link and id badge; chip text fails 4.5:1 in light mode for waiting, idle and done.

## 2. Findings

### Layout

1. **High.** Single-document flow, only the header sticky (board.html:67-68, 85-93); every section's position depends on content above it (`desktop-1440-light-full.webp`, Launch/Ask at y=2001).
2. **High.** Launch and Ask are the last grid items (board.html:224-243). `report.json` `launchBelowFold.screensDown` 2.22 desktop, 6.55 phone. Launching is the second most frequent action and it is the hardest to reach.
3. **Medium.** `main { max-width: 1280px; margin: 0 auto }` (board.html:88) leaves 80 px dead gutters at 1440 (`desktop-1440-light-top.webp`, x 0-96 and 1344-1440) while the content is cramped vertically.
4. **Medium.** The disconnect banner sits outside the sticky header in normal flow (board.html:197). In `desktop-1440-light-disconnected.webp` it pushes all content down 35 px (Decisions h2 at y 103 vs 68), and once the user has scrolled it is off screen: the one state that says "this data is stale" is invisible exactly when the user is deep in the page.
5. **Medium.** At 1024 the ticket `.links` group wraps onto its own line for gated tickets (`tablet-1024-light-top.webp`, widgets#41 and ledger#198: "PR #81 open thread" on a second line), so row height varies 66 to 118 px for the same information.

### Hierarchy

1. **High.** Decisions are ordered by age only (oldest first) and all cards share one visual weight. In `desktop-1440-light-full.webp` the a6 blocker (relay#11, "worker failed twice") is distinguished from gates only by the 13 px red word "blocker" (board.html:112); a report (a7) has no color at all.
2. **High.** The first parsed option is always the primary button (board.html:367: `i === 0 ? " primary"`). On a7 that makes **stop** (end a 3 h run) the filled blue default, and on a1 it makes **merge** primary over "merge and deploy". Primacy should come from the source, not list position; a destructive choice should never be the visually suggested one.
3. **Medium.** A ticket that needs the user is not marked as such in the Tickets list. widgets#41 shows "gate: merge" in amber, identical in hue to a plain "waiting" chip (board.html:159, 162), and nothing links the ticket row to card a1 (no "a1" reference, no jump).
4. **Medium.** The Tickets h2 sits 8 px below the Decisions h2 (`desktop-1440-light-top.webp`, y 76 vs 68) because the 34 px "Settle all done" button is inside it (board.html:204-205); the heading's accessible name becomes "Tickets 23 open Settle all done".

### Scroll behavior

1. **High.** One scroll container: comparing a decision with its ticket, or reaching Launch, scrolls everything away together.
2. **High.** Activity is newest-first and re-rendered whole (board.html:530-531). With the feed open, every new event inserts at the top and pushes every row the user is reading down by one row (about 27 px, `desktop-1440-light-expanded-full.webp` row pitch).
3. **Medium.** The expanded Settled list has no height limit; 26 rows at 44 px each (`desktop-1440-light-expanded-full.webp`) and it will grow without bound. Its filter input scrolls away with the list header.

### Layout shift

1. **High.** Focused Launch textarea moved 556 → 557 → 604 → 770 px top in 36 s (`report.json` `focusedInputDrift`, 214 px total) as cards (7 → 9) and tickets arrived above it.
2. **High.** Focus loss on the ticket tree: I focused the first `#tree a`, waited 9 s; the element was detached (`isConnected: false`) and `document.activeElement` was `BODY`. Cause: `replaceChildren` of the whole tree every update (board.html:528). Same for ask Settle (533) and settled Restore (459). Any keyboard user tabbing through tickets is thrown back to the top.
3. **High.** Element under the pointer moves. The gizmo#91 row (its Settle button) moved 1834 → 1914 → 1877 px document-y over 40 s in my probe as tickets were inserted above it. And answering or dismissing a card removes it (board.html:500), so the next card's option buttons slide up under the cursor; a double-click on "merge" can land on the next card's primary option.
4. **Done well, keep:** a focused card reply survives updates with zero drift (`report.json` `cardReplyDrift`, top 226 for 10 samples) thanks to signature diffing and focus/selection restore (board.html:473-513).

### Density

1. **High.** A decision card is 156-221 px tall (probe: 221, 199, 221, 176, 220, 221, 156...). About 110 px of each is always-visible chrome: a full-width reply input, Send, Dismiss and "Full message mNN". At 1440x900 only a1-a4 are visible (`desktop-1440-light-top.webp`).
2. **Medium.** Ticket `t-last` lines repeat the card text verbatim, including "**ACTION NEEDED:**" (`desktop-1440-light-top.webp`, widgets#41 and ledger#198 rows), adding two to three lines per gated ticket for information already in Decisions.
3. **Medium.** Done ticket rows are 57 px against 43 px for a one-line running row (probe `rowHeights`) only because the Settle button has `min-height: 34px` (board.html:119); done items are the least important and take the most height per line.

### Typography

1. **Medium.** Body is 15 px (board.html:59) while section headings are 13 px grey (board.html:95). The section labels are smaller and lighter than the content they label, so "Decisions" and "Tickets" do not register as landmarks in `desktop-1440-light-top.webp`.
2. **Medium.** Card text is 15 px, ticket labels 15 px bold, ticket notes 13 px, worker rows 14 px, chips 12 px, ids 12 px mono, feed 13 px: seven sizes/weights within 600 px.
3. **Low.** Raw Markdown leaks (`**ACTION NEEDED:**`, board.html:417) and notes are cut mid-word with no ellipsis ("https://githu", "partial migra" in `desktop-1440-light-full.webp`). Activity repeats "MST" on all 50 rows.

### Color

1. **High.** Blue carries four meanings: running chip (`--run` = `--accent`, board.html:19), primary buttons, links, and the id badge (board.html:108-109). In `desktop-1440-light-top.webp` the eye cannot separate "this is running" from "click this".
2. **High.** Chip text contrast below 4.5:1 for 12 px text (computed from tokens): waiting `#a86206` on `#fbf0dd` 4.22, idle `#6b7077` on `#eeeee9` 4.29, done `#2f7a4b` on `#e2f1e7` 4.49. (dark passes).
3. **Medium.** Input and divider borders `#dcdcd5` on white are 1.38:1, failing WCAG 1.4.11 (3:1) for input boundaries; the reply inputs in `desktop-1440-light-top.webp` read as grey bars, not fields. Panel vs page is 1.08:1 light and 1.10:1 dark; panels exist only by shadow, which is invisible in dark (`desktop-1440-dark-top.webp`).
4. **Medium.** Amber means waiting, launching, gate kind and gate tag (board.html:113, 159, 162); red means failed, gone, blocker, error, escalate and the disconnect banner. "gate" (needs you) and "waiting" (agent idle on CI) look identical.

### Affordances

1. **High.** Option buttons send immediately with no confirmation or undo (board.html:367, 355); "merge and deploy" and "stop" are one click, next to a reply input, and subject to the pointer-shift problem above.
2. **Medium.** Reply forms disable on cards without a ticket but still render the full input and Send (board.html:356, 375); disabled controls take space and look broken.
3. **Medium.** On the phone, Dismiss wraps onto its own line 44 px below Send (`phone-390-light-top.webp`, probe), floating without context.
4. **Low.** The first click anywhere triggers the browser notification permission prompt (board.html:577-579), including a click on "merge". The prompt should come from an explicit "Notify me" control.

### Empty and loading states

1. **Medium.** Loading (`desktop-1440-light-loading.webp`): one blank 64 px block per section (board.html:187), no text, and Decisions' block is a different shade from Tickets'. Header says only "connecting". Launch and Ask are live while the board has no state.
2. **Medium.** Empty (`desktop-1440-light-empty.webp`): the two empty panels have different heights (77 vs 113 px) and the text is not vertically centered in the Tickets one. Copy "Launch one below" is positional (board.html:529) and false once Launch moves.
3. **Low.** Disconnected (`desktop-1440-light-disconnected.webp`): content stays fully interactive and un-dimmed; nothing marks it stale or blocks answer buttons that will fail.

### Accessibility

1. **High.** Keyboard focus destroyed by re-render (Layout shift 2).
2. **High.** Tab order: 227 focusable elements, the Launch textarea is number 218 (probe, 12 cards). No skip links, no shortcuts.
3. **Medium.** 15+ empty `role="alert"` paragraphs (one per card plus act-err, board.html:206, 342). Fine when empty, but a card error and a global error can fire together; and card removal on success is silent: there is no polite live region saying "a1 answered" or "2 new decisions".

### Copy

1. **Medium.** Card text is the raw agent instruction ("reply "merge" to merge #81, or reply "merge and deploy" to ship it too, here: https://...") while the buttons already encode the replies; the card should lead with the question ("Merge PR #81?") and keep the raw text in Full message.

## 3. What works and must survive

- Card diffing by JSON signature, draft persistence per card, and focus plus selection restore (board.html:473-513): a card reply does not move or lose text (0 px drift).
- One button per parsed reply option plus free-text reply on every decision.
- Thread links open the T3 desktop app, with modifier-click and `openLinks: "tab"` falling back to the browser (board.html:277-301).
- Title badge "(n) t3 board" and notifications only when the tab is hidden (board.html:463-471, 550).
- Server-side status words on chips (text, not only color); tabular numerals in counts and ages; `title` on age with local time.
- Dark mode via `prefers-color-scheme` with tuned tokens; reduced-motion respected; visible `:focus-visible` ring.
- No icons, no gradients, no emoji; plain, specific empty-state copy. Restrained palette tokens.
- `safeUrl` for every external link, CSRF header on every POST.
- Per-card busy state and inline error, retaining the input on failure.

## 4. Functionality inventory

- [ ] Header: title, service dot (ok / warn / bad / idle), status text (T3 connected, watcher last pass, T3 not connected + error, watcher stale), token expiry tooltip, counts (decisions, tickets, running)
- [ ] Disconnected banner; SSE reconnect with backoff to 30 s; initial `/api/state` fetch
- [ ] Document title with decision count
- [ ] Browser notification for new cards while hidden; permission request
- [ ] Decisions: count; empty state; card id, ticket label as thread link, kind (gate / blocker / error / escalate / report...), age (refreshes every 30 s, tooltip local time), text
- [ ] Card option buttons (first primary), free-text reply + Send, Dismiss, disabled reply when no ticket, Full message lazy-load by handle, busy state, inline error, draft kept across updates, focus restore
- [ ] Tickets: open count, Settle all done (shown when any done ticket or closed ask), error line
- [ ] Ticket row: label, status chip (8 statuses), gate tag, "not joined yet", PR link with number and state, thread link, last message (time, kind, head, hidden when done), Settle for done
- [ ] Worker rows: label, chip, gate tag, PR link, thread link
- [ ] Tickets empty state
- [ ] Activity: collapsible, count, time with ISO tooltip, ticket label, head line
- [ ] Settled: collapsible, count, filter (label, link, question, answer), settled tickets (label, done chip, settled time, PR, thread, Restore), settled asks (question, thread, Restore, answer), "Nothing matches" and "Nothing settled yet"
- [ ] Launch: textarea (URLs or owner/repo#n, split on whitespace or comma), Launch button with "Launching..." state, per-issue result (Launched + thread link, or error + card id), clears on full success
- [ ] Ask: input, Ask button, error, asks list (question, status chip running / waiting / done / failed, thread link, answer or "Working on it...", Settle for closed asks)
- [ ] Thread links open in T3 desktop app with browser fallback

## 5. Proposed layouts

### Layout A (recommended): fixed three-pane shell with a pinned command rail

1440x900:

```
+--------------------------------------------------------------------------------+
| t3 board  * connected · watcher 23:35     12 decisions  23 tickets  12 running |  40px header (banner replaces status text, no insert)
+-----------------------------+-------------------------------+------------------+
| DECISIONS 12   [blockers 1] | TICKETS 23  [Open|Done|Settled]| LAUNCH           |
|-----------------------------|-------------------------------| [textarea 3 rows]|
|! a6 relay#11 blocker   44m  | widgets#41  running  gate a1 > |        [Launch]  |
|  Worker failed twice...     |   PR1/2 idle   PR2/2 running  | results...       |
|  [Reply............][Send]  | gizmo#12    waiting           |------------------|
|  a1 widgets#41 gate    58m  | ledger#198  running  gate a4 >| ASK              |
|  Merge PR #81?              | relay#11    failed   a6 >     | [question] [Ask] |
|  [merge][merge+deploy]  ... | ...                           | answers (scroll) |
|  a2 relay#9 gate       55m  |              (scrolls)        |------------------|
|  ...  (scrolls)             |                               | ACTIVITY 50      |
|        [3 new ^]            |        [2 new ^]              | 23:35 gizmo#15...|
|                             |                               |  (scrolls)       |
+-----------------------------+-------------------------------+------------------+
  ~460px                         flex                            ~340px
```

- Page: `height: 100dvh; overflow: hidden`. Each of the three columns scrolls; the right rail is split: Launch and Ask fixed at top (never scroll), Activity scrolls below. Ask answers scroll in a capped block (max 30% of rail).
- Settled becomes a tab of Tickets (Open / Done / Settled), with its filter pinned in the panel header. Done moves out of Open; "Settle all done" lives in the Done tab header.
- Arrival rules: new items go to the end that is not being read. If a panel is scrolled away from its top, new rows are held and a "3 new" pill appears; nothing inserts above the viewport (or, if it must, compensate `scrollTop` by the inserted height). If the pointer is inside a panel or focus is inside it, defer reflow of that panel until pointerleave/blur (max 10 s). Answered/dismissed cards collapse to a 28 px "a1 answered: merge" ghost that holds its slot until the pointer leaves the panel, then fades.
- Keyed DOM reconciliation for tickets, workers, feed, asks, settled (like cards today), so focus never lands on `body`.
- Keyboard: `g d` / `g t` / `g a` focus Decisions / Tickets / Activity; `l` focuses Launch, `/` Ask, `f` the Settled filter (only when no input has focus); `j`/`k` move between cards or ticket rows (roving tabindex, one tab stop per panel); on a focused card `1`-`9` focus option n, `r` focus reply, `Enter` activates; `Esc` returns to the panel. Panels are `section` regions with headings; skip link "Jump to Launch".
- 900-1200: two columns (Decisions | Tickets). The rail becomes a bottom-docked 56 px command bar spanning both columns with a Launch / Ask switch and the input; Activity becomes a fourth tab in the Tickets panel.

390 wide:

```
+------------------------------+
| t3 board  *     12 · 23 · 12 |  44px, one line
+------------------------------+
| DECISIONS 12                 |
| ! a6 relay#11 blocker   44m  |
|   Worker failed twice...     |
|   [Reply.........] [Send]    |
| a1 widgets#41 gate     58m   |
|   Merge PR #81?              |
|   [merge] [merge+deploy] ... |
|   (panel scrolls)            |
+------------------------------+
| [+ Launch or ask...........] |  pinned composer, opens a sheet
+------------------------------+
| Decisions12|Tickets|Activity|More |  bottom tabs, 56px
+------------------------------+
```

One panel at a time, tab state in the URL hash; badges on tabs; no horizontal scroll; the composer sits above the tab bar and rises with the on-screen keyboard (`env(keyboard-inset-height)` / visualViewport).

### Layout B: queue-first with a fixed composer (Slack-style)

Two scrolling columns (Decisions | Tickets with tabs Open / Done / Settled / Activity) and a 64 px full-width composer pinned to the bottom with a Launch / Ask switch; it grows to 4 lines while typing and Ask answers open in a popover above it. Same arrival and keyboard rules as A. Cheaper and identical at 900-1200; weaker because Activity loses its always-visible column and answers are transient. 390: same as Layout A.

## 6. Visual direction

- **Type**: system UI. 12 / 13 / 14 / 16 px. Body and card text 14/20 regular; ticket label 14/20 600; meta (age, notes, activity) 13/18 regular; chips and tags 12/16 600; panel headings 12/16 600 uppercase tracking 0.04em in `--ink` (not grey) with count in `--ink-2`; header title 15 600. Mono 12.5 only for ids and times. Three weights only: 400, 500, 600.
- **Spacing**: 4 px base; use 4, 8, 12, 16, 24 only. Panel padding 12 horizontal; gutters between panes 1 px borders, not gaps.
- **Density**: ticket row 32 px single line, +18 px for the note line (one line, ellipsis); worker row 28 px; activity row 24 px; settled row 32 px; decision card target 88-120 px (header 20, question 1-2 lines, actions 32). Free-text reply collapsed to a 28 px "Reply" control that expands in place on focus (expanding the focused card is allowed; the card below moves, not the focused one). Buttons 28 px in rows, 32 px in cards, never below 24x24 targets.
- **Color roles**: separate interaction from status. Accent (links, primary, focus) one hue; statuses five roles, each dot + text, no filled pills: running blue-teal distinct from accent, needs-you amber (gate, waiting on human), failed / gone red, done green, idle / launching / unknown grey. "waiting on CI" is grey, not amber. A ticket with an open decision gets an amber left rule and a card-id link. All status text at least 4.5:1 on its background; input borders at least 3:1 (e.g. `#8c9096` on white); panel vs page separation by 1 px border at at least 1.3:1. Dark: same roles, borders `#3d434a`, no shadows.
- **Motion**: new item highlight is a 1.2 s background fade from accent-soft to transparent, no slide or height animation; collapse of answered cards 150 ms opacity only after the pointer leaves; none under reduced motion.
- **Remove**: drop shadows on every card and panel (`--shadow`), 12 px radii (use 6), header blur, pill backgrounds on chips, underlines on repeated "thread"/PR links (underline on hover/focus), the always-visible "Full message mNN" label (becomes "Full message" in a card overflow), raw `**` markdown, duplicated gate text in ticket notes, per-row "MST" in Activity.

## 7. Rubric seeds

1. **Page does not scroll** at 1440x900, 1280x800 and 1024x768: `document.scrollingElement.scrollHeight <= innerHeight` with 25 tickets, 12 cards, Activity 50.
2. **Inputs reachable**: Launch and Ask inputs have `getBoundingClientRect()` fully inside the viewport at all three desktop sizes in every panel scroll state, and are reachable in at most 1 key (shortcut) or 3 tabs from page load.
3. **Focused input drift 0 px**: focus Launch, Ask, a card reply and the Settled filter; sample `top` every 500 ms for 60 s of live updates; max delta 0.
4. **Pointer target stability**: hover a ticket row Settle and a card option mid-panel; over 60 s of updates the element under `elementFromPoint` at that point stays the same element.
5. **Focus survives updates**: focus a ticket thread link, a worker link, an ask Settle and a settled Restore; after 30 s `document.activeElement` is the same connected node.
6. **No insertion above the reader**: scroll Tickets and Activity to mid-list; after 60 s the first visible row is the same item; new items are announced by a "n new" pill.
7. **Decision density**: at 1440x900, at least 6 decision cards fully visible with seeded data; median card height at most 120 px.
8. **Ticket density**: at least 14 ticket rows (with notes) visible at 1440x900 without scrolling the Tickets panel.
9. **Status legibility**: every status text and chip at least 4.5:1, input borders at least 3:1, in light and dark (computed from tokens); running and accent are different hues (delta-E at least 20).
10. **Priority**: blockers and errors sort above gates and reports, and are distinguishable in grayscale (shape or label, not hue only).
11. **Primary option safety**: no option is styled primary unless the source marks it; one-click irreversible actions (merge, deploy, stop) offer undo or confirm.
12. **Narrow fallback**: at 390x844 and 768x1024, `scrollWidth === innerWidth`, one panel visible at a time, composer and tab bar pinned, Launch reachable in 1 tap.
13. **Keyboard parity**: every inventory capability (section 4) operable by keyboard only; tab stops before Launch at most 10; panel headings navigable as regions.
14. **Parity checklist**: every item in section 4 present and working (scripted pass).
15. **No slop**: zero decorative gradients, emoji, or icons without a job; spacing values all from the 4/8/12/16/24 set (CSS audit); no element whose removal loses no information.
