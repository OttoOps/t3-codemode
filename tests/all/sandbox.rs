use std::time::Instant;

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::config::Config;
use t3_codemode::js::normalize_code;
use t3_codemode::truncate::{self, MARKER, MAX_CHARS};

#[tokio::test]
async fn returns_the_result_as_json() {
    let h = H::new();
    let out = h.exec("async () => ({ a: 1, b: [true, null, 'x'] })").await;
    assert_eq!(ok_json(&out), json!({"result": {"a": 1, "b": [true, null, "x"]}}));
    assert_eq!(result(&h.exec("async () => {}").await), Value::Null);
}

#[tokio::test]
async fn strips_typescript() {
    let h = H::new();
    let code = r#"async (): Promise<unknown> => {
        interface Shape { area(): number }
        enum Color { Red, Green }
        type Pair<T> = [T, T];
        class Sq implements Shape { constructor(private s: number) {} area() { return this.s ** 2; } }
        const p: Pair<number> = [1, 2];
        const id = <T,>(v: T): T => v;
        return { green: Color.Green, area: new Sq(3).area(), p, id: id<string>("ok"), n: (p as any)!.length };
    }"#;
    assert_eq!(result(&h.exec(code).await), json!({"green": 1, "area": 9, "p": [1, 2], "id": "ok", "n": 2}));
}

#[tokio::test]
async fn captures_console_and_caps_it() {
    let h = H::new();
    let out = ok_json(
        &h.exec("async () => { console.log('a', {b: 1}); console.warn('w'); console.error('e'); return 1 }").await,
    );
    assert_eq!(out["logs"], json!(["a {\"b\":1}", "[warn] w", "[error] e"]));
    let out =
        ok_json(&h.exec("async () => { for (let i = 0; i < 100; i++) console.log('line ' + i); return 1 }").await);
    let logs = out["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 51);
    assert!(logs[50].as_str().unwrap().contains("50 more log lines dropped"));
    let out = ok_json(&h.exec("async () => { console.log('x'.repeat(10000)); return 1 }").await);
    assert!(out["logs"][0].as_str().unwrap().chars().count() < 4100);
}

#[tokio::test]
async fn interrupts_an_infinite_loop_within_the_cpu_budget() {
    let h = H::new();
    let t = Instant::now();
    let out = h.exec("async () => { while (true) {} }").await;
    let took = t.elapsed();
    assert!(err_text(&out).contains("CPU budget exceeded"), "{}", out.text);
    assert!(took.as_secs_f64() < 3.5, "took {took:?}");
    assert!(took.as_secs_f64() > 1.5, "took {took:?}");
}

#[tokio::test]
async fn time_awaiting_host_calls_does_not_count_against_the_cpu_budget() {
    let h = H::new();
    let out = h.run(None, "async () => { await t3.call('sleep', { ms: 2600 }); return 'awake' }", Some(10_000)).await;
    assert_eq!(result(&out), json!("awake"));
}

#[tokio::test]
async fn the_wall_timeout_stops_a_slow_host_call() {
    let h = H::new();
    let t = Instant::now();
    let out = h.run(None, "async () => { await t3.call('sleep', { ms: 5000 }); return 1 }", Some(300)).await;
    assert!(err_text(&out).contains("timed out after 300 ms"), "{}", out.text);
    assert!(t.elapsed().as_millis() < 2000);
}

#[tokio::test]
async fn a_memory_bomb_fails_cleanly_and_the_next_run_works() {
    let h = H::new();
    let out = h.exec("async () => { const a: number[][] = []; for (;;) a.push(new Array(1e6).fill(1)); }").await;
    assert!(err_text(&out).contains("out of memory"), "{}", out.text);
    let out = h.exec("async () => { let s = 'x'; for (;;) s = s + s; }").await;
    assert!(out.is_error, "{}", out.text);
    assert_eq!(result(&h.exec("async () => 2 + 2").await), json!(4));
}

#[tokio::test]
async fn deep_recursion_is_caught() {
    let h = H::new();
    let out = h.exec("async () => { const f = (n: number): number => f(n + 1) + 1; return f(0); }").await;
    assert!(err_text(&out).contains("Maximum call stack size exceeded"), "{}", out.text);
    assert_eq!(result(&h.exec("async () => 1").await), json!(1));
}

#[tokio::test]
async fn no_require_process_fetch_import_or_timers() {
    let h = H::new();
    let out = h
        .exec(r#"async () => ({
            require: typeof require, process: typeof process, fetch: typeof fetch, setTimeout: typeof setTimeout,
            SharedArrayBuffer: typeof SharedArrayBuffer, Atomics: typeof Atomics, host: typeof __host, names: typeof __names })"#)
        .await;
    let r = result(&out);
    for (k, v) in r.as_object().unwrap() {
        assert_eq!(v, "undefined", "{k} should be unavailable");
    }
    let out = h
        .exec(r#"async () => { try { await import("fs"); return "imported"; } catch (e) { return "blocked"; } }"#)
        .await;
    assert_eq!(result(&out), json!("blocked"));
    let out = h.exec(r#"import fs from "fs"; export default async () => 1"#).await;
    assert!(out.is_error, "{}", out.text);
}

#[tokio::test]
async fn reference_errors_list_the_globals() {
    let h = H::new();
    let out = h.exec("async () => require('fs')").await;
    let text = err_text(&out);
    assert!(text.starts_with("Error: ReferenceError: require is not defined"), "{text}");
    assert!(text.contains("Available globals: t3, mem, vars, wf, console."), "{text}");
    let out = h.app.search("async () => t3.call('x')").await;
    assert!(err_text(&out).contains("Available globals: catalog, console."), "{}", out.text);
}

#[tokio::test]
async fn errors_keep_logs_and_never_echo_code() {
    let h = H::new();
    let out = h.exec("async () => { console.log('before'); throw new Error('boom'); }").await;
    assert_eq!(out.text, "Error: boom\nLogs:\nbefore");
    let out = h.exec("async () => { const secretMarker = 1; throw 42 }").await;
    assert_eq!(out.text, "Error: thrown value: 42");
    let out = h.exec("async () => 10n").await;
    assert!(err_text(&out).contains("not JSON-serializable"), "{}", out.text);
}

#[tokio::test]
async fn syntax_errors_point_at_the_line() {
    let h = H::new();
    let out = h.exec("async () => {\n  const x = foo(;\n  return x\n}").await;
    let text = err_text(&out);
    assert!(text.starts_with("Error: SyntaxError:"), "{text}");
    assert!(text.contains("const x = foo(;"), "{text}");
}

#[tokio::test]
async fn t3_errors_throw_with_the_t3_message() {
    let h = H::new();
    let out = h.exec("async () => t3.call('fail', { message: 'thread x not found' })").await;
    assert_eq!(out.text, "Error: fail: thread x not found");
    let out = h.exec("async () => { try { await t3.call('fail', {}) } catch (e) { return e instanceof Error ? e.message : 'no' } }").await;
    assert_eq!(result(&out), json!("fail: the fake tool failed"));
}

#[tokio::test]
async fn host_calls_run_concurrently_under_promise_all() {
    let h = H::new();
    let t = Instant::now();
    let out = h
        .exec("async () => (await Promise.all([1, 2, 3, 4].map(n => t3.call('sleep', { ms: 400, n })))).map(r => r.n)")
        .await;
    assert_eq!(result(&out), json!([1, 2, 3, 4]));
    assert!(t.elapsed().as_millis() < 1000, "took {:?}", t.elapsed());
}

#[tokio::test]
async fn big_results_are_truncated_structurally() {
    let h = H::new();
    let out = h
        .exec("async () => ({ count: 2000, rows: Array.from({ length: 2000 }, (_, i) => ({ i, text: 'y'.repeat(100) })), tail: 'end' })")
        .await;
    assert!(!out.is_error);
    assert!(out.text.len() <= MAX_CHARS, "{}", out.text.len());
    let v: Value = serde_json::from_str(&out.text).expect("valid JSON");
    assert!(v["truncated"].as_str().unwrap().starts_with(MARKER));
    assert_eq!(v["result"]["count"], 2000);
    assert_eq!(v["result"]["tail"], "end");
    let rows = v["result"]["rows"].as_array().unwrap();
    assert!(rows.last().unwrap().as_str().unwrap().contains("more items"));
    assert_eq!(rows[0]["i"], 0);
}

#[test]
fn truncation_keeps_valid_json_for_any_shape() {
    let cases = vec![
        json!("z".repeat(100_000)),
        json!({"a": "x".repeat(50_000), "b": "small", "c": [1, 2, 3]}),
        json!((0..5000).map(|i| json!({"k": i, "v": "w".repeat(30)})).collect::<Vec<_>>()),
        json!({"deep": {"deeper": {"deepest": (0..3000).map(|i| format!("é{i}")).collect::<Vec<_>>()}}}),
        json!((0..3000).map(|i| (format!("key{i}"), json!("q".repeat(40)))).collect::<serde_json::Map<_, _>>()),
    ];
    for c in cases {
        let text = truncate::render(c, vec!["log".into()]);
        assert!(text.len() <= MAX_CHARS, "{}", text.len());
        let v: Value = serde_json::from_str(&text).expect("valid JSON");
        assert!(v["truncated"].is_string());
        assert_eq!(v["logs"], json!(["log"]));
    }
    let small = truncate::render(json!({"a": 1}), vec![]);
    assert_eq!(small, r#"{"result":{"a":1}}"#);
}

#[test]
fn normalize_code_accepts_the_usual_shapes() {
    assert_eq!(normalize_code("async () => 1"), "async () => 1");
    assert_eq!(normalize_code("```js\nasync () => 1\n```"), "async () => 1");
    assert_eq!(normalize_code("```typescript\nasync () => 1;\n```"), "async () => 1");
    assert_eq!(normalize_code("export default async () => 2"), "async () => 2");
    assert_eq!(normalize_code("async function main() { return 3 }"), "(async function main() { return 3 })");
    assert_eq!(
        normalize_code("export default async function run() { return 4 }"),
        "(async function run() { return 4 })"
    );
    assert_eq!(normalize_code("const a = 1;\na + 4"), "async () => {\nconst a = 1;\nreturn (a + 4);\n}");
    assert_eq!(
        normalize_code("const r = await t3.x();\nreturn r"),
        "async () => {\nconst r = await t3.x();\nreturn r\n}"
    );
}

#[tokio::test]
async fn normalized_shapes_all_run() {
    let h = H::new();
    for (code, want) in [
        ("```js\nasync () => 1\n```", json!(1)),
        ("export default async () => 2", json!(2)),
        ("async function main() { return 3 }", json!(3)),
        ("const a: number = 1;\na + 3", json!(4)),
        ("const r = await t3.call('echo', { v: 5 });\nreturn r.v", json!(5)),
        ("() => 6", json!(6)),
        ("[7].length * 7", json!(7)),
    ] {
        assert_eq!(result(&h.exec(code).await), want, "{code}");
    }
}

#[tokio::test]
async fn search_runs_against_the_catalog() {
    let h = H::with_tools();
    h.load_catalog().await;
    let out = h.app.search("async () => catalog.filter(t => /thread/.test(t.name)).map(t => t.name)").await;
    let names = result(&out);
    assert!(names.as_array().unwrap().contains(&json!("t3_thread_send")));
    let out = h.app.search(r#"async () => catalog.find(t => t.name === "t3_thread_send").signature"#).await;
    let sig = result(&out);
    assert!(sig.as_str().unwrap().contains("t3.t3_thread_send(input: {"), "{sig}");
    let out = h.app.search("async () => [...new Set(catalog.map(t => t.kind))]").await;
    assert_eq!(result(&out), json!(["t3", "mem", "wf"]));
    let out = h.app.search("async () => typeof wf + typeof mem + typeof vars").await;
    assert_eq!(result(&out), json!("undefinedundefinedundefined"));
}

fn capped(max: usize, queue_ms: u64) -> H {
    H::build(Vec::new(), Config::from_value(json!({"maxConcurrentRuns": max, "runQueueMs": queue_ms})).unwrap())
}

const SLOW: &str = "async () => (await t3.call('sleep', { ms: 600 })).ms";

#[tokio::test]
async fn a_run_past_the_concurrency_cap_fails_busy_after_the_queue_wait() {
    let h = capped(1, 150);
    let (slow, fast) = tokio::join!(h.exec(SLOW), async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        h.exec("async () => 1").await
    });
    assert_eq!(result(&slow), json!(600));
    assert!(err_text(&fast).contains("t3cm is busy: 1 runs are in progress"), "{}", fast.text);
    assert_eq!(result(&h.exec("async () => 2").await), json!(2));
}

#[tokio::test]
async fn a_queued_run_starts_when_a_slot_frees() {
    let h = capped(1, 5000);
    let (slow, queued, health) = tokio::join!(
        h.exec(SLOW),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            h.exec("async () => 'next'").await
        },
        async {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            h.app.health()["runs"].clone()
        }
    );
    assert_eq!(result(&slow), json!(600));
    assert_eq!(result(&queued), json!("next"));
    assert_eq!(health, json!({"running": 1, "queued": 1, "max": 1}));
    assert_eq!(h.app.health()["runs"], json!({"running": 0, "queued": 0, "max": 1}));
}

#[test]
fn zero_concurrent_runs_is_rejected() {
    let e = Config::from_value(json!({"maxConcurrentRuns": 0})).unwrap_err();
    assert!(format!("{e:#}").contains("maxConcurrentRuns must be at least 1"));
}
