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

### Fault rules

`FakeT3` and `FakeGitHub` fail or hold calls by rule. A rule names an op, an
optional target, and the matching call it fires on. The op is a T3 tool such
as `t3_thread_send`, or a GitHub operation such as `read` or
`mergePullRequest`, and `*` matches every call. The target is a thread id on
T3, or `owner/name#number` on GitHub:

```rust
h.github.fault(Rule::on("read").pr("acme/widgets", 7).call(1).fail(Failure::refused("try again")));
h.fake.fault(Rule::on("t3_thread_send").fail_after_effect(Failure::down("reply lost")));
```

A gate holds one call while the test runs another, on a harness with no other
`t3_thread_send` rule:

```rust
let gate = Gate::new();
h.fake.fault(Rule::on("t3_thread_send").call(1).gate(&gate));
let parked = h.fake.call("t3_thread_send", json!({"threadId": "mcp:a", "message": "first"}));
let race = async {
    gate.arrived().await;
    h.fake.call("t3_thread_send", json!({"threadId": "mcp:b", "message": "second"})).await.unwrap();
    gate.release();
};
let (parked, ()) = tokio::join!(parked, race);
```

- `fail` returns the error before the call takes effect.
- `fail_after_effect` applies the call, then returns the error.
- `gate` parks the call until `release()`, and `arrived()` waits for it to
  park. A call that matches two gates arrives at both before it waits on
  either.

The op `reconcile.pr` targets one pull request inside the batched reconcile
read, and a `Failure::typed` there keeps its type on that pull request's error.
After `alias_repo`, a pull request target matches the repository's old name and
its new one. `set_jitter(Some(seed))` adds seeded `yield_now()` calls to every fake
call, and the concurrent chaos tests pass their seed.

A hidden `bench` command measures latency and RSS against the in-process fake
T3:

```bash
t3cm bench -n 1000 --tools tests/fixtures/t3-tools.json
```

### Thread lifecycle

`FakeT3` reads the test clock, `H::clock`, so a thread's run lasts in virtual
minutes:

- `start_run(id, Some(minutes))` sets the thread `running`. Once the clock
  passes that many minutes it reads `idle`, with `updatedAt` at the run's end.
  `None` runs until `finish_run`.
- `run_on_send(id, minutes)` starts a run on every `t3_thread_send` to the
  thread, or pushes out the end of the run already going.
- `finish_run(id, status)` ends the run as `idle`, `completed`, `failed`,
  `interrupted` or `cancelled`.
- `archive(id)` archives the thread, and `remove(id)` deletes it.

```rust
h.fake.run_on_send(WORKER, 20);
h.wf(TICKET, "reply", json!({"to": WORKER, "body": "rebase"})).await; // Sent. The worker runs for 20 minutes.
h.wf(TICKET, "reply", json!({"to": WORKER, "body": "lint"})).await; // Queued in the outbox.
h.tick(21).await; // The run has ended, so the watcher delivers the queued reply.
```

`tests/all/lifecycle.rs` drives each watcher branch this reaches and checks its
`Reach` counter: `OutboxHeld`, `StallReported`, `IdleEscalated`,
`BadEndNudged`, `WorkerArchived`, `WorkerThreadGone`, `TicketArchived` and
`TicketThreadGone`.

### GitHub pull request state

`FakeGitHub` keeps each pull request's state and changes it the way GitHub
does:

| Call | Effect |
| --- | --- |
| `push(repo, n, head)` | Moves the head, starts a pending `ci` check on it, and takes the pull request out of the merge queue. |
| `set_status(repo, n, merge_state, draft)` | Sets the merge state and the draft flag. |
| `set_checks(repo, n, checks)` | Replaces the check runs on the head. |
| `set_queue(repo, n, enabled, queued)` | Turns the merge queue on or off for the pull request, and adds it to the queue or takes it out. |
| `drop_from_queue(repo, n, reason, at)` | Takes it out of the queue with a timeline reason. |
| `advance_queue(repo, at)` | Merges the pull request at the front of the queue and moves the rest up. |
| `finish(repo, n, state, at, commit)` | Merges or closes it and takes it out of the queue. |
| `reopen(repo, n)` | Reopens a closed pull request. |
| `reassign_id(repo, n)` | Gives it a new node ID, so a merge by the old ID fails. |

`enqueuePullRequest` and `mergePullRequest` work on the same queue, and queue
positions follow queue order. The chaos tests change GitHub only through these
calls, and after each op `pr_chaos` checks that the fake matches the shadow
model.

### Chaos testing

`tests/all/pr_chaos.rs` runs seeded Swarm-weighted sequential chaos and
concurrent race tests against the shadow model, and it replays every committed
NDJSON op list in `tests/chaos_corpus/*.ops`. Among its ops, a ticket calls
`wf.merge`, the merge queue merges its front pull request, and a worker's run
ends completed, failed, interrupted or cancelled.

| Variable | Effect |
| --- | --- |
| `CHAOS_SEED=<n>` | Run one seed in `seeded_sequential_chaos` or `concurrent_chaos`. |
| `CHAOS_SEED_START=<n>`, `CHAOS_SEEDS=<k>` | Run `k` seeds starting at `n`. Defaults are 150 sequential seeds and 30 concurrent seeds. |
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
`check` job in `ci.yml`, and it passes when mutants survive. It fails only when
cargo-mutants itself fails, such as when the unmutated tree does not build.
Expect survivors in `src/board.rs` rendering code, which the chaos tests do not
reach. A survivor in code your PR adds is worth a look: either a `pr_chaos`
invariant should notice the change, or the line is outside what the chaos
oracle checks. It needs no secrets and runs with `contents: read`, so it works
the same on pull requests from forks.

### Fuzzing

`fuzz/` holds a [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) target,
`decode_ops`, that decodes libFuzzer's bytes into a chaos program with
`OpWeights::UNIFORM` and runs it through the `pr_chaos` sequential executor and
oracle on the in-memory store. It includes `tests/common/mod.rs` and
`tests/all/pr_chaos.rs` with `#[path]`, so it checks the same invariants as the
Swarm sweep, and it is its own workspace, so `cargo test` never builds it. It
needs a nightly toolchain and `cargo install --locked cargo-fuzz@0.13.2`:

```bash
cd fuzz
cargo +nightly fuzz run decode_ops -- -max_total_time=300 -max_len=768
```

The default build uses AddressSanitizer, which also instruments the bundled
SQLite and QuickJS C code on `x86_64` Linux. `-s none` runs two to three times
faster, which suits a session that hunts oracle failures and can do without the
memory checks. `FUZZ_FILE_STORE=1` switches to the file-backed store, and
`FUZZ_DUMP_OPS=1` prints each input as an `.ops` file, which turns a crasher
into a `CHAOS_OPS` replay for the shrinker.

`.github/workflows/fuzz-nightly.yml` runs the target for 30 minutes every night
on Linux, from the cached corpus plus 16 random 768-byte inputs, keeps the
corpus in the Actions cache from one run to the next, and uploads crashers, with
their `.ops` files, as the `fuzz-crashers` artifact.
`docs/research/decode-ops-fuzzing-2026-11.md` has the exec/s measurements and
the sanitizer results.

## This site

`docs/build.sh` generates the [API reference](api.md) page from `t3cm types`,
then builds the site into `docs/book/` with mdBook 0.5.4. CI builds it on every
pull request, checks its links with lychee, and deploys it to GitHub Pages from
`main`. The pages are Markdown under `docs/src/`, and `docs/src/SUMMARY.md`
lists them in order.
