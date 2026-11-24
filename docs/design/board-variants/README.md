# Board layout variants

Design references for [issue #316](https://github.com/OttoOps/t3-codemode/issues/316),
which makes the board layout selectable. During the
[#287](https://github.com/OttoOps/t3-codemode/issues/287) design pass,
Impeccable produced these static mocks at 1440x900. They are kept here so
the layouts the board ships can be checked against them.

| File | Layout | What it is |
| --- | --- | --- |
| [`canon.html`](canon.html) | Workspace | "Modern Dev Workspace", the category-standard baseline: a neutral GitHub-dark palette, a 232px left nav with count badges, a top bar with search, status and Launch, one banner per waiting decision, then stacked Issues in Progress and PRs in Progress lists. The board's Workspace layout is built from it. |
| [`pick.html`](pick.html) | Dispatch | "Swiss Railway Dispatch", Impeccable's pick: a navy command header, a telemetry strip, Needs you, issues with PR route tracks, and an Active PRs panel. The board's Dispatch layout (#287) is built from it. |
| [`compare-modern-vs-swiss-dark.html`](compare-modern-vs-swiss-dark.html) | Both | The two side by side in dark mode, with buttons that switch the comparison view and the load state. |

The mocks use sample data from the project's own public issues. The pool
name in the status line is made up.
