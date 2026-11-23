use serde_json::{Value, json};
use t3_codemode::clock::local_clock;
use t3_codemode::config::Config;
use t3_codemode::fake::{disabled_provider, project, provider, thread};
use t3_codemode::server;
use t3_codemode::store::{Limit, PickRecord};

use crate::board::run_page_async_with;
use crate::common::*;

const OPUS: &str = "claude-opus-5-5";
const RESET: &str = "2026-11-17T21:00:00Z";

fn pooled(members: Value) -> H {
    let c = json!({"pools": {"opus": {"driver": "claudeAgent", "model": OPUS, "effort": "high",
                                      "members": members}},
                   "board": {"pool": "opus"}});
    let h = H::build(Vec::new(), Config::from_value(c).unwrap());
    h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &[OPUS]),
        provider("claude-b", "claudeAgent", &[OPUS]),
        disabled_provider("claude-off", "claudeAgent"),
    ]);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h
}

fn one_to_three() -> H {
    pooled(json!([{"instanceId": "claude-a", "weight": 1}, {"instanceId": "claude-b", "weight": 3}]))
}

async fn launch(h: &H, n: u64) {
    result(&h.exec(&format!("async () => wf.launchTicket({{ issue: 'acme/widgets#{n}' }})")).await);
}

fn limit(h: &H, id: &str) {
    let l = Limit { until: RESET.into(), reason: Some("usage limit".into()), at: T0.into(), thread: None };
    h.app.store.limit_set(id, &l).unwrap();
}

async fn stats(h: &H, input: &str) -> Value {
    result(&h.exec(&format!("async () => wf.usageStats({input})")).await)
}

fn member<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["members"].as_array().unwrap().iter().find(|m| m["instanceId"] == id).unwrap()
}

#[tokio::test]
async fn usage_stats_reports_each_members_share_state_and_pick_reasons() {
    let h = one_to_three();
    for n in 1..=4 {
        launch(&h, n).await;
    }
    limit(&h, "claude-a");
    launch(&h, 5).await;
    let v = stats(&h, "").await;
    assert_eq!(
        (v["mode"].clone(), v["pool"].clone(), v["source"].clone()),
        (json!("pool"), json!("opus"), json!("board"))
    );
    assert_eq!(
        (v["strategy"].clone(), v["window"].clone(), v["picks"].clone()),
        (json!("weighted"), json!("24h"), json!(5))
    );
    assert_eq!(v["shareBasis"], "picks");
    assert_eq!(v["since"], "2026-11-16T12:00:00Z");
    let a = member(&v, "claude-a");
    assert_eq!((a["status"].clone(), a["picks"].clone(), a["share"].clone()), (json!("limited"), json!(1), json!(0.2)));
    assert_eq!(a["limitedUntil"], RESET);
    assert_eq!(a["limitedUntilLocal"], local_clock(RESET));
    assert_eq!(a["skips"], json!({"limited": 1}));
    assert_eq!(a["reasons"], json!({"weighted": 1}));
    let b = member(&v, "claude-b");
    assert_eq!((b["status"].clone(), b["picks"].clone(), b["share"].clone()), (json!("ready"), json!(4), json!(0.8)));
    assert_eq!(b["weightShare"], json!(0.75));
    assert_eq!(b["running"], 4);
    assert_eq!(v["reasons"], json!({"weighted": 5}));
    assert_eq!(v["recent"].as_array().unwrap().len(), 5);
    assert_eq!(v["recent"][0]["instanceId"], "claude-b");
    let buckets = v["series"]["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 24);
    let total: u64 =
        buckets.iter().flat_map(|b| b["counts"].as_object().unwrap().values()).map(|n| n.as_u64().unwrap()).sum();
    assert_eq!(total, 5);
    assert_eq!(buckets[23]["counts"], json!({"claude-a": 1, "claude-b": 4}));
    assert_eq!(
        v["running"],
        json!([{"instanceId": "claude-a", "threads": 1}, {"instanceId": "claude-b", "threads": 4}])
    );
    assert_eq!(v["former"], json!([]));
}

#[tokio::test]
async fn usage_stats_counts_only_the_window_and_falls_back_to_weight() {
    let h = one_to_three();
    launch(&h, 1).await;
    launch(&h, 2).await;
    h.clock.set(&at(120));
    let recent = stats(&h, "{ window: '1h' }").await;
    assert_eq!((recent["picks"].clone(), recent["shareBasis"].clone()), (json!(0), json!("weight")));
    assert_eq!(member(&recent, "claude-a")["share"], json!(0.25));
    assert_eq!(member(&recent, "claude-b")["share"], json!(0.75));
    assert_eq!(recent["since"], at(60));
    let all = stats(&h, "{ window: 'all' }").await;
    assert_eq!((all["picks"].clone(), all["since"].clone()), (json!(2), Value::Null));
    assert_eq!(all["shareBasis"], "picks");
    for bad in ["5x", "0h", "h", "400d"] {
        let out = h.exec(&format!("async () => wf.usageStats({{ window: '{bad}' }})")).await;
        assert!(err_text(&out).contains("wf.usageStats: window"), "{bad}: {}", err_text(&out));
    }
}

#[tokio::test]
async fn picks_of_a_member_no_longer_in_the_pool_still_count_toward_shares() {
    let h = one_to_three();
    launch(&h, 1).await;
    for _ in 0..3 {
        let p = PickRecord {
            pool: "opus".into(),
            instance_id: "claude-gone".into(),
            model: OPUS.into(),
            strategy: "weighted".into(),
            reason: "weighted".into(),
            skipped: json!([]),
            at: T0.into(),
            handle: None,
            thread: None,
            moved_from: None,
        };
        h.app.store.pick_add(&p).unwrap();
    }
    let v = stats(&h, "").await;
    assert_eq!((v["picks"].clone(), v["shareBasis"].clone()), (json!(4), json!("picks")));
    assert_eq!(member(&v, "claude-b")["share"], json!(0.25));
    assert_eq!(member(&v, "claude-a")["share"], json!(0));
    assert_eq!(v["former"], json!([{"instanceId": "claude-gone", "picks": 3}]));
}

#[tokio::test]
async fn usage_stats_marks_a_reserve_member_and_a_member_that_cannot_run() {
    let h = pooled(json!([{"instanceId": "claude-a"}, {"instanceId": "claude-b", "reserve": 20},
                          {"instanceId": "claude-off"}]));
    let v = stats(&h, "").await;
    assert_eq!(member(&v, "claude-a")["status"], "ready");
    assert_eq!(member(&v, "claude-b")["status"], "reserve");
    assert_eq!(member(&v, "claude-off")["status"], "disabled");
    assert_eq!(member(&v, "claude-off")["reason"], "disabled");
    assert_eq!(member(&v, "claude-a")["share"], json!(0.333));
}

#[tokio::test]
async fn usage_stats_without_a_pool_reports_the_single_launch_selection() {
    let h = H::new();
    let mut t = thread("busy:1", "running", T0);
    t["providerInstanceId"] = json!("claudeAgent");
    h.fake.put(t);
    let v = stats(&h, "").await;
    assert_eq!(
        (v["mode"].clone(), v["pool"].clone(), v["source"].clone()),
        (json!("single"), Value::Null, json!("default"))
    );
    assert_eq!(v["error"], Value::Null);
    let m = &v["members"][0];
    assert_eq!(v["members"].as_array().unwrap().len(), 1);
    assert_eq!(
        (m["instanceId"].clone(), m["model"].clone(), m["effort"].clone()),
        (json!("claudeAgent"), json!(OPUS), json!("high"))
    );
    assert_eq!((m["status"].clone(), m["share"].clone(), m["running"].clone()), (json!("ready"), json!(1), json!(1)));
    assert_eq!(m["driver"], "claudeAgent");
    assert_eq!(v["running"], json!([{"instanceId": "claudeAgent", "threads": 1}]));
}

struct Board {
    h: H,
    base: String,
    http: reqwest::Client,
}

impl Board {
    async fn start(h: H) -> Board {
        let (listener, addr) = server::bind(0).await.unwrap();
        let app = h.app.clone();
        tokio::spawn(async move {
            let _ = server::serve(app, listener, std::future::pending()).await;
        });
        Board { h, base: format!("http://{addr}"), http: reqwest::Client::builder().no_proxy().build().unwrap() }
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    async fn page(&self) -> String {
        self.http.get(format!("{}/", self.base)).send().await.unwrap().text().await.unwrap()
    }
}

#[tokio::test]
async fn board_state_carries_routing_and_changes_only_when_it_moves() {
    let b = Board::start(one_to_three()).await;
    assert_eq!(b.get("/api/state").await.1["routing"], Value::Null);
    assert!(b.h.app.wf.refresh_routing().await.unwrap());
    let (_, st) = b.get("/api/state").await;
    assert_eq!(st["routing"]["mode"], "pool");
    assert_eq!(member(&st["routing"], "claude-b")["share"], json!(0.75));
    let rev = st["rev"].as_u64().unwrap();
    assert!(!b.h.app.wf.refresh_routing().await.unwrap());
    assert_eq!(b.get("/api/state").await.1["rev"].as_u64().unwrap(), rev);
    limit(&b.h, "claude-a");
    assert!(b.h.app.wf.refresh_routing().await.unwrap());
    let (_, st) = b.get("/api/state").await;
    assert!(st["rev"].as_u64().unwrap() > rev);
    assert_eq!(member(&st["routing"], "claude-a")["status"], "limited");
}

#[tokio::test]
async fn the_usage_stats_route_takes_a_window() {
    let b = Board::start(one_to_three()).await;
    launch(&b.h, 1).await;
    let (status, v) = b.get("/api/usage-stats?window=7d").await;
    assert_eq!((status, v["window"].clone(), v["picks"].clone()), (200, json!("7d"), json!(1)));
    assert_eq!(b.get("/api/usage-stats").await.1["window"], "24h");
    assert_eq!(b.get("/api/usage-stats?window=%32h").await.1["window"], "2h");
    for bad in ["?window=5x", "?pool=opus", "?window=1h&window=2h", "?window=%2Fh", "?window=%3"] {
        let (status, v) = b.get(&format!("/api/usage-stats{bad}")).await;
        assert_eq!(status, 400, "{bad}: {v}");
    }
}

const HEADER: &str = r#"
const text = (id) => document.getElementById(id).textContent;
const tip = () => document.getElementById("route-tip");
"#;

async fn header(h: H, before: impl FnOnce(&H), scenario: &str) -> Value {
    let b = Board::start(h).await;
    before(&b.h);
    b.h.app.wf.refresh_routing().await.unwrap();
    let html = b.page().await;
    let (_, st) = b.get("/api/state").await;
    run_page_async_with(&html, "", &format!("() => {{\n{HEADER}\nconst base = {st};\n{scenario}\n}}"))
}

#[tokio::test]
async fn the_header_shows_each_members_share_and_marks_a_limited_one() {
    let r = header(
        one_to_three(),
        |h| limit(h, "claude-a"),
        r##"
emitState(base);
const btn = document.getElementById("route-btn");
const before = tip().hidden;
btn.dispatch("click");
const pinned = !tip().hidden;
const tipText = tip().textContent;
btn.dispatch("click");
const hook = [];
routing.open = (r) => hook.push(r.mode);
btn.dispatch("click");
return {
  hidden: document.getElementById("route").hidden,
  wide: text("route-wide"),
  short: text("route-short"),
  dots: document.querySelectorAll("#route-wide .rdot").map((d) => d.className),
  label: btn.getAttribute("aria-label"),
  before, pinned, tipText, closed: tip().hidden, hook,
};"##,
    )
    .await;
    let reset = local_clock(RESET);
    let reset = reset.split(' ').next().unwrap();
    assert_eq!(r["hidden"], false);
    assert_eq!(r["wide"], format!("claude-b 75%claude-a 25% until {reset}"));
    assert_eq!(r["short"], "claude-b 75%");
    assert_eq!(r["dots"], json!(["rdot ready", "rdot limited"]));
    let label = r["label"].as_str().unwrap();
    assert!(label.contains(&format!("claude-a 25% until {reset}, limited")), "{label}");
    assert_eq!(
        (r["before"].clone(), r["pinned"].clone(), r["closed"].clone()),
        (json!(true), json!(true), json!(true))
    );
    let tip = r["tipText"].as_str().unwrap();
    assert!(
        tip.contains("Pool opus, strategy weighted; no picks in the last 24h, so share is the configured weight."),
        "{tip}"
    );
    assert!(tip.contains("claude-a · limited · 25% · 0 running"), "{tip}");
    assert!(tip.contains(&format!("Limit resets {reset} (usage limit)")), "{tip}");
    assert_eq!(r["hook"], json!(["pool"]));
}

#[tokio::test]
async fn the_header_names_the_single_instance_without_a_pool() {
    let r = header(
        H::new(),
        |_| {},
        r#"
emitState(base);
return { wide: text("route-wide"), short: text("route-short"), tip: tip().textContent };"#,
    )
    .await;
    assert_eq!(r["wide"], format!("claudeAgent · {OPUS} · high (no pool)"));
    assert_eq!(r["short"], "claudeAgent (no pool)");
    let tip = r["tip"].as_str().unwrap();
    assert!(tip.starts_with(&format!("No pool: launches run on claudeAgent, {OPUS}, effort high.")), "{tip}");
}
