mod common;

use common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::clock::local_clock;
use t3_codemode::config::Config;
use t3_codemode::fake::{disabled_provider, project, provider, thread};
use t3_codemode::store::Limit;

const OPUS: &str = "claude-opus-5-5";

fn config(strategy: &str, members: Value, board_pool: bool) -> Config {
    let mut c = json!({"pools": {"opus": {"driver": "claudeAgent", "model": OPUS, "effort": "high",
                                          "strategy": strategy, "members": members}}});
    if board_pool {
        c["board"] = json!({"pool": "opus"});
    }
    Config::from_value(c).unwrap()
}

fn pooled(strategy: &str, members: Value) -> H {
    let h = H::build(Vec::new(), config(strategy, members, true));
    h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &[OPUS, "claude-haiku-5-5"]),
        provider("claude-b", "claudeAgent", &[OPUS]),
        provider("claude-c", "claudeAgent", &["claude-haiku-5-5"]),
        disabled_provider("claude-off", "claudeAgent"),
        provider("codex", "codex", &["gpt-6-astra"]),
    ]);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h
}

fn two(wa: u32, wb: u32) -> Value {
    json!([{"instanceId": "claude-a", "weight": wa}, {"instanceId": "claude-b", "weight": wb}])
}

async fn launch(h: &H, n: u64) -> Value {
    let out = h.exec(&format!("async () => wf.launchTicket({{ issue: 'acme/widgets#{n}' }})")).await;
    result(&out)
}

async fn launches(h: &H, from: u64, count: u64) -> Vec<String> {
    let mut out = Vec::new();
    for n in from..from + count {
        out.push(launch(h, n).await["instanceId"].as_str().unwrap().to_string());
    }
    out
}

fn tally(picks: &[String], id: &str) -> usize {
    picks.iter().filter(|p| *p == id).count()
}

fn limit(h: &H, id: &str, until: &str) {
    h.app
        .store
        .limit_set(id, &Limit { until: until.into(), reason: Some("usage limit".into()), at: T0.into() })
        .unwrap();
}

async fn pick(h: &H) -> Value {
    result(&h.exec("async () => wf.pick({ pool: 'opus' })").await)
}

#[tokio::test]
async fn weighted_one_to_one_splits_evenly_and_repeats_from_the_same_state() {
    let first = pooled("weighted", two(1, 1));
    let a = launches(&first, 1, 8).await;
    assert_eq!((tally(&a, "claude-a"), tally(&a, "claude-b")), (4, 4), "{a:?}");
    let second = pooled("weighted", two(1, 1));
    assert_eq!(launches(&second, 1, 8).await, a);
    let l = &first.fake.launched()[0];
    assert_eq!(
        l["modelSelection"],
        json!({"instanceId": a[0], "model": OPUS, "options": [{"id": "effort", "value": "high"}]})
    );
}

#[tokio::test]
async fn weights_three_to_one_lean_toward_the_heavier_member() {
    let h = pooled("weighted", two(1, 3));
    let picks = launches(&h, 1, 8).await;
    assert_eq!((tally(&picks, "claude-a"), tally(&picks, "claude-b")), (2, 6), "{picks:?}");
}

#[tokio::test]
async fn priority_fills_the_first_member_until_it_is_out() {
    let h = pooled("priority", two(1, 1));
    assert_eq!(launches(&h, 1, 3).await, ["claude-a"; 3]);
    limit(&h, "claude-a", "2026-11-17T21:00:00Z");
    let r = launch(&h, 4).await;
    assert_eq!(r["instanceId"], "claude-b");
    let reason = r["reason"].as_str().unwrap();
    assert!(reason.starts_with("priority"), "{reason}");
    assert!(reason.contains(&format!("claude-a limited until {}", local_clock("2026-11-17T21:00:00Z"))), "{reason}");
}

#[tokio::test]
async fn a_priority_pick_reads_no_thread_counts() {
    let h = pooled("priority", two(1, 1));
    assert_eq!(pick(&h).await["instanceId"], "claude-a");
    assert!(!h.fake.calls().iter().any(|c| c == "t3_thread_list"), "{:?}", h.fake.calls());
}

#[tokio::test]
async fn a_limited_member_is_skipped_and_named_with_its_reset_time() {
    let h = pooled("weighted", two(1, 1));
    limit(&h, "claude-a", "2026-11-17T21:00:00Z");
    let picks = launches(&h, 1, 4).await;
    assert_eq!(picks, ["claude-b"; 4]);
    let r = pick(&h).await;
    let reason = r["reason"].as_str().unwrap();
    let until = local_clock("2026-11-17T21:00:00Z");
    assert!(reason.contains(&format!("claude-a limited until {until}")), "{reason}");
    assert_eq!(r["skipped"], json!([{"instanceId": "claude-a", "reason": format!("limited until {until}")}]));
    h.clock.set("2026-11-17T21:00:01Z");
    assert_eq!(pick(&h).await["instanceId"], "claude-a");
}

#[tokio::test]
async fn every_member_out_fails_with_an_error_card_and_launches_nothing() {
    let h = pooled(
        "weighted",
        json!([{"instanceId": "claude-a"}, {"instanceId": "claude-off"}, {"instanceId": "claude-c"}]),
    );
    limit(&h, "claude-a", "2026-11-17T21:00:00Z");
    let out = h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#1' })").await;
    let e = err_text(&out);
    assert!(e.contains("pool opus has no member that can run"), "{e}");
    assert!(e.contains("claude-off disabled"), "{e}");
    assert!(e.contains(&format!("claude-c does not list model {OPUS}")), "{e}");
    let cards = h.app.wf.cards().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, "error");
    assert!(cards[0].text.contains("pool opus"), "{}", cards[0].text);
    assert!(h.fake.launched().is_empty());
    assert!(h.app.store.pending_all().unwrap().is_empty());
    let ask = h.app.wf.ask("Anything waiting?").await.unwrap_err();
    assert!(format!("{ask:#}").contains("pool opus has no member that can run"), "{ask:#}");
    assert_eq!(h.app.wf.cards().unwrap().len(), 2);
    assert!(h.fake.launched().is_empty());
    let direct = h.exec("async () => wf.pick({ pool: 'opus' })").await;
    assert!(err_text(&direct).contains("pool opus has no member that can run"), "{}", direct.text);
}

#[tokio::test]
async fn a_member_missing_the_model_is_skipped_with_that_reason() {
    let h = pooled("weighted", json!([{"instanceId": "claude-c"}, {"instanceId": "claude-b"}]));
    let r = pick(&h).await;
    assert_eq!(r["instanceId"], "claude-b");
    assert_eq!(r["skipped"], json!([{"instanceId": "claude-c", "reason": format!("does not list model {OPUS}")}]));
}

#[tokio::test]
async fn a_member_without_the_pools_effort_is_skipped() {
    let h = pooled("weighted", two(1, 1));
    let mut a = provider("claude-a", "claudeAgent", &[OPUS]);
    a["models"][0]["options"][0]["values"] = json!(["low", "medium"]);
    h.fake.set_providers(vec![a, provider("claude-b", "claudeAgent", &[OPUS])]);
    let r = pick(&h).await;
    assert_eq!(r["instanceId"], "claude-b");
    assert_eq!(
        r["skipped"],
        json!([{"instanceId": "claude-a", "reason": format!("does not offer effort high for {OPUS}")}])
    );
    let mut a = provider("claude-a", "claudeAgent", &[OPUS]);
    a["models"][0]["options"] = json!([{"id": "effort"}]);
    h.fake.set_providers(vec![a, provider("claude-b", "claudeAgent", &[OPUS])]);
    assert_eq!(pick(&h).await["instanceId"], "claude-b");
}

#[tokio::test]
async fn a_reserved_member_is_a_last_resort_while_no_usage_source_exists() {
    let h =
        pooled("weighted", json!([{"instanceId": "claude-a", "weight": 5, "reserve": 30}, {"instanceId": "claude-b"}]));
    let picks = launches(&h, 1, 4).await;
    assert_eq!(picks, ["claude-b"; 4]);
    let r = pick(&h).await;
    assert!(r["reason"].as_str().unwrap().contains("claude-a held at its 30% reserve"), "{r}");
    limit(&h, "claude-b", "2026-11-17T21:00:00Z");
    let r = launch(&h, 5).await;
    assert_eq!(r["instanceId"], "claude-a");
    let reason = r["reason"].as_str().unwrap();
    assert!(reason.starts_with("last resort: claude-a holds a 30% reserve"), "{reason}");
    assert!(reason.contains("claude-b limited until"), "{reason}");
}

#[tokio::test]
async fn a_launch_naming_a_reserved_instance_directly_still_goes_through() {
    let h = pooled("weighted", json!([{"instanceId": "claude-a", "reserve": 30}, {"instanceId": "claude-b"}]));
    let out = h
        .exec(
            "async () => t3.call('t3_thread_launch', { projectId: 'p-widgets', title: 'direct', message: 'go',
               modelSelection: { instanceId: 'claude-a', model: 'claude-opus-5-5' } })",
        )
        .await;
    assert_eq!(result(&out)["modelSelection"]["instanceId"], "claude-a");
    assert_eq!(h.fake.launched().len(), 1);
}

#[tokio::test]
async fn wf_pick_returns_a_model_selection_and_records_the_pick() {
    let h = pooled("weighted", two(1, 1));
    let r = pick(&h).await;
    assert_eq!(r["pool"], "opus");
    assert_eq!(r["model"], OPUS);
    assert_eq!(r["options"], json!([{"id": "effort", "value": "high"}]));
    assert_eq!(r["modelSelection"], json!({"instanceId": r["instanceId"], "model": OPUS, "options": r["options"]}));
    assert!(r["reason"].as_str().unwrap().starts_with("weighted: weight 1 of 2"), "{r}");
    let launched = h
        .exec(
            "async () => { const p = await wf.pick({ pool: 'opus' });
               return t3.call('t3_thread_launch', { projectId: 'p-widgets', title: 't', message: 'm', modelSelection: p.modelSelection }); }",
        )
        .await;
    assert_eq!(result(&launched)["modelSelection"]["model"], OPUS);
    let pools = result(&h.exec("async () => wf.pools()").await);
    let members = pools[0]["members"].as_array().unwrap();
    assert_eq!(members.iter().map(|m| m["picks"].as_u64().unwrap()).sum::<u64>(), 2);
    let unknown = h.exec("async () => wf.pick({ pool: 'nope' })").await;
    assert!(err_text(&unknown).contains("no pool \"nope\"; configured pools: opus"), "{}", unknown.text);
}

#[tokio::test]
async fn wf_pools_reports_state_reset_time_and_running_count() {
    let h = pooled(
        "weighted",
        json!([{"instanceId": "claude-a"}, {"instanceId": "claude-b"}, {"instanceId": "claude-off"}, {"instanceId": "claude-c"}]),
    );
    launch(&h, 1).await;
    limit(&h, "claude-b", "2026-11-17T21:00:00Z");
    let pools = result(&h.exec("async () => wf.pools()").await);
    let p = &pools[0];
    assert_eq!(p["pool"], "opus");
    assert_eq!(p["strategy"], "weighted");
    assert_eq!(p["error"], Value::Null);
    let m: Vec<&Value> = p["members"].as_array().unwrap().iter().collect();
    assert_eq!(m[0]["state"], "ready");
    assert_eq!(m[0]["running"], 1);
    assert_eq!(m[0]["picks"], 1);
    assert_eq!(m[1]["state"], "limited");
    assert_eq!(m[1]["limitedUntil"], "2026-11-17T21:00:00Z");
    assert_eq!(m[1]["limitedUntilLocal"], local_clock("2026-11-17T21:00:00Z"));
    assert_eq!(m[1]["running"], 0);
    assert_eq!(m[2]["state"], "disabled");
    assert_eq!(m[3]["state"], "missing-model");
}

#[tokio::test]
async fn running_counts_read_every_page_of_threads() {
    let h = pooled("weighted", two(1, 1));
    for n in 0..2001 {
        let mut t = thread(&format!("busy:{n:04}"), "running", T0);
        t["providerInstanceId"] = json!("claude-b");
        h.fake.put(t);
    }
    let pools = result(&h.exec("async () => wf.pools()").await);
    assert_eq!(pools[0]["members"][1]["running"], 2001);
}

#[tokio::test]
async fn fewer_running_threads_break_a_weighted_tie() {
    let h = pooled("weighted", two(1, 1));
    let mut t = thread("busy:a", "running", T0);
    t["providerInstanceId"] = json!("claude-a");
    h.fake.put(t);
    assert_eq!(pick(&h).await["instanceId"], "claude-b");
}

#[tokio::test]
async fn the_feed_shows_which_instance_a_pool_launch_went_to() {
    let h = pooled("weighted", two(1, 1));
    let r = launch(&h, 41).await;
    let feed = h.app.wf.feed(10).unwrap();
    let entry = feed.iter().find(|f| f["instanceId"].is_string()).expect("a pool launch entry");
    assert_eq!(entry["instanceId"], r["instanceId"]);
    assert_eq!(entry["reason"], r["reason"]);
    assert_eq!(entry["label"], BOARD);
    assert!(
        entry["head"]
            .as_str()
            .unwrap()
            .starts_with(&format!("widgets#41 launched on {}", r["instanceId"].as_str().unwrap()))
    );
    let a = h.app.wf.ask("Which tickets wait on me?").await.unwrap();
    assert!(a["instanceId"].is_string(), "{a}");
    assert_eq!(h.fake.launched()[1]["modelSelection"]["model"], OPUS);
}

#[tokio::test]
async fn a_pool_launch_refuses_a_different_model() {
    let h = pooled("weighted", two(1, 1));
    let out = h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#1', model: 'claude-haiku-5-5' })").await;
    assert!(err_text(&out).contains("board.pool opus runs claude-opus-5-5"), "{}", out.text);
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn limits_credits_and_pick_history_survive_a_restart() {
    let mut steady = pooled("weighted", two(1, 3));
    let all = launches(&steady, 1, 6).await;
    let mut h = pooled("weighted", two(1, 3));
    let before = launches(&h, 1, 3).await;
    limit(&h, "claude-a", "2026-11-17T21:00:00Z");
    h.reopen();
    let limits = h.app.store.limits().unwrap();
    assert_eq!(limits["claude-a"].until, "2026-11-17T21:00:00Z");
    assert_eq!(h.app.store.picks("opus", 10).unwrap().len(), 3);
    h.app.store.limit_clear("claude-a").unwrap();
    let after = launches(&h, 4, 3).await;
    assert_eq!([before, after].concat(), all);
    steady.reopen();
    assert_eq!(steady.app.store.picks("opus", 100).unwrap().len(), 6);
}

#[tokio::test]
async fn no_pool_set_launches_exactly_as_before() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    let r = launch(&h, 1).await;
    assert!(r.get("instanceId").is_none() && r.get("reason").is_none(), "{r}");
    let l = &h.fake.launched()[0];
    assert_eq!(
        l["modelSelection"],
        json!({"instanceId": "claudeAgent", "model": OPUS, "options": [{"id": "effort", "value": "high"}]})
    );
    assert!(!h.fake.calls().iter().any(|c| c == "orchestrator_capabilities"));
    assert!(h.app.wf.feed(10).unwrap().is_empty());
}

#[tokio::test]
async fn a_member_of_another_driver_or_unknown_instance_is_rejected() {
    let h = pooled("weighted", json!([{"instanceId": "claude-a"}, {"instanceId": "codex"}]));
    let out = h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#1' })").await;
    assert!(err_text(&out).contains("pool opus: member codex is a codex instance, not claudeAgent"), "{}", out.text);
    assert!(h.fake.launched().is_empty());
    assert!(h.app.wf.check_pools().await.is_err());
    let h = pooled("weighted", json!([{"instanceId": "claude-a"}, {"instanceId": "claude-z"}]));
    let e = format!("{:#}", h.app.wf.check_pools().await.unwrap_err());
    assert!(e.contains("pool opus: member claude-z is not a T3 provider instance"), "{e}");
    let pools = result(&h.exec("async () => wf.pools()").await);
    assert!(pools[0]["error"].as_str().unwrap().contains("claude-z"), "{pools}");
}

#[test]
fn config_load_rejects_bad_pools_by_name() {
    let bad = |pool: Value| {
        let e = Config::from_value(json!({"pools": {"opus": pool}})).unwrap_err();
        format!("{e:#}")
    };
    let base = || json!({"driver": "claudeAgent", "model": OPUS, "members": [{"instanceId": "claude-a"}]});
    let mut p = base();
    p["members"] = json!([]);
    assert!(bad(p).contains("pool opus: a pool needs at least one member"));
    let mut p = base();
    p["members"][0]["reserve"] = json!(101);
    assert!(bad(p).contains("pool opus: member claude-a: reserve must be between 0 and 100"));
    let mut p = base();
    p["members"][0]["reserve"] = json!(-1);
    assert!(bad(p).contains("reserve must be between 0 and 100"));
    let mut p = base();
    p["strategy"] = json!("headroom");
    assert!(bad(p).contains("pool opus: strategy headroom needs a usage source"));
    let mut p = base();
    p["members"] = json!([{"instanceId": "claude-a"}, {"instanceId": "claude-a"}]);
    assert!(bad(p).contains("member claude-a is listed twice"));
    let mut p = base();
    p["members"][0]["wieght"] = json!(2);
    let e = bad(p);
    assert!(e.starts_with("pool opus: unknown field `wieght`"), "{e}");
    let e = Config::from_value(json!({"board": {"pool": "missing"}})).unwrap_err();
    assert!(format!("{e:#}").contains("board.pool names \"missing\""));
    let ok = Config::from_value(json!({"pools": {"opus": base()}, "board": {"pool": "opus"}})).unwrap();
    assert_eq!(ok.pools["opus"].strategy, "weighted");
    assert_eq!(ok.pools["opus"].members[0].weight, 1);
    assert!(Config::default().pools.is_empty() && Config::default().board.pool.is_none());
}
