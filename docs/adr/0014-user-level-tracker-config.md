---
status: accepted
date: 2026-11-23
---

# Map ticket trackers in user config

## Context and problem statement

Issue #235 tracks launching board tickets when the issue tracker is Jira or a
self-hosted GitLab instance. Today, `launch_unlocked` in `src/board.rs` accepts
only GitHub issues and maps each issue to a T3 project by matching `owner/repo`
against each project's `github.com` origin remote. That lookup breaks when the
code lives on a self-hosted GitLab host or when tickets live in Jira, because a
bare Jira key such as `PROJ-123` carries no repository URL.

The open question in #235 is where the mapping from a ticket reference to a T3
project belongs. Workflow slots (`review`, `passes`, `credentials`, `delivery`,
and `models`) already resolve per user and per repository, so putting a
`tracker` slot beside them was the first alternative considered.

## Decision drivers

- Launch receives a ticket reference before it knows which repository or T3
  project owns the work, so the mapping must resolve without a repository path.
- Existing workflow slots live in `.t3-workflow.json`, `AGENTS.md`, and
  `~/.config/t3-workflow/config.json`, and every slot lookup begins with the
  caller's repository root.
- A user working across several T3 projects needs one place to map Jira project
  keys and tracker hosts to T3 project ids or titles, with an explicit launch
  argument able to override the map.
- Reloading `config.json` in place already updates runtime board settings
  without restarting the `t3cm` service.

## Considered options

1. Add a top-level `trackers` section to `t3cm`'s user `config.json`, mapping
   Jira project keys under `trackers.jira` and tracker hostnames under
   `trackers.hosts` to T3 project ids or titles, with `LaunchArgs.project`
   overriding the map.
2. Add a new `tracker` workflow slot in `~/.config/t3-workflow/config.json` and
   per-repository `.t3-workflow.json`.
3. Store no mapping in config and require an explicit project argument on every
   non-GitHub launch.

## Decision outcome

Chosen option 1.

- **Resolution order starts with the caller and git remotes.** An explicit
  `project` on `wf.launchTicket` selects that T3 project by id or title and
  skips all automatic lookup. Without an explicit project, GitHub and GitLab
  issue references match the T3 project whose `origin` remote has the same host
  and repository path, including nested GitLab groups.
- **Unmatched references fall back to `trackers` in `config.json`.** Jira
  references check `trackers.jira` by project key first, ignoring case, and
  then check `trackers.hosts` when the reference is a browse URL. GitLab
  references whose repository has no matching `origin` remote also check
  `trackers.hosts` by hostname, while GitHub keeps origin-remote and explicit
  project matching.
- **Bad mappings fail early and unmapped references explain both fixes.** When
  a `trackers` entry names a T3 project that does not exist, launch fails with
  that config path and project name. When no rule matches, launch leaves an
  error card naming the `trackers` config key to add and the explicit `project`
  option.
- **Changes apply on config reload.** `find_project` reads `self.config()` on
  each launch, so editing `trackers` through `t3cm config set` or the Settings
  JSON editor takes effect without restarting `t3cm`.

Option 2 runs in the wrong direction for Launch. Workflow slots answer
repository-scoped questions once the service already has a workspace root, while
Launch starts from a string like `PROJ-123` and must find the workspace root
first. Scanning every T3 project's `.t3-workflow.json` on each launch would add
filesystem reads across every workspace and still require a conflict rule when
two repositories claim the same Jira key.

Option 3 forces the user to pick a T3 project every time they launch a Jira
ticket, even when a Jira project key maps cleanly to one repository. Keeping
the explicit `project` override preserves that escape hatch without making it
mandatory on every launch.

### Consequences

- Good: launching `PROJ-123` or a Jira browse URL resolves straight to its T3
  project with one user config entry, while self-hosted GitLab URLs resolve
  from git `origin` remotes automatically.
- Good: workflow slot resolution stays strictly repository-scoped and needs no
  cross-workspace scan.
- Bad: `trackers` lives in `~/.config/t3-codemode/config.json`, separate from
  the workflow slots in `~/.config/t3-workflow/config.json`, so an operator
  configuring a new Jira project touches `t3cm` config alongside workflow setup.
- Neutral: the board's Settings dialog exposes `trackers` through its JSON
  editor until a later change adds dedicated controls, and a subsequent PR for
  #235 adds a project picker to the Launch form for unmapped references.
