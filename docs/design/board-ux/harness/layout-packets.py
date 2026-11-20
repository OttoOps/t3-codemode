"""Build blinded judge packets for the Dynamic layout criteria (rubric L8 to L13).

Usage: python3 layout-packets.py <candidate stills dir> <references/panels dir> <out dir> <judge> [<judge> ...]

Each judge gets <out dir>/<judge>/ with judge.md and PNG stills named
<letter>-<n>.png. Letters and subject order are random per judge. The key
(letter to subject) goes to <out dir>/key.json and never into a packet.
"""

import json
import random
import subprocess
import sys
from pathlib import Path

CANDIDATE = [
    ("rest.png", "Resting layout, default arrangement."),
    ("drag-into-column.png", "Mid-drag: a panel is being moved by its header toward the top of the middle column."),
    ("drag-to-empty-column.png", "Mid-drag: a panel is being moved to a column that has no panels yet."),
    ("divider-hover.png", "The pointer rests on the divider between two columns."),
    ("divider-resize.png", "Mid-resize: the divider between two stacked panels is being dragged down."),
    ("collapsed.png", "Two panels are collapsed; the rest are expanded."),
    ("rearranged-light.png", "A non-default arrangement chosen by the user, light theme."),
    ("rearranged-dark.png", "The same arrangement, dark theme."),
]

REFERENCES = {
    "vscode": [
        ("vscode-sash-hover.webp", "Resting layout; the pointer rests on the divider beside the left side bar, after a short hover."),
        ("vscode-sash-resize.webp", "Mid-resize of the left side bar's divider."),
        ("vscode-drag-panel-to-sidebar.webp", "Mid-drag: a tab from the bottom panel is held over the lower half of the side bar."),
        ("vscode-drag-editor-split.webp", "Mid-drag: a tab is held near the right edge of the main area."),
        ("vscode-view-sections.webp", "Stacked side bar sections, some expanded and one collapsed; one section header is hovered."),
    ],
    "dockview": [
        ("dockview-drag-split.webp", "Mid-drag: a tab is held over the right part of another panel group."),
        ("dockview-floating-group.webp", "Resting grid with one group floated above it; two panels collapsed to edge rails."),
        ("dockview-resize.webp", "Mid-resize of the divider after the first column."),
    ],
    "goldenlayout": [
        ("goldenlayout-drag-split.webp", "Mid-drag: a panel is held over the right half of another panel stack."),
        ("goldenlayout-drag-tab-insert.webp", "Mid-drag: the same panel held over another stack's tab strip."),
    ],
    "grafana": [
        ("grafana-rows-collapsed-expanded.webp", "Sections (rows): two expanded, the rest collapsed."),
        ("grafana-edit-resize.webp", "Edit mode: mid-resize of a panel by its corner handle."),
        ("grafana-edit-drag.webp", "Edit mode: mid-drag of a panel by its header."),
    ],
    "jetbrains": [
        ("jetbrains-tool-windows-docked.webp", "Resting layout with four docked side windows (green tint and numbers are the publisher's annotations)."),
        ("jetbrains-tool-window-bars.webp", "Every side window collapsed to icons on the edge bars (green highlight is the publisher's annotation)."),
        ("jetbrains-drag-tool-window.webp", "One frame of an animation: a side window's icon is dragged toward the left bar."),
    ],
}

CRITERIA = """\
Score each subject on each criterion from 1 to 5, or `n/a` when none of its
stills can show the criterion. Give one sentence of evidence per score that
names the still it comes from.

| ID | Criterion | Anchors |
| --- | --- | --- |
| L8 | Discoverable at rest | From resting stills. 1: nothing says panels can move, collapse or resize. 3: some affordances show, or only on hover, and a user would have to guess at the rest. 5: move, collapse and resize controls are visible and legible at rest without adding clutter. |
| L9 | Drag feedback | From mid-drag stills. 1: nothing shows what moves or where it will land. 3: either the moving item or the landing spot is shown, or the landing spot is ambiguous. 5: what is moving, the exact landing area, and the source's state are all unmistakable. |
| L10 | Resize affordance and feedback | From divider stills. 1: no visible divider or handle. 3: a divider is visible, but its hover or active state is weak, or the layout does not visibly follow. 5: a clear divider with a distinct hover and active state, and the layout follows the drag live. |
| L11 | Collapsed state | From collapsed stills. 1: collapsed panels vanish, or collapsed and expanded look alike. 3: the collapsed state is clear, but the way back or the reclaimed space is not. 5: a collapsed panel keeps its name (and its count where it has one), the way to expand it is obvious, and its space goes to the rest. |
| L12 | A rearranged layout still reads | From stills of a non-default arrangement. 1: rearranging leaves the screen cramped, misaligned or broken. 3: usable, with visible seams. 5: the rearranged layout looks designed, in both themes where shown. |
| L13 | Layout chrome is polished and restrained | From all stills. 1: handles, chevrons, dividers and drop targets are noisy or inconsistent. 3: consistent but plain or slightly heavy. 5: cohesive and restrained; the chrome stays out of the content's way until it is needed. |
"""


def to_png(src: Path, dst: Path) -> None:
    subprocess.run(["convert", str(src), "-resize", "1600x>", str(dst)], check=True)


def main() -> None:
    cand, refs, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    judges = sys.argv[4:]
    subjects = {"candidate": [(cand / f, c) for f, c in CANDIDATE]}
    subjects |= {k: [(refs / f, c) for f, c in v] for k, v in REFERENCES.items()}
    key = {}
    for judge in judges:
        rng = random.SystemRandom()
        letters = rng.sample("BCDFGHJKMNPQRSTVWXZ", len(subjects))
        order = list(subjects)
        rng.shuffle(order)
        dest = out / judge
        dest.mkdir(parents=True, exist_ok=True)
        key[judge] = {}
        lines = []
        for letter, name in zip(letters, order):
            key[judge][letter] = name
            lines.append(f"## Subject {letter}\n")
            for n, (src, caption) in enumerate(subjects[name], 1):
                to_png(src, dest / f"{letter}-{n}.png")
                lines.append(f"- `{letter}-{n}.png`: {caption}")
            lines.append("")
        body = f"""# Judging panel layouts

You are one of several independent judges. Each subject below is a product
whose panels a user can move, collapse or resize. Every subject is shown by a
few stills, captioned with the state each one captures. The stills were taken
headless, so no mouse pointer is drawn in any of them. Judge only what the
stills show; do not guess which product a subject is, and do not reward or
penalize a product for its reputation. Judge each subject on its own panel
layout behaviour, not on its content or its domain.

Open every still (they are PNG files in this folder) before scoring.

{CRITERIA}
Reply with JSON only, no prose before or after it, in this shape:

```json
{{"scores": {{"<letter>": {{"L8": [<1-5 or "n/a">, "<evidence>"], "L9": [...], "L10": [...], "L11": [...], "L12": [...], "L13": [...]}}}}}}
```

{chr(10).join(lines)}"""
        (dest / "judge.md").write_text(body)
    (out / "key.json").write_text(json.dumps(key, indent=2) + "\n")


if __name__ == "__main__":
    main()
