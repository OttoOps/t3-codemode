use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use t3_codemode::fake::{Failure, Rule, SeqIds};
use t3_codemode::reach::ReachSnapshot;
use t3_codemode::store::StoreCrash;

use super::races::{board_in, safety_faults, state};
use super::{REPO, TICKETS, WORKERS};
use crate::common::*;

const SETTLE_PASSES: usize = 8;

type Step = fn(&H) -> LocalBoxFuture<'_, String>;

struct Scenario {
    name: &'static str,
    setup: fn(&H) -> LocalBoxFuture<'_, ()>,
    steps: &'static [(&'static str, Step)],
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Inject {
    None,
    T3Fail(usize),
    T3FailAfterEffect(usize),
    GitHubFail(usize),
    GitHubFailAfterEffect(usize),
    Crash(usize),
}

pub(super) struct Run {
    state: Vec<String>,
    wakes: BTreeSet<Wake>,
    statuses: Vec<String>,
    reach: ReachSnapshot,
    pub(super) writes: usize,
    pub(super) t3_calls: usize,
    pub(super) gh_calls: usize,
    crashed_in: Option<&'static str>,
    landed: bool,
    broke: Vec<String>,
}

pub(super) type Count = fn(&Run) -> usize;
pub(super) type Make = fn(usize) -> Inject;

const KNOWN: &[(&str, &str)] = &[("answer merge", "#365")];

const FALLBACK_TO: &str = "mcp:t1";

const FALLBACK_WAKES: &[(&str, &str)] = &[("event", "t3cm"), ("gate", "mcp:w1"), ("reply", "board")];

const HELD_REPLY: (&str, &str, &str, &str) = ("mcp:w1", "reply", "mcp:t1", "Merged #7 at abc1234. Hand off.");

type Wake = (String, String, String, String);

fn core(h: &H) -> Vec<String> {
    let feed = h.app.wf.feed(200).unwrap();
    let mut feed: Vec<String> = feed
        .iter()
        .map(|m| {
            let head = m["head"].as_str().unwrap_or_default();
            format!("{} {} {}", m["kind"], m["ticket"], head.split(':').next().unwrap_or_default())
        })
        .collect();
    feed.sort();
    let live: BTreeMap<(String, u64), SeenView> = h
        .app
        .store
        .pr_seen_all()
        .unwrap()
        .into_iter()
        .filter(|r| r.state == "OPEN")
        .map(|r| ((r.repo, r.pr), (r.queued, r.since, r.left, r.queued_at)))
        .collect();
    let mut out: Vec<String> = state(h)
        .into_iter()
        .filter(|l| !l.starts_with("delivered ") && !l.starts_with("feed ") && !l.starts_with("seen "))
        .collect();
    out.push(format!("live seen {live:?}"));
    out.push(format!("feed {feed:?}"));
    out
}

fn wakes(h: &H) -> BTreeSet<Wake> {
    let mut out = BTreeSet::new();
    for s in h.sent() {
        let to = s["threadId"].as_str().unwrap_or_default();
        for line in s["message"].as_str().unwrap_or_default().lines() {
            let Some(rest) = line.strip_prefix("[t3cm ") else { continue };
            let Some((head, handle)) = rest.rsplit_once(" msg=") else { continue };
            let handle = handle.split([' ', ']']).next().unwrap_or_default();
            let Some(m) = h.app.store.msg_get(handle).unwrap() else { continue };
            let kind = head.split(' ').next().unwrap_or_default();
            let body = gist(m.body.lines().next().unwrap_or_default());
            out.insert((to.to_string(), kind.to_string(), m.from, body));
        }
    }
    out
}

fn gist(text: &str) -> String {
    let head = text.split(": http").next().unwrap_or_default();
    head.split(' ')
        .map(|w| {
            let w0 = w.trim_end_matches(':');
            let card = w0.len() == 2 && w0.starts_with(['a', 'b', 'c', 'd']) && w0.as_bytes()[1].is_ascii_digit();
            if card { "<card>" } else { w }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn closings(h: &H, seen: &mut BTreeMap<String, usize>, sent: usize) {
    for ln in h.app.store.links_all().unwrap().into_iter().filter(|l| l.closed) {
        seen.entry(ln.child).or_insert(sent);
    }
}

fn woken_after_close(h: &H, closed: &BTreeMap<String, usize>, what: &str) -> Vec<String> {
    let sent = h.sent();
    let mut out = Vec::new();
    for (child, &at) in closed {
        let late: Vec<&str> = sent[at.min(sent.len())..]
            .iter()
            .filter(|s| s["threadId"] == child.as_str())
            .filter_map(|s| s["message"].as_str())
            .collect();
        if !late.is_empty() {
            out.push(format!("{what}: {child} was woken after its link closed: {late:?}"));
        }
    }
    out
}

fn status(r: anyhow::Result<Value>) -> String {
    match r {
        Ok(v) => {
            let said = match v["status"].as_str().or(v["delivery"].as_str()).unwrap_or("ok") {
                "ticket" => format!("ticket: {}", v["why"].as_str().unwrap_or_default()),
                s => s.to_string(),
            };
            match v["merge"]["status"].as_str() {
                Some(merge) => format!("{said} {merge}"),
                None => said,
            }
        }
        Err(e) => format!("error: {e:#}"),
    }
}

fn ready_card(h: &H, pr: u64) -> Option<t3_codemode::board::Card> {
    h.app.wf.cards().unwrap().into_iter().find(|c| c.kind == "gate" && c.pr.as_ref().is_some_and(|p| p.number == pr))
}

fn ticket_card(h: &H, gate: &str) -> Option<t3_codemode::board::Card> {
    h.app.wf.cards().unwrap().into_iter().find(|c| c.kind == "gate" && c.pr.is_none() && c.text.contains(gate))
}

fn ready_7(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let args = json!({"event": "ready", "repo": REPO, "pr": 7, "head": super::HEADS[0]});
        status(h.app.wf.dispatch("pr", Some(WORKERS[0]), args).await)
    })
}

fn reconcile(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        match h.app.wf.reconcile_ran().await {
            Ok(Some(a)) if std::env::var("SWEEP_TRACE").is_ok() => format!("ran {a:?}"),
            Ok(Some(_)) => "ran".into(),
            Ok(None) => "skipped".into(),
            Err(e) => format!("error: {e:#}"),
        }
    })
}

fn tick(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        match h.app.wf.tick().await {
            Ok(_) => "ticked".into(),
            Err(e) => format!("error: {e:#}"),
        }
    })
}

fn answer_ready(option: &'static str) -> impl Fn(&H) -> LocalBoxFuture<'_, String> {
    move |h| {
        Box::pin(async move {
            let Some(card) = ready_card(h, 7) else { return "no card".into() };
            let pick = card.options.iter().find(|o| o.as_str() == option).cloned();
            let pick = pick.unwrap_or_else(|| panic!("card {} offers no {option:?}: {:?}", card.id, card.options));
            status(h.app.wf.answer(&card.id, &pick).await)
        })
    }
}

fn merge_and_deploy(h: &H) -> LocalBoxFuture<'_, String> {
    answer_ready("merge and deploy")(h)
}

fn merge_alone(h: &H) -> LocalBoxFuture<'_, String> {
    answer_ready("merge")(h)
}

fn delivered_reply(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let args = json!({"to": WORKERS[0], "body": "Delivered: v0.1.0 https://ci.acme.example/run/1. Hand off."});
        status(h.app.wf.dispatch("reply", Some(TICKETS[0]), args).await)
    })
}

fn merged_reply(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let args = json!({"to": WORKERS[0], "body": "Merged #7 at abc1234. Hand off."});
        status(h.app.wf.dispatch("reply", Some(TICKETS[0]), args).await)
    })
}

fn handoff(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let args = json!({"kind": "handoff", "body": "## Handoff\n\nPR #7 merged and delivered."});
        status(h.app.wf.dispatch("send", Some(WORKERS[0]), args).await)
    })
}

fn verify_gate(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let body = "**ACTION NEEDED:** verify the deploy, here: https://ci.acme.example/run/1";
        let args =
            json!({"kind": "gate", "gate": "delivery:verify", "body": body, "options": ["verified", "roll back"]});
        status(h.app.wf.dispatch("send", Some(TICKETS[0]), args).await)
    })
}

fn answer_verify(h: &H) -> LocalBoxFuture<'_, String> {
    Box::pin(async move {
        let Some(card) = ticket_card(h, "verify the deploy") else { return "no card".into() };
        status(h.app.wf.answer(&card.id, "verified").await)
    })
}

fn no_setup(_: &H) -> LocalBoxFuture<'_, ()> {
    Box::pin(async {})
}

fn worker_busy(h: &H) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        h.fake.start_run(WORKERS[0], Some(30));
        h.app.store.meta_set(t3_codemode::wf::LAST_PASS, T0).unwrap();
    })
}

const APPROVE_MERGE_DELIVER: Scenario = Scenario {
    name: "approve-merge-deliver",
    setup: no_setup,
    steps: &[
        ("ready", ready_7),
        ("reconcile", reconcile),
        ("answer merge and deploy", merge_and_deploy),
        ("reply delivered", delivered_reply),
        ("tick", tick),
        ("handoff", handoff),
    ],
};

const READY_DELIVERY_GATE: Scenario = Scenario {
    name: "ready plus a delivery gate",
    setup: no_setup,
    steps: &[
        ("ready", ready_7),
        ("reconcile", reconcile),
        ("open delivery:verify", verify_gate),
        ("answer verified", answer_verify),
        ("answer merge", merge_alone),
        ("reconcile", reconcile),
    ],
};

const HANDOFF_HELD_REPLY: Scenario = Scenario {
    name: "handoff with a held reply",
    setup: worker_busy,
    steps: &[("reply merged", merged_reply), ("tick", tick), ("handoff", handoff), ("tick", tick)],
};

const SCENARIOS: &[&Scenario] = &[&APPROVE_MERGE_DELIVER, &READY_DELIVERY_GATE, &HANDOFF_HELD_REPLY];

async fn harness(sc: &Scenario) -> H {
    let h = board_in(false, false, Some("squash")).await;
    (sc.setup)(&h).await;
    h
}

fn arm(h: &H, inject: Inject) {
    let down = || Failure::down("injected outage");
    match inject {
        Inject::None => {}
        Inject::T3Fail(n) => h.fake.fault(Rule::on("*").call(n).fail(down())),
        Inject::T3FailAfterEffect(n) => h.fake.fault(Rule::on("*").call(n).fail_after_effect(down())),
        Inject::GitHubFail(n) => h.github.fault(Rule::on("*").call(n).fail(down())),
        Inject::GitHubFailAfterEffect(n) => {
            h.github.fault(Rule::on("*").call(n).fail_after_effect(Failure::refused("injected error after the write")))
        }
        Inject::Crash(n) => h.app.store.set_fault(Some(h.app.store.writes() + n)),
    }
}

fn restart(h: &mut H) {
    h.reopen();
    h.app.wf.set_id_source(Arc::new(SeqIds::new("appr-")));
}

async fn attempt(h: &mut H, step: Step, reach: &mut ReachSnapshot) -> Option<String> {
    match AssertUnwindSafe(step(h)).catch_unwind().await {
        Ok(out) => Some(out),
        Err(payload) if payload.downcast_ref::<StoreCrash>().is_some() => {
            reach.merge(&h.app.wf.reach().snapshot());
            restart(h);
            None
        }
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

async fn settle(h: &mut H, reach: &mut ReachSnapshot) -> Vec<String> {
    h.github.clear_faults();
    h.fake.clear_faults();
    h.fake.finish_run(WORKERS[0], "idle");
    let mut last = core(h);
    for pass in 0..SETTLE_PASSES {
        h.clock.set(&at(60 + pass as i64));
        attempt(h, reconcile, reach).await;
        attempt(h, tick, reach).await;
        let now = core(h);
        if now == last {
            return now;
        }
        last = now;
    }
    panic!("no fixpoint in {SETTLE_PASSES} passes:\n{}", last.join("\n"));
}

async fn drive(sc: &Scenario, inject: Inject) -> Run {
    let trace = std::env::var("SWEEP_TRACE").is_ok();
    let mut h = harness(sc).await;
    let (writes, t3, gh) = (h.app.store.writes(), h.fake.calls().len(), h.github.calls().len());
    arm(&h, inject);
    let mut reach = ReachSnapshot::default();
    let mut crashed_in = None;
    let mut statuses = Vec::new();
    let mut closed = BTreeMap::new();
    for (i, (name, step)) in sc.steps.iter().enumerate() {
        h.clock.set(&at(i as i64 + 1));
        let sent = h.sent().len();
        let out = attempt(&mut h, *step, &mut reach).await;
        if trace {
            eprintln!("{} {inject:?} step {i}: {out:?}", sc.name);
        }
        if out.is_none() {
            crashed_in = Some(*name);
        }
        statuses.push(out.clone().unwrap_or_else(|| "crashed".into()));
        if out.as_deref().is_none_or(|o| o.starts_with("error")) {
            h.github.clear_faults();
            h.fake.clear_faults();
            let again = attempt(&mut h, *step, &mut reach).await.expect("a retry after the restart crashed again");
            if trace {
                eprintln!("{} {inject:?} retry {i}: {again}", sc.name);
            }
            if out.is_some() {
                statuses.pop();
                statuses.push(again);
            }
        }
        closings(&h, &mut closed, sent);
    }
    let (writes, t3_calls, gh_calls) = match crashed_in {
        Some(_) => (0, 0, 0),
        None => (h.app.store.writes() - writes, h.fake.calls().len() - t3, h.github.calls().len() - gh),
    };
    let state = settle(&mut h, &mut reach).await;
    let what = format!("{} under {inject:?}", sc.name);
    let mut broke = safety_faults(&h, &what);
    broke.extend(woken_after_close(&h, &closed, &what));
    reach.merge(&h.app.wf.reach().snapshot());
    let landed = h.github.pr(REPO, 7)["state"] == "MERGED";
    Run { state, wakes: wakes(&h), statuses, reach, writes, t3_calls, gh_calls, crashed_in, landed, broke }
}

fn diff(got: &[String], want: &[String]) -> String {
    let mut out = String::new();
    for (g, w) in got.iter().zip(want) {
        if g != w {
            out.push_str(&format!("\n  want {w}\n  got  {g}"));
        }
    }
    out
}

fn fallback(status: &str) -> bool {
    status.starts_with("ticket: ")
        || status.starts_with("error: ")
        || status.ends_with(" refused")
        || status.ends_with(" error")
}

fn told(run: &Run, base: &Run) -> bool {
    run.statuses.iter().zip(&base.statuses).any(|(got, want)| got != want && fallback(got))
}

fn entries(line: &str) -> Vec<&str> {
    let Some(inner) = line.find(['{', '[']).map(|i| &line[i + 1..line.len() - 1]) else { return Vec::new() };
    let (mut out, mut depth, mut quoted, mut escaped, mut from) = (Vec::new(), 0, false, false, 0);
    for (i, c) in inner.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => depth += 1,
            ')' | ']' | '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(inner[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[from..].trim());
    out.retain(|e| !e.is_empty());
    out
}

fn without(line: &str, drop: impl Fn(&&str) -> bool) -> Vec<&str> {
    entries(line).into_iter().filter(|e| !drop(e)).collect()
}

fn approval_lost(got: &str, want: &str) -> bool {
    let merge_event = |e: &&str| e.starts_with(r#"("acme/widgets", 7, "merged"#);
    let open_seen = |e: &&str| e.starts_with(r#"("acme/widgets", 7): (false, "#) && e.ends_with(", None, None)");
    let merged = |e: &&str| *e == r#""\"event\" \"mcp:t1\" #41 PR 1/3 is merged""#;
    let open =
        r#""mcp:t1": LinkView { parent: "board", closed: false, paused: false, gate: None, gate_pr: None, pr: None }"#;
    let gated = |e: &str| {
        [r#"Some(("acme/widgets", 7))"#, "None"]
            .iter()
            .any(|g| e == open.replace("gate: None, gate_pr: None", &format!(r#"gate: Some("merge"), gate_pr: {g}"#)))
    };
    let prefix = |p: &str| got.starts_with(p) && want.starts_with(p);
    if prefix("events ") {
        return entries(got) == without(want, merge_event);
    }
    if prefix("live seen ") {
        return without(got, open_seen) == entries(want);
    }
    if prefix("feed ") {
        return entries(got) == without(want, merged);
    }
    if prefix("links ") {
        let (got, want) = (entries(got), entries(want));
        return got.len() == want.len() && got.iter().zip(&want).all(|(g, w)| g == w || (*w == open && gated(g)));
    }
    prefix("github #7 ") && got == want.replace(r#""MERGED""#, r#""OPEN""#)
}

fn known(run: &Run, base: &Run, inject: Inject) -> Option<&'static str> {
    if !matches!(inject, Inject::Crash(_)) || run.landed {
        return None;
    }
    let issue = KNOWN.iter().find(|(prefix, _)| run.crashed_in.is_some_and(|s| s.starts_with(prefix)))?.1;
    let lost: Vec<&Wake> = base.wakes.difference(&run.wakes).collect();
    let only_merge_wake = matches!(lost[..], [(to, kind, _, body)] if to == "mcp:t1" && kind == "event" && body == "#41 PR 1/3 is merged");
    let state = run.state.iter().zip(&base.state).all(|(g, w)| g == w || approval_lost(g, w));
    let broke =
        run.broke.iter().all(|b| b.ends_with("mcp:t1 points its merge gate at #7, which holds no card of its own"));
    (only_merge_wake && run.wakes.is_subset(&base.wakes) && state && broke).then_some(issue)
}

fn verdict(run: &Run, base: &Run, inject: Inject) -> Vec<String> {
    let strict = matches!(inject, Inject::Crash(_));
    let mut out: Vec<String> = run.broke.iter().map(|why| format!("{inject:?}: {why}")).collect();
    if run.state != base.state && (strict || !told(run, base)) {
        out.push(format!("{inject:?} {:?}:{}", run.statuses, diff(&run.state, &base.state)));
    }
    let handed_back = |w: &&Wake| {
        !strict && told(run, base) && w.0 == FALLBACK_TO && FALLBACK_WAKES.contains(&(w.1.as_str(), w.2.as_str()))
    };
    let fail_open = |w: &&Wake| !strict && (w.0.as_str(), w.1.as_str(), w.2.as_str(), w.3.as_str()) == HELD_REPLY;
    let lost: Vec<&Wake> = base.wakes.difference(&run.wakes).filter(|w| !handed_back(w)).collect();
    let extra: Vec<&Wake> = run.wakes.difference(&base.wakes).filter(|w| !fail_open(w) && !handed_back(w)).collect();
    if !lost.is_empty() || !extra.is_empty() {
        out.push(format!("{inject:?} {:?}: lost wakes {lost:?}, extra wakes {extra:?}", run.statuses));
    }
    out
}

struct Swept {
    runs: usize,
    known: Vec<(Inject, &'static str)>,
    failures: Vec<String>,
    reach: ReachSnapshot,
}

async fn sweep_all(sc: &Scenario, of: Count, inject: Make) -> Swept {
    let base = drive(sc, Inject::None).await;
    assert!(base.broke.is_empty(), "{}: the clean run broke a safety check: {:?}", sc.name, base.broke);
    let only: Option<usize> = std::env::var("SWEEP_K").ok().and_then(|v| v.parse().ok());
    let mut swept = Swept { runs: of(&base), known: Vec::new(), failures: Vec::new(), reach: ReachSnapshot::default() };
    for k in (1..=swept.runs).filter(|k| only.is_none_or(|o| o == *k)) {
        let run = drive(sc, inject(k)).await;
        swept.reach.merge(&run.reach);
        let failed = verdict(&run, &base, inject(k));
        match known(&run, &base, inject(k)) {
            Some(issue) if !failed.is_empty() => swept.known.push((inject(k), issue)),
            _ => swept.failures.extend(failed),
        }
    }
    swept
}

pub(super) async fn reach_of(of: Count, inject: Make) -> ReachSnapshot {
    let mut reach = ReachSnapshot::default();
    for sc in SCENARIOS {
        reach.merge(&sweep(sc, of, inject).await.reach);
    }
    reach
}

async fn sweep(sc: &Scenario, of: Count, inject: Make) -> Swept {
    let swept = sweep_all(sc, of, inject).await;
    assert!(
        swept.failures.is_empty(),
        "{}: {} of {} runs ended elsewhere:\n{}",
        sc.name,
        swept.failures.len(),
        swept.runs,
        swept.failures.join("\n")
    );
    swept
}

#[tokio::test(start_paused = true)]
async fn nth_t3_call_failures_end_where_a_clean_run_ends() {
    for sc in SCENARIOS {
        sweep(sc, |r| r.t3_calls, Inject::T3Fail).await;
    }
}

#[tokio::test(start_paused = true)]
async fn nth_t3_call_failures_after_their_effect_end_where_a_clean_run_ends() {
    for sc in SCENARIOS {
        sweep(sc, |r| r.t3_calls, Inject::T3FailAfterEffect).await;
    }
}

#[tokio::test(start_paused = true)]
async fn nth_github_call_failures_end_where_a_clean_run_ends() {
    let mut runs = 0;
    for sc in SCENARIOS {
        runs += sweep(sc, |r| r.gh_calls, Inject::GitHubFail).await.runs;
    }
    assert!(runs > 0);
}

#[tokio::test(start_paused = true)]
async fn nth_github_call_failures_after_their_effect_end_where_a_clean_run_ends() {
    for sc in SCENARIOS {
        sweep(sc, |r| r.gh_calls, Inject::GitHubFailAfterEffect).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_crash_at_any_store_write_then_reopen_ends_where_a_clean_run_ends() {
    for sc in SCENARIOS {
        let swept = sweep(sc, |r| r.writes, Inject::Crash).await;
        assert!(swept.runs > 10, "{}: only {} writes to crash at", sc.name, swept.runs);
        assert!(swept.known.len() < swept.runs / 4, "{}: known failures {:?}", sc.name, swept.known);
    }
}

#[tokio::test(start_paused = true)]
async fn the_clean_scenarios_reach_their_ends() {
    let ends = [
        ("approve-merge-deliver", "MERGED", true),
        ("ready plus a delivery gate", "MERGED", false),
        ("handoff with a held reply", "OPEN", true),
    ];
    for (sc, (name, pr, worker_closed)) in SCENARIOS.iter().zip(ends) {
        assert_eq!(sc.name, name);
        let mut h = harness(sc).await;
        let mut reach = ReachSnapshot::default();
        for (i, (_, step)) in sc.steps.iter().enumerate() {
            h.clock.set(&at(i as i64 + 1));
            attempt(&mut h, *step, &mut reach).await.expect("a clean step crashed");
        }
        settle(&mut h, &mut reach).await;
        assert_eq!(h.github.pr(REPO, 7)["state"], pr, "{name}");
        let w1 = h.app.store.link_get(WORKERS[0]).unwrap().unwrap();
        assert_eq!(w1.closed, worker_closed, "{name}: {w1:?}");
        assert!(h.app.store.outbox_all().unwrap().is_empty(), "{name}");
    }
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn survey() {
    let kinds: [(&str, Count, Make); 5] = [
        ("t3 fail", |r| r.t3_calls, Inject::T3Fail),
        ("t3 after", |r| r.t3_calls, Inject::T3FailAfterEffect),
        ("gh fail", |r| r.gh_calls, Inject::GitHubFail),
        ("gh after", |r| r.gh_calls, Inject::GitHubFailAfterEffect),
        ("crash", |r| r.writes, Inject::Crash),
    ];
    for (kind, of, inject) in kinds {
        for sc in SCENARIOS {
            let swept = sweep_all(sc, of, inject).await;
            eprintln!(
                "== {kind} / {}: {} failed, {} known of {}",
                sc.name,
                swept.failures.len(),
                swept.known.len(),
                swept.runs
            );
            for (inject, issue) in &swept.known {
                eprintln!("known {inject:?}: {issue}");
            }
            for f in swept.failures {
                eprintln!("{}", f.lines().take(12).map(|l| &l[..l.len().min(300)]).collect::<Vec<_>>().join("\n"));
            }
        }
    }
}
