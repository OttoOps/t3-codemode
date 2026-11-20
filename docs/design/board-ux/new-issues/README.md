# New issues stills

Before and after stills for the New issues section ([issue #57](https://github.com/OttoOps/t3-codemode/issues/57), PR 2 of 3).
Both replay the live board snapshot in [`../live/state.json`](../live/README.md)
with three made-up `acme/*` issues reported and one dismissed. `before/` is
`main` at `f5ba03d`, before the section; `after/` is this PR. Files ending in `-empty` show
the section collapsed with no new issues, and `1366-light-dismissed` shows the
Show dismissed view.

| Width | Tickets column, before | New issues | Tickets, after |
| --- | --- | --- | --- |
| 1920 x 1080 | 995 px | 209 px | 786 px |
| 1366 x 768 | 683 px | 209 px | 474 px |
| 1024 x 768 | 627 px | 209 px | 418 px |
| Any, empty | - | 41 px | column less 41 px |

The section holds three rows before it scrolls, and takes at most 40% of the
column. No view scrolls the page or sideways.

## Run it

`add_issues.py` writes the state with the issues (`full`) or without them
(`empty`). `capture.js` takes the stills at each width and reports the section
and Tickets heights.

```sh
out=/tmp/new-issue-stills; mkdir -p "$out"
python3 docs/design/board-ux/new-issues/add_issues.py \
  docs/design/board-ux/live/state.json "$out/state-full.json" full
python3 docs/design/board-ux/live/replay.py src/board.html "$out/state-full.json" 47792 &
sed "s|__BOARD_URL__|http://127.0.0.1:47792/|; s|__CAPTURE_DIR__|$out|; s|__TAG__|after|" \
  docs/design/board-ux/new-issues/capture.js > "$out/capture.run.js"
playwright-cli -s=new open about:blank
playwright-cli -s=new --raw run-code --filename="$out/capture.run.js" > "$out/report-after.json"
```

For the before stills, replay `git show f5ba03d:src/board.html` the same way.
