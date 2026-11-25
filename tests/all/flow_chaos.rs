use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, Card, CardRef, LaunchArgs};
use t3_codemode::clock::{Clock, iso};
use t3_codemode::config::{Config, Launch};
use t3_codemode::fake::{check_run, github_issue, github_pr, project, thread};

const REPO: &str = "acme/widgets";
const TOOLS: &str = "acme/tools";
const FLOW: &str = "flow";
const HEADS: &[&str] = &[
    "1111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333",
];
const GOOD_BODY: &str = "## Summary\n\n```text\nwf.pr\n  flow-auto\n```\n\n## Evidence\n\n- **Before:** old\n  **After:** new\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\nRefs #101\n";
const PUBLISHING: &str = "Publishing: pr skill; passes: none configured";
const INIT_TICKETS: &[(&str, u64)] = &[("mcp:t1", 101), ("mcp:t2", 102), ("mcp:t3", 103)];
const INIT_WORKERS: &[(&str, u64)] = &[("mcp:w1", 7), ("mcp:w2", 8), ("mcp:w3", 9)];

fn same_head(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

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
}

#[derive(Clone, Debug)]
struct GhPrModel {
    head_idx: usize,
    state: &'static str,
    draft: bool,
    merge_state: &'static str,
    checks: GhChecks,
    ended_at: Option<String>,
    merge_commit: Option<String>,
}

impl Default for GhPrModel {
    fn default() -> Self {
        Self {
            head_idx: 0,
            state: "OPEN",
            draft: false,
            merge_state: "CLEAN",
            checks: GhChecks::Passing,
            ended_at: None,
            merge_commit: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TicketProjection {
    gate: Option<String>,
    gate_pr: Option<(String, u64)>,
    ready_cards: BTreeSet<(String, String, u64)>,
    relayed_cards: BTreeSet<String>,
    own_gate_cards: BTreeSet<String>,
}

#[derive(Default, Debug)]
struct ScenarioCounters {
    auto_launches_ok: usize,
    auto_launches_failed: usize,
    joined_flow_tickets: usize,
    flow_cards_coexisted_with_ready: usize,
    flow_cards_retired_by_auto: usize,
    flow_cards_answered: usize,
    ticket_closes_handoff: usize,
    ticket_closes_board: usize,
    ticket_closes_archived: usize,
    flow_next_calls: usize,
    direct_launches: usize,
    ready_cards_opened: usize,
    ready_cards_answered: usize,
    concurrent_races: usize,
}

fn flow_config_json(max_tickets: usize, launch: &str) -> Value {
    json!({"flow": {"repos": [REPO, TOOLS], "optIn": {"label": FLOW}, "maxTickets": max_tickets, "launch": launch}})
}

async fn set_flow_launch(h: &H, max_tickets: usize, launch: &str) {
    let path = h.dir.path().join("t3cm-config.json");
    std::fs::write(&path, flow_config_json(max_tickets, launch).to_string()).unwrap();
    h.app.wf.reload_config().await.unwrap();
}

fn put_issue(h: &H, repo: &str, number: u64, labels: &[&str], day: u32) {
    let created = format!("2026-10-{day:02}T09:00:00Z");
    h.github.set_issue(repo, github_issue(number, &format!("Issue {number}"), labels, &created));
}

fn sync_gh(h: &H, prs: &BTreeMap<u64, GhPrModel>) {
    h.github.clear_faults_on("reconcile.pr");
    for (&number, m) in prs {
        let mut pr = github_pr(HEADS[m.head_idx], m.merge_state, false);
        pr["state"] = json!(m.state);
        pr["isDraft"] = json!(m.draft);
        pr["body"] = json!(GOOD_BODY);
        pr["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
        if m.state == "MERGED" {
            let oid = m.merge_commit.as_deref().unwrap_or(HEADS[(m.head_idx + 1) % HEADS.len()]);
            pr["mergeCommit"] = json!({"oid": oid});
            if let Some(at) = &m.ended_at {
                pr["mergedAt"] = json!(at);
            }
        } else if m.state == "CLOSED"
            && let Some(at) = &m.ended_at
        {
            pr["closedAt"] = json!(at);
        }
        h.github.set_pr(REPO, number, pr);
        let checks = match m.checks {
            GhChecks::Pending => {
                json!([{"__typename": "CheckRun", "name": "ci", "status": "IN_PROGRESS", "conclusion": null}])
            }
            GhChecks::Passing => json!([check_run("ci", "SUCCESS")]),
            GhChecks::Failing => json!([check_run("ci", "FAILURE")]),
            GhChecks::Cancelled => json!([check_run("ci", "SUCCESS"), check_run("opt", "CANCELLED")]),
        };
        h.github.set_checks(REPO, number, checks);
    }
}

fn pull_gh_mutations(h: &H, prs: &mut BTreeMap<u64, GhPrModel>) {
    for (&number, m) in prs.iter_mut() {
        let cur = h.github.pr(REPO, number);
        if cur["state"] == "MERGED" && m.state != "MERGED" {
            m.state = "MERGED";
            m.merge_commit = cur["mergeCommit"]["oid"].as_str().map(String::from);
            m.ended_at = cur["mergedAt"].as_str().map(String::from);
        }
    }
}

fn setup_flow_chaos(max_tickets: usize, chained_delivery: bool) -> (H, BTreeMap<u64, GhPrModel>) {
    let raw_cfg = flow_config_json(max_tickets, "auto");
    let h = H::build(Vec::new(), Config::from_value(raw_cfg.clone()).unwrap());
    std::fs::write(h.dir.path().join("t3cm-config.json"), raw_cfg.to_string()).unwrap();

    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for skill in ["deploy", "publish"] {
        let d = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {skill}\ndescription: test\n---\n\nBody.\n")).unwrap();
    }
    let steps = if chained_delivery {
        json!([
            {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
            {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"},
            {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"}
        ])
    } else {
        json!([{"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"}])
    };
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, json!({"delivery": {"steps": steps}}).to_string()).unwrap();

    h.github.add_repo(REPO);
    h.github.add_repo(TOOLS);
    let repo_str = repo.to_string_lossy().to_string();
    h.fake.set_projects(vec![project("proj:widgets", &repo_str)]);
    h.remotes.set(&repo_str, "git@github.com:acme/widgets.git");

    for &(t, n) in INIT_TICKETS {
        let mut th = thread(t, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/{t}"));
        h.fake.put(th);
        put_issue(&h, REPO, n, &[FLOW], (n - 100) as u32);
    }
    for (i, &(w, _)) in INIT_WORKERS.iter().enumerate() {
        let mut th = thread(w, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/worker-{i}"));
        h.fake.put(th);
    }

    // Flow queue: mixed priorities, an unblocked issue, a blocked issue, a sub-issue, and an unmapped repo issue.
    put_issue(&h, REPO, 104, &[FLOW, "priority: high"], 4);
    put_issue(&h, REPO, 105, &[FLOW, "priority: medium"], 5);
    put_issue(&h, REPO, 106, &[FLOW, "priority: medium"], 6);
    put_issue(&h, REPO, 107, &[FLOW, "priority: low"], 7);
    put_issue(&h, REPO, 108, &[FLOW], 8);
    put_issue(&h, REPO, 109, &[FLOW, "priority: high"], 9);
    put_issue(&h, REPO, 110, &[FLOW], 10);
    put_issue(&h, REPO, 111, &[FLOW, "priority: high"], 11);
    put_issue(&h, REPO, 190, &[], 1);
    put_issue(&h, REPO, 191, &[], 2);
    h.github.set_blockers(REPO, 104, &[(REPO, 190)]);
    h.github.close_issue(REPO, 190);
    h.github.set_blockers(REPO, 109, &[(REPO, 191)]);
    h.github.set_parent(REPO, 111, (REPO, 104));
    put_issue(&h, TOOLS, 201, &[FLOW, "priority: high"], 5);

    let prs: BTreeMap<u64, GhPrModel> = INIT_WORKERS.iter().map(|&(_, pr)| (pr, GhPrModel::default())).collect();
    sync_gh(&h, &prs);
    (h, prs)
}

async fn join_initial(h: &H, worker_owners: [&str; 3]) -> Vec<String> {
    let mut open_tickets = Vec::new();
    for &(t, n) in INIT_TICKETS {
        h.wf(
            t,
            "join",
            json!({"parent": BOARD, "label": format!("widgets#{n}"), "role": "ticket",
                   "issue": format!("{REPO}#{n}")}),
        )
        .await;
        open_tickets.push(t.to_string());
    }
    for (i, &(w, _)) in INIT_WORKERS.iter().enumerate() {
        let owner = worker_owners[i];
        let part = format!("{}/3", i + 1);
        let label = format!("#101 PR {part}");
        h.wf(
            owner,
            "openChild",
            json!({"role": "worker", "label": label, "issue": format!("{REPO}#101"), "part": part,
                   "pr": {"format": "pr"}}),
        )
        .await;
        h.wf(w, "join", json!({"parent": owner, "label": label, "role": "worker"})).await;
    }
    open_tickets
}

fn sent_counts(h: &H) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for s in h.sent() {
        *m.entry(s["threadId"].as_str().unwrap_or_default().to_string()).or_insert(0) += 1;
    }
    m
}

fn open_flow_ids(h: &H) -> BTreeSet<String> {
    h.app.wf.cards().unwrap().into_iter().filter(|c| c.kind == "flow").map(|c| c.id).collect()
}

fn open_error_count(h: &H) -> usize {
    h.app.wf.cards().unwrap().iter().filter(|c| c.kind == "error" && c.ticket.is_none()).count()
}

fn is_valid_flow_card_resolution(note: &str) -> bool {
    matches!(note, "flow.launch is auto" | "flow.launch is off" | "no longer ready" | "no free slot" | "dismissed")
        || note.starts_with("answered: ")
        || note.starts_with("not launched: ")
}

fn snapshot_ticket_projections(h: &H, exclude: Option<&str>) -> BTreeMap<String, TicketProjection> {
    let cards = h.app.wf.cards().unwrap();
    let mut out = BTreeMap::new();
    for tl in h.app.store.children(BOARD, true).unwrap() {
        if exclude.is_some_and(|e| e == tl.child) {
            continue;
        }
        let (mut ready_cards, mut relayed_cards, mut own_gate_cards) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for c in cards.iter().filter(|c| c.ticket.as_deref() == Some(tl.child.as_str())) {
            if c.worker.is_some() {
                relayed_cards.insert(c.id.clone());
            } else if c.kind == "gate" {
                own_gate_cards.insert(c.id.clone());
                if let Some(p) = c.pr.as_ref().filter(|p| p.ready) {
                    ready_cards.insert((c.id.clone(), p.repo.clone(), p.number));
                }
            }
        }
        out.insert(
            tl.child,
            TicketProjection { gate: tl.gate, gate_pr: tl.gate_pr, ready_cards, relayed_cards, own_gate_cards },
        );
    }
    out
}

fn assert_projections_unchanged(
    before: &BTreeMap<String, TicketProjection>,
    after: &BTreeMap<String, TicketProjection>,
    what: &str,
    ctx: &str,
) {
    for (t, prev) in before {
        let cur = after.get(t).unwrap_or_else(|| panic!("{what} dropped open ticket {t}!\n{ctx}"));
        assert_eq!(cur, prev, "{what} disrupted ticket {t}'s gate or ready-card projection!\n{ctx}");
    }
}

fn assert_flow_pr_gate_invariants(
    h: &H,
    label: &str,
    trace: &[String],
    workers: &[(String, u64)],
    closed_workers: &BTreeMap<String, usize>,
    after_healthy_reconcile: bool,
    gh: &BTreeMap<u64, GhPrModel>,
) {
    let ctx = || format!("context={label}\ntrace:\n{}", trace.join("\n"));
    let cards = h.app.wf.cards().unwrap();
    let all_cards = h.app.wf.cards_with(true).unwrap();

    // 1. Flow-card & ticket==None isolation across open and resolved cards.
    for c in &all_cards {
        if c.kind == "flow" || c.flow.is_some() {
            assert_eq!(c.kind, "flow", "flow payload on non-flow card: {c:?}\n{}", ctx());
            assert!(c.flow.is_some(), "flow card missing flow payload: {c:?}\n{}", ctx());
            assert!(c.ticket.is_none(), "flow card leaked into ticket {:?}: {c:?}\n{}", c.ticket, ctx());
            assert!(c.worker.is_none(), "flow card carried worker ref: {c:?}\n{}", ctx());
            assert!(c.pr.is_none(), "flow card carried PR ref: {c:?}\n{}", ctx());
            if c.resolved.is_some() {
                let res = c.resolution.as_deref().unwrap_or("");
                assert!(
                    is_valid_flow_card_resolution(res),
                    "flow card {} resolved by non-flow path ({res:?}): {c:?}\n{}",
                    c.id,
                    ctx()
                );
            }
        }
        if c.ticket.is_none() {
            assert_ne!(c.kind, "gate", "ticket==None card must never be a gate card: {c:?}\n{}", ctx());
            assert!(c.pr.is_none(), "ticket==None card must never carry PR/ready data: {c:?}\n{}", ctx());
            assert!(c.worker.is_none(), "ticket==None card must never carry worker relay data: {c:?}\n{}", ctx());
        }
    }

    // 2. Board snapshot and tree projection isolation.
    let snap = h.app.wf.snapshot().unwrap();
    let snap_cards = snap["cards"].as_array().unwrap();
    assert_eq!(snap_cards.len(), cards.len(), "snapshot card count drifted\n{}", ctx());
    for sc in snap_cards {
        if sc["kind"] == "flow" || sc["ticket"].is_null() {
            assert!(sc["ticket"].is_null(), "snapshot unticketed card leaked ticket: {sc}\n{}", ctx());
            assert!(sc["pr"].is_null(), "snapshot unticketed card leaked pr: {sc}\n{}", ctx());
            assert!(sc["worker"].is_null(), "snapshot unticketed card leaked worker: {sc}\n{}", ctx());
            assert_ne!(sc["kind"], "gate", "snapshot unticketed card is gate: {sc}\n{}", ctx());
        }
    }
    for row in h.app.wf.tree().unwrap() {
        if row["joined"] == true {
            let tid = row["thread"].as_str().unwrap();
            let tl = h.app.store.link_get(tid).unwrap().unwrap();
            let want = if tl.closed { None } else { tl.gate.as_deref() };
            assert_eq!(row["gate"].as_str(), want, "tree ticket {tid} gate drifted: row={row} link={tl:?}\n{}", ctx());
        }
    }

    // 3. Card metadata, ready-card uniqueness, and worker ownership invariants.
    let mut ready_seen: BTreeSet<(String, String, u64)> = BTreeSet::new();
    for c in &cards {
        let Some(ticket) = c.ticket.as_deref() else { continue };
        let tl = h
            .app
            .store
            .link_get(ticket)
            .unwrap()
            .filter(|l| !l.closed)
            .unwrap_or_else(|| panic!("open card points to closed/missing ticket {ticket}: {c:?}\n{}", ctx()));
        if c.worker.is_none() {
            assert_eq!(c.label.as_deref(), Some(tl.label.as_str()), "card label drifted: {c:?}\n{}", ctx());
            assert_eq!(c.link.as_deref(), tl.link.as_deref(), "card link drifted: {c:?}\n{}", ctx());
            if c.kind == "gate"
                && let Some(p) = c.pr.as_ref().filter(|p| p.ready)
            {
                assert!(c.flow.is_none(), "ready merge card carries flow payload: {c:?}\n{}", ctx());
                assert!(
                    ready_seen.insert((ticket.to_string(), p.repo.clone(), p.number)),
                    "duplicate open ready merge card for ({ticket}, {}, {}): {cards:?}\n{}",
                    p.repo,
                    p.number,
                    ctx()
                );
            }
        } else if let Some(w) = &c.worker {
            let wl = h.app.store.link_get(&w.thread).unwrap().expect("relayed worker link");
            assert!(!wl.closed, "open relayed card points to closed worker: {c:?} wl={wl:?}\n{}", ctx());
            assert_eq!(wl.parent, ticket, "relayed card ticket != worker parent: {c:?} wl={wl:?}\n{}", ctx());
        }
    }

    // 4. Ticket gate invariants across all board tickets (initial + flow-launched, open + closed).
    for tl in h.app.store.children(BOARD, false).unwrap() {
        let t = tl.child.as_str();
        if tl.closed {
            assert!(tl.gate.is_none(), "closed ticket {t} kept gate {:?}\n{}", tl.gate, ctx());
            assert!(tl.gate_pr.is_none(), "closed ticket {t} kept gate_pr {:?}\n{}", tl.gate_pr, ctx());
            assert!(
                !cards.iter().any(|c| c.ticket.as_deref() == Some(t)),
                "closed ticket {t} kept open cards: {cards:?}\n{}",
                ctx()
            );
            continue;
        }
        let has_ready = cards.iter().any(|c| {
            c.kind == "gate"
                && c.worker.is_none()
                && c.ticket.as_deref() == Some(t)
                && c.pr.as_ref().is_some_and(|p| p.ready)
        });
        let has_relayed = cards.iter().any(|c| c.worker.is_some() && c.ticket.as_deref() == Some(t));
        if has_ready || has_relayed {
            assert!(tl.gate.is_some(), "ticket {t} has open gate/relayed card but gate is None\n{}", ctx());
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

    // 5. Worker link invariants and post-reconcile guarantees.
    let counts = sent_counts(h);
    for (w, &count_at_close) in closed_workers {
        let wl = h.app.store.link_get(w).unwrap().unwrap();
        assert!(wl.closed && wl.gate.is_none() && wl.gate_pr.is_none(), "closed worker {w} dirty: {wl:?}\n{}", ctx());
        assert_eq!(
            counts.get(w).copied().unwrap_or(0),
            count_at_close,
            "closed worker {w} woke after close\n{}",
            ctx()
        );
    }
    if after_healthy_reconcile {
        if h.app.wf.config().flow.as_ref().is_some_and(|f| f.launch == Launch::Auto) {
            let open_flow: Vec<&Card> = cards.iter().filter(|c| c.kind == "flow").collect();
            assert!(open_flow.is_empty(), "launch=auto left open flow cards after reconcile: {open_flow:?}\n{}", ctx());
        }
        for (w, _) in workers {
            let Some(wl) = h.app.store.link_get(w).unwrap().filter(|l| !l.closed) else { continue };
            if let Some(p) = wl.pr.as_ref().filter(|p| p.waiting)
                && let Some(m) = gh.get(&p.number)
            {
                let should_wait = m.state == "OPEN"
                    && !m.draft
                    && same_head(HEADS[m.head_idx], &p.head)
                    && (m.checks == GhChecks::Pending
                        || (matches!(m.checks, GhChecks::Passing | GhChecks::Cancelled) && m.merge_state == "UNKNOWN"));
                assert!(should_wait, "worker {w} stuck waiting after reconcile: pr={p:?} gh={m:?}\n{}", ctx());
            }
        }
    }

    // 6. Active board tickets and pending launches never duplicate an issue slug.
    let mut active_issues: BTreeSet<String> = BTreeSet::new();
    for tl in h.app.store.children(BOARD, true).unwrap() {
        if let Some(iss) = tl.identity.issue.as_deref() {
            assert!(active_issues.insert(iss.to_ascii_lowercase()), "duplicate open ticket for {iss}\n{}", ctx());
        }
    }
    for p in h.app.store.pending_all().unwrap().into_iter().filter(|p| p.parent == BOARD) {
        if let Some(iss) = p.identity.issue.as_deref() {
            assert!(active_issues.insert(iss.to_ascii_lowercase()), "duplicate pending ticket for {iss}\n{}", ctx());
        }
    }
}

async fn assert_flow_view_consistent(h: &H, ctx: &str) {
    let view = h.app.wf.flow(false).await.unwrap();
    assert_eq!(view["enabled"], true, "flow view disabled: {view}\n{ctx}");
    let max = view["maxTickets"].as_u64().unwrap() as usize;
    let running = view["running"].as_array().unwrap();
    let free = view["free"].as_u64().unwrap() as usize;
    assert_eq!(free, max.saturating_sub(running.len()), "flow free slots mismatch: {view}\n{ctx}");
    let running_keys: BTreeSet<(String, u64)> = running
        .iter()
        .map(|r| (r["repo"].as_str().unwrap().to_ascii_lowercase(), r["number"].as_u64().unwrap()))
        .collect();
    let mut seen: BTreeSet<(String, u64)> = BTreeSet::new();
    for list_name in ["queue", "blocked", "skipped"] {
        for item in view[list_name].as_array().unwrap() {
            let key = (item["repo"].as_str().unwrap().to_ascii_lowercase(), item["number"].as_u64().unwrap());
            assert!(!running_keys.contains(&key), "issue {key:?} in both running and {list_name}\n{ctx}");
            assert!(seen.insert(key.clone()), "issue {key:?} in multiple flow lists ({list_name})\n{ctx}");
        }
    }
}

async fn run_deterministic_isolation_phase(counters: &mut ScenarioCounters) {
    let (h, gh) = setup_flow_chaos(4, true);
    let mut trace = Vec::new();
    let workers: Vec<(String, u64)> = INIT_WORKERS.iter().map(|&(w, p)| (w.to_string(), p)).collect();
    let closed_workers = BTreeMap::new();
    join_initial(&h, ["mcp:t1", "mcp:t1", "mcp:t2"]).await;

    // Open two ready merge cards on t1 and a relayed review gate on t2.
    for &(w, pr) in &INIT_WORKERS[..2] {
        let r = h.wf(w, "pr", json!({"event": "ready", "repo": REPO, "pr": pr, "head": HEADS[0]})).await;
        assert_eq!(r["status"], "gated", "expected gated for {w} PR #{pr}: {r}");
        counters.ready_cards_opened += 1;
    }
    let rel = h
        .wf(
            "mcp:w3",
            "send",
            json!({"kind": "gate", "gate": "review",
                   "body": "**ACTION NEEDED:** review: reply \"fix\" or \"ship\"", "options": ["fix", "ship"]}),
        )
        .await;
    assert!(rel["card"].is_string(), "expected relayed card: {rel}");

    // Raise two live kind=="flow" cards (ticket==None), then switch back to launch="auto" so they
    // coexist with open ready merge cards and relayed gates while launch=="auto".
    set_flow_launch(&h, 5, "ask").await;
    let fc1 = h.app.wf.flow_next().await.unwrap()["picked"][0]["card"].as_str().unwrap().to_string();
    let fc2 = h.app.wf.flow_next().await.unwrap()["picked"][0]["card"].as_str().unwrap().to_string();
    set_flow_launch(&h, 5, "auto").await;
    trace.push(format!("det: raised flow cards {fc1}, {fc2} then reloaded launch=auto"));
    counters.flow_cards_coexisted_with_ready += 1;
    assert_flow_pr_gate_invariants(&h, "det:coexist", &trace, &workers, &closed_workers, false, &gh);

    // Ticket gate clear on t1 must clear PR #7, promote PR #8, and leave both flow cards untouched.
    h.wf("mcp:t1", "gate", json!({"clear": true})).await;
    trace.push("det: t1 gate clear".into());
    assert!(open_flow_ids(&h).contains(&fc1) && open_flow_ids(&h).contains(&fc2), "gate clear swept flow cards!");
    assert_eq!(h.app.store.link_get("mcp:t1").unwrap().unwrap().gate_pr, Some((REPO.into(), 8)));
    assert_flow_pr_gate_invariants(&h, "det:after-gate-clear", &trace, &workers, &closed_workers, false, &gh);

    // Adopting t1 into t3 moves PR #8's ready merge card and leaves both flow cards untouched.
    h.wf("mcp:t3", "adopt", json!({"from": "mcp:t1"})).await;
    trace.push("det: t3 adopt t1".into());
    assert!(open_flow_ids(&h).contains(&fc1) && open_flow_ids(&h).contains(&fc2), "adopt swept flow cards!");
    assert_eq!(h.app.store.link_get("mcp:t3").unwrap().unwrap().gate_pr, Some((REPO.into(), 8)));
    assert_flow_pr_gate_invariants(&h, "det:after-adopt", &trace, &workers, &closed_workers, false, &gh);

    // Archiving t2 + tick() sets deferred flow_soon() without launching or sweeping flow cards yet.
    let before_archive = snapshot_ticket_projections(&h, Some("mcp:t2"));
    let launched_before_archive = h.fake.launched().len();
    h.fake.update("mcp:t2", json!({"archived": true}));
    h.tick(10).await;
    counters.ticket_closes_archived += 1;
    trace.push("det: archive t2 + tick(10) -> deferred flow_soon".into());
    assert_eq!(h.fake.launched().len(), launched_before_archive, "tick() should defer flow_soon until flow_if_due");
    assert_projections_unchanged(&before_archive, &snapshot_ticket_projections(&h, None), "archive t2", "det:archive");
    assert!(open_flow_ids(&h).contains(&fc1) && open_flow_ids(&h).contains(&fc2), "archive swept flow cards!");
    assert_flow_pr_gate_invariants(&h, "det:after-archive", &trace, &workers, &closed_workers, false, &gh);

    // Answering fc1 with "skip" under launch="auto" triggers flow_pick(None): retires fc2 with
    // "flow.launch is auto", fails on unmapped tools#201 with a ticket==None error card, auto-launches
    // ready widgets issues into the remaining free slots, and leaves t3 untouched.
    let before_skip = snapshot_ticket_projections(&h, None);
    let launched_before_skip = h.fake.launched().len();
    h.app.wf.answer(&fc1, "skip").await.unwrap();
    counters.flow_cards_answered += 1;
    trace.push(format!("det: answer {fc1} skip -> auto retire {fc2} + auto-launch"));
    assert_projections_unchanged(&before_skip, &snapshot_ticket_projections(&h, None), "answer flow skip", "det:skip");
    let all_after = h.app.wf.cards_with(true).unwrap();
    assert_eq!(all_after.iter().find(|c| c.id == fc2).unwrap().resolution.as_deref(), Some("flow.launch is auto"));
    counters.flow_cards_retired_by_auto += 1;
    let newly_launched = h.fake.launched().len() - launched_before_skip;
    assert!(newly_launched >= 1, "expected auto-launch after flow skip under launch=auto");
    counters.auto_launches_ok += newly_launched;
    let det_errs = all_after.iter().filter(|c| c.kind == "error" && c.ticket.is_none()).count();
    assert!(det_errs >= 1, "expected failed auto-launch error card for unmapped {TOOLS}#201");
    counters.auto_launches_failed += det_errs;
    assert_flow_pr_gate_invariants(&h, "det:after-skip", &trace, &workers, &closed_workers, true, &gh);

    // Fill the remaining slot (if tools#201 failure left one free) so slots are saturated, then verify
    // that close_ticket(t3) itself clears gate_pr and immediately auto-launches the next queued issue.
    h.app.wf.reconcile().await.unwrap();
    let flow_before_close = h.app.wf.flow(false).await.unwrap();
    assert_eq!(flow_before_close["free"], 0, "expected saturated slots before close_ticket: {flow_before_close}");
    let next_queued = flow_before_close["queue"][0]["label"]
        .as_str()
        .expect("expected a queued issue before close_ticket")
        .to_string();
    let before_close = snapshot_ticket_projections(&h, Some("mcp:t3"));
    let launched_before_close = h.fake.launched().len();
    h.fake.update("mcp:t3", json!({"status": "idle"}));
    h.app.wf.close_ticket("mcp:t3", false).await.unwrap();
    counters.ticket_closes_board += 1;
    trace.push(format!("det: close_ticket t3 -> auto-launch {next_queued}"));
    let launched_after_close = h.fake.launched();
    assert_eq!(
        launched_after_close.len(),
        launched_before_close + 1,
        "close_ticket must immediately trigger flow_soon -> auto-launch"
    );
    assert!(
        launched_after_close.last().unwrap()["title"].as_str().unwrap().starts_with(&format!("{next_queued}:")),
        "close_ticket launched wrong issue: {:?}",
        launched_after_close.last()
    );
    counters.auto_launches_ok += 1;
    assert_projections_unchanged(&before_close, &snapshot_ticket_projections(&h, None), "close_ticket t3", "det:close");
    assert_flow_pr_gate_invariants(&h, "det:after-close-ticket", &trace, &workers, &closed_workers, true, &gh);

    // Verify deferred flow_soon path in isolation: keep clock at T+10 (same timestamp as the reconcile()
    // above, so reconcile_if_due() is not due), archive t1 + tick() (defers launch), then verify
    // board_tick() drains flow_soon -> flow_if_due() without running PR reconcile, and an immediate
    // second board_tick() makes zero GitHub calls.
    let flow_before_arch2 = h.app.wf.flow(false).await.unwrap();
    assert_eq!(flow_before_arch2["free"], 0, "expected saturated slots before archiving t1: {flow_before_arch2}");
    let next_after_arch2 = flow_before_arch2["queue"][0]["label"].as_str().unwrap().to_string();
    let launched_before_arch2 = h.fake.launched().len();
    h.fake.update("mcp:t1", json!({"archived": true}));
    h.app.wf.tick().await.unwrap();
    assert_eq!(h.fake.launched().len(), launched_before_arch2, "tick(archived) must defer launch to flow_if_due");
    let gh_ops_before = h.github.operations().len();
    h.app.wf.board_tick().await.unwrap();
    let gh_ops_mid = h.github.operations();
    assert!(
        gh_ops_mid[gh_ops_before..].iter().all(|op| op == "flowIssues" || op == "flowTickets"),
        "expected only flow_if_due queries (no reconcile_if_due) in board_tick, got {:?}",
        &gh_ops_mid[gh_ops_before..]
    );
    let launched_after_drain = h.fake.launched();
    assert_eq!(launched_after_drain.len(), launched_before_arch2 + 1, "board_tick must drain flow_soon and launch");
    assert!(
        launched_after_drain.last().unwrap()["title"].as_str().unwrap().starts_with(&format!("{next_after_arch2}:")),
        "board_tick launched wrong issue: {:?}",
        launched_after_drain.last()
    );
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(
        h.github.operations().len(),
        gh_ops_mid.len(),
        "second immediate board_tick must make zero GitHub calls (confirming reconcile_if_due is idle)"
    );
    counters.auto_launches_ok += 1;
    assert_flow_view_consistent(&h, "det:end").await;
}

fn flow_chaos_seeds(default: usize) -> usize {
    std::env::var("FLOW_CHAOS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::test]
async fn flow_auto_launch_isolation_holds_pr_and_gate_invariants() {
    let mut counters = ScenarioCounters::default();

    // Phase 1: deterministic flow-card coexistence, ask->auto retirement, close-to-refill, and deferred flow_soon.
    run_deterministic_isolation_phase(&mut counters).await;

    // Phase 2: bounded seeded sequential interleaving under config.flow launch="auto".
    let seeds = flow_chaos_seeds(10);
    for seed in 0..seeds as u64 {
        let mut rng = Rng::new(seed ^ 0x2420_f100_u64.wrapping_mul(31));
        let max_tickets = 2 + (seed as usize % 3);
        let (h, mut gh) = setup_flow_chaos(max_tickets, seed % 2 == 1);
        let owners = [INIT_TICKETS[rng.pick(2)].0, INIT_TICKETS[rng.pick(2)].0, INIT_TICKETS[rng.pick(2)].0];
        let mut open_tickets = join_initial(&h, owners).await;
        let mut trace = vec![format!("init workers under {owners:?}")];
        let mut closed_workers: BTreeMap<String, usize> = BTreeMap::new();
        let mut workers: Vec<(String, u64)> = INIT_WORKERS.iter().map(|&(w, p)| (w.to_string(), p)).collect();
        let mut active_workers: Vec<usize> = vec![0, 1, 2];
        let (mut next_issue_num, mut next_pr_num, mut next_worker_seq) = (120u64, 20u64, 4usize);

        for step in 0..32 {
            h.clock.advance_minutes(1);
            let op = rng.pick(22);
            let mut healthy_reconcile = false;
            let res = tokio::time::timeout(Duration::from_secs(5), async {
                match op {
                    0..=3 => {
                        // Worker PR lifecycle: opened (0), updated (1), ready (2 | 3).
                        if active_workers.is_empty() {
                            return;
                        }
                        let wi = active_workers[rng.pick(active_workers.len())];
                        let (ref w, pr) = workers[wi];
                        let head = HEADS[gh[&pr].head_idx];
                        let sha = if rng.coin() { &head[..7] } else { head };
                        let (event, args) = match op {
                            0 => {
                                let checks = if rng.pick(6) == 0 { "none" } else { "pending" };
                                (
                                    "opened",
                                    json!({"event": "opened", "repo": REPO, "pr": pr, "head": sha,
                                           "publishing": PUBLISHING, "checks": checks}),
                                )
                            }
                            1 => ("updated", json!({"event": "updated", "repo": REPO, "pr": pr, "head": sha})),
                            _ => ("ready", json!({"event": "ready", "repo": REPO, "pr": pr, "head": sha})),
                        };
                        let r = h.app.wf.dispatch("pr", Some(w), args).await;
                        if op >= 2
                            && let Ok(ref v) = r
                            && v["status"] == "gated"
                        {
                            counters.ready_cards_opened += 1;
                        }
                        trace.push(format!("#{step} {w} pr {event} #{pr}@{sha} -> ok={}", r.is_ok()));
                    }
                    4 => {
                        // Mutate GitHub PR head, checks/merge_state/draft, or lifecycle.
                        let pr_keys: Vec<u64> = gh.keys().copied().collect();
                        let pr = pr_keys[rng.pick(pr_keys.len())];
                        let now = iso(h.clock.now());
                        let changed = {
                            let m = gh.get_mut(&pr).unwrap();
                            if m.state == "OPEN" {
                                match rng.pick(4) {
                                    0 => m.head_idx = (m.head_idx + 1) % HEADS.len(),
                                    1 | 2 => {
                                        m.checks = [
                                            GhChecks::Pending,
                                            GhChecks::Passing,
                                            GhChecks::Failing,
                                            GhChecks::Cancelled,
                                        ][rng.pick(4)];
                                        m.merge_state = ["CLEAN", "UNKNOWN", "BLOCKED", "BEHIND", "DIRTY"][rng.pick(5)];
                                        if rng.pick(5) == 0 {
                                            m.draft = !m.draft;
                                        }
                                    }
                                    _ => {
                                        m.state = if rng.coin() { "MERGED" } else { "CLOSED" };
                                        m.ended_at = Some(now);
                                    }
                                }
                                trace.push(format!("#{step} gh mutate #{pr} state={} checks={:?}", m.state, m.checks));
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
                        // Ticket gate clear / open / delivery:verify.
                        if open_tickets.is_empty() {
                            return;
                        }
                        let t = open_tickets[rng.pick(open_tickets.len())].clone();
                        match rng.pick(3) {
                            0 => {
                                let _ = h.app.wf.dispatch("gate", Some(&t), json!({"clear": true})).await;
                            }
                            1 => {
                                let _ = h
                                    .app
                                    .wf
                                    .dispatch(
                                        "send",
                                        Some(&t),
                                        json!({"kind": "gate", "gate": "delivery:verify",
                                               "body": "**ACTION NEEDED:** verify: reply \"ok\"", "options": ["ok"]}),
                                    )
                                    .await;
                            }
                            _ => {
                                let _ = h.app.wf.dispatch("gate", Some(&t), json!({"open": "merge"})).await;
                            }
                        }
                        trace.push(format!("#{step} {t} gate op"));
                    }
                    6 => {
                        // Worker relayed review gate or blocker.
                        if active_workers.is_empty() {
                            return;
                        }
                        let w = workers[active_workers[rng.pick(active_workers.len())]].0.clone();
                        let args = if rng.coin() {
                            json!({"kind": "gate", "gate": "review",
                                   "body": "**ACTION NEEDED:** review: reply \"fix\" or \"ship\"",
                                   "options": ["fix", "ship"]})
                        } else {
                            json!({"kind": "blocker", "body": "**ACTION NEEDED:** unblock: reply \"retry\"",
                                   "options": ["retry"]})
                        };
                        let _ = h.app.wf.dispatch("send", Some(&w), args).await;
                        trace.push(format!("#{step} {w} relayed gate/blocker"));
                    }
                    7 => {
                        // Answer or dismiss_at a random open card.
                        let open = h.app.wf.cards().unwrap();
                        if open.is_empty() {
                            return;
                        }
                        let c = &open[rng.pick(open.len())];
                        if rng.pick(3) != 0 {
                            let is_flow = c.kind == "flow";
                            let is_ready =
                                c.kind == "gate" && c.worker.is_none() && c.pr.as_ref().is_some_and(|p| p.ready);
                            let before_proj = is_flow.then(|| snapshot_ticket_projections(&h, None));
                            let launched_before = h.fake.launched().len();
                            let ans = if is_flow {
                                if rng.coin() { "launch" } else { "skip" }
                            } else {
                                c.options.get(rng.pick(c.options.len().max(1))).map(String::as_str).unwrap_or("ok")
                            };
                            let r = h.app.wf.answer(&c.id, ans).await;
                            pull_gh_mutations(&h, &mut gh);
                            if r.is_ok() {
                                if is_flow {
                                    counters.flow_cards_answered += 1;
                                } else if is_ready {
                                    counters.ready_cards_answered += 1;
                                }
                            }
                            counters.auto_launches_ok += h.fake.launched().len().saturating_sub(launched_before);
                            if let Some(before) = before_proj {
                                assert_projections_unchanged(
                                    &before,
                                    &snapshot_ticket_projections(&h, None),
                                    "answer flow card",
                                    &format!("seed={seed} step={step}"),
                                );
                            }
                            trace.push(format!("#{step} answer {} ({}) {ans:?} -> ok={}", c.id, c.kind, r.is_ok()));
                        } else {
                            let _ = h.app.wf.dismiss_at(&c.id, &CardRef::default()).await;
                            trace.push(format!("#{step} dismiss_at {} ({})", c.id, c.kind));
                        }
                    }
                    8 => {
                        // Ticket adopt must never alter open flow cards.
                        if open_tickets.len() < 2 {
                            return;
                        }
                        let i = rng.pick(open_tickets.len());
                        let j = (i + 1 + rng.pick(open_tickets.len() - 1)) % open_tickets.len();
                        let (from, to) = (open_tickets[i].clone(), open_tickets[j].clone());
                        let flow_before = open_flow_ids(&h);
                        let _ = h.app.wf.dispatch("adopt", Some(&to), json!({"from": from})).await;
                        assert_eq!(open_flow_ids(&h), flow_before, "adopt altered flow cards!\nseed={seed}");
                        trace.push(format!("#{step} {to} adopt {from}"));
                    }
                    9 => {
                        // Answer ready merge card + clear gate; verify answer result and card resolution first,
                        // and verify sibling ready merge cards and open flow cards survive.
                        let open = h.app.wf.cards().unwrap();
                        let ready: Vec<_> = open
                            .iter()
                            .filter(|c| {
                                c.kind == "gate"
                                    && c.worker.is_none()
                                    && c.ticket.is_some()
                                    && c.pr.as_ref().is_some_and(|p| p.ready)
                            })
                            .collect();
                        if ready.is_empty() {
                            return;
                        }
                        let chosen = ready[rng.pick(ready.len())];
                        let ticket = chosen.ticket.clone().unwrap();
                        let others: Vec<String> = ready
                            .iter()
                            .filter(|c| c.ticket.as_deref() == Some(ticket.as_str()) && c.id != chosen.id)
                            .map(|c| c.id.clone())
                            .collect();
                        let flow_before = open_flow_ids(&h);
                        let opt = chosen.options.first().map(String::as_str).unwrap_or("merge");
                        let ans_res = h.app.wf.answer(&chosen.id, opt).await;
                        pull_gh_mutations(&h, &mut gh);
                        if ans_res.is_ok() {
                            let after_ans: BTreeSet<String> =
                                h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                            assert!(
                                !after_ans.contains(&chosen.id),
                                "successful answer left ready card {} open\nseed={seed}\n{}",
                                chosen.id,
                                trace.join("\n")
                            );
                            counters.ready_cards_answered += 1;
                        }
                        let _ = h.app.wf.dispatch("gate", Some(&ticket), json!({"clear": true})).await;
                        let after_ids: BTreeSet<String> = h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                        for other in &others {
                            assert!(after_ids.contains(other), "wiped sibling ready card {other}\nseed={seed}");
                        }
                        for fid in &flow_before {
                            assert!(after_ids.contains(fid), "answer + gate clear wiped flow card {fid}\nseed={seed}");
                        }
                        trace.push(format!(
                            "#{step} answer ready {} ({opt} ok={}) + {ticket} gate clear",
                            chosen.id,
                            ans_res.is_ok()
                        ));
                    }
                    10..=12 => {
                        // Ticket close paths: send(handoff/closed) (10), close_ticket (11), archived+tick (12).
                        if open_tickets.len() <= 1 {
                            return;
                        }
                        let pos = rng.pick(open_tickets.len());
                        let t = open_tickets.remove(pos);
                        let before_others = snapshot_ticket_projections(&h, Some(&t));
                        let launched_before = h.fake.launched().len();
                        match op {
                            10 => {
                                let kind = if rng.coin() { "handoff" } else { "closed" };
                                h.wf(&t, "send", json!({"kind": kind, "body": format!("{t} {kind}")})).await;
                                counters.ticket_closes_handoff += 1;
                                trace.push(format!("#{step} {t} send {kind} -> flow_soon"));
                            }
                            11 => {
                                h.fake.update(&t, json!({"status": "idle"}));
                                h.app.wf.close_ticket(&t, rng.coin()).await.unwrap();
                                counters.ticket_closes_board += 1;
                                trace.push(format!("#{step} close_ticket({t}) -> flow_soon"));
                            }
                            _ => {
                                let flow_before = open_flow_ids(&h);
                                h.fake.update(&t, json!({"archived": true}));
                                h.clock.advance_minutes(5);
                                h.app.wf.tick().await.unwrap();
                                assert_eq!(open_flow_ids(&h), flow_before, "tick(archived) swept flow cards!");
                                counters.ticket_closes_archived += 1;
                                trace.push(format!("#{step} archive {t} + tick -> deferred flow_soon"));
                            }
                        }
                        counters.auto_launches_ok += h.fake.launched().len().saturating_sub(launched_before);
                        assert_projections_unchanged(
                            &before_others,
                            &snapshot_ticket_projections(&h, None),
                            "ticket close + flow_soon",
                            &format!("seed={seed} step={step}"),
                        );
                    }
                    13 | 14 => {
                        // Explicit flow_pick via flow_next().
                        let before = snapshot_ticket_projections(&h, None);
                        let launched_before = h.fake.launched().len();
                        let r = h.app.wf.flow_next().await.unwrap();
                        counters.flow_next_calls += 1;
                        counters.auto_launches_ok += h.fake.launched().len().saturating_sub(launched_before);
                        assert_projections_unchanged(
                            &before,
                            &snapshot_ticket_projections(&h, None),
                            "wf.flow_next()",
                            &format!("seed={seed} step={step}"),
                        );
                        trace.push(format!(
                            "#{step} flow_next -> picked={}",
                            r["picked"].as_array().map_or(0, Vec::len)
                        ));
                    }
                    15 | 16 => {
                        // Reconcile (PR reconcile + flow_pick(None)) + idempotency when no launch failed.
                        let launched_before = h.fake.launched().len();
                        let err_before = open_error_count(&h);
                        let rec = h.app.wf.reconcile().await;
                        counters.auto_launches_ok += h.fake.launched().len().saturating_sub(launched_before);
                        let launch_failed_here = open_error_count(&h) > err_before;
                        trace.push(format!("#{step} reconcile -> ok={}", rec.is_ok()));
                        if rec.is_ok() {
                            healthy_reconcile = true;
                            if !launch_failed_here {
                                let before_msgs = h.sent().len();
                                let before_launched = h.fake.launched().len();
                                let before_cards: Vec<String> =
                                    h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                                h.app.wf.reconcile().await.unwrap();
                                let after_cards: Vec<String> =
                                    h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                                assert_eq!(h.sent().len(), before_msgs, "reconcile not idempotent (msgs)\nseed={seed}");
                                assert_eq!(
                                    h.fake.launched().len(),
                                    before_launched,
                                    "reconcile not idempotent (launches)\nseed={seed}"
                                );
                                assert_eq!(after_cards, before_cards, "reconcile not idempotent (cards)\nseed={seed}");
                            }
                        }
                    }
                    17 => {
                        // board_tick() drains deferred flow_soon via flow_if_due().
                        let launched_before = h.fake.launched().len();
                        h.app.wf.board_tick().await.unwrap();
                        counters.auto_launches_ok += h.fake.launched().len().saturating_sub(launched_before);
                        trace.push(format!("#{step} board_tick"));
                    }
                    18 => {
                        // Join a pending auto-launched ticket and optionally attach a worker + PR.
                        let pending: Vec<_> = h
                            .app
                            .store
                            .pending_all()
                            .unwrap()
                            .into_iter()
                            .filter(|p| p.parent == BOARD && p.thread.is_some())
                            .collect();
                        if pending.is_empty() {
                            return;
                        }
                        let p = &pending[rng.pick(pending.len())];
                        let tid = p.thread.clone().unwrap();
                        let mut args = json!({"parent": BOARD, "label": p.label, "role": "ticket"});
                        if let Some(ref iss) = p.identity.issue {
                            args["issue"] = json!(iss);
                        }
                        if let Some(ref par) = p.identity.parent_issue {
                            args["parentIssue"] = json!(par);
                        }
                        h.wf(&tid, "join", args).await;
                        open_tickets.push(tid.clone());
                        counters.joined_flow_tickets += 1;
                        if workers.len() < 6 && rng.coin() {
                            let w = format!("mcp:w{next_worker_seq}");
                            next_worker_seq += 1;
                            let pr = next_pr_num;
                            next_pr_num += 1;
                            let mut th = thread(&w, "idle", T0);
                            th["projectId"] = json!("proj:widgets");
                            th["link"] = json!(format!("https://t3.local/{w}"));
                            h.fake.put(th);
                            gh.insert(pr, GhPrModel::default());
                            sync_gh(&h, &gh);
                            let label = format!("{} PR 1/1", p.label);
                            let iss = p.identity.issue.clone().unwrap_or_else(|| format!("{REPO}#101"));
                            h.wf(
                                &tid,
                                "openChild",
                                json!({"role": "worker", "label": label, "issue": iss, "part": "1/1",
                                       "pr": {"format": "pr"}}),
                            )
                            .await;
                            h.wf(&w, "join", json!({"parent": tid, "label": label, "role": "worker"})).await;
                            active_workers.push(workers.len());
                            workers.push((w, pr));
                        }
                        trace.push(format!("#{step} joined flow ticket {tid} ({})", p.label));
                    }
                    19 => {
                        // Direct launch_ticket (contends on flow_lock).
                        let before = snapshot_ticket_projections(&h, None);
                        let num = next_issue_num;
                        next_issue_num += 1;
                        put_issue(&h, REPO, num, &[FLOW, "priority: low"], 15);
                        let r = h
                            .app
                            .wf
                            .launch_ticket(LaunchArgs {
                                issue: format!("{REPO}#{num}"),
                                parent_issue: None,
                                part: None,
                                project: None,
                                title: Some(format!("Manual {num}")),
                                model: None,
                                effort: None,
                            })
                            .await;
                        if let Ok(ref v) = r
                            && v["ok"] == true
                        {
                            counters.direct_launches += 1;
                        }
                        assert_projections_unchanged(
                            &before,
                            &snapshot_ticket_projections(&h, None),
                            "direct launch_ticket",
                            &format!("seed={seed} step={step}"),
                        );
                        trace.push(format!("#{step} direct launch #{num} -> ok={}", r.is_ok()));
                    }
                    20 => {
                        // GitHub issue mutations or worker close/handoff.
                        match rng.pick(4) {
                            0 => {
                                h.github.close_issue(REPO, 191);
                                trace.push(format!("#{step} gh close blocker #191"));
                            }
                            1 => {
                                let num = next_issue_num;
                                next_issue_num += 1;
                                put_issue(&h, REPO, num, &[FLOW, "priority: high"], 18);
                                trace.push(format!("#{step} gh add flow issue #{num}"));
                            }
                            2 if !open_tickets.is_empty() => {
                                let t = &open_tickets[rng.pick(open_tickets.len())];
                                if let Some(ln) = h.app.store.link_get(t).unwrap()
                                    && let Some((repo, num_s)) =
                                        ln.identity.issue.as_deref().and_then(|s| s.split_once('#'))
                                    && let Ok(num) = num_s.parse::<u64>()
                                {
                                    h.github.close_issue(repo, num);
                                    trace.push(format!("#{step} gh close issue {repo}#{num} of {t}"));
                                }
                            }
                            _ if active_workers.len() > 1 => {
                                let wi = active_workers.remove(rng.pick(active_workers.len()));
                                let w = workers[wi].0.clone();
                                let parent = h.app.store.link_get(&w).unwrap().unwrap().parent;
                                if rng.coin() {
                                    let _ = h.app.wf.dispatch("close", Some(&parent), json!({"child": w})).await;
                                } else {
                                    let _ = h
                                        .app
                                        .wf
                                        .dispatch("send", Some(&w), json!({"kind": "handoff", "body": "done"}))
                                        .await;
                                }
                                let count = sent_counts(&h).get(&w).copied().unwrap_or(0);
                                closed_workers.insert(w.clone(), count);
                                trace.push(format!("#{step} closed worker {w} (sent={count})"));
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        // Pulse a live kind=="flow" card into launch="auto" so flow cards coexist directly
                        // with randomized PR/gate ops until answered or retired by auto flow_pick.
                        let before = snapshot_ticket_projections(&h, None);
                        let occupied = h.app.store.children(BOARD, true).unwrap().len()
                            + h.app.store.pending_all().unwrap().into_iter().filter(|p| p.parent == BOARD).count();
                        set_flow_launch(&h, occupied.max(max_tickets) + 1, "ask").await;
                        let r = h.app.wf.flow_next().await.unwrap();
                        set_flow_launch(&h, max_tickets, "auto").await;
                        assert_projections_unchanged(
                            &before,
                            &snapshot_ticket_projections(&h, None),
                            "pulse flow card",
                            &format!("seed={seed} step={step}"),
                        );
                        let cards_now = h.app.wf.cards().unwrap();
                        let has_flow = cards_now.iter().any(|c| c.kind == "flow");
                        let has_ready = cards_now.iter().any(|c| {
                            c.kind == "gate"
                                && c.worker.is_none()
                                && c.ticket.is_some()
                                && c.pr.as_ref().is_some_and(|p| p.ready)
                        });
                        if has_flow && has_ready {
                            counters.flow_cards_coexisted_with_ready += 1;
                        }
                        trace.push(format!(
                            "#{step} pulse flow card -> picked={}",
                            r["picked"].as_array().map_or(0, Vec::len)
                        ));
                    }
                }
            })
            .await;
            assert!(res.is_ok(), "step timed out at seed={seed}\n{}", trace.join("\n"));
            assert_flow_pr_gate_invariants(
                &h,
                &format!("seq:seed={seed}:step={step}"),
                &trace,
                &workers,
                &closed_workers,
                healthy_reconcile,
                &gh,
            );
        }

        h.app.wf.reconcile().await.unwrap();
        let all_cards = h.app.wf.cards_with(true).unwrap();
        counters.flow_cards_retired_by_auto += all_cards
            .iter()
            .filter(|c| c.kind == "flow" && c.resolution.as_deref() == Some("flow.launch is auto"))
            .count();
        counters.auto_launches_failed += all_cards.iter().filter(|c| c.kind == "error" && c.ticket.is_none()).count();
        assert_flow_pr_gate_invariants(
            &h,
            &format!("seq:seed={seed}:final"),
            &trace,
            &workers,
            &closed_workers,
            true,
            &gh,
        );
        assert_flow_view_consistent(&h, &format!("seq:seed={seed}:final")).await;
    }

    // Phase 3: bounded concurrent races combining flow_soon/flow_pick/launch_ticket with PR/gate ops.
    let race_seeds = flow_chaos_seeds(4).min(12);
    for seed in 0..race_seeds as u64 {
        let mut rng = Rng::new(seed ^ 0x55aa_f100_u64);
        let (h, mut gh) = setup_flow_chaos(3, seed % 2 == 1);
        h.fake.set_jitter(Some(seed));
        h.github.set_jitter(Some(seed));
        join_initial(&h, ["mcp:t1", "mcp:t1", "mcp:t2"]).await;
        let workers: Vec<(String, u64)> = INIT_WORKERS.iter().map(|&(w, p)| (w.to_string(), p)).collect();
        let closed_workers = BTreeMap::new();
        let mut trace = Vec::new();

        h.github.set_delay(Duration::from_millis(5));
        for round in 0..4 {
            let race = (seed as usize + round) % 4;
            let res = tokio::time::timeout(Duration::from_secs(5), async {
                match race {
                    0 => {
                        h.fake.update("mcp:t3", json!({"status": "idle"}));
                        let rdy = h.app.wf.dispatch(
                            "pr",
                            Some("mcp:w1"),
                            json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[gh[&7].head_idx]}),
                        );
                        let close = h.app.wf.close_ticket("mcp:t3", false);
                        let pick = h.app.wf.flow_next();
                        let (r_pr, r_close, r_pick) = tokio::join!(rdy, close, pick);
                        r_pr.unwrap();
                        r_pick.unwrap();
                        trace.push(format!(
                            "round {round}: ready(#7) + close_ticket(t3 ok={}) + flow_next",
                            r_close.is_ok()
                        ));
                    }
                    1 => {
                        let num = 150 + seed * 10 + round as u64;
                        put_issue(&h, REPO, num, &[FLOW], 20);
                        let rec = h.app.wf.reconcile();
                        let launch = h.app.wf.launch_ticket(LaunchArgs {
                            issue: format!("{REPO}#{num}"),
                            parent_issue: None,
                            part: None,
                            project: None,
                            title: Some(format!("Race {num}")),
                            model: None,
                            effort: None,
                        });
                        let clr = h.app.wf.dispatch("gate", Some("mcp:t1"), json!({"clear": true}));
                        let (r_rec, r_launch, _) = tokio::join!(rec, launch, clr);
                        r_rec.unwrap();
                        assert!(r_launch.is_ok(), "direct launch_ticket failed in race: {r_launch:?}");
                        trace.push(format!("round {round}: reconcile + launch_ticket(#{num}) + t1 gate clear"));
                    }
                    2 => {
                        let handoff = h.app.wf.dispatch(
                            "send",
                            Some("mcp:t2"),
                            json!({"kind": "handoff", "body": "race handoff"}),
                        );
                        let rdy = h.app.wf.dispatch(
                            "pr",
                            Some("mcp:w2"),
                            json!({"event": "ready", "repo": REPO, "pr": 8, "head": HEADS[gh[&8].head_idx]}),
                        );
                        let adopt = h.app.wf.dispatch("adopt", Some("mcp:t1"), json!({"from": "mcp:t2"}));
                        let (_, r_pr, _) = tokio::join!(handoff, rdy, adopt);
                        r_pr.unwrap();
                        trace.push(format!("round {round}: t2 handoff(flow_soon) + ready(#8) + t1 adopt(t2)"));
                    }
                    _ => {
                        set_flow_launch(&h, 6, "ask").await;
                        let picked = h.app.wf.flow_next().await.unwrap();
                        set_flow_launch(&h, 6, "auto").await;
                        let card_id = picked["picked"][0]["card"].as_str().map(String::from);
                        let ans = async {
                            if let Some(id) = card_id {
                                let word = if rng.coin() { "skip" } else { "launch" };
                                let _ = h.app.wf.answer(&id, word).await;
                            }
                        };
                        let rdy = h.app.wf.dispatch(
                            "pr",
                            Some("mcp:w1"),
                            json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[gh[&7].head_idx]}),
                        );
                        let rec = h.app.wf.reconcile();
                        let (_, r_pr, r_rec) = tokio::join!(ans, rdy, rec);
                        r_pr.unwrap();
                        r_rec.unwrap();
                        trace.push(format!("round {round}: answer(flow card) + ready(#7) + reconcile"));
                    }
                }
            })
            .await;
            assert!(res.is_ok(), "concurrent race timed out at seed={seed}\n{}", trace.join("\n"));
            pull_gh_mutations(&h, &mut gh);
            counters.concurrent_races += 1;
            assert_flow_pr_gate_invariants(
                &h,
                &format!("race:seed={seed}:round={round}"),
                &trace,
                &workers,
                &closed_workers,
                false,
                &gh,
            );
        }

        h.github.set_delay(Duration::ZERO);
        h.app.wf.reconcile().await.unwrap();
        assert_flow_pr_gate_invariants(
            &h,
            &format!("race:seed={seed}:final"),
            &trace,
            &workers,
            &closed_workers,
            true,
            &gh,
        );
    }

    // Coverage assertions ensuring every exercised path ran (no silent dead zones).
    assert!(counters.auto_launches_ok > 0, "expected successful auto-launches: {counters:?}");
    assert!(counters.auto_launches_failed > 0, "expected failed auto-launch error card: {counters:?}");
    assert!(counters.joined_flow_tickets > 0, "expected auto-launched tickets to join: {counters:?}");
    assert!(counters.flow_cards_coexisted_with_ready > 0, "expected flow + ready coexistence: {counters:?}");
    assert!(counters.flow_cards_retired_by_auto > 0, "expected auto retirement of flow cards: {counters:?}");
    assert!(counters.flow_cards_answered > 0, "expected flow cards answered: {counters:?}");
    assert!(counters.ticket_closes_handoff > 0, "expected handoff/closed closes: {counters:?}");
    assert!(counters.ticket_closes_board > 0, "expected board close_ticket closes: {counters:?}");
    assert!(counters.ticket_closes_archived > 0, "expected archived+tick closes: {counters:?}");
    assert!(counters.flow_next_calls > 0, "expected flow_next calls: {counters:?}");
    assert!(counters.direct_launches > 0, "expected direct launch_ticket calls: {counters:?}");
    assert!(counters.ready_cards_opened > 0, "expected ready merge cards opened: {counters:?}");
    assert!(counters.ready_cards_answered > 0, "expected ready merge cards answered: {counters:?}");
    assert!(counters.concurrent_races > 0, "expected concurrent race rounds: {counters:?}");
}
