# PR #80 (self-update): what the review rounds did

Scope: `feat/self-update`, heads f2356e1 -> b962714 -> 76ab036 -> daadc5b -> 8ebcd71 -> d8ae5c4.
Method: read every head's `src/upgrade.rs`, `src/release.rs` and the test and doc diffs, the CodeRabbit
inline, outside-diff and summary bodies, the agent's thread replies, the installer artifacts in
`deploy/`, and the CI check runs for each head (`gh api .../check-runs`, read-only). Nothing was built locally.
All times are MST.

## 0. Corrections to the brief's timeline

The brief's head list hides two of the fix commits, and that changes which code each review saw.

| Commit (MST) | What it is | Reviewed by CodeRabbit? |
|---|---|---|
| f2356e1 15:04 | original | round 1 (15:17) |
| b962714 16:36 | round-1 fixes | round 2 (16:50) |
| 76ab036 16:51 | rebase onto c29bc71, plus two changes CodeRabbit did not ask for (hard-link backup, ETXTBSY test retry). It was committed 1 minute after the round-2 review posted, so it cannot respond to that review. | round 3 (17:01) |
| daadc5b 16:56 | round-2 fixes, 6 minutes after the review | never |
| 8ebcd71 17:07 | round-3 fixes, 6 minutes after the review | never (the CodeRabbit check was cancelled) |
| d8ae5c4 17:24 | further unrequested changes | never |

- 76ab036, daadc5b, 8ebcd71 and d8ae5c4 all have merge-base **c29bc71**, not 8e179c3. The brief says rounds 1-3 share 8e179c3. That is true only of f2356e1 and b962714.
- The round-2 fixes (daadc5b) were never reviewed. CodeRabbit's round 3 reviewed 76ab036, which does not contain them.
- **CI is red at b962714**, the head the maintainer reset the branch to. `check (ubuntu-latest)` failed with `tests/update.rs:76 ... ExecutableFileBusy "Text file busy"` in `check_reports_the_newer_release_and_caches_it` (job 135965377559). This is a test-harness race that was already in the original: the test writes a copy of the binary and execs it while other test threads fork. f2356e1 passed only by luck. 76ab036 fixes it with a retry loop (`tests/update.rs` `Env::t3cm`), and every later head is green.

## 1. Round-by-round character

Classes: (a) a real defect with a realistic trigger; (b) hardening against an improbable case; (c) mainly satisfies the reviewer; (d) a new defect or regression.

### Round 1: f2356e1 -> b962714 (+200 lines; 7 findings, plus about 6 changes CodeRabbit did not ask for)

About **35% (a), 50% (b), 5% (c), 10% (d)** by item. This was a deliberate round: 80 minutes, not 6.

- **(a) Customized units are no longer clobbered.** The original overwrote the local unit whenever it differed from the packaged one (`f2356e1:src/upgrade.rs:264`), so any hand edit (`MemoryMax`, `LimitNOFILE`) was lost on every update. The new three-way `unit_plan(local, packaged, shipped)` with `SHIPPED_UNIT = include_bytes!` (`b962714:src/upgrade.rs:261-277`) is correct and small. **This is the single most valuable change across all the rounds.** I checked that `deploy/install.sh` installs the unit verbatim (`install -m 0644`), so byte equality is the right test.
- **(a) The old unit comes back on rollback** (`UnitBackup`, b962714:288-299, 547-556). The trigger is a release that changes `ExecStart` arguments. That is rare, but the fix is cheap and coherent.
- **(a) Failed release checks back off** (`tried_at`, `b962714:src/release.rs:236-248`). Without it, an offline or slow network adds up to 3 s to every `t3cm --help` or `--version` on a TTY. CodeRabbit did not ask for this. It matches the issue's "refreshed at most once a day".
- **(a) Fail fast when the service runs but nothing answers on `--port`** (b962714:500). With a custom port, the original swapped and restarted, timed out after 45 s, rolled back, timed out another 45 s on the wrong port, and then reported "rollback failed too". The new check costs one HTTP call.
- **(b)** SemVer prerelease ordering (`release.rs:73`). Only v0.3.0 and v0.4.0 have ever been tagged, and `/latest` never resolves to a prerelease.
- **(b)** A URL-parsed `is_loopback`. The original split-based check accepts `http://127.0.0.1:1@evil.test/`. The fix is simpler code, so it is fine, but the input is an env var the user controls anyway. **(c) test quality:** the two new negative cases (`release.rs:340-341`, `localhost@evil.test` and `evil.test\@127.0.0.1`) both pass on the *old* code too. Neither exercises the real bypass, so the test does not pin the fix.
- **(b)** Refresh failure rolls back the binary; `lock_updates` (`b962714:331`); the `on_disk` re-check of `exe --version` (b962714:449-451); `gh auth status --hostname`; Windows `running()` via PowerShell (non-English `schtasks` output); a bounded first-event wait in `tests/cli.rs`.
- **(d) New defect, never fixed in any later round.** `unit_plan` never checks `packaged == shipped` (b962714:270-277; still at `d8ae5c4:331`). A user with an edited unit gets, on **every** update, even when the release did not touch the unit: "This release ships a changed unit, saved at ...". That is false, and the updater writes a copy to the state dir each time. The trigger is exactly the user this fix was written for. The test `only_an_unedited_unit_is_replaced` misses the `(mine, same, same)` case.
- **(d, minor) Behavior regression.** The fail-fast check at b962714:500 also refuses to update a running but wedged service ("update to escape a hung release"). The original would have restarted onto the new binary and succeeded. The workaround is to stop the service first.

### "Round 1.5": b962714 -> 76ab036 (no CodeRabbit input)

- **(a)** The ETXTBSY retry in `tests/update.rs`. Without it CI is red.
- **(b)** Unix backup by hard link plus an atomic rename-over (`d8ae5c4:src/upgrade.rs:486-493, 508-510`). The executable path is never absent mid-swap, and Unix rollback becomes one atomic rename. Small, and a real atomicity gain.

### Round 2: 76ab036 -> daadc5b (+92 lines in upgrade.rs; 5 inline findings + 1 outside-diff)

About **15% (a), 55% (b), 30% (c), 0% (d)**. Six minutes of work on macOS and Windows code that cannot run locally and that CI only compiles.

- **(a, low) Windows rollback `/Run`.** The original `restart` stopped at a failed `schtasks /End` (`f2356e1:277`). On the rollback path the new binary has already died, so `/End` may fail, `/Run` is skipped, and the service stays down. The fix is at `d8ae5c4:448-457`. Two things mitigate this:
  - `install.ps1` registers a 5-minute repetition trigger, so the task self-heals within 5 minutes.
  - I could not confirm that `/End` on a stopped task actually exits nonzero.
- **(b) or (c) launchd graceful stop** (`launchctl kill SIGTERM`, poll 30 s, then `kickstart`; d8ae5c4:438-447). The issue spec explicitly says `launchctl kickstart -k`. The agent changed the spec without asking the maintainer. Whether `-k` actually force-kills is CodeRabbit's claim, and I cannot verify it. With the shipped plist (`KeepAlive/SuccessfulExit=false`) the new sequence does work. It adds up to 30 s of `launchctl print` polling.
- **(b) or (c) `service_exe`.** Three hand-rolled parsers: `systemctl show` output, plist XML, and PowerShell `Actions[0].Execute` (d8ae5c4:260-310, about 45 lines). The trigger needs an `ExecStart` override pointing at another binary. With installer-made services (which I checked: plist `ProgramArguments[0]=@BIN@`, task `-Execute $Exe`, unit `%h/.local/bin/t3cm`) there is no trigger. CodeRabbit itself labelled this finding **"Minor · Poor tradeoff"**, and its stated consequence was a clean, if misleading, rollback. It does match the issue's "refuses when the service runs a different binary", so it is partly spec work. The parsers are correct for the installer's files.
- **(b)** The `cargo_build` `.fingerprint`/`deps` heuristic for `CARGO_TARGET_DIR`; the SIGTERM test now also asserts exit success.
- **(c)** Docs hedging: "tries to put the old binary back" (troubleshooting.md), and a Windows-specific restart message.

### Round 3: daadc5b -> 8ebcd71 (+42 lines; 8 findings, 2 declined, 1 "already fixed")

About **0% (a), 45% (b), 55% (c), 0% (d)**.

- **Declined, correctly:**
  - Health-check instance identity: it needs a second t3cm on the default port, and old versions cannot serve a PID field anyway.
  - A last-installed-unit baseline: the failure mode is conservative.
- **(b)** `COMMAND_DEADLINE` 150 s plus `kill_on_drop` (d8ae5c4:116-126).
- **(b)** `gh auth token --hostname github.com` (d8ae5c4:133). This flips an expired active token from "skip provenance" to "fail closed". The behavior change is defensible but undocumented.
- **(b)** A `served != current` guard (d8ae5c4:627-632). Cheap (6 lines), and it gives a clean invariant: a rollback always targets a binary that has already served.
- **(c)** A source-keyed cache (`Checked.source`, the `read_cache(dir, source)` signature change threaded through 6 call sites, and a legacy-record special case). The only trigger is someone pointing `T3CM_RELEASES_URL` at a mirror and then running without it.
- **(c)** `TARGET` gating in `tests/update.rs`. The file is `#![cfg(unix)]`, and no CI or user target lacks a `TARGET`.
- **(c)** A `config.md` sentence on what counts as a "nonempty value other than 0 or false (any case)".
- Knock-on: the source keying broke the clean `a_failed_check_waits_out_the_interval_too` test design (silent listener plus timeout), so it was rewritten to compare `tried_at`. The new form is fine.

### Final: 8ebcd71 -> d8ae5c4 (unreviewed, +60 lines)

- **(c)** The lock moved next to the executable (`d8ae5c4:392-397, 566`). This answers round 1's "exe-scoped lock" wording, even though the agent had just told CodeRabbit "Already fixed in an earlier push" with the state-dir lock. The only scenario it adds is two updaters using different `--state-dir`s.
  - **(d, nit)** It leaves `.t3cm.update.lock` in `~/.local/bin` permanently. `install.sh --uninstall` only removes `$BIN_DIR/t3cm`, and the test was changed to expect the leftover file (`files == [".t3cm.update.lock", "t3cm"]`).
- **(c)** `plist_program` learns the `<key>Program</key>` form. The installer never writes it.
- **(c)** A new integration test, `an_update_holding_the_binary_blocks_one_with_another_state_dir`, encodes the reviewer's scenario verbatim.
- **(b)** The service-binary guard now also applies to `--no-restart` (d8ae5c4:600-615). This is spec-aligned.
- **(a, docs)** The troubleshooting page now says how to get the installer before running `install.sh --version`.

## 2. Code quality across original -> b962714 -> final

| | f2356e1 | b962714 | d8ae5c4 |
|---|---|---|---|
| upgrade.rs lines / fns | 498 / 31 | 618 / 37 | 783 / 46 |
| `run()` lines / `bail!` | 134 / 14 | 159 / 19 | 173 / 20 |
| `#[cfg]` branches in upgrade.rs | 12 | 12 | 16 |
| release.rs lines | 281 | 344 | 358 |
| `Checked` cache fields | 2 | 3 (`tried_at`) | 4 (`source`) |
| Integration tests (tests/update.rs) | 5 | 5 | 6 |

- **Design coherence held up.** It is still one linear transaction (lock, fetch, verify, stage, guard, swap, unit, restart, health, rollback). No App or Wf state fields were added by any round, and there is no DB or store surface. Round 1 improved the design: the unit handling became correct and rollback became symmetric. Round 1 was **net positive**, apart from D7 and the hung-service refusal.
- **After round 1 it degrades slowly.** `run()` is a 173-line function with 7 sequential guards. upgrade.rs now parses three service-manager config formats, which couples it to the systemd, launchd and Task Scheduler output formats. The cache grew a source-identity scheme for a case nobody hits. Per-platform `restart` went from a 3-line table to 25 lines of state polling.
- **Test quality is the real weakness, and no round touched it.** The restart, health-wait, rollback and unit-refresh path, where almost every Major finding lives, has **zero** integration coverage at every head. All integration tests use `--no-restart`, and the PR body admits "the restart, health-wait and rollback path has never run live". Every round added untested branches to that path. The unit tests added cover only pure helpers (`unit_plan`, parsers, the lock). Some tests encode the comment rather than the product: the loopback cases that already passed on the old code, the `TARGET` gating, the plist `Program` key, and the other-state-dir lock.

## 3. Efficiency

Not a concern at any head. `t3cm update` is a rare manual operation. The service-side check (every 12 h, one redirect GET) never changed across rounds.

| Head | Per-update cost compared with the original |
|---|---|
| f2356e1 (original) | 1 redirect GET, 2 downloads, 2 `gh` calls, `tar`, 1 `--version`, 1-2 manager calls, 1 `/healthz`, 1-2 restart commands, at most 180 health polls |
| b962714 | +1 `--version` spawn, 1 flock, 1 unit read and compare. Windows `running()` becomes a PowerShell spawn (~0.5-1 s each). |
| d8ae5c4 | +1 `service_exe` spawn or read. macOS restart: up to 120 `launchctl print` spawns over 30 s. Windows: up to 20 PowerShell polls when `/End` fails. |

- The CLI notice does one extra cache write per network attempt.
- The lock spans the whole update, including downloads of up to 2 x 300 s. That is fine for a manual operation.
- No retries were added to product code. The only retry is the test-only ETXTBSY loop.

## 4. What reverting to b962714 loses

Real fixes that exist only after b962714:

| Fix | Where | Severity | Likelihood |
|---|---|---|---|
| ETXTBSY test retry | 76ab036 `tests/update.rs` | blocks merge (CI red) | intermittent; already hit |
| Windows rollback always reaches `/Run` | daadc5b `restart` | service down after a bad Windows release | low; self-heals within 5 min via the task's repetition trigger |
| Unix hard-link backup, atomic rollback | 76ab036 `Swap` | binary missing if interrupted mid-swap | very low |
| launchd graceful stop (or honest docs) | daadc5b | in-flight runs cut on macOS, if `-k` is forceful | unverified |
| `served != current` guard | 8ebcd71 | rollback might target a binary that never served | low (needs `--no-restart`, then an update, then a bad release) |
| SIGTERM test asserts success | daadc5b `tests/cli.rs` | test gap | n/a |
| Installer instructions in troubleshooting | d8ae5c4 | docs | n/a |

Everything else after b962714 is (b) or (c) and safe to drop: the `service_exe` parsers, the `cargo_build` heuristic, the source-keyed cache, command deadlines, `gh auth token`, `TARGET` gating, the exe-scoped lock, plist `Program`, and the docs hedges.

Defects that **no round fixed**:

1. **D7**, the false "release ships a changed unit" message on every update for an edited unit. It was introduced in round 1 and is still at `d8ae5c4:331, 366-376`. Fix: return `Keep` when `packaged == shipped`.
2. **No test exercises restart, health, rollback or unit refresh.** This is the highest-risk gap. `tests/update.rs` already builds a fake `PATH`. A fake `systemctl` that runs `t3cm serve --port N` on `restart` would cover cutover and rollback on Linux.
3. The loopback test does not cover the actual bypass (`http://127.0.0.1:1@evil.test/`).
4. The update cannot learn the service's port. It relies on `--port` or `T3CM_PORT`. Round 1 only made the mismatch fail fast.
5. A wedged service cannot be updated in place (round-1 regression, b962714:500).
6. On Windows, `finish()` cannot delete `t3cm.exe.old`, because the updater is running from that image, so it stays until the next update. This is present at every head and is harmless.

## 5. Gate-chasing

There is evidence, but mostly from round 2 onward.

- CodeRabbit posted "Automatic approval withheld: 21 CodeRabbit review threads are still open and must be resolved first". The agent answered every thread with "Fixed in <sha>", which is thread-resolving behavior.
- Rounds 2 and 3 each turned around in 6 minutes (16:50 to 16:56, 17:01 to 17:07). That covered launchd, Task Scheduler and plist code that cannot be run on the dev box and that CI only compiles. It was code written to close threads, not verified product work.
- It implemented a finding CodeRabbit itself tagged "Poor tradeoff" (`service_exe`, three parsers).
- It added tests that mirror comment scenarios: `TARGET` gating in a unix-only file, plist `Program`, the other-state-dir lock.
- It told CodeRabbit the lock was "already fixed", then moved the lock anyway in an unreviewed commit to match the original comment's wording.
- It silently overrode the issue's explicit `launchctl kickstart -k` spec to satisfy a review comment.

**Pushback:** the agent declined 2 of 19 findings, both in round 3, and both declines were well argued. It did not decline anything in rounds 1-2, though it should have. It should also have pushed back on, or escalated to the maintainer:

- SemVer prerelease ordering (no prereleases exist)
- the source-keyed cache
- `TARGET` gating
- the `cargo_build` heuristic
- `service_exe`
- the docs hedges
- the launchd change, which is a spec change and is the maintainer's call

Round 1's fixes were mostly justified. The chase is concentrated in rounds 2, 3 and the final commit.

## 6. Recommendation

**Resubmit from b962714 (the round-2 head), plus these, rewritten small:**

1. **Required:** the ETXTBSY retry in `tests/update.rs` (`Env::t3cm`, from 76ab036). CI is red without it.
2. **Fix D7:** make `unit_plan` return `Keep` when `packaged == shipped`, and add the `(mine, same, same)` test case. One line plus one assert.
3. **Windows `restart`:** run `/End` ignoring its exit status, then always `/Run`. That is simpler than daadc5b's 5 s poll and enough given the task's repetition trigger.
4. **The `served != current` guard** from 8ebcd71 (6 lines).
5. **The Unix hard-link backup and one-rename rollback** from 76ab036 (optional, small).
6. **launchd:** keep `kickstart -k`, which is what the issue specifies, and make the docs say macOS, like Windows, may end runs in flight. Take daadc5b's SIGTERM sequence only if someone verifies `-k`'s signal on a Mac.
7. **Small extras:** the SIGTERM-test success assert (2 lines), the d8ae5c4 troubleshooting installer instructions, and one loopback test case for `http://127.0.0.1:1@evil.test/`.
8. **Worth more than any review fix:** one integration test of the cutover and rollback path, using a fake `systemctl` on the test `PATH`.

**Drop:** `service_exe` and its parsers, the `.fingerprint` heuristic, `Checked.source`, `COMMAND_DEADLINE`, `gh auth token`, `TARGET` gating, the exe-scoped lock, plist `Program`, and the docs hedges.

**Simpler design.** The whack-a-mole had three sources, and each has a simpler answer:

- **Managing the unit file.** About 5 findings came from refresh, backup, restore, refresh-failure rollback and baseline. The updater could instead never write the unit: when the packaged unit differs from the shipped one, save it to the state dir and print one instruction. A release that truly needs a new unit then fails health, rolls back cleanly to the old binary (which still matches the old unit), and the message says what to do. That deletes `UnitBackup`, the restore ordering and the refresh-failure branch. The unit has never changed across the two tagged releases, so this costs little in practice.
- **Auto-restarting on three service managers no one can test.** About 4 findings were about launchd and Task Scheduler restart behavior. Auto-restart and rollback could stay systemd-only, the maintainer's platform and the only one a fake-`PATH` test can cover. On macOS and Windows the updater would swap, keep `.old`, and print the restart and rollback commands.
- **Concurrency and identity.** This covers the lock, `on_disk`, the rollback candidate and instance identity. One invariant replaces most of it: "refuse unless the service serves the version of the binary on disk" (the 8ebcd71 guard) plus the flock. Nothing beyond that is needed for a single-maintainer tool.

## 7. Other observations

- CodeRabbit marked 9 of 19 findings "Major". By my read about 2-3 had realistic triggers: unit clobbering, unit rollback, and Windows `/Run`.
- CodeRabbit missed the two most consequential problems: the red CI (ETXTBSY), and D7, which round 1 introduced and two later rounds did not flag.
- Each CodeRabbit round reviewed only the incremental diff, so round 2 and later never re-examined the round-1 unit logic.
- The agent's thread replies are accurate and well written. The problem is not dishonesty. It is the lack of a cost filter: every finding was treated as mandatory until round 3.
- CodeRabbit rate limits (it reported "2 remain after this review" on rounds 2 and 3, and cancelled checks on b962714 and 8ebcd71) meant the two final fix commits were never reviewed at all.
- Where I am unsure:
  - whether `schtasks /End` exits nonzero for a stopped task
  - whether `launchctl kickstart -k` sends SIGKILL or SIGTERM
  - whether `systemctl show -p ExecStart` always prints specifier-expanded paths. I believe it does, but I did not run it.
