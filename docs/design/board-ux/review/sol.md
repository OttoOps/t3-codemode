# Verdict

The largest failure is spatial instability: control positions depend on queue length, and Launch moved 214 px during the supplied typing measurement. Second, controls needed to direct agents are below monitoring and archive content, especially on phones. Third, the visual hierarchy promotes arbitrary reply options and routine running states while making the workspace headings and blocked work comparatively quiet. The board needs a restrained operations workspace with fixed input locations, aligned records, and explicit attention states.

Review basis: I opened all 12 supplied PNGs, read the HTML and relevant snapshot definitions, inspected the live board, and opened then closed a full message. A live keyboard-focus probe found a ticket PR link replaced by an update, leaving focus on BODY. No Launch, Ask, reply, Dismiss, Settle, or Restore action was submitted; the review tab was closed. This is the independent taste/style assessment requested by the brief.

# Findings

## Layout

1. **High.** Inputs occupy the final grid row after Activity and Settled. [Desktop full](../current/desktop-1440-light-full.webp), bottom; [measurements](../current/report.json) put Launch at document y=2001, or 2.22 viewport heights from the top. Starting work requires leaving the monitoring context.
2. **High.** Below 900 px, Decisions precede all Tickets. [Phone full](../current/phone-390-light-full.webp) is 6263 px tall; Launch is 6.55 screens down. This preserves wrapping but removes useful navigation between agent supervision and control.

## Hierarchy

1. **High.** The first parsed reply gets primary styling solely because its index is zero ([board.html:366](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L366)). In [desktop full](../current/desktop-1440-light-full.webp), a7's solid-blue "stop" appears recommended, just like a1's "merge." The interface implies judgment that the agent's word order does not justify.
2. **Medium.** Section headings are muted 13 px, smaller than 15 px task text and 16 px branding ([board.html:75](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L75), [board.html:94](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L94)). [Desktop top](../current/desktop-1440-light-top.webp), top band, has weak workspace boundaries. The eye follows repeated controls before learning where each queue lives.

## Scroll behavior

1. **High.** Expanding history changes page height from 2345 to 5323 px ([report](../current/report.json); [expanded full](../current/desktop-1440-light-expanded-full.webp), Activity through footer). Reading history displaces live controls by thousands of pixels; those surfaces need independent scroll ownership.
2. **Medium.** Only the global header sticks ([board.html:67](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L67)). In [typing Launch](../current/desktop-1440-light-typing-launch.webp), queue headings are gone and the header cuts across a partial card. Counts alone cannot orient the operator within long queues.

## Layout shift

1. **High.** Launch's focused top changes from 556 to 770 px in the supplied samples, a 214 px range ([report](../current/report.json)). Card growth changes the shared grid row above it. Preserve the separate positive result: the sampled reply stayed at 226 px with its draft intact; this is not evidence that every input drifts.
2. **Medium.** The disconnect banner enters document flow ([board.html:83](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L83)); [disconnected screenshot](../current/desktop-1440-light-disconnected.webp), top, shifts panel starts down roughly 35 px. Connection changes must occupy reserved chrome space rather than relocate controls.

## Density

1. **Medium.** Each decision repeats an option row, full-width reply, Send, Dismiss, and disclosure. [Phone top](../current/phone-390-light-top.webp), a1/a2, wraps Dismiss onto its own line and makes each card about 285 px tall. Very few decisions fit in view. Progressive reply disclosure can reduce repetition while keeping options directly available.
2. **Medium.** [Tablet top](../current/tablet-1024-light-top.webp), widgets#41 and ledger#198, wraps PR links below title/status and worker labels. The unstructured flex rows ([board.html:148](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L148)) produce changing scan paths. Defined title, status, and link slots would make agent comparison faster.

## Typography

1. **Medium.** [Desktop top](../current/desktop-1440-light-top.webp), widgets#41 note, exposes literal `**ACTION NEEDED:**` and ends its excerpt at `https://githu`. This is source text, not a composed summary. It obscures the actual condition and makes the board feel unfinished.
2. **Low.** The service line, panel headings, note text, and navigation links all use 13 px ([board.html:76](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L76), [board.html:95](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L95), [board.html:150](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L150), [board.html:163](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L163)). Similar sizing leaves weight and color to perform nearly all grouping. Distinguish panel labels structurally; keep operational notes readable.

## Color

1. **Medium.** Blue encodes IDs, links, running chips, and primary replies ([desktop top](../current/desktop-1440-light-top.webp), both columns). Routine running workers create repeated blue islands that compete with human decisions. Make normal running quieter and reserve filled accent for deliberate actions or selection.
2. **High.** The declared light-theme idle pair `#6b7077` on `#eeeee9` is 4.29:1; waiting `#a86206` on `#fbf0dd` is 4.22:1 ([board.html:12](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L12), [board.html:20](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L20), [board.html:153](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L153)). Calculated with WCAG sRGB relative luminance, both miss 4.5:1 for these 12 px labels. Status readability is essential here.

## Affordances

1. **Medium.** Send and Dismiss repeat without ticket context in their accessible names; reply inputs are named only by card ID ([board.html:356](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L356), [board.html:375](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L375)). Repeated controls make keyboard and assistive navigation harder to associate with the intended agent.
2. **Low.** Every container gets the same rounded, shadowed treatment, including archive disclosure bars ([board.html:100](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L100), [board.html:165](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L165), [board.html:171](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L171)). [Expanded full](../current/desktop-1440-light-expanded-full.webp) gives passive history the visual weight of decision work. Flatten history and retain stronger boundaries for actionable content.

## Empty and loading states

1. **Medium.** [Loading](../current/desktop-1440-light-loading.webp) shows anonymous 64 px slabs for queues while Launch and Ask already look ready. No region has `aria-busy` or local loading text ([board.html:187](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L187), [board.html:201](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L201)). Distinguish an incomplete snapshot from an empty queue and communicate action readiness.
2. **Low.** [Empty](../current/desktop-1440-light-empty.webp), Tickets, says "Launch one below" without a control that takes focus there ([board.html:529](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L529)). Empty-state help should point to a stable, immediately reachable Launch input.

## Accessibility

1. **High.** Live probe: focused the first ticket PR link; after a snapshot its old node was disconnected and focus was BODY. [board.html:528](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L528) replaces the entire ticket tree; asks and archive rows are also replaced. Preserve keyed controls, keyboard position, and selection across updates.
2. **Medium.** Card errors and bulk-action errors have alert roles, but Ask errors and Launch results do not ([board.html:231](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L231), [board.html:239](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L239)). Add concise polite announcements for results and explicit error announcements, without reading out every feed update.

## Copy

1. **Medium.** [Disconnected](../current/desktop-1440-light-disconnected.webp), header, simultaneously shows green "T3 connected" and the red retry banner. The captured state contradicts itself. Present transport freshness and T3/watcher health distinctly; do not infer from this fixture that production always has the same contradiction.
2. **Low.** [Desktop full](../current/desktop-1440-light-full.webp), Ask label, explains the short-lived thread implementation instead of naming the task. Use "Question about tickets and decisions"; retain thread details in secondary help. Preserve precise technical terms such as PR, gate, and worker.

# What works and must survive

- Legible system type, neutral surfaces, no decorative imagery, gradients, or emoji. The [dark capture](../current/desktop-1440-dark-top.webp) has a coherent adapted palette rather than inverted light colors.
- Text labels accompany status color; worker indentation explains parentage; PR and thread links are direct.
- Native disclosures keep full messages and history optional. Inline option buttons and custom replies support different decisions.
- The [focused reply capture](../current/desktop-1440-light-typing-card.webp) has a clear focus treatment. Draft and caret preservation already exists for cards.
- Tab-title counts, hidden-tab notifications, explicit service health, batch launch, and reversible settlement are useful operations behavior.

# Functionality inventory

Source contract from [board.html](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L192), including handlers below the markup:

- [ ] Service dot/text for connecting, connected, T3 error, stale watcher; last-pass and token-expiry detail.
- [ ] Decision, ticket, running counts; decision count in browser title.
- [ ] Live snapshots, initial fetch, disconnect banner, automatic retry with backoff.
- [ ] Decision ID, linked ticket label, kind, relative age, exact local-time tooltip.
- [ ] Every parsed reply option sends its original text immediately.
- [ ] Custom reply via Send or form Enter; trim blank submissions.
- [ ] Draft retention, focus and caret restoration; disable replies for cards lacking tickets.
- [ ] Dismiss card; in-flight disabled/busy treatment; inline failure recovery.
- [ ] Expand full message; lazy fetch, loading, error/not-found body, bounded scrolling.
- [ ] Ticket labels, all status strings, gate tag, not-joined state, latest timed message.
- [ ] Worker labels/status/gates nested under their ticket; PR number/state and thread links at both levels.
- [ ] Settle finished tickets; conditional Settle all done covers eligible tickets and asks.
- [ ] Activity disclosure/count; time with ISO tooltip, ticket label, event text.
- [ ] Settled disclosure/count; tickets and asks, settlement time for tickets, retained answers.
- [ ] Case-insensitive archive filter: ticket label/link or ask question/answer; no-match and empty feedback.
- [ ] Restore archived tickets and asks; retained PR/thread links.
- [ ] Batch launch via issue URL or owner/repo#n; whitespace/comma separation; progress and per-item results/errors, thread links; clear only on all-success.
- [ ] Ask submission; running/waiting/answered/failed display, answer or pending text, thread link, Settle when closed; input/error recovery.
- [ ] T3 desktop thread opening with browser fallback; configured tab mode and modified-click behavior; external PR links.
- [ ] New-card notifications only after initial state, while hidden and permission granted; permission requested on first click.
- [ ] Automatic age refresh, system light/dark, responsive wrapping, reduced-motion support, labels/landmarks/focus states.

# Proposed layouts

## Preferred: fixed queues, quiet utility rail, pinned inputs

1440 x 900. Header 48 px; 12 px gutters; queue area 652 px; bottom composer area 176 px.

```text
+---------------------------------------------------------------+
| t3 board | service / freshness          counts     Launch  Ask |
+--------------------+------------------------+-----------------+
| Decisions (n)      | Tickets (n)   Settle all| Activity|Settled |
| fixed new-item slot| fixed new-item slot    | Answers         |
|                    |                        |                 |
| decision cards     | ticket + worker rows   | active utility  |
| own scroll         | own scroll             | own scroll      |
|                    |                        |                 |
+--------------------+------------------------+-----------------+
| Launch: issues textarea + results           | Ask: question   |
|                               [Launch]      |           [Ask] |
+--------------------------------------------+------------------+
```

Use approximately 460/590/330 px columns with remaining width for gutters. Launch spans the first two columns; Ask sits below the utility rail. Both composers have fixed outer bounds; launch results and answer history scroll separately. Full messages expand only inside Decisions. Activity and Settled retain separate scroll positions in utility tabs; Answers holds ask history.

390 wide:

```text
+-----------------------------------+
| t3 board       service    counts   |
| Decisions | Tickets | History      |
+-----------------------------------+
| fixed new-item slot               |
| active panel, own vertical scroll |
| History: Activity/Settled/Answers  |
+-----------------------------------+
| Launch | Ask                      |
| active composer input             |
|                          [Submit] |
+-----------------------------------+
```

Both composer tabs stay visible and are one tap away; drafts survive switching. The active composer docks above the software keyboard using the visual viewport, with safe-area padding. At short viewport heights it becomes a focus-managed full-height composer sheet; closing returns to the previous row and scroll position.

At 900-1200 px use two columns, Decisions 44% and Tickets 56%. Utility content moves to a named overlay launched from the fixed header; it owns its scroll, traps focus, and restores focus on close. The bottom 176 px contains two equal-width composers. Below 900 px use the phone tab structure; never squeeze three columns.

Arrival contract: reserve a 32 px update slot in each panel header. Queue new decisions/tickets behind "N new" until accepted; reconcile existing data by ID. Do not reorder or resize visible records during hover, focus, or text selection. Freeze their height and expose changed notes in detail until interaction ends. Preserve the top visible record and pixel offset when applying changes. Removed active records remain as disabled "Resolved elsewhere" rows until focus/pointer leaves. Activity follows only when already at its live edge and inactive; otherwise buffer with a count. No automatic focus changes.

Keyboard: skip links to Decisions, Tickets, utility, Launch, Ask; Tab follows visual order. Tab widgets use arrow keys and roving focus. Native Enter/Space activate buttons/disclosures; Enter submits single-line Ask/reply; Ctrl/Cmd+Enter submits multiline Launch. Escape closes sheets and restores the invoker. No unmodified letter shortcuts inside fields.

# Visual direction

Use an operations console with flat panel divisions and precise alignment. Remove per-card drop shadows, header blur, filled ID badges, automatic first-option emphasis, and pill backgrounds for normal running. Avoid new decorative icons, status ornaments, gradients, and congratulatory filler.

- Type: system sans. App title 16/24 px, weight 650; panel titles 14/20, 650; ticket labels 14/20, 600; action text 15/22, 450; metadata 13/18, 400; status 12/18, 600. Monospace only handles, hashes, and code. Tabular numerals for times/counts. Phone inputs 16/24.
- Spacing: 4/8/12/16/24 px. Panel headers 40 px; ticket base rows 64 px, worker rows 28 px; activity 32 px; settled 44 px. Decision cards roughly 144-200 px with options visible and custom reply disclosed on demand. Preserve full text access. Phone actions at least 44 px high.
- Color: one blue accent; amber for human waiting/gates; red for failures; green for completed/healthy; neutral for idle/unknown. Preserve distinct status words even when colors match. Target text contrast at least 4.5:1 and meaningful control boundaries/focus 3:1.
- Light: canvas #f6f6f3, panel #fff, ink #1d1f22. Dark: canvas #15171a, panel #1d2024, ink #e7e8ea. Dark mode retains semantic roles, softened fills, and visible separators. Choose darker light-theme status text to fix measured failures.
- Motion: 120 ms color/opacity feedback only; no insertion, reordering, or height animation. Reduced-motion removes transitions. Announce new-work counts once, politely.

# Rubric seeds

1. Desktop shell stays fixed: at 1440x900, 1200x800, and 1024x768, assert document scrollHeight equals clientHeight with 30 tickets and 15 decisions.
2. Reachable inputs: Launch and Ask remain visible on desktop after scrolling each panel; on 390 px each is one tap away, including with keyboard open.
3. Input stability: sample focused Launch, Ask, and reply rectangles for 60 seconds of updates; top/left displacement is 0 px, value/caret unchanged.
4. Pointer stability: hover a middle-row action while inserting/removing records above it; its rectangle and item identity stay unchanged until interaction ends.
5. Keyboard continuity: focus PR, Restore, and Settle controls through multiple snapshots; focus remains on the same logical control, never BODY.
6. Independent scrolling: wheel/keyboard-scroll Decisions, Tickets, Activity, and Settled; only the intended region moves. Switching utility tabs restores offsets.
7. Scan hierarchy: with realistic seeded load, identify blocked work, its ticket, and next action within five seconds; compare median completion time in blind trials.
8. Honest choices: reorder parsed options in a fixture; none becomes primary solely due to its index. All original reply values remain available.
9. Legibility: measure every status text/background pair in both themes at 4.5:1 or better; inspect focus/boundaries at 3:1 and 200% zoom without clipping.
10. Responsive density: at 390 px, no horizontal page scroll or clipped controls; all actions have 44 px touch height. At 1024 px, PR/status columns remain consistent.
11. State clarity: inspect empty, loading, stale, and disconnected fixtures. Labels never contradict health, and state changes move no composer or panel boundary.
12. Visual restraint: blind side-by-side review confirms consistent spacing/type tokens, no decorative gradients/emoji, no raw Markdown in summaries, and no added meaningless badges.
13. Functional parity: exercise every inventory item against fixtures, including failures, mixed launch results, hidden-tab notifications, archive restoration, and thread-link fallback.
