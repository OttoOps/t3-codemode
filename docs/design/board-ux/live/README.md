# Live board replay

Issue #67's captures come from the real board. #28's judging passed a
design on the seeded fixture that fails under real use, so these files
freeze one moment of the live board and replay it.

- `state.json` is the live `/api/state` from 18 Nov 2026, 17:26 MST: 4
  decision cards, 8 open tickets with 18 workers, 50 Activity rows and 19
  settled items. `anonymize.py` made it from the raw snapshot. It keeps
  every item, kind, status, gate and time, keeps text lengths close, and
  replaces the private repository's name and content and every thread id. This
  project's own public tickets keep their real labels.
- `replay.py` serves a board page against `state.json`. `GET /` serves the
  page, `/api/state` the state, and `/api/events` one `state` event on a
  stream that stays open. Every other request gets 405, so nothing clicked
  during a capture reaches a real service.
- `capture.js` takes the stills and measures clipped text, sideways and
  line-clamped, and the type scale. It blocks every request that is not a `GET` and can fix the clock
  so relative ages match the snapshot.
- `before/` holds the current board (`main` at `49f9731`) replayed against
  `state.json`, with `report.json`.

The replay matches the live board closely. Run minutes apart with the same
probe, the replay and the live service, read-only, gave the same
clipped-element counts at 390 px and counts within two of each other at
1920 and 1366, where anonymized text differs in length and the live board
had moved on.

## Run it

```sh
git show 49f9731:src/board.html > /tmp/board.html
python3 docs/design/board-ux/live/replay.py /tmp/board.html \
  docs/design/board-ux/live/state.json 47792 &
out=/tmp/board-captures; mkdir -p "$out"
sed "s|__BOARD_URL__|http://127.0.0.1:47792/|; s|__CAPTURE_DIR__|$out|; s|__NOW__|2026-11-19T00:26:30Z|" \
  docs/design/board-ux/live/capture.js > /tmp/capture.run.js
playwright-cli -s=live open about:blank
playwright-cli -s=live --raw run-code --filename=/tmp/capture.run.js > "$out/report.json"
```

PR 2 replays its own `src/board.html` the same way for the after-captures.
To take a fresh snapshot, save the live service's `/api/state` and run
`python3 docs/design/board-ux/live/anonymize.py <raw.json> <private.json> docs/design/board-ux/live/state.json`. The private
file stays outside the repository, because it names what it hides. It holds
`subs`, a list of `[pattern, replacement]` regex pairs, and `forbid`, the
words that must not survive. The script refuses to write while any
forbidden word remains, so add a substitution for each new private name.
