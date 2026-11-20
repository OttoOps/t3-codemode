# Report the GitHub issues you file

The setup-t3-workflow skill installs this rule and rewrites it on each run.
Put your own rules in another file.

When you file a GitHub issue, report it to the t3cm board as part of filing
it. This holds in every thread, whatever skills it runs and whether or not it
has a T3 workflow link. It covers every way of filing: `gh issue create`, the
GitHub MCP, and any skill that files an issue, such as a ticket's follow-up or
a review's follow-up.

Report it with t3cm's `execute` tool:

    async () => wf.issue({ url: "https://github.com/OWNER/REPO/issues/N", title: "TITLE" })

When that tool is not listed, pipe the same function to `t3cm exec -`
(`~/.local/bin/t3cm` when `t3cm` is not on `PATH`). When neither is there,
t3cm is not installed, so skip the report.

- Report the issue once, by its issue URL, as soon as GitHub returns it and
  before you tell the user about it.
- A result with `added: false` means the board already lists the issue or has
  a ticket for it. Nothing more is needed.
- An error that asks for the caller's thread id: pass `thread` to `execute`,
  set to `$T3CODE_THREAD_ID`.
- Reporting is not an offer to the user. You can still offer to start work on
  the issue afterwards.
