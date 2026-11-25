# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
