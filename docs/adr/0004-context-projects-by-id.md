---
status: accepted
date: 2026-11-20
---

# Key saved context projects by T3 project id

## Context and problem statement

A saved context, added in #71, stored each project as the board's facet value
from #66, which is the item's `owner/repo` when the board knew it, or else the
T3 project's title, lowercased. The board learns a repo some time after a ticket joins, so
a context saved in that window held the title. Once the repo arrived, the
ticket's facet value changed to the repo and the context no longer matched
it. Issue #75 has the history.

Contexts already saved under the old values have to keep working. Nothing
stored maps an old value to a project id, so the only way to resolve one is to
compare it with the repos and titles the board knows at the time.

## Decision drivers

- A context must keep matching a ticket after the ticket's repo arrives.
- Contexts saved before this change keep matching what they matched before,
  without a step the user has to take.
- `wf.cards()` and `wf.feed()` will filter by context later in #75, so the
  service has to be able to apply the same rule.

## Considered options

- Store the project id, and match old values against an item's repo or title
- Store the project id, and rewrite old values on the service once it resolves them
- Keep storing the facet value, and add the title as a second value

## Decision outcome

The chosen option stores the project id and matches old values against the
repo or title.

- A project facet value is the item's T3 project id. An item with no project
  id falls back to its lowercased `owner/repo`, then its lowercased title.
- The page shows each project by its repo, or its title while the repo is
  unknown.
- An item matches a saved value when the value equals its project id
  exactly, or equals its repo or project title ignoring case. The page takes
  that repo and title from any item on the board with the same project id,
  so a ticket still matches an old repo value while only its feed rows carry
  the repo. The second test stands in for a migration, because an old value
  keeps matching, and an old title now matches after the repo arrives as
  well.
- An item with a repo and no project id, such as a new issue, also matches
  a picked id when any item on the board ties that id to the same repo.
- The service trims project values and no longer lowercases them, since ids
  are case-sensitive. It sorts and dedupes them as before.
- The Project facet shows an old value as a tick on the project it matches,
  not as a separate entry. Unticking that project drops the old value too, so
  the next save writes the id. An old value that matches no project still
  shows as its own entry.

### Consequences

- Good: a context saved with an id matches its tickets whatever the board
  learns later about their repo.
- Good: no stored context is rewritten, and no migration has to run.
- Bad: an old value stays in a context until someone edits that project in
  it, so the matching rule has to cover old values for as long as such
  contexts exist.
- Bad: an old title value matches every project with that title.
- Neutral: the filter that `wf.cards()` and `wf.feed()` get later applies the
  same rule on the service.

## Pros and cons of the options

### Store the project id, and match old values against the repo or title

- Good: one rule covers old and new values, on the page and on the service.
- Bad: old values never go away on their own.

### Store the project id, and rewrite old values on the service once it resolves them

- Good: stored contexts end up holding only ids.
- Bad: the service learns repos lazily and may never resolve a title, so
  some contexts would stay unresolved and need the matching rule anyway.
- Bad: a rewrite is a write nobody asked for, racing edits from open pages.

### Keep storing the facet value, and add the title as a second value

- Good: the page needs no id handling.
- Bad: it still breaks when a project is renamed or its repo changes.

## More information

Issue #75. `placeOf`, `placeKey` and `placeHit` in `src/board.html` hold the
rule, and `check_spec` in `src/contexts.rs` sets the stored form. The Filtering
section of `docs/src/board.md` describes the behavior.
