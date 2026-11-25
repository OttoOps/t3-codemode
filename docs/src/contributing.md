# Contributing

Bug reports, fixes, tests, documentation, ports to other platforms and new
upstream integrations are welcome. Start with the
[contribution guide](https://github.com/OttoOps/t3-codemode/blob/main/.github/CONTRIBUTING.md).
It covers the DCO signoff every commit carries (`git commit -s`), how pull
requests merge, and what the project will not take.

Report vulnerabilities privately through the
[security policy](https://github.com/OttoOps/t3-codemode/blob/main/.github/SECURITY.md).
Participation follows the
[Code of Conduct](https://github.com/OttoOps/t3-codemode/blob/main/.github/CODE_OF_CONDUCT.md).
The code is under the
[MIT license](https://github.com/OttoOps/t3-codemode/blob/main/LICENSE).

## Checks

Run the checks CI runs, from the repository root:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests/skills -v
shellcheck deploy/*.sh              # when you change an installer
actionlint                          # when you change a workflow
```

To change the installers or workflows, also install
[ShellCheck](https://www.shellcheck.net/) and
[actionlint](https://github.com/rhysd/actionlint). `deploy/install.ps1` must run
on Windows PowerShell 5.1. The contribution
guide shows how to try `install.sh` against a scratch `HOME`.

The test suite runs against an in-process fake T3 and fake MCP servers, so most
work needs no T3 Code server. `tests/fixtures/t3-tools.json` is a snapshot of
T3's 74 tools. At runtime the catalog comes live from `tools/list`, refreshed
every 10 minutes and cached in the store for when T3 is down.

A hidden `bench` command measures latency and RSS against the in-process fake
T3:

```bash
t3cm bench -n 1000 --tools tests/fixtures/t3-tools.json
```

### Chaos testing

`tests/all/pr_chaos.rs` runs seeded Swarm-weighted sequential chaos and
concurrent race tests against the shadow model, and it replays every committed
NDJSON op list in `tests/chaos_corpus/*.ops`.

| Variable | Effect |
| --- | --- |
| `CHAOS_SEED=<n>` | Run one seed in `seeded_sequential_chaos` or `concurrent_chaos`. |
| `CHAOS_SEED_START=<n>`, `CHAOS_SEEDS=<k>` | Run `k` seeds starting at `n`. Defaults are 60 sequential seeds and 30 concurrent seeds. |
| `CHAOS_REACH=1` | Print `ReachSnapshot` and `OpStats` execution coverage after sequential chaos. |
| `CHAOS_SHRINK=1` | On a sequential failure, run 1-minimal `ddmin` and value simplification, write `tests/chaos_corpus/<hash>.ops` or `$CHAOS_CORPUS_DIR/<hash>.ops`, and print a ready-to-paste `#[tokio::test]`. |
| `CHAOS_OPS=<path>` | Replay one `.ops` file in `pr_chaos::seeded_sequential_chaos`. Combines with `CHAOS_SHRINK=1`. |
| `CHAOS_SENTINEL=<name>` | With `--features test-sentinels`, enable one planted sentinel during seeded or `CHAOS_OPS` runs. |

Examples:

```bash
# Shrink a failing seed into tests/chaos_corpus/<hash>.ops
CHAOS_SHRINK=1 CHAOS_SEED=42 cargo test --locked --test all pr_chaos::seeded_sequential_chaos -- --nocapture

# Replay one saved .ops file
CHAOS_OPS=tests/chaos_corpus/<hash>.ops cargo test --locked --test all pr_chaos::seeded_sequential_chaos -- --nocapture

# Run the planted-sentinel acceptance and shrink checks
cargo test --locked --features test-sentinels --test all sentinel_acceptance -- --nocapture
```

`pr_chaos::chaos_corpus_replays_cleanly` replays every `.ops` file committed
under `tests/chaos_corpus/` during plain `cargo test --locked`. The NDJSON
loader ignores blank lines and `#` comment lines, so a file can carry
`# source:` and `# failure:` headers above its ops.

### Nightly chaos

`.github/workflows/chaos-nightly.yml` runs every night on Linux, and on demand
from the Actions tab. It has three parts:

- **Swarm sweep.** 200,000 sequential seeds in a release build, split across
  the runner's cores with each shard's temporary files on `/dev/shm`. The first
  seed is the last eight digits of the run ID times the seed count, so runs
  sweep different ranges until those digits repeat. A
  failing shard's seed is shrunk with `CHAOS_SHRINK=1`, and the
  `chaos-failures` artifact holds one `<hash>/` folder per distinct
  shrunk `.ops` file. Each folder has the `.ops` file, the shrink log, a replay
  trace and the seeds that shrank to it. The job summary lists each hash and
  the `Reach` hit rates per 1,000 seeds.
- **Coverage and reach.** `cargo-llvm-cov` line and region coverage of `src/`
  by the `pr_chaos` tests, with the text summary and the `Reach` table in the
  job summary and an `lcov.info` in the `chaos-coverage` artifact.
- **Mutation grading.** `cargo-mutants`, configured in `.cargo/mutants.toml`,
  mutates `src/reconcile.rs`, `src/wf_pr.rs`, `src/merge.rs`, `src/board.rs`
  and `src/wf.rs` and grades each mutant with the `pr_chaos` tests only, so a
  survivor is a change the chaos oracle does not notice. Eight shards run in
  parallel, and the `mutants-survivors` artifact collects `missed.txt` and
  `timeout.txt`.

A dispatch can set the seed count, the first seed, and a planted sentinel. A
sentinel builds with `test-sentinels`, so the sweep fails on purpose and shows
the shrink path working. To promote a failure, download its `.ops` file, check
that it replays, and commit it under `tests/chaos_corpus/` with the fix:

```bash
CHAOS_OPS=<hash>.ops cargo test --locked --test all pr_chaos::seeded_sequential_chaos_holds_all_invariants -- --exact --nocapture
```

The workflow does not file issues. A later job could read the hashes from the
`chaos-failures` artifact and open or comment on one tracking issue per hash,
with `issues: write` granted to that job alone.

### Diff mutation check

`.github/workflows/mutants-diff.yml` runs the same mutation grading on a pull
request, limited to the lines the PR changes. It starts only when the PR
touches a Rust file under `src/`, `.cargo/mutants.toml`, or the workflow and
its scripts, and its first step skips the rest when none of the changed files
is in `.cargo/mutants.toml`'s `examine_globs`. Otherwise it diffs the merge
commit against the base it was merged onto, runs
`cargo mutants --in-diff` with the `mutants` profile and the `pr_chaos` tests,
and writes the caught, missed, timeout and unviable counts per file to the job
summary, with each missed and timed-out mutant listed below the table. The
`mutants-diff` artifact holds the same files.

The check is advisory. It runs in its own workflow, outside the required
`check` job in `ci.yml`, and passes when mutants survive: it fails only when
cargo-mutants itself fails, such as when the unmutated tree does not build.
Expect survivors in `src/board.rs` rendering code, which the chaos tests do not
reach. A survivor in code your PR adds is worth a look: either a `pr_chaos`
invariant should notice the change, or the line is outside what the chaos
oracle checks. It needs no secrets and runs with `contents: read`, so it works
the same on pull requests from forks.

## This site

`docs/build.sh` generates the [API reference](api.md) page from `t3cm types`,
then builds the site into `docs/book/` with mdBook 0.5.4. CI builds it on every
pull request, checks its links with lychee, and deploys it to GitHub Pages from
`main`. The pages are Markdown under `docs/src/`, and `docs/src/SUMMARY.md`
lists them in order.
