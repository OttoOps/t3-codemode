use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, CardRef};
use t3_codemode::fake::{Gate, Rule, SeqIds};
use t3_codemode::reach::ReachPoint;

use super::{HEADS, PUBLISHING, REPO, Rng, TICKETS, WORKERS, orders, setup_chaos};
use crate::common::*;

const BURST_SEEDS: u64 = 24;
const QUIESCE_PASSES: usize = 6;
const STALL: Duration = Duration::from_secs(30);

type Ctx = BTreeMap<&'static str, String>;
type Run = fn(Rc<H>, Rc<Ctx>) -> LocalBoxFuture<'static, String>;

#[derive(Clone, Copy, PartialEq)]
enum At {
    Start,
    Now,
    Before(&'static str, usize),
    After(&'static str, usize),
}

#[derive(Clone, Copy)]
struct Step {
    name: &'static str,
    at: At,
    run: Run,
}

struct Table {
    name: &'static str,
    setup: fn(bool) -> LocalBoxFuture<'static, (H, Ctx)>,
    steps: Vec<Step>,
    queue: bool,
    serial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    statuses: Vec<String>,
    state: Vec<String>,
}

fn status(r: anyhow::Result<Value>) -> String {
    match r {
        Ok(v) => v["status"].as_str().map(String::from).unwrap_or_else(|| {
            if v["dismissed"] == true {
                "dismissed".into()
            } else if v.get("card").is_some() {
                "answered".into()
            } else {
                "ok".into()
            }
        }),
        Err(e) => {
            let e = format!("{e:#}");
            if e.contains("no open board card") { "no card".into() } else { format!("error: {e}") }
        }
    }
}

fn merge(ticket: &'static str, pr: u64, head: usize) -> impl Fn(Rc<H>) -> LocalBoxFuture<'static, String> {
    move |h| {
        Box::pin(async move {
            let args = json!({"repo": REPO, "pr": pr, "head": HEADS[head]});
            status(h.app.wf.dispatch("merge", Some(ticket), args).await)
        })
    }
}

fn op_merge_t1_7(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    merge(TICKETS[0], 7, 0)(h)
}

fn op_reconcile(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move {
        match h.app.wf.reconcile_ran().await {
            Ok(Some(_)) => "ran".into(),
            Ok(None) => "skipped".into(),
            Err(e) => format!("error: {e:#}"),
        }
    })
}

fn ready(worker: &'static str, pr: u64) -> impl Fn(Rc<H>) -> LocalBoxFuture<'static, String> {
    move |h| {
        Box::pin(async move {
            let args = json!({"event": "ready", "repo": REPO, "pr": pr, "head": HEADS[0]});
            status(h.app.wf.dispatch("pr", Some(worker), args).await)
        })
    }
}

fn op_ready_w1_7(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    ready(WORKERS[0], 7)(h)
}

fn op_ready_w2_8(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    ready(WORKERS[1], 8)(h)
}

fn op_push_7(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move {
        if h.github.pr(REPO, 7)["state"] != "OPEN" {
            return "not open".into();
        }
        h.github.push(REPO, 7, HEADS[1]);
        "pushed".into()
    })
}

fn op_answer_merge(h: Rc<H>, c: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move { status(h.app.wf.answer(&c["ready"], &c["option"]).await) })
}

fn op_dismiss_at(h: Rc<H>, c: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move {
        let want = CardRef { at: None, token: Some(&c["token"]) };
        status(h.app.wf.dismiss_at(&c["ready"], &want).await)
    })
}

fn op_adopt(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move { status(h.app.wf.dispatch("adopt", Some(TICKETS[1]), json!({"from": TICKETS[0]})).await) })
}

async fn board(queue: bool) -> H {
    let (h, _) = setup_chaos(true);
    h.app.wf.set_id_source(Arc::new(SeqIds::new("appr-")));
    for &n in &[7, 8, 9] {
        h.github.set_queue(REPO, n, queue, false);
    }
    for (i, &t) in TICKETS.iter().take(2).enumerate() {
        h.wf(t, "join", json!({"parent": BOARD, "label": format!("widgets#{}", 41 + i), "role": "ticket"})).await;
    }
    for (i, &w) in WORKERS.iter().take(2).enumerate() {
        let part = format!("{}/3", i + 1);
        let label = format!("#41 PR {part}");
        let args = json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part,
                          "pr": {"format": "pr"}});
        h.wf(TICKETS[0], "openChild", args).await;
        h.wf(w, "join", json!({"parent": TICKETS[0], "label": label, "role": "worker"})).await;
        let opened = json!({"event": "opened", "repo": REPO, "pr": 7 + i as u64, "head": HEADS[0],
                            "publishing": PUBLISHING});
        assert_eq!(h.wf(w, "pr", opened).await["status"], "open");
    }
    h
}

fn set_checks(h: &H, pr: u64, checks: Value) {
    h.github.set_checks(REPO, pr, checks);
}

fn setup_waiting_merge(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move {
        let h = board(queue).await;
        h.github.set_status(REPO, 7, "BLOCKED", false);
        let r = h.app.wf.dispatch("merge", Some(TICKETS[0]), json!({"repo": REPO, "pr": 7, "head": HEADS[0]})).await;
        assert_eq!(r.unwrap()["status"], "waiting");
        h.github.set_status(REPO, 7, "CLEAN", false);
        (h, Ctx::new())
    })
}

fn setup_ready_and_wait(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move {
        let (h, mut ctx) = ready_card(queue).await;
        h.github.set_status(REPO, 7, "BLOCKED", false);
        let r = h.app.wf.dispatch("merge", Some(TICKETS[0]), json!({"repo": REPO, "pr": 7, "head": HEADS[0]})).await;
        assert_eq!(r.unwrap()["status"], "waiting");
        h.github.set_status(REPO, 7, "CLEAN", false);
        ctx.remove("card");
        (h, ctx)
    })
}

fn setup_ready_waiting(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move {
        let h = board(queue).await;
        set_checks(&h, 7, json!([t3_codemode::fake::pending_check("ci")]));
        let r = h.wf(WORKERS[0], "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[0]})).await;
        assert_eq!(r["status"], "waiting", "{r}");
        set_checks(&h, 7, json!([t3_codemode::fake::check_run("ci", "SUCCESS")]));
        (h, Ctx::new())
    })
}

async fn ready_card(queue: bool) -> (H, Ctx) {
    let h = board(queue).await;
    let r = h.wf(WORKERS[0], "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[0]})).await;
    assert_eq!(r["status"], "gated", "{r}");
    h.app.wf.reconcile().await.unwrap();
    let card = h.app.wf.cards().unwrap().into_iter().find(|c| c.kind == "gate").expect("a ready merge card");
    let mut ctx = Ctx::new();
    ctx.insert("option", card.options.first().cloned().expect("a merge option"));
    ctx.insert("token", card.token.clone());
    ctx.insert("ready", card.id);
    (h, ctx)
}

fn setup_ready_card(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move {
        let (h, ctx) = ready_card(queue).await;
        h.github.push(REPO, 8, HEADS[0]);
        set_checks(&h, 8, json!([t3_codemode::fake::check_run("ci", "SUCCESS")]));
        (h, ctx)
    })
}

fn normalize(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("appr-") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        out.push('#');
        rest = &rest[digits..];
    }
    out.push_str(rest);
    out
}

fn state(h: &H) -> Vec<String> {
    let snap = h.snapshot();
    let mut delivered = snap.delivered.clone();
    delivered.sort();
    let mut out = vec![
        format!("cards {:?}", snap.cards),
        format!("links {:?}", snap.links),
        format!("waits {:?}", snap.merge_waits),
        format!("events {:?}", snap.pr_events),
        format!("seen {:?}", snap.pr_seen),
        format!("outbox {:?}", snap.outbox),
        format!("delivered {delivered:?}"),
    ];
    let mut feed: Vec<String> =
        h.app.wf.feed(200).unwrap().iter().map(|m| format!("{} {} {}", m["kind"], m["ticket"], m["head"])).collect();
    feed.sort();
    out.push(format!("feed {feed:?}"));
    for n in [7, 8, 9] {
        let pr = h.github.pr(REPO, n);
        out.push(format!("github #{n} {} queued={} head={}", pr["state"], pr["isInMergeQueue"], pr["headRefOid"]));
    }
    out.into_iter().map(normalize).collect()
}

async fn quiesce(h: &H) {
    h.github.clear_faults();
    h.fake.clear_faults();
    let mut last = state(h);
    for _ in 0..QUIESCE_PASSES {
        match h.app.wf.reconcile_ran().await {
            Ok(Some(_)) => {}
            Ok(None) => panic!("a reconcile pass still held the lock after every op finished"),
            Err(e) => panic!("reconcile failed after every op finished: {e:#}"),
        }
        let now = state(h);
        if now == last {
            return;
        }
        last = now;
    }
    panic!("reconcile did not reach a fixpoint in {QUIESCE_PASSES} passes:\n{}", last.join("\n"));
}

fn check_safety(h: &H, what: &str) {
    let snap = h.snapshot();
    for ((repo, n), w) in &snap.merge_waits {
        let pr = h.github.pr(repo, *n);
        assert_eq!(pr["state"], "OPEN", "{what}: a merge wait {w:?} outlived #{n}, which is {}", pr["state"]);
    }
    for c in &snap.cards {
        if let (Some((repo, n)), true) = (&c.pr, c.kind == "gate") {
            assert_eq!(h.github.pr(repo, *n)["state"], "OPEN", "{what}: merge card {c:?} outlived its pull request");
        }
    }
    for (child, l) in &snap.links {
        if let Some((repo, n)) = &l.gate_pr {
            let held = snap.cards.iter().any(|c| {
                c.kind == "gate"
                    && c.worker.is_none()
                    && c.ticket.as_deref() == Some(child)
                    && c.pr == Some((repo.clone(), *n))
            });
            assert!(held, "{what}: {child} points its merge gate at #{n}, which holds no card of its own");
        }
    }
    let merged_wakes = |n: u64| {
        h.sent()
            .iter()
            .filter(|s| s["message"].as_str().is_some_and(|m| m.contains(&format!("#{n} merged on GitHub"))))
            .count()
    };
    for n in [7, 8, 9] {
        assert!(merged_wakes(n) <= 1, "{what}: #{n}'s merge woke a thread {} times", merged_wakes(n));
        let merged: Vec<&String> = snap
            .pr_events
            .iter()
            .filter(|(_, pr, e, _)| *pr == n && e.starts_with("merged:"))
            .map(|(_, _, _, to)| to)
            .collect();
        let board = merged.iter().any(|to| to.as_str() == BOARD);
        assert!(!board || merged.len() == 1, "{what}: #{n}'s merge went to the board and to {merged:?}");
    }
}

async fn settle<T: 'static>(what: &str, handle: tokio::task::JoinHandle<T>) -> T {
    match tokio::time::timeout(STALL, handle).await {
        Ok(r) => r.unwrap(),
        Err(_) => panic!("{what} did not finish: a deadlock"),
    }
}

async fn run_table(table: &Table, order: &[usize], serial: bool) -> (Outcome, Rc<H>) {
    let (h, ctx) = (table.setup)(table.queue).await;
    let (h, ctx) = (Rc::new(h), Rc::new(ctx));
    let gates: Vec<Option<Gate>> = table
        .steps
        .iter()
        .map(|s| {
            let (op, n, after) = match s.at {
                At::Start | At::Now => return None,
                _ if serial => return None,
                At::Before(op, n) => (op, n, false),
                At::After(op, n) => (op, n, true),
            };
            let g = Gate::new();
            let on = Rule::on(op).call(n);
            h.github.fault(if after { on.gate_after_effect(&g) } else { on.gate(&g) });
            Some(g)
        })
        .collect();
    let mut running: Vec<Option<tokio::task::JoinHandle<String>>> = table.steps.iter().map(|_| None).collect();
    let mut statuses = vec![String::new(); table.steps.len()];
    for (i, s) in table.steps.iter().enumerate() {
        if let Some(g) = &gates[i] {
            running[i] = Some(tokio::task::spawn_local((s.run)(h.clone(), ctx.clone())));
            let what = format!("{}: {} never reached its gate", table.name, s.name);
            tokio::time::timeout(STALL, g.arrived()).await.unwrap_or_else(|_| panic!("{what}"));
        } else if s.at == At::Start && !serial {
            statuses[i] = (s.run)(h.clone(), ctx.clone()).await;
        }
    }
    for &i in order {
        let s = &table.steps[i];
        let what = format!("{} order {order:?}: {}", table.name, s.name);
        statuses[i] = match running[i].take() {
            Some(handle) => {
                gates[i].as_ref().unwrap().release();
                settle(&what, handle).await
            }
            None => settle(&what, tokio::task::spawn_local((s.run)(h.clone(), ctx.clone()))).await,
        };
    }
    let what = format!("{} order {order:?}", table.name);
    quiesce(&h).await;
    check_safety(&h, &what);
    (Outcome { statuses, state: state(&h) }, h)
}

fn diff(got: &[String], want: &[String]) -> String {
    got.iter()
        .zip(want)
        .filter(|(g, w)| g != w)
        .map(|(g, w)| format!("  sut:    {g}\n  serial: {w}"))
        .collect::<Vec<_>>()
        .join("\n")
}

type Serial<'a> = &'a dyn Fn(Vec<usize>) -> LocalBoxFuture<'a, Outcome>;

const DROPPED: &[&str] = &["superseded", "skipped"];
const SERIAL_LIMIT: usize = 120;

async fn assert_serializable(
    name: &str,
    order: &[usize],
    got: &Outcome,
    k: usize,
    serial: Serial<'_>,
    cache: &mut BTreeMap<Vec<usize>, Vec<(Vec<usize>, Outcome)>>,
) {
    let skip: Vec<usize> = (0..k).filter(|&i| DROPPED.contains(&got.statuses[i].as_str())).collect();
    if !cache.contains_key(&skip) {
        let kept: Vec<usize> = (0..k).filter(|i| !skip.contains(i)).collect();
        let mut outs = Vec::new();
        for o in orders(kept.len()) {
            let o: Vec<usize> = o.into_iter().map(|i| kept[i]).collect();
            outs.push((o.clone(), serial(o).await));
        }
        assert!(outs.len() <= SERIAL_LIMIT, "{name}: {} serial orders is over the {SERIAL_LIMIT} limit", outs.len());
        cache.insert(skip.clone(), outs);
    }
    let outs: Vec<(Vec<usize>, Outcome)> = cache[&skip]
        .iter()
        .cloned()
        .map(|(o, mut want)| {
            for &i in &skip {
                want.statuses[i] = got.statuses[i].clone();
            }
            (o, want)
        })
        .collect();
    if outs.iter().any(|(_, o)| o == got) {
        return;
    }
    let near: Vec<String> = outs
        .iter()
        .map(|(o, want)| format!("serial order {o:?} statuses {:?}:\n{}", want.statuses, diff(&got.state, &want.state)))
        .collect();
    panic!("{name}: schedule {order:?} with statuses {:?} matches no serial order\n{}", got.statuses, near.join("\n"));
}

struct Ran {
    runs: Vec<(Outcome, Rc<H>)>,
}

impl Ran {
    fn reached(&self, point: ReachPoint) -> u64 {
        self.runs.iter().map(|(_, h)| h.app.wf.reach().snapshot().get(point)).sum()
    }

    fn saw(&self, step: usize, status: &str) -> bool {
        self.runs.iter().any(|(o, _)| o.statuses[step] == status)
    }
}

async fn table_holds(table: Table) -> Ran {
    let k = table.steps.len();
    assert!((2..=4).contains(&k), "a race table runs 2 to 4 steps");
    let table = &table;
    let serial = |order: Vec<usize>| -> LocalBoxFuture<'_, Outcome> {
        Box::pin(async move {
            let (got, _) = run_table(table, &order, true).await;
            got
        })
    };
    let mut cache = BTreeMap::new();
    let mut runs = Vec::new();
    let started: Vec<usize> = (0..k).filter(|&i| table.steps[i].at == At::Start).collect();
    let mut schedules: Vec<Vec<usize>> =
        orders(k).into_iter().map(|o| o.into_iter().filter(|i| !started.contains(i)).collect()).collect();
    schedules.dedup();
    schedules.sort();
    schedules.dedup();
    for order in schedules {
        let (got, h) = run_table(table, &order, false).await;
        if table.serial {
            assert_serializable(table.name, &order, &got, k, &serial, &mut cache).await;
        }
        let (again, _) = run_table(table, &order, false).await;
        assert_eq!(got, again, "{} schedule {order:?} is not deterministic", table.name);
        runs.push((got, h));
    }
    Ran { runs }
}

fn local<F: std::future::Future<Output = ()>>(f: F) -> impl std::future::Future<Output = ()> {
    let set = tokio::task::LocalSet::new();
    async move { set.run_until(f).await }
}

fn r1(queue: bool) -> Table {
    let mutation = if queue { "enqueuePullRequest" } else { "mergePullRequest" };
    Table {
        name: if queue { "R1 wf.merge vs advance (queue)" } else { "R1 wf.merge vs advance" },
        setup: setup_waiting_merge,
        steps: vec![
            Step { name: "wf.merge", at: At::After("read", 1), run: op_merge_t1_7 },
            Step { name: "reconcile", at: At::Before(mutation, 1), run: op_reconcile },
            Step { name: "push", at: At::Now, run: op_push_7 },
        ],
        queue,
        serial: true,
    }
}

#[tokio::test(start_paused = true)]
async fn r1_wf_merge_races_advance_in_every_order() {
    local(async {
        for queue in [false, true] {
            let runs = table_holds(r1(queue)).await;
            assert!(runs.reached(ReachPoint::AdvanceMerged) + runs.reached(ReachPoint::AdvanceEnqueued) > 0);
        }
    })
    .await;
}

fn op_clean_7(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move {
        h.github.set_status(REPO, 7, "CLEAN", false);
        "clean".into()
    })
}

fn setup_blocked_wait(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move {
        let (h, ctx) = setup_waiting_merge(queue).await;
        h.github.set_status(REPO, 7, "BLOCKED", false);
        (h, ctx)
    })
}

fn r1_stale(queue: bool) -> Table {
    let mutation = if queue { "enqueuePullRequest" } else { "mergePullRequest" };
    Table {
        name: if queue { "R1 stale wf.merge read vs advance (queue)" } else { "R1 stale wf.merge read vs advance" },
        setup: setup_blocked_wait,
        steps: vec![
            Step { name: "wf.merge", at: At::After("read", 1), run: op_merge_t1_7 },
            Step { name: "clean", at: At::Start, run: op_clean_7 },
            Step { name: "reconcile", at: At::Before(mutation, 1), run: op_reconcile },
        ],
        queue,
        serial: true,
    }
}

#[tokio::test(start_paused = true)]
async fn r1_a_stale_wf_merge_read_never_overwrites_advance() {
    local(async {
        for queue in [false, true] {
            let runs = table_holds(r1_stale(queue)).await;
            assert!(runs.saw(0, "waiting") || runs.saw(0, "queued") || runs.saw(0, "merged"));
        }
    })
    .await;
}

fn op_land_queue(h: Rc<H>, _: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move {
        match h.github.advance_queue(REPO, T0) {
            Some(n) => format!("landed #{n}"),
            None => "empty".into(),
        }
    })
}

fn setup_ready_queue(queue: bool) -> LocalBoxFuture<'static, (H, Ctx)> {
    Box::pin(async move { ready_card(queue).await })
}

fn r1_landing(wait: bool) -> Table {
    Table {
        name: if wait {
            "R1 wf.merge enqueue reply arrives after the queue lands"
        } else {
            "R1 wf.merge enqueue reply arrives after the queue lands (no wait)"
        },
        setup: if wait { setup_waiting_merge } else { setup_ready_queue },
        steps: vec![
            Step { name: "wf.merge", at: At::After("enqueuePullRequest", 1), run: op_merge_t1_7 },
            Step { name: "land", at: At::Now, run: op_land_queue },
            Step { name: "reconcile", at: At::Now, run: op_reconcile },
        ],
        queue: true,
        serial: false,
    }
}

#[tokio::test(start_paused = true)]
async fn r1_a_late_enqueue_reply_never_resurrects_a_retired_wait() {
    local(async {
        for wait in [true, false] {
            let runs = table_holds(r1_landing(wait)).await;
            assert!(runs.saw(1, "landed #7"));
        }
    })
    .await;
}

fn r2(queue: bool) -> Table {
    let mutation = if queue { "enqueuePullRequest" } else { "mergePullRequest" };
    Table {
        name: if queue { "R2 advance vs answer (queue)" } else { "R2 advance vs answer" },
        setup: setup_ready_and_wait,
        steps: vec![
            Step { name: "reconcile", at: At::Before(mutation, 1), run: op_reconcile },
            Step { name: "answer", at: At::Now, run: op_answer_merge },
            Step { name: "dismiss_at", at: At::Now, run: op_dismiss_at },
        ],
        queue,
        serial: true,
    }
}

#[tokio::test(start_paused = true)]
async fn r2_advance_races_answer_and_dismiss_in_every_order() {
    local(async {
        for queue in [false, true] {
            let runs = table_holds(r2(queue)).await;
            assert!(runs.reached(ReachPoint::DismissAtResolved) > 0);
            assert!(runs.saw(1, "answered") && runs.saw(2, "dismissed"));
            assert!(runs.reached(ReachPoint::AdvanceMerged) + runs.reached(ReachPoint::AdvanceEnqueued) > 0);
        }
    })
    .await;
}

fn r3(queue: bool) -> Table {
    Table {
        name: if queue { "R3 direct vs reconcile ready (queue)" } else { "R3 direct vs reconcile ready" },
        setup: setup_ready_waiting,
        steps: vec![
            Step { name: "ready", at: At::After("read", 1), run: op_ready_w1_7 },
            Step { name: "reconcile", at: At::After("read", 2), run: op_reconcile },
            Step { name: "push", at: At::Now, run: op_push_7 },
        ],
        queue,
        serial: true,
    }
}

#[tokio::test(start_paused = true)]
async fn r3_direct_and_reconcile_ready_race_in_every_order() {
    local(async {
        for queue in [false, true] {
            let runs = table_holds(r3(queue)).await;
            assert!(runs.reached(ReachPoint::ReadyWaitingRetried) > 0);
        }
    })
    .await;
}

fn r4(queue: bool) -> Table {
    Table {
        name: if queue { "R4 tracked snapshot (queue)" } else { "R4 tracked snapshot" },
        setup: setup_ready_card,
        steps: vec![
            Step { name: "reconcile", at: At::After("reconcile", 1), run: op_reconcile },
            Step { name: "adopt", at: At::Now, run: op_adopt },
            Step { name: "ready #8", at: At::After("read", 1), run: op_ready_w2_8 },
            Step { name: "answer", at: At::Now, run: op_answer_merge },
        ],
        queue,
        serial: true,
    }
}

#[tokio::test(start_paused = true)]
async fn r4_reconcile_tracked_snapshot_races_adopt_ready_and_answer() {
    local(async {
        for queue in [false, true] {
            let runs = table_holds(r4(queue)).await;
            assert!(runs.reached(ReachPoint::ReconcileDroppedUntracked) > 0);
        }
    })
    .await;
}

#[derive(Clone, Copy, Debug)]
enum Burst {
    Merge,
    Reconcile,
    Ready7,
    Ready8,
    Answer,
    Dismiss,
    Adopt,
    Push,
}

const ALPHABET: &[Burst] = &[
    Burst::Merge,
    Burst::Reconcile,
    Burst::Ready7,
    Burst::Ready8,
    Burst::Answer,
    Burst::Dismiss,
    Burst::Adopt,
    Burst::Push,
];

impl Burst {
    fn run(self) -> Run {
        match self {
            Burst::Merge => op_merge_t1_7,
            Burst::Reconcile => op_reconcile,
            Burst::Ready7 => op_ready_w1_7,
            Burst::Ready8 => op_ready_w2_8,
            Burst::Answer => op_answer_merge,
            Burst::Dismiss => op_dismiss_ready,
            Burst::Adopt => op_adopt,
            Burst::Push => op_push_7,
        }
    }
}

fn op_dismiss_ready(h: Rc<H>, c: Rc<Ctx>) -> LocalBoxFuture<'static, String> {
    Box::pin(async move { status(h.app.wf.dismiss(&c["ready"]).await) })
}

async fn burst_run(ops: &[Burst], seed: u64) -> Outcome {
    let (h, ctx) = setup_ready_card(seed % 2 == 1).await;
    let (h, ctx) = (Rc::new(h), Rc::new(ctx));
    h.github.set_jitter(Some(seed));
    h.fake.set_jitter(Some(seed));
    h.github.set_delay(Duration::from_millis(seed % 7));
    let handles: Vec<_> = ops.iter().map(|o| tokio::task::spawn_local((o.run())(h.clone(), ctx.clone()))).collect();
    let mut statuses = Vec::new();
    for handle in handles {
        statuses.push(settle(&format!("burst seed {seed} {ops:?}"), handle).await);
    }
    h.github.set_jitter(None);
    h.fake.set_jitter(None);
    h.github.set_delay(Duration::ZERO);
    quiesce(&h).await;
    check_safety(&h, &format!("burst {ops:?} seed {seed}"));
    Outcome { statuses, state: state(&h) }
}

#[tokio::test(start_paused = true)]
async fn bounded_bursts_quiesce_safely_and_match_a_serial_order() {
    local(async {
        for seed in 0..BURST_SEEDS {
            let mut rng = Rng::new(seed);
            let k = 2 + rng.pick(4);
            let ops: Vec<Burst> = (0..k).map(|_| ALPHABET[rng.pick(ALPHABET.len())]).collect();
            let got = burst_run(&ops, seed).await;
            let serial = |order: Vec<usize>| -> LocalBoxFuture<'_, Outcome> {
                let ops = ops.clone();
                Box::pin(async move { burst_run_serial(&ops, seed, &order).await })
            };
            let name = format!("burst seed {seed} {ops:?}");
            assert_serializable(&name, &[], &got, k, &serial, &mut BTreeMap::new()).await;
        }
    })
    .await;
}

async fn burst_run_serial(ops: &[Burst], seed: u64, order: &[usize]) -> Outcome {
    let (h, ctx) = setup_ready_card(seed % 2 == 1).await;
    let (h, ctx) = (Rc::new(h), Rc::new(ctx));
    let mut statuses = vec![String::new(); ops.len()];
    for &i in order {
        statuses[i] = (ops[i].run())(h.clone(), ctx.clone()).await;
    }
    quiesce(&h).await;
    check_safety(&h, &format!("burst {ops:?} serial order {order:?}"));
    Outcome { statuses, state: state(&h) }
}

#[test]
fn every_pr_seen_write_moves_its_revision_even_within_one_second() {
    let h = H::new_memory();
    let seen = t3_codemode::store::PrSeen {
        repo: REPO.into(),
        pr: 7,
        state: "CLOSED".into(),
        at: T0.into(),
        ..Default::default()
    };
    let rev = || h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().rev;
    h.app.store.pr_seen_put(&seen).unwrap();
    let first = rev();
    h.app.store.pr_seen_put(&seen).unwrap();
    assert_ne!(rev(), first, "a same-second rewrite of the same terminal row must change its revision");
}
