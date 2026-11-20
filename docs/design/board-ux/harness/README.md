# Demo board harness

These files reproduce the board state the design review and the reference
comparison used. They run against the in-process fake T3, so no T3 Code
server is needed and no real repository, thread or person appears on screen.

- `board_demo.rs` seeds a realistic load and serves the board: 15 open
  tickets with nested workers and PRs (running, waiting, failed, idle, and
  one launched but not yet joined), 4 done tickets, 7 decision cards (5 gates, a blocker, a report),
  about 45 activity lines, 3 asks (2 answered, 1 running) and 26 settled
  tickets. Each ticket's thread title reads like a launched ticket's
  (`relay#11: Move tenant settings into their own table`). It then keeps
  the board live: a note every 4 seconds, a new gate
  card every 6 notes and a new ticket every 9. With `BOARD_DEMO_CHURN=1` it
  every 5 notes also dismisses the oldest card and, when a ticket has
  arrived during the run, moves the oldest such ticket to Done, for the rubric's stability tests.
  Notes go to the seeded tickets, which never close. Each new gate goes to
  a ticket that arrived during the run and has no gate yet, or to a seeded
  ticket when there is none, so decisions keep accumulating.
- `capture.js` takes the screenshots in `../current/` and measures how far
  the Launch textarea sits below the fold and how far a focused input moves
  while live updates arrive.
- `capture-states.js` takes the empty, loading and disconnected states.
- `cut-stream.js` drops the live event stream without reloading the page,
  holds it down for 5 seconds, and waits for the board to reconnect. It
  reports where a focused Launch textarea sat before and after. The
  rubric's U3 and U11 use the same method.

PR 2 added the scripts that score the redesign against
[`../rubric.md`](../rubric.md). Each returns JSON:

- `gates.js` measures the static gates: U1, U2, U5 to U10, P1 to P3, N1,
  N2 and F4. Run it on the seeded board, and again on a heavy one
  (`BOARD_DEMO_EVERY=1`, left running until it has at least 30 open
  tickets and 15 decisions). The heavy board sends each new gate to a
  ticket that joined during the run and has no gate yet, so the decision
  count keeps rising instead of a new gate replacing an old one. U1 records
  both counts, and a heavy run below 30 tickets or 15 decisions does not
  count.
- `stability.js` runs U3 on a fresh board started with
  `BOARD_DEMO_EVERY=1 BOARD_DEMO_CHURN=1`. Churn drains the decision cards
  within a few minutes, so start the board right before the run. A page
  script samples every 20 ms for each whole case, stream cut included: the
  focused element's position, value and caret, whether focus stays on the
  same connected node, and what sits under the pointer. Each hover lasts
  60 s; the card option is answered from a second context at 1 s, and the
  ticket Settle hover includes a stream cut.
- `holds.js` runs U4, U11 and F3 on the seeded board, plus two F1 checks: a
  launch with one good and one failing issue, and a browser notification
  for a new decision while the tab is hidden.
- `judge-stills.js` captures the blinded stills the judges score, for any
  subject. It needs `__EMPTY_URL__` as well.

Issue #93 added the scripts for the [Dynamic layout](../rubric.md#dynamic-layout)
criteria. They run against the live replay in [`../live/`](../live/README.md),
not the demo harness, and only at desktop widths (1200 px and up), where
panels can be moved:

- `layout-gates.js` measures L1 to L7 and returns JSON with a `summary` line.
  Besides `__BOARD_URL__` it needs `__MAIN_URL__`, a second replay serving
  `main`'s `src/board.html`, for L5's comparison, and `__CAPTURE_DIR__`, where
  it saves the narrow-width screenshots it compares.
- `layout-stills.js` captures the candidate's stills for L8 to L13 at
  1600x1000 (the references' size) and two 1920x1080 stills of a rearranged
  board, light and dark.
- `layout-packets.py` builds one blinded packet per judge from those stills
  and [`../references/panels/`](../references/panels/README.md), and writes
  the letter key next to the packets:
  `python3 layout-packets.py <stills> ../references/panels <out> opus sonnet sol`.
- `layout-score.py` turns the key and each judge's JSON into the round's
  table: `python3 layout-score.py <out>/key.json opus=<file> sonnet=<file> sol=<file>`.

The replay server reads the page once when it starts, so restart it after
every change to `src/board.html`.

## Run it

The harness is an ignored integration test, kept here so it does not run in
CI. Copy it into `tests/` for the session and remove it afterwards:

```sh
cp docs/design/board-ux/harness/board_demo.rs tests/board_demo.rs
BOARD_DEMO_PORT=47790 cargo test --test board_demo -- --ignored --nocapture
```

It prints `BOARD_DEMO_READY http://127.0.0.1:47790/` and serves until you
stop it. Environment variables:

| Variable | Default | Effect |
| --- | --- | --- |
| `BOARD_DEMO_PORT` | `47790` | Port on 127.0.0.1. |
| `BOARD_DEMO_SCENARIO` | (seeded) | `empty` serves a board with nothing on it. |
| `BOARD_DEMO_EVERY` | `4` | Seconds between live updates; `0` keeps the board still. |
| `BOARD_DEMO_CHURN` | off | `1` dismisses a card every 5 notes, and moves a ticket that arrived during the run to Done when one is waiting. |
| `BOARD_DEMO_STATE` | a temp dir | State directory, if you want to keep it. |

Use a port other than the live service's 47731.

## Capture

The capture scripts are `playwright-cli run-code` functions with
placeholders. Substitute them and run:

```sh
out=/path/to/captures
mkdir -p "$out"
sed "s|__BOARD_URL__|http://127.0.0.1:47790/|; s|__CAPTURE_DIR__|$out|" \
  docs/design/board-ux/harness/capture.js > /tmp/capture.run.js
playwright-cli -s=board open about:blank
playwright-cli -s=board --raw run-code --filename=/tmp/capture.run.js > "$out/report.json"
```

`capture.js` samples the focused input every 2.5 s. In both capture runs
one `waitForTimeout` returned after about 750 ms at the same sample, so
`report.json` has one short interval. The drift figures use only the
sampled positions, so that interval does not change them.

`capture-states.js` also needs `__EMPTY_URL__`, a second instance started
with `BOARD_DEMO_SCENARIO=empty BOARD_DEMO_EVERY=0` on another port.

ImageMagick made the committed WebP files with
`convert in.png -resize '1440x>' -quality 78 out.webp`.
