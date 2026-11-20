use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub const CONFIG_VERSION: u64 = 1;
pub const SLOTS: &[&str] = &["review", "passes", "credentials", "delivery", "models"];
const TOP_KEYS: &[&str] = &["version", "review", "passes", "credentials", "delivery", "models", "writtenBy", "steps"];
pub const DELIVERY_MODES: &[&str] = &["merge", "tag-deploy"];
pub const MODEL_ROLES: &[&str] = &["ticket", "worker"];
const DEFAULT_REVIEW: &str = "sol-loop";
const DEFAULT_PROVIDER: &str = "claudeAgent";
const DEFAULT_MODEL: &str = "claude-opus-5-5";
const DEFAULT_EFFORT: &str = "high";
const REVIEW_SUGGESTION: &str = "sol-loop from https://github.com/t3dotgg/tstack";
const POOL_NOTE: &str = "t3cm picks the instance at launch with wf.pick";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in pattern")
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^(#{1,6})\s+(.*?)\s*#*\s*$"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*[-*]\s+([^:]+?):\s*(.*)$"));
static TICKED: LazyLock<Regex> = LazyLock::new(|| re(r"`([^`]+)`"));
static ARROW: LazyLock<Regex> = LazyLock::new(|| re(r"\s*(?:->|→)\s*"));
static POOL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^\s*(?:the\s+)?pool:?\s+(.*)$"));
static EFFORT_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(low|medium|high|xhigh|max|ultrathink)\b"));
static FRONTMATTER: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)^---\s*\n(.*?)\n---\s*(?:\n|$)"));
static META_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"^([A-Za-z0-9_-]+):\s*(.*)$"));

fn captures<'t>(r: &Regex, text: &'t str) -> Option<fancy_regex::Captures<'t>> {
    r.captures(text).ok().flatten()
}

fn group<'t>(c: &fancy_regex::Captures<'t>, i: usize) -> &'t str {
    c.get(i).map_or("", |m| m.as_str())
}

static PY_ASCII_FLAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!\\)\(\?([imsxu]*)a([imsxu]*)(-[imsx]+)?([:)])"));

fn without_ascii_flag(pattern: &str) -> std::borrow::Cow<'_, str> {
    PY_ASCII_FLAG.replace_all(pattern, |c: &fancy_regex::Captures| {
        let on = format!("{}{}", group(c, 1), group(c, 2));
        match (on.is_empty(), group(c, 3), group(c, 4)) {
            (true, "", ")") => String::new(),
            (_, off, end) => format!("(?{on}{off}{end}"),
        }
    })
}

pub fn compile(pattern: &str) -> std::result::Result<Regex, String> {
    Regex::new(&without_ascii_flag(pattern)).map_err(|e| e.to_string())
}

pub fn search(pattern: &str, text: &str) -> std::result::Result<bool, String> {
    compile(pattern)?.is_match(text).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folders {
    pub home: PathBuf,
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub config: PathBuf,
}

impl Folders {
    pub fn from_env() -> Result<Folders> {
        let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = crate::skills::user_home().context("no home folder: set HOME (USERPROFILE on Windows)")?;
        let mut f = Folders::under(&home, var("XDG_CONFIG_HOME").as_deref());
        if let Some(p) = var("T3WF_CONFIG") {
            f.config = p;
        }
        if let Some(p) = var("CLAUDE_CONFIG_DIR") {
            f.claude = p;
        }
        if let Some(p) = var("CODEX_HOME") {
            f.codex = p;
        }
        Ok(f)
    }

    pub fn under(home: &Path, config_home: Option<&Path>) -> Folders {
        let config = config_home.map(Path::to_path_buf).unwrap_or_else(|| home.join(".config"));
        Folders {
            home: home.to_path_buf(),
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            config: config.join("t3-workflow").join("config.json"),
        }
    }

    pub fn default_claude(&self) -> PathBuf {
        self.home.join(".claude")
    }

    pub fn default_codex(&self) -> PathBuf {
        self.home.join(".codex")
    }

    pub fn manifest(&self) -> PathBuf {
        self.config.parent().map_or_else(|| PathBuf::from("installed.json"), |d| d.join("installed.json"))
    }
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn py_repr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => py_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => {
            let items: Vec<String> = o.iter().map(|(k, v)| format!("{}: {}", py_str(k), py_repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

fn py_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn py_list(items: &[String]) -> String {
    py_repr(&json!(items))
}

fn skill_ref_ok(s: &str) -> bool {
    let s = s.strip_suffix('\n').unwrap_or(s);
    let word = |w: &str, extra: &[char]| {
        !w.is_empty()
            && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || extra.contains(&c))
    };
    match s.rsplit_once(':') {
        Some((ns, name)) => word(ns, &['/']) && word(name, &[]),
        None => word(s, &[]),
    }
}

fn skill_ref_problem(at: &str, v: &Value) -> Option<String> {
    match v {
        Value::String(s) if skill_ref_ok(s) => None,
        _ => Some(format!("{at}: {} is not a skill name (bare `name` or `namespace:name`)", py_repr(v))),
    }
}

fn nonempty_str(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

pub fn validate_slots(cfg: &Map<String, Value>, at: &str) -> Vec<String> {
    let mut problems: Vec<Option<String>> = Vec::new();
    if let Some(r) = cfg.get("review").filter(|r| !r.is_null()) {
        problems.push(skill_ref_problem(&format!("{at}.review"), r));
    }
    if let Some(p) = cfg.get("passes") {
        match p.as_array() {
            None => problems.push(Some(format!("{at}.passes: must be a list of skill names"))),
            Some(a) => {
                for (i, v) in a.iter().enumerate() {
                    problems.push(skill_ref_problem(&format!("{at}.passes[{i}]"), v));
                }
            }
        }
    }
    if let Some(c) = cfg.get("credentials") {
        match c.as_array() {
            None => {
                problems.push(Some(format!("{at}.credentials: must be a list of {{\"match\", \"skill\"}} objects")))
            }
            Some(a) => {
                for (i, c) in a.iter().enumerate() {
                    let w = format!("{at}.credentials[{i}]");
                    let keys: HashSet<&str> =
                        c.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
                    if !c.is_object() || keys != HashSet::from(["match", "skill"]) {
                        problems.push(Some(format!("{w}: needs exactly the keys match and skill")));
                        continue;
                    }
                    problems.push(skill_ref_problem(&format!("{w}.skill"), &c["skill"]));
                    let compiled = match c["match"].as_str() {
                        Some(p) => compile(p),
                        None => Err("first argument must be string or compiled pattern".into()),
                    };
                    if let Err(e) = compiled {
                        problems.push(Some(format!("{w}.match: not a regular expression: {e}")));
                    }
                }
            }
        }
    }
    if let Some(d) = cfg.get("delivery") {
        match d.as_object() {
            None => {
                problems.push(Some(format!("{at}.delivery: must be an object with mode (and skill for tag-deploy)")))
            }
            Some(d) => {
                let mut extra: Vec<String> = d.keys().filter(|k| *k != "mode" && *k != "skill").cloned().collect();
                if !extra.is_empty() {
                    extra.sort();
                    problems.push(Some(format!("{at}.delivery: unknown keys {}", py_list(&extra))));
                }
                if let Some(m) = d.get("mode")
                    && !m.as_str().is_some_and(|m| DELIVERY_MODES.contains(&m))
                {
                    problems.push(Some(format!(
                        "{at}.delivery.mode: {} is not one of {}",
                        py_repr(m),
                        DELIVERY_MODES.join(", ")
                    )));
                }
                if let Some(s) = d.get("skill") {
                    problems.push(skill_ref_problem(&format!("{at}.delivery.skill"), s));
                }
            }
        }
    }
    if let Some(m) = cfg.get("models") {
        match m.as_object() {
            None => problems.push(Some(format!("{at}.models: must be an object keyed by ticket and worker"))),
            Some(m) => {
                for (role, spec) in m {
                    let w = format!("{at}.models.{role}");
                    let obj = spec.as_object();
                    if !MODEL_ROLES.contains(&role.as_str()) {
                        problems.push(Some(format!("{w}: unknown role; roles are {}", MODEL_ROLES.join(", "))));
                    } else if let Some(o) = obj.filter(|o| o.contains_key("pool")) {
                        if o.len() != 1 {
                            problems.push(Some(format!("{w}: a pool sets the model and effort, so pool stands alone")));
                        } else if !nonempty_str(&o["pool"]) {
                            problems.push(Some(format!("{w}.pool: must be a non-empty string")));
                        }
                    } else if obj
                        .is_none_or(|o| o.keys().any(|k| !["model", "effort", "provider"].contains(&k.as_str())))
                    {
                        problems.push(Some(format!(
                            "{w}: must be an object with model, and optionally effort and provider, or with pool alone"
                        )));
                    } else if let Some(o) = obj {
                        for (k, v) in o {
                            if !nonempty_str(v) {
                                problems.push(Some(format!("{w}.{k}: must be a non-empty string")));
                            }
                        }
                    }
                }
            }
        }
    }
    problems.into_iter().flatten().collect()
}

fn version_ok(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() == Some(CONFIG_VERSION as f64),
        _ => false,
    }
}

pub fn validate_config(cfg: &Map<String, Value>, at: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut extra: Vec<String> = cfg.keys().filter(|k| !TOP_KEYS.contains(&k.as_str())).cloned().collect();
    if !extra.is_empty() {
        extra.sort();
        problems.push(format!("{at}: unknown keys {}; keys are {}", py_list(&extra), TOP_KEYS.join(", ")));
    }
    if let Some(v) = cfg.get("version").filter(|v| !version_ok(v)) {
        problems.push(format!("{at}.version: {}, expected {CONFIG_VERSION}", py_repr(v)));
    }
    if cfg.get("writtenBy").is_some_and(|v| !v.is_string()) {
        problems.push(format!("{at}.writtenBy: must be a release string"));
    }
    if cfg.get("steps").is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
        problems.push(format!("{at}.steps: must be a list of step ids"));
    }
    problems.extend(validate_slots(cfg, at));
    problems
}

pub fn load_user_config(path: &Path) -> std::result::Result<Map<String, Value>, String> {
    let raw = match std::fs::read(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    match serde_json::from_slice::<Value>(&raw) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(format!("{}: the top level must be a JSON object", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let breaks =
            matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !breaks {
            continue;
        }
        out.push(&text[start..i]);
        let mut next = i + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|(_, n)| *n == '\n') {
            chars.next();
            next += 1;
        }
        start = next;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

const PY_STRIP: &[char] = &[
    ' ', '\t', '\n', '\r', '\x0b', '\x0c', '\x1c', '\x1d', '\x1e', '\x1f', '\u{85}', '\u{a0}', '\u{2028}', '\u{2029}',
];

fn strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || PY_STRIP.contains(&c))
}

fn first_name(value: &str) -> String {
    if let Some(c) = captures(&TICKED, value) {
        return strip(group(&c, 1)).to_string();
    }
    value.split_whitespace().next().map_or(String::new(), |w| w.trim_matches(['.', ',', ';', '(', ')']).to_string())
}

fn names(value: &str) -> Vec<String> {
    let ticked: Vec<String> =
        TICKED.captures_iter(value).filter_map(|c| c.ok()).map(|c| strip(group(&c, 1)).to_string()).collect();
    if !ticked.is_empty() {
        return ticked;
    }
    value.split(',').map(|w| w.trim_matches([' ', '.'])).filter(|w| !w.is_empty()).map(String::from).collect()
}

fn is_none(value: &str) -> bool {
    matches!(strip(value).trim_matches('`').to_lowercase().as_str(), "none" | "no" | "off" | "-")
}

pub fn agent_workflow_lines(text: &str) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let mut level: Option<usize> = None;
    for raw in split_lines(text) {
        if let Some(m) = captures(&HEADING, raw) {
            let depth = group(&m, 1).len();
            if level.is_some_and(|l| depth <= l) {
                break;
            }
            if level.is_none() && strip(group(&m, 2)).to_lowercase() == "agent workflow" {
                level = Some(depth);
            }
            continue;
        }
        if level.is_some()
            && let Some(b) = captures(&BULLET, raw)
        {
            lines.push((strip(group(&b, 1)).to_lowercase(), strip(group(&b, 2)).to_string()));
        }
    }
    lines
}

pub fn parse_agent_workflow(text: &str) -> (Map<String, Value>, Vec<String>) {
    let mut cfg = Map::new();
    let mut problems = Vec::new();
    let entry = |cfg: &mut Map<String, Value>, key: &str| -> Map<String, Value> {
        cfg.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
    };
    for (key, value) in agent_workflow_lines(text) {
        match key.as_str() {
            "review" | "independent review" => {
                let v = if is_none(&value) { Value::Null } else { json!(first_name(&value)) };
                cfg.insert("review".into(), v);
            }
            "passes" | "worker passes" => {
                let v = if is_none(&value) { Vec::new() } else { names(&value) };
                cfg.insert("passes".into(), json!(v));
            }
            "credential renewal" | "credentials" => {
                let mut pairs = Vec::new();
                if !is_none(&value) {
                    for part in value.split(';') {
                        let part = strip(part);
                        let sides: Vec<&str> = match ARROW.find(part).ok().flatten() {
                            Some(m) => vec![&part[..m.start()], &part[m.end()..]],
                            None => vec![part],
                        };
                        if sides.len() != 2 {
                            problems.push(format!(
                                "AGENTS.md Credential renewal: {} is not `match` -> `skill`",
                                py_str(part)
                            ));
                            continue;
                        }
                        pairs.push(json!({"match": strip(sides[0]).trim_matches('`'), "skill": first_name(sides[1])}));
                    }
                }
                cfg.insert("credentials".into(), json!(pairs));
            }
            "delivery" => {
                let mut d = entry(&mut cfg, "delivery");
                d.insert("mode".into(), json!(first_name(&value)));
                cfg.insert("delivery".into(), Value::Object(d));
            }
            "deploy skill" => {
                let mut d = entry(&mut cfg, "delivery");
                d.insert("skill".into(), json!(first_name(&value)));
                cfg.insert("delivery".into(), Value::Object(d));
            }
            "ticket model" | "worker model" => {
                let role = key.split_whitespace().next().unwrap_or_default().to_string();
                let mut models = entry(&mut cfg, "models");
                if let Some(p) = captures(&POOL, &value) {
                    models.insert(role, json!({"pool": first_name(group(&p, 1))}));
                } else {
                    let model = first_name(&value);
                    let mut spec = Map::new();
                    spec.insert("model".into(), json!(model));
                    let rest = value.replacen(&model, "", 1);
                    if let Some(e) = captures(&EFFORT_WORD, &rest) {
                        spec.insert("effort".into(), json!(group(&e, 1).to_lowercase()));
                    }
                    models.insert(role, Value::Object(spec));
                }
                cfg.insert("models".into(), Value::Object(models));
            }
            _ => {}
        }
    }
    problems.extend(validate_slots(&cfg, "AGENTS.md").into_iter().map(|p| p.replace("AGENTS.md.", "AGENTS.md ")));
    (cfg, problems)
}

pub fn repo_override(root: &Path) -> (Map<String, Value>, Vec<String>) {
    match std::fs::read(root.join("AGENTS.md")) {
        Ok(raw) => parse_agent_workflow(&String::from_utf8_lossy(&raw)),
        Err(_) => (Map::new(), Vec::new()),
    }
}

fn frontmatter(path: &Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read(path) else { return HashMap::new() };
    let all = String::from_utf8_lossy(&raw);
    let head: String = all.chars().take(8192).collect();
    let Some(m) = captures(&FRONTMATTER, &head) else { return HashMap::new() };
    let mut meta = HashMap::new();
    for line in split_lines(group(&m, 1)) {
        if let Some(kv) = captures(&META_LINE, line) {
            meta.insert(group(&kv, 1).to_string(), strip(group(&kv, 2)).trim_matches(['"', '\'']).to_string());
        }
    }
    meta
}

fn is_inline(meta: &HashMap<String, String>) -> bool {
    meta.get("disable-model-invocation").is_some_and(|v| matches!(v.to_lowercase().as_str(), "true" | "yes" | "1"))
}

fn sorted_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> =
        rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn skill_files(base: &Path) -> Vec<PathBuf> {
    sorted_names(base).into_iter().map(|n| base.join(n).join("SKILL.md")).filter(|p| p.is_file()).collect()
}

fn version_key(v: &str) -> Vec<i128> {
    v.split(['.', '+', '-'])
        .map(
            |x| {
                if !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()) { x.parse().unwrap_or(i128::MAX) } else { -1 }
            },
        )
        .collect()
}

fn newest_cached(cache: &Path) -> Vec<(String, String, PathBuf)> {
    let mut roots = Vec::new();
    if !cache.is_dir() {
        return roots;
    }
    for market in sorted_names(cache) {
        let mdir = cache.join(&market);
        if !mdir.is_dir() {
            continue;
        }
        for plugin in sorted_names(&mdir) {
            let pdir = mdir.join(&plugin);
            let versions: Vec<String> = sorted_names(&pdir).into_iter().filter(|v| pdir.join(v).is_dir()).collect();
            let mut best: Option<&String> = None;
            for v in &versions {
                if best.is_none_or(|b| version_key(v) > version_key(b)) {
                    best = Some(v);
                }
            }
            if let Some(b) = best {
                roots.push((plugin.clone(), format!("{plugin}@{market}"), pdir.join(b).join("skills")));
            }
        }
    }
    roots
}

pub fn realpath(p: &Path) -> PathBuf {
    if let Ok(r) = std::fs::canonicalize(p) {
        return r;
    }
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) if !dir.as_os_str().is_empty() => realpath(dir).join(name),
        _ => std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Enabled(Option<HashMap<String, bool>>);

impl Enabled {
    pub fn load(claude: &Path, root: &Path) -> Enabled {
        let mut merged: Option<HashMap<String, bool>> = None;
        for p in [
            claude.join("settings.json"),
            root.join(".claude").join("settings.json"),
            root.join(".claude").join("settings.local.json"),
        ] {
            let Ok(raw) = std::fs::read(&p) else { continue };
            let Ok(v) = serde_json::from_slice::<Value>(&raw) else { continue };
            if let Some(m) = v.get("enabledPlugins").and_then(Value::as_object) {
                let into = merged.get_or_insert_with(HashMap::new);
                for (k, v) in m {
                    into.insert(k.clone(), v == &Value::Bool(true));
                }
            }
        }
        Enabled(merged)
    }

    pub fn all() -> Enabled {
        Enabled(None)
    }

    fn allows(&self, key: &str) -> bool {
        self.0.as_ref().is_none_or(|m| m.get(key).copied().unwrap_or(false))
    }

    pub fn disabled(&self) -> Vec<String> {
        let mut out: Vec<String> = self.0.iter().flatten().filter(|(_, on)| !**on).map(|(k, _)| k.clone()).collect();
        out.sort();
        out
    }
}

fn installed_plugin_roots(claude: &Path, root: &Path) -> Option<Vec<(String, String, PathBuf)>> {
    let raw = std::fs::read(claude.join("plugins").join("installed_plugins.json")).ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    let top = v.as_object()?;
    let Some(installed) = top.get("plugins") else { return Some(Vec::new()) };
    let installed = installed.as_object()?;
    let want = realpath(root);
    let mut roots = Vec::new();
    for (key, entries) in installed {
        let name = key.split('@').next().unwrap_or_default().to_string();
        let list = match entries {
            Value::Array(a) => a.clone(),
            other => vec![other.clone()],
        };
        for e in list {
            let e = e.as_object()?;
            if let Some(scoped) = e.get("projectPath").filter(|s| truthy(s))
                && realpath(Path::new(scoped.as_str()?)) != want
            {
                continue;
            }
            if let Some(ip) = e.get("installPath").filter(|s| truthy(s)) {
                roots.push((name.clone(), key.clone(), Path::new(ip.as_str()?).join("skills")));
            }
        }
    }
    Some(roots)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn claude_plugin_roots(claude: &Path, root: &Path) -> Vec<(String, String, PathBuf)> {
    installed_plugin_roots(claude, root).unwrap_or_else(|| newest_cached(&claude.join("plugins").join("cache")))
}

#[derive(Clone, Debug)]
pub struct SearchRoot {
    pub label: String,
    pub namespace: Option<String>,
    pub dir: PathBuf,
}

pub fn search_roots(f: &Folders, root: &Path, enabled: &Enabled) -> Vec<SearchRoot> {
    let plain = |label: &str, dir: PathBuf| SearchRoot { label: label.into(), namespace: None, dir };
    let mut roots = vec![
        plain("~/.claude/skills", f.claude.join("skills")),
        plain("~/.agents/skills", f.home.join(".agents").join("skills")),
        plain("project .claude/skills", root.join(".claude").join("skills")),
        plain("~/.codex/skills", f.codex.join("skills")),
    ];
    let claude = claude_plugin_roots(&f.claude, root).into_iter().filter(|(_, key, _)| enabled.allows(key));
    let codex = newest_cached(&f.codex.join("plugins").join("cache"));
    for (plugin, _, dir) in claude.chain(codex) {
        roots.push(SearchRoot { label: "plugin cache".into(), namespace: Some(plugin), dir });
    }
    roots
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub bare: String,
    pub dir: Option<String>,
    pub path: Option<String>,
    pub inline: bool,
    pub at: String,
}

impl Skill {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), json!(self.name));
        m.insert("bare".into(), json!(self.bare));
        if let Some(d) = &self.dir {
            m.insert("dir".into(), json!(d));
        }
        m.insert("path".into(), json!(self.path));
        m.insert("inline".into(), json!(self.inline));
        m.insert("where".into(), json!(self.at));
        Value::Object(m)
    }
}

pub fn installed_skills(roots: &[SearchRoot]) -> Vec<Skill> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for r in roots {
        for path in skill_files(&r.dir) {
            let meta = frontmatter(&path);
            let dir =
                path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let bare = meta.get("name").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| dir.clone());
            let name = match &r.namespace {
                Some(ns) => format!("{ns}:{bare}"),
                None => bare.clone(),
            };
            if !seen.insert((realpath(&path), name.clone())) {
                continue;
            }
            found.push(Skill {
                name,
                bare,
                dir: Some(dir),
                path: Some(text(&path)),
                inline: is_inline(&meta),
                at: r.label.clone(),
            });
        }
    }
    found
}

pub fn bare(reference: &str) -> &str {
    reference.rsplit(':').next().unwrap_or(reference)
}

pub struct Resolver {
    listed: Vec<String>,
    disk: Vec<Skill>,
}

impl Resolver {
    pub fn new(roots: &[SearchRoot], listed: &[String]) -> Resolver {
        let listed =
            listed.iter().map(|s| s.trim().trim_start_matches('/').to_string()).filter(|s| !s.is_empty()).collect();
        Resolver { listed, disk: installed_skills(roots) }
    }

    pub fn candidates(&self, reference: &str) -> Vec<Skill> {
        let want = bare(reference);
        let mut out = Vec::new();
        let mut used = HashSet::new();
        let mut once = HashSet::new();
        for name in self.listed.iter().filter(|n| bare(n) == want) {
            if !once.insert(name.clone()) {
                continue;
            }
            match self.disk.iter().find(|s| &s.name == name) {
                Some(d) => {
                    if let Some(p) = &d.path {
                        used.insert(p.clone());
                    }
                    out.push(Skill { at: "session".into(), ..d.clone() });
                }
                None => out.push(Skill {
                    name: name.clone(),
                    bare: want.to_string(),
                    dir: None,
                    path: None,
                    inline: false,
                    at: "session".into(),
                }),
            }
        }
        for s in &self.disk {
            let hit = s.bare == want || s.dir.as_deref() == Some(want);
            if hit && !s.path.as_ref().is_some_and(|p| used.contains(p)) {
                out.push(s.clone());
            }
        }
        out
    }

    pub fn resolve(&self, reference: &str) -> Value {
        let cands = self.candidates(reference);
        let mut names: Vec<String> = Vec::new();
        for c in &cands {
            if !names.contains(&c.name) {
                names.push(c.name.clone());
            }
        }
        let exact: Vec<&Skill> = cands.iter().filter(|c| c.name == reference).collect();
        let hit: Vec<&Skill> = if !exact.is_empty() {
            exact.clone()
        } else if reference.contains(':') {
            Vec::new()
        } else {
            cands.iter().collect()
        };
        let Some(first) = hit.first() else {
            return json!({"ref": reference, "status": "missing", "others": names});
        };
        let others: Vec<String> = names.iter().filter(|n| **n != first.name).cloned().collect();
        let mut e = json!({
            "ref": reference,
            "name": first.name,
            "path": first.path,
            "where": first.at,
            "invoke": if first.inline { "inline" } else { "skill" },
        });
        if !others.is_empty() && exact.is_empty() {
            e["status"] = json!("ambiguous");
            e["others"] = json!(others);
        } else {
            e["status"] = json!("ok");
        }
        e
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSlot {
    pub role: String,
    pub spec: Option<Map<String, Value>>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub repo: String,
    pub config: Option<String>,
    pub review: Value,
    pub passes: Value,
    pub credentials: Value,
    pub delivery: Value,
    pub models: Vec<ModelSlot>,
    pub config_problems: Vec<String>,
    pub repo_problems: Vec<String>,
}

pub struct Inputs<'a> {
    pub folders: &'a Folders,
    pub root: &'a Path,
    pub listed: &'a [String],
    pub enabled: &'a Enabled,
}

struct Eff {
    review: (Value, String),
    passes: (Vec<String>, String),
    credentials: (Vec<(String, String)>, String),
    delivery: (Map<String, Value>, String),
    models: BTreeMap<String, (Option<Map<String, Value>>, String)>,
    review_set: bool,
}

fn merged(user: &Map<String, Value>, block: &Map<String, Value>) -> Eff {
    let mut eff = Eff {
        review: (Value::Null, "default".into()),
        passes: (Vec::new(), "default".into()),
        credentials: (Vec::new(), "default".into()),
        delivery: ([("mode".to_string(), json!("merge"))].into_iter().collect(), "default".into()),
        models: MODEL_ROLES.iter().map(|r| (r.to_string(), (None, "default".to_string()))).collect(),
        review_set: false,
    };
    for (src, cfg) in [("user", user), ("AGENTS.md", block)] {
        if let Some(r) = cfg.get("review") {
            eff.review = (r.clone(), src.into());
            eff.review_set = true;
        }
        if let Some(p) = cfg.get("passes") {
            let refs = p.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
            eff.passes = (refs, src.into());
        }
        if let Some(c) = cfg.get("credentials") {
            let pairs = c
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
                    (s("match"), s("skill"))
                })
                .collect();
            eff.credentials = (pairs, src.into());
        }
        if let Some(d) = cfg.get("delivery").and_then(Value::as_object) {
            let mut next = eff.delivery.0.clone();
            next.extend(d.clone());
            eff.delivery = (next, src.into());
        }
        if let Some(m) = cfg.get("models").and_then(Value::as_object) {
            for (role, spec) in m {
                let (Some(slot), Some(spec)) = (eff.models.get_mut(role), spec.as_object()) else { continue };
                let prev = slot.0.clone().unwrap_or_default();
                let same = spec.get("model").filter(|m| !m.is_null()).is_none_or(|m| prev.get("model") == Some(m));
                let mut base =
                    if same && !spec.contains_key("pool") && !prev.contains_key("pool") { prev } else { Map::new() };
                base.extend(spec.clone());
                *slot = (Some(base), src.into());
            }
        }
    }
    eff
}

impl Resolved {
    pub fn build(i: &Inputs) -> Resolved {
        let config_file = &i.folders.config;
        let config_exists = config_file.exists();
        let mut config_problems = Vec::new();
        let mut user = match load_user_config(config_file) {
            Ok(c) => c,
            Err(e) => {
                config_problems.push(e);
                Map::new()
            }
        };
        if !user.is_empty() {
            let base = config_file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            config_problems.extend(validate_config(&user, &base));
        }
        if !config_problems.is_empty() {
            user = Map::new();
        }
        let (block, repo_problems) = repo_override(i.root);
        let eff = merged(&user, &block);
        let resolver = Resolver::new(&search_roots(i.folders, i.root, i.enabled), i.listed);
        let review = if !eff.review_set {
            let hit = resolver.resolve(DEFAULT_REVIEW);
            if hit["status"] == "missing" {
                json!({"slot": "review", "source": "default", "skill": null,
                       "note": format!("no reviewer installed; suggest {REVIEW_SUGGESTION}")})
            } else {
                json!({"slot": "review", "source": "default", "skill": hit})
            }
        } else {
            match eff.review.0.as_str() {
                None => json!({"slot": "review", "source": eff.review.1, "skill": null, "note": "none configured"}),
                Some(r) => json!({"slot": "review", "source": eff.review.1, "skill": resolver.resolve(r)}),
            }
        };
        let passes = json!({"slot": "passes", "source": eff.passes.1,
                            "skills": eff.passes.0.iter().map(|r| resolver.resolve(r)).collect::<Vec<_>>()});
        let renewals: Vec<Value> =
            eff.credentials.0.iter().map(|(m, s)| json!({"match": m, "skill": resolver.resolve(s)})).collect();
        let credentials = json!({"slot": "credentials", "source": eff.credentials.1, "renewals": renewals});
        let d = &eff.delivery.0;
        let mode = d.get("mode").cloned().unwrap_or_else(|| json!("merge"));
        let mut delivery = json!({"slot": "delivery", "source": eff.delivery.1, "mode": mode});
        match d.get("skill").filter(|s| truthy(s)) {
            Some(s) => delivery["skill"] = resolver.resolve(s.as_str().unwrap_or_default()),
            None if mode == "tag-deploy" => {
                delivery["skill"] = Value::Null;
                delivery["problem"] = json!("tag-deploy names no deploy skill");
            }
            None => {}
        }
        let mut models: Vec<ModelSlot> =
            eff.models.into_iter().map(|(role, (spec, source))| ModelSlot { role, spec, source }).collect();
        models.sort_by_key(|m| MODEL_ROLES.iter().position(|r| *r == m.role));
        Resolved {
            repo: text(i.root),
            config: config_exists.then(|| text(config_file)),
            review,
            passes,
            credentials,
            delivery,
            models,
            config_problems,
            repo_problems,
        }
    }

    pub fn problems(&self) -> Vec<String> {
        self.config_problems.iter().chain(&self.repo_problems).cloned().collect()
    }

    pub fn model_searches(&self) -> Vec<String> {
        let mut out = Vec::new();
        for m in &self.models {
            let search = match &m.spec {
                None => Some(DEFAULT_MODEL.to_string()),
                Some(s) if s.contains_key("pool") => None,
                Some(s) => s.get("model").and_then(Value::as_str).map(String::from),
            };
            if let Some(s) = search.filter(|s| !out.contains(s)) {
                out.push(s);
            }
        }
        out
    }

    pub fn models_json(&self, caps: &Caps) -> Value {
        let mut out = Map::new();
        for m in &self.models {
            out.insert(m.role.clone(), model_entry(m, caps));
        }
        Value::Object(out)
    }

    pub fn all(&self, caps: &Caps) -> Value {
        json!({
            "repo": self.repo,
            "config": self.config,
            "review": self.review,
            "passes": self.passes,
            "credentials": self.credentials,
            "delivery": self.delivery,
            "models": self.models_json(caps),
        })
    }
}

#[derive(Clone, Debug)]
pub enum Caps {
    Unchecked,
    Failed(String),
    Got(Value),
}

impl Caps {
    fn get(&self) -> Option<&Value> {
        match self {
            Caps::Got(v) => Some(v),
            _ => None,
        }
    }

    fn why(&self) -> String {
        match self {
            Caps::Failed(e) => format!("T3 unreachable ({e})"),
            _ => "T3 not checked".into(),
        }
    }
}

fn disabled(p: &Value) -> bool {
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

pub fn model_offers(caps: &Value, model: &str, provider: Option<&str>) -> Vec<(String, Value)> {
    let mut offers: Vec<(String, Value)> = Vec::new();
    for p in caps["providers"].as_array().into_iter().flatten() {
        if disabled(p) {
            continue;
        }
        let pid = p["providerInstanceId"].as_str().unwrap_or_default().to_string();
        if let Some(want) = provider
            && want != pid
            && p["driverKind"].as_str() != Some(want)
        {
            continue;
        }
        for m in p["models"].as_array().into_iter().flatten() {
            if m["id"].as_str() == Some(model) {
                match offers.iter_mut().find(|(k, _)| *k == pid) {
                    Some(slot) => slot.1 = m.clone(),
                    None => offers.push((pid.clone(), m.clone())),
                }
            }
        }
    }
    offers
}

fn effort_values(entry: &Value) -> Option<Vec<String>> {
    for o in entry["options"].as_array().into_iter().flatten() {
        if matches!(o["id"].as_str(), Some("effort" | "thinking_level")) {
            return Some(
                o["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
                    .collect(),
            );
        }
    }
    None
}

fn check_model(spec: &Map<String, Value>, caps: &Value) -> Option<String> {
    let model = spec.get("model").and_then(Value::as_str).unwrap_or_default();
    let provider = spec.get("provider").and_then(Value::as_str);
    let offers = model_offers(caps, model, provider);
    let on = provider.map_or(String::new(), |p| format!(" on {p}"));
    if offers.is_empty() {
        return Some(format!("T3 offers no enabled provider with model {model}{on}"));
    }
    if let Some(effort) = spec.get("effort").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        let allowed: Vec<Option<Vec<String>>> = offers.iter().map(|(_, m)| effort_values(m)).collect();
        if allowed.iter().all(|a| a.as_ref().is_some_and(|a| !a.iter().any(|v| v == effort))) {
            let first = allowed[0].clone().unwrap_or_default().join(", ");
            return Some(format!("effort {effort} is not offered for {model}{on} ({first})"));
        }
    }
    None
}

fn model_entry(m: &ModelSlot, caps: &Caps) -> Value {
    let slot = format!("model.{}", m.role);
    let Some(spec) = &m.spec else {
        let (spec, note) = match caps.get() {
            None => (Value::Null, Value::String(format!("{}, so Claude is unconfirmed", caps.why()))),
            Some(c) if !model_offers(c, DEFAULT_MODEL, Some(DEFAULT_PROVIDER)).is_empty() => {
                (json!({"provider": DEFAULT_PROVIDER, "model": DEFAULT_MODEL, "effort": DEFAULT_EFFORT}), Value::Null)
            }
            Some(_) => (Value::Null, json!("T3 offers no Claude")),
        };
        return json!({"slot": slot, "source": "default", "spec": spec, "note": note});
    };
    let mut out = json!({"slot": slot, "source": m.source, "spec": spec});
    if spec.contains_key("pool") {
        out["note"] = json!(POOL_NOTE);
        return out;
    }
    if !spec.contains_key("model") {
        out["problem"] = json!("names no model");
        return out;
    }
    match caps.get() {
        None => out["note"] = json!(format!("unverified: {}", caps.why())),
        Some(c) => {
            if let Some(p) = check_model(spec, c) {
                out["problem"] = json!(p);
            }
        }
    }
    out
}

pub fn skill_text(e: &Value) -> String {
    if e.is_null() {
        return "none".into();
    }
    let reference = e["ref"].as_str().unwrap_or_default();
    if e["status"] == "missing" {
        let others = strings(&e["others"]);
        let alt = if others.is_empty() { String::new() } else { format!(" (installed: {})", others.join(", ")) };
        return format!("SKIP {reference}: no longer resolves{alt}");
    }
    let how = if e["invoke"] == "inline" {
        format!("inline: read {} and apply it", e["path"].as_str().unwrap_or_default())
    } else {
        "Skill tool".into()
    };
    let mut t = format!("{} ({how})", e["name"].as_str().unwrap_or_default());
    if e["status"] == "ambiguous" {
        t.push_str(&format!(" [also: {}]", strings(&e["others"]).join(", ")));
    }
    t
}

pub fn skill_how(e: &Value) -> String {
    if e["status"] == "missing" {
        return skill_text(e).replacen("SKIP ", "skip: ", 1);
    }
    if e["invoke"] == "inline" {
        format!("inline: read {} and apply it", e["path"].as_str().unwrap_or_default())
    } else {
        "Skill tool".into()
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

fn model_text(m: &Value) -> String {
    let s = &m["spec"];
    let mut t = if s.is_null() {
        "provider default".to_string()
    } else if let Some(p) = s.get("pool") {
        format!("pool {}", p.as_str().unwrap_or_default())
    } else {
        let mut t = s["model"].as_str().unwrap_or("?").to_string();
        if let Some(e) = s["effort"].as_str().filter(|e| !e.is_empty()) {
            t.push_str(&format!(" effort {e}"));
        }
        if let Some(p) = s["provider"].as_str().filter(|p| !p.is_empty()) {
            t.push_str(&format!(" on {p}"));
        }
        t
    };
    if let Some(p) = m["problem"].as_str().filter(|p| !p.is_empty()) {
        t.push_str(&format!(" - PROBLEM: {p}"));
    } else if let Some(n) = m["note"].as_str().filter(|n| !n.is_empty()) {
        t.push_str(&format!(" ({n})"));
    }
    t
}

pub fn slot_line(eff: &Value, slot: &str) -> Vec<String> {
    let src = |v: &Value| v["source"].as_str().unwrap_or_default().to_string();
    match slot {
        "review" => {
            let r = &eff["review"];
            let note = r["note"].as_str().map_or(String::new(), |n| format!(" ({n})"));
            vec![format!("review: {}{note} [{}]", skill_text(&r["skill"]), src(r))]
        }
        "passes" => {
            let p = &eff["passes"];
            let list: Vec<String> = p["skills"].as_array().into_iter().flatten().map(skill_text).collect();
            let text = if list.is_empty() { "none".into() } else { list.join("; ") };
            vec![format!("passes: {text} [{}]", src(p))]
        }
        "credentials" => {
            let c = &eff["credentials"];
            let list: Vec<String> = c["renewals"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| format!("/{}/ -> {}", x["match"].as_str().unwrap_or_default(), skill_text(&x["skill"])))
                .collect();
            let text = if list.is_empty() { "none".into() } else { list.join("; ") };
            vec![format!("credentials: {text} [{}]", src(c))]
        }
        "delivery" => {
            let d = &eff["delivery"];
            let mode = d["mode"].as_str().map(String::from).unwrap_or_else(|| d["mode"].to_string());
            let mut text = mode.clone();
            if truthy(&d["skill"]) {
                text.push_str(if mode == "tag-deploy" { " via " } else { "; deploy skill " });
                text.push_str(&skill_text(&d["skill"]));
            } else if let Some(p) = d["problem"].as_str() {
                text.push_str(&format!(" via ? - PROBLEM: {p}"));
            }
            vec![format!("delivery: {text} [{}]", src(d))]
        }
        "models" => eff["models"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(role, m)| format!("model {role}: {} [{}]", model_text(m), src(m)))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn report_lines(eff: &Value) -> Vec<String> {
    let config = eff["config"].as_str().unwrap_or("none (defaults)");
    let mut lines = vec![format!("repo: {}; config: {config}", eff["repo"].as_str().unwrap_or_default())];
    for slot in SLOTS {
        lines.extend(slot_line(eff, slot));
    }
    lines
}

pub fn configured_skills(eff: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if !eff["review"]["skill"].is_null() && eff["review"]["source"] != "default" {
        out.push(("review".to_string(), eff["review"]["skill"].clone()));
    }
    for (i, s) in eff["passes"]["skills"].as_array().into_iter().flatten().enumerate() {
        out.push((format!("passes[{i}]"), s.clone()));
    }
    for x in eff["credentials"]["renewals"].as_array().into_iter().flatten() {
        out.push((format!("credentials /{}/", x["match"].as_str().unwrap_or_default()), x["skill"].clone()));
    }
    if truthy(&eff["delivery"]["skill"]) {
        out.push(("delivery".to_string(), eff["delivery"]["skill"].clone()));
    }
    out
}

pub fn skill_problems(eff: &Value) -> (Vec<String>, Vec<String>) {
    let (mut problems, mut warnings) = (Vec::new(), Vec::new());
    for (at, e) in configured_skills(eff) {
        let reference = e["ref"].as_str().unwrap_or_default();
        if e["status"] == "missing" {
            let others = strings(&e["others"]);
            let alt = if others.is_empty() { String::new() } else { format!("; installed: {}", others.join(", ")) };
            problems.push(format!("{at}: skill {reference} no longer resolves{alt}"));
        } else if e["status"] == "ambiguous" {
            warnings.push(format!(
                "{at}: {reference} matches {} and {}; record the full name",
                e["name"].as_str().unwrap_or_default(),
                strings(&e["others"]).join(", ")
            ));
        }
    }
    (problems, warnings)
}

pub fn doctor(r: &Resolved, caps: &Caps) -> (Vec<String>, Vec<String>, Value) {
    let eff = r.all(caps);
    let (problems, warnings) = doctor_eff(r.problems(), &eff);
    (problems, warnings, eff)
}

pub fn doctor_eff(mut problems: Vec<String>, eff: &Value) -> (Vec<String>, Vec<String>) {
    let (skill_problems, mut warnings) = skill_problems(eff);
    problems.extend(skill_problems);
    if let Some(p) = eff["delivery"]["problem"].as_str() {
        problems.push(format!("delivery: {p}"));
    }
    for (role, m) in eff["models"].as_object().into_iter().flatten() {
        if let Some(p) = m["problem"].as_str() {
            problems.push(format!("model {role}: {p}"));
        } else if m["source"] != "default"
            && let Some(n) = m["note"].as_str().filter(|n| !n.is_empty())
            && m["spec"].get("pool").is_none()
        {
            warnings.push(format!("model {role}: {n}"));
        }
    }
    if eff["review"]["skill"].is_null() && eff["review"]["source"] == "default" {
        warnings.push(format!("review: {}", eff["review"]["note"].as_str().unwrap_or_default()));
    }
    (problems, warnings)
}

pub fn renewal(r: &Resolved, output: &str) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    for x in r.credentials["renewals"].as_array().into_iter().flatten() {
        let pattern = x["match"].as_str().unwrap_or_default();
        match search(pattern, output) {
            Ok(false) => {}
            Ok(true) => {
                let skill = &x["skill"];
                let skipped = skill["status"] == "missing";
                let mut hit = json!({"match": pattern, "skill": skill, "how": skill_how(skill), "skipped": skipped});
                if skipped {
                    hit["reason"] = json!(format!(
                        "the renewal skill {} no longer resolves, so carry on as if no renewal were configured",
                        skill["ref"].as_str().unwrap_or_default()
                    ));
                }
                return (hit, warnings);
            }
            Err(e) => warnings.push(format!("credentials /{pattern}/: t3cm cannot compile it ({e}); skipped")),
        }
    }
    (Value::Null, warnings)
}

fn stamp_contents(h: &mut Sha256, p: &Path) {
    h.update(text(p).as_bytes());
    match std::fs::read(p) {
        Ok(raw) => {
            h.update(b"|");
            h.update(Sha256::digest(&raw));
            h.update(b"\n");
        }
        Err(_) => h.update(b"|missing\n"),
    }
}

fn stamp(h: &mut Sha256, p: &Path) {
    h.update(text(p).as_bytes());
    h.update(text(&realpath(p)).as_bytes());
    match std::fs::metadata(p) {
        Ok(m) => {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            h.update(format!("|{}|{}|{:?}\n", m.is_dir(), m.len(), mtime).as_bytes());
        }
        Err(_) => h.update(b"|missing\n"),
    }
}

pub fn fingerprint(i: &Inputs) -> String {
    let mut h = Sha256::new();
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    for s in i.listed {
        h.update(s.as_bytes());
        h.update(b",");
    }
    let f = i.folders;
    for p in [
        f.config.clone(),
        i.root.join("AGENTS.md"),
        f.claude.join("plugins").join("installed_plugins.json"),
        f.claude.join("settings.json"),
        i.root.join(".claude").join("settings.json"),
        i.root.join(".claude").join("settings.local.json"),
        f.manifest(),
    ] {
        stamp_contents(&mut h, &p);
    }
    for r in search_roots(f, i.root, i.enabled) {
        stamp(&mut h, &r.dir);
        for n in sorted_names(&r.dir) {
            let skill = r.dir.join(&n).join("SKILL.md");
            stamp(&mut h, &skill);
            stamp_contents(&mut h, &skill);
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn git_root(start: &Path) -> PathBuf {
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&start)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { start } else { PathBuf::from(s) }
        }
        _ => start,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_repr_matches_for_the_shapes_messages_use() {
        assert_eq!(py_repr(&json!("ship")), "'ship'");
        assert_eq!(py_repr(&json!("it's")), "\"it's\"");
        assert_eq!(py_repr(&json!(["a", "b"])), "['a', 'b']");
        assert_eq!(py_repr(&json!(null)), "None");
        assert_eq!(py_repr(&json!(2)), "2");
    }

    #[test]
    fn skill_refs_follow_the_python_pattern() {
        for ok in ["pr", "cbrip:sol-loop", "a/b:c.d", "x_y-z"] {
            assert!(skill_ref_ok(ok), "{ok}");
        }
        for bad in ["", "a b", "a:b:c", ":x", "x:", "a/b"] {
            assert!(!skill_ref_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_block_reads_every_slot() {
        let (cfg, problems) = parse_agent_workflow(
            "# R\n\n## Agent workflow\n\n- Review: none\n- Passes: `a`, `b`\n\
             - Credential renewal: `X|Y` -> `renew`; 401 → gh-login\n- Delivery: tag-deploy (see below)\n\
             - Deploy skill: `deploy` (`content/v*`)\n- Ticket model: pool `opus`\n\
             - Worker model: claude-opus-5-5, max effort\n\n## Other\n\n- Review: ignored\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            Value::Object(cfg),
            json!({"review": null, "passes": ["a", "b"],
                   "credentials": [{"match": "X|Y", "skill": "renew"}, {"match": "401", "skill": "gh-login"}],
                   "delivery": {"mode": "tag-deploy", "skill": "deploy"},
                   "models": {"ticket": {"pool": "opus"}, "worker": {"model": "claude-opus-5-5", "effort": "max"}}})
        );
    }

    #[test]
    fn python_only_regex_syntax_compiles() {
        assert!(search(r"(?<=token )expired", "token expired").unwrap());
        assert!(search(r"(a)\1", "aa").unwrap());
        assert!(compile("(").is_err());
        assert!(search("(?a)^token expired$", "token expired").unwrap());
        assert!(search("(?ai)^TOKEN", "token").unwrap());
        assert!(search("(?a:x)y", "xy").unwrap());
    }
}
