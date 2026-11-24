---
version: 1
slug: "docs-design-board-dispatch-html"
primary_target: "docs/design/board-dispatch.html"
related_targets: ["src/board.html"]
---

# Surface Brief: t3 board Dispatch Prototype

- **Scope:** Standalone interactive prototype at `docs/design/board-dispatch.html` (exploring a cleaner visual design for `src/board.html`).
- **Visitor mode:** Operate.
- **Audience & job:** Solo/small-team T3 Code operator performing 3-to-15-second operational glances across active agent threads, answering approval gates/blockers, checking real titles of in-progress issues and PRs, and triaging newly filed issues.
- **Proof & content:** A live snapshot, with the repository, account, commit SHAs and T3 ids replaced by made-up values (`acme/widgets`), captured from `http://127.0.0.1:47731/api/state`, `/api/flow`, and `/api/usage-stats`, enriched with human-readable titles from the flow graph, plus optional live same-origin/bridge polling and a labeled load-state switcher (live state vs busy multi-gate demo vs all-clear 0-waiting state).
- **Constraints:** Zero external dependencies (single self-contained HTML/CSS/JS, system sans + tabular mono, inline drawn SVGs), dark default with light toggle, desktop (1440px/1280px) and mobile (390px) responsive layouts, full keyboard navigation and AA contrast.

## Direction contract

- **THESIS:** Full-width nocturnal Swiss Railway Dispatch board that puts waiting decisions, human-readable active issues, and in-flight PRs directly on the primary canvas, refusing the current 3-column hairline box cage where an empty left column wastes 30% width and nested chips crush titles.
- **OWN-WORLD:** Nocturnal station slate dark-default (`#0c1017` canvas, `#131c2b` deep railway-navy command header, `#121822` work panels with crisp 2px structural top rules, `#eef2f7` primary ink, `#98a7bd` slate metadata; plus crisp architectural daylight light theme `#f4f6f4` with `#182232` header), ochre-amber (`#e29524`) strictly for waiting human decisions and active station nodes, emerald (`#2da46e`) strictly for merged/ready stations, signal blue (`#4da6ec`) for in-review PRs, vermilion (`#e5534b`) strictly for blockers and CI failures, paired with system neo-grotesque sans and tabular monospace identifiers.
- **STORY:** Within 3 seconds an operator sees whether any gate or blocker waits (with one-click answer and 5s undo, or a single-line all-clear bar when 0 wait), reads every active issue's full human title and multi-PR progress chain, inspects every in-progress PR with its readiness and parent context, and switches cleanly to New issues, Dependency graph, Activity, Launch/Ask, or Routing without cluttering the main dispatch view.
- **FIRST VIEWPORT:** Full-width 56px railway-navy top bar (`t3 board`, view tabs with single canonical counts, Launch and Ask buttons, theme toggle, shortcuts trigger); 42px live telemetry strip (waiting count, active flow slots, active PRs, merged count, pool routing popover, live search); collapsible Needs-you interrupt zone (full decision cards when > 0, quiet 38px emerald all-clear bar when 0); and a two-column split (1.25fr Active Issues with horizontal multi-PR station route-line tracks on the left, 1fr PRs in Progress ledger on the right) plus slide-over detail inspector.
- **FORM:** Swiss Railway Dispatch (Dark default with Light toggle), grounded candidate 1 of 7 (`IMPECCABLE'S PICK`, locked by user with dark-mode steer), `seed:05739e7fff24b901`. Signature interaction: clicking any station stop on an issue's PR route line highlights and opens that exact PR or sub-issue in the detail inspector, and answering a gate card launches a live 5-second undo countdown before committing.
- **FINISH:** unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
