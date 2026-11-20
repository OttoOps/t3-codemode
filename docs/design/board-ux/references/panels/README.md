# Panel layout references

Sixteen screenshots of web apps and IDEs whose panels can be moved, collapsed
and resized. Blind judges score the board's panel layout against them, so each
image is chosen for the feedback it shows while a panel is being handled: the
drop overlay, the insertion slot, the drag preview, the resize handle, the
collapse chevron, the collapsed rail.

## How they were collected

Collected on 2026-11-20 with Playwright (headless Chromium, 1600x1000
viewport), following the rules in [`../README.md`](../README.md). Nothing was
captured while signed in. Live captures come from public pages that need no
login and show staged or synthetic data: vscode.dev with an untitled scratch
file, the Dockview demo (market data labeled "Demo Account"), the Golden Layout
home page (fictional companies such as Acme and LexCorp), and the Grafana Play
demo stack. Grafana Play lets anonymous visitors switch a dashboard into edit
mode; the edits were never saved. The JetBrains images are the vendor's own
documentation images, downloaded rather than recreated; one is a single frame
taken from the animated GIF on the docs page. No image shows an email address
or an identifiable face, so nothing needed covering.

Mid-drag stills were taken with `mouse.down` and `mouse.move` and no
`mouse.up`, so the drop feedback is on screen. Headless Chromium does not draw
the mouse pointer, so the live captures show no cursor; where a pointer
position matters, the entry says where it was. Each image was resized to at
most 1600 px wide and saved as WebP at quality 78. The Golden Layout captures
are clipped to the live layout inside the home page's monitor mockup
(1060x620).

The images remain their owners' property, kept here under fair use for design
commentary with attribution to the vendor named in each entry. None of them is
covered by this repository's MIT license.

## Index

| File | Product and view | What it shows about moving, collapsing or resizing panels | Source page | Basis |
| --- | --- | --- | --- | --- |
| [vscode-sash-hover.webp](vscode-sash-hover.webp) | VS Code for the Web, resting layout: Explorer side bar, editor, bottom panel (Problems) | Resting three-region layout; the side bar / editor sash lit after a hover | https://vscode.dev | live capture, public page |
| [vscode-sash-resize.webp](vscode-sash-resize.webp) | VS Code for the Web, same layout | Mid-resize of the side bar: sash stays lit, side bar content reflows live | https://vscode.dev | live capture, public page |
| [vscode-drag-panel-to-sidebar.webp](vscode-drag-panel-to-sidebar.webp) | VS Code for the Web, Problems tab dragged over the side bar | Drop overlay: lower half of the Explorer view shaded as the split target | https://vscode.dev | live capture, public page |
| [vscode-drag-editor-split.webp](vscode-drag-editor-split.webp) | VS Code for the Web, editor tab dragged to the editor's right edge | Drop overlay: right half of the editor group shaded, "split right" | https://vscode.dev | live capture, public page |
| [vscode-view-sections.webp](vscode-view-sections.webp) | VS Code for the Web, Explorer side bar sections | Expanded and collapsed view sections, chevrons, hover actions on a section header | https://vscode.dev | live capture, public page |
| [dockview-drag-split.webp](dockview-drag-split.webp) | Dockview demo, "News" tab dragged over the "Correlation" group | Drop overlay: right half of the target group dimmed | https://dockview.dev/demo | live capture, public page |
| [dockview-floating-group.webp](dockview-floating-group.webp) | Dockview demo, "Tech View" floated with Shift+click | Docked grid plus a floating group with a drop shadow; collapsed edge rails "Explorer" and "Outline" | https://dockview.dev/demo | live capture, public page |
| [dockview-resize.webp](dockview-resize.webp) | Dockview demo, mid-resize of the first column sash | Live reflow while resizing: FX Rates tiles drop from three columns to two | https://dockview.dev/demo | live capture, public page |
| [goldenlayout-drag-split.webp](goldenlayout-drag-split.webp) | Golden Layout home page demo, "Layout" tab dragged over the chart stack | Dashed drop-zone rectangle on the right half of the target, plus a full drag proxy of the panel | https://golden-layout.com/ | live capture, public page |
| [goldenlayout-drag-tab-insert.webp](goldenlayout-drag-tab-insert.webp) | Golden Layout home page demo, same drag over the "Fnts 100" tab strip | Tab insertion slot: dashed box in the tab strip next to the existing tab | https://golden-layout.com/ | live capture, public page |
| [grafana-rows-collapsed-expanded.webp](grafana-rows-collapsed-expanded.webp) | Grafana Play, "Kubernetes / Compute Resources / Cluster" dashboard | Two rows expanded (chevron down), ten collapsed to one-line headers (chevron right) | https://play.grafana.org/d/efa86fd1d0c121a26444b636a3f509a8/kubernetes-compute-resources-cluster | live capture, public demo |
| [grafana-edit-resize.webp](grafana-edit-resize.webp) | Grafana Play, same dashboard in edit mode | Mid-resize of the "CPU Usage" panel by its corner handle; panel snaps to the grid | https://play.grafana.org/d/efa86fd1d0c121a26444b636a3f509a8/kubernetes-compute-resources-cluster | live capture, public demo |
| [grafana-edit-drag.webp](grafana-edit-drag.webp) | Grafana Play, same dashboard in edit mode | Mid-drag of the "Memory Utilisation" stat panel: grid placeholder, dashed floating panel, neighbors reflow | https://play.grafana.org/d/efa86fd1d0c121a26444b636a3f509a8/kubernetes-compute-resources-cluster | live capture, public demo |
| [jetbrains-tool-windows-docked.webp](jetbrains-tool-windows-docked.webp) | IntelliJ IDEA, docs figure of the main window with four tool windows docked | Docked tool windows (Project, Structure, Run, Maven) around the editor, numbered callouts | https://www.jetbrains.com/help/idea/tool-windows.html (image: https://resources.jetbrains.com/help/img/idea/2026.3/ij_tool_windows_new.png) | vendor image, fair use |
| [jetbrains-tool-window-bars.webp](jetbrains-tool-window-bars.webp) | IntelliJ IDEA, docs figure of the tool window bars | Every tool window collapsed to icons on the left and right bars (highlighted in green) | https://www.jetbrains.com/help/idea/tool-windows.html (image: https://resources.jetbrains.com/help/img/idea/2026.3/tool_window_bar_and_buttons_new_ui.png) | vendor image, fair use |
| [jetbrains-drag-tool-window.webp](jetbrains-drag-tool-window.webp) | IntelliJ IDEA, frame 121 of 238 from the docs GIF of moving tool windows | Tool window icon dragged to the left bar: dashed slot in the bar, target area tinted, "Move to Left Top" tooltip | https://www.jetbrains.com/help/idea/manipulating-the-tool-windows.html (image: https://resources.jetbrains.com/help/img/idea/2026.3/ij_moving_tool_windows.gif) | vendor image, fair use |

## What each image shows

**vscode-sash-hover.webp.** This is also the resting layout: activity bar,
Explorer side bar, a TypeScript file in the editor, and the bottom panel open
on Problems. The pointer rests on the side bar's right edge. After VS Code's
short hover delay, the sash draws a 4 px accent-blue line the full height of
the side bar. At rest the sash is invisible except for a small grip of dots
at its midpoint, visible on the panel's top edge here. The still cannot show
the resize cursor or the hover delay.

**vscode-sash-resize.webp.** The same sash mid-drag, about 120 px to the
right. The blue line stays on for the whole drag, and the layout reflows
live: the Explorer's buttons and text rewrap, and the editor and panel shrink
to match. There is no ghost outline and no preview, only the real layout
following the pointer.

**vscode-drag-panel-to-sidebar.webp.** The Problems tab is dragged out of the
bottom panel and held over the lower half of the Explorer view. VS Code shades
that half in grey-blue to say "drop here to split this view container below".
The rest of the layout stays put. There is no floating drag image of the tab,
because a native HTML5 drag image is not drawn in a headless screenshot. Note
that dropping a panel tab on the editor area shows nothing, because views
cannot dock into the editor area.

**vscode-drag-editor-split.webp.** An editor tab dragged to the right edge of
the editor group. The right half of the group is shaded as the drop overlay,
which previews the split's exact size. Over the middle of the group the
overlay instead covers the whole group and shows a "Hold Shift to drop into
editor" hint; that state was seen but not kept.

**vscode-view-sections.webp.** The Explorer side bar holds three stacked
sections. "No Folder Opened" and "Outline" are expanded (chevron down);
"Timeline" is collapsed to a single-line header pinned at the bottom
(chevron right). The Outline header is hovered, so its actions (collapse all,
more) appear on the right of the header only while hovered. The sash between
two sections was dragged up before the shot to give Outline more room; that
horizontal sash lights up blue on hover just like the vertical one.

**dockview-drag-split.webp.** The "News" tab is dragged from the right column
and held over the right part of the "Correlation" group. Dockview dims the
target half in translucent grey, so the content stays readable through the
overlay. Over the group's center the whole body dims instead (merge as a new
tab). The headless capture does not show the native drag image of the tab.

**dockview-floating-group.webp.** Shift+click on the "Tech View" tab turns
that group into a floating window above the grid, with a drop shadow, its own
tab bar and header actions. The grid it left reflows, and the News group now
fills the right column. The image also shows Dockview's resting chrome: every
group has a tab bar with a "+" button and pop-out and maximize icons, and the
"Explorer" and "Outline" panels sit collapsed as vertical labels on rails at
the left and right edges.

**dockview-resize.webp.** The sash between the first and second columns,
dragged about 145 px to the right and not released. Compare it with the
floating-group image: the Correlation heat map widens and the FX Rates grid
reflows from three tile columns to two. Dockview's sash has no visible hover
or drag state in this demo theme, so the still shows only the reflow, not the
handle. The demo's ticking data also changed between shots.

**goldenlayout-drag-split.webp.** Golden Layout drags with mouse events, not
native drag and drop, so the full feedback is visible. The "Layout" panel
follows the pointer as a proxy (its tab plus a scaled copy of its content),
and a dashed white rectangle marks the drop zone, here the right half of the
"Acme, inc." chart stack. The source slot has already closed up, and the
"Golden" panel now spans the row.

**goldenlayout-drag-tab-insert.webp.** The same drag, held over the "Fnts 100"
tab strip. The drop zone shrinks to a dashed tab-sized box just right of the
existing tab, which says "add as a tab here" rather than "split".

**grafana-rows-collapsed-expanded.webp.** Dashboard rows as collapsible
sections. "Headlines" and "CPU" are expanded (chevron down, panels below, a
thin vertical rule on the left tying the panels to their row). Ten rows are
collapsed to one-line headers with a chevron right and a hairline divider, so
the whole dashboard outline fits on one screen. The row title is the toggle;
a link icon appears beside it on hover.

**grafana-edit-resize.webp.** In edit mode every panel gets a small diagonal
resize grip in its bottom-right corner. Here "CPU Usage" is mid-resize by
that grip: its outline turns orange, it snaps to the 24-column grid, and its
content (legend and chart) reflows live. Edit mode also adds "Add panel" and
"Group panels" buttons under the row.

**grafana-edit-drag.webp.** "Memory Utilisation" is dragged by its header.
The panel floats with a dashed border, and an orange-tinted placeholder shows
where it will land. The other panels move out of the way: the Headlines row
grows to fit the placeholder, and the CPU row is pushed down. The placeholder
carries the panel's hover toolbar (edit, settings, copy, duplicate, delete).

**jetbrains-tool-windows-docked.webp.** JetBrains' reference figure of the
New UI with four tool windows docked: Project (1) and Structure (2) stacked on
the left, Run (3) across the bottom, Maven (4) on the right, all tinted green
by the vendor to mark them. Each has a header with a title, a kebab menu and a
minus-shaped hide button; the hide button minimizes a tool window to its bar icon. The
active icons on the side bars carry a filled background.

**jetbrains-tool-window-bars.webp.** Every tool window hidden: what remains
are the two tool window bars (highlighted in green by the vendor), columns of
icons on the left and right edges, grouped top and bottom with a divider.
This is the IDE's collapsed rail. Clicking an icon reopens its tool window in
place. The still cannot show the hover tooltips that name each icon.

**jetbrains-drag-tool-window.webp.** One frame from the docs animation. A
tool window icon is dragged from the right bar toward the left bar. The left
bar shows a dashed, blue-tinted slot where the icon will land, the area the
tool window will occupy is tinted blue across the editor, and a dark tooltip
names the target, "Move to Left Top". Unlike the overlays above, the feedback
names the destination in words. Later frames show "Move to Bottom Left" with
a slot at the bottom of the bar. The still shows one moment of an animation
that also covers dragging an editor tab out into its own window.

## Captures left out

- **Linear.** The public docs images (for example the issue list on
  https://linear.app/docs/conceptual-model) show a sidebar with collapsible
  group carets and a details-panel toggle icon, but no resize handle, no split
  pane and no collapsed state. VS Code's view sections cover the same pattern
  better, so it was skipped as too weak.
- **Separate resting shots** of VS Code and Dockview were taken and dropped as
  duplicates: `vscode-sash-hover.webp` is the VS Code resting layout plus a
  4 px highlight, and `dockview-floating-group.webp` shows Dockview's resting
  grid.
- **JetBrains "View Mode" and "Move to" menus**
  (https://www.jetbrains.com/help/idea/viewing-modes.html and
  https://www.jetbrains.com/help/idea/manipulating-the-tool-windows.html) were
  downloaded but left out to stay within sixteen. They list Dock Pinned, Dock
  Unpinned, Undock, Float and Window, and the six bar positions.
- **VS Code terminal.** Terminals are not available in the web editor, so the
  bottom panel shows Problems instead.
