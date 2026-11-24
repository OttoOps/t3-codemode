# Report the GitHub issues you file

The setup-t3-workflow skill installs this rule and rewrites it on each run.
Put your own rules in another file.

Every GitHub issue you file carries its dependency metadata and is reported to
the t3cm board as part of filing it. This holds in every thread, whatever
skills it runs and whether or not it has a T3 workflow link, across every
filing path (`gh issue create`, GitHub MCP, or any skill filing a follow-up).

## Canonical `## Flow` block

Include a `## Flow` section in the issue body whenever any dependency, parent,
track, priority, or touched-file path applies (omit lines that do not apply):

```markdown
## Flow
- Parent: #N
- Blocked by: #M, owner/repo#K
- Track: <slug>
- Priority: high|medium|low
- Touches: `path/a`, `path/b`
```

## Wiring and reporting through t3cm

Call t3cm's `execute` tool, or pipe the same async arrow function to
`t3cm exec -` (`~/.local/bin/t3cm` when `t3cm` is not on `PATH`). When neither
is installed, still write the `## Flow` block and skip the t3cm call.

- **Multiple related issues** (an epic with sub-issues, or a phased plan): file
  the whole plan in one `wf.fileIssues` call using symbolic local `id`s:

  ```js
  async () => wf.fileIssues({
    repo: "OWNER/REPO",
    track: "SLUG",
    priority: "high",
    issues: [
      { id: "E", title: "Epic title", body: "..." },
      { id: "P0", title: "Phase 0", parent: "E", touches: ["src/a.rs"], body: "..." },
      { id: "P1", title: "Phase 1", parent: "E", blockedBy: ["P0"], touches: ["src/b.rs"], body: "..." },
    ],
  })
  ```

  `wf.fileIssues` validates the DAG before creating anything, creates issues in
  topological order, writes `## Flow` with real `#N` references, wires native
  GitHub `addSubIssue` / `addBlockedBy` links and labels, and reports each
  issue to the board.

- **Single issue** (`gh issue create`, GitHub MCP, or a follow-up): include
  `## Flow` in the body, then report it and wire native links and labels in one
  `wf.issue` call (since `gh issue create` has no `--blocked-by` or `--parent`
  flags):

  ```js
  async () => wf.issue({
    url: "https://github.com/OWNER/REPO/issues/N",
    title: "TITLE",
    parent: "#EPIC",
    blockedBy: ["#M"],
    track: "SLUG",
    priority: "high",
  })
  ```

  Omit `parent`, `blockedBy`, `track`, `priority`, and `note` when they do not
  apply. Passing any of them wires GitHub's native links and labels
  idempotently (`sync: false` skips GitHub wiring).

- Report each issue once, by URL, as soon as GitHub returns it and before
  telling the user about it.
- `added: false` means the board already lists the issue or has a ticket for
  it; any `parent`, `blockedBy`, `track`, or `priority` you passed still wires
  idempotently.
- An error asking for the caller's thread id: pass `thread` to `execute`, set
  to `$T3CODE_THREAD_ID`.
- Reporting is separate from offering work; you can still offer to start the
  issue afterwards.
