use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::skills::{Bundle, user_home, write_atomic};
use crate::store::Store;

pub const CATALOG: &str = "setup-t3-workflow/steps.json";
pub const RULE: &str = "t3-workflow/rules/report-filed-issues.md";
pub const RULE_STEP: &str = "rule:filed-issues";
pub const RULE_FILE: &str = "t3-workflow-filed-issues.md";
pub const RULE_START: &str = "<!-- t3-workflow:filed-issues:start -->";
pub const RULE_END: &str = "<!-- t3-workflow:filed-issues:end -->";
pub const HEADER_STEP: &str = "mcp:config-dir-header";
pub const RETIRE_T3WF_STEP: &str = "retire:t3wf";
pub const T3WF_UNITS: [&str; 2] = ["t3wf-tick", "cbrip-t3wf-tick"];
pub const SKILLS_CHANGED_NOTE: &str =
    "The workflow skills changed. Invoke the t3-workflow skill again before your next step.";
pub const MCP_NAME: &str = "t3cm";
pub const CONFIG_DIR_HEADER: &str = "X-T3-Config-Dir";
pub const CONFIG_DIR_VALUE: &str = "${CLAUDE_CONFIG_DIR:-}";
pub const CONFIG_VERSION: u64 = 1;
const CATALOG_VERSION: u32 = 1;
const SETUP_SKILL: &str = "setup-t3-workflow";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Auto,
    Interview,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub since: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(default)]
    pub legacy: bool,
}

#[derive(Deserialize)]
struct Catalog {
    version: u32,
    steps: Vec<Step>,
}

pub fn catalog(bundle: &Bundle) -> Result<Vec<Step>> {
    let raw = bundle.raw(CATALOG).with_context(|| format!("release {} carries no {CATALOG}", bundle.release))?;
    let c: Catalog = serde_json::from_slice(raw).with_context(|| format!("{CATALOG} is not a step list"))?;
    if c.version != CATALOG_VERSION {
        bail!("{CATALOG} is version {}, this t3cm reads version {CATALOG_VERSION}", c.version);
    }
    for s in &c.steps {
        match s.kind {
            Kind::Auto if s.id != RULE_STEP && s.id != HEADER_STEP && s.id != RETIRE_T3WF_STEP => {
                bail!("{CATALOG}: t3cm does not know how to apply step {}", s.id)
            }
            Kind::Interview if s.slot.is_none() => bail!("{CATALOG}: interview step {} names no slot", s.id),
            _ => {}
        }
    }
    Ok(c.steps)
}

pub struct Env {
    pub config: PathBuf,
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub gemini: PathBuf,
    pub claude_json: PathBuf,
    pub claude_config_dir: Option<PathBuf>,
    pub claude_cmd: PathBuf,
    pub systemd_user: PathBuf,
    pub t3wf_state: PathBuf,
    pub skills_dir: PathBuf,
    pub systemctl_cmd: Option<PathBuf>,
}

impl Env {
    pub fn from_env() -> Result<Env> {
        let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = user_home().context("no home folder: set HOME (USERPROFILE on Windows)")?;
        let mut env = Env::under(&home, var("XDG_CONFIG_HOME").as_deref());
        if let Some(p) = var("T3WF_CONFIG") {
            env.config = p;
        }
        if let Some(p) = var("CLAUDE_CONFIG_DIR") {
            env.claude_json = p.join(".claude.json");
            env.claude = p.clone();
            env.claude_config_dir = Some(p);
        }
        if let Some(p) = var("CODEX_HOME") {
            env.codex = p;
        }
        if let Some(p) = var("T3WF_STATE_DIR") {
            env.t3wf_state = p;
        } else if let Some(p) = var("XDG_STATE_HOME") {
            env.t3wf_state = p.join("t3-workflow");
        }
        env.systemctl_cmd = var("T3WF_SYSTEMCTL").or_else(|| Some(PathBuf::from("systemctl")));
        if let Some(p) = var("GEMINI_HOME") {
            env.gemini = p;
        }
        Ok(env)
    }

    pub fn under(home: &Path, config_home: Option<&Path>) -> Env {
        let config = config_home.map(Path::to_path_buf).unwrap_or_else(|| home.join(".config"));
        Env {
            config: config.join("t3-workflow").join("config.json"),
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            gemini: home.join(".gemini"),
            claude_json: home.join(".claude.json"),
            claude_config_dir: None,
            claude_cmd: PathBuf::from("claude"),
            systemd_user: config.join("systemd").join("user"),
            t3wf_state: home.join(".local").join("state").join("t3-workflow"),
            skills_dir: home.join(".agents").join("skills"),
            systemctl_cmd: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RuleState {
    #[serde(rename = "current")]
    Current,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "no agent")]
    NoAgent,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleTarget {
    pub agent: String,
    pub path: PathBuf,
    pub status: RuleState,
    #[serde(skip)]
    new: String,
}

impl RuleTarget {
    pub fn content(&self) -> &str {
        &self.new
    }
}

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

pub fn real(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return PathBuf::from(crate::upgrade::plain(&p));
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) if !dir.as_os_str().is_empty() => real(dir).join(name),
        _ => path.to_path_buf(),
    }
}

fn read_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(lf(&s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn codex_block(text: &str) -> String {
    format!("{RULE_START}\n{}\n{RULE_END}\n", text.trim_end())
}

pub fn spliced(old: &str, block: &str) -> String {
    if let (Some(start), Some(end)) = (old.find(RULE_START), old.find(RULE_END))
        && end > start
    {
        let rest = old[end + RULE_END.len()..].trim_start_matches('\n');
        let tail = if rest.is_empty() { String::new() } else { format!("\n{rest}") };
        return format!("{}{block}{tail}", &old[..start]);
    }
    if old.trim().is_empty() { block.to_string() } else { format!("{}\n\n{block}", old.trim_end_matches('\n')) }
}

pub fn rule_targets(env: &Env, bundle: &Bundle) -> Result<Vec<RuleTarget>> {
    let raw = bundle.raw(RULE).with_context(|| format!("release {} carries no {RULE}", bundle.release))?;
    let text = lf(std::str::from_utf8(raw).with_context(|| format!("{RULE} is not UTF-8"))?);
    let over = env.codex.join("AGENTS.override.md");
    let codex = if over.is_file() { over } else { env.codex.join("AGENTS.md") };
    let mut out = Vec::new();
    for (agent, home, path) in
        [("claude", &env.claude, env.claude.join("rules").join(RULE_FILE)), ("codex", &env.codex, codex)]
    {
        if !home.is_dir() {
            out.push(RuleTarget { agent: agent.into(), path, status: RuleState::NoAgent, new: String::new() });
            continue;
        }
        let path = real(&path);
        let old = read_text(&path)?;
        let (new, ours) = if agent == "claude" {
            (text.clone(), !old.is_empty())
        } else {
            match (old.find(RULE_START), old.find(RULE_END)) {
                (None, None) => {}
                (Some(start), Some(end)) if end > start => {}
                _ => bail!(
                    "{} has a {RULE_START} or {RULE_END} marker without its pair; fix or remove it, then run the command again",
                    path.display()
                ),
            }
            (spliced(&old, &codex_block(&text)), old.contains(RULE_START))
        };
        let status = if old == new {
            RuleState::Current
        } else if ours {
            RuleState::Stale
        } else {
            RuleState::Missing
        };
        out.push(RuleTarget { agent: agent.into(), path, status, new });
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RegState {
    Current,
    Stale,
    NoEntry,
}

#[derive(Clone, Debug, Serialize)]
pub struct Registration {
    pub path: PathBuf,
    pub status: RegState,
    #[serde(skip)]
    entry: Option<Map<String, Value>>,
}

pub fn registration(env: &Env) -> Registration {
    let none = || Registration { path: env.claude_json.clone(), status: RegState::NoEntry, entry: None };
    let Ok(raw) = std::fs::read(&env.claude_json) else { return none() };
    let Ok(v) = serde_json::from_slice::<Value>(&raw) else { return none() };
    let Some(entry) = v["mcpServers"][MCP_NAME].as_object() else { return none() };
    let has = entry
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|h| h.iter().any(|(k, v)| k.eq_ignore_ascii_case(CONFIG_DIR_HEADER) && v.is_string()));
    let status = if has { RegState::Current } else { RegState::Stale };
    Registration { path: env.claude_json.clone(), status, entry: Some(entry.clone()) }
}

fn reg_state(r: &Registration) -> AutoState {
    match r.status {
        RegState::Current => AutoState::Done,
        RegState::Stale => AutoState::Missing,
        RegState::NoEntry => AutoState::NotApplicable,
    }
}

pub fn registered_entry(r: &Registration) -> Option<Value> {
    let mut entry = r.entry.clone()?;
    let mut headers = entry.get("headers").and_then(Value::as_object).cloned().unwrap_or_default();
    headers.retain(|k, _| !k.eq_ignore_ascii_case(CONFIG_DIR_HEADER));
    headers.insert(CONFIG_DIR_HEADER.into(), Value::String(CONFIG_DIR_VALUE.into()));
    entry.insert("headers".into(), Value::Object(headers));
    Some(Value::Object(entry))
}

fn claude(env: &Env, args: &[&str]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(&env.claude_cmd);
    cmd.args(args).stdin(std::process::Stdio::null());
    match &env.claude_config_dir {
        Some(d) => cmd.env("CLAUDE_CONFIG_DIR", d),
        None => cmd.env_remove("CLAUDE_CONFIG_DIR"),
    };
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "claude {} exited {}: {}",
            args.first().map_or("", |a| a),
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("cannot run {}: {e}", env.claude_cmd.display())),
    }
}

enum Reregister {
    Kept(String),
    Lost(String),
}

fn reregister(env: &Env, r: &Registration) -> Result<(), Reregister> {
    let new = registered_entry(r).ok_or_else(|| Reregister::Kept("no t3cm entry".into()))?.to_string();
    let old = Value::Object(r.entry.clone().unwrap_or_default()).to_string();
    claude(env, &["mcp", "remove", "--scope", "user", MCP_NAME]).map_err(Reregister::Kept)?;
    if let Err(e) = claude(env, &["mcp", "add-json", "--scope", "user", MCP_NAME, &new]) {
        return Err(match claude(env, &["mcp", "add-json", "--scope", "user", MCP_NAME, &old]) {
            Ok(()) => Reregister::Kept(format!("{e}; the old entry is back in place")),
            Err(e2) => Reregister::Lost(format!("{e}; restoring the old entry failed too ({e2})")),
        });
    }
    Ok(())
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn json_escape_win_shell(s: &str) -> String {
    // Compact serde_json output has none of these characters outside JSON string
    // literals. Escaping them as `\u00xx` keeps the JSON valid while preventing
    // PowerShell 5.1 Legacy whitespace splitting and cmd.exe metacharacter parsing
    // when `claude` resolves to an exe, .ps1, or .cmd shim.
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            ' ' | '%' | '&' | '|' | '<' | '>' | '^' | '!' | '(' | ')' => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            _ => out.push(c),
        }
    }
    out
}

fn ps_legacy_quote(s: &str) -> String {
    // Under PowerShell 5.1 / Legacy argument passing, embedded `"` is not escaped
    // for CommandLineToArgvW. Pre-escape k backslashes before `"` as 2k+1
    // backslashes + `"` (the input has no whitespace after json_escape_win_shell).
    let mut escaped = String::with_capacity(s.len() + 16);
    let mut slashes = 0usize;
    for c in s.chars() {
        if c == '\\' {
            slashes += 1;
        } else if c == '"' {
            for _ in 0..slashes * 2 + 1 {
                escaped.push('\\');
            }
            escaped.push('"');
            slashes = 0;
        } else {
            for _ in 0..slashes {
                escaped.push('\\');
            }
            slashes = 0;
            escaped.push(c);
        }
    }
    for _ in 0..slashes {
        escaped.push('\\');
    }
    ps_quote(&escaped)
}

pub fn reregister_command_for(env: &Env, r: &Registration, removed: bool, windows: bool) -> String {
    let entry = registered_entry(r).map(|v| v.to_string()).unwrap_or_default();
    if windows {
        let raw = json_escape_win_shell(&entry);
        let legacy = ps_legacy_quote(&raw);
        let modern = ps_quote(&raw);
        let prefix = env
            .claude_config_dir
            .as_ref()
            .map_or(String::new(), |d| format!("$env:CLAUDE_CONFIG_DIR = {}; ", ps_quote(&d.to_string_lossy())));
        let select = format!(
            "$j = if ($PSVersionTable.PSVersion -lt [version]'7.3' -or $PSNativeCommandArgumentPassing -eq 'Legacy' \
             -or ($PSNativeCommandArgumentPassing -eq 'Windows' -and ($c = Get-Command claude -ErrorAction SilentlyContinue) -and $c.Source -match '\\.(cmd|bat)$')) \
             {{ {legacy} }} else {{ {modern} }}; "
        );
        let add = format!("claude mcp add-json --scope user {MCP_NAME} $j");
        if removed {
            format!("{prefix}{select}{add}")
        } else {
            format!(
                "{prefix}{select}claude mcp remove --scope user {MCP_NAME}; if ($? -and $LASTEXITCODE -eq 0) {{ {add} }}"
            )
        }
    } else {
        let json = sh_quote(&entry);
        let prefix = env
            .claude_config_dir
            .as_ref()
            .map_or(String::new(), |d| format!("CLAUDE_CONFIG_DIR={} ", sh_quote(&d.to_string_lossy())));
        let add = format!("{prefix}claude mcp add-json --scope user {MCP_NAME} {json}");
        if removed { add } else { format!("{prefix}claude mcp remove --scope user {MCP_NAME} && {add}") }
    }
}

pub fn reregister_command(env: &Env, r: &Registration, removed: bool) -> String {
    reregister_command_for(env, r, removed, cfg!(windows))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RetireState {
    Present,
    Clean,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct T3wfRetire {
    pub status: RetireState,
    pub units: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shim: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pycache: Option<PathBuf>,
}

pub fn inspect_retire(env: &Env) -> T3wfRetire {
    let mut units = Vec::new();
    let wants = env.systemd_user.join("timers.target.wants");
    for unit in T3WF_UNITS {
        for ext in ["timer", "service"] {
            let p = env.systemd_user.join(format!("{unit}.{ext}"));
            if p.exists() || std::fs::symlink_metadata(&p).is_ok() {
                units.push(p);
            }
        }
        let link = wants.join(format!("{unit}.timer"));
        if link.exists() || std::fs::symlink_metadata(&link).is_ok() {
            units.push(link);
        }
    }
    let shim_path = env.t3wf_state.join("v2").join("bin").join("t3wf-tick");
    let shim = (shim_path.exists() || std::fs::symlink_metadata(&shim_path).is_ok()).then_some(shim_path);
    let pycache_path = env.skills_dir.join("t3-workflow").join("scripts").join("__pycache__");
    let pycache = pycache_path.is_dir().then_some(pycache_path);
    let status = if !units.is_empty() || shim.is_some() || pycache.is_some() {
        RetireState::Present
    } else {
        RetireState::Clean
    };
    T3wfRetire { status, units, shim, pycache }
}

fn retire_state(r: &T3wfRetire) -> AutoState {
    match r.status {
        RetireState::Present => AutoState::Missing,
        RetireState::Clean => AutoState::NotApplicable,
    }
}

enum SysctlOutcome {
    Ok,
    NotInstalled,
    Failed(String),
}

fn run_systemctl(cmd: &Path, args: &[&str]) -> SysctlOutcome {
    match std::process::Command::new(cmd).arg("--user").args(args).stdin(std::process::Stdio::null()).output() {
        Ok(o) if o.status.success() => SysctlOutcome::Ok,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let msg = err.trim();
            SysctlOutcome::Failed(if msg.is_empty() {
                format!("`systemctl --user {}` exited {}", args.join(" "), o.status)
            } else {
                format!("`systemctl --user {}` exited {}: {msg}", args.join(" "), o.status)
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SysctlOutcome::NotInstalled,
        Err(e) => SysctlOutcome::Failed(format!("could not run {}: {e}", cmd.display())),
    }
}

fn apply_retire(env: &Env, r: &T3wfRetire) -> Result<(Vec<PathBuf>, Vec<String>, bool)> {
    let mut removed = Vec::new();
    let mut notes = Vec::new();
    let mut complete = true;
    if !r.units.is_empty() {
        let mut failed_units = BTreeSet::new();
        let mut sysctl_present = false;
        if let Some(sysctl) = &env.systemctl_cmd {
            for unit in T3WF_UNITS {
                let has_unit = r.units.iter().any(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{unit}.")))
                });
                if !has_unit {
                    continue;
                }
                match run_systemctl(sysctl, &["disable", "--now", &format!("{unit}.timer")]) {
                    SysctlOutcome::Ok => sysctl_present = true,
                    SysctlOutcome::NotInstalled => {}
                    SysctlOutcome::Failed(e) => {
                        sysctl_present = true;
                        complete = false;
                        failed_units.insert(unit);
                        notes.push(format!(
                            "could not disable {unit}.timer ({e}); leaving its unit files in place so `t3cm skills update` retries"
                        ));
                    }
                }
            }
        }
        for path in &r.units {
            let blocked = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| failed_units.iter().any(|u| n.starts_with(&format!("{u}."))));
            if blocked {
                continue;
            }
            match std::fs::remove_file(path) {
                Ok(()) => removed.push(path.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
            }
        }
        if sysctl_present
            && let Some(sysctl) = &env.systemctl_cmd
            && let SysctlOutcome::Failed(e) = run_systemctl(sysctl, &["daemon-reload"])
        {
            complete = false;
            notes.push(format!("could not reload systemd user daemon after removing t3wf timers ({e})"));
        }
    }
    if let Some(shim) = &r.shim {
        match std::fs::remove_file(shim) {
            Ok(()) => removed.push(shim.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", shim.display())),
        }
        if let Some(bin_dir) = shim.parent() {
            let _ = std::fs::remove_dir(bin_dir);
        }
    }
    if let Some(pycache) = &r.pycache {
        match std::fs::remove_dir_all(pycache) {
            Ok(()) => removed.push(pycache.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", pycache.display())),
        }
        if let Some(scripts_dir) = pycache.parent() {
            let _ = std::fs::remove_dir(scripts_dir);
        }
    }
    Ok((removed, notes, complete))
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenLinksReport {
    pub warnings: Vec<String>,
    pub t3cm_threads: Vec<String>,
    pub t3wf_threads: Vec<String>,
}

pub fn check_open_links(env: &Env, t3cm_state_dir: &Path) -> OpenLinksReport {
    let mut out = OpenLinksReport::default();
    let mut t3wf_ids = BTreeSet::new();
    let links_path = env.t3wf_state.join("v2").join("links.json");
    if let Ok(raw) = std::fs::read(&links_path)
        && let Ok(v) = serde_json::from_slice::<Value>(&raw)
    {
        let mut labels = Vec::new();
        for l in v["links"].as_object().into_iter().flatten().map(|(_, l)| l).filter(|l| l["closed"] != true) {
            let child = l["child"].as_str().unwrap_or("");
            let parent = l["parent"].as_str().unwrap_or("");
            let label = l["label"].as_str().unwrap_or(child);
            if !child.is_empty() {
                t3wf_ids.insert(child.to_string());
            }
            if !parent.is_empty() && parent != "board" {
                t3wf_ids.insert(parent.to_string());
            }
            labels.push(format!("{label} ({child})"));
        }
        let queued = v["outbox"].as_array().map_or(0, Vec::len);
        if let Some(outbox) = v["outbox"].as_array() {
            for m in outbox {
                if let Some(to) = m["to"].as_str().filter(|s| !s.is_empty() && *s != "board") {
                    t3wf_ids.insert(to.to_string());
                }
            }
        }
        if !labels.is_empty() || queued > 0 {
            let list = if labels.is_empty() { String::new() } else { format!(": {}", labels.join(", ")) };
            out.warnings.push(format!(
                "t3wf registry {} has {} open link(s) and {queued} undelivered message(s){list}",
                links_path.display(),
                labels.len()
            ));
        }
    }
    out.t3wf_threads = t3wf_ids.into_iter().collect();

    let mut t3cm_ids = BTreeSet::new();
    if t3cm_state_dir.join("store.db").is_file()
        && let Ok(store) = Store::open(t3cm_state_dir)
        && let Ok(open) = store.links_open()
    {
        let mut labels = Vec::new();
        for l in open.iter().filter(|l| l.role == "worker" || l.role == "ticket") {
            t3cm_ids.insert(l.child.clone());
            if !l.parent.is_empty() && l.parent != "board" {
                t3cm_ids.insert(l.parent.clone());
            }
            labels.push(format!("{} ({})", l.label, l.child));
        }
        if !labels.is_empty() {
            out.warnings.push(format!("t3cm store has {} open workflow link(s): {}", labels.len(), labels.join(", ")));
        }
    }
    out.t3cm_threads = t3cm_ids.into_iter().collect();
    out
}

pub fn nudge_threads(t3_bin: Option<&Path>, threads: &[String]) -> (Vec<String>, Vec<String>) {
    if threads.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let Some(bin) = t3_bin else {
        return (
            Vec::new(),
            vec![format!(
                "could not nudge {} open workflow thread(s) because the `t3` CLI was not found: {}",
                threads.len(),
                threads.join(", ")
            )],
        );
    };
    let mut nudged = Vec::new();
    let mut notes = Vec::new();
    for id in threads {
        let res = std::process::Command::new(bin)
            .args(["thread", "send", id, "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(SKILLS_CHANGED_NOTE.as_bytes());
                }
                child.wait_with_output()
            });
        match res {
            Ok(o) if o.status.success() => nudged.push(id.clone()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let msg = err.trim();
                notes.push(format!("could not nudge thread {id}: `t3 thread send` exited {} ({msg})", o.status));
            }
            Err(e) => notes.push(format!("could not nudge thread {id}: {e}")),
        }
    }
    (nudged, notes)
}

fn auto_state(id: &str, rule: &[RuleTarget], reg: &Registration, retire: &T3wfRetire) -> AutoState {
    if id == HEADER_STEP {
        reg_state(reg)
    } else if id == RETIRE_T3WF_STEP {
        retire_state(retire)
    } else {
        rule_state(rule)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoState {
    Done,
    Missing,
    NotApplicable,
}

fn rule_state(targets: &[RuleTarget]) -> AutoState {
    if targets.iter().all(|t| t.status == RuleState::NoAgent) {
        AutoState::NotApplicable
    } else if targets.iter().any(|t| matches!(t.status, RuleState::Stale | RuleState::Missing)) {
        AutoState::Missing
    } else {
        AutoState::Done
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigState {
    Absent,
    Legacy,
    Recorded,
    Problem,
}

enum Config {
    Absent,
    Problem(String),
    Found { cfg: Map<String, Value>, recorded: Option<Vec<String>> },
}

fn load_config(path: &Path) -> Config {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::Absent,
        Err(e) => return Config::Problem(format!("cannot read it: {e}")),
    };
    let cfg = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => m,
        Ok(_) => return Config::Problem("the top level must be a JSON object".into()),
        Err(e) => return Config::Problem(format!("not JSON: {e}")),
    };
    if let Some(v) = cfg.get("version")
        && v.as_u64() != Some(CONFIG_VERSION)
    {
        return Config::Problem(format!("config version {v}, and t3cm records steps only in version {CONFIG_VERSION}"));
    }
    if cfg.get("writtenBy").is_some_and(|v| !v.is_string()) {
        return Config::Problem("writtenBy must be a release string".into());
    }
    let recorded = match cfg.get("steps") {
        None => None,
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => {
            Some(a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        }
        Some(_) => return Config::Problem("steps must be a list of step ids".into()),
    };
    Config::Found { cfg, recorded }
}

fn done_ids(steps: &[Step], recorded: &Option<Vec<String>>) -> Vec<String> {
    match recorded {
        Some(r) => r.clone(),
        None => steps.iter().filter(|s| s.legacy).map(|s| s.id.clone()).collect(),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Missing {
    pub id: String,
    pub kind: Kind,
    pub since: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    pub fix: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Steps {
    pub release: String,
    pub config: PathBuf,
    pub config_state: ConfigState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    pub written_by: Option<String>,
    pub recorded: Vec<String>,
    pub missing: Vec<Missing>,
    pub rule: Vec<RuleTarget>,
    pub registration: Registration,
    pub retire_t3wf: T3wfRetire,
}

impl Steps {
    pub fn needs_update(&self) -> bool {
        self.missing.iter().any(|m| m.kind == Kind::Auto)
    }

    pub fn needs_setup(&self) -> impl Iterator<Item = &Missing> {
        self.missing.iter().filter(|m| m.kind == Kind::Interview)
    }
}

pub fn check(env: &Env, bundle: &Bundle) -> Result<Steps> {
    let steps = catalog(bundle)?;
    let rule = rule_targets(env, bundle)?;
    let reg = registration(env);
    let retire = inspect_retire(env);
    let config = load_config(&env.config);
    let (state, problem, written_by, recorded) = match &config {
        Config::Absent => (ConfigState::Absent, None, None, Vec::new()),
        Config::Problem(p) => (ConfigState::Problem, Some(p.clone()), None, Vec::new()),
        Config::Found { cfg, recorded } => (
            if recorded.is_some() { ConfigState::Recorded } else { ConfigState::Legacy },
            None,
            cfg.get("writtenBy").and_then(Value::as_str).map(str::to_string),
            done_ids(&steps, recorded),
        ),
    };
    let mut missing = Vec::new();
    for s in &steps {
        let fix = match s.kind {
            Kind::Auto if auto_state(&s.id, &rule, &reg, &retire) == AutoState::Missing => {
                "t3cm skills update".to_string()
            }
            Kind::Auto => continue,
            Kind::Interview if recorded.contains(&s.id) || state == ConfigState::Problem => continue,
            Kind::Interview if state == ConfigState::Absent => format!("run {SETUP_SKILL}"),
            Kind::Interview => format!("run {SETUP_SKILL}: new slot {}", s.slot.as_deref().unwrap_or(&s.id)),
        };
        missing.push(Missing { id: s.id.clone(), kind: s.kind, since: s.since.clone(), slot: s.slot.clone(), fix });
    }
    Ok(Steps {
        release: bundle.release.clone(),
        config: env.config.clone(),
        config_state: state,
        problem,
        written_by,
        recorded,
        missing,
        rule,
        registration: reg,
        retire_t3wf: retire,
    })
}

pub fn missing_steps(env: &Env, bundle: &Bundle) -> Result<Vec<Missing>> {
    Ok(check(env, bundle)?.missing)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub wrote: Vec<PathBuf>,
    pub registered: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub nudged: Vec<String>,
    pub notes: Vec<String>,
    pub newly_recorded: Vec<String>,
    pub steps: Steps,
}

pub fn apply(env: &Env, bundle: &Bundle) -> Result<Applied> {
    let steps = catalog(bundle)?;
    let mut wrote = Vec::new();
    let mut rule = rule_targets(env, bundle)?;
    if steps.iter().any(|s| s.id == RULE_STEP) {
        for t in rule.iter_mut().filter(|t| matches!(t.status, RuleState::Stale | RuleState::Missing)) {
            write_atomic(&t.path, t.new.as_bytes())?;
            t.status = RuleState::Current;
            wrote.push(t.path.clone());
        }
    }
    let (mut registered, mut removed, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let mut reg = registration(env);
    if steps.iter().any(|s| s.id == HEADER_STEP) && reg.status == RegState::Stale {
        match reregister(env, &reg) {
            Ok(()) => {
                registered.push(reg.path.clone());
                reg = registration(env);
            }
            Err(Reregister::Kept(e)) => notes.push(format!(
                "could not re-register Claude Code's {MCP_NAME} server with the {CONFIG_DIR_HEADER} header ({e}); run: {}",
                reregister_command(env, &reg, false)
            )),
            Err(Reregister::Lost(e)) => bail!(
                "removed Claude Code's {MCP_NAME} server but could not add it back ({e}); Claude Code has no {MCP_NAME} server until you run: {}",
                reregister_command(env, &reg, true)
            ),
        }
    }
    let mut retire = inspect_retire(env);
    let mut retire_complete = true;
    if steps.iter().any(|s| s.id == RETIRE_T3WF_STEP) && retire.status == RetireState::Present {
        let (retire_removed, retire_notes, ok) = apply_retire(env, &retire)?;
        removed = retire_removed;
        notes.extend(retire_notes);
        retire_complete = ok;
        retire = inspect_retire(env);
    }
    let retired_any = !removed.is_empty();
    let mut newly = Vec::new();
    if let Config::Found { mut cfg, recorded } = load_config(&env.config) {
        let mut ids = done_ids(&steps, &recorded);
        for s in steps.iter().filter(|s| s.kind == Kind::Auto) {
            let done = if s.id == RETIRE_T3WF_STEP {
                retired_any && retire_complete && retire.status == RetireState::Clean
            } else {
                auto_state(&s.id, &rule, &reg, &retire) == AutoState::Done
            };
            if done && !ids.contains(&s.id) {
                ids.push(s.id.clone());
                newly.push(s.id.clone());
            }
        }
        if !newly.is_empty() {
            let known: Vec<String> = steps.iter().map(|s| s.id.clone()).filter(|id| ids.contains(id)).collect();
            let extra = ids.iter().filter(|id| !known.contains(id)).cloned();
            let ordered: Vec<Value> = known.iter().cloned().chain(extra).map(Value::String).collect();
            cfg.insert("writtenBy".into(), Value::String(bundle.release.clone()));
            cfg.insert("steps".into(), Value::Array(ordered));
            let mut text = serde_json::to_string_pretty(&Value::Object(cfg))?;
            text.push('\n');
            write_atomic(&real(&env.config), text.as_bytes())?;
        }
    }
    Ok(Applied {
        wrote,
        registered,
        removed,
        nudged: Vec::new(),
        notes,
        newly_recorded: newly,
        steps: check(env, bundle)?,
    })
}

fn setup_lines(s: &Steps, out: &mut String) {
    if let Some(p) = &s.problem {
        out.push_str(&format!(
            "warning: {} is ignored ({p}); fix it, then run {SETUP_SKILL}. Setup steps are not recorded until then\n",
            s.config.display()
        ));
    }
    if s.config_state == ConfigState::Absent && s.needs_setup().next().is_some() {
        out.push_str(&format!(
            "no T3 workflow config at {} yet: run {SETUP_SKILL} to pick the reviewer, passes, delivery and models\n",
            s.config.display()
        ));
    } else if let Some(fix) = new_slots(s) {
        out.push_str(&fix);
        out.push('\n');
    }
}

fn new_slots(s: &Steps) -> Option<String> {
    let slots: Vec<&str> = s.needs_setup().filter_map(|m| m.slot.as_deref()).collect();
    let noun = if slots.len() == 1 { "slot" } else { "slots" };
    (!slots.is_empty()).then(|| format!("run {SETUP_SKILL}: new {noun} {}", slots.join(", ")))
}

pub fn setup_fix(s: &Steps) -> Option<String> {
    match s.config_state {
        ConfigState::Problem => Some(format!("fix {}, then run {SETUP_SKILL}", s.config.display())),
        ConfigState::Absent if s.needs_setup().next().is_some() => Some(format!("run {SETUP_SKILL}")),
        _ => new_slots(s),
    }
}

pub fn describe_steps(s: &Steps) -> String {
    let mut out = String::new();
    if s.needs_update() {
        for t in s.rule.iter().filter(|t| matches!(t.status, RuleState::Stale | RuleState::Missing)) {
            let what = if t.status == RuleState::Stale { "out of date" } else { "missing" };
            out.push_str(&format!(
                "setup step {RULE_STEP}: the filed-issue rule for {} is {what} at {}\n",
                t.agent,
                t.path.display()
            ));
        }
        if s.registration.status == RegState::Stale {
            out.push_str(&format!(
                "setup step {HEADER_STEP}: Claude Code's {MCP_NAME} server in {} does not send {CONFIG_DIR_HEADER}\n",
                s.registration.path.display()
            ));
        }
        if s.retire_t3wf.status == RetireState::Present {
            let mut items = Vec::new();
            items.extend(s.retire_t3wf.units.iter().map(|p| p.display().to_string()));
            items.extend(s.retire_t3wf.shim.iter().map(|p| p.display().to_string()));
            items.extend(s.retire_t3wf.pycache.iter().map(|p| p.display().to_string()));
            out.push_str(&format!("setup step {RETIRE_T3WF_STEP}: t3wf leftovers remain ({})\n", items.join(", ")));
        }
        out.push_str("fix: t3cm skills update\n");
    }
    setup_lines(s, &mut out);
    if out.is_empty() {
        out.push_str("setup steps: none missing\n");
    }
    out
}

pub fn describe_applied(a: &Applied) -> String {
    let mut out = String::new();
    for p in &a.wrote {
        out.push_str(&format!("wrote the filed-issue rule to {}\n", p.display()));
    }
    for p in &a.registered {
        out.push_str(&format!(
            "re-registered Claude Code's {MCP_NAME} server in {} with the {CONFIG_DIR_HEADER} header; new sessions send it\n",
            p.display()
        ));
    }
    for p in &a.removed {
        out.push_str(&format!("removed t3wf leftover {}\n", p.display()));
    }
    if !a.nudged.is_empty() {
        out.push_str(&format!("nudged {} open workflow thread(s): {}\n", a.nudged.len(), a.nudged.join(", ")));
    }
    for n in &a.notes {
        out.push_str(&format!("warning: {n}\n"));
    }
    if !a.newly_recorded.is_empty() {
        out.push_str(&format!(
            "recorded setup steps in {}: {}\n",
            a.steps.config.display(),
            a.newly_recorded.join(", ")
        ));
    }
    setup_lines(&a.steps, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_catalog_parses_and_t3cm_applies_every_auto_step() {
        let steps = catalog(&Bundle::embedded()).unwrap();
        assert!(steps.iter().any(|s| s.id == RULE_STEP && s.kind == Kind::Auto));
        for slot in ["review", "passes", "credentials", "delivery", "models"] {
            assert!(steps.iter().any(|s| s.slot.as_deref() == Some(slot) && s.legacy), "{slot}");
        }
    }

    #[test]
    fn spliced_appends_a_block_and_replaces_one_in_place() {
        let block = codex_block("rule");
        assert_eq!(spliced("", &block), block);
        assert_eq!(spliced("# Mine\n\nKeep.\n", &block), format!("# Mine\n\nKeep.\n\n{block}"));
        let old = format!("# Mine\n\n{RULE_START}\nold\n{RULE_END}\n\n## After\n");
        assert_eq!(spliced(&old, &block), format!("# Mine\n\n{block}\n## After\n"));
        let tail = format!("{RULE_START}\nold\n{RULE_END}\n");
        assert_eq!(spliced(&tail, &block), block);
    }
}
