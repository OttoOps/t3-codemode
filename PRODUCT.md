# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Solo and small-team software engineers operating autonomous AI coding agents via T3 Code (`t3-codemode` / `t3cm`). They check the board throughout the workday on desktop browsers (and occasionally remotely on mobile via Tailscale) for quick 3-to-15-second operational glances between coding sessions, plus occasional triage bursts.

## Product Purpose

The t3 board is the live operational control surface for autonomous ticket and PR workflows. It replaces an LLM overseer thread by aggregating every ticket thread, worker thread, PR, human approval gate, newly filed issue, flow queue, and activity stream into one local web UI.

Success means an operator glancing at the board can answer three questions within 3 seconds without parsing prose or opening GitHub tabs:
1. **What is waiting for me to answer?** (Gates, blockers, escalations, and pool-limit moves, with exact one-click reply options.)
2. **What issues are currently in progress?** (Shown by real human-readable titles and summaries, not just clickable issue numbers, along with multi-PR progress.)
3. **What PRs are currently in progress?** (PR numbers, real titles, CI/review readiness, and which ticket or epic each PR advances.)

## Positioning

Unlike general issue trackers (Linear, Jira, GitHub Projects) or generic agent logs, t3 board couples deterministic workflow orchestration directly to live agent threads and GitHub PR readiness: gates enforce head-SHA merge checks, workers report structured multi-PR chains (`PR 4/10`), and human replies route directly back into the exact waiting agent thread without spending tokens on an overseer LLM.

## Operating Context

- Served locally by the `t3cm` Rust daemon (`http://127.0.0.1:47731/`) and optionally remotely over Tailscale Serve HTTPS or pairing cookies.
- Live state flows from `GET /api/state`, `GET /api/flow`, `GET /api/usage-stats`, `GET /api/msg/{handle}`, and SSE `/api/events`.
- Must work across both quiet states (0 decisions waiting, 1-2 active tickets) and busy states (multiple gates/blockers, 5-10 open tickets with multi-PR worker chains, 15+ newly filed issues, 50+ activity items, 30+ dependency graph nodes).
- Desktop-primary at 1280px-1920px+ with responsive phone support (390px) for remote check-ins.

## Capabilities and Constraints

- Zero external runtime dependencies: self-contained HTML, CSS, and vanilla JavaScript (CSP blocks external scripts, styles, and remote fonts in production `src/board.html`).
- Full functional surface of the board: Needs you decision cards (with option buttons, custom reply, 5s undo, dismiss, full-message view), active Tickets (Open, Done, Settled, Dependency Graph), active PR tracking, New issues triage (Launch / Dismiss / Restore), Up next flow queue (Ready / Blocked / Skipped), Activity stream, Launch tickets & Ask your agent composers, Launch routing & pool health, Contexts & filters, Inspector drawer, Settings, light/dark themes, and keyboard navigation (`j`/`k`, `/`, `l`, `a`, `Esc`, `?`).
- Current task scope: standalone interactive prototype in `docs/design/` wired to real live snapshot data (with optional live `/api/state` and `/api/flow` refresh), free to reorganize secondary panels (Launch/Ask, Routing, New issues, Up next, Activity) into cleaner regions, drawers, popovers, or tabs so primary operational information reads effortlessly.

## Evidence on Hand

- Live board implementation: `src/board.html`, `src/board.rs`, `src/web.rs`.
- Full board specification: `docs/src/board.md`.
- Prior board UX critique and references: `docs/design/board-ux/critique.md`, `docs/design/board-ux/README.md`.
- Live runtime data captured from `http://127.0.0.1:47731/api/state` and `/api/flow` (2 active multi-PR tickets `#254` at 4/10 with PR `#283` in review and `#244` at 3/5 with `#282` running, 14 new issues, 28 dismissed, 90 settled, 35 graph nodes, `claude` pool routing). The snapshot embedded in the prototype keeps that shape, with the repository, GitHub account, commit SHAs, T3 ids and pool accounts replaced by made-up values (`acme/widgets`).

## Product Principles

1. **Interrupts lead when present, disappear when clear.** Waiting decisions demand immediate visual dominance when count > 0, and yield their space to active work when count = 0.
2. **Human titles over opaque identifiers.** Issue numbers and PR numbers are anchors, never substitutes for readable titles, summaries, and progress ratios.
3. **One fact, one place.** Eliminate redundant counts, repeated chips, and stacked status chrome across header, pulse strip, tabs, and rows.
4. **Active work and PRs are first-class.** Multi-PR ticket chains and live PR readiness should read as a structured visual glance, not buried text snippets.
5. **Secondary controls stay reachable without taxing every glance.** Composers, routing telemetry, filters, and triage queues remain one click or shortcut away without crowding live operational status.

## Accessibility & Inclusion

- AA contrast in both light and dark modes; status conveyed through shape + label + color (never color alone).
- Full keyboard operability with visible focus rings, skip links, semantic landmarks, and reduced-motion respect.
