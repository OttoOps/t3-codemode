use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{gh_name, is_jira_project_key, is_tracker_host};
use crate::field::{self, invalid, within};
use crate::pools::PoolConfig;
use crate::upstream::UpstreamConfig;

pub const UP_KINDS: &[&str] = &["note", "event", "gate", "blocker", "report", "handoff", "closed"];
pub const CLOSING: &[&str] = &["handoff", "closed"];
const EXTRA_WAKE_KINDS: &[&str] = &["escalate", "reply"];

pub const MIN_TIMEOUT_MS: u64 = 100;
pub const MAX_LIMIT_HOLD_MINUTES: u64 = 7 * 24 * 60;
pub const LEGACY_BOARD_TICKET_KEYS: &[&str] = &["instanceId", "ticketModel", "ticketEffort", "pool"];
const BOARD_ASK_KEYS: &[&str] = &["askInstanceId", "askModel", "askEffort", "askPool"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]

pub struct LinkParams {
    pub down: i64,
    pub miss: i64,
    pub stall: i64,
}

impl LinkParams {
    pub fn validate(&self) -> Result<()> {
        for (k, v) in [("down", self.down), ("miss", self.miss), ("stall", self.stall)] {
            if v < 1 {
                return Err(invalid(k, format!("{k} must be a positive integer, got {v}")));
            }
        }
        if self.stall <= self.down {
            return Err(invalid("stall", format!("stall ({}) must exceed down ({})", self.stall, self.down)));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildPolicy {
    pub down: Option<i64>,
    pub miss: Option<i64>,
    pub stall: Option<i64>,
    pub wake: Option<Vec<String>>,
    pub body_cap: Option<usize>,
    pub relay: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkPolicy {
    pub wake: Vec<String>,
    pub body_cap: usize,
    pub relay: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    #[serde(default)]
    pub children: BTreeMap<String, ChildPolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardConfig {
    pub instance_id: String,
    #[serde(default)]
    pub pool: Option<String>,
    pub ticket_model: String,
    pub ticket_effort: Option<String>,
    #[serde(default)]
    pub ask_instance_id: Option<String>,
    #[serde(default)]
    pub ask_pool: Option<String>,
    pub ask_model: String,
    pub ask_effort: Option<String>,
    pub runtime_mode: Option<String>,
    pub ask_poll_secs: u64,
    pub ask_timeout_minutes: u64,
    pub feed: usize,
    pub settle_after_hours: u64,
    pub reconcile_active_minutes: u64,
    pub reconcile_idle_minutes: u64,
    pub open_links: OpenLinks,
    #[serde(default)]
    pub layout: BoardLayout,
    #[serde(default)]
    pub remote: Option<BoardRemoteConfig>,
    #[serde(skip)]
    pub legacy_ticket_keys: Vec<String>,
    #[serde(skip)]
    pub ask_configured: bool,
}

impl BoardConfig {
    pub fn has_legacy_ticket_keys(&self) -> bool {
        !self.legacy_ticket_keys.is_empty()
    }

    pub fn has_legacy_ticket_key(&self, key: &str) -> bool {
        self.legacy_ticket_keys.iter().any(|k| k == key)
    }

    pub(crate) fn copy_ask_from(&mut self, other: &BoardConfig) {
        self.ask_instance_id.clone_from(&other.ask_instance_id);
        self.ask_pool.clone_from(&other.ask_pool);
        self.ask_model.clone_from(&other.ask_model);
        self.ask_effort.clone_from(&other.ask_effort);
        self.ask_configured = other.ask_configured;
    }

    pub fn active_remote(&self) -> Option<(RemoteMode, RemoteOrigin)> {
        self.remote.as_ref().and_then(BoardRemoteConfig::active)
    }

    pub fn active_remote_config(&self) -> Option<(&BoardRemoteConfig, RemoteOrigin)> {
        self.remote.as_ref().and_then(|r| r.active().map(|(_, origin)| (r, origin)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteMode {
    #[default]
    Off,
    Pairing,
    Tailscale,
}

impl RemoteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RemoteMode::Off => "off",
            RemoteMode::Pairing => "pairing",
            RemoteMode::Tailscale => "tailscale",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardRemoteConfig {
    #[serde(default)]
    pub mode: RemoteMode,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_users: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_pairing: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings_users: Vec<String>,
}

impl BoardRemoteConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(origin) = &self.origin {
            RemoteOrigin::parse(origin).map_err(|e| within(e, "board.remote.origin", "board.remote.origin"))?;
        }
        if self.mode != RemoteMode::Off && self.origin.is_none() {
            return Err(invalid(
                "board.remote.origin",
                format!("board.remote.origin is required when board.remote.mode is {:?}", self.mode.as_str()),
            ));
        }
        for (i, user) in self.allow_users.iter().enumerate() {
            crate::tailscale::validate_login(user)
                .map_err(|e| within(e, &format!("board.remote.allowUsers.{i}"), "board.remote.allowUsers"))?;
        }
        for (i, user) in self.settings_users.iter().enumerate() {
            let at = format!("board.remote.settingsUsers.{i}");
            crate::tailscale::validate_login(user).map_err(|e| within(e, &at, "board.remote.settingsUsers"))?;
            if !self.allow_users.is_empty() && !self.allow_users.iter().any(|u| u.eq_ignore_ascii_case(user)) {
                return Err(invalid(
                    at,
                    format!("board.remote.settingsUsers: {user:?} is not in board.remote.allowUsers"),
                ));
            }
        }
        Ok(())
    }

    pub fn active(&self) -> Option<(RemoteMode, RemoteOrigin)> {
        if self.mode == RemoteMode::Off {
            return None;
        }
        let origin = self.origin.as_deref().and_then(|o| RemoteOrigin::parse(o).ok())?;
        Some((self.mode, origin))
    }

    pub fn pairing_enabled(&self) -> bool {
        match self.mode {
            RemoteMode::Off => false,
            RemoteMode::Pairing => true,
            RemoteMode::Tailscale => self.require_pairing,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOrigin {
    pub host: String,
    pub port: u16,
}

impl RemoteOrigin {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw != raw.trim() {
            bail!("must be an https:// origin with no surrounding whitespace");
        }
        let Some(authority) = raw.strip_prefix("https://") else {
            bail!("must use https://, got {raw:?}");
        };
        if authority.is_empty()
            || authority.contains(['/', '?', '#', '@'])
            || authority.bytes().any(|b| b.is_ascii_whitespace())
        {
            bail!("must be an https:// origin with no path, query, fragment, or credentials, got {raw:?}");
        }
        let Some((host, parsed_port)) = parse_host_header(authority) else {
            bail!("invalid host or port in {raw:?}");
        };
        let url = reqwest::Url::parse(raw).with_context(|| format!("invalid https:// origin {raw:?}"))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            bail!("must be an https:// origin with no path, query, fragment, or credentials, got {raw:?}");
        }
        if !host.starts_with('[') && url.host_str() != Some(host.as_str()) {
            bail!("host must be a canonical hostname or IP address, got {raw:?}");
        }
        let port = parsed_port.unwrap_or(443);
        Ok(Self { host, port })
    }

    pub fn canonical(&self) -> String {
        if self.port == 443 { format!("https://{}", self.host) } else { format!("https://{}:{}", self.host, self.port) }
    }

    pub fn matches_host(&self, host_header: &str) -> bool {
        let Some((host, port)) = parse_host_header(host_header) else {
            return false;
        };
        host == self.host && port.unwrap_or(443) == self.port
    }

    pub fn matches_origin(&self, origin_header: &str) -> bool {
        Self::parse(origin_header).is_ok_and(|o| o == *self)
    }
}

pub fn parse_host_header(raw: &str) -> Option<(String, Option<u16>)> {
    if raw.is_empty() || raw.bytes().any(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'@' | b'?' | b'#' | b'\\'))
    {
        return None;
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let (v6, after) = rest.split_once(']')?;
        let addr: std::net::Ipv6Addr = v6.parse().ok()?;
        let port = match after {
            "" => None,
            s => Some(parse_port(s.strip_prefix(':')?)?),
        };
        return Some((format!("[{addr}]"), port));
    }
    let (host, port) = match raw.split_once(':') {
        Some((h, p)) => (h, Some(parse_port(p)?)),
        None => (raw, None),
    };
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')) {
        return None;
    }
    Some((host.to_ascii_lowercase(), port))
}

fn parse_port(raw: &str) -> Option<u16> {
    if raw.is_empty() || raw.starts_with('0') || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenLinks {
    Desktop,
    Tab,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoardLayout {
    #[default]
    Dispatch,
    Workspace,
}

impl BoardLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            BoardLayout::Dispatch => "dispatch",
            BoardLayout::Workspace => "workspace",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub deny: Vec<String>,
    pub watcher_interval_secs: u64,
    pub max_hold_minutes: f64,
    pub cpu_budget_ms: u64,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub max_concurrent_runs: usize,
    pub run_queue_ms: u64,
    pub max_tickets_total: usize,
    pub link: LinkParams,
    pub wake: Vec<String>,
    pub body_cap: usize,
    pub reply_cap: usize,
    pub digest_lines: usize,
    pub roles: BTreeMap<String, RoleConfig>,
    pub board: BoardConfig,
    #[serde(default)]
    pub upstreams: Vec<UpstreamConfig>,
    #[serde(default)]
    pub pools: BTreeMap<String, PoolConfig>,
    pub limit_hold_minutes: u64,
    pub update_check: bool,
    #[serde(default)]
    pub flow: Option<FlowConfig>,
    #[serde(default)]
    pub trackers: TrackersConfig,
    #[serde(default)]
    pub forges: Forges,
}

pub type Forges = BTreeMap<String, ForgeConfig>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    Gitlab,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgeConfig {
    pub kind: ForgeKind,
    #[serde(default = "glab")]
    pub cli: String,
}

fn glab() -> String {
    "glab".into()
}

pub fn gitlab_cli<'a>(forges: &'a Forges, host: &str) -> Option<&'a str> {
    forges.iter().find(|(h, f)| f.kind == ForgeKind::Gitlab && h.eq_ignore_ascii_case(host)).map(|(_, f)| f.cli.trim())
}

fn validate_forges(forges: &Forges) -> Result<()> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for (host, forge) in forges {
        let at = field::display(&["forges", host]);
        if !is_tracker_host(host) {
            return Err(invalid(
                at,
                format!(
                    "forges: {host:?} must be a hostname without scheme, port or path (such as \"gitlab.example.com\")"
                ),
            ));
        }
        if host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("www.github.com") {
            return Err(invalid(at, "forges: github.com is built in and needs no entry"));
        }
        if let Some(other) = seen.insert(host.to_ascii_lowercase(), host) {
            return Err(invalid(at, format!("forges lists both {other:?} and {host:?}, which are the same host")));
        }
        if forge.cli.trim().is_empty() {
            return Err(invalid(
                format!("{at}.cli"),
                format!("forges.{}.cli must name the CLI to run", field::segment(host)),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackersConfig {
    #[serde(default)]
    pub jira: BTreeMap<String, String>,
    #[serde(default)]
    pub hosts: BTreeMap<String, String>,
}

impl TrackersConfig {
    pub fn validate(&self) -> Result<()> {
        let mut seen_jira: BTreeMap<String, &str> = BTreeMap::new();
        for (key, project) in &self.jira {
            let at = field::display(&["jira", key]);
            if !is_jira_project_key(key) {
                return Err(invalid(
                    at,
                    format!(
                        "jira: {key:?} is not a Jira project key (start with a letter and use at least two letters, digits or _, such as \"PROJ\")"
                    ),
                ));
            }
            if let Some(other) = seen_jira.insert(key.to_ascii_uppercase(), key) {
                return Err(invalid(
                    at,
                    format!("jira maps both {other:?} and {key:?}, which are the same Jira project key"),
                ));
            }
            if project.trim().is_empty() {
                return Err(invalid(at, format!("jira.{key} must name a T3 project id or title")));
            }
        }
        let mut seen_hosts: BTreeMap<String, &str> = BTreeMap::new();
        for (host, project) in &self.hosts {
            let at = field::display(&["hosts", host]);
            if !is_tracker_host(host) {
                return Err(invalid(
                    at,
                    format!(
                        "hosts: {host:?} must be a hostname without scheme, port or path (such as \"jira.example.com\")"
                    ),
                ));
            }
            if host.eq_ignore_ascii_case("github.com") {
                return Err(invalid(
                    at,
                    "hosts: github.com repositories match by git origin remote, not trackers.hosts",
                ));
            }
            if let Some(other) = seen_hosts.insert(host.to_ascii_lowercase(), host) {
                return Err(invalid(at, format!("hosts maps both {other:?} and {host:?}, which are the same host")));
            }
            if project.trim().is_empty() {
                return Err(invalid(at, format!("hosts.{} must name a T3 project id or title", field::segment(host))));
            }
        }
        Ok(())
    }

    pub fn jira_project(&self, project_key: &str) -> Option<(&str, &str)> {
        self.jira.iter().find(|(k, _)| k.eq_ignore_ascii_case(project_key)).map(|(k, v)| (k.as_str(), v.trim()))
    }

    pub fn host_project(&self, host: &str) -> Option<(&str, &str)> {
        self.hosts.iter().find(|(h, _)| h.eq_ignore_ascii_case(host)).map(|(h, v)| (h.as_str(), v.trim()))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repos: Option<Vec<String>>,
    #[serde(default)]
    pub include_repos: Vec<String>,
    #[serde(default)]
    pub exclude_repos: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opt_in: Option<OptIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tickets: Option<usize>,
    #[serde(default)]
    pub launch: Launch,
    #[serde(default)]
    pub serialize_file_conflicts: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptIn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Launch {
    #[default]
    Ask,
    Auto,
    Off,
}

impl Launch {
    pub fn as_str(self) -> &'static str {
        match self {
            Launch::Ask => "ask",
            Launch::Auto => "auto",
            Launch::Off => "off",
        }
    }
}

fn check_repo_list(field: &str, repos: &[String], forbid_shared_short: bool) -> Result<()> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut names: BTreeMap<String, &str> = BTreeMap::new();
    for (i, repo) in repos.iter().enumerate() {
        let at = format!("{field}.{i}");
        let Some((_, name)) = repo.split_once('/').filter(|(o, n)| gh_name(o) && gh_name(n)) else {
            return Err(invalid(at, format!("{field}: {repo:?} is not an owner/name repository")));
        };
        if seen.insert(repo.to_ascii_lowercase(), repo).is_some() {
            return Err(invalid(at, format!("{field} lists {repo} twice")));
        }
        if forbid_shared_short && let Some(other) = names.insert(name.to_ascii_lowercase(), repo) {
            return Err(invalid(
                at,
                format!(
                    "{field}: {other} and {repo} share the name {name}, and board ticket labels ({name}#41) cannot tell them apart"
                ),
            ));
        }
    }
    Ok(())
}

impl FlowConfig {
    pub fn queue_enabled(&self) -> bool {
        self.max_tickets.is_some()
    }

    fn validate(&self) -> Result<()> {
        if let Some(repos) = &self.repos {
            if repos.is_empty() {
                return Err(invalid("repos", "repos must name at least one owner/name repository"));
            }
            check_repo_list("repos", repos, true)?;
        }
        check_repo_list("includeRepos", &self.include_repos, false)?;
        check_repo_list("excludeRepos", &self.exclude_repos, false)?;
        if let Some(opt_in) = &self.opt_in {
            if opt_in.label.as_deref().is_some_and(|l| l.trim().is_empty()) {
                return Err(invalid("optIn.label", "optIn.label must name a label"));
            }
            if opt_in.milestone.as_deref().is_some_and(|m| m.trim().is_empty()) {
                return Err(invalid("optIn.milestone", "optIn.milestone must name a milestone"));
            }
            if opt_in.label.is_none() && opt_in.milestone.is_none() {
                return Err(invalid("optIn", "optIn must set label or milestone"));
            }
        }
        if self.max_tickets == Some(0) {
            return Err(invalid("maxTickets", "maxTickets must be at least 1"));
        }
        if self.max_tickets.is_none() && self.launch != Launch::Off {
            return Err(invalid(
                "maxTickets",
                format!("maxTickets must be at least 1 when launch is {:?}", self.launch.as_str()),
            ));
        }
        Ok(())
    }
}

pub fn builtin_defaults() -> Value {
    json!({
        "deny": ["t3_project_delete", "t3_environment_unlink"],
        "watcherIntervalSecs": 60,
        "maxHoldMinutes": 30,
        "cpuBudgetMs": 2000,
        "defaultTimeoutMs": 60000,
        "maxTimeoutMs": 600000,
        "maxConcurrentRuns": 16,
        "runQueueMs": 10000,
        "maxTicketsTotal": 8,
        "link": {"down": 45, "miss": 2, "stall": 240},
        "wake": ["event", "gate", "blocker", "handoff", "closed", "report", "escalate", "reply"],
        "bodyCap": 1500,
        "replyCap": 1500,
        "digestLines": 6,
        "limitHoldMinutes": 60,
        "updateCheck": true,
        "roles": {
            "overseer": {"children": {"ticket": {
                "down": 75, "stall": 480,
                "wake": ["gate", "blocker", "handoff", "closed", "report", "escalate"],
                "bodyCap": 600
            }}},
            "ticket": {"children": {"worker": {}}},
            "worker": {"children": {}}
        },
        "board": {
            "instanceId": "claudeAgent",
            "pool": null,
            "ticketModel": "claude-opus-5-5",
            "ticketEffort": "high",
            "askInstanceId": null,
            "askPool": null,
            "askModel": "claude-haiku-5-5",
            "askEffort": null,
            "runtimeMode": "full-access",
            "askPollSecs": 4,
            "askTimeoutMinutes": 15,
            "feed": 50,
            "settleAfterHours": 24,
            "reconcileActiveMinutes": 2,
            "reconcileIdleMinutes": 10,
            "openLinks": "desktop",
            "layout": "dispatch",
            "remote": null
        },
        "trackers": {
            "jira": {},
            "hosts": {}
        },
        "forges": {}
    })
}

fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

pub(crate) fn with_defaults(over: Value) -> Value {
    let mut v = builtin_defaults();
    if !over.is_null() {
        merge(&mut v, over);
    }
    v
}

pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("T3CM_CONFIG") {
        return PathBuf::from(p);
    }
    crate::platform::config_dir().join("config.json")
}

impl Default for Config {
    fn default() -> Self {
        Config::from_value(Value::Null).expect("built-in defaults are valid")
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let over = match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).with_context(|| format!("{} is not valid JSON", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Config::from_value(over).with_context(|| format!("config {}", path.display()))
    }

    pub fn from_value(over: Value) -> Result<Config> {
        let legacy_ticket_keys = over
            .get("board")
            .and_then(Value::as_object)
            .map(|b| {
                LEGACY_BOARD_TICKET_KEYS
                    .iter()
                    .filter(|&&k| match b.get(k) {
                        Some(v) if k == "pool" => !v.is_null(),
                        Some(_) => true,
                        None => false,
                    })
                    .map(|&k| k.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let ask_configured = over.get("board").and_then(Value::as_object).is_some_and(|b| {
            BOARD_ASK_KEYS.iter().any(|&k| match b.get(k) {
                Some(v) if matches!(k, "askInstanceId" | "askEffort" | "askPool") => !v.is_null(),
                Some(_) => true,
                None => false,
            })
        });
        let flow_launch_explicit =
            over.get("flow").and_then(Value::as_object).is_some_and(|f| f.contains_key("launch"));
        let v = with_defaults(over);
        if let Some(pools) = v.get("pools") {
            let pools = pools.as_object().ok_or_else(|| invalid("pools", "pools must be an object of named pools"))?;
            for (name, raw) in pools {
                crate::pools::check_value(name, raw)?;
            }
        }
        let mut cfg: Config = field::deserialize(v).map_err(|(at, e)| {
            let message = if at.is_empty() { e.to_string() } else { format!("{at}: {e}") };
            invalid(at, message)
        })?;
        cfg.board.legacy_ticket_keys = legacy_ticket_keys;
        cfg.board.ask_configured = ask_configured;
        if let Some(flow) = &mut cfg.flow
            && flow.max_tickets.is_none()
            && !flow_launch_explicit
        {
            flow.launch = Launch::Off;
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.max_concurrent_runs == 0 {
            return Err(invalid("maxConcurrentRuns", "maxConcurrentRuns must be at least 1"));
        }
        if self.max_timeout_ms < MIN_TIMEOUT_MS {
            return Err(invalid("maxTimeoutMs", format!("maxTimeoutMs must be at least {MIN_TIMEOUT_MS}")));
        }
        if self.max_tickets_total == 0 {
            return Err(invalid("maxTicketsTotal", "maxTicketsTotal must be at least 1"));
        }
        self.link.validate().map_err(|e| within(e, "link", "link"))?;
        let (active, idle) = (self.board.reconcile_active_minutes, self.board.reconcile_idle_minutes);
        if idle > 0 && !(1..=idle).contains(&active) {
            return Err(invalid(
                "board.reconcileActiveMinutes",
                format!("board.reconcileActiveMinutes must be from 1 to reconcileIdleMinutes ({idle}), got {active}"),
            ));
        }
        if !self.roles.get("overseer").is_some_and(|r| r.children.contains_key("ticket")) {
            return Err(invalid(
                "roles.overseer.children.ticket",
                "the board needs the overseer role to open ticket links",
            ));
        }
        check_wake(&self.wake).map_err(|e| within(e, "wake", "wake"))?;
        for (role, rc) in &self.roles {
            for child in rc.children.keys() {
                let at = child_path(role, child);
                if !self.roles.contains_key(child) {
                    return Err(invalid(at, format!("role {role} opens links to unknown role {child}")));
                }
                self.link_params(role, child)?;
                if let Some(w) = &rc.children[child].wake {
                    check_wake(w).map_err(|e| within(e, &format!("{at}.wake"), &format!("{role} -> {child} wake")))?;
                }
                if let Some(c) = rc.children[child].body_cap
                    && c < 200
                {
                    return Err(invalid(
                        format!("{at}.bodyCap"),
                        format!("{role} -> {child}: bodyCap must be at least 200"),
                    ));
                }
            }
        }
        crate::upstream::config::validate(&self.upstreams).context("upstreams")?;
        if !(1..=MAX_LIMIT_HOLD_MINUTES).contains(&self.limit_hold_minutes) {
            return Err(invalid(
                "limitHoldMinutes",
                format!("limitHoldMinutes must be between 1 and {MAX_LIMIT_HOLD_MINUTES} (a week)"),
            ));
        }
        for (name, pool) in &self.pools {
            let at = field::display(&["pools", name]);
            if name.trim().is_empty() {
                return Err(invalid(at, "a pool needs a name"));
            }
            pool.validate().map_err(|e| within(e, &at, &format!("pool {name}")))?;
        }
        if let Some(p) = &self.board.pool
            && !self.pools.contains_key(p)
        {
            return Err(invalid("board.pool", format!("board.pool names {p:?}, which is not in pools")));
        }
        if let Some(p) = &self.board.ask_pool
            && !self.pools.contains_key(p)
        {
            return Err(invalid("board.askPool", format!("board.askPool names {p:?}, which is not in pools")));
        }
        if self.board.ask_model.trim().is_empty() {
            return Err(invalid("board.askModel", "board.askModel must not be empty"));
        }
        if self.board.ask_instance_id.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err(invalid("board.askInstanceId", "board.askInstanceId must not be empty"));
        }
        if self.board.ask_effort.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err(invalid("board.askEffort", "board.askEffort must not be empty"));
        }
        if let Some(remote) = &self.board.remote {
            remote.validate()?;
        }
        if let Some(flow) = &self.flow {
            flow.validate().map_err(|e| within(e, "flow", "flow"))?;
            if let Some(max) = flow.max_tickets
                && max > self.max_tickets_total
            {
                return Err(invalid(
                    "flow.maxTickets",
                    format!(
                        "flow.maxTickets ({max}) must not exceed maxTicketsTotal ({}); lower flow.maxTickets or raise maxTicketsTotal",
                        self.max_tickets_total
                    ),
                ));
            }
        }
        self.trackers.validate().map_err(|e| within(e, "trackers", "trackers"))?;
        validate_forges(&self.forges)?;
        Ok(())
    }

    pub(crate) fn hold_restart_only(&mut self, running: &Config) -> Vec<&'static str> {
        let mut held = Vec::new();
        if self.max_concurrent_runs != running.max_concurrent_runs {
            self.max_concurrent_runs = running.max_concurrent_runs;
            held.push("maxConcurrentRuns");
        }
        if self.upstreams != running.upstreams {
            self.upstreams.clone_from(&running.upstreams);
            held.push("upstreams");
        }
        let remote = |config: &Config| config.board.remote.clone().unwrap_or_default();
        if remote(self) != remote(running) {
            held.push("board.remote");
        }
        self.board.remote.clone_from(&running.board.remote);
        held
    }

    pub fn check_role(&self, role: &str) -> Result<()> {
        if self.roles.contains_key(role) {
            Ok(())
        } else {
            let known: Vec<&str> = self.roles.keys().map(String::as_str).collect();
            Err(anyhow!("unknown role {role:?}; roles are {}", known.join(", ")))
        }
    }

    pub fn board_oversees(&self, role: &str) -> bool {
        self.roles.get("overseer").is_some_and(|r| r.children.contains_key(role))
    }

    pub fn link_params(&self, parent_role: &str, child_role: &str) -> Result<LinkParams> {
        self.check_role(parent_role)?;
        let child = self.roles[parent_role]
            .children
            .get(child_role)
            .ok_or_else(|| anyhow!("role {parent_role:?} opens no links to {child_role:?}"))?;
        let p = LinkParams {
            down: child.down.unwrap_or(self.link.down),
            miss: child.miss.unwrap_or(self.link.miss),
            stall: child.stall.unwrap_or(self.link.stall),
        };
        p.validate().map_err(|e| {
            let key = match field::of(&e) {
                "stall" if child.stall.is_none() && child.down.is_some() => "down",
                key => key,
            };
            let own = match key {
                "down" => child.down.is_some(),
                "miss" => child.miss.is_some(),
                _ => child.stall.is_some(),
            };
            let at = if own { format!("{}.{key}", child_path(parent_role, child_role)) } else { format!("link.{key}") };
            invalid(at, format!("{parent_role} -> {child_role} link numbers: {e}"))
        })?;
        Ok(p)
    }

    pub fn link_policy(&self, parent_role: Option<&str>, child_role: &str) -> LinkPolicy {
        let child = parent_role.and_then(|r| self.roles.get(r)).and_then(|rc| rc.children.get(child_role));
        LinkPolicy {
            wake: child.and_then(|c| c.wake.clone()).unwrap_or_else(|| self.wake.clone()),
            body_cap: child.and_then(|c| c.body_cap).unwrap_or(self.body_cap),
            relay: child.and_then(|c| c.relay).unwrap_or(true),
        }
    }
}

fn child_path(role: &str, child: &str) -> String {
    field::display(&["roles", role, "children", child])
}

fn check_wake(kinds: &[String]) -> Result<()> {
    for (i, k) in kinds.iter().enumerate() {
        if k == "note" {
            return Err(invalid(i.to_string(), "a note never wakes its parent; take it out of wake"));
        }
        if !UP_KINDS.contains(&k.as_str()) && !EXTRA_WAKE_KINDS.contains(&k.as_str()) {
            return Err(invalid(i.to_string(), format!("unknown wake kind {k:?}")));
        }
    }
    Ok(())
}
