use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::{
    HttpConfig, MAX_NAME_LEN, MAX_TIMEOUT_MS, MIN_SECRET_LEN, MIN_TIMEOUT_MS, RESERVED_JS, RESERVED_NAMES, Reference,
    ResolvedTransport, ResolvedUpstream, SECRET_NAME_HINTS, Secret, StdioConfig, ToolFilter, Transport, UpstreamConfig,
    UpstreamError, is_loopback_host,
};
use crate::types::sanitize_identifier;

const MAX_UPSTREAMS: usize = 32;
const SECRET_HEADERS: &[&str] = &["authorization", "proxy-authorization", "cookie"];

pub fn validate(upstreams: &[UpstreamConfig]) -> Result<()> {
    if upstreams.len() > MAX_UPSTREAMS {
        bail!("at most {MAX_UPSTREAMS} upstreams are allowed, got {}", upstreams.len());
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, up) in upstreams.iter().enumerate() {
        let fail = |field: &str, msg: String| anyhow!("upstreams[{i}] {:?}: {field}: {msg}", up.name);
        validate_name(&up.name).map_err(|e| fail("name", e.to_string()))?;
        if let Some(j) = seen.insert(&up.name, i) {
            return Err(fail("name", format!("duplicate of upstreams[{j}]")));
        }
        check_upstream(up).map_err(|(field, msg)| fail(&field, msg))?;
    }
    Ok(())
}

type FieldError = (String, String);

fn check_upstream(up: &UpstreamConfig) -> Result<(), FieldError> {
    let err = |field: &str, msg: &str| Err((field.to_string(), msg.to_string()));
    match (up.transport, &up.stdio, &up.http) {
        (_, Some(_), Some(_)) => return err("transport", "only one of stdio and http may be present"),
        (Transport::Stdio, Some(s), None) => check_stdio(s)?,
        (Transport::Http, None, Some(h)) => check_http(h)?,
        (Transport::Stdio, _, _) => return err("stdio", "transport \"stdio\" needs a stdio section"),
        (Transport::Http, _, _) => return err("http", "transport \"http\" needs an http section"),
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&up.timeout_ms) {
        return Err((
            "timeout_ms".into(),
            format!("must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS}, got {}", up.timeout_ms),
        ));
    }
    for (key, globs) in [("allow", &up.allow), ("deny", &up.deny)] {
        if let Some(i) = globs.iter().position(|g| g.is_empty()) {
            return err(&format!("{key}[{i}]"), "a glob must not be empty");
        }
    }
    Ok(())
}

fn no_reference(field: &str, value: &str) -> Result<(), FieldError> {
    if value.contains("${") { Err((field.into(), "references (${...}) are not allowed here".into())) } else { Ok(()) }
}

fn check_stdio(s: &StdioConfig) -> Result<(), FieldError> {
    if s.command.trim().is_empty() {
        return Err(("stdio.command".into(), "must not be empty".into()));
    }
    no_reference("stdio.command", &s.command)?;
    for (i, a) in s.args.iter().enumerate() {
        no_reference(&format!("stdio.args[{i}]"), a)?;
    }
    if let Some(cwd) = &s.cwd {
        no_reference("stdio.cwd", cwd)?;
    }
    check_values("stdio.env", &s.env, false)
}

fn masked(raw: &str) -> String {
    parse_references(raw)
        .map(|refs| {
            refs.iter()
                .map(|r| match r {
                    Reference::Literal(s) => s.clone(),
                    _ => "\u{1}".to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn url_host(url: &str) -> Option<(bool, String)> {
    let (secure, rest) = match url.strip_prefix("https://") {
        Some(r) => (true, r),
        None => (false, url.strip_prefix("http://")?),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host_port.split(':').next().unwrap_or_default(),
    };
    Some((secure, host.to_string()))
}

fn check_http(h: &HttpConfig) -> Result<(), FieldError> {
    let refs = parse_references(&h.url).map_err(|e| ("http.url".to_string(), e.to_string()))?;
    let Some((secure, host)) = url_host(&masked(&h.url)) else {
        return Err(("http.url".into(), "must be an http:// or https:// URL".into()));
    };
    if host.is_empty() {
        return Err(("http.url".into(), "must be an http:// or https:// URL".into()));
    }
    if host.contains('\u{1}') {
        return Err(("http.url".into(), "the host must be literal; references may appear in the path or query".into()));
    }
    check_values("http.headers", &h.headers, true)?;
    let credentials = refs.iter().any(is_reference) || h.headers.values().any(|v| has_reference(v));
    if !secure && credentials && !is_loopback_host(&host) && !h.allow_insecure {
        return Err((
            "http.url".into(),
            "would send credentials in cleartext over http:// to a non-loopback host; use https:// or set http.allow_insecure: true".into(),
        ));
    }
    Ok(())
}

fn check_values(section: &str, values: &BTreeMap<String, String>, headers: bool) -> Result<(), FieldError> {
    for (name, value) in values {
        let field = format!("{section}.{name}");
        if name.is_empty() {
            return Err((section.into(), "names must not be empty".into()));
        }
        let refs = parse_references(value).map_err(|e| (field.clone(), e.to_string()))?;
        if is_secret_name(name, headers) && !refs.iter().any(is_reference) {
            return Err((
                field,
                "looks like a secret, so its value must use a reference (${env:NAME} or ${file:path})".into(),
            ));
        }
    }
    Ok(())
}

fn is_secret_name(name: &str, header: bool) -> bool {
    let upper = name.to_ascii_uppercase();
    (header && SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        || SECRET_NAME_HINTS.iter().any(|h| upper.contains(h))
}

fn is_reference(r: &Reference) -> bool {
    !matches!(r, Reference::Literal(_))
}

pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !first_ok || !rest_ok || name.len() > MAX_NAME_LEN {
        bail!(
            "{name:?} must start with a lowercase letter, then up to {} lowercase letters, digits or underscores",
            MAX_NAME_LEN - 1
        );
    }
    if RESERVED_NAMES.contains(&name) {
        bail!("{name:?} is reserved; reserved names are {}", RESERVED_NAMES.join(", "));
    }
    if RESERVED_JS.contains(&name) || sanitize_identifier(name) != name {
        bail!("{name:?} is a JavaScript reserved word or global");
    }
    Ok(())
}

pub fn parse_references(raw: &str) -> Result<Vec<Reference>> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut rest = raw;
    while let Some(pos) = rest.find('$') {
        lit.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        if let Some(after) = tail.strip_prefix("$${") {
            lit.push_str("${");
            rest = after;
        } else if let Some(after) = tail.strip_prefix("${") {
            let end = after.find('}').ok_or_else(|| anyhow!("unterminated reference: ${{ without a closing }}"))?;
            let reference = parse_one(&after[..end])?;
            if !lit.is_empty() {
                out.push(Reference::Literal(std::mem::take(&mut lit)));
            }
            out.push(reference);
            rest = &after[end + 1..];
        } else {
            lit.push('$');
            rest = &tail[1..];
        }
    }
    lit.push_str(rest);
    if !lit.is_empty() {
        out.push(Reference::Literal(lit));
    }
    Ok(out)
}

fn parse_one(body: &str) -> Result<Reference> {
    if let Some(name) = body.strip_prefix("env:") {
        let mut chars = name.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            bail!("invalid reference ${{env:{name}}}: the variable name must match [A-Za-z_][A-Za-z0-9_]*");
        }
        return Ok(Reference::Env(name.into()));
    }
    if let Some(path) = body.strip_prefix("file:") {
        if !(path.starts_with('/') || path.starts_with("~/")) {
            bail!("invalid reference ${{file:{path}}}: the path must be absolute or start with ~/");
        }
        return Ok(Reference::File(PathBuf::from(path)));
    }
    let kind = body.split_once(':').map_or(body, |(k, _)| k);
    bail!("unknown reference kind {kind:?}: use ${{env:NAME}} or ${{file:path}}, or $${{ for a literal ${{")
}

pub fn has_reference(raw: &str) -> bool {
    parse_references(raw).is_ok_and(|refs| refs.iter().any(is_reference))
}

struct Resolved {
    value: String,
    parts: Vec<String>,
}

fn resolve_parts(raw: &str) -> Result<Resolved, UpstreamError> {
    let refs = parse_references(raw).map_err(|e| UpstreamError::Config(e.to_string()))?;
    let mut value = String::new();
    let mut parts = Vec::new();
    for r in refs {
        let part = match r {
            Reference::Literal(s) => {
                value.push_str(&s);
                continue;
            }
            Reference::Env(name) => match std::env::var(&name) {
                Ok(v) if !v.is_empty() => v,
                _ => return Err(UpstreamError::Config(format!("env var {name} is not set"))),
            },
            Reference::File(path) => {
                let shown = path.display().to_string();
                let full = expand_tilde(&shown);
                let text = std::fs::read_to_string(&full)
                    .map_err(|e| UpstreamError::Config(format!("cannot read file {shown}: {}", e.kind())))?;
                text.trim_end_matches(['\r', '\n']).to_string()
            }
        };
        value.push_str(&part);
        parts.push(part);
    }
    Ok(Resolved { value, parts })
}

pub fn resolve_value(raw: &str) -> Result<Secret, UpstreamError> {
    resolve_parts(raw).map(|r| Secret::new(r.value))
}

pub fn resolve(cfg: &UpstreamConfig) -> Result<ResolvedUpstream, UpstreamError> {
    let ns = &cfg.name;
    let named = |e: UpstreamError| UpstreamError::Config(format!("{ns}: {}", e.message()));
    let mut secrets: Vec<Secret> = Vec::new();
    let mut resolve_map = |map: &BTreeMap<String, String>| -> Result<Vec<(String, Secret)>, UpstreamError> {
        let mut out = Vec::new();
        for (k, v) in map {
            let r = resolve_parts(v).map_err(named)?;
            if !r.parts.is_empty() {
                for p in r.parts {
                    push_secret(&mut secrets, p);
                }
                push_secret(&mut secrets, r.value.clone());
            }
            out.push((k.clone(), Secret::new(r.value)));
        }
        Ok(out)
    };
    let transport = match (cfg.transport, &cfg.stdio, &cfg.http) {
        (Transport::Stdio, Some(s), _) => {
            let cwd = match &s.cwd {
                Some(c) => {
                    let dir = expand_tilde(c);
                    if !dir.is_dir() {
                        return Err(UpstreamError::Config(format!("{ns}: cwd {c:?} does not exist")));
                    }
                    Some(dir)
                }
                None => None,
            };
            let command = find_command(&s.command, cwd.as_deref())
                .ok_or_else(|| UpstreamError::Config(format!("{ns}: command {:?} not found", s.command)))?;
            let env = resolve_map(&s.env)?;
            ResolvedTransport::Stdio { command, args: s.args.clone(), env, cwd }
        }
        (Transport::Http, _, Some(h)) => {
            let headers = resolve_map(&h.headers)?;
            let url = resolve_parts(&h.url).map_err(named)?;
            for p in url.parts {
                push_secret(&mut secrets, p);
            }
            let parsed = reqwest::Url::parse(&url.value)
                .map_err(|e| UpstreamError::Config(format!("{ns}: http.url is not a valid URL: {e}")))?;
            for s in url_credentials(&parsed) {
                push_secret(&mut secrets, s);
            }
            ResolvedTransport::Http { url: Secret::new(url.value), headers }
        }
        (t, _, _) => {
            let section = if t == Transport::Stdio { "stdio" } else { "http" };
            return Err(UpstreamError::Config(format!("{ns}: missing the {section} section")));
        }
    };
    Ok(ResolvedUpstream { name: ns.clone(), transport, timeout: Duration::from_millis(cfg.timeout_ms), secrets })
}

fn url_credentials(url: &reqwest::Url) -> Vec<String> {
    let mut out: Vec<String> = url.password().map(String::from).into_iter().collect();
    for (key, value) in url.query_pairs() {
        if is_secret_name(&key, false) && value.len() >= MIN_SECRET_LEN {
            out.push(value.into_owned());
        }
    }
    out
}

fn push_secret(secrets: &mut Vec<Secret>, value: String) {
    if !value.is_empty() && !secrets.iter().any(|s| s.expose() == value) {
        secrets.push(Secret::new(value));
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn expand_tilde(raw: &str) -> PathBuf {
    match (raw, home()) {
        ("~", Some(h)) => h,
        (r, Some(h)) if r.starts_with("~/") => h.join(&r[2..]),
        (r, _) => PathBuf::from(r),
    }
}

fn is_executable(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn find_command(command: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    if command.starts_with('/') || command.starts_with('~') {
        let path = expand_tilde(command);
        return is_executable(&path).then_some(path);
    }
    if command.contains('/') {
        let base = match cwd {
            Some(dir) => dir.to_path_buf(),
            None => std::env::current_dir().ok()?,
        };
        let path = base.join(command);
        return is_executable(&path).then_some(path);
    }
    let path_dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    path_dirs
        .into_iter()
        .filter(|d| !d.as_os_str().is_empty())
        .chain(home().map(|h| h.join(".local/bin")))
        .map(|d| d.join(command))
        .find(|p| is_executable(p))
}

pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

impl ToolFilter {
    pub fn new(allow: &[String], deny: &[String]) -> ToolFilter {
        ToolFilter { allow: allow.to_vec(), deny: deny.to_vec() }
    }

    pub fn allows(&self, tool: &str) -> bool {
        !self.deny.iter().any(|g| glob_match(g, tool)) && self.allow.iter().any(|g| glob_match(g, tool))
    }
}
