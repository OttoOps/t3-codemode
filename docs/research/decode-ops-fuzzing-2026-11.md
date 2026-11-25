# Coverage-guided fuzzing of decode_ops

Research for [#246](https://github.com/OttoOps/t3-codemode/issues/246), PR 3 of 3. It spikes a libFuzzer target that feeds raw fuzzer bytes into `decode_ops(u, &OpWeights::UNIFORM)` and runs the decoded program through the `pr_chaos` sequential executor and oracle. It records whether AddressSanitizer (ASan) and libFuzzer build and run with the bundled `rusqlite` and with `rquickjs` 0.14 and its `parallel` feature, measures exec/s with the file-backed and in-memory stores, and decides whether a nightly fuzz job is worth wiring.

## Decision

- **Wired.** `.github/workflows/fuzz-nightly.yml` runs the `decode_ops` target nightly on Linux for 30 minutes under ASan, keeps the corpus between runs in the Actions cache, and uploads crashers as the `fuzz-crashers` artifact. The nightly toolchain stays inside that job. `ci.yml` is unchanged.
- **Sanitizer result.** ASan and libFuzzer build and run cleanly on `nightly-2026-10-28`, with no ASan, LeakSanitizer or ODR report in 24 minutes of fuzzing. The SQLite and QuickJS C code is instrumented too, with no extra flags, because `cc` 1.11.2 passes `-fsanitize=address` to GCC when Cargo sets `CARGO_CFG_SANITIZE=address`.
- **Throughput.** 28 exec/s in memory and 24 exec/s on the file-backed store under ASan, 79 and 55 without a sanitizer, one libFuzzer process each, 60 seconds on the same corpus. Over 10 minutes under ASan the rate falls to 15 exec/s as the inputs grow longer. Each input pays a fixed setup cost that caps short inputs near 56 exec/s, and a 768-byte program takes about 160 ms under ASan. The in-memory store runs 17-38% faster under ASan, so the target uses it.
- **Why wire it at this rate.** The Swarm sweep already runs 200,000 seeded programs a night, so it has the volume covered. The fuzzer adds coverage feedback, which keeps the inputs that reach new branches, and ASan over the C code that the sweep runs uninstrumented. This machine settles at 15 to 16 exec/s once the corpus fills with longer programs, and a `ubuntu-latest` runner at 10, so a 30-minute nightly run is about 20,000 inputs. The cached corpus carries what one night finds into the next, so each run starts from the inputs the last one kept.

## Sharing the chaos test code

`decode_ops`, `OpWeights` and the executor live in the `tests/all` test crate, in `tests/all/pr_chaos.rs` and its child modules `ops.rs`, `shadow.rs` and `shrink.rs`, on top of `tests/common/mod.rs`. The fuzz crate includes those files with `#[path]` instead of moving them:

```rust
#[path = "../../tests/common/mod.rs"]
mod common;

#[path = "../../tests/all"]
mod all {
    pub mod pr_chaos;
}
```

The directory `#[path]` on the inline `mod all` keeps the child modules where they are. A file loaded straight through `#[path]` resolves its own `mod ops;` the way a `mod.rs` file would, as `tests/all/ops.rs`, and rustc stops with E0583. Declared inside the inline module, `pr_chaos.rs` resolves `ops.rs`, `shadow.rs` and `shrink.rs` from `tests/all/pr_chaos/` as it does in the test crate, and `crate::common` resolves to the included `tests/common/mod.rs`.

The one change to the test crate is `run_fuzz_bytes` in `tests/all/pr_chaos.rs`, a `pub(crate)` entry that decodes the bytes with `OpWeights::UNIFORM`, optionally prints them as an `.ops` file, and calls `run_sequential_ops`. The test `fuzz_entry_replays_raw_bytes_on_both_stores` keeps it compiled and run on stable, in 0.13 seconds. The fuzz crate is its own workspace under `fuzz/` with its own `Cargo.lock`, so the root `Cargo.lock`, `cargo test --locked` and Tier 1 do not build it or its `libfuzzer-sys` dependency.

The other options cost more:

- **Moving the chaos test code into a shared crate** would move about 7,700 lines of test code and add a crate that Tier 1 builds, to serve one fuzz target.
- **A `cfg(fuzzing)` gate in the test crate** would need a `check-cfg` entry in the root `Cargo.toml` and would leave the entry unbuilt on stable, so a refactor could break it unseen until the nightly run.
- **`bolero`** runs inside `cargo test`, but it would add dev-dependencies to the root crate and its lock file, and Tier 1 would compile them.

`Swarm` weights stay out of the byte stream. `OpWeights::swarm(seed)` derives a profile from a seed outside the `Unstructured` input, and deriving one from the fuzzer's own bytes would let a one-byte mutation switch the whole op mix. libFuzzer would then see each mutation as a different program, so the target uses `OpWeights::UNIFORM`.

## Sanitizer compatibility

| Item | Value |
| --- | --- |
| Toolchain | `rustc 1.101.0-nightly (92ffebece 2026-10-28)`, LLVM 23.1.4 |
| Fuzz driver | `cargo-fuzz` 0.13.2, `libfuzzer-sys` 0.4.13 |
| C compiler | GCC 13.3.0 (Ubuntu 24.04), through `cc` 1.11.2 |
| Crates | `rusqlite` 0.42.0, then 0.43.0 after a rebase, both on `libsqlite3-sys` 0.40.0 with `bundled`, and `rquickjs` 0.14.0 with `futures` and `parallel` |
| Build | `cargo +nightly fuzz build decode_ops` |

`cargo fuzz build` defaults to `--sanitizer address`, a release build with debug assertions. The `decode_ops` crate compiles with:

```text
-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4
-Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-pc-table
-Cllvm-args=-sanitizer-coverage-trace-compares -Cllvm-args=-sanitizer-coverage-stack-depth
-Cllvm-args=-simplifycfg-branch-fold-threshold=0 --cfg fuzzing -Zsanitizer=address
-Cdebug-assertions -Ccodegen-units=1 -C opt-level=3 -C debuginfo=line-tables-only
```

The build finished with no errors. The only warnings were `unexpected_cfgs` for `feature = "test-sentinels"` in the included test files. The fuzz crate now declares its own `test-sentinels` feature, which forwards to `t3-codemode/test-sentinels`.

**The C code is instrumented.** I counted `__asan_report_*` calls in two hot C functions of the linked binary with `objdump -d`:

| Binary | `sqlite3VdbeExec` | `JS_CallInternal` |
| --- | --- | --- |
| `cargo fuzz build` (ASan) | 3241 | 2425 |
| ASan, `CFLAGS="-fsanitize=address -fno-omit-frame-pointer"` | 3241 | 2425 |
| `cargo fuzz build -s none` | 0 | 0 |

An explicit `CFLAGS` changes nothing, because `cc` 1.11.2 already adds `-fsanitize=address -fno-ipa-icf-variables` for GCC and Clang on `x86_64` Linux with glibc whenever Cargo's `CARGO_CFG_SANITIZE` contains `address`. Cargo sets that cfg only with a nightly rustc. GCC's objects link against the ASan runtime from rustc's LLVM. `-fno-ipa-icf-variables` stops GCC from merging identical read-only variables, which that runtime would otherwise report as an ODR violation. No ODR, use-after-free, overflow or leak report appeared in any run below. After `main` moved to `rusqlite` 0.43.0 and `jiff` 1.0, I rebuilt on the rebased tree: the build was clean, both functions kept the same counts, and a 60-second run executed 1,700 inputs at 27 exec/s with no report.

On a target outside `x86_64` Linux with glibc, such as macOS, `cc` leaves the C code uninstrumented, so only the Rust code carries ASan. The nightly job runs on `ubuntu-latest` only.

## Throughput

**Hardware.** AMD Ryzen 7 9700X with 8 cores and 16 threads, 35 GiB RAM, WSL2 on Windows 11 with Linux 6.18.58. Each run is one libFuzzer process on one core, with nothing else fuzzing at the time.

**Corpus.** Every run starts from the same 417 inputs. 397 are small inputs of 1 to 4 bytes, 1,375 bytes in all, that a 60-second warm-up run kept. The other 20 are random 768-byte inputs from `random.Random(246)`, the size of a Swarm sweep seed in `SEED_BYTES`, which decode up to the 70-op cap. `FUZZ_FILE_STORE=1` switches the target to the file-backed WAL store in a temporary directory, and the default is `Store::open_memory()`.

**Fuzzing.** `-max_total_time=60 -max_len=768 -seed=246`, each run on a fresh copy of the corpus:

| Build | Store | Executed | exec/s | Peak RSS |
| --- | --- | --- | --- | --- |
| ASan | in-memory | 1,722 | 28 | 759 MB |
| ASan | file | 1,503 | 24 | 727 MB |
| none | in-memory | 4,863 | 79 | 109 MB |
| none | file | 3,364 | 55 | 104 MB |

**Fixed replay.** Each input ran once with no mutation, timed by wall clock:

| Build | Store | 397 small inputs | 20 inputs of 768 bytes |
| --- | --- | --- | --- |
| ASan | in-memory | 7.08 s (56 /s) | 3.24 s (6.2 /s) |
| ASan | file | 9.74 s (41 /s) | 3.78 s (5.3 /s) |
| none | in-memory | 2.82 s (141 /s) | 0.65 s (31 /s) |
| none | file | 4.98 s (80 /s) | 0.94 s (21 /s) |

The in-memory store is 17-38% faster under ASan and 44-77% faster without a sanitizer. Before its first op runs, each input builds a fresh `H` test fixture with a temporary directory, skill files, a config file, the fake T3 and the fake forges, then joins 3 tickets and 3 workers through `wf`. Even a 1-byte input pays for that setup, which caps the target near 56 exec/s under ASan in memory. Without ASan the target runs 2.8 times as fast in memory and 2.3 times as fast on the file store.

`cov` counts are not comparable across builds, because ASan's instrumentation adds edges of its own. Within a build, both stores reached about the same coverage in 60 seconds, 29,906 and 29,868 edges under ASan.

**Soak.** A 10-minute ASan run on the in-memory store, from the same 417-input corpus with `-seed=7` and nothing else running, executed 9,398 inputs, 15 exec/s on average. The rate falls as the corpus moves toward longer programs: 36 exec/s at 512 executions, 25 at 2,048, 16 at 8,192, while the mean corpus input grew from 40 bytes to 119. RSS rose from 673 MB to 897 MB and flattened, from 892 MB at 8,192 executions to 897 MB at the end, under libFuzzer's 2,048 MB default limit. Coverage was still climbing at the end, from 30,424 to 30,996 over the last 7,000 executions. An earlier 10-minute run that overlapped a full `cargo test` build averaged 13 exec/s, with the same shape.

## The nightly job

`.github/workflows/fuzz-nightly.yml`, its own workflow so the nightly toolchain and its cache stay out of `chaos-nightly.yml`:

- `schedule` at 10:53 UTC and `workflow_dispatch` with a `seconds` budget that defaults to 1800 and stops at 3000, `permissions: contents: read`, `ubuntu-latest`, a 75-minute timeout.
- The same pins as `chaos-nightly.yml` and `mutants-diff.yml`: `actions/checkout@v7`, `dtolnay/rust-toolchain@683b588c` with `toolchain: nightly-2026-10-28`, `Swatinem/rust-cache@d1527c8f` with key `fuzz-decode-ops` on the `fuzz` workspace, and `actions/upload-artifact@v7`. `cargo install --locked cargo-fuzz@0.13.2` installs the driver, since `taiki-e/install-action` has no `cargo-fuzz` manifest at the pinned SHA.
- `actions/cache/restore` and `actions/cache/save` at `55cc8345`, v6.1.0, carry `fuzz/corpus/decode_ops` from run to run, under a key per run ID and attempt with a shared restore prefix. `cargo fuzz cmin` minimizes the corpus within a 20-minute step timeout, and the job saves it only when that step succeeds, so the next run's startup replay stays short. A night whose `cmin` times out keeps the previous night's corpus.
- Each run adds 16 random 768-byte inputs from `/dev/urandom` as a second, read-only corpus directory, and libFuzzer copies the ones that add coverage into the saved corpus. The first evidence run started from an empty corpus without them, and libFuzzer's length control let inputs grow to only 11 bytes in 30 minutes. It executed 49,646 inputs at 27 exec/s and reached 29,640 edges, below the 30,996 the seeded local soak reached in 10 minutes.
- libFuzzer runs with `-timeout=120`, so an input that hangs becomes a `timeout-*` crasher. On a crash, the job replays each crasher with `FUZZ_DUMP_OPS=1` under a 300-second `timeout`, which prints the decoded program as an `.ops` file. It then uploads the crashers, their `.ops` files, the replay logs and `fuzz.log` as `fuzz-crashers` for 30 days, before it minimizes the corpus.

The green run at the final workflow, https://github.com/OttoOps/t3-codemode/actions/runs/45672550399, restored the 1,462-input corpus that the first run saved and added the 16 seeds. It executed 19,627 inputs at 10 exec/s with inputs up to 768 bytes, reached 31,492 edges against the first run's 29,640, and peaked at 905 MB RSS. The build took 6 minutes, fuzzing 30, and `cmin` 6, which cut 2,960 files to 2,674. The job took 42 minutes against its 75-minute timeout. Each night's `cmin` replays the whole corpus at about 7 inputs a second, so its time grows with the corpus until the runs stop finding new features. The 20-minute step timeout caps it.

To reproduce a crasher locally and turn it into a corpus file:

```bash
cd fuzz
cargo +nightly fuzz run decode_ops artifacts/decode_ops/crash-<hash>
FUZZ_DUMP_OPS=1 target/x86_64-unknown-linux-gnu/release/decode_ops artifacts/decode_ops/crash-<hash> 2>&1 \
  | grep -E '^(#|\{)' > artifacts/decode_ops/crash-<hash>.ops
cd ..
CHAOS_OPS=fuzz/artifacts/decode_ops/crash-<hash>.ops CHAOS_SHRINK=1 cargo test --locked --test all pr_chaos::seeded_sequential_chaos_holds_all_invariants -- --exact --nocapture
```

The last command shrinks the program with the existing `ddmin` shrinker and writes a minimized `.ops` file under `tests/chaos_corpus/`, the same path a Swarm sweep failure takes.

## Limits

- A change to `pr_chaos.rs` or `tests/common/mod.rs` that breaks the fuzz build shows up in the next nightly run. Pull request CI never builds the fuzz crate, and the stable test keeps only `run_fuzz_bytes` itself compiling.
- `fuzz/Cargo.toml` names `jiff` itself, because `pr_chaos.rs` passes `jiff::Timestamp` values into `t3-codemode`. When the root moves `jiff` to a new major version, the fuzz build fails with a type mismatch until `fuzz/Cargo.toml` follows.
- `fuzz/Cargo.lock` started as a copy of the root lock plus `libfuzzer-sys` and `jobserver`. A root lock bump alone leaves the fuzz build on the older versions until someone refreshes `fuzz/Cargo.lock`. The job does not pass `--locked`, so when the root `Cargo.toml` asks for a version the fuzz lock lacks, Cargo updates the copy in the runner.
- The target covers the sequential executor only. The concurrent executor in `run_concurrent_seed` runs on paused Tokio time with its own interleaving schedule, and this target leaves it to the seeded tests.
- An uninstrumented build, `cargo fuzz run -s none`, runs 2.8 times faster, which suits a local session that hunts oracle failures and can do without the memory checks.
