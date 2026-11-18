use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::prelude::Async;
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Function, Promise};
use serde_json::{Value, json};

use crate::t3::BoxFut;

pub const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const STACK_LIMIT: usize = 512 * 1024;
const LOG_LINES: usize = 50;
const LOG_CHARS: usize = 4000;
const RUN_GAP: Duration = Duration::from_millis(50);

pub trait Host: Send + Sync + 'static {
    fn call(&self, op: String, args: Value) -> BoxFut<'static, Result<Value, String>>;
}

pub struct NoHost;

impl Host for NoHost {
    fn call(&self, op: String, _args: Value) -> BoxFut<'static, Result<Value, String>> {
        Box::pin(async move { Err(format!("{op} is not available here")) })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory: usize,
    pub cpu: Duration,
    pub wall: Duration,
}

pub struct Run {
    pub js: String,
    pub prelude: &'static str,
    pub globals: Vec<(&'static str, String)>,
    pub host: Arc<dyn Host>,
    pub limits: Limits,
    pub keep_vars: bool,
}

#[derive(Debug)]
pub struct Outcome {
    pub result: Result<Value, String>,
    pub logs: Vec<String>,
    pub vars: Option<Value>,
    pub cpu: Duration,
}

#[derive(Default)]
struct Logs {
    lines: Vec<String>,
    chars: usize,
    dropped: usize,
}

impl Logs {
    fn push(&mut self, line: String) {
        if self.lines.len() >= LOG_LINES || self.chars >= LOG_CHARS {
            self.dropped += 1;
            return;
        }
        let room = LOG_CHARS - self.chars;
        let line: String =
            if line.chars().count() > room { line.chars().take(room).collect::<String>() + " ...(cut)" } else { line };
        self.chars += line.chars().count();
        self.lines.push(line);
    }

    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.lines.push(format!(
                "... {} more log lines dropped (cap {LOG_LINES} lines / {LOG_CHARS} chars)",
                self.dropped
            ));
        }
        self.lines
    }
}

const TRIP_NONE: u8 = 0;
const TRIP_CPU: u8 = 1;
const TRIP_WALL: u8 = 2;

struct Meter {
    epoch: Instant,
    last: AtomicU64,
    cpu: AtomicU64,
    cpu_budget: u64,
    wall: u64,
    tripped: AtomicU8,
}

impl Meter {
    fn new(limits: &Limits) -> Meter {
        Meter {
            epoch: Instant::now(),
            last: AtomicU64::new(0),
            cpu: AtomicU64::new(0),
            cpu_budget: limits.cpu.as_nanos() as u64,
            wall: limits.wall.as_nanos() as u64,
            tripped: AtomicU8::new(TRIP_NONE),
        }
    }

    fn check(&self) -> bool {
        let now = self.epoch.elapsed().as_nanos() as u64;
        let last = self.last.swap(now, Ordering::Relaxed);
        let delta = now.saturating_sub(last);
        let cpu = if delta < RUN_GAP.as_nanos() as u64 {
            self.cpu.fetch_add(delta, Ordering::Relaxed) + delta
        } else {
            self.cpu.load(Ordering::Relaxed)
        };
        if cpu > self.cpu_budget {
            self.tripped.store(TRIP_CPU, Ordering::Relaxed);
            return true;
        }
        if now > self.wall {
            self.tripped.store(TRIP_WALL, Ordering::Relaxed);
            return true;
        }
        false
    }

    fn reset_gap(&self) {
        self.last.store(self.epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    fn cpu(&self) -> Duration {
        Duration::from_nanos(self.cpu.load(Ordering::Relaxed))
    }
}

const RUNNER: &str = r#"
(async () => {
  const fmt = globalThis.__fmtError;
  try {
    const r = await globalThis.__main();
    try {
      return JSON.stringify({ ok: true, result: r === undefined ? null : r });
    } catch (e) {
      return JSON.stringify({ ok: false, error: "the return value is not JSON-serializable: " + fmt(e) });
    }
  } catch (e) {
    return JSON.stringify({ ok: false, error: fmt(e) });
  }
})()
"#;

const VARS_OUT: &str = r#"
(() => {
  try { return JSON.stringify(globalThis.vars === undefined ? {} : globalThis.vars) ?? "null"; }
  catch (e) { return null; }
})()
"#;

pub const BASE_PRELUDE: &str = r#"
globalThis.__fmtError = (e) => {
  if (e instanceof Error) {
    const name = e.name && e.name !== "Error" ? e.name + ": " : "";
    return name + e.message;
  }
  try { return "thrown value: " + JSON.stringify(e); } catch (_) { return "thrown value: " + String(e); }
};
globalThis.console = ((log) => {
  const show = (x) => {
    if (typeof x === "string") return x;
    try { const s = JSON.stringify(x); return s === undefined ? String(x) : s; } catch (_) { return String(x); }
  };
  const line = (prefix) => (...xs) => { log(prefix + xs.map(show).join(" ")); };
  return Object.freeze({ log: line(""), info: line(""), debug: line(""), warn: line("[warn] "), error: line("[error] ") });
})(globalThis.__log);
delete globalThis.__log;
delete globalThis.SharedArrayBuffer;
delete globalThis.Atomics;
"#;

fn caught(ctx: &Ctx<'_>, e: CaughtError<'_>) -> String {
    let _ = ctx;
    match e {
        CaughtError::Exception(ex) => {
            let msg = ex.message().unwrap_or_default();
            let name: Option<String> = ex.get("name").ok();
            match name {
                Some(n) if n != "Error" && !n.is_empty() => format!("{n}: {msg}"),
                _ => msg,
            }
        }
        CaughtError::Value(v) => format!("thrown value: {v:?}"),
        CaughtError::Error(e) => e.to_string(),
    }
}

pub async fn run(job: Run) -> Outcome {
    let logs = Arc::new(Mutex::new(Logs::default()));
    let meter = Arc::new(Meter::new(&job.limits));
    let rt = match AsyncRuntime::new() {
        Ok(rt) => rt,
        Err(e) => return failed(format!("sandbox runtime: {e}"), logs, &meter),
    };
    rt.set_memory_limit(job.limits.memory).await;
    rt.set_max_stack_size(STACK_LIMIT).await;
    let m = meter.clone();
    rt.set_interrupt_handler(Some(Box::new(move || m.check()))).await;
    let ctx = match AsyncContext::full(&rt).await {
        Ok(c) => c,
        Err(e) => return failed(format!("sandbox context: {e}"), logs, &meter),
    };
    let l2 = logs.clone();
    let host = job.host.clone();
    let m2 = meter.clone();
    let keep_vars = job.keep_vars;
    let fut = ctx.async_with(async move |ctx| -> Result<(String, Option<String>), String> {
        let g = ctx.globals();
        let m3 = m2.clone();
        let call = Function::new(
            ctx.clone(),
            Async(move |op: String, args: String| {
                let host = host.clone();
                let m = m3.clone();
                async move {
                    let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
                    let r = host.call(op, args).await;
                    m.reset_gap();
                    let env = match r {
                        Ok(v) => json!({"ok": true, "value": v}),
                        Err(e) => json!({"ok": false, "error": e}),
                    };
                    Ok::<String, rquickjs::Error>(env.to_string())
                }
            }),
        )
        .map_err(|e| e.to_string())?;
        let log = Function::new(ctx.clone(), move |s: String| l2.lock().unwrap().push(s)).map_err(|e| e.to_string())?;
        g.set("__host", call).map_err(|e| e.to_string())?;
        g.set("__log", log).map_err(|e| e.to_string())?;
        for (name, json) in job.globals {
            let v = ctx.json_parse(json).catch(&ctx).map_err(|e| caught(&ctx, e))?;
            g.set(name, v).map_err(|e| e.to_string())?;
        }
        ctx.eval::<(), _>(BASE_PRELUDE).catch(&ctx).map_err(|e| caught(&ctx, e))?;
        ctx.eval::<(), _>(job.prelude).catch(&ctx).map_err(|e| caught(&ctx, e))?;
        m2.reset_gap();
        ctx.eval::<(), _>(job.js).catch(&ctx).map_err(|e| caught(&ctx, e))?;
        let p: Promise = ctx.eval(RUNNER).catch(&ctx).map_err(|e| caught(&ctx, e))?;
        let out: String = p.into_future::<String>().await.catch(&ctx).map_err(|e| caught(&ctx, e))?;
        let vars = if keep_vars {
            ctx.eval::<Option<String>, _>(VARS_OUT).catch(&ctx).map_err(|e| caught(&ctx, e))?
        } else {
            None
        };
        Ok((out, vars))
    });
    let res = tokio::time::timeout(job.limits.wall, fut).await;
    drop(ctx);
    drop(rt);
    let logs = std::mem::take(&mut *logs.lock().unwrap()).finish();
    let tripped = meter.tripped.load(Ordering::Relaxed);
    let wall_msg = || format!("timed out after {} ms (timeout_ms)", job.limits.wall.as_millis());
    let cpu_msg = || {
        format!(
            "CPU budget exceeded: {} ms of JavaScript run time (waits on t3/mem/wf calls do not count)",
            job.limits.cpu.as_millis()
        )
    };
    let (result, vars) = match res {
        Err(_) => (Err(wall_msg()), None),
        Ok(Err(_)) if tripped == TRIP_CPU => (Err(cpu_msg()), None),
        Ok(Err(_)) if tripped == TRIP_WALL => (Err(wall_msg()), None),
        Ok(Err(e)) => (Err(e), None),
        Ok(Ok((out, vars))) => {
            let mut v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
            let result = if v["ok"] == true {
                Ok(v["result"].take())
            } else {
                Err(v["error"].as_str().unwrap_or("unknown error").to_string())
            };
            (result, vars.and_then(|s| serde_json::from_str(&s).ok()))
        }
    };
    Outcome { result, logs, vars, cpu: meter.cpu() }
}

fn failed(msg: String, logs: Arc<Mutex<Logs>>, meter: &Meter) -> Outcome {
    let logs = std::mem::take(&mut *logs.lock().unwrap()).finish();
    Outcome { result: Err(msg), logs, vars: None, cpu: meter.cpu() }
}
