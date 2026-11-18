use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use t3_codemode::app::App;
use t3_codemode::catalog::Catalog;
use t3_codemode::clock::{SystemClock, local_datetime, parse_ts};
use t3_codemode::config::{self, Config};
use t3_codemode::store::Store;
use t3_codemode::t3_http::HttpT3;
use t3_codemode::{bench, platform, server, watcher};

#[derive(Parser)]
#[command(name = "t3cm", version, about = "Code Mode MCP server for T3 Code: two tools, search and execute")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct StateArgs {
    #[arg(long, env = "T3CM_STATE_DIR")]
    state_dir: Option<PathBuf>,
}

#[derive(clap::Args, Clone)]
struct PortArgs {
    #[arg(long, env = "T3CM_PORT", default_value_t = 47731)]
    port: u16,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve MCP over HTTP on 127.0.0.1, with the link watcher
    Serve {
        #[command(flatten)]
        port: PortArgs,
        #[command(flatten)]
        state: StateArgs,
        #[arg(long)]
        no_watcher: bool,
    },
    /// Serve MCP over stdio (no watcher unless --watcher)
    Stdio {
        #[command(flatten)]
        state: StateArgs,
        #[arg(long)]
        watcher: bool,
    },
    /// Acquire a fresh T3 OAuth session now; prints only its expiry
    Auth {
        #[command(flatten)]
        state: StateArgs,
    },
    /// Show the running service's /healthz
    Status {
        #[command(flatten)]
        port: PortArgs,
    },
    /// Run a script through the running service's execute tool
    Exec {
        file: String,
        #[command(flatten)]
        port: PortArgs,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long, env = "T3CODE_THREAD_ID")]
        thread: Option<String>,
    },
    /// Run a query through the running service's search tool
    Search {
        file: String,
        #[command(flatten)]
        port: PortArgs,
    },
    /// Print the generated TypeScript declarations for the catalog
    Types {
        #[arg(long)]
        from: Option<PathBuf>,
        #[command(flatten)]
        state: StateArgs,
    },
    #[command(hide = true)]
    Bench {
        #[arg(short, default_value_t = 1000)]
        n: usize,
        #[arg(long)]
        tools: Option<PathBuf>,
    },
}

fn state_dir(a: &StateArgs) -> PathBuf {
    a.state_dir.clone().unwrap_or_else(platform::state_dir)
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(false)
        .init();
}

fn build(state: &StateArgs) -> Result<(Arc<App>, Arc<HttpT3>)> {
    let cfg = Config::load(&config::default_path())?;
    let store = Arc::new(Store::open(&state_dir(state))?);
    let clock = Arc::new(SystemClock);
    let t3 = Arc::new(HttpT3::new(store.clone(), clock.clone()));
    Ok((App::new(cfg, store, t3.clone(), clock), t3))
}

async fn warm_up(app: Arc<App>, t3: Arc<HttpT3>) {
    let up = app.clone();
    tokio::spawn(async move {
        let listed = up.upstreams.tools().await;
        tracing::info!(upstreams = listed.len(), "upstream catalogs loaded");
    });
    if let Err(e) = t3.ensure_token(false).await {
        tracing::warn!("T3 auth not ready yet: {e}");
        return;
    }
    match app.refresh_catalog().await {
        Ok(n) => tracing::info!(tools = n, "T3 catalog loaded"),
        Err(e) => tracing::warn!("T3 catalog not loaded yet: {e}"),
    }
}

#[cfg(unix)]
async fn os_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    term.recv().await;
}

#[cfg(windows)]
async fn os_shutdown() {
    use tokio::signal::windows::{ctrl_break, ctrl_close, ctrl_shutdown};
    let mut brk = ctrl_break().expect("CTRL_BREAK handler");
    let mut close = ctrl_close().expect("CTRL_CLOSE handler");
    let mut shutdown = ctrl_shutdown().expect("CTRL_SHUTDOWN handler");
    tokio::select! {
        _ = brk.recv() => {},
        _ = close.recv() => {},
        _ = shutdown.recv() => {},
    }
}

async fn shutdown_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = os_shutdown() => {},
    }
    tracing::info!("shutting down");
}

fn read_source(file: &str) -> Result<String> {
    if file == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(file).with_context(|| format!("reading {file}"))
    }
}

async fn call_service(port: u16, tool: &str, args: Value, thread: Option<&str>) -> Result<(bool, String)> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    let mut req =
        reqwest::Client::builder().no_proxy().build()?.post(format!("http://127.0.0.1:{port}/mcp")).json(&body);
    if let Some(t) = thread {
        req = req.header(server::THREAD_HEADER, t);
    }
    let v: Value = req.send().await.with_context(|| format!("is t3cm serving on 127.0.0.1:{port}?"))?.json().await?;
    if let Some(e) = v.get("error") {
        bail!("{}", e["message"]);
    }
    let r = &v["result"];
    let text = r["content"][0]["text"].as_str().unwrap_or_default().to_string();
    Ok((r["isError"] == true, text))
}

async fn run(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Serve { port, state, no_watcher } => {
            init_logging();
            let (app, t3) = build(&state)?;
            let (listener, addr) =
                server::bind(port.port).await.with_context(|| format!("binding 127.0.0.1:{}", port.port))?;
            tracing::info!(%addr, store = %app.store.path().display(), "t3-codemode serving MCP at /mcp");
            tokio::spawn(warm_up(app.clone(), t3));
            if !no_watcher {
                watcher::spawn(app.clone());
            }
            server::serve(app.clone(), listener, shutdown_signal()).await?;
            app.upstreams.shutdown().await;
        }
        Cmd::Stdio { state, watcher: with_watcher } => {
            init_logging();
            let (app, t3) = build(&state)?;
            tokio::spawn(warm_up(app.clone(), t3));
            if with_watcher {
                watcher::spawn(app.clone());
            }
            let caller = std::env::var("T3CODE_THREAD_ID").ok().filter(|s| !s.is_empty());
            server::stdio(app.clone(), caller).await?;
            app.upstreams.shutdown().await;
        }
        Cmd::Auth { state } => {
            let (_, t3) = build(&state)?;
            let tok = t3.force_acquire().await?;
            let when = parse_ts(&tok.expires_at).map(local_datetime).unwrap_or(tok.expires_at);
            println!("T3 session acquired; expires {when}");
        }
        Cmd::Status { port } => {
            let v: Value = reqwest::Client::builder()
                .no_proxy()
                .build()?
                .get(format!("http://127.0.0.1:{}/healthz", port.port))
                .send()
                .await
                .with_context(|| format!("is t3cm serving on 127.0.0.1:{}?", port.port))?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::Exec { file, port, session, timeout_ms, thread } => {
            let mut args = json!({"code": read_source(&file)?});
            if let Some(s) = session {
                args["session"] = json!(s);
            }
            if let Some(t) = timeout_ms {
                args["timeout_ms"] = json!(t);
            }
            let (is_error, text) = call_service(port.port, "execute", args, thread.as_deref()).await?;
            println!("{text}");
            return Ok(if is_error { ExitCode::FAILURE } else { ExitCode::SUCCESS });
        }
        Cmd::Search { file, port } => {
            let (is_error, text) =
                call_service(port.port, "search", json!({"code": read_source(&file)?}), None).await?;
            println!("{text}");
            return Ok(if is_error { ExitCode::FAILURE } else { ExitCode::SUCCESS });
        }
        Cmd::Types { from, state } => {
            let tools: Vec<Value> = match from {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
                None => {
                    let store = Store::open(&state_dir(&state))?;
                    let cached = store
                        .meta_get("t3_tools")?
                        .context("no cached T3 tool list; run `t3cm serve` once or pass --from <tools.json>")?;
                    serde_json::from_str(&cached)?
                }
            };
            let cat = Catalog::build(&tools);
            let decls = cat.declarations();
            println!("{decls}");
            let raw = serde_json::to_string(&tools)?.len();
            eprintln!(
                "{} T3 tools + {} built-ins: {} chars of declarations (~{} tokens) vs {} chars of raw tool JSON (~{} tokens)",
                cat.t3_names.len(),
                cat.entries.len() - cat.t3_names.len(),
                decls.len(),
                decls.len() / 4,
                raw,
                raw / 4
            );
        }
        Cmd::Bench { n, tools } => bench::run(n, tools.as_deref()).await?,
    }
    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("t3cm: {e:#}");
            ExitCode::FAILURE
        }
    }
}
