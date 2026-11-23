use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agent_link::{self, Agent, LinkState};
use crate::config::{Config, RemoteMode};
use crate::drift::{self, Home};
use crate::skills::Bundle;
use crate::slots::{self, Enabled, Folders, Inputs, Resolved};
use crate::store::InstanceFolder;
use crate::tailscale::{TailscaleRunner, check_serve_listener, proxy_matches_local_port};

#[derive(Debug, Default)]
pub struct Findings {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

impl Findings {
    pub fn extend(&mut self, other: Findings) {
        self.problems.extend(other.problems);
        self.warnings.extend(other.warnings);
    }
}

pub fn skills(home: &Home, bundle: &Bundle) -> Findings {
    let mut f = Findings::default();
    let d = drift::check(home, bundle);
    if let Some(e) = &d.error {
        f.problems.push(format!("skills: {e}"));
    } else if !d.present {
        f.warnings.push("skills: t3cm has not installed the T3 workflow skills; run t3cm skills install".into());
    } else if !d.current {
        let fix = if d.fixes.is_empty() { "t3cm skills update".to_string() } else { d.fixes.join(", then ") };
        f.problems.push(format!("skills: not current with release {}; fix: {fix}", d.release));
    } else if let Some(p) = &d.problem {
        f.problems.push(format!("setup: {p}"));
    } else if !d.fixes.is_empty() {
        f.warnings.push(format!("setup: {}", d.fixes.join(", then ")));
    }
    f
}

pub fn stale_copies(home: &Home, bundle: &Bundle, codex: &Path) -> Findings {
    let mut f = Findings::default();
    let names = bundle.skills();
    for agent in Agent::all(&home.env) {
        for l in agent_link::check(&agent, &home.paths.dir, &names) {
            if l.state == LinkState::Taken {
                f.problems.push(format!(
                    "skills: {} is a copy, not a link to {}, so {} loads it instead; remove it and run t3cm skills update",
                    l.path.display(),
                    l.target.display(),
                    agent.name
                ));
            }
        }
    }
    for name in &names {
        let path = codex.join("skills").join(name);
        let target = home.paths.dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok() && slots::realpath(&path) != slots::realpath(&target) {
            f.problems.push(format!(
                "skills: {} is a copy, not a link to {}, so Codex loads it instead; remove it",
                path.display(),
                target.display()
            ));
        }
    }
    f
}

fn t3wf_state(folders: &Folders) -> PathBuf {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(p) = var("T3WF_STATE_DIR") {
        return p;
    }
    var("XDG_STATE_HOME").unwrap_or_else(|| folders.home.join(".local").join("state")).join("t3-workflow")
}

pub fn t3wf_leftovers(folders: &Folders) -> Findings {
    let mut f = Findings::default();
    let units = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| folders.home.join(".config"))
        .join("systemd")
        .join("user");
    for unit in ["t3wf-tick.timer", "cbrip-t3wf-tick.timer"] {
        if units.join(unit).exists() {
            f.warnings.push(format!(
                "t3wf: {} is installed; the t3cm service watches links itself",
                units.join(unit).display()
            ));
        }
    }
    let links = t3wf_state(folders).join("v2").join("links.json");
    if let Ok(raw) = std::fs::read(&links)
        && let Ok(v) = serde_json::from_slice::<Value>(&raw)
    {
        let open = v["links"].as_object().map_or(0, |m| m.values().filter(|l| l["closed"] != true).count());
        let queued = v["outbox"].as_array().map_or(0, Vec::len);
        if open > 0 || queued > 0 {
            f.warnings.push(format!(
                "t3wf: {} has {open} open link(s) and {queued} undelivered message(s); keep it until they close",
                links.display()
            ));
        } else {
            f.warnings
                .push(format!("t3wf: {} has no open links or undelivered messages; safe to delete", links.display()));
        }
    }
    f
}

pub fn t3cm_config(path: &Path) -> (Option<Config>, Findings) {
    let mut f = Findings::default();
    match Config::load(path) {
        Ok(c) => (Some(c), f),
        Err(e) => {
            f.problems.push(format!("t3cm config {}: {e:#}", path.display()));
            (None, f)
        }
    }
}

pub fn pools(eff: &Value, cfg: Option<&Config>) -> Findings {
    let mut f = Findings::default();
    let Some(cfg) = cfg else { return f };
    for (role, m) in eff["models"].as_object().into_iter().flatten() {
        if let Some(pool) = m["spec"]["pool"].as_str()
            && !cfg.pools.contains_key(pool)
        {
            f.problems.push(format!("model {role}: pool {pool} is not in t3cm's config"));
        }
    }
    f
}

pub fn legacy_board(cfg: Option<&Config>) -> Findings {
    let mut f = Findings::default();
    let Some(cfg) = cfg else { return f };
    let keys = &cfg.board.legacy_ticket_keys;
    if !keys.is_empty() {
        let names: Vec<String> = keys.iter().map(|k| format!("board.{k}")).collect();
        let ask_replacements: Vec<&str> = keys
            .iter()
            .filter_map(|k| match k.as_str() {
                "instanceId" => Some("board.askInstanceId"),
                "pool" => Some("board.askPool"),
                _ => None,
            })
            .collect();
        let (verb, pron) = if names.len() == 1 {
            ("is a legacy ticket launch setting", "it")
        } else {
            ("are legacy ticket launch settings", "them")
        };
        let base = format!(
            "move {pron} into the models.ticket slot (run setup-t3-workflow, edit ~/.config/t3-workflow/config.json, or pick on the board)"
        );
        let tail = if ask_replacements.is_empty() {
            base
        } else {
            let ask_list = ask_replacements.join(" and ");
            format!("{base}; they no longer apply to the ask box (use {ask_list} for the ask box instead)")
        };
        f.warnings.push(format!("t3cm config: {} {verb}; {tail}", names.join(", ")));
    }
    f
}

pub fn service(health: Option<&Value>, unreachable: Option<&str>) -> Findings {
    let mut f = Findings::default();
    match (health, unreachable) {
        (_, Some(e)) => f.problems.push(format!("service: {e}")),
        (Some(h), None) if h["t3"]["connected"] != true => {
            let why = h["t3"]["lastError"].as_str().unwrap_or("no T3 connection yet");
            f.problems.push(format!("service: up, but not connected to T3 ({why})"));
        }
        _ => {}
    }
    f
}

fn intercepts_board_route(mount: &str) -> bool {
    let trimmed = mount.trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    trimmed == "/api" || trimmed.starts_with("/api/") || trimmed == "/pair" || trimmed.starts_with("/pair/")
}

pub fn remote_board(
    cfg: Option<&Config>,
    local_port: u16,
    health: Option<&Value>,
    ts: &dyn TailscaleRunner,
) -> Findings {
    let mut f = Findings::default();
    let Some(cfg) = cfg else { return f };
    let active = cfg.board.active_remote_config();

    // 1. Check config vs running service `/healthz["remote"]` drift.
    if let Some(health) = health {
        match health.get("remote").and_then(Value::as_object) {
            Some(svc_remote) => {
                let cfg_mode = active.as_ref().map(|(r, _)| r.mode).unwrap_or(RemoteMode::Off);
                let cfg_origin = active.as_ref().map(|(_, o)| o.canonical());
                let cfg_req_pair = active.as_ref().is_some_and(|(r, _)| r.pairing_enabled());
                let svc_mode = svc_remote.get("mode").and_then(Value::as_str).unwrap_or("off");
                let svc_origin = svc_remote.get("origin").and_then(Value::as_str);
                let svc_req_pair = svc_remote.get("requirePairing").and_then(Value::as_bool).unwrap_or(false);
                let allow_drift = match &active {
                    Some((r, _)) if r.mode == RemoteMode::Tailscale && !r.allow_users.is_empty() => {
                        let svc_src = svc_remote.get("allowUsersSource").and_then(Value::as_str);
                        let svc_users: Vec<&str> = svc_remote
                            .get("allowUsers")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        let cfg_users: Vec<&str> = r.allow_users.iter().map(String::as_str).collect();
                        svc_src != Some("config") || svc_users != cfg_users
                    }
                    Some((r, _)) if r.mode == RemoteMode::Tailscale => {
                        svc_remote.get("allowUsersSource").and_then(Value::as_str) == Some("config")
                    }
                    _ => false,
                };
                if svc_mode != cfg_mode.as_str()
                    || svc_origin != cfg_origin.as_deref()
                    || svc_req_pair != cfg_req_pair
                    || allow_drift
                {
                    f.problems.push(format!(
                        "remote: config sets mode {:?} (origin {:?}, requirePairing={cfg_req_pair}), \
                         but the running service reports mode {svc_mode:?} (origin {svc_origin:?}, requirePairing={svc_req_pair}); \
                         restart t3cm to apply",
                        cfg_mode.as_str(),
                        cfg_origin.as_deref()
                    ));
                }
            }
            None => {
                if let Some((remote, origin)) = &active {
                    f.problems.push(format!(
                        "remote: config enables mode {:?} ({}), but the running service does not report remote status \
                         (older t3cm binary?); restart t3cm to apply",
                        remote.mode.as_str(),
                        origin.canonical()
                    ));
                }
            }
        }
    }

    match active {
        Some((remote, origin)) if remote.mode == RemoteMode::Tailscale => {
            if ts.binary().is_none() {
                f.problems
                    .push("remote: board.remote.mode is \"tailscale\", but the tailscale CLI was not found".into());
                return f;
            }
            let mut ipv4 = None;
            match ts.status() {
                Err(e) => f.problems.push(format!("remote: tailscale status failed ({e:#})")),
                Ok(st) => {
                    ipv4 = st.ipv4;
                    if !st.is_running() {
                        let state = st.backend_state.as_deref().unwrap_or("unknown");
                        f.problems.push(format!("remote: Tailscale is not running (BackendState: {state})"));
                    }
                    match &st.dns_name {
                        None => f.problems.push(
                            "remote: Tailscale MagicDNS name is empty; enable MagicDNS in the Tailscale admin console"
                                .into(),
                        ),
                        Some(dns) if dns != &origin.host => f.problems.push(format!(
                            "remote: board.remote.origin host ({}) does not match this machine's Tailscale DNS name ({dns}); \
                             run `t3cm board expose --tailscale` to update",
                            origin.host
                        )),
                        Some(_) => {}
                    }
                    if !st.has_cert_for(&origin.host) {
                        f.problems.push(format!(
                            "remote: Tailscale HTTPS certificates are not enabled for {}; \
                             enable HTTPS Certificates in the Tailscale admin console",
                            origin.host
                        ));
                    }
                    if remote.allow_users.is_empty() && st.host_login.is_none() {
                        f.problems.push(
                            "remote: cannot determine this machine's Tailscale user login and board.remote.allowUsers is empty; \
                             every remote request will be refused"
                                .into(),
                        );
                    }
                }
            }
            match ts.serve_status() {
                Err(e) => f.problems.push(format!("remote: tailscale serve status failed ({e:#})")),
                Ok(serve) => {
                    if let Some(tcp) = serve.tcp.get(&origin.port)
                        && tcp.tcp_forward.is_some()
                    {
                        f.problems.push(format!(
                            "remote: Tailscale Serve port {} uses raw TCP forwarding, which bypasses remote auth; \
                             replace it with `t3cm board expose --tailscale`",
                            origin.port
                        ));
                    }
                    match serve.web_routes_for(&origin.host, origin.port) {
                        None => f.problems.push(format!(
                            "remote: Tailscale Serve is not serving {}; run `t3cm board expose --tailscale`",
                            origin.canonical()
                        )),
                        Some(routes) => {
                            if serve.tcp.get(&origin.port).is_none_or(|t| !t.https) {
                                f.problems.push(format!(
                                    "remote: Tailscale Serve port {} is not serving HTTPS for {}; \
                                     run `t3cm board expose --tailscale`",
                                    origin.port,
                                    origin.canonical()
                                ));
                            }
                            match routes.iter().find(|r| r.mount == "/") {
                                Some(r)
                                    if r.proxy
                                        .as_deref()
                                        .is_some_and(|p| proxy_matches_local_port(p, local_port)) =>
                                {
                                    if let Some(ip) = ipv4
                                        && let Err(problem) = check_serve_listener(ts, ip, &origin.host, origin.port)
                                    {
                                        f.problems.push(format!("remote: {problem}"));
                                    }
                                }
                                Some(r) => {
                                    let got = r
                                        .proxy
                                        .as_deref()
                                        .or(r.path.as_deref())
                                        .or(r.text.as_deref())
                                        .unwrap_or("non-proxy handler");
                                    f.problems.push(format!(
                                        "remote: Tailscale Serve for {} targets {got:?}, expected http://127.0.0.1:{local_port}; \
                                         run `t3cm board expose --tailscale`",
                                        origin.canonical()
                                    ));
                                }
                                None => f.problems.push(format!(
                                    "remote: Tailscale Serve for {} has no `/` proxy handler for http://127.0.0.1:{local_port}; \
                                     run `t3cm board expose --tailscale`",
                                    origin.canonical()
                                )),
                            }
                            for r in routes {
                                if intercepts_board_route(&r.mount)
                                    && !r.proxy.as_deref().is_some_and(|p| proxy_matches_local_port(p, local_port))
                                {
                                    f.problems.push(format!(
                                        "remote: Tailscale Serve mount {:?} on {} intercepts board routes; \
                                         remove it or proxy it to http://127.0.0.1:{local_port}",
                                        r.mount,
                                        origin.canonical()
                                    ));
                                }
                            }
                        }
                    }
                    if serve.is_funnel_enabled(&origin.host, origin.port) {
                        f.problems.push(format!(
                            "remote: Tailscale Funnel is enabled for {}, exposing the board to the public internet; \
                             disable Funnel on port {}",
                            origin.canonical(),
                            origin.port
                        ));
                    }
                    let wanted_key = format!("{}:{}", origin.host, origin.port);
                    for r in serve.routes_targeting_local_port(local_port) {
                        if r.is_raw_tcp && r.port != origin.port {
                            f.problems.push(format!(
                                "remote: Tailscale Serve raw TCP port {} forwards to 127.0.0.1:{local_port}, \
                                 which bypasses remote auth",
                                r.port
                            ));
                        } else if !r.is_raw_tcp && r.host_port.as_deref() != Some(wanted_key.as_str()) {
                            let hp = r.host_port.as_deref().unwrap_or("unknown");
                            if serve.funnel.iter().any(|k| k == hp) {
                                f.problems.push(format!(
                                    "remote: Tailscale Funnel is enabled for {hp} (targeting 127.0.0.1:{local_port}), \
                                     exposing the board to the public internet; disable Funnel on port {}",
                                    r.port
                                ));
                            } else {
                                f.warnings.push(format!(
                                    "remote: Tailscale Serve also exposes 127.0.0.1:{local_port} at {hp}, \
                                     which does not match board.remote.origin ({})",
                                    origin.canonical()
                                ));
                            }
                        }
                    }
                }
            }
        }
        Some((remote, _)) if remote.mode == RemoteMode::Pairing => {
            if ts.binary().is_some()
                && let Ok(serve) = ts.serve_status()
            {
                for r in serve.routes_targeting_local_port(local_port) {
                    if r.is_raw_tcp {
                        f.problems.push(format!(
                            "remote: Tailscale Serve raw TCP port {} forwards to 127.0.0.1:{local_port}, \
                             which bypasses remote auth",
                            r.port
                        ));
                    } else if let Some(hp) = r.host_port.as_deref()
                        && serve.funnel.iter().any(|k| k == hp)
                    {
                        f.problems.push(format!(
                            "remote: Tailscale Funnel is enabled for {hp} (targeting 127.0.0.1:{local_port}), \
                             exposing the board to the public internet; disable Funnel on port {}",
                            r.port
                        ));
                    }
                }
            }
        }
        _ => {
            if ts.binary().is_some()
                && let Ok(serve) = ts.serve_status()
            {
                for r in serve.routes_targeting_local_port(local_port) {
                    if r.is_raw_tcp {
                        f.problems.push(format!(
                            "remote: Tailscale Serve raw TCP port {} forwards to 127.0.0.1:{local_port} \
                             while board.remote is off (bypasses loopback boundary)",
                            r.port
                        ));
                    } else {
                        f.problems.push(format!(
                            "remote: Tailscale Serve exposes 127.0.0.1:{local_port} at {}, \
                             but board.remote is off; run `t3cm board unexpose` or `t3cm board expose --tailscale`",
                            r.host_port.as_deref().unwrap_or("unknown")
                        ));
                    }
                }
            }
        }
    }
    f
}

fn enabled_instances(caps: &Value) -> Option<Vec<String>> {
    let providers = caps["providers"].as_array()?;
    Some(
        providers
            .iter()
            .filter(|p| {
                !p["constraints"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|x| x.to_string().to_lowercase().contains("disabled")))
            })
            .filter_map(|p| p["providerInstanceId"].as_str().map(String::from))
            .collect(),
    )
}

pub fn instances(records: &[InstanceFolder], here: &Folders, root: &Path, caps: Option<&Value>) -> Findings {
    let mut f = Findings::default();
    let enabled = caps.and_then(enabled_instances);
    for r in records {
        if enabled.as_ref().is_some_and(|e| !e.contains(&r.instance)) {
            continue;
        }
        let mut folders = here.clone();
        let folder = PathBuf::from(&r.folder);
        if r.driver == "codex" {
            if slots::realpath(&folder) == slots::realpath(&here.codex) {
                continue;
            }
            folders.codex = folder;
        } else {
            if slots::realpath(&folder) == slots::realpath(&here.claude) {
                continue;
            }
            folders.claude = folder;
        }
        let en = Enabled::load(&folders.claude, root);
        let res = Resolved::build(&Inputs { folders: &folders, root, listed: &[], enabled: &en });
        let eff = json!({"review": res.review, "passes": res.passes, "credentials": res.credentials, "delivery": res.delivery});
        let (problems, warnings) = slots::skill_problems(&eff);
        f.problems.extend(problems.into_iter().map(|p| format!("instance {}: {p}", r.instance)));
        f.warnings.extend(warnings.into_iter().map(|w| format!("instance {}: {w}", r.instance)));
    }
    f
}

pub fn lines(eff: &Value, f: &Findings) -> Vec<String> {
    let mut out = slots::report_lines(eff);
    out.extend(f.warnings.iter().map(|w| format!("warn: {w}")));
    out.extend(f.problems.iter().map(|p| format!("PROBLEM: {p}")));
    out.push(if f.problems.is_empty() {
        "doctor: ok".into()
    } else {
        format!("doctor: {} problem(s)", f.problems.len())
    });
    out
}

pub fn resolve_lines(name: &str, resolved: &Value, cands: &[slots::Skill]) -> Vec<String> {
    let mut out = Vec::new();
    if resolved["status"] == "missing" {
        out.push(format!("{name}: not installed{}", if cands.is_empty() { "" } else { "; other matches:" }));
    }
    for c in cands {
        out.push(format!(
            "{}: {}{} ({})",
            c.name,
            c.path.as_deref().unwrap_or("(listed in this session)"),
            if c.inline { " [inline]" } else { "" },
            c.at
        ));
    }
    out
}
