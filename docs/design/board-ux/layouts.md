# Proposed layouts and visual direction

Two layouts come out of the [paired review](review.md). Layout A is the one
the pair recommends; Layout B is a cheaper fallback that keeps most of A's
behavior. Both share the behavior contract and the visual direction below.
PR 2 builds against these, and [`rubric.md`](rubric.md) scores the result.

## Layout A: three panes and a command rail (recommended)

### 1440x900

```text
+------------------------------------------------------------------------------+
| t3 board   * connected, watcher 23:35       12 decisions  23 tickets  9 run  | 40 px
+----------------------------+------------------------------+------------------+
| Decisions 12    [3 new]    | Tickets 23   Open|Done|Settled| Launch           |
|----------------------------|------------------------------| [issues, 2 rows] |
| a6 relay#11 blocker    44m | | widgets#41  running        |         [Launch] |
|   Worker failed twice on   | |   needs you - a1  PR #81   | results (capped) |
|   the same migration.      | |   PR 1/2 idle  PR 2/2 run  |------------------|
|   [Reply] [Dismiss]        |   gizmo#12    waiting  PR #30| Ask              |
| a1 widgets#41 gate     58m | | ledger#198  running        | [question] [Ask] |
|   Merge PR #81?            |   ...                        | answers (capped) |
|   [merge] [merge and deploy]                              |------------------|
|   [Reply] [Dismiss]        |                              | Activity 50      |
|   ...         (scrolls)    |            (scrolls)         | 23:35 gizmo#15 ..|
|                            |                              |    (scrolls)     |
+----------------------------+------------------------------+------------------+
   about 460 px                  flexible                       about 340 px
```

- The page never scrolls: `height: 100dvh; overflow: hidden`. Decisions,
  Tickets and Activity each scroll on their own.
- The header is 40 px. The disconnect state replaces the status text and
  dims the panels, and never inserts a banner.
- Launch and Ask are fixed at the top of the rail. Launch is two rows and
  grows to six while focused. Launch results and Ask answers sit in capped
  blocks (together at most 30% of the rail) that scroll on their own.
  Activity fills the rest of the rail and scrolls.
- Tickets has three tabs: Open, Done, Settled. Done tickets leave Open for
  Done, which carries "Settle all done". Settled holds settled tickets and
  asks, with the filter pinned in the panel header and Restore on each row.
- Every panel header is 40 px with a reserved slot for the "N new" control,
  so the control appearing never moves anything.

### 900 to 1200 px

Two panes, Decisions (44%) and Tickets (56%). Activity becomes a fourth tab
of Tickets. A 56 px command bar is docked at the bottom of both panes with a
Launch and Ask switch. It grows upward while focused, over the panes rather
than pushing them.

### 390 wide

```text
+------------------------------+
| t3 board  *     12 - 23 - 9  |  44 px, one line
+------------------------------+
| Decisions 12        [3 new]  |
| a6 relay#11 blocker     44m  |
|   Worker failed twice on...  |
|   [Reply] [Dismiss]          |
| a1 widgets#41 gate      58m  |
|   Merge PR #81?              |
|   [merge] [merge and deploy] |
|        (panel scrolls)       |
+------------------------------+
| [Launch issues or ask...]    |  pinned composer
+------------------------------+
| Decisions | Tickets | Activity | More |  56 px bottom tabs
+------------------------------+
```

- One panel at a time; the tab is kept in the URL hash. Tabs carry counts.
  Settled sits under More.
- The composer sits above the tab bar. When it takes focus, the tab bar hides
  and the composer docks on top of the on-screen keyboard (`visualViewport`).
  If less than about 300 px is left, it opens as a full-height sheet, and
  closing it returns focus and scroll to where they were. Drafts survive
  switching between Launch and Ask.
- Touch targets are at least 44 px. No horizontal page scroll.

## Layout B: two queues and a bottom composer

Decisions and Tickets side by side at full height, Tickets with Open, Done,
Settled and Activity tabs, and a 64 px composer across the bottom with a
Launch and Ask switch. It grows to four lines while typing, and Ask answers
open in a popover above it. Its 390 px and 900 to 1200 px forms match A.

B is cheaper to build and identical to A at mid widths. It is weaker at
1440 because Activity loses its always-visible column and Ask answers are
transient. Build B only if A fails the rubric on space.

## Behavior contract (both layouts)

### Arrivals, changes and removals

- **New items.** When a panel is scrolled to its top and neither the pointer
  nor focus is inside it, new items insert live with a 1.2 s background fade
  and no slide. Otherwise they wait behind the "N new" control in the panel
  header, which names urgent kinds ("3 new - 1 blocker"). Applying them is
  the user's choice. There is no time limit on holding.
- **Changes.** A visible row updates in place inside a fixed-height slot.
  Notes clamp to one line, so a change never changes a row's height.
- **Removals.** An answered or dismissed card, a ticket that moves to Done,
  or a settled ask becomes a placeholder of the same height ("a1 answered:
  merge"), with its controls disabled. It collapses only after the pointer
  and focus leave the panel. Nothing collapses on a timer.
- **Scroll anchoring** (keep the first visible item and its pixel offset)
  is the fallback for any insertion above the viewport that cannot be
  avoided.
- **Activity** follows new lines only while it is scrolled to its live edge
  and nobody is pointing at it. Otherwise it buffers with a count.
- **Rendering** reconciles every list by key (cards, tickets, workers, feed,
  asks, settled rows), as the board reconciles cards today, so a focused control
  is never detached.

### Replies

- Every parsed option has the same weight. Nothing is primary unless the
  source marks it.
- Clicking an option or Send starts a 5 s window. The card shows "Sending
  'merge' to widgets#41 - Undo" in its own reserved row, and the request
  leaves only when the window ends. The card never claims the merge or
  deploy finished.
- The free-text reply is a quiet "Reply" control. It expands in place on
  focus. The focused card stays where it is; cards below it move.
- Full message moves to a link in the card's header line.
- An explicit "Notify me" control in the header turns notifications on.
  The first click on the page no longer asks for permission.

### Connection

The header shows transport freshness and T3 and watcher health as separate
facts. When the event stream drops, the header says so, the panels dim, and
reply controls are disabled. No layout moves.

### Keyboard

- Skip links: Decisions, Tickets, Launch, Ask, Activity.
- Shortcuts work only when no field, contenteditable or IME composition has
  focus. The user can turn them off, and `?` lists them:
  `j`/`k` next and previous row or card; `g d`, `g t`, `g a` jump to
  Decisions, Tickets, Activity; `l` focuses Launch; `/` focuses Ask; `r`
  opens the reply on the focused card; `1` to `9` focus option n (Enter
  sends it); `Esc` returns to the panel.
- One tab stop per panel, with a roving tabindex inside it. Tabs use arrow
  keys.
- Enter sends a one-line Ask or reply. Ctrl or Cmd+Enter submits Launch.
- A needs-you ticket's "a1" link moves focus to its card; the card's ticket
  label moves focus back to the row.

### Announcements

One polite live region announces Launch results, Ask answers and "N new
decisions" once each. Errors are alerts. Activity lines are never announced.
Every option, Send, Dismiss, Settle and Restore control has an accessible
name that includes its card id or ticket label.

## Visual direction: flat operations console

A quiet, flat console. Position and weight do the work that shadows and
colored pills do today. The direction keeps what the review says works: the
system font, neutral surfaces, status words, the tuned dark theme.

### Type

System UI stack. Four sizes, three weights (400, 500, 600). Tabular numerals
for counts, ages and times. Monospace only for ids and hashes.

| Role | Size / line | Weight | Color |
| --- | --- | --- | --- |
| App title | 15/20 | 600 | ink |
| Panel heading (sentence case) | 14/20 | 600 | ink, count in ink-2 |
| Ticket label, card question | 14/20 | 600 / 500 | ink |
| Body, card detail, answers | 14/20 | 400 | ink |
| Meta: age, notes, Activity, links | 13/18 | 400 | ink-2 |
| Status word, tags, ids | 12/16 | 600 | status role |

On phones, inputs are 16 px so the browser does not zoom.

### Spacing and density

- Every margin, padding and gap comes from 4, 8, 12, 16 and 24 px. Panels
  have 12 px of horizontal padding.
- A panel header is 40 px. A ticket row is 32 px, plus 18 px when it shows
  a one-line note. A worker row is 28 px, an Activity row 24 px, and a
  Settled row 32 px.
- A decision card has a 20 px header line, a question of at most two lines,
  and one 32 px action row, for a median height of 128 to 144 px.
- Buttons are 28 px tall in rows, 32 px in cards, and 44 px on touch
  screens.
- Corners have a 6 px radius, and nothing has a drop shadow.

### Color roles

Interaction and status are separate. One accent hue, for links, focus and
the few primary actions (Launch, Ask, Undo). Five status roles, each a small
dot and a word, with no filled pills:

| Role | States | Light text | Dark text |
| --- | --- | --- | --- |
| Neutral | running, idle, launching, unknown | `#4f555c` | `#b4b9c0` |
| Needs you | gate, waiting on the human | `#8a4f00` | `#e8ac55` |
| Failed | failed, gone, blocker, error, escalate | `#a52a22` | `#f08a80` |
| Done | done, answered | `#24663d` | `#79c995` |
| Accent | links, focus, primary | `#2f5bd3` | `#7c9cf5` |

- Running is neutral. A running ticket is the normal case and should recede.
- "Waiting on CI" is neutral. Amber is only for things that wait on the
  person.
- A needs-you ticket gets a 3 px amber left rule and a "needs you - a1"
  link.
- Every status text is at least 4.5:1 on its background in both themes.
  Input borders are at least 3:1 (for example `#8c9096` on white).
- Surfaces keep today's canvas and panel tokens (`#f6f6f3` / `#ffffff`
  light, `#15171a` / `#1d2024` dark), separated by 1 px borders
  (`#dcdcd5` light, `#3d434a` dark) as well as the background step.

### Motion

Background fades only: 1.2 s for a new item, 120 ms for hover and press
feedback, 150 ms opacity for a placeholder collapse. No slides, no height
animation. Reduced motion turns all of it off.

### Remove

Drop shadows on cards and panels; 12 px radii; the header blur; pill
backgrounds on chips; the filled id badge; underlines on the repeated
"thread" and PR links (underline on hover and focus instead); the
always-visible "Full message mNN" label; raw `**` Markdown and the
duplicated gate text in ticket notes; "MST" on every Activity row (show the
zone once in the header); first-option primary styling.
