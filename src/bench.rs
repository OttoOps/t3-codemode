use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::app::{App, ExecuteRequest};
use crate::clock::SystemClock;
use crate::config::Config;
use crate::fake::{FakeT3, thread};
use crate::store::Store;

#[cfg(not(target_os = "linux"))]
pub fn rss_kb() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
pub fn rss_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(String::from))
        .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
}

fn rss() -> String {
    rss_kb().map_or_else(|| "n/a (Linux only)".into(), |kb| format!("{kb} KB"))
}

fn pct(xs: &mut [f64], q: f64) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[((xs.len() as f64 - 1.0) * q).round() as usize]
}

async fn time_exec(app: &Arc<App>, code: &str, n: usize) -> (f64, f64) {
    let mut t = Vec::with_capacity(n);
    for _ in 0..n {
        let s = Instant::now();
        let out = app
            .execute(ExecuteRequest {
                code: code.into(),
                session: Some("bench".into()),
                timeout_ms: None,
                caller: Some("bench:thread".into()),
                caller_from_arg: false,
                reported: Default::default(),
            })
            .await;
        t.push(s.elapsed().as_secs_f64() * 1e3);
        assert!(!out.is_error, "{}", out.text);
    }
    (pct(&mut t, 0.5), pct(&mut t, 0.95))
}

pub async fn run(n: usize, tools: Option<&Path>) -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join(format!("t3cm-bench-{}", std::process::id()));
    let tools: Vec<Value> = match tools {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => Vec::new(),
    };
    let fake = Arc::new(FakeT3::with_tools(tools));
    fake.put(thread("bench:thread", "idle", "2026-11-17T12:00:00Z"));
    let store = Arc::new(Store::open(&dir)?);
    let rss0 = rss();
    let app = App::new(Config::default(), store, fake, Arc::new(SystemClock));
    app.refresh_catalog().await.map_err(anyhow::Error::msg)?;
    println!("tools in catalog: {}", app.catalog().t3_names.len());
    println!("rss idle (app ready): {} (process start {rss0})", rss());
    let (p50, p95) = time_exec(&app, "async () => 1", n).await;
    println!("execute trivial x{n}: p50 {p50:.2} ms, p95 {p95:.2} ms");
    println!("rss after {n} executes: {}", rss());
    let (p50, p95) = time_exec(&app, "async () => (await t3.t3_thread_list({ limit: 1 })).threads.length", n).await;
    println!("execute 1 host call x{n}: p50 {p50:.2} ms, p95 {p95:.2} ms");
    let (p50, p95) = time_exec(
        &app,
        "async () => { vars.n = (vars.n ?? 0) + 1; await mem.set('k', vars.n); return await mem.get('k'); }",
        n,
    )
    .await;
    println!("execute vars+mem x{n}: p50 {p50:.2} ms, p95 {p95:.2} ms");
    let mut t = Vec::with_capacity(n);
    for _ in 0..n {
        let s = Instant::now();
        let out = app.search("async () => catalog.filter(t => /thread/.test(t.name)).length").await;
        t.push(s.elapsed().as_secs_f64() * 1e3);
        assert!(!out.is_error, "{}", out.text);
    }
    println!("search x{n}: p50 {:.2} ms, p95 {:.2} ms", pct(&mut t, 0.5), pct(&mut t, 0.95));
    println!("rss after {} runs: {}", 4 * n, rss());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
