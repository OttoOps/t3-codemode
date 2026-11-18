# Cross-review position

This is my proposed joint position after reading both reviews. Evidence checked this round includes the supplied measurements, desktop light/dark and phone captures, source, and nine reference images opened as images. I made no live submissions. References below are files in [the reference collection](../references/README.md); stills establish visual composition, not update or keyboard behavior.

## Findings they have that I missed or underrated

- **High: irreversible replies need a review step.** I identified arbitrary first-option emphasis but missed the separate execution risk. In [board.html](https://github.com/OttoOps/t3-codemode/blob/7e29c9d/src/board.html#L355), options post immediately, and removed cards release their slots at line 500. The alleged double-click consequence is a plausible failure mode, not a reproduced incident.
- **High: decision urgency needs structure.** The desktop capture gives a blocker the same card treatment as routine gates. Use explicit blocker/error grouping when initially displaying or accepting buffered work. The Needs input / Working / Completed groups in `claude-code-agent-view.webp` make the distinction legible without colored containers.
- **Medium: tickets need a decision link.** Widgets#41 has a gate label but no link to a1. Add "Needs you · a1" and reciprocal navigation; Datadog's selected list row beside its detail demonstrates the value of preserving that relationship.
- **Medium: done work consumes active-queue space.** The source renders the entire ticket tree despite counting only open tickets. A separate Done view removes that mismatch while retaining explicit settlement.
- **Medium: notification permission needs an explicit control.** The source requests permission on the first document click at line 577. Preserve notifications, but make permission a deliberate "Notify me" action.
- **High: done-chip contrast also fails.** My independent recalculation gives 4.4854:1 for `#2f7a4b` on `#e2f1e7`. Add it to the waiting and idle failures I already reported.

## Findings or prescriptions I dispute

- **Medium border defect, overbroad diagnosis.** Input boundaries need stronger contrast, but the dark capture's cards remain distinguishable by background steps. "Panels exist only by shadow" overstates the evidence; a universal 1.3:1 panel-divider rule is an arbitrary scoring gate.
- **Density prescription is too tight.** An 88 px decision cannot reliably hold identity, a two-line question, several options and accessible disclosures. GitHub's small board cards contain fewer controls. Keep compact defaults, allow wrapping, and test actual actions rather than a six-card quota.
- **Ten-second reflow timeout is unacceptable.** A stationary pointer, focused field or text selection remains protected after ten seconds. Also drop mandatory running/accent delta-E separation; neutral running text eliminates that competition more directly.

## Positions on the open questions

1. **Use the right rail at 1440x900.** Fix Launch and Ask above bounded results/answers, with Activity scrolling below; this replaces my bottom-strip preference because full-height decision and ticket queues matter more than a wide launch field. `linear-issue-agent-panel.webp` shows commands contained beside the work; `slack-channel-thread.webp` supports separating composers from scrolling content, though neither proves this exact arrangement.

2. **Offer a limited, disableable shortcut set.** Outside editable fields, allow `j`/`k`, `g d`/`g t`/`g a`, `l`, `/`, and `r`; digits focus a selected card's options and never submit them. Keep native Tab/Enter/Space, visible shortcut help, and disable/rebind support; the agent-view image's hint line shows how shortcuts can remain discoverable.

3. **Target 128-152 px for a default desktop decision.** Always show identity, kind, age, the question, every option, Reply, Dismiss and Full message; disclose the reply input/Send and message body, allowing taller cards for wrapped options or expanded content. My previous 144-200 px target retained too much repetition, while `linear-triage.webp` demonstrates how a clear title and secondary line reduce scanning cost.

4. **Keep running neutral and quiet.** Retain its explicit status word; mark human-needed tickets with an amber rule plus "Needs you" and linked decision IDs/count, independently of their running/waiting status. The agent-view reference separates working from needs-input structurally; do not infer human attention solely from a generic waiting status.

5. **Confirm irreversible replies.** Show the ticket, exact reply and consequence before sending; an undo window is valid only if execution genuinely waits, and successful delivery must not claim the merge/deploy finished. All parsed options receive equal emphasis; only the explicit confirmation action receives primary styling.

6. **Put Open, Done and Settled tabs inside Tickets; keep Activity in the rail.** Done tickets leave Open without automatic settlement, and Settled retains both tickets and asks, filtering and Restore. `github-projects-board.webp` visibly separates Done from other views, while agent view groups completed sessions separately.

7. **Combine buffering, deferred reflow and anchored scrolling.** Hold insertions/reordering behind a reserved "N new" control while reading, hovering, focusing or selecting; reconcile status in place, preserve the first visible item and pixel offset, and follow Activity only when already at its live edge and inactive. Answered or externally removed cards immediately lose actionable controls but retain their slot as a resolved placeholder until interaction ends or the user explicitly applies changes; never collapse on a timer.

8. **Use bottom tabs below 900 px.** Show one panel, with Launch/Ask entry immediately above the tabs; opening a composer moves it above the visual keyboard viewport, preserves drafts and returns focus/scroll on close, using a sheet when remaining height is too short. At 900-1200 px retain two queues and a bottom command strip; this is our responsive choice, since the references do not establish phone behavior.

9. **Use sentence case, 14/20 px, weight 650, primary ink.** Counts are secondary; align headings in fixed header rows and separate flat panels with one-pixel borders plus restrained background steps, without card shadows. `linear-inbox.webp`, `linear-triage.webp` and the agent-view headings establish hierarchy through position and weight without uppercase labels.

## Joint rubric changes

- Merge drift, focus and pointer tests into an interaction-stability suite. Inject arrivals, removals, status changes and reconnects for 60 seconds; require zero unsolicited target displacement, retained logical focus/caret, and no adjacent action activation.
- Add queue relationships and lifecycle. From a needs-you ticket, reach its decision and return; move a completed ticket to Done, settle it, find it by filter and restore it.
- Replace card-count quotas with a fixed two-line fixture and wrapped-option stress case. Check the default height target, visible options, reachable disclosures, and unclipped controls at 200% zoom.
- Add confirmation tests. Reorder option text, cancel an irreversible reply, then confirm it; require no index-based primacy and exactly one request after confirmation.
- Extend shortcut tests to inputs, contenteditable, IME composition and disabled shortcuts. Digits must never execute replies.
- Test phone keyboard geometry on a real browser/device, including rotation and safe areas. Keep the focused input and submit control visible and restore the previous panel position.
- Retain parity, independent scrolling, text/control contrast and state clarity. Replace "no removable element" and rigid hue/spacing thresholds with visible hierarchy, grayscale attention recognition and readable controls in both themes.
