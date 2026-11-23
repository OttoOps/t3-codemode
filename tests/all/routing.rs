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
const opens = routing.open;
routing.open = null;
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
  before, pinned, tipText, closed: tip().hidden, hook, opens: typeof opens,
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
    assert_eq!(r["opens"], "function");
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

#[tokio::test]
async fn a_failed_follow_up_capabilities_search_reports_the_instance_as_unknown() {
    let c = json!({"board": {"instanceId": "claude-a", "ticketModel": "claude-catalog", "ticketEffort": "high"}});
    let h = H::build(Vec::new(), Config::from_value(c).unwrap());
    h.fake.set_providers(vec![provider("claude-a", "claudeAgent", &[OPUS, "claude-catalog"])]);
    h.fake.set_short_list(1);
    let found = &stats(&h, "").await["members"][0];
    assert_eq!((found["status"].clone(), found["instanceId"].clone()), (json!("ready"), json!("claude-a")));
    h.fake.set_search_fail(true);
    let failed = &stats(&h, "").await["members"][0];
    assert_eq!(failed["status"], "unknown");
    assert!(failed["reason"].as_str().unwrap().starts_with("T3 capabilities unreachable"), "{failed}");
    h.fake.set_search_fail(false);
    h.fake.set_providers(vec![provider("claude-a", "claudeAgent", &[OPUS])]);
    assert_eq!(stats(&h, "").await["members"][0]["status"], "disabled");
}

const PANEL: &str = r##"
const q = (sel) => document.querySelector(sel);
const tick = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
const reply = (status, body) => Promise.resolve({ ok: status < 400, status, json: async () => body });
const asked = () => fetches.filter((f) => f.url.startsWith("/api/usage-stats")).map((f) => f.url);
const members = () => document.querySelectorAll("#rp-members li").map((li) => li.textContent);
const panel = () => ({
  hidden: q("#p-route").hidden,
  collapsed: q("#p-route").classList.contains("collapsed"),
  expanded: q("#rp-fold").getAttribute("aria-expanded"),
});
"##;

async fn panel(h: H, prelude: &str, scenario: &str) -> Value {
    let b = Board::start(h).await;
    b.h.app.wf.refresh_routing().await.unwrap();
    let html = b.page().await;
    let (_, st) = b.get("/api/state").await;
    let mut views = serde_json::Map::new();
    for w in ["1h", "24h", "7d", "all"] {
        views.insert(w.into(), b.get(&format!("/api/usage-stats?window={w}")).await.1);
    }
    let views = Value::Object(views);
    run_page_async_with(
        &html,
        prelude,
        &format!(
            "async () => {{\n{PANEL}\nconst base = {st};\nconst views = {views};\nfetchReply = (url) => {{ const m = /^\\/api\\/usage-stats\\?window=(\\w+)$/.exec(url); return m ? reply(200, views[m[1]]) : new Promise(() => {{}}); }};\n{scenario}\n}}"
        ),
    )
}

#[tokio::test]
async fn the_routing_panel_opens_from_the_header_and_shows_shares_reasons_and_resets() {
    let h = one_to_three();
    limit(&h, "claude-a");
    let r = panel(
        h,
        "",
        r##"
emitState(base);
await tick();
const first = { ...panel(), asked: asked() };
document.getElementById("route-btn").dispatch("click");
await tick();
const opened = {
  ...panel(), asked: asked(), tip: q("#route-tip").hidden, focus: document.activeElement === q("#h-route"),
  meta: q("#rp-meta").textContent, totals: q("#rp-totals").textContent, members: members(),
  running: q("#rp-running").textContent, series: q("#rp-series-sec").hidden,
  pressed: document.querySelectorAll("#rp-windows button").map((b) => [b.textContent, b.getAttribute("aria-pressed")]),
};
return { first, opened };
"##,
    )
    .await;
    assert_eq!(r["first"], json!({"hidden": false, "collapsed": true, "expanded": "false", "asked": []}), "{r}");
    let o = &r["opened"];
    assert_eq!(
        (o["collapsed"].clone(), o["expanded"].clone(), o["tip"].clone(), o["focus"].clone()),
        (json!(false), json!("true"), json!(true), json!(true)),
        "{r}"
    );
    assert_eq!(o["asked"], json!(["/api/usage-stats?window=24h"]), "{r}");
    assert_eq!(o["meta"], "pool opus · weighted", "{r}");
    assert_eq!(o["totals"], "0 picks in the last 24h · share is the configured weight · 0 running", "{r}");
    let reset = local_clock(RESET);
    let reset = reset.split(' ').next().unwrap();
    assert_eq!(
        o["members"],
        json!([
            "claude-b75%ready · 0 picks · 0 running",
            format!("claude-a25%limited · 0 picks · 0 running · resets {reset} (usage limit)"),
        ]),
        "{r}"
    );
    assert_eq!(o["running"], "No threads running.", "{r}");
    assert_eq!(o["series"], true, "{r}");
    assert_eq!(o["pressed"], json!([["1h", "false"], ["24h", "true"], ["7d", "false"], ["All", "false"]]), "{r}");
}

#[tokio::test]
async fn the_routing_panel_counts_picks_over_a_chosen_window_and_follows_live_state() {
    let h = one_to_three();
    for n in 1..=4 {
        launch(&h, n).await;
    }
    limit(&h, "claude-a");
    launch(&h, 5).await;
    let r = panel(
        h,
        r#"globalThis.__storage = {"t3board.routing.window": "7d"};"#,
        r##"
emitState(base);
q("#rp-fold").dispatch("click");
await tick();
const week = {
  asked: asked(), totals: q("#rp-totals").textContent, members: members(), why: q("#rp-why").textContent,
  running: q("#rp-running").textContent, bars: q("#rp-series").children.length,
  filled: document.querySelectorAll("#rp-series .rp-col span").map((s) => [s.dataset.id, s.style.height]),
  key: q("#rp-key").textContent, label: q("#rp-series").getAttribute("aria-label"),
  recent: document.querySelectorAll("#rp-recent li").length,
};
q('#rp-windows [data-win="1h"]').dispatch("click");
await tick();
const hour = { asked: asked().slice(1), stored: localStorage.getItem("t3board.routing.window"),
  pressed: q('#rp-windows [data-win="1h"]').getAttribute("aria-pressed") };
emitState(base);
await tick();
const same = asked().length;
const moved = JSON.parse(JSON.stringify(base));
moved.routing.running = [{ instanceId: "claude-b", threads: 2 }];
moved.routing.members.find((m) => m.instanceId === "claude-b").running = 2;
emitState(moved);
await tick();
const live = { asked: asked().slice(same), running: q("#rp-running").textContent, totals: q("#rp-totals").textContent, top: members()[0] };
q("#rp-fold").dispatch("click");
emitState(moved);
await tick();
const shut = { ...panel(), asked: asked().length - same - live.asked.length };
q("#rp-fold").dispatch("click");
await tick();
return { week, hour, live, shut, reopened: asked().slice(same + live.asked.length) };
"##,
    )
    .await;
    let w = &r["week"];
    assert_eq!(w["asked"], json!(["/api/usage-stats?window=7d"]), "{r}");
    assert_eq!(w["totals"], "5 picks in the last 7d · 5 running", "{r}");
    let reset = local_clock(RESET);
    let reset = reset.split(' ').next().unwrap();
    assert_eq!(
        w["members"],
        json!([
            "claude-b80%ready · 4 picks · 4 runningpicked by weight 4",
            format!(
                "claude-a20%limited · 1 pick · 1 running · resets {reset} (usage limit)picked by weight 1. skipped: limited 1"
            ),
        ]),
        "{r}"
    );
    assert_eq!(w["why"], "Picks: weight 5. Skips: limited 1.", "{r}");
    assert_eq!(w["running"], "claude-a 1 · claude-b 4", "{r}");
    assert_eq!(w["bars"], 24, "{r}");
    assert_eq!(w["filled"], json!([["claude-a", "20%"], ["claude-b", "80%"]]), "{r}");
    assert_eq!(w["key"], "claude-aclaude-b", "{r}");
    assert!(w["label"].as_str().unwrap().starts_with("5 picks in the last 7d in 24 bars of 7 h"), "{r}");
    assert_eq!(w["recent"], 5, "{r}");
    assert_eq!(r["hour"], json!({"asked": ["/api/usage-stats?window=1h"], "stored": "1h", "pressed": "true"}), "{r}");
    assert_eq!(
        r["live"],
        json!({"asked": ["/api/usage-stats?window=1h"], "running": "claude-b 2",
               "totals": "5 picks in the last 1h · 2 running",
               "top": "claude-b80%ready · 4 picks · 2 runningpicked by weight 4"}),
        "{r}"
    );
    assert_eq!(r["reopened"], json!(["/api/usage-stats?window=1h"]), "{r}");
    assert_eq!(r["shut"], json!({"hidden": false, "collapsed": true, "expanded": "false", "asked": 0}), "{r}");
}

const WIDE: &str = r#"
globalThis.__storage = { "t3board.layout": JSON.stringify({ cols: [{ w: null, p: ["dec"] }, { w: null, p: ["new", "flow", "tix"] }, { w: null, p: ["act", "cmd"] }], h: {}, c: { act: true } }) };
const __mq = { wide: false, ls: [] };
globalThis.matchMedia = (query) => ({
  get matches() { return /min-width: 1200px/.test(query) ? __mq.wide : false; },
  addEventListener: (_, f) => __mq.ls.push(f),
});
"#;

#[tokio::test]
async fn in_the_wide_layout_the_routing_panel_starts_collapsed_and_remembers_its_fold() {
    let r = panel(
        one_to_three(),
        WIDE,
        r##"
const ph = document.createElement("div"); ph.className = "ph"; q("#p-route").append(ph);
const saved = () => JSON.parse(localStorage.getItem("t3board.layout") || "null");
__mq.wide = true; for (const f of __mq.ls) f();
emitState(base);
await tick();
const col = (id) => q("#main").children.filter((c) => c.classList.contains("lcol")).findIndex((c) => c.contains(q("#" + id)));
const fold = q("#p-route .fold");
const start = { ...panel(), fold: fold.getAttribute("aria-expanded"), col: col("p-route"), saved: saved(), asked: asked() };
fold.dispatch("click");
await tick();
const open = { ...panel(), fold: fold.getAttribute("aria-expanded"), saved: saved().c, asked: asked(), members: members().length };
__mq.wide = false; for (const f of __mq.ls) f();
const narrow = { ...panel(), inTickets: q("#tixcol").contains(q("#p-route")) };
q("#rp-fold").dispatch("click");
const folded = { ...panel(), saved: saved().c };
return { start, open, narrow, folded };
"##,
    )
    .await;
    assert_eq!(
        r["start"],
        json!({"hidden": false, "collapsed": true, "expanded": "false", "fold": "false", "col": 2,
               "saved": {"cols": [{"w": null, "p": ["dec"]}, {"w": null, "p": ["new", "flow", "tix"]}, {"w": null, "p": ["act", "cmd", "route"]}],
                         "h": {}, "c": {"act": true, "route": true}},
               "asked": []}),
        "{r}"
    );
    assert_eq!(
        r["open"],
        json!({"hidden": false, "collapsed": false, "expanded": "true", "fold": "true", "saved": {"act": true},
               "asked": ["/api/usage-stats?window=24h"], "members": 2}),
        "{r}"
    );
    assert_eq!(r["narrow"], json!({"hidden": false, "collapsed": false, "expanded": "true", "inTickets": true}), "{r}");
    assert_eq!(
        r["folded"],
        json!({"hidden": false, "collapsed": true, "expanded": "false", "saved": {"act": true, "route": true}}),
        "{r}"
    );
}

#[tokio::test]
async fn a_failed_read_shows_under_the_routing_panel_and_retry_reads_again() {
    let r = panel(
        one_to_three(),
        "",
        r##"
fetchReply = () => reply(502, { error: "T3 said no" });
emitState(base);
document.getElementById("route-btn").dispatch("click");
await tick();
const failed = { err: q("#rp-err").hidden, text: q("#rp-err-text").textContent, asked: asked().length };
const busy = JSON.parse(JSON.stringify(views["24h"]));
busy.members.find((m) => m.instanceId === "claude-b").running = 2;
fetchReply = (url) => reply(200, busy);
q("#rp-retry").dispatch("click");
await tick();
const retried = { err: q("#rp-err").hidden, asked: asked().length, members: members().length };
fetchReply = () => reply(502, { error: "T3 said no" });
const moved = JSON.parse(JSON.stringify(base));
moved.routing.running = [{ instanceId: "claude-b", threads: 2 }];
moved.routing.members.find((m) => m.instanceId === "claude-b").running = 2;
emitState(moved);
await tick();
const stale = { err: q("#rp-err").hidden, totals: q("#rp-totals").textContent, top: members()[0], running: q("#rp-running").textContent };
const gone = JSON.parse(JSON.stringify(moved));
gone.routing.members = gone.routing.members.filter((m) => m.instanceId !== "claude-b");
gone.routing.running = [];
emitState(gone);
await tick();
const left = { top: members()[0], totals: q("#rp-totals").textContent };
return { failed, retried, stale, left };
"##,
    )
    .await;
    assert_eq!(
        r["failed"],
        json!({"err": false, "text": "Could not read launch routing: T3 said no", "asked": 1}),
        "{r}"
    );
    assert_eq!(r["retried"], json!({"err": true, "asked": 2, "members": 2}), "{r}");
    assert_eq!(
        r["stale"],
        json!({"err": false, "totals": "0 picks in the last 24h · share is the configured weight · 2 running",
               "top": "claude-b75%ready · 0 picks · 2 running", "running": "claude-b 2"}),
        "{r}"
    );
    assert_eq!(
        r["left"],
        json!({"top": "claude-b75%ready · 0 picks · 0 running",
               "totals": "0 picks in the last 24h · share is the configured weight · 0 running"}),
        "{r}"
    );
}
