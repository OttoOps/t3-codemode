---
name: "t3 board"
description: "Nocturnal Swiss Railway Dispatch design system for t3 board (dark default with architectural daylight toggle)."
colors:
  station-slate: "#0c1017"
  railway-navy: "#131c2b"
  telemetry-slate: "#101621"
  panel-slate: "#121822"
  panel-elevated: "#161e2b"
  panel-hover: "#1b2434"
  code-slate: "#1c2738"
  hairline-border: "#233044"
  control-border: "#4c6282"
  structural-rule: "#3a5378"
  track-rail: "#2c3b52"
  pure-white: "#ffffff"
  ink-primary: "#eef2f7"
  ink-secondary: "#a3b3c9"
  ink-muted: "#8395ae"
  ink-header: "#f4f7fb"
  ink-header-muted: "#9eb0c9"
  signal-ochre: "#e29524"
  ochre-ink: "#170f00"
  ochre-text: "#f5bd63"
  ochre-surface: "#1d170c"
  ochre-border: "#5e4419"
  signal-emerald: "#2da46e"
  emerald-text: "#6ee0a8"
  emerald-surface: "#0f1d16"
  emerald-border: "#224f3a"
  signal-blue: "#46a0e6"
  blue-text: "#7cc4fa"
  blue-surface: "#0f1e2e"
  blue-border: "#244a6e"
  signal-vermilion: "#e5534b"
  vermilion-fill: "#c93830"
  vermilion-text: "#ff948d"
  vermilion-surface: "#241214"
  vermilion-border: "#6b2929"
  daylight-ground: "#f4f6f4"
  daylight-navy: "#182232"
  daylight-border: "#d5dce5"
  daylight-ink: "#14181f"
  daylight-ink-secondary: "#465366"
  daylight-ochre: "#d98218"
  daylight-emerald: "#1c8253"
  daylight-blue: "#1a6cb0"
  daylight-vermilion: "#cc3329"
typography:
  headline:
    fontFamily: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif'
    fontSize: "17px"
    fontWeight: 800
    lineHeight: "22px"
    letterSpacing: "-0.015em"
  title:
    fontFamily: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif'
    fontSize: "15.5px"
    fontWeight: 700
    lineHeight: "1.35"
    letterSpacing: "0em"
  body:
    fontFamily: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif'
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "1.45"
    letterSpacing: "0em"
  label:
    fontFamily: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif'
    fontSize: "13px"
    fontWeight: 600
    lineHeight: "1.3"
    letterSpacing: "0em"
  code:
    fontFamily: 'ui-monospace, "SF Mono", "Cascadia Mono", Menlo, Consolas, monospace'
    fontSize: "12px"
    fontWeight: 700
    lineHeight: "1.4"
    letterSpacing: "0.02em"
rounded:
  sm: "3px"
  md: "4px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
components:
  button-primary:
    backgroundColor: "{colors.signal-ochre}"
    textColor: "{colors.ochre-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 13px"
    height: "32px"
  button-secondary:
    backgroundColor: "{colors.panel-slate}"
    textColor: "{colors.ink-primary}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 13px"
    height: "32px"
  button-danger:
    backgroundColor: "{colors.vermilion-fill}"
    textColor: "{colors.pure-white}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 13px"
    height: "32px"
  panel:
    backgroundColor: "{colors.panel-slate}"
    textColor: "{colors.ink-primary}"
    rounded: "{rounded.md}"
    padding: "16px 20px"
  status-pill-gate:
    backgroundColor: "{colors.ochre-surface}"
    textColor: "{colors.ochre-text}"
    typography: "{typography.code}"
    rounded: "{rounded.sm}"
    padding: "2px 8px"
  gate-card:
    backgroundColor: "{colors.ochre-surface}"
    textColor: "{colors.ink-primary}"
    typography: "{typography.title}"
    rounded: "{rounded.md}"
    padding: "14px 18px"
  input-search:
    backgroundColor: "{colors.panel-slate}"
    textColor: "{colors.ink-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 28px 0 32px"
    height: "30px"
    width: "220px"
  nav-tab:
    backgroundColor: "{colors.railway-navy}"
    textColor: "{colors.ink-header-muted}"
    typography: "{typography.label}"
    padding: "0 14px"
    height: "56px"
---

# Design System: t3 board

## Overview

**Creative North Star: "Nocturnal Swiss Railway Dispatch"**

t3 board is the live operational control surface for autonomous ticket and pull request workflows in T3 Code (t3cm). Rather than burying live engineering state inside multi-column kanban boxes or generic chat feeds, the interface draws from nocturnal Swiss railway departure boards: high-contrast typography, rigid horizontal alignments, unambiguous signal colors, and instant scannability built for 3-to-15-second operational glances between coding sessions.

Every architectural surface follows a strict hierarchy of urgency. Waiting human decisions lead the canvas with full-border tinted frames and one-click resolution controls, then collapse into a quiet single-line all-clear strip when zero gates remain. Beneath the interrupt zone, human-readable active issues pair with connected horizontal Perlschnur station route lines on the left while an aligned PR readiness ledger tracks open, merge-gated, and recently landed pull requests on the right.

The system defaults to Nocturnal Station Slate for sustained low-glare readability alongside terminal and editor environments, paired with an Architectural Swiss Daylight light toggle that preserves the dark Railway Navy command bar as a constant navigational anchor. Built with zero external runtime dependencies - system sans and tabular monospace typography, inline drawn SVGs, and vanilla CSS custom properties - the interface runs cleanly under strict local CSP constraints across desktop monitors and 390px remote mobile check-ins.

**Key Characteristics:**
- Interrupt-first spatial hierarchy: waiting decision gates command the top of the workspace when present and collapse to a compact emerald confirmation bar when clear.
- Human titles over opaque identifiers: issue numbers and PR badges anchor monospace metadata while real human-readable titles and multi-PR progress chains take primary visual weight.
- Connected Swiss Perlschnur route tracks: every multi-PR ticket visualizes merged, active, gated, failing, and remaining pull requests as an interactive horizontal station line.
- Zero-dependency architectural precision: crisp 3px and 4px corners, 2px structural panel header rules, flat tonal elevation, system neo-grotesque and tabular monospace type, and unified 16x16 inline SVG iconography.
- Dual-theme operational parity: Nocturnal Station Slate default paired with an Architectural Swiss Daylight light mode and a comfortable/compact row density switch.

## Colors

### Primary

Signal Ochre drives human attention and primary operational commitments across both themes.

- **Signal Ochre (`signal-ochre`, `#e29524` dark / `daylight-ochre`, `#d98218` light):** Primary call-to-action fills (`Launch`, merge-gate answers), active navigation tab underlines, warning tab counts, text selection highlight, input caret color, and merge-gated station rings.
- **Ochre Ink (`ochre-ink`, `#170f00`):** High-contrast dark ink paired on solid Signal Ochre fills to maintain WCAG AA+ contrast on buttons and warning pills.
- **Ochre Text (`ochre-text`, `#f5bd63`):** Luminous amber text used for gate category labels, status pills, and station identifiers on dark tinted surfaces.
- **Ochre Surface & Border (`ochre-surface`, `#1d170c` / `ochre-border`, `#5e4419`):** Tinted ground and four-sided 1px frame for waiting decision cards and gate pills.

### Secondary

Three functional signal families complete the Swiss railway dispatch semaphore without competing with Signal Ochre.

- **Emerald - Merged & Ready (`signal-emerald`, `#2da46e` dark / `daylight-emerald`, `#1c8253` light):** Solid merged station circles, completed Perlschnur track rails, ready-to-merge rings, healthy telemetry dots, and the all-clear confirmation bar (`emerald-surface` `#0f1d16`, `emerald-text` `#6ee0a8`, `emerald-border` `#224f3a`).
- **Signal Blue - In Flight & Focus (`signal-blue`, `#46a0e6` dark / `daylight-blue`, `#1a6cb0` light):** Keyboard `:focus-visible` rings, active running or in-review PR station rings, selected DAG dependency traces, and interactive hyperlinks (`blue-surface` `#0f1e2e`, `blue-text` `#7cc4fa`, `blue-border` `#244a6e`).
- **Vermilion - Blockers & Failures (`signal-vermilion`, `#e5534b` dark / `daylight-vermilion`, `#cc3329` light):** CI-failing stations, blocker decision cards (`vermilion-surface` `#241214`, `vermilion-text` `#ff948d`, `vermilion-border` `#6b2929`), and destructive or escalation primary CTAs (`vermilion-fill` `#c93830` with `pure-white` `#ffffff` ink).

### Neutral

Nocturnal Station Slate establishes a disciplined five-step dark tonal ladder while Architectural Swiss Daylight flips the workspace ground to paper-cool off-white under the same dark railway header.

- **Dark Canvas & Surfaces:** `station-slate` (`#0c1017`) forms the base viewport; `telemetry-slate` (`#101621`) anchors the live metrics strip; `panel-slate` (`#121822`) holds primary operational panels; `panel-elevated` (`#161e2b`) recesses Perlschnur route tracks, drawer headers, and secondary bars; `panel-hover` (`#1b2434`) highlights interactive row hover and selection; `code-slate` (`#1c2738`) backs monospace reference badges and pressed segmented controls.
- **Command Header & Rules:** `railway-navy` (`#131c2b` dark / `daylight-navy` `#182232` light) anchors the sticky top bar with `ink-header` (`#f4f7fb` / `#ffffff`) and `ink-header-muted` (`#9eb0c9`). `structural-rule` (`#3a5378` dark / `#182232` light) renders the signature 2px panel header underline.
- **Borders & Rails:** `hairline-border` (`#233044` dark / `daylight-border` `#d5dce5` light) divides panels and ledger rows; `control-border` (`#4c6282`) frames interactive buttons, inputs, popovers, and drawers; `track-rail` (`#2c3b52`) links unmerged station nodes.
- **Ink Ladder:** `ink-primary` (`#eef2f7` dark / `daylight-ink` `#14181f` light) for titles and active values; `ink-secondary` (`#a3b3c9` dark / `daylight-ink-secondary` `#465366` light) for metadata and summaries; `ink-muted` (`#8395ae`) for tertiary timestamps, placeholders, and subtle icons.
- **Daylight Ground:** `daylight-ground` (`#f4f6f4`) surrounds crisp `pure-white` (`#ffffff`) panels so light mode retains Swiss editorial contrast without harsh glare.

### Named Rules

- **The Ochre Interrupt Rule.** Reserve Signal Ochre (#e29524 in dark mode, #d98218 in daylight mode) strictly for human-action interrupts and active operational focal points: waiting decision gate cards, primary approval and Launch buttons, active navigation tab underlines, warning count badges, and merge-gated station rings. Never spend ochre on decorative chrome, passive labels, or completed states.
- **The Full-Border Framing Rule.** Frame waiting gate cards, blocker alerts, and the all-clear confirmation bar with complete four-sided 1px tinted borders (#5e4419 ochre, #6b2929 vermilion, #224f3a emerald) over deeply tinted surfaces. Never use thick one-sided left accent stripes, which break Swiss grid alignment and introduce visual noise.
- **The Triple-Encoded Status Rule.** Express every operational state through three simultaneous cues: semantic hue, distinct geometric shape or icon (diamond for merge gates, warning triangle for blockers, check for ready/merged, play triangle for building workers, clock for review, and solid/ringed/dashed Perlschnur nodes), and explicit uppercase monospace wording. Never rely on color alone.

## Typography

t3 board uses strictly zero-dependency system typography so local `t3cm` serving renders instantaneously under CSP policies that forbid remote font requests.

- **Display & UI Sans (`--font-sans`):** `system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`
- **Technical Monospace (`--font-mono`):** `ui-monospace, "SF Mono", "Cascadia Mono", Menlo, Consolas, monospace`
- **Character:** Authoritative Swiss departure-board legibility. Extra-bold tight-tracked headings (`800`, `-0.015em` to `-0.02em`) establish clean vertical landmarks, medium-bold titles (`700`) make human issue and PR descriptions immediately scannable, and uppercase technical monospace locks identifiers, commit SHAs, and status pills into rigid alignment.

### Hierarchy

| Role | Font Family | Size | Weight | Line Height | Letter Spacing | Primary Usage |
| --- | --- | --- | --- | --- | --- | --- |
| `headline` | `--font-sans` | `17px` | `800` | `22px` | `-0.015em` | Brand title (`-0.02em`), panel headers, drawer headers |
| `title` | `--font-sans` | `15.5px` | `700` | `1.35` | `0em` | Decision gate headlines, active issue titles (`14.5px` for PR rows) |
| `body` | `--font-sans` | `14px` | `400` | `1.45` | `0em` | Base copy, issue sub-summaries (`13px`), inspector prose (`13.5px`) |
| `label` | `--font-sans` | `13px` | `600` | `1.3` | `0em` | Buttons, navigation tabs (`13.5px`), telemetry strip (`12.5px`), form labels |
| `code` | `--font-mono` | `12px` | `700` | `1.4` | `0.02em` | Reference badges (`#254`, `PR #283`), uppercase status pills (`11px`), station codes (`11px`), raw gate payloads |

### Named Rules

- **The Human-Title-First Rule.** Always lead issue rows, PR ledger entries, decision headlines, and inspector headers with the cleaned human-readable title in bold 14.5px-17px sans-serif rather than raw thread handles or bare issue numbers. Monospace reference badges (#254, PR #283) serve as fixed-width scanning anchors beside the title, never as substitutes for it.
- **The Tabular-Telemetry Rule.** Apply font-variant-numeric: tabular-nums (.num) to every live metric, slot fraction, progress percentage, timestamp, tab counter, and 5-second undo countdown so dynamic updates never jitter neighboring columns or shift layout widths.

## Layout

The board is structured as a full-bleed operational shell paired with a `1640px` maximum-width centered workspace (`padding: 20px 24px 40px`, `20px` vertical grid gap).

- **Sticky Command Header (`.topbar`):** Full-width `56px` Railway Navy bar pinned to `top: 0` (`z-index: 40`). Houses the `26x26` railway switch brand mark and `t3 board` title on the left, horizontally scrollable primary view tabs (`Dispatch`, `New Issues`, `Queue & Graph`, `Activity & Settled`) with one canonical count badge each in the center, and compact utility actions (`Ask`, primary `Launch`, density toggle, theme toggle, keyboard shortcuts) on the right.
- **Live Telemetry & Filter Strip (`.telemetry`):** Full-width status rail directly below the header (`padding: 7px 24px`, `12.5px` text) separated by `1px` vertical dividers. Displays connection status and snapshot timestamp, clickable flow-slot utilization (`2 / 2 slots`), landed PR total, an interactive pool routing popover trigger (`Pool claude: claudeSpare 100%`), scenario selector, and the `220px` `/` live search filter.
- **Collapsible Interrupt Zone (`.needs-zone`):** Full-width region at the top of Dispatch. Expands into stacked decision gate cards when count > 0, and collapses into a single-line emerald `All clear` confirmation strip (`padding: 10px 16px`) when count = 0 so active work immediately rises into the top viewport.
- **Two-Column Dispatch Split (`.dispatch-grid`):** Asymmetric `minmax(0, 1.24fr) minmax(0, 1fr)` grid with `24px` column gap (`align-items: start`). The wider left column gives multi-PR Perlschnur station lines breathing room inside `Issues in progress` (plus a quiet blocked-queue footer bar when flow blockers exist). The right column stacks `PRs in progress` (with `Active | Merged | All` segmented control) above a compact `Recently merged` companion panel (`20px` stack gap) when viewing active PRs.
- **Secondary Workspace Views:** `New Issues` uses a single full-width triage ledger with reveal-on-hover actions; `Queue & Graph` stacks the flow queue above a full-width track-grouped dependency map (`270px` DAG cards with click-to-trace blocker/unlock highlighting); `Activity & Settled` uses a `1.3fr / 1fr` split grouping consecutive feed updates per ticket beside settled archives.
- **Density Mode (`data-density`):** Toggled via the header button or `d` shortcut. `comfortable` sets row padding to `16px 20px` (`--pad-row-y: 16px`, `--pad-row-x: 20px`); `compact` tightens row padding to `10px 16px` (`--pad-row-y: 10px`, `--pad-row-x: 16px`) for high-load sessions without shrinking font sizes.
- **Responsive Breakpoints:**
  - `<= 1180px`: Telemetry strip wraps gracefully (`padding: 8px 18px`).
  - `<= 1120px`: `.dispatch-grid` and `.act-grid` collapse into a single column (`minmax(0, 1fr)`).
  - `<= 820px`: `.topbar` drops `.main-nav` onto a full-width second row (`order: 3`), hides text labels on secondary header buttons, expands DAG cards to `100%`, keeps triage action buttons permanently visible (`opacity: 1`), and tightens workspace padding to `14px`.
  - `<= 640px`: Phone layout (`390px` target) abbreviates tab labels (`Issues`, `Graph`, `Activity`), hides secondary telemetry separators and desktop-only shortcut triggers, increases touch targets (`38px` header buttons, `36px` inputs and selects, full-width `38px` primary gate actions), and wraps issue and PR titles cleanly beneath their badges and pills.

## Elevation & Depth

t3 board relies on flat architectural tonal layering rather than drop-shadow depth.

- **Grounded Surfaces (Elevation 0):** The canvas (`#0c1017`), telemetry strip (`#101621`), panels (`#121822`), inset route containers (`#161e2b`), and decision cards (`#1d170c` / `#241214` / `#0f1d16`) sit flush with zero box-shadows, separated cleanly by tonal steps, `1px` hairlines, and `2px` panel header rules.
- **Floating Layers (Elevation 1):** Shadows appear exclusively when an element detaches above the workspace plane - specifically the `360px` pool routing `.popover` (`z-index: 50`), `460px` right-edge slide-over `.drawer` overlays (`z-index: 60`), native `<dialog>` shortcuts modal, and bottom-center `.toast` confirmations (`z-index: 80`). Floating surfaces carry `--shadow-pop`: `0 12px 32px rgba(0, 0, 0, 0.45)` in dark mode and `0 12px 28px rgba(15, 23, 42, 0.14)` in light mode.
- **Modal Scrims:** Drawers dim the underlying workspace with `rgba(6, 9, 14, 0.45)` (`.backdrop`, `z-index: 55`), while native modal dialogs use `rgba(6, 9, 14, 0.55)`.

## Shapes

Geometry stays crisp, rectilinear, and engineered for dense information grids.

- **Architectural Corners:** `3px` (`--r-sm`) for compact inline elements (reference badges, status pills, tab counts, keyboard focus outlines, station hit targets); `4px` (`--r-md`) for structural containers and interactive controls (panels, buttons, inputs, decision cards, route boxes, popovers, dialogs, toasts); `999px` (`--r-pill`) reserved for pill-shaped modifiers where explicitly needed.
- **2px Panel Header Rules:** Every `.panel-head` ends with a `2px solid` bottom rule (`#3a5378` dark / `#182232` light), creating an unmistakable horizontal ledger line across both columns.
- **Perlschnur Station Geometry:** Circular `50%` nodes distinguish workflow state purely by outline and fill:
  - `14px` solid filled circle (`#2da46e`) for merged stops.
  - `16px` hollow ring with `3.5px` stroke for active stops (`#e29524` merge gate, `#2da46e` ready, `#46a0e6` running/review, `#e5534b` CI failing/blocked).
  - `14px` dashed `2px` outline (`#8395ae`) for remaining planned PR stops.
- **Iconography:** All icons are inline drawn `16x16` SVGs with `1.7px-2px` strokes (`15px` `.ico` for controls and alerts, `13px` `.ico-sm` for pills and inline links).

## Components

### Buttons
- **Primary (`.btn.primary`, `.hbtn.primary`):** `32px` height, `0 13px` padding (`0 12px` in header), `4px` radius, `Signal Ochre` background (`#e29524`), `Ochre Ink` text (`#170f00`), `700` weight. Reserved for primary approvals and `Launch`.
- **Secondary (`.btn`, `.hbtn`):** `32px` height, `Panel Slate` (`#121822`) fill with `1px solid` `control-border` (`#4c6282`) on the workspace, or translucent `rgba(255, 255, 255, 0.04)` with `rgba(255, 255, 255, 0.2)` border inside the Railway Navy command bar.
- **Danger (`.btn.danger`):** `Vermilion Fill` (`#c93830`) with `pure-white` (`#ffffff`) bold text, used strictly for primary blocker resolutions.
- **Quiet & Small (`.btn.quiet`, `.btn.sm`):** Borderless transparent ghost actions for secondary utilities (`Pause`, `Reply`, `Details`, `Dismiss`); `.sm` tightens height to `28px` with `0 10px` padding and `12px` type.

### Status Pills
- `.status-pill` renders as an inline-flex `3px`-radius monospace badge (`11px`, `700`, uppercase, `0.02em` tracking, `2px 8px` padding) combining a `13px` semantic icon, explicit label, tinted surface, and `1px` tinted border across six states: `gate`, `ready`, `running`, `review`, `blocked`, and `merged`.

### Panels
- `.panel` encloses tabular ledgers on `Panel Slate` (`#121822`) with a `1px solid` `hairline-border` (`#233044`) and `4px` corners. `.panel-head` locks title (`17px`, `800`) and metadata or `.seg` segmented filters above the `2px` `--rule-top` divider; `.panel-foot-bar` attaches quiet summary strips on `panel-elevated` (`#161e2b`).

### Decision Gate Cards & 5-Second Undo
- `.gate-card` uses a full four-sided `1px` border (`#5e4419` ochre for gates, `#6b2929` vermilion for `.kind-blocker`), `14px 18px` padding, a `28x28` filled icon badge (diamond or warning triangle), human headline, single-line metadata with external PR link, primary option buttons, and quiet `Reply` / `Details` / `Dismiss` triggers.
- Clicking any answer replaces the card body in place with a DOM-stable 5-second `.undo-bar` countdown (`100ms` interval updates on `.undo-fill` and remaining seconds without re-rendering or stealing keyboard focus from the `Undo` button) before committing and emitting a `.toast`.
- When zero decisions wait, `.needs-zone` renders `.all-clear` (`#0f1d16` fill, `#224f3a` border, `#6ee0a8` ink).

### Swiss Railway PR Route Track
- `.route-wrap` sits inset inside each active issue row on `panel-elevated` (`#161e2b`) with a `1px` hairline border. Its `.route-header` reports step progress (`Step 4 of 10 · 3 merged`) alongside a tabular right-aligned percentage (`30% landed`).
- `.route-track` lays out equal-flex `.station` buttons connected by `2px` horizontal pseudo-element rails (`#2c3b52` default, turning `#2da46e` emerald between consecutive merged stations). Each interactive stop displays its `.st-dot`, monospace `.st-code` (`#283`), and `.st-state` (`merge gate`), scales `1.16x` on hover, and opens that exact PR inside the slide-over Detail Inspector on click. Unstarted future steps compress into one non-interactive dashed `.is-remaining` destination stop (`5..10 · 6 left`).

### Inputs & Filters
- `.search-box` houses a `30px`-tall (`36px` mobile) input with left magnifier icon, `0 28px 0 32px` padding, `#e29524` text caret, `/` focus shortcut, and conditional clear button.
- Drawer form controls (`.field input`, `.field textarea`) sit on `station-slate` (`#0c1017`) with `control-border` (`#4c6282`), `4px` radius, and `2px solid` `signal-blue` (`#46a0e6`) `:focus-visible` outlines.

### Navigation
- `.main-nav` implements WAI-ARIA `role="tablist"` with roving `tabindex` (`ArrowLeft`, `ArrowRight`, `Home`, `End`, plus `1`-`4` shortcuts). Selected tabs lock a `3px` bottom `Signal Ochre` bar and full-brightness `#f4f7fb` label; `.nav-count` badges switch to solid ochre with dark ink when waiting decisions exceed zero.

### Slide-Over Drawers, Popovers & Dialogs
- **Detail Inspector & Launch/Ask Drawers (`.drawer`):** `460px` (`max-width: 100vw`) right-aligned surfaces sliding in over `200ms` (`cubic-bezier(0.16, 1, 0.3, 1)`), with `panel-elevated` headers, scrolling sections, external GitHub/T3 links, and automatic focus return on `Esc` or close.
- **Routing Popover (`.popover`):** `360px` floating telemetry panel showing pool member share bars (`#2da46e` ready, `#e29524` reserve) and a 24-hour picks bar chart (`#46a0e6` active buckets).
- **Shortcuts Dialog (`<dialog>`):** Centered `440px` modal documenting all single-key dispatch bindings (`1-4`, `/`, `l`, `a`, `t`, `d`, `j/k`, `Enter`, `Esc`, `?`).

## Do's and Don'ts

### Do:
- Do lead with the Needs-you interrupt zone when any gate or blocker waits, and collapse it into the single-line emerald all-clear bar when the queue is empty.
- Do pair every semantic color with a distinct icon or station geometry and an explicit text label so states stay legible under color-vision deficiency and across both themes.
- Do run interactive gate resolutions through the DOM-stable 5-second Undo countdown bar so operators can catch accidental clicks before a reply dispatches to a live agent thread.
- Do keep secondary surfaces - Launch/Ask composers, pool routing telemetry, detail inspection, and keyboard shortcuts - inside slide-over drawers, popovers, or dialogs rather than crowding the Dispatch canvas.
- Do use tabular numerals (.num) for all timestamps, PR step ratios, utilization shares, and counters, and system fonts only so strict local CSP headers never block rendering.
- Do honor prefers-reduced-motion: reduce by disabling CSS transitions and animations, and restore focus to the triggering element when closing drawers or dialogs.

### Don't:
- Don't add thick one-sided left border bars to decision cards, issue rows, or alert strips - always use full four-sided 1px borders and tonal surfaces.
- Don't use Signal Ochre for decorative highlights, passive tags, or completed items; preserve it exclusively for waiting decisions, active CTAs, and active tab focus.
- Don't apply drop shadows to grounded cards, panels, or table rows; reserve --shadow-pop strictly for floating popovers, slide-over drawers, modal dialogs, and toasts.
- Don't import remote web fonts, external icon libraries, or third-party CSS frameworks that break self-contained single-file serving and production CSP rules.
- Don't duplicate counts or status chips across the top bar, telemetry strip, and panel headers - state each live metric once in its canonical location.
- Don't trap mobile operators with desktop-only hover actions or cramped hit targets - reveal triage buttons permanently below 820px and expand primary controls to 36px-38px below 640px.
