use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::platform;
use crate::skills::{self, Bundle};
use crate::slots::{self, CONFIG_VERSION, Enabled, Folders, MODEL_ROLES, Resolver, SLOTS};
use crate::steps::{self, Env, HEADER_STEP, Kind, RETIRE_T3WF_STEP, RULE_STEP, RegState, RetireState, RuleState};

pub const DEFAULT_PORT: u16 = 47731;
pub const REVIEW_SOURCE: &str = "https://github.com/t3dotgg/tstack";
pub const RECORD_KEYS: &[&str] = &["writtenBy", "steps"];

const EFFORT_OPTIONS: &[&str] = &["effort", "reasoningEffort", "thinking_level"];
const SUGGESTED: &[(&str, &[&str])] = &[
    ("review", &["sol-loop"]),
    ("passes", &["unslop", "pr"]),
    ("credentials", &["aws-sso-login"]),
    ("delivery", &["deploy"]),
];

pub fn parse_listed(skills: Option<&str>) -> Vec<String> {
    let Some(raw) = skills else { return Vec::new() };
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().trim_start_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn systemd_user() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let bin = std::env::var_os("T3WF_SYSTEMCTL").filter(|v| !v.is_empty()).unwrap_or_else(|| "systemctl".into());
    std::process::Command::new(bin)
        .args(["--user", "show-environment"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn platform_info() -> Value {
    let system = if cfg!(target_os = "macos") { "darwin" } else { std::env::consts::OS };
    let wsl = cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/sys/kernel/osrelease").is_ok_and(|s| s.to_lowercase().contains("microsoft"));
    let user_units = system == "linux" && systemd_user();
    json!({
        "system": system,
        "wsl": wsl,
        "systemdUser": user_units,
        "t3cmSupported": user_units,
    })
}

pub fn find_t3cm_binary(home: &Path) -> Option<String> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            for cand in platform::executable_variants(dir.join("t3cm")) {
                if platform::is_executable(&cand) {
                    return Some(cand.to_string_lossy().into_owned());
                }
            }
        }
    }
    let local = if cfg!(windows) { platform::installed_exe() } else { home.join(".local").join("bin").join("t3cm") };
    local.is_file().then(|| local.to_string_lossy().into_owned())
}

pub struct T3cmProbe {
    pub json: Value,
    pub port: Option<u16>,
    pub running: bool,
    pub connected: bool,
}

pub async fn probe_t3cm(home: &Path, port_raw: Option<&str>) -> T3cmProbe {
    let binary = find_t3cm_binary(home);
    let port_str = port_raw.map(str::trim).filter(|s| !s.is_empty());
    let port: u16 = match port_str {
        None => DEFAULT_PORT,
        Some(raw) => match raw.parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                return T3cmProbe {
                    json: json!({
                        "binary": binary,
                        "running": false,
                        "connected": false,
                        "error": format!("T3CM_PORT='{raw}' is not a port number"),
                    }),
                    port: None,
                    running: false,
                    connected: false,
                };
            }
        },
    };
    let url = format!("http://127.0.0.1:{port}/");
    let mut out = json!({
        "binary": binary,
        "running": false,
        "connected": false,
        "url": url,
    });
    let client = match reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(e) => {
            out["error"] = json!(e.to_string());
            return T3cmProbe { json: out, port: Some(port), running: false, connected: false };
        }
    };
    let resp = match client.get(format!("http://127.0.0.1:{port}/healthz")).send().await {
        Ok(r) => r,
        Err(e) => {
            out["error"] = json!(e.to_string());
            return T3cmProbe { json: out, port: Some(port), running: false, connected: false };
        }
    };
    let health: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            out["error"] = json!(e.to_string());
            return T3cmProbe { json: out, port: Some(port), running: false, connected: false };
        }
    };
    let connected = health
        .as_object()
        .and_then(|h| h.get("t3"))
        .and_then(Value::as_object)
        .and_then(|t3| t3.get("connected"))
        .and_then(Value::as_bool);
    let Some(connected) = connected else {
        out["error"] = json!("/healthz did not answer like t3cm");
        return T3cmProbe { json: out, port: Some(port), running: false, connected: false };
    };
    out["running"] = json!(true);
    out["version"] = health.get("version").cloned().unwrap_or(Value::Null);
    out["connected"] = json!(connected);
    T3cmProbe { json: out, port: Some(port), running: true, connected }
}

fn model_line(model: &Value) -> Value {
    let id = model.get("id").and_then(Value::as_str);
    let Some(id_str) = id else {
        return model.get("id").cloned().unwrap_or(Value::Null);
    };
    for o in model["options"].as_array().into_iter().flatten() {
        if o["id"].as_str().is_some_and(|k| EFFORT_OPTIONS.contains(&k)) {
            let vals: Vec<String> = o["values"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
                .collect();
            let default = match o.get("default") {
                Some(Value::String(s)) if !s.is_empty() => format!("; default {s}"),
                Some(v) if !v.is_null() && v != &Value::Bool(false) && v != &json!(0) => format!("; default {v}"),
                _ => String::new(),
            };
            return json!(format!("{id_str} (effort {}{default})", vals.join("|")));
        }
    }
    json!(id_str)
}

fn provider_disabled(p: &Value) -> bool {
    p["constraints"].as_array().is_some_and(|c| {
        c.iter().any(|x| {
            let s = match x {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            s.to_lowercase().contains("disabled")
        })
    })
}

pub fn format_providers(caps: &Value) -> Value {
    let mut enabled = Vec::new();
    let mut disabled = Vec::new();
    for p in caps["providers"].as_array().into_iter().flatten() {
        if provider_disabled(p) {
            disabled.push(p.get("providerInstanceId").cloned().unwrap_or(Value::Null));
            continue;
        }
        let models: Vec<Value> = p["models"].as_array().into_iter().flatten().map(model_line).collect();
        let mut entry = json!({
            "provider": p.get("providerInstanceId").cloned().unwrap_or(Value::Null),
            "name": p.get("displayName").cloned().unwrap_or(Value::Null),
            "models": models,
        });
        if let Some(omitted) = p.get("omittedModels").filter(|v| match v {
            Value::Null | Value::Bool(false) => false,
            Value::Number(n) => n.as_f64() != Some(0.0),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            Value::Bool(true) => true,
        }) {
            entry["omittedModels"] = omitted.clone();
        }
        enabled.push(entry);
    }
    json!({"enabled": enabled, "disabled": disabled})
}

fn configured_names(eff: &Value) -> BTreeMap<&'static str, Vec<String>> {
    let mut names: BTreeMap<&'static str, Vec<String>> =
        [("review", Vec::new()), ("passes", Vec::new()), ("credentials", Vec::new()), ("delivery", Vec::new())]
            .into_iter()
            .collect();
    if eff["review"]["skill"].is_object()
        && eff["review"]["source"] != "default"
        && let Some(r) = eff["review"]["skill"]["ref"].as_str()
    {
        names.get_mut("review").unwrap().push(r.to_string());
    }
    for s in eff["passes"]["skills"].as_array().into_iter().flatten() {
        if let Some(r) = s["ref"].as_str() {
            names.get_mut("passes").unwrap().push(r.to_string());
        }
    }
    for c in eff["credentials"]["renewals"].as_array().into_iter().flatten() {
        if let Some(r) = c["skill"]["ref"].as_str() {
            names.get_mut("credentials").unwrap().push(r.to_string());
        }
    }
    if eff["delivery"]["skill"].is_object()
        && let Some(r) = eff["delivery"]["skill"]["ref"].as_str()
    {
        names.get_mut("delivery").unwrap().push(r.to_string());
    }
    names
}

fn resolve_candidates(resolver: &Resolver, name: &str) -> (bool, Vec<Value>) {
    let resolved = resolver.resolve(name);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in resolver.candidates(name) {
        if seen.insert(c.name.clone()) {
            out.push(json!({
                "name": c.name,
                "path": c.path,
                "where": c.at,
                "invoke": if c.inline { "inline" } else { "skill" },
            }));
        }
    }
    (resolved["status"] != "missing", out)
}

pub fn candidates(resolver: &Resolver, eff: &Value, listed: &[String]) -> Value {
    let voices: Vec<String> = listed.iter().filter(|n| slots::bare(n).contains("voice")).cloned().collect();
    let have = configured_names(eff);
    let mut out = Map::new();
    for (slot, suggested) in SUGGESTED {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        let extra: &[String] = if *slot == "passes" { &voices } else { &[] };
        for n in have[*slot]
            .iter()
            .map(String::as_str)
            .chain(suggested.iter().copied())
            .chain(extra.iter().map(String::as_str))
        {
            if seen.insert(n.to_string()) {
                names.push(n.to_string());
            }
        }
        let mut found = Map::new();
        let mut missing = Vec::new();
        let mut others = Map::new();
        for n in names {
            let (ok, cands) = resolve_candidates(resolver, &n);
            if ok {
                found.insert(n, Value::Array(cands));
            } else {
                missing.push(Value::String(n.clone()));
                if !cands.is_empty() {
                    others.insert(n, Value::Array(cands));
                }
            }
        }
        let mut entry = json!({
            "configured": have[*slot],
            "found": found,
            "missing": missing,
        });
        if !others.is_empty() {
            entry["installedInstead"] = Value::Object(others);
        }
        if *slot == "review" && !entry["found"].as_object().is_some_and(|f| f.contains_key("sol-loop")) {
            entry["suggest"] = json!(format!("sol-loop from {REVIEW_SOURCE}"));
        }
        out.insert((*slot).to_string(), entry);
    }
    Value::Object(out)
}

pub fn rule_status(env: &Env, bundle: &Bundle) -> Value {
    match steps::rule_targets(env, bundle) {
        Ok(targets) => {
            let mut m = Map::new();
            for t in targets {
                m.insert(t.agent, json!({"status": t.status, "path": t.path.to_string_lossy()}));
            }
            Value::Object(m)
        }
        Err(e) => json!({"error": format!("{e:#}")}),
    }
}

fn done_steps(cfg: &Map<String, Value>, catalog: &[steps::Step]) -> Vec<String> {
    if let Some(a) = cfg.get("steps").and_then(Value::as_array) {
        return a.iter().filter_map(Value::as_str).map(String::from).collect();
    }
    catalog.iter().filter(|s| s.legacy).map(|s| s.id.clone()).collect()
}

fn ordered_steps(done: &[String], catalog: &[steps::Step]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for s in catalog {
        if done.contains(&s.id) && seen.insert(s.id.clone()) {
            out.push(Value::String(s.id.clone()));
        }
    }
    for id in done {
        if seen.insert(id.clone()) {
            out.push(Value::String(id.clone()));
        }
    }
    out
}

fn rule_done(targets: &[steps::RuleTarget]) -> bool {
    targets.iter().any(|t| t.status == RuleState::Current)
        && targets.iter().all(|t| matches!(t.status, RuleState::Current | RuleState::NoAgent))
}

pub fn steps_status(env: &Env, bundle: &Bundle) -> Result<Value> {
    let catalog = steps::catalog(bundle)?;
    let path = &env.config;
    let mut out = json!({
        "config": path.to_string_lossy(),
        "writtenBy": Value::Null,
        "recorded": [],
        "missing": [],
    });
    let cfg = match slots::load_user_config(path) {
        Ok(c) => c,
        Err(_) => return Ok(out),
    };
    let exists = path.exists();
    let recorded = if exists {
        out["writtenBy"] = cfg.get("writtenBy").cloned().unwrap_or(Value::Null);
        let r = done_steps(&cfg, &catalog);
        out["recorded"] = json!(r);
        r
    } else {
        Vec::new()
    };
    let rule_missing = match steps::rule_targets(env, bundle) {
        Ok(targets) => targets.iter().any(|t| matches!(t.status, RuleState::Missing | RuleState::Stale)),
        Err(_) => true,
    };
    let header_missing = steps::registration(env).status == RegState::Stale;
    let retire_missing = steps::inspect_retire(env).status == RetireState::Present;
    let mut missing = Vec::new();
    for s in &catalog {
        if s.kind == Kind::Interview && !recorded.contains(&s.id) {
            missing.push(json!({"id": s.id, "kind": "interview", "slot": s.slot}));
        } else if s.kind == Kind::Auto {
            let is_missing = if s.id == HEADER_STEP {
                header_missing
            } else if s.id == RETIRE_T3WF_STEP {
                retire_missing
            } else {
                rule_missing
            };
            if is_missing {
                missing.push(json!({"id": s.id, "kind": "auto"}));
            }
        }
    }
    out["missing"] = Value::Array(missing);
    Ok(out)
}

pub struct DetectInputs<'a> {
    pub folders: &'a Folders,
    pub env: &'a Env,
    pub bundle: &'a Bundle,
    pub root: &'a Path,
    pub listed: &'a [String],
    pub caps: &'a slots::Caps,
    pub providers: Value,
    pub t3cm: Value,
}

pub fn detect_json(i: &DetectInputs) -> Result<Value> {
    let enabled = Enabled::load(&i.folders.claude, i.root);
    let resolved = slots::Resolved::build(&slots::Inputs {
        folders: i.folders,
        root: i.root,
        listed: i.listed,
        enabled: &enabled,
    });
    let eff = resolved.all(i.caps);
    let resolver = Resolver::new(&slots::search_roots(i.folders, i.root, &enabled), i.listed);
    Ok(json!({
        "platform": platform_info(),
        "config": {
            "path": i.folders.config.to_string_lossy(),
            "exists": resolved.config.is_some(),
            "problems": resolved.config_problems,
            "repoProblems": resolved.repo_problems,
        },
        "summary": slots::report_lines(&eff),
        "slots": candidates(&resolver, &eff, i.listed),
        "providers": i.providers,
        "rule": rule_status(i.env, i.bundle),
        "steps": steps_status(i.env, i.bundle)?,
        "t3cm": i.t3cm,
    }))
}

fn skill_refs(cfg: &Map<String, Value>) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    if let Some(r) = cfg.get("review").and_then(Value::as_str) {
        refs.push(("review".into(), r.to_string()));
    }
    for (i, r) in cfg.get("passes").and_then(Value::as_array).into_iter().flatten().enumerate() {
        if let Some(s) = r.as_str() {
            refs.push((format!("passes[{i}]"), s.to_string()));
        }
    }
    for (i, c) in cfg.get("credentials").and_then(Value::as_array).into_iter().flatten().enumerate() {
        if let Some(s) = c.as_object().and_then(|o| o.get("skill")).and_then(Value::as_str) {
            refs.push((format!("credentials[{i}]"), s.to_string()));
        }
    }
    if let Some(s) = cfg
        .get("delivery")
        .and_then(Value::as_object)
        .and_then(|d| d.get("skill"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        refs.push(("delivery".into(), s.to_string()));
    }
    refs
}

fn merge_config(old: &Map<String, Value>, new: &Map<String, Value>, unset: &[String]) -> Result<Map<String, Value>> {
    let mut cfg = old.clone();
    for (key, value) in new {
        if key == "models"
            && let Some(new_models) = value.as_object()
        {
            let mut base = cfg.get("models").and_then(Value::as_object).cloned().unwrap_or_default();
            for (r, s) in new_models {
                if s.is_null() {
                    base.remove(r);
                } else {
                    base.insert(r.clone(), s.clone());
                }
            }
            cfg.insert("models".into(), Value::Object(base));
        } else {
            cfg.insert(key.clone(), value.clone());
        }
    }
    for key in unset {
        let (slot, role) = match key.split_once('.') {
            Some((s, r)) => (s, Some(r)),
            None => (key.as_str(), None),
        };
        let bad_role = role.is_some_and(|r| slot != "models" || !MODEL_ROLES.contains(&r));
        if !SLOTS.contains(&slot) || bad_role {
            bail!("--unset {key}: name a slot ({}) or models.ticket or models.worker", SLOTS.join(", "));
        }
        if let Some(r) = role {
            if let Some(models) = cfg.get_mut("models").and_then(Value::as_object_mut) {
                models.remove(r);
            }
        } else {
            cfg.remove(slot);
        }
    }
    cfg.remove("version");
    let mut out = Map::new();
    out.insert("version".into(), json!(CONFIG_VERSION));
    for (k, v) in cfg {
        out.insert(k, v);
    }
    Ok(out)
}

fn asked_slots(asked: &[String], new: &Map<String, Value>, unset: &[String]) -> Result<HashSet<String>> {
    let names: Vec<String> =
        asked.iter().flat_map(|v| v.split(',')).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let bad: Vec<&str> = names.iter().map(String::as_str).filter(|n| !SLOTS.contains(n)).collect();
    if !bad.is_empty() {
        bail!("--asked {}: name a slot ({})", bad.join(", "), SLOTS.join(", "));
    }
    let mut out: HashSet<String> = names.into_iter().collect();
    for k in new.keys() {
        if SLOTS.contains(&k.as_str()) {
            out.insert(k.clone());
        }
    }
    for u in unset {
        let slot = u.split('.').next().unwrap_or_default();
        out.insert(slot.to_string());
    }
    Ok(out)
}

fn save_config(path: &Path, cfg: &Map<String, Value>) -> Result<()> {
    let mut text = serde_json::to_string_pretty(&Value::Object(cfg.clone()))?;
    text.push('\n');
    skills::write_atomic(path, text.as_bytes()).map_err(|e| anyhow::anyhow!("not written: {e:#}"))
}

pub struct WriteOpts {
    pub repo: PathBuf,
    pub listed: Vec<String>,
    pub unset: Vec<String>,
    pub replace: bool,
    pub asked: Vec<String>,
}

pub struct WriteResult {
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

pub fn write_config(
    folders: &Folders,
    env: &Env,
    bundle: &Bundle,
    stdin_json: &str,
    opts: &WriteOpts,
) -> Result<WriteResult> {
    let new_val: Value = serde_json::from_str(stdin_json).map_err(|e| anyhow::anyhow!("stdin is not JSON: {e}"))?;
    let Some(new) = new_val.as_object() else {
        bail!("stdin must be a JSON object of slot settings");
    };
    let kept: Vec<&str> = RECORD_KEYS.iter().copied().filter(|k| new.contains_key(*k)).collect();
    if !kept.is_empty() {
        bail!("stdin sets {}; setup records those itself", kept.join(", "));
    }
    let path = steps::real(&folders.config);
    let old = if opts.replace {
        Map::new()
    } else {
        slots::load_user_config(&path).map_err(|e| anyhow::anyhow!("{e}; fix it, or pass --replace to overwrite it"))?
    };
    if let Some(v) = old.get("version")
        && v.as_u64() != Some(CONFIG_VERSION)
    {
        bail!(
            "{} is config version {}; pass --replace to start a version {CONFIG_VERSION} file",
            path.display(),
            slots::py_repr(v)
        );
    }
    let mut cfg = merge_config(&old, new, &opts.unset)?;
    let mut problems = slots::validate_config(&cfg, "new config");
    if new.contains_key("delivery")
        && let Some(d) = cfg.get("delivery").and_then(Value::as_object)
        && d.get("mode").and_then(Value::as_str) == Some("tag-deploy")
        && d.get("skill").and_then(Value::as_str).is_none_or(str::is_empty)
    {
        problems.push("delivery: tag-deploy needs a deploy skill".into());
    }
    if let Some(models) = cfg.get("models").and_then(Value::as_object) {
        for (r, spec) in models {
            if let Some(o) = spec.as_object()
                && !o.contains_key("model")
                && !o.contains_key("pool")
            {
                problems.push(format!("models.{r}: names no model"));
            }
        }
    }
    if !problems.is_empty() {
        bail!("not written:\n{}", problems.join("\n"));
    }
    let enabled = Enabled::load(&folders.claude, &opts.repo);
    let resolver = Resolver::new(&slots::search_roots(folders, &opts.repo, &enabled), &opts.listed);
    let mut subset = Map::new();
    for k in new.keys() {
        if let Some(v) = cfg.get(k) {
            subset.insert(k.clone(), v.clone());
        }
    }
    let changed: HashSet<(String, String)> = skill_refs(&subset).into_iter().collect();
    let mut warnings = Vec::new();
    for (at, reference) in skill_refs(&cfg) {
        let hit = resolver.resolve(&reference);
        if hit["status"] == "ambiguous" {
            let others: Vec<&str> = hit["others"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let text = format!(
                "{at}: {reference} matches {} and {}; record one full name",
                hit["name"].as_str().unwrap_or_default(),
                others.join(", ")
            );
            if changed.contains(&(at, reference)) {
                problems.push(text);
            } else {
                warnings.push(text);
            }
        } else if hit["status"] == "missing" {
            warnings.push(format!("{at}: {reference} is not installed yet; the roles skip it until it is"));
        }
    }
    if !problems.is_empty() {
        bail!("not written:\n{}", problems.join("\n"));
    }
    let catalog = steps::catalog(bundle)?;
    let mut done = if path.exists() && !opts.replace { done_steps(&old, &catalog) } else { Vec::new() };
    let asked = asked_slots(&opts.asked, new, &opts.unset)?;
    let mut asked_sorted: Vec<String> = asked.into_iter().collect();
    asked_sorted.sort();
    for slot in asked_sorted {
        if let Some(s) = catalog.iter().find(|s| s.kind == Kind::Interview && s.slot.as_deref() == Some(slot.as_str()))
        {
            done.push(s.id.clone());
        }
    }
    if let Ok(targets) = steps::rule_targets(env, bundle)
        && rule_done(&targets)
    {
        done.push(RULE_STEP.into());
    }
    cfg.insert("steps".into(), Value::Array(ordered_steps(&done, &catalog)));
    if !bundle.release.is_empty() {
        cfg.insert("writtenBy".into(), Value::String(bundle.release.clone()));
    } else {
        cfg.remove("writtenBy");
    }
    save_config(&path, &cfg)?;
    Ok(WriteResult { path, warnings })
}

pub fn apply_rule(env: &Env, bundle: &Bundle) -> Result<Vec<String>> {
    let plan = steps::rule_targets(env, bundle)?;
    if plan.iter().all(|p| p.status == RuleState::NoAgent) {
        bail!("found neither a Claude Code nor a Codex config folder; install one of them first");
    }
    let mut lines = Vec::new();
    for p in &plan {
        let home = if p.agent == "claude" { &env.claude } else { &env.codex };
        match p.status {
            RuleState::NoAgent => lines.push(format!("{}: skipped, {} does not exist", p.agent, home.display())),
            RuleState::Current => lines.push(format!("{}: {} is current", p.agent, p.path.display())),
            RuleState::Missing | RuleState::Stale => {
                skills::write_atomic(&p.path, p.content().as_bytes())
                    .with_context(|| format!("not written: {}", p.path.display()))?;
                lines.push(format!("{}: wrote {}", p.agent, p.path.display()));
            }
        }
    }
    let path = steps::real(&env.config);
    if path.exists()
        && let Ok(mut cfg) = slots::load_user_config(&path)
        && cfg.get("version").and_then(Value::as_u64).unwrap_or(CONFIG_VERSION) == CONFIG_VERSION
        && slots::validate_config(&cfg, "config.json").is_empty()
    {
        let catalog = steps::catalog(bundle)?;
        let mut done = done_steps(&cfg, &catalog);
        if !done.iter().any(|s| s == RULE_STEP) {
            done.push(RULE_STEP.into());
            cfg.insert("steps".into(), Value::Array(ordered_steps(&done, &catalog)));
            if !bundle.release.is_empty() {
                cfg.insert("writtenBy".into(), Value::String(bundle.release.clone()));
            } else {
                cfg.remove("writtenBy");
            }
            save_config(&path, &cfg)?;
            lines.push(format!("recorded {RULE_STEP} in {}", path.display()));
        }
    }
    Ok(lines)
}
