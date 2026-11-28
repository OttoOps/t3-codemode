# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.16.1](https://github.com/OttoOps/t3-codemode/compare/v0.16.0...v0.16.1) - 2026-11-28

### Fixed

- close board drawer on back gesture

## [0.16.0](https://github.com/OttoOps/t3-codemode/compare/v0.15.0...v0.16.0) - 2026-11-26

### Added

- ask about the ticket cap in setup
- set the ticket cap in Settings
- show the ticket cap in the board header
- add context menus to board lists
- cap running tickets across every launch

### Fixed

- start the stall clock when a link goes busy
- treat a null link field as left out
- skip a close's wake when its link moved
- keep a peer's close in a link merge
- keep closes and moves intact in link merges
- merge a link's note and report sets
- finish an in-code merge after a crash
- close a link and queue its handoff at once
- draw pr_seen revisions from one sequence
- merge a peer's link changes on save
- keep wf.merge refusals and raced approvals
- lock wf.merge and CAS its merge waits
- skip PRs reconcile stopped tracking

### Other

- excuse only the fallback's own wakes
- keep handoffs out of the fallback excuse
- excuse fallback wakes only at the ticket
- assert each handoff write crashes
- tighten the sweep oracle's excuses
- excuse only a gated crash ticket link
- hold the #365 excuse to PR 7 and its ticket
- match #365 runs by their exact diff
- narrow what the sweep oracle excuses
- confirm all nine fault classes in Reach
- race serve and stdio on one store
- sweep N-th call faults and store crashes
- add a store write crash hook
- allow a null approval on a queued merge
- bound the approval race test's waits
- catch a resurrected wait before quiesce
- describe race tables and gate hooks
- add R1-R4 race tables and burst checks
- add race hooks to reconcile and fakes
- describe fake lifecycle and PR state
- drive chaos through a stateful FakeGitHub
- model thread lifecycle in FakeT3
- note the cap for flow limits above 8

## [0.15.0](https://github.com/OttoOps/t3-codemode/compare/v0.14.0...v0.15.0) - 2026-11-26

### Added

- toggle flow opt-in from the board
- add board context menus on the graph

### Fixed

- keep triage list scroll on repaint

### Other

- describe fake fault rules
- add fault rules and gates to the fakes

## [0.14.0](https://github.com/OttoOps/t3-codemode/compare/v0.13.5...v0.14.0) - 2026-11-25

### Added

- name upstreams in MCP instructions

### Fixed

- accept colon directives after a clause break
- parse inline dep refs without prose hits
- hide advisory graph edges until selected

## [0.13.5](https://github.com/OttoOps/t3-codemode/compare/v0.13.4...v0.13.5) - 2026-11-25

### Other

- record decode_ops fuzzing results
- add nightly decode_ops fuzz workflow
- add a decode_ops libFuzzer target

## [0.13.4](https://github.com/OttoOps/t3-codemode/compare/v0.13.3...v0.13.4) - 2026-11-25

### Other

- add project avatar and social preview

## [0.13.3](https://github.com/OttoOps/t3-codemode/compare/v0.13.2...v0.13.3) - 2026-11-25

### Fixed

- use jiff 1.0 to_zoned_datetime

### Other

- Bump jiff from 0.2.46 to 1.0.2
- Bump unicase from 2.9.0 to 2.10.0
- Bump webpki-roots from 1.0.9 to 1.0.12
- Bump clap from 4.6.8 to 4.6.9
- Bump rusqlite from 0.42.0 to 0.43.0
- split a colon-joined sentence
- summarize diff mutants only after grading
- pin diff prefixes for cargo-mutants
- describe the diff mutation check
- add advisory diff-mutation check on PRs

## [0.13.2](https://github.com/OttoOps/t3-codemode/compare/v0.13.1...v0.13.2) - 2026-11-25

### Other

- describe the nightly chaos workflow
- add nightly chaos sweep and mutation grading
- add cargo-mutants profile and config

## [0.13.1](https://github.com/OttoOps/t3-codemode/compare/v0.13.0...v0.13.1) - 2026-11-25

### Other

- limit GitLab status read to chosen pipeline

## [0.13.0](https://github.com/OttoOps/t3-codemode/compare/v0.12.0...v0.13.0) - 2026-11-25

### Added

- route board views in Workspace nav
- add a server default board layout
- route graph edges through lane gutters
- add Workspace board layout and switch
- edit graph dependencies from inspector
- merge GitLab MRs and merge trains

### Fixed

- keep board header buttons on screen
- guard non-GitHub and failed pending joins
- show pending tickets on the Graph tab
- keep board scrolling inside its panes
- block body thread fallback during triage
- address CodeRabbit review findings

### Other

- add Workspace view captures
- give the layout comparison a dark page
- let skipped and manual GitLab jobs pass
- tie merged-results pipelines to the head
- read merged-results pipelines and all pages
- fix GitLab review findings in skills
- add fake glab and GitLab skill evals
- route tracked GitLab hosts through wf.pr
- add Workspace layout captures
- keep the board layout design mocks
- document GitLab merges in wf.merge
- add ignored live GitLab merge tests
- run wf.merge chaos ops against GitLab

## [0.12.0](https://github.com/OttoOps/t3-codemode/compare/v0.11.0...v0.12.0) - 2026-11-24

### Added

- check for new releases every hour

## [0.11.0](https://github.com/OttoOps/t3-codemode/compare/v0.10.2...v0.11.0) - 2026-11-24

### Added

- verify GitLab merge requests in wf.pr
- move secondary board views to full-width tabs
- reconcile GitLab merge requests
- add Flow settings and setup interview
- port board to Swiss Railway Dispatch shell
- add advisory codeDep module hints
- enrich board titles and issue overlap
- infer PR-stack and file-conflict edges
- sync parsed links to GitHub from board
- wire GitHub dependencies in wf.issue

### Fixed

- tighten board PR rows and merge gates
- bound workspace globs and UTF-8 py escapes
- simplify stopword check for clippy
- isolate JS pkg dirs and regex/division scan
- scope JS bare imports and handle regex quotes
- harden codeDep scanner and curve routing
- suggest service restart when skills are newer
- restart launchd and schtasks services on update
- harden PR-stack and file-conflict analysis
- verify merged base PR commit on same repo
- address PR-stack and conflict edge cases
- pass fresh wait status to exited
- address CodeRabbit major review findings
- strip bold markers before backticked targets
- ignore inline-code keywords in form cue check
- route form tasklist directives and cues
- preserve form lists, parent race, poll queue
- address review findings in board link sync
- replace moved ready card with matching wait
- address CodeRabbit review findings
- keep reconcile idempotent on queue drops

### Other

- document GitLab merge requests in wf.pr
- run wf.pr chaos ops against GitLab
- document GitLab forges and their reconcile
- run the PR chaos suite against GitLab
- cover GitLab merge, train, squash and close
- record the CodeHost interface ADR
- add the CodeHost forge interface
- run untracked post-merge steps once
- watch merges on untracked forges
- recheck shrunk ops and keep full signatures
- shrink chaos failures with ddmin
- add board dispatch design artifacts
- cover sweep pending drops and probe lookup
- widen the sequential chaos op alphabet
- map created sub-issue numbers into split
- use as_chunks_mut in seed_bytes
- support per-PR read error recovery
- decode chaos ops with swarm weights
- add in-memory store for chaos soaks

## [0.10.2](https://github.com/OttoOps/t3-codemode/compare/v0.10.1...v0.10.2) - 2026-11-24

### Fixed

- sync gate_pr when gate cards resolve

## [0.10.1](https://github.com/OttoOps/t3-codemode/compare/v0.10.0...v0.10.1) - 2026-11-24

### Other

- plant runtime sentinels for chaos

## [0.10.0](https://github.com/OttoOps/t3-codemode/compare/v0.9.3...v0.10.0) - 2026-11-24

### Added

- discover graph repos from T3 projects

### Fixed

- use saved project for pending flow tickets
- revalidate aliases and preserve capacity
- harden repo discovery, renames, and capacity

### Other

- check raced forbidden wakes per op boundary
- parse wakes by delivery record boundaries
- require stored body after each wake header
- check GitHub attempts per reconcile pass
- match wake headers to message records
- tighten shadow oracle per review
- add shadow-model oracle to pr chaos

## [0.9.3](https://github.com/OttoOps/t3-codemode/compare/v0.9.2...v0.9.3) - 2026-11-24

### Other

- make chaos replayable and clock-paused

## [0.9.2](https://github.com/OttoOps/t3-codemode/compare/v0.9.1...v0.9.2) - 2026-11-23

### Fixed

- guard code_merged on pre-tracking merges
- clear gate_pr when closing board ticket

### Other

- add Reach reachability assertions to chaos
- add flow auto-launch isolation scenario

## [0.9.1](https://github.com/OttoOps/t3-codemode/compare/v0.9.0...v0.9.1) - 2026-11-23

### Fixed

- fold live ticket since in reconcile

### Other

- state the breaking-change version bump
- release from release-plz release PRs
- unblind pr_chaos dead zones
- add choose and queue_order property tests
