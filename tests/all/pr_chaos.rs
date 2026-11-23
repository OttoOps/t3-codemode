use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::fake::{check_run, github_pr, project, pull_request, thread};
use t3_codemode::store::{MergeWait, WaitStatus};

fn same_head(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

const REPO: &str = "acme/widgets";
const HEADS: &[&str] = &[
    "1111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333",
    "4444444444444444444444444444444444444444",
];
const GOOD_BODY: &str = "## Summary\n\n```text\nwf.pr\n  chaos\n```\n\n## Evidence\n\n- **Before:** old\n  **After:** new\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\nRefs #41\n";
const PUBLISHING: &str = "Publishing: pr skill; passes: none configured";
const TICKETS: &[&str] = &["mcp:t1", "mcp:t2", "mcp:t3"];
const WORKERS: &[&str] = &["mcp:w1", "mcp:w2", "mcp:w3"];
const PRS: &[u64] = &[7, 8, 9];

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_add(0x9e3779b97f4a7c15))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }

    fn coin(&mut self) -> bool {
        (self.next() & 1) == 0
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum GhChecks {
    Pending,
    Passing,
    Failing,
    Cancelled,
    Unread,
}

#[derive(Clone, Debug)]
struct GhPrModel {
    head_idx: usize,
    state: &'static str,
    draft: bool,
    merge_state: &'static str,
    checks: GhChecks,
    exists: bool,
    read_err: bool,
    queue_enabled: bool,
    queued: bool,
    removed_reason: Option<&'static str>,
}

impl Default for GhPrModel {
    fn default() -> Self {
        Self {
            head_idx: 0,
            state: "OPEN",
            draft: false,
            merge_state: "CLEAN",
            checks: GhChecks::Passing,
            exists: true,
            read_err: false,
            queue_enabled: false,
            queued: false,
            removed_reason: None,
        }
    }
}

fn pending_check(name: &str) -> Value {
    json!({"__typename": "CheckRun", "name": name, "status": "IN_PROGRESS", "conclusion": null})
}

fn setup_chaos() -> (H, BTreeMap<u64, GhPrModel>) {
    let h = H::new();
    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for skill in ["deploy", "verify", "publish"] {
        let d = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {skill}\ndescription: test\n---\n\nBody.\n")).unwrap();
    }
    let cfg = json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"}
    ]}});
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, cfg.to_string()).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);

    for (i, &t) in TICKETS.iter().enumerate() {
        let mut th = thread(t, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/ticket-{i}"));
        h.fake.put(th);
    }
    for (i, &w) in WORKERS.iter().enumerate() {
        let mut th = thread(w, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/worker-{i}"));
        h.fake.put(th);
    }
    let mut prs = BTreeMap::new();
    for &n in PRS {
        prs.insert(n, GhPrModel::default());
    }
    sync_gh(&h, &prs);
    (h, prs)
}

fn sync_gh(h: &H, prs: &BTreeMap<u64, GhPrModel>) {
    h.github.clear_read_errors();
    for (&number, m) in prs {
        if !m.exists {
            h.github.remove_pr(REPO, number);
            continue;
        }
        let mut pr = github_pr(HEADS[m.head_idx], m.merge_state, m.queue_enabled);
        pr["state"] = json!(m.state);
        pr["isDraft"] = json!(m.draft);
        pr["isInMergeQueue"] = json!(m.queued);
        pr["body"] = json!(GOOD_BODY);
        pr["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
        if let Some(reason) = m.removed_reason {
            pr["timelineItems"] = json!({"nodes": [{"reason": reason, "createdAt": "2026-11-23T12:04:00Z"}]});
        }
        if m.state == "MERGED" {
            pr["mergeCommit"] = json!({"oid": HEADS[(m.head_idx + 1) % HEADS.len()]});
            pr["mergedAt"] = json!("2026-11-23T12:05:00Z");
        } else if m.state == "CLOSED" {
            pr["closedAt"] = json!("2026-11-23T12:05:00Z");
        }
        h.github.set_pr(REPO, number, pr);
        let checks = match m.checks {
            GhChecks::Pending => json!([pending_check("ci")]),
            GhChecks::Passing => json!([check_run("ci", "SUCCESS")]),
            GhChecks::Failing => json!([check_run("ci", "FAILURE")]),
            GhChecks::Cancelled => json!([check_run("ci", "SUCCESS"), check_run("opt", "CANCELLED")]),
            GhChecks::Unread => json!([]),
        };
        h.github.set_checks(REPO, number, checks);
        if m.read_err {
            h.github.fail_read(REPO, number, "transient chaos read error");
        }
    }
}

fn sent_counts(h: &H) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for s in h.sent() {
        let to = s["threadId"].as_str().unwrap_or_default().to_string();
        *m.entry(to).or_insert(0) += 1;
    }
    m
}

fn assert_invariants(
    h: &H,
    seed: u64,
    trace: &[String],
    closed_workers: &BTreeMap<String, usize>,
    after_healthy_reconcile: bool,
    gh: &BTreeMap<u64, GhPrModel>,
) {
    let ctx = || format!("seed={seed}\ntrace:\n{}", trace.join("\n"));
    let cards = h.app.wf.cards().unwrap();

    // 1. Card metadata, uniqueness, and worker ownership invariants
    let mut ready_seen: BTreeSet<(String, String, u64)> = BTreeSet::new();
    for c in &cards {
        if let Some(ticket) = c.ticket.as_deref() {
            let tlink = h.app.store.link_get(ticket).unwrap();
            if c.worker.is_none() {
                if let Some(tl) = &tlink {
                    assert_eq!(
                        c.label.as_deref(),
                        Some(tl.label.as_str()),
                        "ticket-owned card label drifted from owner: card={c:?} link={tl:?}\n{}",
                        ctx()
                    );
                    assert_eq!(
                        c.link.as_deref(),
                        tl.link.as_deref(),
                        "ticket-owned card link drifted from owner: card={c:?} link={tl:?}\n{}",
                        ctx()
                    );
                }
                if c.kind == "gate"
                    && let Some(p) = c.pr.as_ref().filter(|p| p.ready)
                {
                    let key = (ticket.to_string(), p.repo.clone(), p.number);
                    assert!(
                        ready_seen.insert(key.clone()),
                        "duplicate open ready merge card for {key:?}: {cards:?}\n{}",
                        ctx()
                    );
                }
            } else if let Some(w) = &c.worker {
                let wl = h.app.store.link_get(&w.thread).unwrap().expect("relayed worker link");
                assert!(!wl.closed, "open relayed card points to closed worker: card={c:?} wl={wl:?}\n{}", ctx());
                assert_eq!(
                    wl.parent,
                    ticket,
                    "relayed card ticket does not match worker parent: card={c:?} wl={wl:?}\n{}",
                    ctx()
                );
            }
        }
    }

    // 2. Ticket gate invariants
    for &t in TICKETS {
        let Some(tl) = h.app.store.link_get(t).unwrap() else { continue };
        if tl.closed {
            continue;
        }
        let has_ready = cards.iter().any(|c| {
            c.kind == "gate"
                && c.worker.is_none()
                && c.ticket.as_deref() == Some(t)
                && c.pr.as_ref().is_some_and(|p| p.ready)
        });
        let has_relayed = cards.iter().any(|c| c.worker.is_some() && c.ticket.as_deref() == Some(t));
        if has_ready {
            assert!(
                tl.gate.is_some(),
                "ticket {t} has open ready merge card but gate is None: cards={cards:?}\n{}",
                ctx()
            );
        }
        if has_relayed {
            assert!(tl.gate.is_some(), "ticket {t} has open relayed card but gate is None: cards={cards:?}\n{}", ctx());
        }
        if tl.gate_pr.is_some() {
            assert_eq!(
                tl.gate.as_deref(),
                Some("merge"),
                "ticket {t} has gate_pr={:?} while gate={:?}\n{}",
                tl.gate_pr,
                tl.gate,
                ctx()
            );
        }
    }

    // 3. Worker link invariants (no closed-worker wakes, no stuck waits after healthy reconcile)
    let counts = sent_counts(h);
    for (w, &count_at_close) in closed_workers {
        let wl = h.app.store.link_get(w).unwrap().unwrap();
        assert!(wl.closed, "worker {w} expected closed\n{}", ctx());
        assert!(wl.gate.is_none(), "closed worker {w} kept gate {:?}\n{}", wl.gate, ctx());
        assert!(wl.gate_pr.is_none(), "closed worker {w} kept gate_pr {:?}\n{}", wl.gate_pr, ctx());
        let now_count = counts.get(w).copied().unwrap_or(0);
        assert_eq!(
            now_count,
            count_at_close,
            "closed worker {w} received a message after closing! messages={:?}\n{}",
            h.sent().into_iter().filter(|s| s["threadId"] == w.as_str()).collect::<Vec<_>>(),
            ctx()
        );
    }

    if after_healthy_reconcile {
        for &w in WORKERS {
            let Some(wl) = h.app.store.link_get(w).unwrap().filter(|l| !l.closed) else { continue };
            if let Some(p) = wl.pr.as_ref().filter(|p| p.waiting)
                && let Some(m) = gh.get(&p.number)
                && !m.read_err
            {
                let should_keep_waiting = m.exists
                    && m.state == "OPEN"
                    && !m.draft
                    && same_head(HEADS[m.head_idx], &p.head)
                    && (m.checks == GhChecks::Pending
                        || (matches!(m.checks, GhChecks::Passing | GhChecks::Cancelled) && m.merge_state == "UNKNOWN"));
                assert!(
                    should_keep_waiting,
                    "worker {w} stayed stuck in pr.waiting=true after healthy reconcile: pr={p:?} gh={m:?}\n{}",
                    ctx()
                );
            }
        }
    }
}

fn chaos_seeds(default: usize) -> usize {
    std::env::var("CHAOS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::test]
async fn seeded_sequential_chaos_holds_all_invariants() {
    let seeds = chaos_seeds(60);
    for seed in 0..seeds as u64 {
        let mut rng = Rng::new(seed);
        let (h, mut gh) = setup_chaos();
        let mut trace: Vec<String> = Vec::new();
        let mut closed_workers: BTreeMap<String, usize> = BTreeMap::new();

        for (i, &t) in TICKETS.iter().enumerate() {
            h.wf(t, "join", json!({"parent": BOARD, "label": format!("widgets#{}", 41 + i), "role": "ticket"})).await;
        }
        let mut active_workers: Vec<usize> = vec![0, 1, 2];
        for (i, &w) in WORKERS.iter().enumerate() {
            let owner = TICKETS[rng.pick(2)];
            let part = format!("{}/3", i + 1);
            let label = format!("#41 PR {part}");
            h.wf(
                owner,
                "openChild",
                json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part,
                       "pr": {"format": "pr"}}),
            )
            .await;
            h.wf(w, "join", json!({"parent": owner, "label": label, "role": "worker"})).await;
            trace.push(format!("init {w} under {owner} -> PR #{}", PRS[i]));
        }

        for step in 0..45 {
            let op = rng.pick(20);
            let mut healthy_reconcile = false;
            let res = tokio::time::timeout(Duration::from_secs(5), async {
                match op {
                    0 => {
                        if active_workers.is_empty() {
                            return;
                        }
                        let wi = active_workers[rng.pick(active_workers.len())];
                        let w = WORKERS[wi];
                        let pr = PRS[wi];
                        let head = HEADS[gh[&pr].head_idx];
                        let sha = if rng.coin() { &head[..7] } else { head };
                        let checks = if rng.pick(8) == 0 { "none" } else { "pending" };
                        let r = h
                            .wf(
                                w,
                                "pr",
                                json!({"event": "opened", "repo": REPO, "pr": pr, "head": sha,
                                       "publishing": PUBLISHING, "checks": checks}),
                            )
                            .await;
                        trace.push(format!("#{step} {w} pr opened #{pr}@{sha} checks={checks} -> {}", r["status"]));
                    }
                    1 => {
                        if active_workers.is_empty() {
                            return;
                        }
                        let wi = active_workers[rng.pick(active_workers.len())];
                        let w = WORKERS[wi];
                        let pr = PRS[wi];
                        let head = HEADS[gh[&pr].head_idx];
                        let sha = if rng.coin() { &head[..7] } else { head };
                        let r = h.wf(w, "pr", json!({"event": "updated", "repo": REPO, "pr": pr, "head": sha})).await;
                        trace.push(format!("#{step} {w} pr updated #{pr}@{sha} -> {}", r["status"]));
                    }
                    2 | 3 => {
                        if active_workers.is_empty() {
                            return;
                        }
                        let wi = active_workers[rng.pick(active_workers.len())];
                        let w = WORKERS[wi];
                        let pr = PRS[wi];
                        let head = HEADS[gh[&pr].head_idx];
                        let sha = if rng.coin() { &head[..7] } else { head };
                        let r = h.wf(w, "pr", json!({"event": "ready", "repo": REPO, "pr": pr, "head": sha})).await;
                        trace.push(format!("#{step} {w} pr ready #{pr}@{sha} -> {}", r["status"]));
                    }
                    4 => {
                        let pr = PRS[rng.pick(PRS.len())];
                        let changed = {
                            let m = gh.get_mut(&pr).unwrap();
                            if m.exists && m.state == "OPEN" {
                                m.head_idx = (m.head_idx + 1) % HEADS.len();
                                trace.push(format!("#{step} gh push #{pr} -> head {}", &HEADS[m.head_idx][..7]));
                                true
                            } else {
                                false
                            }
                        };
                        if changed {
                            sync_gh(&h, &gh);
                        }
                    }
                    5 => {
                        let pr = PRS[rng.pick(PRS.len())];
                        let changed = {
                            let m = gh.get_mut(&pr).unwrap();
                            if m.exists && m.state == "OPEN" {
                                m.checks = match rng.pick(5) {
                                    0 => GhChecks::Pending,
                                    1 => GhChecks::Passing,
                                    2 => GhChecks::Failing,
                                    3 => GhChecks::Cancelled,
                                    _ => GhChecks::Unread,
                                };
                                m.merge_state = match rng.pick(5) {
                                    0 => "CLEAN",
                                    1 => "UNKNOWN",
                                    2 => "BLOCKED",
                                    3 => "BEHIND",
                                    _ => "DIRTY",
                                };
                                if rng.pick(4) == 0 {
                                    m.draft = !m.draft;
                                }
                                trace.push(format!(
                                    "#{step} gh status #{pr} checks={:?} merge={} draft={}",
                                    m.checks, m.merge_state, m.draft
                                ));
                                true
                            } else {
                                false
                            }
                        };
                        if changed {
                            sync_gh(&h, &gh);
                        }
                    }
                    6 => {
                        let pr = PRS[rng.pick(PRS.len())];
                        let changed = {
                            let m = gh.get_mut(&pr).unwrap();
                            if m.exists && m.state == "OPEN" {
                                match rng.pick(4) {
                                    0 => m.state = "MERGED",
                                    1 => m.state = "CLOSED",
                                    2 => m.read_err = !m.read_err,
                                    _ => m.exists = false,
                                }
                                trace.push(format!(
                                    "#{step} gh lifecycle #{pr} state={} exists={} err={}",
                                    m.state, m.exists, m.read_err
                                ));
                                true
                            } else {
                                false
                            }
                        };
                        if changed {
                            sync_gh(&h, &gh);
                        }
                    }
                    7 | 8 => {
                        let had_err = gh.values().any(|m| m.read_err);
                        let rec = h.app.wf.reconcile().await;
                        trace.push(format!("#{step} reconcile -> ok={}", rec.is_ok()));
                        if !had_err && rec.is_ok() {
                            healthy_reconcile = true;
                            let before_msgs = h.sent().len();
                            let before_cards: Vec<String> =
                                h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                            let rec2 = h.app.wf.reconcile().await;
                            assert!(
                                rec2.is_ok(),
                                "second reconcile failed: {rec2:?}\nseed={seed}\n{}",
                                trace.join("\n")
                            );
                            let after_cards: Vec<String> =
                                h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                            assert_eq!(
                                h.sent().len(),
                                before_msgs,
                                "reconcile was not idempotent (sent extra messages)\nseed={seed}\n{}",
                                trace.join("\n")
                            );
                            assert_eq!(
                                after_cards,
                                before_cards,
                                "reconcile was not idempotent (changed cards)\nseed={seed}\n{}",
                                trace.join("\n")
                            );
                        }
                    }
                    9 => {
                        let from = TICKETS[rng.pick(TICKETS.len())];
                        let mut to = TICKETS[rng.pick(TICKETS.len())];
                        if to == from {
                            to = TICKETS[(TICKETS.iter().position(|x| *x == from).unwrap() + 1) % TICKETS.len()];
                        }
                        let r = h.wf(to, "adopt", json!({"from": from})).await;
                        trace.push(format!("#{step} {to} adopt from {from} -> {r}"));
                        assert!(
                            h.app.store.children(from, true).unwrap().is_empty(),
                            "adopt left open children on {from}\nseed={seed}\n{}",
                            trace.join("\n")
                        );
                    }
                    10 => {
                        let t = TICKETS[rng.pick(TICKETS.len())];
                        if rng.coin() {
                            let _ = h.wf(t, "gate", json!({"clear": true})).await;
                            trace.push(format!("#{step} {t} gate clear"));
                        } else {
                            let r = h
                                .wf(
                                    t,
                                    "send",
                                    json!({"kind": "gate", "gate": "delivery:verify",
                                           "body": "**ACTION NEEDED:** verify: reply \"ok\"", "options": ["ok"]}),
                                )
                                .await;
                            trace.push(format!("#{step} {t} open delivery:verify -> {}", r["card"]));
                        }
                    }
                    11 => {
                        let open = h.app.wf.cards().unwrap();
                        if !open.is_empty() {
                            let c = &open[rng.pick(open.len())];
                            if rng.coin() {
                                let ans = c.options.first().map(String::as_str).unwrap_or("ok");
                                let _ = h.app.wf.answer(&c.id, ans).await;
                                trace.push(format!("#{step} answer {} with {ans:?}", c.id));
                            } else {
                                let _ = h.app.wf.dismiss(&c.id);
                                trace.push(format!("#{step} dismiss {}", c.id));
                            }
                        }
                    }
                    12 => {
                        if !active_workers.is_empty() {
                            let wi = active_workers[rng.pick(active_workers.len())];
                            let w = WORKERS[wi];
                            let r = h
                                .wf(
                                    w,
                                    "send",
                                    json!({"kind": "gate", "gate": "review",
                                           "body": "**ACTION NEEDED:** decide review: reply \"fix\" or \"ship\"",
                                           "options": ["fix", "ship"]}),
                                )
                                .await;
                            trace.push(format!("#{step} {w} relayed review gate -> {}", r["card"]));
                        }
                    }
                    13 => {
                        if active_workers.len() > 1 {
                            let pos = rng.pick(active_workers.len());
                            let wi = active_workers.remove(pos);
                            let w = WORKERS[wi];
                            let parent = h.app.store.link_get(w).unwrap().unwrap().parent;
                            h.wf(&parent, "close", json!({"child": w})).await;
                            let count = sent_counts(&h).get(w).copied().unwrap_or(0);
                            closed_workers.insert(w.to_string(), count);
                            trace.push(format!("#{step} {parent} close {w} (sent_at_close={count})"));
                        }
                    }
                    14 => {
                        let waiting_before: BTreeMap<String, usize> = active_workers
                            .iter()
                            .filter_map(|&wi| {
                                let w = WORKERS[wi];
                                let wl = h.app.store.link_get(w).unwrap()?;
                                wl.pr.as_ref().filter(|p| p.waiting).map(|_| {
                                    let nudges = h
                                        .sent()
                                        .into_iter()
                                        .filter(|s| {
                                            s["threadId"] == w
                                                && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
                                        })
                                        .count();
                                    (w.to_string(), nudges)
                                })
                            })
                            .collect();
                        h.tick(200).await;
                        trace.push(format!("#{step} tick(200)"));
                        for (w, before) in waiting_before {
                            let after = h
                                .sent()
                                .into_iter()
                                .filter(|s| {
                                    s["threadId"] == w.as_str()
                                        && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
                                })
                                .count();
                            assert_eq!(
                                after,
                                before,
                                "waiting worker {w} was nudged during tick(200)\nseed={seed}\n{}",
                                trace.join("\n")
                            );
                        }
                    }
                    15 => {
                        let mut cleared = false;
                        for (&n, m) in gh.iter_mut() {
                            if m.read_err {
                                m.read_err = false;
                                cleared = true;
                                trace.push(format!("#{step} clear read_err #{n}"));
                            }
                        }
                        if cleared {
                            sync_gh(&h, &gh);
                        }
                    }
                    16 => {
                        // Exercise MergeWait and merge queue state transitions
                        let pr = PRS[rng.pick(PRS.len())];
                        let ticket = TICKETS[rng.pick(TICKETS.len())];
                        let m = gh.get_mut(&pr).unwrap();
                        if m.exists && m.state == "OPEN" {
                            match rng.pick(4) {
                                0 => {
                                    let head = HEADS[m.head_idx].to_string();
                                    let w = MergeWait {
                                        repo: REPO.into(),
                                        pr,
                                        head: head.clone(),
                                        pull_request_id: "PR_id".into(),
                                        method: None,
                                        thread: Some(ticket.into()),
                                        label: Some(format!("widgets#{pr}")),
                                        merge_state: m.merge_state.into(),
                                        at: "2026-11-23T12:01:00Z".into(),
                                        status: if m.queued { WaitStatus::Sent } else { WaitStatus::Waiting },
                                        approval: format!("appr-{seed}-{step}"),
                                        failures: 0,
                                        last_error: None,
                                    };
                                    h.app.store.merge_wait_put(&w).unwrap();
                                    trace.push(format!("#{step} merge_wait_put #{pr}@{head} for {ticket}"));
                                }
                                1 => {
                                    m.queue_enabled = !m.queue_enabled;
                                    m.queued = m.queue_enabled && rng.coin();
                                    m.removed_reason = None;
                                    trace.push(format!(
                                        "#{step} gh queue #{pr} enabled={} queued={}",
                                        m.queue_enabled, m.queued
                                    ));
                                    sync_gh(&h, &gh);
                                }
                                2 => {
                                    m.queued = false;
                                    m.removed_reason = Some(if rng.coin() { "failed_checks" } else { "merge_conflict" });
                                    trace.push(format!("#{step} gh queue drop #{pr} reason={:?}", m.removed_reason));
                                    sync_gh(&h, &gh);
                                }
                                _ => {
                                    // Attach PR directly to a worker's live_of to populate pr_seen independently
                                    if !active_workers.is_empty() {
                                        let wi = active_workers[rng.pick(active_workers.len())];
                                        let w = WORKERS[wi];
                                        h.fake.set_prs(w, vec![pull_request(REPO, pr, "open")]);
                                        trace.push(format!("#{step} fake.set_prs({w}, #{pr})"));
                                    }
                                }
                            }
                        }
                    }
                    17 => {
                        let t = TICKETS[rng.pick(TICKETS.len())];
                        let _ = h.wf(t, "gate", json!({"open": "merge"})).await;
                        trace.push(format!("#{step} {t} manual gate open merge"));
                    }
                    18 => {
                        if active_workers.len() > 1 {
                            let pos = rng.pick(active_workers.len());
                            let wi = active_workers.remove(pos);
                            let w = WORKERS[wi];
                            h.wf(w, "send", json!({"kind": "handoff", "body": "chaos handoff"})).await;
                            let count = sent_counts(&h).get(w).copied().unwrap_or(0);
                            closed_workers.insert(w.to_string(), count);
                            trace.push(format!("#{step} {w} handoff (sent_at_close={count})"));
                        }
                    }
                    _ => {
                        // Answer a ready merge card and immediately clear the ticket's gate;
                        // every other unanswered ready merge card on that ticket must survive!
                        let ready_cards: Vec<_> = h
                            .app
                            .wf
                            .cards()
                            .unwrap()
                            .into_iter()
                            .filter(|c| {
                                c.kind == "gate"
                                    && c.worker.is_none()
                                    && c.ticket.is_some()
                                    && c.pr.as_ref().is_some_and(|p| p.ready)
                            })
                            .collect();
                        if !ready_cards.is_empty() {
                            let chosen = &ready_cards[rng.pick(ready_cards.len())];
                            let ticket = chosen.ticket.clone().unwrap();
                            let others: Vec<String> = ready_cards
                                .iter()
                                .filter(|c| c.ticket.as_deref() == Some(ticket.as_str()) && c.id != chosen.id)
                                .map(|c| c.id.clone())
                                .collect();
                            let opt = chosen.options.first().map(String::as_str).unwrap_or("merge");
                            let _ = h.app.wf.answer(&chosen.id, opt).await;
                            let _ = h.wf(&ticket, "gate", json!({"clear": true})).await;
                            trace.push(format!(
                                "#{step} answer ready {} ({opt}) + {ticket} gate clear (others={others:?})",
                                chosen.id
                            ));
                            let after_ids: BTreeSet<String> =
                                h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                            for other in &others {
                                assert!(
                                    after_ids.contains(other),
                                    "answering {} and clearing gate wiped unanswered ready card {other}!\nseed={seed}\n{}",
                                    chosen.id,
                                    trace.join("\n")
                                );
                            }
                        }
                    }
                }
            })
            .await;
            assert!(res.is_ok(), "step timed out (possible deadlock) at seed={seed}\n{}", trace.join("\n"));
            assert_invariants(&h, seed, &trace, &closed_workers, healthy_reconcile, &gh);
        }
    }
}

#[tokio::test]
async fn concurrent_chaos_races_never_deadlock_or_violate_invariants() {
    let seeds = chaos_seeds(30);
    for seed in 0..seeds as u64 {
        let mut rng = Rng::new(seed ^ 0x55aa_1234_9876_fedc);
        let (h, mut gh) = setup_chaos();
        let mut trace = Vec::new();

        for (i, &t) in TICKETS.iter().enumerate() {
            h.wf(t, "join", json!({"parent": BOARD, "label": format!("widgets#{}", 41 + i), "role": "ticket"})).await;
        }
        for (i, &w) in WORKERS.iter().enumerate() {
            let owner = TICKETS[0];
            let part = format!("{}/3", i + 1);
            let label = format!("#41 PR {part}");
            h.wf(
                owner,
                "openChild",
                json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part,
                       "pr": {"format": "pr"}}),
            )
            .await;
            h.wf(w, "join", json!({"parent": owner, "label": label, "role": "worker"})).await;
        }

        h.github.set_delay(Duration::from_millis(15));
        for round in 0..6 {
            let race = rng.pick(5);
            let res = tokio::time::timeout(Duration::from_secs(5), async {
                match race {
                    0 => {
                        // Two workers call ready concurrently while ticket adopts/clears gate
                        let r1 = h.wf(
                            WORKERS[0],
                            "pr",
                            json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[gh[&7].head_idx]}),
                        );
                        let r2 = h.wf(
                            WORKERS[1],
                            "pr",
                            json!({"event": "ready", "repo": REPO, "pr": 8, "head": HEADS[gh[&8].head_idx]}),
                        );
                        let clr = async {
                            tokio::time::sleep(Duration::from_millis(8)).await;
                            h.wf(TICKETS[0], "gate", json!({"clear": true})).await
                        };
                        let _ = tokio::join!(r1, r2, clr);
                        trace.push(format!("round {round}: concurrent ready(#7,#8) + gate clear"));
                    }
                    1 => {
                        // Reconcile races with adopt + PR merge/move
                        {
                            let m = gh.get_mut(&7).unwrap();
                            if m.state == "OPEN" {
                                if rng.coin() {
                                    m.head_idx = (m.head_idx + 1) % HEADS.len();
                                } else {
                                    m.state = "MERGED";
                                }
                            }
                        }
                        sync_gh(&h, &gh);
                        let from = TICKETS[rng.pick(2)];
                        let to = if from == TICKETS[0] { TICKETS[1] } else { TICKETS[2] };
                        let rec = h.app.wf.reconcile();
                        let ad = async {
                            tokio::time::sleep(Duration::from_millis(6)).await;
                            h.wf(to, "adopt", json!({"from": from})).await
                        };
                        let (r, _) = tokio::join!(rec, ad);
                        r.unwrap();
                        trace.push(format!("round {round}: reconcile racing with {to} adopt({from})"));
                    }
                    2 => {
                        // ready races with updated to new head + reconcile
                        let old_head = HEADS[gh[&8].head_idx];
                        let next_idx = (gh[&8].head_idx + 1) % HEADS.len();
                        let new_head = HEADS[next_idx];
                        let rdy =
                            h.wf(WORKERS[1], "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": old_head}));
                        let upd = async {
                            tokio::time::sleep(Duration::from_millis(7)).await;
                            h.wf(WORKERS[1], "pr", json!({"event": "updated", "repo": REPO, "pr": 8, "head": new_head}))
                                .await
                        };
                        let rec = h.app.wf.reconcile();
                        let (_, _, r) = tokio::join!(rdy, upd, rec);
                        r.unwrap();
                        gh.get_mut(&8).unwrap().head_idx = next_idx;
                        sync_gh(&h, &gh);
                        trace.push(format!("round {round}: ready(#8) racing with updated + reconcile"));
                    }
                    3 => {
                        // opened races with adopt
                        let from = TICKETS[rng.pick(2)];
                        let to = TICKETS[2];
                        let op = h.wf(
                            WORKERS[2],
                            "pr",
                            json!({"event": "opened", "repo": REPO, "pr": 9, "head": HEADS[gh[&9].head_idx],
                                   "publishing": PUBLISHING}),
                        );
                        let ad = async {
                            tokio::time::sleep(Duration::from_millis(6)).await;
                            h.wf(to, "adopt", json!({"from": from})).await
                        };
                        let _ = tokio::join!(op, ad);
                        trace.push(format!("round {round}: opened(#9) racing with {to} adopt({from})"));
                    }
                    _ => {
                        // pending ready + reconcile + answer open cards concurrently
                        {
                            let m = gh.get_mut(&9).unwrap();
                            if m.state == "OPEN" {
                                m.checks =
                                    if m.checks == GhChecks::Pending { GhChecks::Passing } else { GhChecks::Pending };
                            }
                        }
                        sync_gh(&h, &gh);
                        let rdy = h.wf(
                            WORKERS[2],
                            "pr",
                            json!({"event": "ready", "repo": REPO, "pr": 9, "head": HEADS[gh[&9].head_idx]}),
                        );
                        let rec = h.app.wf.reconcile();
                        let open = h.app.wf.cards().unwrap();
                        let ans = async {
                            if let Some(c) = open.first() {
                                let opt = c.options.first().map(String::as_str).unwrap_or("ok");
                                let _ = h.app.wf.answer(&c.id, opt).await;
                            }
                        };
                        let (_, r, _) = tokio::join!(rdy, rec, ans);
                        r.unwrap();
                        trace.push(format!("round {round}: ready(#9) + reconcile + answer"));
                    }
                }
            })
            .await;
            assert!(res.is_ok(), "concurrent race timed out (deadlock?) at seed={seed}\n{}", trace.join("\n"));
            assert_invariants(&h, seed, &trace, &BTreeMap::new(), false, &gh);
        }

        h.github.set_delay(Duration::ZERO);
        h.app.wf.reconcile().await.unwrap();
        assert_invariants(&h, seed, &trace, &BTreeMap::new(), true, &gh);
    }
}
