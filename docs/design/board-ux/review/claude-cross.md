# Cross-read (Claude on Sol)

## 1. Their findings

### Accept (I missed or underrated)

1. **Medium - header contradicts the banner.** `desktop-1440-light-disconnected.webp` shows a green "T3 connected" dot under a red "Disconnected" banner. Not a fixture artifact: `setConnected(false)` writes "disconnected" (board.html:556), then the initial `/api/state` fetch (620) calls `render()`, which rewrites dot and text (541-548). Transport freshness and T3 health share one slot.
2. **Medium - Launch results and Ask errors are not announced.** `#launch-out` and `#ask-err` (board.html:231, 239) have no live region. Only `#act-err` and the card errors are alerts.
3. **Medium - repeated controls lack context.** Twelve identical "Send" and "Dismiss" buttons, and reply inputs named only "Reply to a1" (356, 375).
4. **Medium (was low in mine) - phone card density.** a1/a2 in `phone-390-light-top.webp` are about 285 px each, so roughly two cards fit per screen.
5. **Low:** loading lacks `aria-busy` and local text (187, 201); Ask label describes the implementation; passive disclosures get card shadows (100, 165, 171).

### Dispute

1. **Decision cards of 144-200 px.** In their own 652 px queue that is 3 to 4 cards, no better than today's 3.5 (`desktop-1440-light-top.webp`).
2. **A 176 px bottom composer band at 1440x900.** 20% of height taken from both queues for inputs used a few times a session; `claude-code-agent-view.webp` spends one line on dispatch.
3. **Ticket base rows of 64 px.** About 8 tickets in 652 px; use 32 px plus an 18 px clamped note.
4. **Utility as a focus-trapping overlay at 900-1200 px.** Activity hides behind a modal; make it a Tickets tab.
5. **Rubric seed 7, five-second blind trials.** Judges cannot run timed trials; replaced in section 3.
6. **"No unmodified letter shortcuts" (partly).** Agreed inside fields; outside them the references use j/k (`linear-inbox.webp` notes) and digit answers (`claude-code-agent-view.webp`).

## 2. Positions on the open questions

1. **Launch and Ask: right rail.** Launch (a 2-line textarea that grows on focus) and Ask sit fixed at the top of a ~340 px third column. Ask answers sit in a capped block below them, then Activity scrolls. `datadog-oncall-pages.webp` puts its composer at the top of its pane, not at the foot. A rail costs width the queues do not need, while a bottom strip costs height they do.
2. **Shortcuts: yes, a subset, only when no field has focus.** `j`/`k` move between rows or cards, `l` focuses Launch, `/` focuses Ask, `g d`/`g t`/`g a` jump between panels, `r` opens a reply, `?` shows help, and `Esc` returns to the panel. Digits only focus option n; Enter activates it. A single keystroke must never send a merge.
3. **Card density: median 112 px or less, 128 px or less for a two-line question.** Always visible:
   - a header line (id, ticket, kind, age)
   - the question, clamped to 2 lines
   - one row of options followed by a quiet "Reply" and "Dismiss"

   Disclosed: the reply input (expands in place; the focused card stays put) and Full message (a link in the header line).
4. **Running: quiet and neutral.** Use a grey-teal dot with plain text and no pill fill, never the accent hue. Both reviews say running should recede. A "needs you" ticket gets:
   - an amber left rule
   - a "needs you · a1" link that scrolls to and focuses its card

   It does not move to the top. Reordering would break position stability. `zulip-stream-topics.webp` shows the needs-you `@` badge sitting distinct from plain unread.
5. **Irreversible replies: a 5 s undo window, applied uniformly to every option and Send.** The card shows "Sending 'merge' to widgets#41 - Undo" in its own slot, and the POST fires only when the window ends. Confirm dialogs would tax the most common action. No option is styled primary, because the source never marks one (board.html:367 picks by index).
6. **Settled and Activity:** Activity goes in the right rail below Launch and Ask. Settled becomes a tab of the Tickets panel (Open | Done | Settled) with its filter pinned in the panel header. Done tickets leave Open for the Done tab, which carries "Settle all done". This follows the needs-you versus everything-else split in `linear-inbox.webp` and `claude-code-agent-view.webp`. They leave by the removal rule in item 7.
7. **Arrival rule: a combination.**
   - If a panel is at its top and the pointer and focus are outside it, new items insert live with a 1.2 s highlight.
   - Otherwise, new items are held behind an "N new" pill in a reserved 32 px header slot (Sol's idea; it stops the pill itself from causing a shift). The pill names urgent kinds: "3 new · 1 blocker". There is no time cap.
   - Changes to visible rows update in place inside fixed-height slots (notes clamped to one line), so no row changes height.
   - **Removals:** an answered or dismissed card, a done ticket or a removed ask becomes a ghost row that **keeps its full height** ("a1 answered: merge"). It collapses only after the pointer and focus leave the panel. I am correcting my own 28 px ghost: it would still pull the next card up under the cursor.
   - Scroll compensation (anchoring the first visible item) is only a fallback for insertions above the viewport that cannot be avoided.
8. **Phone: tabs at the bottom, with the composer pinned above them.** Bottom tabs are within thumb reach. When the keyboard opens, the tab bar hides and the composer docks on the keyboard via `visualViewport`. If less than ~300 px remains, it becomes Sol's full-height sheet, which restores focus and scroll on close. Touch targets are at least 44 px.
9. **Headings and separation.**
   - Headings use sentence case at 14/20, weight 600, in `--ink`, with a tabular count in `--ink-2`, inside a 40 px panel header with a 1 px bottom border. I am dropping my uppercase 12 px: the references use larger sentence-case section titles ("Description", "Timeline" in `datadog-oncall-pages.webp`; "Activity" in `linear-issue-agent-panel.webp`).
   - Panels are separated by 1 px borders (at least 1.3:1 against the panel) plus one background step between canvas and panel. Use no shadows; they vanish in `desktop-1440-dark-top.webp`.

## 3. Joint rubric changes

1. **Merge** my 1 with their 1 (no page scroll), my 3 with their 3 (zero drift; add caret and value unchanged), and my 5 with their 5 (focus survives, never BODY).
2. **Merge** my 4 with their 4 (pointer stability), and add removals. Test: hover a card option, answer the card above it, then sample `elementFromPoint` for 10 s. It must return the same element or its full-height ghost.
3. **Merge** my 6 with their 6. Test: no insertion above the reader, independent scrolling, and tab switches that restore each panel's offset.
4. **Replace** their 7 with **Priority and linkage**. Test: blockers and errors sort above gates and reports and are distinguishable in grayscale, and every needs-you ticket row links to its card in one activation.
5. **Change** my 11 to **Safe options**. Test: reorder the options in a fixture and check that none turns primary. Click an option and check that no network POST happens for at least 5 s and that Undo cancels it.
6. **Add Connection honesty.** Test: abort `/api/events` while `/api/state` succeeds (the `capture-states.js` path). The header must never show healthy, stale content must be marked, and no panel or composer may move when the banner appears (top deltas of 0 px).
7. **Add Announcements.** Test: Launch results, Ask errors, card errors and "N new decisions" each reach a live region exactly once, and feed rows are never announced.
8. **Add Contextual names.** Test: the accessible name of every option, Send, Dismiss, Settle and Restore control includes its card id or ticket label (check with axe or the accessibility tree).
9. **Change** my 7 to **Decision density**. Test: at 1440x900 with seeded data, at least 5 cards are fully visible and the median card height is 128 px or less. On a 390 px phone, at least 2.5 cards are visible.
10. **Merge** their 10 into my 12 (narrow fallback): add 44 px touch targets, no clipped controls, and a composer that stays visible with the keyboard open.
11. **Add Type audit.** Test: computed styles use at most 4 font sizes and 3 weights, every spacing value comes from {4, 8, 12, 16, 24}, and no raw `**` reaches the page. This sharpens my 15 and their 12.
12. **Keep** my 2, 8, 9, 13 and one parity pass (my 14 = their 13).
