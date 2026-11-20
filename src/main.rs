use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use t3_codemode::app::App;
use t3_codemode::catalog::Catalog;
use t3_codemode::clock::{SystemClock, local_datetime, parse_ts};
use t3_codemode::config::{self, Config};
use t3_codemode::release::{self, Version};
use t3_codemode::store::Store;
use t3_codemode::t3_http::HttpT3;
use t3_codemode::{bench, platform, server, skills, steps, upgrade, watcher};

#[derive(Parser)]
#[command(
    name = "t3cm",
    version,
    about = "Code Mode MCP server for T3 Code: two tools, search and execute",
    arg_required_else_help = true
)]
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
        /// Append logs to this file instead of stderr; past 10 MB it rolls to <file>.1 at start.
        /// On Windows it also lets go of a console that t3cm alone holds, so a logon task runs
        /// without a window.
        #[arg(long, value_name = "FILE")]
        log_file: Option<PathBuf>,
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
    /// Install the latest release in place and restart the service on it
    #[command(visible_alias = "upgrade")]
    Update {
        /// Only report whether a newer release is out
        #[arg(long)]
        check: bool,
        /// Install this release instead of the latest (vX.Y.Z)
        #[arg(long = "version", value_name = "vX.Y.Z", conflicts_with = "check")]
        tag: Option<String>,
        /// Replace the binary but leave the running service alone
        #[arg(long)]
        no_restart: bool,
        #[command(flatten)]
        port: PortArgs,
        #[command(flatten)]
        state: StateArgs,
    },
    /// Print the generated TypeScript declarations for the catalog
    Types {
        #[arg(long)]
        from: Option<PathBuf>,
        #[command(flatten)]
        state: StateArgs,
    },
    /// Install, update or check the T3 workflow skills this binary carries
    Skills {
        #[command(subcommand)]
        cmd: SkillsCmd,
    },
    #[command(hide = true)]
    Bench {
        #[arg(short, default_value_t = 1000)]
        n: usize,
        #[arg(long)]
        tools: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// Write the skills to ~/.agents/skills, record them in ~/.config/t3-workflow/installed.json, and apply new setup steps
    Install {
        /// Replace skills another tool (npx skills, a copy) installed, and drop them from its lock file
        #[arg(long)]
        take_over: bool,
        /// Overwrite files edited since t3cm installed them
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    /// Bring installed skills to this binary's release, keeping locally edited files, and apply new setup steps
    Update {
        /// Overwrite files edited since t3cm installed them
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    /// Compare the installed skills and setup steps with this binary's release; exits 1 unless all are current
    Status {
        #[arg(long)]
        json: bool,
    },
}

fn run_skills(cmd: SkillsCmd) -> Result<ExitCode> {
    let paths = skills::Paths::from_env()?;
    let env = steps::Env::from_env()?;
    let bundle = skills::Bundle::embedded();
    let (opts, json) = match cmd {
        SkillsCmd::Status { json } => {
            let st = skills::status(&paths, &bundle)?;
            let setup = steps::check(&env, &bundle)?;
            let ok = st.current && !setup.needs_update();
            if json {
                let mut v = serde_json::to_value(&st)?;
                v["current"] = Value::Bool(ok);
                v["steps"] = serde_json::to_value(&setup)?;
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}{}", skills::describe_status(&st), steps::describe_steps(&setup));
            }
            return Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE });
        }
        SkillsCmd::Install { take_over, force, json } => {
            (skills::Options { mode: skills::Mode::Install, force, take_over }, json)
        }
        SkillsCmd::Update { force, json } => {
            (skills::Options { mode: skills::Mode::Update, force, take_over: false }, json)
        }
    };
    let report = skills::sync(&paths, &bundle, opts)?;
    let applied = steps::apply(&env, &bundle)?;
    if json {
        let mut v = serde_json::to_value(&report)?;
        v["steps"] = serde_json::to_value(&applied)?;
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}{}", skills::describe_report(&report), steps::describe_applied(&applied));
    }
    Ok(ExitCode::SUCCESS)
}

fn state_dir(a: &StateArgs) -> PathBuf {
    a.state_dir.clone().unwrap_or_else(platform::state_dir)
}

fn init_logging(log_file: Option<&Path>) -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let fmt = tracing_subscriber::fmt().with_env_filter(filter).with_target(false);
    match log_file {
        Some(path) => {
            fmt.with_writer(std::sync::Mutex::new(open_log(path)?)).with_ansi(false).init();
            LOGS_TO_FILE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        None => fmt.with_writer(std::io::stderr).with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr())).init(),
    }
    Ok(())
}

const LOG_ROLL_BYTES: u64 = 10 << 20;

static LOGS_TO_FILE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn open_log(path: &Path) -> Result<std::fs::File> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_ROLL_BYTES) {
        let mut rolled = path.as_os_str().to_owned();
        rolled.push(".1");
        std::fs::rename(path, &rolled).with_context(|| format!("rolling {}", path.display()))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))
}

#[cfg(windows)]
fn release_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut pids = [0u32; 2];
    if unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) } == 1 {
        unsafe { FreeConsole() };
    }
}

#[cfg(not(windows))]
fn release_own_console() {}

fn not_serving(port: u16) -> String {
    let svc = &platform::SERVICE;
    format!("is t3cm serving on 127.0.0.1:{port}? Start it with `{}`; logs: {}", svc.start, svc.logs)
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
    if let Err(e) = app.wf.check_pools().await {
        tracing::error!(
            "pools: {}; every pick from the pool fails until the config is fixed",
            app.t3.redact(format!("{e:#}"))
        );
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

fn update_check_allowed() -> bool {
    !release::check_disabled(Config::load(&config::default_path()).map(|c| c.update_check).unwrap_or(true))
}

async fn print_update_notice() {
    if !std::io::IsTerminal::is_terminal(&std::io::stderr()) || !update_check_allowed() {
        return;
    }
    let Ok(releases) = release::releases_url() else { return };
    let dir = std::env::var_os("T3CM_STATE_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
    let dir = dir.unwrap_or_else(platform::state_dir);
    let check = release::known_latest(&dir, &releases, release::CLI_FRESH);
    let Ok(Some(latest)) = tokio::time::timeout(std::time::Duration::from_secs(3), check).await else { return };
    if let Some(n) = release::notice(&Version::current(), &latest, &releases) {
        eprintln!("\n{n}");
    }
}

fn spawn_release_checks(app: Arc<App>, state_dir: PathBuf) {
    if !app.config.update_check || release::check_disabled(true) {
        return;
    }
    tokio::spawn(async move {
        let Ok(releases) = release::releases_url() else { return };
        let every = release::SERVICE_EVERY;
        let mut wait = std::time::Duration::from_secs(5);
        if let Some(c) = release::read_cache(&state_dir)
            && let Some(v) = c.version()
        {
            app.wf.set_release(v);
            if let Some(age) = c.age().and_then(|a| a.try_into().ok()) {
                wait = every.saturating_sub(age).max(wait);
            }
        }
        loop {
            tokio::time::sleep(wait).await;
            wait = every;
            match release::latest(&releases).await {
                Ok(v) => {
                    if let Err(e) = release::write_cache(&state_dir, &release::Checked::now(&v)) {
                        tracing::debug!("update check cache: {e:#}");
                    }
                    if v > Version::current() {
                        tracing::info!(latest = %v, "a newer t3cm release is out; run `t3cm update`");
                    }
                    app.wf.set_release(v);
                }
                Err(e) => tracing::debug!("update check: {e:#}"),
            }
        }
    });
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
    let v: Value = req.send().await.with_context(|| not_serving(port))?.json().await?;
    if let Some(e) = v.get("error") {
        bail!("{}", e["message"]);
    }
    let r = &v["result"];
    let text = r["content"][0]["text"].as_str().unwrap_or_default().to_string();
    Ok((r["isError"] == true, text))
}

async fn run(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Serve { port, state, no_watcher, log_file } => {
            init_logging(log_file.as_deref())?;
            if log_file.is_some() {
                release_own_console();
            }
            let (app, t3) = build(&state)?;
            let (listener, addr) =
                server::bind(port.port).await.with_context(|| format!("binding 127.0.0.1:{}", port.port))?;
            tracing::info!(%addr, store = %app.store.path().display(), "t3-codemode serving MCP at /mcp");
            tokio::spawn(warm_up(app.clone(), t3));
            spawn_release_checks(app.clone(), state_dir(&state));
            if !no_watcher {
                watcher::spawn(app.clone());
            }
            server::serve(app.clone(), listener, shutdown_signal()).await?;
            app.upstreams.shutdown().await;
        }
        Cmd::Stdio { state, watcher: with_watcher } => {
            init_logging(None)?;
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
                .with_context(|| not_serving(port.port))?
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
        Cmd::Update { check, tag, no_restart, port, state } => {
            upgrade::run(upgrade::Opts {
                check,
                tag,
                port: port.port,
                restart: !no_restart,
                state_dir: state_dir(&state),
            })
            .await?;
        }
        Cmd::Skills { cmd } => return run_skills(cmd),
        Cmd::Bench { n, tools } => bench::run(n, tools.as_deref()).await?,
    }
    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let shown = matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            let _ = e.print();
            if shown {
                print_update_notice().await;
            }
            return ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2));
        }
    };
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            if LOGS_TO_FILE.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::error!("t3cm: {e:#}");
            }
            eprintln!("t3cm: {e:#}");
            ExitCode::FAILURE
        }
    }
}
