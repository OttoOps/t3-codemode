use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::config::{Config, RemoteMode, RemoteOrigin, parse_host_header};
use crate::platform;

pub const DEFAULT_HTTPS_PORT: u16 = 47732;
pub const ENV_BIN: &str = "T3CM_TAILSCALE_BIN";
const MACOS_APP_BIN: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const CLI_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailscaleStatus {
    pub backend_state: Option<String>,
    pub dns_name: Option<String>,
    pub cert_domains: Vec<String>,
    pub host_login: Option<String>,
}

impl TailscaleStatus {
    pub fn is_running(&self) -> bool {
        self.backend_state.as_deref() == Some("Running")
    }

    pub fn has_cert_for(&self, host: &str) -> bool {
        let target = normalize_dns_name(host);
        if target.is_empty() {
            return false;
        }
        self.cert_domains.iter().any(|d| normalize_dns_name(d) == target)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeRoute {
    pub mount: String,
    pub proxy: Option<String>,
    pub path: Option<String>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServeTcp {
    pub https: bool,
    pub http: bool,
    pub tcp_forward: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServeStatus {
    pub web: BTreeMap<String, Vec<ServeRoute>>,
    pub tcp: BTreeMap<u16, ServeTcp>,
    pub funnel: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPortRoute {
    pub host_port: Option<String>,
    pub port: u16,
    pub is_raw_tcp: bool,
    pub mount: Option<String>,
    pub target: String,
}

impl ServeStatus {
    pub fn web_routes_for(&self, host: &str, port: u16) -> Option<&[ServeRoute]> {
        let key = format!("{}:{port}", normalize_dns_name(host));
        self.web.get(&key).map(Vec::as_slice)
    }

    pub fn is_funnel_enabled(&self, host: &str, port: u16) -> bool {
        let key = format!("{}:{port}", normalize_dns_name(host));
        self.funnel.iter().any(|k| k == &key)
    }

    pub fn routes_targeting_local_port(&self, local_port: u16) -> Vec<LocalPortRoute> {
        let mut out = Vec::new();
        for (port, tcp) in &self.tcp {
            if let Some(target) = &tcp.tcp_forward
                && tcp_forward_matches_local_port(target, local_port)
            {
                out.push(LocalPortRoute {
                    host_port: None,
                    port: *port,
                    is_raw_tcp: true,
                    mount: None,
                    target: target.clone(),
                });
            }
        }
        for (host_port, routes) in &self.web {
            let port = parse_host_port_key(host_port).map(|(_, p)| p).unwrap_or(443);
            for r in routes {
                if let Some(proxy) = &r.proxy
                    && proxy_matches_local_port(proxy, local_port)
                {
                    out.push(LocalPortRoute {
                        host_port: Some(host_port.clone()),
                        port,
                        is_raw_tcp: false,
                        mount: Some(r.mount.clone()),
                        target: proxy.clone(),
                    });
                }
            }
        }
        out
    }

    pub fn is_http_only_port(&self, port: u16) -> bool {
        self.tcp.get(&port).is_some_and(|t| t.http && !t.https)
    }

    pub fn web_board_mounts_for_host(&self, host: &str, port: u16, local_port: u16) -> Vec<String> {
        let mut mounts = Vec::new();
        if let Some(routes) = self.web_routes_for(host, port) {
            for r in routes {
                if r.proxy.as_deref().is_some_and(|p| proxy_matches_local_port(p, local_port))
                    && !mounts.contains(&r.mount)
                {
                    mounts.push(r.mount.clone());
                }
            }
        }
        mounts
    }

    pub fn board_mounts_for_host(&self, host: &str, port: u16, local_port: u16) -> Vec<String> {
        if self.is_http_only_port(port) {
            return Vec::new();
        }
        self.web_board_mounts_for_host(host, port, local_port)
    }

    pub fn host_port_has_any_handler(&self, host: &str, port: u16) -> bool {
        self.web_routes_for(host, port).is_some_and(|routes| !routes.is_empty())
            || self.tcp.get(&port).is_some_and(|tcp| tcp.tcp_forward.is_some())
    }

    pub fn has_board_root_route(&self, host: &str, https_port: u16, local_port: u16) -> bool {
        self.tcp.get(&https_port).is_some_and(|t| t.https)
            && self.web_routes_for(host, https_port).is_some_and(|routes| {
                routes.iter().any(|r| {
                    r.mount == "/" && r.proxy.as_deref().is_some_and(|p| proxy_matches_local_port(p, local_port))
                })
            })
    }

    pub fn port_has_other_handlers(&self, host: &str, https_port: u16, local_port: u16) -> Option<String> {
        if let Some(tcp) = self.tcp.get(&https_port) {
            if let Some(target) = &tcp.tcp_forward {
                return Some(format!("raw TCP forward to {target}"));
            }
            if tcp.http && !tcp.https {
                return Some("HTTP (non-HTTPS) serve handler".into());
            }
        }
        let wanted = format!("{}:{https_port}", normalize_dns_name(host));
        for (host_port, routes) in &self.web {
            let entry_port = parse_host_port_key(host_port).map(|(_, p)| p);
            if entry_port != Some(https_port) {
                continue;
            }
            for r in routes {
                if host_port == &wanted
                    && r.mount == "/"
                    && r.proxy.as_deref().is_some_and(|p| proxy_matches_local_port(p, local_port))
                {
                    continue;
                }
                let desc =
                    r.proxy.as_deref().or(r.path.as_deref()).or(r.text.as_deref()).unwrap_or("configured handler");
                return Some(format!("{host_port}{} -> {desc}", r.mount));
            }
        }
        None
    }
}

pub trait TailscaleRunner: Send + Sync {
    fn binary(&self) -> Option<PathBuf>;
    fn status(&self) -> Result<TailscaleStatus>;
    fn serve_status(&self) -> Result<ServeStatus>;
    fn serve_enable(&self, https_port: u16, local_port: u16) -> Result<()>;
    fn serve_disable(&self, https_port: u16, mount: &str) -> Result<()>;

    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(300)
    }
}

#[derive(Clone, Debug)]
pub struct TailscaleCli {
    bin: Option<PathBuf>,
}

fn run_command_with_timeout(bin: &Path, args: &[&str], timeout: Duration) -> Result<Output> {
    let deadline = Instant::now() + timeout;
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    platform::ProcessTree::prepare_std(&mut cmd);

    let mut child = cmd.spawn().with_context(|| format!("running `{} {}`", bin.display(), args.join(" ")))?;
    let tree = match platform::ProcessTree::start_std(&child) {
        Ok(tree) => tree,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e).with_context(|| format!("starting `{} {}`", bin.display(), args.join(" ")));
        }
    };

    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = out_tx.send(buf);
    });
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = err_tx.send(buf);
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    tree.kill();
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("`tailscale {}` timed out after {}s", args.join(" "), timeout.as_secs().max(1));
                };
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
            Err(e) => {
                tree.kill();
                let _ = child.kill();
                let _ = child.wait();
                return Err(e).with_context(|| format!("waiting for `{} {}`", bin.display(), args.join(" ")));
            }
        }
    };

    let collect = |rx: mpsc::Receiver<Vec<u8>>| -> Option<Vec<u8>> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(buf) => Some(buf),
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Vec::new()),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
        }
    };

    let Some(stdout) = collect(out_rx) else {
        tree.kill();
        bail!("`tailscale {}` timed out after {}s", args.join(" "), timeout.as_secs().max(1));
    };
    let Some(stderr) = collect(err_rx) else {
        tree.kill();
        bail!("`tailscale {}` timed out after {}s", args.join(" "), timeout.as_secs().max(1));
    };
    Ok(Output { status, stdout, stderr })
}

impl TailscaleCli {
    pub fn from_env() -> Self {
        Self { bin: find_binary() }
    }

    pub fn with_binary(bin: Option<PathBuf>) -> Self {
        Self { bin }
    }

    fn require_bin(&self) -> Result<&Path> {
        self.bin.as_deref().ok_or_else(|| {
            anyhow!("tailscale CLI not found; install Tailscale, add `tailscale` to PATH, or set {ENV_BIN}")
        })
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let bin = self.require_bin()?;
        let out = run_command_with_timeout(bin, args, CLI_TIMEOUT)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            let mut msg = if detail.is_empty() {
                format!("`tailscale {}` exited with {}", args.join(" "), out.status)
            } else {
                format!("`tailscale {}` failed: {detail}", args.join(" "))
            };
            if cfg!(target_os = "linux")
                && (detail.to_ascii_lowercase().contains("access denied")
                    || detail.to_ascii_lowercase().contains("permission denied")
                    || detail.to_ascii_lowercase().contains("operator"))
            {
                msg.push_str(" (hint: run `sudo tailscale set --operator=$USER`)");
            }
            bail!("{msg}");
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl TailscaleRunner for TailscaleCli {
    fn binary(&self) -> Option<PathBuf> {
        self.bin.clone()
    }

    fn status(&self) -> Result<TailscaleStatus> {
        let raw = self.run(&["status", "--json"])?;
        parse_status_json(&raw)
    }

    fn serve_status(&self) -> Result<ServeStatus> {
        let raw = self.run(&["serve", "status", "--json"])?;
        parse_serve_status_json(&raw)
    }

    fn serve_enable(&self, https_port: u16, local_port: u16) -> Result<()> {
        let https_arg = format!("--https={https_port}");
        let target = format!("http://127.0.0.1:{local_port}");
        self.run(&["serve", "--bg", &https_arg, &target])?;
        Ok(())
    }

    fn serve_disable(&self, https_port: u16, mount: &str) -> Result<()> {
        let https_arg = format!("--https={https_port}");
        let path_arg = format!("--set-path={mount}");
        self.run(&["serve", &https_arg, &path_arg, "off"])?;
        Ok(())
    }
}

fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|s| s.to_ascii_lowercase().contains("microsoft"))
}

fn wsl_tailscale_candidates() -> Vec<PathBuf> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(_), Some(dir), Some(_), Some(opts)) = (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if opts.contains("drvfs") {
            let root = Path::new(dir);
            out.push(root.join("Program Files").join("Tailscale").join("tailscale.exe"));
            out.push(root.join("Program Files (x86)").join("Tailscale").join("tailscale.exe"));
        }
    }
    out
}

fn platform_extra_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        out.push(PathBuf::from(MACOS_APP_BIN));
        if let Some(home) = platform::home_dir() {
            out.push(home.join("Applications/Tailscale.app/Contents/MacOS/Tailscale"));
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                out.push(PathBuf::from(base).join("Tailscale").join("tailscale.exe"));
            }
        }
    } else if is_wsl() {
        out.extend(wsl_tailscale_candidates());
    }
    out
}

pub fn find_binary() -> Option<PathBuf> {
    let env_bin = std::env::var_os(ENV_BIN).filter(|v| !v.is_empty());
    let path_env = std::env::var_os("PATH");
    let extras = platform_extra_candidates();
    find_binary_with(env_bin.as_deref(), path_env.as_deref(), &extras, is_wsl())
}

pub fn find_binary_with(
    env_bin: Option<&OsStr>,
    path_env: Option<&OsStr>,
    extra_candidates: &[PathBuf],
    wsl_interop: bool,
) -> Option<PathBuf> {
    if let Some(explicit) = env_bin.filter(|v| !v.is_empty()) {
        let path = PathBuf::from(explicit);
        return platform::is_executable(&path).then_some(path);
    }
    if let Some(path_var) = path_env {
        for dir in std::env::split_paths(path_var) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            for cand in platform::executable_variants(dir.join("tailscale")) {
                if platform::is_executable(&cand) {
                    return Some(cand);
                }
            }
        }
        if wsl_interop {
            for dir in std::env::split_paths(path_var) {
                if dir.as_os_str().is_empty() {
                    continue;
                }
                let cand = dir.join("tailscale.exe");
                if platform::is_executable(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    for cand in extra_candidates {
        if platform::is_executable(cand) {
            return Some(cand.clone());
        }
    }
    None
}

pub fn normalize_dns_name(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

pub fn validate_login(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Tailscale user login must not be empty");
    }
    if trimmed != raw {
        bail!("Tailscale user login {raw:?} must not have leading or trailing whitespace");
    }
    if trimmed.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        bail!("Tailscale user login {raw:?} must not contain whitespace or control characters");
    }
    Ok(trimmed.to_string())
}

pub fn parse_status_json(raw: &str) -> Result<TailscaleStatus> {
    let v: Value = serde_json::from_str(raw).context("parsing `tailscale status --json` output")?;
    let backend_state =
        v.get("BackendState").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let self_obj = v.get("Self").and_then(Value::as_object);
    let dns_name = self_obj
        .and_then(|s| s.get("DNSName"))
        .and_then(Value::as_str)
        .map(normalize_dns_name)
        .filter(|s| !s.is_empty());
    let cert_domains = v
        .get("CertDomains")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter().filter_map(Value::as_str).map(normalize_dns_name).filter(|s| !s.is_empty()).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let host_login = extract_host_login(&v, self_obj);
    Ok(TailscaleStatus { backend_state, dns_name, cert_domains, host_login })
}

fn extract_host_login(root: &Value, self_obj: Option<&Map<String, Value>>) -> Option<String> {
    let user_id = self_obj.and_then(|s| s.get("UserID"))?;
    let id_str = match user_id {
        Value::Number(n) => n.to_string(),
        Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return None,
    };
    let users = root.get("User").and_then(Value::as_object)?;
    let entry = users.get(&id_str).or_else(|| {
        users.values().find(|u| {
            u.get("ID").is_some_and(|id| match id {
                Value::Number(n) => n.to_string() == id_str,
                Value::String(s) => s.trim() == id_str,
                _ => false,
            })
        })
    })?;
    entry
        .get("LoginName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()))
        .map(String::from)
}

fn parse_host_port_key(raw: &str) -> Option<(String, u16)> {
    let (host, port) = parse_host_header(raw.trim())?;
    Some((normalize_dns_name(&host), port.unwrap_or(443)))
}

fn merge_serve_config(v: &Value, status: &mut ServeStatus) {
    if let Some(tcp) = v.get("TCP").and_then(Value::as_object) {
        for (port_str, entry) in tcp {
            let Ok(port) = port_str.trim().parse::<u16>() else {
                continue;
            };
            let https = entry.get("HTTPS").and_then(Value::as_bool).unwrap_or(false);
            let http = entry.get("HTTP").and_then(Value::as_bool).unwrap_or(false);
            let tcp_forward = entry
                .get("TCPForward")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            let slot = status.tcp.entry(port).or_default();
            slot.https |= https;
            slot.http |= http;
            if let Some(fwd) = tcp_forward {
                slot.tcp_forward = Some(fwd);
            }
        }
    }
    if let Some(web) = v.get("Web").and_then(Value::as_object) {
        for (host_port, entry) in web {
            let normalized = match parse_host_port_key(host_port) {
                Some((h, p)) => format!("{h}:{p}"),
                None => host_port.trim().to_ascii_lowercase(),
            };
            let slot = status.web.entry(normalized).or_default();
            if let Some(handlers) = entry.get("Handlers").and_then(Value::as_object) {
                for (mount, h) in handlers {
                    let proxy = h
                        .get("Proxy")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    let path = h
                        .get("Path")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    let text = h.get("Text").and_then(Value::as_str).map(String::from);
                    let route = ServeRoute { mount: mount.clone(), proxy, path, text };
                    if !slot.contains(&route) {
                        slot.push(route);
                    }
                }
            }
        }
    }
    if let Some(funnel) = v.get("AllowFunnel").and_then(Value::as_object) {
        for (host_port, enabled) in funnel {
            if enabled.as_bool() == Some(true) {
                let normalized = match parse_host_port_key(host_port) {
                    Some((h, p)) => format!("{h}:{p}"),
                    None => host_port.trim().to_ascii_lowercase(),
                };
                if !status.funnel.contains(&normalized) {
                    status.funnel.push(normalized);
                }
            }
        }
    }
}

pub fn parse_serve_status_json(raw: &str) -> Result<ServeStatus> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(ServeStatus::default());
    }
    let v: Value = serde_json::from_str(trimmed).context("parsing `tailscale serve status --json` output")?;
    let mut status = ServeStatus::default();
    merge_serve_config(&v, &mut status);
    if let Some(foreground) = v.get("Foreground").and_then(Value::as_object) {
        for entry in foreground.values() {
            merge_serve_config(entry, &mut status);
        }
    }
    Ok(status)
}

pub fn proxy_matches_local_port(proxy: &str, local_port: u16) -> bool {
    let Ok(url) = reqwest::Url::parse(proxy.trim()) else {
        return false;
    };
    if url.scheme() != "http" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1");
    let port_ok = url.port_or_known_default() == Some(local_port);
    let path_ok = matches!(url.path(), "" | "/");
    loopback && port_ok && path_ok && url.query().is_none() && url.fragment().is_none()
}

pub fn tcp_forward_matches_local_port(target: &str, local_port: u16) -> bool {
    let Some((host, port)) = parse_host_header(target.trim()) else {
        return false;
    };
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") && port == Some(local_port)
}

#[derive(Clone, Debug)]
pub struct ExposeOptions {
    pub https_port: u16,
    pub local_port: u16,
    pub allow_users: Vec<String>,
    pub require_pairing: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExposeReport {
    pub mode: String,
    pub origin: String,
    pub https_port: u16,
    pub local_port: u16,
    pub host_login: Option<String>,
    pub allow_users: Vec<String>,
    pub require_pairing: bool,
    pub config_path: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnexposeReport {
    pub previous_mode: String,
    pub disabled_tailscale_ports: Vec<u16>,
    pub config_updated: bool,
    pub config_path: String,
}

fn read_config_object(path: &Path) -> Result<(Map<String, Value>, bool)> {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(o)) => Ok((o, true)),
            Ok(_) => bail!("{}: top level must be a JSON object", path.display()),
            Err(e) => bail!("{} is not valid JSON: {e}", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Map::new(), false)),
        Err(e) => bail!("reading {}: {e}", path.display()),
    }
}

fn write_board_remote(path: &Path, remote_val: Value) -> Result<()> {
    let (mut root, _) = read_config_object(path)?;
    let mut board = match root.get("board") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => bail!("{}: board must be a JSON object", path.display()),
    };
    board.insert("remote".into(), remote_val);
    root.insert("board".into(), Value::Object(board));
    let merged = Value::Object(root);
    Config::from_value(merged.clone()).with_context(|| format!("{} would be invalid after update", path.display()))?;
    let mut text = serde_json::to_string_pretty(&merged)?;
    text.push('\n');
    crate::skills::write_atomic(path, text.as_bytes())
}

pub fn expose_tailscale(ts: &dyn TailscaleRunner, cfg_path: &Path, opts: &ExposeOptions) -> Result<ExposeReport> {
    if opts.https_port == 0 {
        bail!("--https-port must be between 1 and 65535");
    }
    let mut allow_users = Vec::new();
    let mut seen = HashSet::new();
    for raw in &opts.allow_users {
        let clean = validate_login(raw)?;
        if seen.insert(clean.to_ascii_lowercase()) {
            allow_users.push(clean);
        }
    }

    let status = ts.status()?;
    if !status.is_running() {
        let state = status.backend_state.as_deref().unwrap_or("unknown");
        bail!("Tailscale is not running (BackendState: {state}); run `tailscale up` first");
    }
    let dns_name = status
        .dns_name
        .clone()
        .ok_or_else(|| anyhow!("Tailscale MagicDNS name is empty; enable MagicDNS in your Tailscale admin console"))?;
    if !status.has_cert_for(&dns_name) {
        bail!(
            "Tailscale HTTPS certificates are not enabled for {dns_name}; \
             enable HTTPS Certificates in your Tailscale admin console"
        );
    }
    if allow_users.is_empty() && status.host_login.is_none() {
        bail!(
            "could not determine this machine's Tailscale user login from `tailscale status --json`; \
             pass `--allow-user <login>` to set board.remote.allowUsers"
        );
    }

    let origin_str = if opts.https_port == 443 {
        format!("https://{dns_name}")
    } else {
        format!("https://{dns_name}:{}", opts.https_port)
    };
    let origin = RemoteOrigin::parse(&origin_str)?;

    let serve_status = ts.serve_status()?;
    if let Some(conflict) = serve_status.port_has_other_handlers(&dns_name, opts.https_port, opts.local_port) {
        bail!(
            "Tailscale Serve port {} is already in use ({conflict}); \
             pass a different --https-port or remove the existing handler first",
            opts.https_port
        );
    }
    let already_serving_board = serve_status.has_board_root_route(&dns_name, opts.https_port, opts.local_port);

    ts.serve_enable(opts.https_port, opts.local_port)?;

    let mut remote_obj = Map::new();
    remote_obj.insert("mode".into(), json!("tailscale"));
    remote_obj.insert("origin".into(), json!(origin.canonical()));
    if !allow_users.is_empty() {
        remote_obj.insert("allowUsers".into(), json!(allow_users));
    }
    if opts.require_pairing {
        remote_obj.insert("requirePairing".into(), json!(true));
    }
    if let Err(e) = write_board_remote(cfg_path, Value::Object(remote_obj)) {
        if !already_serving_board {
            let _ = ts.serve_disable(opts.https_port, "/");
        }
        return Err(e);
    }

    Ok(ExposeReport {
        mode: "tailscale".into(),
        origin: origin.canonical(),
        https_port: opts.https_port,
        local_port: opts.local_port,
        host_login: status.host_login,
        allow_users,
        require_pairing: opts.require_pairing,
        config_path: cfg_path.display().to_string(),
    })
}

fn shell_quote(raw: &str) -> String {
    if !raw.is_empty()
        && raw.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'@'))
    {
        raw.to_string()
    } else {
        format!("'{}'", raw.replace('\'', "'\\''"))
    }
}

pub fn unexpose_board(
    ts: &dyn TailscaleRunner,
    cfg_path: &Path,
    https_port: Option<u16>,
    local_port: u16,
) -> Result<UnexposeReport> {
    if https_port == Some(0) {
        bail!("--https-port must be between 1 and 65535");
    }
    let cfg = Config::load(cfg_path)?;
    let previous_mode = cfg.board.remote.as_ref().map(|r| r.mode).unwrap_or(RemoteMode::Off);
    let configured_port = (previous_mode == RemoteMode::Tailscale).then(|| {
        cfg.board
            .remote
            .as_ref()
            .and_then(|r| r.origin.as_deref())
            .and_then(|o| RemoteOrigin::parse(o).ok())
            .map(|o| o.port)
            .unwrap_or(DEFAULT_HTTPS_PORT)
    });

    let mut mounts_to_disable: Vec<(u16, String)> = Vec::new();
    let must_check_tailscale = https_port.is_some() || previous_mode == RemoteMode::Tailscale;
    if must_check_tailscale {
        let status = ts.status()?;
        let current_host = status.dns_name.ok_or_else(|| {
            anyhow!("Tailscale MagicDNS name is empty; cannot verify which Serve routes belong to this host")
        })?;
        let serve = ts.serve_status()?;

        let check_non_https_board_route = |port: u16| -> Result<()> {
            let http_mounts = if serve.is_http_only_port(port) {
                serve.web_board_mounts_for_host(&current_host, port, local_port)
            } else {
                Vec::new()
            };
            if !http_mounts.is_empty() {
                let cmds: Vec<String> = http_mounts
                    .iter()
                    .map(|m| format!("`tailscale serve --http={port} --set-path={} off`", shell_quote(m)))
                    .collect();
                bail!(
                    "Tailscale Serve port {port} on {current_host} proxies to http://127.0.0.1:{local_port} \
                     over plain HTTP instead of HTTPS; remove it with {} first",
                    cmds.join(" and ")
                );
            }
            if serve
                .tcp
                .get(&port)
                .and_then(|t| t.tcp_forward.as_deref())
                .is_some_and(|t| tcp_forward_matches_local_port(t, local_port))
            {
                bail!(
                    "Tailscale Serve port {port} uses raw TCP forwarding to 127.0.0.1:{local_port}; \
                     remove it with `tailscale serve --tcp={port} off` first"
                );
            }
            Ok(())
        };

        if let Some(cfg_p) = configured_port {
            check_non_https_board_route(cfg_p)?;
        }

        if let Some(p) = https_port {
            check_non_https_board_route(p)?;
            if let Some(cfg_p) = configured_port
                && cfg_p != p
                && !serve.board_mounts_for_host(&current_host, cfg_p, local_port).is_empty()
            {
                bail!(
                    "Tailscale Serve still exposes the configured board on port {cfg_p}; \
                     run `t3cm board unexpose` without --https-port (or pass --https-port {cfg_p}) \
                     so the configured route is not left exposed"
                );
            }
            let mounts = serve.board_mounts_for_host(&current_host, p, local_port);
            if mounts.is_empty() && serve.host_port_has_any_handler(&current_host, p) {
                bail!(
                    "Tailscale Serve port {p} on {current_host} does not proxy to http://127.0.0.1:{local_port}; \
                     refusing to remove an unrelated handler"
                );
            }
            for mount in mounts {
                mounts_to_disable.push((p, mount));
            }
        } else if let Some(port) = configured_port {
            for mount in serve.board_mounts_for_host(&current_host, port, local_port) {
                mounts_to_disable.push((port, mount));
            }
        }
    } else if previous_mode == RemoteMode::Off
        && ts.binary().is_some()
        && let Ok(status) = ts.status()
        && let Some(current_host) = status.dns_name
        && let Ok(serve) = ts.serve_status()
    {
        for r in serve.routes_targeting_local_port(local_port) {
            let expected_key = format!("{current_host}:{}", r.port);
            if !r.is_raw_tcp
                && r.host_port.as_deref() == Some(expected_key.as_str())
                && let Some(mount) = r.mount
                && !mounts_to_disable.iter().any(|(ep, em)| *ep == r.port && em == &mount)
            {
                mounts_to_disable.push((r.port, mount));
            }
        }
    }

    let mut disabled = Vec::new();
    for (p, mount) in mounts_to_disable {
        ts.serve_disable(p, &mount)?;
        if !disabled.contains(&p) {
            disabled.push(p);
        }
    }

    let (root, exists) = read_config_object(cfg_path)?;
    let had_remote = root.get("board").and_then(Value::as_object).and_then(|b| b.get("remote")).is_some();
    let config_updated = if exists && (had_remote || previous_mode != RemoteMode::Off) {
        write_board_remote(cfg_path, json!({"mode": "off"}))?;
        true
    } else {
        false
    };

    Ok(UnexposeReport {
        previous_mode: previous_mode.as_str().into(),
        disabled_tailscale_ports: disabled,
        config_updated,
        config_path: cfg_path.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_exec(path: &Path) {
        crate::fake::write_executable(path, "#!/bin/sh\n");
    }

    #[test]
    fn find_binary_checks_env_path_wsl_and_platform_extras() {
        let dir = tempfile::tempdir().unwrap();
        let path_dir = dir.path().join("bin");
        let wsl_dir = dir.path().join("winbin");
        let mac_app = dir.path().join("Applications/Tailscale.app/Contents/MacOS/Tailscale");
        let override_bin = dir.path().join("custom/tailscale");

        // 1. Explicit env override takes precedence and does not fall back when missing.
        make_exec(&path_dir.join("tailscale"));
        let missing = dir.path().join("does-not-exist");
        assert_eq!(find_binary_with(Some(missing.as_os_str()), Some(path_dir.as_os_str()), &[], false), None);
        make_exec(&override_bin);
        assert_eq!(
            find_binary_with(Some(override_bin.as_os_str()), Some(path_dir.as_os_str()), &[], false),
            Some(override_bin.clone())
        );

        // 2. PATH finds `tailscale` ahead of extra candidates.
        make_exec(&mac_app);
        assert_eq!(
            find_binary_with(None, Some(path_dir.as_os_str()), std::slice::from_ref(&mac_app), false),
            Some(path_dir.join("tailscale"))
        );

        // 3. Extra candidates (e.g. macOS app bundle or Windows Program Files) used when not on PATH.
        let empty_path = dir.path().join("empty");
        std::fs::create_dir_all(&empty_path).unwrap();
        assert_eq!(
            find_binary_with(None, Some(empty_path.as_os_str()), std::slice::from_ref(&mac_app), false),
            Some(mac_app.clone())
        );

        // 4. WSL interop finds `tailscale.exe` on PATH when Linux `tailscale` is absent.
        make_exec(&wsl_dir.join("tailscale.exe"));
        assert_eq!(find_binary_with(None, Some(wsl_dir.as_os_str()), &[], false), None);
        assert_eq!(find_binary_with(None, Some(wsl_dir.as_os_str()), &[], true), Some(wsl_dir.join("tailscale.exe")));
    }

    #[test]
    fn parse_status_json_extracts_dns_certs_and_host_login() {
        let raw = json!({
            "BackendState": "Running",
            "Self": {
                "DNSName": "BOX.ACME.TS.NET.",
                "UserID": 424242
            },
            "CertDomains": ["box.acme.ts.net."],
            "User": {
                "424242": {
                    "ID": 424242,
                    "LoginName": "alice@acme.example",
                    "DisplayName": "Alice"
                }
            }
        })
        .to_string();
        let st = parse_status_json(&raw).unwrap();
        assert!(st.is_running());
        assert_eq!(st.dns_name.as_deref(), Some("box.acme.ts.net"));
        assert_eq!(st.cert_domains, vec!["box.acme.ts.net"]);
        assert!(st.has_cert_for("BOX.ACME.TS.NET"));
        assert!(!st.has_cert_for("other.acme.ts.net"));
        assert_eq!(st.host_login.as_deref(), Some("alice@acme.example"));

        let missing_certs = json!({
            "BackendState": "Stopped",
            "Self": {"DNSName": "", "UserID": 99},
            "CertDomains": null,
            "User": {}
        })
        .to_string();
        let st2 = parse_status_json(&missing_certs).unwrap();
        assert!(!st2.is_running());
        assert_eq!(st2.dns_name, None);
        assert!(st2.cert_domains.is_empty());
        assert_eq!(st2.host_login, None);
    }

    #[test]
    fn parse_serve_status_json_handles_empty_web_tcp_and_funnel() {
        assert_eq!(parse_serve_status_json("").unwrap(), ServeStatus::default());
        assert_eq!(parse_serve_status_json("{}").unwrap(), ServeStatus::default());

        let raw = json!({
            "TCP": {
                "47732": { "HTTPS": true },
                "443": { "HTTPS": true },
                "9000": { "TCPForward": "127.0.0.1:47731" }
            },
            "Web": {
                "BOX.ACME.TS.NET:47732": {
                    "Handlers": {
                        "/": { "Proxy": "http://127.0.0.1:47731" }
                    }
                },
                "box.acme.ts.net:443": {
                    "Handlers": {
                        "/": { "Proxy": "http://127.0.0.1:3773" }
                    }
                }
            },
            "AllowFunnel": {
                "box.acme.ts.net:47732": true
            }
        })
        .to_string();
        let st = parse_serve_status_json(&raw).unwrap();
        assert!(st.tcp.get(&47732).is_some_and(|t| t.https));
        assert!(st.is_funnel_enabled("box.acme.ts.net", 47732));
        assert!(!st.is_funnel_enabled("box.acme.ts.net", 443));
        assert!(st.port_has_other_handlers("box.acme.ts.net", 47732, 47731).is_none());
        assert!(st.port_has_other_handlers("box.acme.ts.net", 443, 47731).is_some());
        let targeting = st.routes_targeting_local_port(47731);
        assert_eq!(targeting.len(), 2);
        assert!(targeting.iter().any(|r| r.is_raw_tcp && r.port == 9000));
        assert!(targeting.iter().any(|r| !r.is_raw_tcp && r.port == 47732));
    }

    #[cfg(unix)]
    #[test]
    fn run_command_with_timeout_kills_stalled_subprocess() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow-tailscale");
        crate::fake::write_executable(&script, "#!/bin/sh\nexec sleep 5\n");
        let start = Instant::now();
        let err = run_command_with_timeout(&script, &["status", "--json"], Duration::from_millis(150)).unwrap_err();
        assert!(format!("{err:#}").contains("timed out"), "{err:#}");
        assert!(start.elapsed() < Duration::from_secs(3), "took too long: {:?}", start.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn run_command_with_timeout_bounds_output_when_helper_keeps_stdout_open() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("leaking-tailscale");
        crate::fake::write_executable(&script, "#!/bin/sh\nsleep 5 &\nprintf '{\"ok\":true}'\nexit 0\n");
        let start = Instant::now();
        let err = run_command_with_timeout(&script, &["status", "--json"], Duration::from_millis(150)).unwrap_err();
        assert!(format!("{err:#}").contains("timed out"), "{err:#}");
        assert!(start.elapsed() < Duration::from_secs(3), "took too long: {:?}", start.elapsed());
    }
}
