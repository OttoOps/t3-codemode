use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::flow_git::{LocalRepoInfo, TargetPath};

const MAX_REPO_FILES_TO_SCAN: usize = 2_000;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_DIR_FILES: usize = 64;
const MAX_REASONS_PER_EDGE: usize = 8;
const MAX_MULTILINE_STMT: usize = 40;
const MAX_BARREL_HOPS: usize = 3;
const MIN_SYMBOL_LEN: usize = 6;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    "site-packages",
    ".mypy_cache",
    ".pytest_cache",
];

const JS_TS_EXTS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

const COMMON_SYMBOL_STOPWORDS: &[&str] = &[
    "action",
    "adapter",
    "analyzer",
    "argument",
    "artifact",
    "boolean",
    "builder",
    "callback",
    "category",
    "checked",
    "client",
    "collection",
    "command",
    "compiler",
    "component",
    "config",
    "configure",
    "connect",
    "constant",
    "consumer",
    "container",
    "context",
    "create",
    "current",
    "database",
    "default",
    "delete",
    "deserialize",
    "details",
    "dictionary",
    "disabled",
    "display",
    "document",
    "duration",
    "enabled",
    "endpoint",
    "execute",
    "export",
    "factory",
    "filter",
    "fixtures",
    "format",
    "fragment",
    "function",
    "handle",
    "handler",
    "headers",
    "helper",
    "helpers",
    "import",
    "initial",
    "initialize",
    "insert",
    "interval",
    "invalid",
    "iterator",
    "listener",
    "loading",
    "manager",
    "manifest",
    "matches",
    "message",
    "metadata",
    "missing",
    "module",
    "mutation",
    "normalize",
    "number",
    "object",
    "observer",
    "option",
    "options",
    "payload",
    "pending",
    "pipeline",
    "priority",
    "process",
    "property",
    "protocol",
    "provider",
    "publish",
    "reconcile",
    "refresh",
    "registry",
    "remove",
    "render",
    "renderer",
    "reporter",
    "request",
    "resolve",
    "resolver",
    "response",
    "result",
    "return",
    "running",
    "schedule",
    "schema",
    "search",
    "selector",
    "sequence",
    "serialize",
    "server",
    "service",
    "settings",
    "severity",
    "snapshot",
    "status",
    "stopped",
    "string",
    "summary",
    "template",
    "testing",
    "timeout",
    "unknown",
    "update",
    "validate",
    "variable",
    "version",
    "warning",
    "workflow",
    "wrapper",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeDepEdge {
    pub(crate) from: (String, u64),
    pub(crate) to: (String, u64),
    pub(crate) modules: Vec<String>,
    pub(crate) symbols: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RepoIssueCodeInput {
    pub(crate) repo: String,
    pub(crate) number: u64,
    pub(crate) key: String,
    pub(crate) paths: BTreeSet<TargetPath>,
    pub(crate) body: String,
}

#[derive(Clone, Debug)]
struct ReexportEdge {
    target: String,
    /// `None` means wildcard re-export (`*`); `Some` lists specific exported identifiers.
    names: Option<HashSet<String>>,
}

#[derive(Clone, Debug)]
struct ImportRef {
    target: String,
    /// Specific symbol names requested from `target`, or `None` for whole-module/wildcard import.
    names: Option<HashSet<String>>,
}

#[derive(Clone, Debug, Default)]
struct CargoDepSpec {
    package: Option<String>,
    path: Option<String>,
    workspace: bool,
}

#[derive(Clone, Debug, Default)]
struct ParsedCargoToml {
    dir: String,
    package_name: Option<String>,
    lib_name: Option<String>,
    lib_path: Option<String>,
    workspace_members: Vec<String>,
    workspace_excludes: Vec<String>,
    workspace_deps: BTreeMap<String, CargoDepSpec>,
    normal_deps: BTreeMap<String, CargoDepSpec>,
    dev_deps: BTreeMap<String, CargoDepSpec>,
}

#[derive(Clone, Debug)]
struct CargoCrate {
    name: String,
    lib_ident: String,
    dir: String,
    lib_file: Option<String>,
    mod_to_file: HashMap<Vec<String>, String>,
    file_to_mod: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
struct JsPackage {
    name: String,
    dir: String,
    entry_file: Option<String>,
}

#[derive(Clone, Debug)]
struct TsAliasRule {
    config_dir: String,
    base_dir: String,
    prefix: String,
    suffix: String,
    targets: Vec<String>,
}

#[derive(Default)]
struct RepoCodeGraph {
    file_deps: HashMap<String, BTreeSet<String>>,
    crate_dirs: Vec<(String, String)>,
    crate_deps: HashMap<String, BTreeSet<String>>,
    crate_dev_deps: HashMap<String, BTreeSet<String>>,
    pkg_dirs: Vec<(String, String)>,
    pkg_dir_to_name: HashMap<String, String>,
    pkg_deps: HashMap<String, BTreeSet<String>>,
    pkg_dev_deps: HashMap<String, BTreeSet<String>>,
    file_to_unique_symbols: HashMap<String, Vec<String>>,
    target_file_identifiers: HashMap<String, HashSet<String>>,
    has_any_unique_symbol: bool,
}

fn is_ignored_path(rel: &str) -> bool {
    rel.split('/').any(|seg| IGNORED_DIRS.contains(&seg))
}

fn file_ext(path: &str) -> Option<&str> {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').map(|(_, ext)| ext)
}

fn is_js_ts_file(path: &str) -> bool {
    file_ext(path).is_some_and(|e| JS_TS_EXTS.contains(&e))
}

fn is_rust_file(path: &str) -> bool {
    file_ext(path) == Some("rs")
}

fn is_python_file(path: &str) -> bool {
    file_ext(path) == Some("py")
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join_rel(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    let clean = rel.replace('\\', "/");
    if clean.starts_with('/') {
        return None;
    }
    for seg in clean.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn path_in_dir(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    path == dir || (path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/')
}

fn read_capped_file(root: &Path, rel: &str) -> Option<String> {
    if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|p| p == "..") {
        return None;
    }
    let full = root.join(rel);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let file = File::open(&full).ok()?;
    let mut buf = Vec::with_capacity((meta.len().min(MAX_FILE_BYTES)) as usize);
    file.take(MAX_FILE_BYTES).read_to_end(&mut buf).ok()?;
    if buf.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_distinctive_symbol(sym: &str) -> bool {
    if sym.len() < MIN_SYMBOL_LEN {
        return false;
    }
    let bytes = sym.as_bytes();
    if !bytes.first().is_some_and(|&b| is_ident_start(b)) || !bytes.iter().all(|&b| is_ident_continue(b)) {
        return false;
    }
    if sym.starts_with('_') {
        return false;
    }
    let low = sym.to_ascii_lowercase();
    COMMON_SYMBOL_STOPWORDS.binary_search(&low.as_str()).is_err()
}

fn is_compound_code_ident(sym: &str) -> bool {
    if !is_distinctive_symbol(sym) {
        return false;
    }
    let bytes = sym.as_bytes();
    let has_underscore = bytes.contains(&b'_');
    let has_internal_upper =
        bytes.iter().skip(1).any(u8::is_ascii_uppercase) && bytes.iter().any(u8::is_ascii_lowercase);
    has_underscore || has_internal_upper
}

fn is_test_like_rust_path(path: &str, crate_dir: &str) -> bool {
    let rel = if crate_dir.is_empty() {
        path
    } else {
        path.strip_prefix(crate_dir).and_then(|s| s.strip_prefix('/')).unwrap_or(path)
    };
    rel.starts_with("tests/") || rel.starts_with("benches/") || rel.starts_with("examples/")
}

fn is_test_like_js_path(path: &str, pkg_dir: &str) -> bool {
    let rel = if pkg_dir.is_empty() {
        path
    } else {
        path.strip_prefix(pkg_dir).and_then(|s| s.strip_prefix('/')).unwrap_or(path)
    };
    if rel.starts_with("tests/")
        || rel.starts_with("test/")
        || rel.starts_with("__tests__/")
        || rel.contains("/__tests__/")
    {
        return true;
    }
    let fname = rel.rsplit('/').next().unwrap_or(rel);
    JS_TS_EXTS.iter().any(|ext| fname.ends_with(&format!(".test.{ext}")) || fname.ends_with(&format!(".spec.{ext}")))
}

fn matches_workspace_glob(pattern: &str, dir: &str) -> bool {
    let pat = pattern.trim().trim_start_matches("./").trim_end_matches('/');
    if pat.is_empty() {
        return dir.is_empty();
    }
    let pat_segs: Vec<&str> = pat.split('/').collect();
    let dir_segs: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    glob_segs_match(&pat_segs, &dir_segs)
}

fn glob_segs_match(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => glob_segs_match(&pat[1..], path) || (!path.is_empty() && glob_segs_match(pat, &path[1..])),
        (Some(p), Some(s)) => wildcard_match(p.as_bytes(), s.as_bytes()) && glob_segs_match(&pat[1..], &path[1..]),
        _ => false,
    }
}

fn wildcard_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < text.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = Some(pi);
            pi += 1;
            star_ti = ti;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

fn transitive_closure(direct: &HashMap<String, BTreeSet<String>>) -> HashMap<String, BTreeSet<String>> {
    let mut out = HashMap::new();
    for node in direct.keys() {
        let mut reachable = BTreeSet::new();
        let mut stack: Vec<String> = direct.get(node).into_iter().flatten().cloned().collect();
        while let Some(cur) = stack.pop() {
            if cur == *node || !reachable.insert(cur.clone()) {
                continue;
            }
            if let Some(nexts) = direct.get(&cur) {
                for n in nexts {
                    if !reachable.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }
        if !reachable.is_empty() {
            out.insert(node.clone(), reachable);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cargo.toml lightweight scanner
// ---------------------------------------------------------------------------

fn strip_toml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_double {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if b == b'"' {
            in_double = true;
        } else if b == b'\'' {
            in_single = true;
        } else if b == b'#' {
            return &line[..i];
        }
    }
    line
}

fn unquote_toml_str(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"));
    }
    if let Some(inner) = t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        return Some(inner.to_string());
    }
    None
}

fn parse_toml_string_array(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            let mut esc = false;
            while i < bytes.len() {
                if quote == b'"' && esc {
                    esc = false;
                    i += 1;
                    continue;
                }
                if quote == b'"' && bytes[i] == b'\\' {
                    esc = true;
                    i += 1;
                    continue;
                }
                if bytes[i] == quote {
                    break;
                }
                i += 1;
            }
            if i < bytes.len() {
                let s = &raw[start..i];
                if !s.is_empty() {
                    out.push(s.to_string());
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn split_toml_key_val(stmt: &str) -> Option<(&str, &str)> {
    let bytes = stmt.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_double {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if b == b'"' {
            in_double = true;
        } else if b == b'\'' {
            in_single = true;
        } else if b == b'=' {
            return Some((stmt[..i].trim(), stmt[i + 1..].trim()));
        }
    }
    None
}

fn parse_inline_dep_table(val: &str, spec: &mut CargoDepSpec) {
    let Some(inner) = val.trim().strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return;
    };
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if in_double {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    parts.push(&inner[start..]);
    for part in parts {
        let Some((k, v)) = split_toml_key_val(part) else { continue };
        match k {
            "path" => {
                if let Some(p) = unquote_toml_str(v) {
                    spec.path = Some(p);
                }
            }
            "package" => {
                if let Some(p) = unquote_toml_str(v) {
                    spec.package = Some(p);
                }
            }
            "workspace" if v == "true" => {
                spec.workspace = true;
            }
            _ => {}
        }
    }
}

fn split_toml_dotted_path(header: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    for ch in header.chars() {
        if in_double {
            if esc {
                cur.push(ch);
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_double = false;
            } else {
                cur.push(ch);
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                cur.push(ch);
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '.' => {
                    let t = cur.trim().to_string();
                    if !t.is_empty() {
                        segs.push(t);
                    }
                    cur.clear();
                }
                _ => cur.push(ch),
            }
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        segs.push(t);
    }
    segs
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CargoSection {
    Other,
    Package,
    Lib,
    Workspace,
    WorkspaceDeps,
    WorkspaceDep(String),
    NormalDeps,
    NormalDep(String),
    DevDeps,
    DevDep(String),
}

fn classify_cargo_section(raw_header: &str) -> CargoSection {
    let segs = split_toml_dotted_path(raw_header);
    let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
    match refs.as_slice() {
        ["package"] => CargoSection::Package,
        ["lib"] => CargoSection::Lib,
        ["workspace"] => CargoSection::Workspace,
        ["workspace", "dependencies"] => CargoSection::WorkspaceDeps,
        ["workspace", "dependencies", name] => CargoSection::WorkspaceDep((*name).to_string()),
        ["dependencies"] | ["build-dependencies"] => CargoSection::NormalDeps,
        ["dependencies", name] | ["build-dependencies", name] => CargoSection::NormalDep((*name).to_string()),
        ["dev-dependencies"] => CargoSection::DevDeps,
        ["dev-dependencies", name] => CargoSection::DevDep((*name).to_string()),
        ["target", _, "dependencies"] | ["target", _, "build-dependencies"] => CargoSection::NormalDeps,
        ["target", _, "dependencies", name] | ["target", _, "build-dependencies", name] => {
            CargoSection::NormalDep((*name).to_string())
        }
        ["target", _, "dev-dependencies"] => CargoSection::DevDeps,
        ["target", _, "dev-dependencies", name] => CargoSection::DevDep((*name).to_string()),
        _ => CargoSection::Other,
    }
}

fn bracket_balance(s: &str) -> i32 {
    let mut bal = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    for &b in s.as_bytes() {
        if in_double {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'[' | b'{' => bal += 1,
                b']' | b'}' => bal -= 1,
                _ => {}
            }
        }
    }
    bal
}

fn apply_dep_field(map: &mut BTreeMap<String, CargoDepSpec>, dep_name: &str, field: &str, val: &str) {
    let spec = map.entry(dep_name.to_string()).or_default();
    match field {
        "path" => {
            if let Some(p) = unquote_toml_str(val) {
                spec.path = Some(p);
            }
        }
        "package" => {
            if let Some(p) = unquote_toml_str(val) {
                spec.package = Some(p);
            }
        }
        "workspace" if val.trim() == "true" => {
            spec.workspace = true;
        }
        _ => {}
    }
}

fn apply_dep_entry(map: &mut BTreeMap<String, CargoDepSpec>, key: &str, val: &str) {
    let segs = split_toml_dotted_path(key);
    match segs.as_slice() {
        [dep_name] => {
            let spec = map.entry(dep_name.clone()).or_default();
            if val.trim().starts_with('{') {
                parse_inline_dep_table(val, spec);
            }
        }
        [dep_name, field] => {
            apply_dep_field(map, dep_name, field, val);
        }
        _ => {}
    }
}

fn parse_cargo_toml(dir: &str, text: &str) -> ParsedCargoToml {
    let mut parsed = ParsedCargoToml { dir: dir.to_string(), ..ParsedCargoToml::default() };
    let mut section = CargoSection::Other;
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let stripped = strip_toml_comment(lines[i]).trim();
        i += 1;
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with('[') && stripped.ends_with(']') {
            let header = stripped.trim_start_matches('[').trim_end_matches(']').trim();
            section = classify_cargo_section(header);
            continue;
        }
        let mut stmt = stripped.to_string();
        let mut bal = bracket_balance(&stmt);
        let mut joined = 0usize;
        while bal > 0 && i < lines.len() && joined < MAX_MULTILINE_STMT {
            let next = strip_toml_comment(lines[i]).trim();
            i += 1;
            joined += 1;
            if !next.is_empty() {
                stmt.push(' ');
                stmt.push_str(next);
                bal += bracket_balance(next);
            }
        }
        let Some((key, val)) = split_toml_key_val(&stmt) else { continue };
        match &section {
            CargoSection::Package => {
                if key == "name"
                    && let Some(name) = unquote_toml_str(val)
                {
                    parsed.package_name = Some(name);
                }
            }
            CargoSection::Lib => {
                if key == "name"
                    && let Some(name) = unquote_toml_str(val)
                {
                    parsed.lib_name = Some(name);
                } else if key == "path"
                    && let Some(path) = unquote_toml_str(val)
                {
                    parsed.lib_path = Some(path);
                }
            }
            CargoSection::Workspace => {
                if key == "members" {
                    parsed.workspace_members.extend(parse_toml_string_array(val));
                } else if key == "exclude" {
                    parsed.workspace_excludes.extend(parse_toml_string_array(val));
                }
            }
            CargoSection::WorkspaceDeps => apply_dep_entry(&mut parsed.workspace_deps, key, val),
            CargoSection::WorkspaceDep(dep) => apply_dep_field(&mut parsed.workspace_deps, dep, key, val),
            CargoSection::NormalDeps => apply_dep_entry(&mut parsed.normal_deps, key, val),
            CargoSection::NormalDep(dep) => apply_dep_field(&mut parsed.normal_deps, dep, key, val),
            CargoSection::DevDeps => apply_dep_entry(&mut parsed.dev_deps, key, val),
            CargoSection::DevDep(dep) => apply_dep_field(&mut parsed.dev_deps, dep, key, val),
            CargoSection::Other => {}
        }
    }
    parsed
}

// ---------------------------------------------------------------------------
// Rust source scanner
// ---------------------------------------------------------------------------

fn strip_rust_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        if block_depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_depth += 1;
                out.push_str("  ");
                i += 2;
            } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_depth -= 1;
                out.push_str("  ");
                i += 2;
            } else if bytes[i] == b'\n' {
                out.push('\n');
                i += 1;
            } else {
                let ch = src[i..].chars().next().expect("valid utf-8");
                out.push(' ');
                i += ch.len_utf8();
            }
            continue;
        }
        // Line comment
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                let ch = src[i..].chars().next().expect("valid utf-8");
                i += ch.len_utf8();
            }
            continue;
        }
        // Block comment
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_depth = 1;
            out.push_str("  ");
            i += 2;
            continue;
        }
        // Raw string literal: r"...", r#"..."#, br#"..."#, cr#"..."#
        let raw_start = if bytes[i] == b'r' {
            Some(i + 1)
        } else if (bytes[i] == b'b' || bytes[i] == b'c') && bytes.get(i + 1) == Some(&b'r') {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut j) = raw_start
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
        {
            let hash_start = j;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            if bytes.get(j) == Some(&b'"') {
                let hashes = j - hash_start;
                j += 1;
                out.push_str("\"\"");
                while j < bytes.len() {
                    if bytes[j] == b'"' {
                        let mut k = j + 1;
                        let mut matched = 0;
                        while matched < hashes && k < bytes.len() && bytes[k] == b'#' {
                            matched += 1;
                            k += 1;
                        }
                        if matched == hashes {
                            j = k;
                            break;
                        }
                    }
                    if bytes[j] == b'\n' {
                        out.push('\n');
                        j += 1;
                    } else {
                        let ch = src[j..].chars().next().expect("valid utf-8");
                        j += ch.len_utf8();
                    }
                }
                i = j;
                continue;
            }
        }
        // Normal string literal
        if bytes[i] == b'"' {
            out.push_str("\"\"");
            i += 1;
            let mut esc = false;
            while i < bytes.len() {
                if esc {
                    esc = false;
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    if ch == '\n' {
                        out.push('\n');
                    }
                    i += ch.len_utf8();
                } else if bytes[i] == b'\\' {
                    esc = true;
                    i += 1;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else {
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    i += ch.len_utf8();
                }
            }
            continue;
        }
        // Char literal like 'a' or '\'' (must not eat lifetimes like 'a or 'static)
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\\') {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' && j - i < 12 {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'\'') {
                    out.push(' ');
                    i = j + 1;
                    continue;
                }
            } else if i + 1 < bytes.len() {
                let ch = src[i + 1..].chars().next().expect("valid utf-8");
                let end = i + 1 + ch.len_utf8();
                if ch != '\n' && bytes.get(end) == Some(&b'\'') {
                    out.push(' ');
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = src[i..].chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn strip_rust_vis(s: &str) -> (bool, &str) {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix("pub") {
        let bytes = rest.as_bytes();
        if bytes.first().is_some_and(|b| b.is_ascii_whitespace()) {
            return (true, rest.trim_start());
        }
        if rest.starts_with('(')
            && let Some(close) = rest.find(')')
        {
            let scope = rest[1..close].trim();
            let after = rest[close + 1..].trim_start();
            let exported = scope != "self";
            return (exported, after);
        }
    }
    (false, t)
}

fn expand_rust_use_tree(prefix: &[String], tree: &str, out: &mut Vec<Vec<String>>) {
    let t = tree.trim();
    if t.is_empty() {
        return;
    }
    // Split top-level commas outside `{ ... }`
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in t.bytes().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&t[start..]);

    for part in parts {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(open) = p.find('{')
            && let Some(close) = p.rfind('}')
            && close > open
        {
            let head = p[..open].trim().trim_end_matches("::").trim();
            let mut next_prefix = prefix.to_vec();
            for seg in head.split("::").map(str::trim).filter(|s| !s.is_empty()) {
                next_prefix.push(seg.to_string());
            }
            expand_rust_use_tree(&next_prefix, &p[open + 1..close], out);
            continue;
        }
        // Strip `as Alias`
        let base = p.split_whitespace().next().unwrap_or(p);
        let mut full = prefix.to_vec();
        for seg in base.split("::").map(str::trim).filter(|s| !s.is_empty()) {
            if seg == "self" && !full.is_empty() {
                continue;
            }
            full.push(seg.to_string());
        }
        if !full.is_empty() {
            out.push(full);
        }
    }
}

fn extract_rust_path_attr(raw_line: &str) -> Option<String> {
    let t = raw_line.trim();
    let inner = t.strip_prefix("#[")?.strip_suffix(']')?.trim();
    let rest = inner.strip_prefix("path")?.trim();
    let val = rest.strip_prefix('=')?.trim();
    unquote_toml_str(val)
}

fn resolve_rust_mod_prefix(
    mod_to_file: &HashMap<Vec<String>, String>,
    segs: &[String],
) -> Option<(String, Option<String>)> {
    if segs.is_empty() {
        return None;
    }
    for len in (1..=segs.len()).rev() {
        let key = &segs[..len];
        if let Some(file) = mod_to_file.get(key) {
            let tail = segs.get(len).cloned();
            return Some((file.clone(), tail));
        }
    }
    None
}

fn extract_rust_file_info(
    file: &str,
    raw: &str,
    crate_info: &CargoCrate,
    dep_crates: &HashMap<String, &CargoCrate>,
    tree_files: &HashSet<String>,
) -> (Vec<ImportRef>, Vec<ReexportEdge>, Vec<String>) {
    let clean = strip_rust_comments_and_strings(raw);
    let raw_lines: Vec<&str> = raw.lines().collect();
    let lines: Vec<&str> = clean.lines().collect();
    let mut imports: Vec<ImportRef> = Vec::new();
    let mut reexports: Vec<ReexportEdge> = Vec::new();
    let mut defs: Vec<String> = Vec::new();
    let cur_mod = crate_info.file_to_mod.get(file).cloned().unwrap_or_default();
    let is_mod_dir_file = crate_info.lib_file.as_deref() == Some(file)
        || file.ends_with("/lib.rs")
        || file == "lib.rs"
        || file.ends_with("/main.rs")
        || file == "main.rs"
        || file.ends_with("/mod.rs")
        || file == "mod.rs";
    let cur_dir = parent_dir(file);
    let is_non_lib_entry = is_test_like_rust_path(file, &crate_info.dir)
        || file.ends_with("/main.rs")
        || file == "main.rs"
        || file.contains("/src/bin/");

    let resolve_path =
        |segs: &[String], is_reexport: bool, imports: &mut Vec<ImportRef>, reexports: &mut Vec<ReexportEdge>| {
            let Some(first) = segs.first().map(String::as_str) else { return };
            let resolved: Option<(String, Option<String>)> = match first {
                "crate" => resolve_rust_mod_prefix(&crate_info.mod_to_file, &segs[1..])
                    .or_else(|| crate_info.lib_file.as_ref().map(|lf| (lf.clone(), segs.get(1).cloned()))),
                "self" => {
                    let mut full = cur_mod.clone();
                    full.extend_from_slice(&segs[1..]);
                    resolve_rust_mod_prefix(&crate_info.mod_to_file, &full)
                }
                "super" => {
                    let supers = segs.iter().take_while(|s| s.as_str() == "super").count();
                    if supers <= cur_mod.len() {
                        let mut full = cur_mod[..cur_mod.len() - supers].to_vec();
                        full.extend_from_slice(&segs[supers..]);
                        resolve_rust_mod_prefix(&crate_info.mod_to_file, &full).or_else(|| {
                            (full.is_empty()).then(|| crate_info.lib_file.clone()).flatten().map(|lf| (lf, None))
                        })
                    } else {
                        None
                    }
                }
                other => {
                    if let Some(target_crate) = dep_crates.get(other) {
                        resolve_rust_mod_prefix(&target_crate.mod_to_file, &segs[1..])
                            .or_else(|| target_crate.lib_file.as_ref().map(|lf| (lf.clone(), segs.get(1).cloned())))
                    } else if is_non_lib_entry && other == crate_info.lib_ident {
                        resolve_rust_mod_prefix(&crate_info.mod_to_file, &segs[1..])
                            .or_else(|| crate_info.lib_file.as_ref().map(|lf| (lf.clone(), segs.get(1).cloned())))
                    } else {
                        let mut full = cur_mod.clone();
                        full.extend_from_slice(segs);
                        resolve_rust_mod_prefix(&crate_info.mod_to_file, &full)
                    }
                }
            };
            let Some((target, tail)) = resolved else { return };
            if target == file {
                return;
            }
            let names = match tail.as_deref() {
                Some("*") | None => None,
                Some(sym) => {
                    let mut set = HashSet::new();
                    set.insert(sym.to_string());
                    Some(set)
                }
            };
            imports.push(ImportRef { target: target.clone(), names: names.clone() });
            if is_reexport {
                reexports.push(ReexportEdge { target, names });
            }
        };

    let mut i = 0usize;
    let mut pending_path_attr: Option<String> = None;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        let raw_line = raw_lines.get(i).copied().unwrap_or("");
        if let Some(p) = extract_rust_path_attr(raw_line) {
            pending_path_attr = Some(p);
            i += 1;
            continue;
        }
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if trimmed.starts_with("#[") {
            i += 1;
            continue;
        }

        let (is_pub, after_vis) = strip_rust_vis(trimmed);

        // 1. `mod foo;`
        if let Some(rest) = after_vis.strip_prefix("mod ") {
            let rest = rest.trim_start();
            let ident_end = rest.bytes().take_while(|&b| is_ident_continue(b)).count();
            if ident_end > 0 && rest[ident_end..].trim_start().starts_with(';') {
                let mod_name = &rest[..ident_end];
                let target = if let Some(explicit) = pending_path_attr.take() {
                    join_rel(cur_dir, &explicit).filter(|p| tree_files.contains(p))
                } else {
                    let base = if is_mod_dir_file {
                        cur_dir.to_string()
                    } else {
                        let stem = file.rsplit('/').next().unwrap_or(file).strip_suffix(".rs").unwrap_or("");
                        if cur_dir.is_empty() { stem.to_string() } else { format!("{cur_dir}/{stem}") }
                    };
                    let c1 = if base.is_empty() { format!("{mod_name}.rs") } else { format!("{base}/{mod_name}.rs") };
                    let c2 = if base.is_empty() {
                        format!("{mod_name}/mod.rs")
                    } else {
                        format!("{base}/{mod_name}/mod.rs")
                    };
                    if tree_files.contains(&c1) {
                        Some(c1)
                    } else if tree_files.contains(&c2) {
                        Some(c2)
                    } else {
                        None
                    }
                };
                if let Some(t) = target
                    && t != file
                {
                    imports.push(ImportRef { target: t, names: None });
                }
                i += 1;
                continue;
            }
        }
        pending_path_attr = None;

        // 2. `use ...;` (possibly multi-line)
        if let Some(after_use) = after_vis.strip_prefix("use ") {
            let mut stmt = after_use.to_string();
            let mut joined = 0usize;
            while !stmt.contains(';') && i + 1 < lines.len() && joined < MAX_MULTILINE_STMT {
                i += 1;
                joined += 1;
                stmt.push(' ');
                stmt.push_str(lines[i].trim());
            }
            if let Some(semi) = stmt.find(';') {
                let tree = stmt[..semi].trim().trim_start_matches("::");
                let mut paths = Vec::new();
                expand_rust_use_tree(&[], tree, &mut paths);
                for segs in paths {
                    resolve_path(&segs, is_pub, &mut imports, &mut reexports);
                }
            }
            i += 1;
            continue;
        }

        // 3. Top-level public symbol definitions (column 0 only, avoids `mod tests { ... }` / impl methods)
        if !line.starts_with([' ', '\t']) {
            if is_pub {
                let mut rem = after_vis;
                if let Some(r) = rem.strip_prefix("async ") {
                    rem = r.trim_start();
                }
                if let Some(r) = rem.strip_prefix("const ") {
                    let after_const = r.trim_start();
                    if after_const.starts_with("fn ") || after_const.starts_with("unsafe ") {
                        rem = after_const;
                    }
                }
                if let Some(r) = rem.strip_prefix("unsafe ") {
                    rem = r.trim_start();
                }
                for item_kw in ["fn ", "struct ", "enum ", "trait ", "type ", "const ", "static "] {
                    if let Some(r) = rem.strip_prefix(item_kw) {
                        let r = r.trim_start();
                        let end = r.bytes().take_while(|&b| is_ident_continue(b)).count();
                        if end > 0 {
                            let sym = &r[..end];
                            if is_distinctive_symbol(sym) {
                                defs.push(sym.to_string());
                            }
                        }
                        break;
                    }
                }
            } else if let Some(r) = trimmed.strip_prefix("macro_rules!") {
                let r = r.trim_start();
                let end = r.bytes().take_while(|&b| is_ident_continue(b)).count();
                if end > 0 {
                    let sym = &r[..end];
                    if is_distinctive_symbol(sym) {
                        defs.push(sym.to_string());
                    }
                }
            }
        }

        // 4. Inline qualified `crate::...` or `<dep_lib>::...` references on this line
        scan_rust_inline_paths(line, |segs: &[String]| {
            let first = segs[0].as_str();
            if first == "crate"
                || first == "super"
                || dep_crates.contains_key(first)
                || (is_non_lib_entry && first == crate_info.lib_ident)
            {
                resolve_path(segs, false, &mut imports, &mut reexports);
            }
        });

        i += 1;
    }

    (imports, reexports, defs)
}

fn scan_rust_inline_paths(line: &str, mut on_path: impl FnMut(&[String])) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_start(bytes[i]) && (i == 0 || !is_ident_continue(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if i + 2 <= bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
                let mut segs = vec![line[start..i].to_string()];
                while i + 2 <= bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
                    let next_start = i + 2;
                    if next_start < bytes.len() && is_ident_start(bytes[next_start]) {
                        let mut k = next_start;
                        while k < bytes.len() && is_ident_continue(bytes[k]) {
                            k += 1;
                        }
                        segs.push(line[next_start..k].to_string());
                        i = k;
                    } else {
                        break;
                    }
                }
                if segs.len() >= 2 {
                    on_path(&segs);
                }
            }
        } else {
            let ch = line[i..].chars().next().expect("valid utf-8");
            i += ch.len_utf8();
        }
    }
}

// ---------------------------------------------------------------------------
// JS / TS + package.json workspace scanner
// ---------------------------------------------------------------------------

fn skip_js_regex_literal(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if bytes.get(start) != Some(&b'/')
        || matches!(bytes.get(start + 1), None | Some(b'/') | Some(b'*') | Some(b'\n') | Some(b'\r'))
    {
        return None;
    }
    let mut j = start + 1;
    let mut in_class = false;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'\n' || b == b'\r' {
            return None;
        }
        if b == b'\\' {
            j += 1;
            if j < bytes.len() && bytes[j] != b'\n' && bytes[j] != b'\r' {
                let ch = src[j..].chars().next().expect("valid utf-8");
                j += ch.len_utf8();
            }
            continue;
        }
        if in_class {
            if b == b']' {
                in_class = false;
            }
            let ch = src[j..].chars().next().expect("valid utf-8");
            j += ch.len_utf8();
            continue;
        }
        if b == b'[' {
            in_class = true;
            j += 1;
            continue;
        }
        if b == b'/' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            return Some(j);
        }
        let ch = src[j..].chars().next().expect("valid utf-8");
        j += ch.len_utf8();
    }
    None
}

fn is_js_regex_prefix_byte(b: u8) -> bool {
    matches!(
        b,
        b'(' | b'['
            | b'{'
            | b','
            | b';'
            | b':'
            | b'='
            | b'!'
            | b'&'
            | b'|'
            | b'?'
            | b'+'
            | b'-'
            | b'*'
            | b'%'
            | b'^'
            | b'~'
            | b'<'
            | b'>'
    )
}

fn is_js_regex_prefix_kw(ident: &str) -> bool {
    matches!(
        ident,
        "return"
            | "throw"
            | "case"
            | "in"
            | "of"
            | "typeof"
            | "void"
            | "delete"
            | "new"
            | "yield"
            | "await"
            | "else"
            | "do"
    )
}

struct JsLexState {
    regex_allowed: bool,
    stmt_head_kw: bool,
    paren_opens_stmt: Vec<bool>,
}

impl JsLexState {
    fn new() -> Self {
        Self { regex_allowed: true, stmt_head_kw: false, paren_opens_stmt: Vec::new() }
    }

    fn on_value(&mut self) {
        self.regex_allowed = false;
        self.stmt_head_kw = false;
    }

    fn on_ident(&mut self, ident: &str, prev_dot: bool) {
        self.stmt_head_kw = !prev_dot && matches!(ident, "if" | "while" | "for" | "with");
        self.regex_allowed = !prev_dot && is_js_regex_prefix_kw(ident);
    }

    fn on_punct(&mut self, b: u8) {
        match b {
            b'(' => {
                self.paren_opens_stmt.push(self.stmt_head_kw);
                self.stmt_head_kw = false;
                self.regex_allowed = true;
            }
            b')' => {
                self.regex_allowed = self.paren_opens_stmt.pop().unwrap_or(false);
                self.stmt_head_kw = false;
            }
            _ => {
                self.stmt_head_kw = false;
                self.regex_allowed = is_js_regex_prefix_byte(b);
            }
        }
    }
}

fn strip_js_comments(src: &str, blank_strings: bool) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let mut in_block = false;
    let mut lex = JsLexState::new();
    while i < bytes.len() {
        if in_block {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_block = false;
                out.push_str("  ");
                i += 2;
            } else if bytes[i] == b'\n' {
                out.push('\n');
                i += 1;
            } else {
                let ch = src[i..].chars().next().expect("valid utf-8");
                out.push(' ');
                i += ch.len_utf8();
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                let ch = src[i..].chars().next().expect("valid utf-8");
                i += ch.len_utf8();
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_block = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if lex.regex_allowed
            && bytes[i] == b'/'
            && let Some(end) = skip_js_regex_literal(src, i)
        {
            if blank_strings {
                out.push(' ');
            } else {
                out.push_str(&src[i..end]);
            }
            i = end;
            lex.on_value();
            continue;
        }
        if matches!(bytes[i], b'"' | b'\'' | b'`') {
            let quote = bytes[i];
            out.push(quote as char);
            i += 1;
            let mut esc = false;
            while i < bytes.len() {
                if esc {
                    esc = false;
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    if !blank_strings {
                        out.push('\\');
                        out.push(ch);
                    } else if ch == '\n' {
                        out.push('\n');
                    }
                    i += ch.len_utf8();
                } else if bytes[i] == b'\\' {
                    esc = true;
                    i += 1;
                } else if bytes[i] == quote {
                    out.push(quote as char);
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                    if quote != b'`' {
                        break;
                    }
                } else {
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    if !blank_strings {
                        out.push(ch);
                    }
                    i += ch.len_utf8();
                }
            }
            lex.on_value();
            continue;
        }
        // `++` and `--` cannot precede a regex literal (e.g. `x++ / require('./a')` is division).
        if matches!(bytes[i], b'+' | b'-') && bytes.get(i + 1) == Some(&bytes[i]) {
            out.push(bytes[i] as char);
            out.push(bytes[i] as char);
            i += 2;
            lex.on_value();
            continue;
        }
        if is_ident_start(bytes[i]) {
            let start = i;
            let prev_dot = start > 0 && bytes[start - 1] == b'.';
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &src[start..i];
            out.push_str(ident);
            lex.on_ident(ident, prev_dot);
            continue;
        }
        let b = bytes[i];
        if !b.is_ascii_whitespace() {
            lex.on_punct(b);
        }
        let ch = src[i..].chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn parse_pnpm_workspace_yaml(text: &str) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut in_packages = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            in_packages = trimmed == "packages:";
            continue;
        }
        if in_packages && let Some(item) = trimmed.strip_prefix('-') {
            let pat = item.trim().trim_matches(['"', '\'']).trim();
            if let Some(ex) = pat.strip_prefix('!') {
                if !ex.is_empty() {
                    excludes.push(ex.to_string());
                }
            } else if !pat.is_empty() {
                includes.push(pat.to_string());
            }
        }
    }
    (includes, excludes)
}

fn strip_jsonc_trailing_commas(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        if in_str {
            let ch = s[i..].chars().next().expect("valid utf-8");
            out.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            i += ch.len_utf8();
            continue;
        }
        if bytes[i] == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
            continue;
        }
        if bytes[i] == b',' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j).is_some_and(|&b| b == b'}' || b == b']') {
                i += 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn parse_tsconfig_aliases(dir: &str, text: &str) -> Vec<TsAliasRule> {
    let no_comments = strip_js_comments(text, false);
    let clean = strip_jsonc_trailing_commas(&no_comments);
    let Ok(v) = serde_json::from_str::<Value>(&clean) else {
        return Vec::new();
    };
    let Some(co) = v.get("compilerOptions") else {
        return Vec::new();
    };
    let base_url = co.get("baseUrl").and_then(Value::as_str).unwrap_or(".");
    let base_dir = join_rel(dir, base_url).unwrap_or_else(|| dir.to_string());
    let Some(paths) = co.get("paths").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (pat, targets_v) in paths {
        let targets: Vec<String> =
            targets_v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
        if targets.is_empty() {
            continue;
        }
        let (prefix, suffix) = match pat.split_once('*') {
            Some((p, s)) => (p.to_string(), s.to_string()),
            None => (pat.clone(), String::new()),
        };
        out.push(TsAliasRule { config_dir: dir.to_string(), base_dir: base_dir.clone(), prefix, suffix, targets });
    }
    out
}

fn probe_js_ts_file(base: &str, tree_files: &HashSet<String>) -> Option<String> {
    if tree_files.contains(base) {
        return Some(base.to_string());
    }
    // TypeScript NodeNext ESM convention: `.js` import -> `.ts`/`.tsx`, `.mjs` -> `.mts`, `.cjs` -> `.cts`, `.jsx` -> `.tsx`
    for (from_ext, to_exts) in
        [(".js", &[".ts", ".tsx"][..]), (".jsx", &[".tsx"][..]), (".mjs", &[".mts"][..]), (".cjs", &[".cts"][..])]
    {
        if let Some(stem) = base.strip_suffix(from_ext) {
            for to_ext in to_exts {
                let cand = format!("{stem}{to_ext}");
                if tree_files.contains(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    for ext in JS_TS_EXTS {
        let cand = format!("{base}.{ext}");
        if tree_files.contains(&cand) {
            return Some(cand);
        }
    }
    for ext in JS_TS_EXTS {
        let cand = if base.is_empty() { format!("index.{ext}") } else { format!("{base}/index.{ext}") };
        if tree_files.contains(&cand) {
            return Some(cand);
        }
    }
    None
}

fn resolve_js_specifier(
    from_file: &str,
    spec: &str,
    tree_files: &HashSet<String>,
    pkgs_by_name: &HashMap<String, JsPackage>,
    ts_aliases: &[TsAliasRule],
) -> Option<String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
        let rel = join_rel(parent_dir(from_file), spec)?;
        return probe_js_ts_file(&rel, tree_files);
    }
    // Try tsconfig path aliases scoped to this file's directory
    for rule in ts_aliases {
        if !path_in_dir(from_file, &rule.config_dir) {
            continue;
        }
        let matched = if rule.suffix.is_empty() && !rule.targets.iter().any(|t| t.contains('*')) {
            (spec == rule.prefix).then_some("")
        } else if spec.starts_with(&rule.prefix)
            && spec.ends_with(&rule.suffix)
            && spec.len() >= rule.prefix.len() + rule.suffix.len()
        {
            Some(&spec[rule.prefix.len()..spec.len() - rule.suffix.len()])
        } else {
            None
        };
        if let Some(wild) = matched {
            for target in &rule.targets {
                let sub = target.replace('*', wild);
                if let Some(joined) = join_rel(&rule.base_dir, &sub)
                    && let Some(found) = probe_js_ts_file(&joined, tree_files)
                {
                    return Some(found);
                }
            }
        }
    }
    // Try workspace packages (exact name or `<pkg>/<subpath>`)
    let mut best_pkg: Option<(&JsPackage, &str)> = None;
    for (name, pkg) in pkgs_by_name {
        if spec == name {
            best_pkg = Some((pkg, ""));
            break;
        }
        if let Some(rest) = spec.strip_prefix(name.as_str()).and_then(|s| s.strip_prefix('/'))
            && best_pkg.is_none_or(|(prev, _)| name.len() > prev.name.len())
        {
            best_pkg = Some((pkg, rest));
        }
    }
    if let Some((pkg, subpath)) = best_pkg {
        if subpath.is_empty() {
            if let Some(entry) = &pkg.entry_file {
                return Some(entry.clone());
            }
            for base in ["src/index", "index"] {
                if let Some(joined) = join_rel(&pkg.dir, base)
                    && let Some(found) = probe_js_ts_file(&joined, tree_files)
                {
                    return Some(found);
                }
            }
        } else {
            for cand_rel in [subpath.to_string(), format!("src/{subpath}")] {
                if let Some(joined) = join_rel(&pkg.dir, &cand_rel)
                    && let Some(found) = probe_js_ts_file(&joined, tree_files)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn parse_js_named_clause(clause: &str) -> Option<HashSet<String>> {
    // Handles `{ a, b as c }` (returns imported/exported source names `{a, b}`), returns `None` for `*` or default imports.
    if clause.contains('*') {
        return None;
    }
    let open = clause.find('{')?;
    let close = clause.rfind('}')?;
    if close <= open {
        return None;
    }
    let before = clause[..open].trim();
    if !before.is_empty() && before != "type" {
        // Default import alongside named import (`import Foo, { bar }`) -> don't restrict to only `{bar}`
        return None;
    }
    let mut names = HashSet::new();
    for item in clause[open + 1..close].split(',') {
        let mut t = item.trim();
        if let Some(rest) = t.strip_prefix("type ") {
            t = rest.trim();
        }
        let orig = t.split_whitespace().next().unwrap_or("");
        if !orig.is_empty() && orig.bytes().all(is_ident_continue) {
            names.insert(orig.to_string());
        }
    }
    (!names.is_empty()).then_some(names)
}

fn extract_quoted_specifier(s: &str) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut i = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return Some((&s[1..i], i + 1));
        }
        i += 1;
    }
    None
}

fn extract_js_ts_file_info(
    file: &str,
    raw: &str,
    tree_files: &HashSet<String>,
    pkgs_by_name: &HashMap<String, JsPackage>,
    ts_aliases: &[TsAliasRule],
) -> (Vec<ImportRef>, Vec<ReexportEdge>, Vec<String>) {
    let clean = strip_js_comments(raw, false);
    let lines: Vec<&str> = clean.lines().collect();
    let mut imports = Vec::new();
    let mut reexports = Vec::new();
    let mut defs = Vec::new();

    let mut add_spec = |spec: &str, names: Option<HashSet<String>>, is_reexport: bool| {
        if let Some(target) = resolve_js_specifier(file, spec, tree_files, pkgs_by_name, ts_aliases)
            && target != file
        {
            imports.push(ImportRef { target: target.clone(), names: names.clone() });
            if is_reexport {
                reexports.push(ReexportEdge { target, names });
            }
        }
    };

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 1. Static `import ...` / `export ... from ...` (with multi-line support)
        let is_import = trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
            || trimmed.starts_with("import\"")
            || trimmed.starts_with("import'");
        let is_export =
            trimmed.starts_with("export ") || trimmed.starts_with("export{") || trimmed.starts_with("export*");
        if is_import || is_export {
            let mut stmt = trimmed.to_string();
            let mut joined = 0usize;
            while is_import
                && !stmt.contains("from ")
                && !stmt.contains("from\"")
                && !stmt.contains("from'")
                && !stmt.starts_with("import \"")
                && !stmt.starts_with("import '")
                && !stmt.starts_with("import\"")
                && !stmt.starts_with("import'")
                && !stmt.contains(';')
                && i + 1 < lines.len()
                && joined < MAX_MULTILINE_STMT
            {
                i += 1;
                joined += 1;
                stmt.push(' ');
                stmt.push_str(lines[i].trim());
            }
            while is_export
                && stmt.contains('{')
                && !stmt.contains('}')
                && i + 1 < lines.len()
                && joined < MAX_MULTILINE_STMT
            {
                i += 1;
                joined += 1;
                stmt.push(' ');
                stmt.push_str(lines[i].trim());
            }

            if is_import {
                let after = stmt.strip_prefix("import").unwrap_or("").trim_start();
                if let Some((spec, _)) = extract_quoted_specifier(after) {
                    add_spec(spec, None, false);
                    i += 1;
                    continue;
                }
                if let Some((clause, spec)) = split_js_from_clause(after) {
                    let names = parse_js_named_clause(clause);
                    add_spec(spec, names, false);
                    i += 1;
                    continue;
                }
            } else if is_export {
                let after = stmt.strip_prefix("export").unwrap_or("").trim_start();
                if let Some((clause, spec)) = split_js_from_clause(after) {
                    let names = parse_js_named_clause(clause);
                    add_spec(spec, names, true);
                    i += 1;
                    continue;
                }
                // Top-level local export definitions (column 0)
                if !line.starts_with([' ', '\t']) {
                    extract_js_export_defs(after, &mut defs);
                }
            }
        }

        i += 1;
    }

    // 2. Inline `require("...")` and dynamic `import("...")` outside strings and template literals
    scan_js_call_specifiers(&clean, |spec: &str| {
        add_spec(spec, None, false);
    });

    (imports, reexports, defs)
}

fn split_js_from_clause(after_kw: &str) -> Option<(&str, &str)> {
    let bytes = after_kw.as_bytes();
    let mut i = 0usize;
    while i + 4 < bytes.len() {
        if &bytes[i..i + 4] == b"from"
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
            && !is_ident_continue(bytes[i + 4])
        {
            let rest = after_kw[i + 4..].trim_start();
            if let Some((spec, _)) = extract_quoted_specifier(rest) {
                return Some((after_kw[..i].trim(), spec));
            }
        }
        i += 1;
    }
    None
}

fn extract_js_export_defs(after_export: &str, defs: &mut Vec<String>) {
    let mut rem = after_export.trim_start();
    for kw in ["default ", "declare ", "abstract ", "async "] {
        if let Some(r) = rem.strip_prefix(kw) {
            rem = r.trim_start();
        }
    }
    for decl in ["function ", "class ", "interface ", "type ", "enum ", "const ", "let ", "var "] {
        if let Some(r) = rem.strip_prefix(decl) {
            let r = r.trim_start().trim_start_matches('*').trim_start();
            let end = r.bytes().take_while(|&b| is_ident_continue(b)).count();
            if end > 0 {
                let sym = &r[..end];
                if is_distinctive_symbol(sym) {
                    defs.push(sym.to_string());
                }
            }
            return;
        }
    }
    if rem.starts_with('{')
        && let Some(close) = rem.find('}')
    {
        for item in rem[1..close].split(',') {
            let t = item.trim();
            let parts: Vec<&str> = t.split_whitespace().collect();
            let exported = match parts.as_slice() {
                [name] => *name,
                [_, "as", alias] => *alias,
                _ => continue,
            };
            if is_distinctive_symbol(exported) {
                defs.push(exported.to_string());
            }
        }
    }
}

fn scan_js_call_specifiers(src: &str, mut on_spec: impl FnMut(&str)) {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut lex = JsLexState::new();
    while i < bytes.len() {
        // Skip JS regex literals like `/'/` so quotes inside a regex do not hide later calls
        if lex.regex_allowed
            && bytes[i] == b'/'
            && let Some(end) = skip_js_regex_literal(src, i)
        {
            i = end;
            lex.on_value();
            continue;
        }
        // Skip string and template literals so text like `'require("./storage")'` is ignored
        if matches!(bytes[i], b'"' | b'\'' | b'`') {
            let quote = bytes[i];
            i += 1;
            let mut esc = false;
            while i < bytes.len() {
                if esc {
                    esc = false;
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    i += ch.len_utf8();
                } else if bytes[i] == b'\\' {
                    esc = true;
                    i += 1;
                } else if bytes[i] == quote || (bytes[i] == b'\n' && quote != b'`') {
                    i += 1;
                    break;
                } else {
                    let ch = src[i..].chars().next().expect("valid utf-8");
                    i += ch.len_utf8();
                }
            }
            lex.on_value();
            continue;
        }
        // `++` and `--` cannot precede a regex literal (e.g. `x++ / require('./a')` is division).
        if matches!(bytes[i], b'+' | b'-') && bytes.get(i + 1) == Some(&bytes[i]) {
            i += 2;
            lex.on_value();
            continue;
        }
        if is_ident_start(bytes[i]) {
            let start = i;
            let prev_dot = start > 0 && bytes[start - 1] == b'.';
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &src[start..i];
            if !prev_dot && matches!(ident, "require" | "import") {
                let rest = src[i..].trim_start();
                if let Some(inside) = rest.strip_prefix('(') {
                    let inside = inside.trim_start();
                    if let Some((spec, _)) = extract_quoted_specifier(inside) {
                        on_spec(spec);
                    }
                }
            }
            lex.on_ident(ident, prev_dot);
            continue;
        }
        let b = bytes[i];
        if !b.is_ascii_whitespace() {
            lex.on_punct(b);
        }
        let ch = src[i..].chars().next().expect("valid utf-8");
        i += ch.len_utf8();
    }
}

// ---------------------------------------------------------------------------
// Python scanner
// ---------------------------------------------------------------------------

fn strip_python_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_double {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'\'' {
                in_single = false;
            }
        } else if b == b'"' {
            in_double = true;
        } else if b == b'\'' {
            in_single = true;
        } else if b == b'#' {
            return &line[..i];
        }
    }
    line
}

fn strip_python_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                let ch = src[i..].chars().next().expect("valid utf-8");
                i += ch.len_utf8();
            }
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            if i + 2 < bytes.len() && bytes[i + 1] == q && bytes[i + 2] == q {
                i += 3;
                out.push(' ');
                while i < bytes.len() {
                    if i + 2 < bytes.len() && bytes[i] == q && bytes[i + 1] == q && bytes[i + 2] == q {
                        i += 3;
                        break;
                    }
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        out.push('\n');
                        i += 1;
                    } else {
                        let ch = src[i..].chars().next().expect("valid utf-8");
                        i += ch.len_utf8();
                    }
                }
                continue;
            }
            i += 1;
            out.push(' ');
            while i < bytes.len() && bytes[i] != b'\n' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                let ch = src[i..].chars().next().expect("valid utf-8");
                i += ch.len_utf8();
            }
            continue;
        }
        let ch = src[i..].chars().next().expect("valid utf-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn build_python_module_map(tree_files: &HashSet<String>) -> (Vec<String>, HashMap<String, String>) {
    let mut roots: BTreeSet<String> = BTreeSet::new();
    roots.insert(String::new());
    for f in tree_files {
        if is_ignored_path(f) {
            continue;
        }
        let fname = f.rsplit('/').next().unwrap_or(f);
        if matches!(fname, "pyproject.toml" | "setup.py" | "setup.cfg") {
            let d = parent_dir(f).to_string();
            let src_d = if d.is_empty() { "src".to_string() } else { format!("{d}/src") };
            roots.insert(d);
            roots.insert(src_d);
        } else if f.starts_with("src/") && is_python_file(f) {
            roots.insert("src".to_string());
        }
    }
    // Prefer longer (more specific) roots first when computing a file's module name
    let mut root_list: Vec<String> = roots.into_iter().collect();
    root_list.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let mut mod_to_file: HashMap<String, String> = HashMap::new();
    let mut py_files: Vec<&String> = tree_files.iter().filter(|f| !is_ignored_path(f) && is_python_file(f)).collect();
    py_files.sort();
    for f in py_files {
        for r in &root_list {
            let rel = if r.is_empty() {
                f.as_str()
            } else if let Some(rest) = f.strip_prefix(r.as_str()).and_then(|s| s.strip_prefix('/')) {
                rest
            } else {
                continue;
            };
            let mod_name = if let Some(pkg) = rel.strip_suffix("/__init__.py") {
                pkg.replace('/', ".")
            } else if rel == "__init__.py" {
                continue;
            } else if let Some(stem) = rel.strip_suffix(".py") {
                stem.replace('/', ".")
            } else {
                continue;
            };
            if !mod_name.is_empty() {
                mod_to_file.entry(mod_name).or_insert_with(|| (*f).clone());
            }
        }
    }
    (root_list, mod_to_file)
}

fn resolve_python_relative(file: &str, dots: usize, mod_tail: &str, tree_files: &HashSet<String>) -> Option<String> {
    if dots == 0 {
        return None;
    }
    let dir = parent_dir(file);
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    let up = dots - 1;
    if up > parts.len() {
        return None;
    }
    parts.truncate(parts.len() - up);
    if !mod_tail.is_empty() {
        for seg in mod_tail.split('.').filter(|s| !s.is_empty()) {
            parts.push(seg);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let base = parts.join("/");
    let f1 = format!("{base}.py");
    if tree_files.contains(&f1) {
        return Some(f1);
    }
    let f2 = format!("{base}/__init__.py");
    if tree_files.contains(&f2) {
        return Some(f2);
    }
    None
}

fn extract_python_file_info(
    file: &str,
    raw: &str,
    tree_files: &HashSet<String>,
    mod_to_file: &HashMap<String, String>,
) -> (Vec<ImportRef>, Vec<ReexportEdge>, Vec<String>) {
    let mut imports = Vec::new();
    let mut reexports = Vec::new();
    let mut defs = Vec::new();
    let is_init = file == "__init__.py" || file.ends_with("/__init__.py");

    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0usize;
    let mut triple_quote: Option<&str> = None;

    while i < lines.len() {
        let raw_line = lines[i];
        // Track multiline triple-quoted docstrings/strings cleanly
        if let Some(delim) = triple_quote {
            if raw_line.contains(delim) {
                triple_quote = None;
            }
            i += 1;
            continue;
        }
        let stripped_lead = raw_line.trim_start();
        for delim in ["\"\"\"", "'''"] {
            if let Some(rest) = stripped_lead.strip_prefix(delim)
                && !rest.contains(delim)
            {
                triple_quote = Some(delim);
                break;
            }
        }
        if triple_quote.is_some() {
            i += 1;
            continue;
        }

        let code = strip_python_comment(raw_line);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Top-level defs (column 0)
        if !code.starts_with([' ', '\t']) {
            let rem = trimmed.strip_prefix("async ").unwrap_or(trimmed);
            for kw in ["def ", "class "] {
                if let Some(r) = rem.strip_prefix(kw) {
                    let r = r.trim_start();
                    let end = r.bytes().take_while(|&b| is_ident_continue(b)).count();
                    if end > 0 {
                        let sym = &r[..end];
                        if is_distinctive_symbol(sym) {
                            defs.push(sym.to_string());
                        }
                    }
                }
            }
        }

        // `import a.b, c.d as e`
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let mut stmt = rest.to_string();
            let mut joined = 0usize;
            while stmt.trim_end().ends_with('\\') && i + 1 < lines.len() && joined < MAX_MULTILINE_STMT {
                stmt = stmt.trim_end().trim_end_matches('\\').to_string();
                i += 1;
                joined += 1;
                stmt.push(' ');
                stmt.push_str(strip_python_comment(lines[i]).trim());
            }
            for item in stmt.split(',') {
                let m = item.split_whitespace().next().unwrap_or("");
                if let Some(target) = mod_to_file.get(m)
                    && target != file
                {
                    imports.push(ImportRef { target: target.clone(), names: None });
                    if is_init {
                        reexports.push(ReexportEdge { target: target.clone(), names: None });
                    }
                }
            }
            i += 1;
            continue;
        }

        // `from <mod> import <names>` (supports relative dots and parenthesized multi-line lists)
        if let Some(rest) = trimmed.strip_prefix("from ") {
            let mut stmt = rest.to_string();
            let mut joined = 0usize;
            while ((stmt.contains('(') && !stmt.contains(')')) || stmt.trim_end().ends_with('\\'))
                && i + 1 < lines.len()
                && joined < MAX_MULTILINE_STMT
            {
                stmt = stmt.trim_end().trim_end_matches('\\').to_string();
                i += 1;
                joined += 1;
                stmt.push(' ');
                stmt.push_str(strip_python_comment(lines[i]).trim());
            }
            if let Some((mod_part, names_part)) = stmt.split_once(" import ") {
                let mod_spec = mod_part.trim();
                let dots = mod_spec.bytes().take_while(|&b| b == b'.').count();
                let mod_tail = &mod_spec[dots..];
                let names_clean = names_part.trim().trim_start_matches('(').trim_end_matches(')');
                let mut imported_names: Vec<&str> = Vec::new();
                let mut is_star = false;
                for item in names_clean.split(',') {
                    let n = item.split_whitespace().next().unwrap_or("");
                    if n == "*" {
                        is_star = true;
                    } else if !n.is_empty() && n.bytes().all(is_ident_continue) {
                        imported_names.push(n);
                    }
                }
                let mut fallback_names: HashSet<String> = HashSet::new();
                for name in &imported_names {
                    let sub_target = if dots > 0 {
                        let sub = if mod_tail.is_empty() { (*name).to_string() } else { format!("{mod_tail}.{name}") };
                        resolve_python_relative(file, dots, &sub, tree_files)
                    } else {
                        let full = format!("{mod_tail}.{name}");
                        mod_to_file.get(&full).cloned()
                    };
                    if let Some(target) = sub_target
                        && target != file
                    {
                        imports.push(ImportRef { target: target.clone(), names: None });
                        if is_init {
                            let mut ns = HashSet::new();
                            ns.insert((*name).to_string());
                            reexports.push(ReexportEdge { target, names: Some(ns) });
                        }
                    } else {
                        fallback_names.insert((*name).to_string());
                    }
                }
                if is_star || !fallback_names.is_empty() {
                    let base_target = if dots > 0 {
                        resolve_python_relative(file, dots, mod_tail, tree_files)
                    } else {
                        mod_to_file.get(mod_tail).cloned()
                    };
                    if let Some(target) = base_target
                        && target != file
                    {
                        let names = if is_star { None } else { Some(fallback_names) };
                        imports.push(ImportRef { target: target.clone(), names: names.clone() });
                        if is_init {
                            reexports.push(ReexportEdge { target, names });
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        i += 1;
    }

    (imports, reexports, defs)
}

// ---------------------------------------------------------------------------
// Body & file identifier scanning (for symbol references)
// ---------------------------------------------------------------------------

fn find_inline_code_end(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut i = start;
    while i < bytes.len() && bytes[i] == b'`' {
        i += 1;
    }
    let tick_len = i - start;
    let mut j = i;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let cs = j;
            while j < bytes.len() && bytes[j] == b'`' {
                j += 1;
            }
            if j - cs == tick_len {
                return Some((i, j));
            }
        } else {
            j += 1;
        }
    }
    None
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let ch = *bytes.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = bytes.iter().take_while(|b| **b == ch).count();
    if len < 3 || (ch == b'`' && bytes[len..].contains(&b'`')) {
        return None;
    }
    Some((ch, len))
}

fn fence_closes(line: &str, ch: u8, open_len: usize) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let len = bytes.iter().take_while(|b| **b == ch).count();
    len >= open_len && bytes[len..].iter().all(|b| matches!(b, b' ' | b'\t' | b'\r'))
}

fn collect_ident_tokens(text: &str, filter: fn(&str) -> bool, out: &mut HashSet<String>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_start(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let tok = &text[start..i];
            if filter(tok) {
                out.insert(tok.to_string());
            }
        } else {
            let ch = text[i..].chars().next().expect("valid utf-8");
            i += ch.len_utf8();
        }
    }
}

fn extract_body_symbol_candidates(body: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut in_comment = false;

    for raw_line in body.lines() {
        if let Some((ch, len)) = fence {
            if fence_closes(raw_line, ch, len) {
                fence = None;
            }
            continue;
        }
        // Strip HTML comments while preserving inline code spans
        let mut line = String::with_capacity(raw_line.len());
        let mut rest = raw_line;
        loop {
            if in_comment {
                if let Some(end) = rest.find("-->") {
                    in_comment = false;
                    rest = &rest[end + 3..];
                } else {
                    break;
                }
            } else if let Some(start) = rest.find("<!--") {
                line.push_str(&rest[..start]);
                in_comment = true;
                rest = &rest[start + 4..];
            } else {
                line.push_str(rest);
                break;
            }
        }
        if !in_comment && let Some(f) = fence_marker(&line) {
            fence = Some(f);
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0usize;
        let mut plain = String::with_capacity(line.len());
        while i < bytes.len() {
            if bytes[i] == b'`'
                && let Some((content_start, end)) = find_inline_code_end(bytes, i)
            {
                let tick_len = content_start - i;
                let inner = &line[content_start..end - tick_len];
                collect_ident_tokens(inner, is_distinctive_symbol, &mut out);
                plain.push(' ');
                i = end;
                continue;
            }
            let ch = line[i..].chars().next().expect("valid utf-8");
            plain.push(ch);
            i += ch.len_utf8();
        }
        collect_ident_tokens(&plain, is_compound_code_ident, &mut out);
    }
    out
}

fn extract_code_file_identifiers(file: &str, raw: &str) -> HashSet<String> {
    let stripped = if is_rust_file(file) {
        strip_rust_comments_and_strings(raw)
    } else if is_js_ts_file(file) {
        strip_js_comments(raw, true)
    } else if is_python_file(file) {
        strip_python_comments_and_strings(raw)
    } else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    collect_ident_tokens(&stripped, is_distinctive_symbol, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Repo-wide graph assembly
// ---------------------------------------------------------------------------

struct CargoGraphData {
    crates: Vec<CargoCrate>,
    normal_deps: HashMap<String, BTreeSet<String>>,
    dev_deps: HashMap<String, BTreeSet<String>>,
}

struct JsWorkspaceData {
    /// Per consumer package dir -> visible target packages (same workspace + explicit `file:`/`link:` links).
    visible_by_pkg_dir: HashMap<String, HashMap<String, JsPackage>>,
    /// Workspace roots (longest dir first) -> member packages for files outside any named package dir.
    ws_root_pkgs: Vec<(String, HashMap<String, JsPackage>)>,
    pkg_dirs: Vec<(String, String)>,
    pkg_dir_to_name: HashMap<String, String>,
    /// Keyed by package directory so same-named packages in different workspaces never collide.
    normal_deps: HashMap<String, BTreeSet<String>>,
    dev_deps: HashMap<String, BTreeSet<String>>,
    ts_aliases: Vec<TsAliasRule>,
}

fn build_cargo_graph(root: &Path, tree_files: &HashSet<String>, scanned: &mut usize) -> CargoGraphData {
    let mut manifest_paths: Vec<&String> = tree_files
        .iter()
        .filter(|f| !is_ignored_path(f) && (*f == "Cargo.toml" || f.ends_with("/Cargo.toml")))
        .collect();
    manifest_paths.sort();

    let mut parsed_manifests: Vec<ParsedCargoToml> = Vec::new();
    for mp in manifest_paths {
        if *scanned >= MAX_REPO_FILES_TO_SCAN {
            break;
        }
        *scanned += 1;
        let Some(text) = read_capped_file(root, mp) else { continue };
        parsed_manifests.push(parse_cargo_toml(parent_dir(mp), &text));
    }
    if parsed_manifests.is_empty() {
        let has_rs = tree_files.iter().any(|f| !is_ignored_path(f) && is_rust_file(f));
        if !has_rs {
            return CargoGraphData { crates: Vec::new(), normal_deps: HashMap::new(), dev_deps: HashMap::new() };
        }
        parsed_manifests.push(ParsedCargoToml {
            dir: String::new(),
            package_name: Some("crate".to_string()),
            ..ParsedCargoToml::default()
        });
    }

    // Build workspace records (root manifests with `[workspace]`)
    let mut dir_to_pkg: HashMap<String, String> = HashMap::new();
    let mut pkg_to_dir: HashMap<String, String> = HashMap::new();
    for m in &parsed_manifests {
        if let Some(name) = &m.package_name {
            dir_to_pkg.insert(m.dir.clone(), name.clone());
            pkg_to_dir.entry(name.clone()).or_insert_with(|| m.dir.clone());
        }
    }

    // Map each manifest dir to its enclosing workspace (longest prefix workspace manifest)
    let ws_manifests: Vec<&ParsedCargoToml> =
        parsed_manifests.iter().filter(|m| !m.workspace_members.is_empty() || !m.workspace_deps.is_empty()).collect();

    let enclosing_workspace = |dir: &str| -> Option<&ParsedCargoToml> {
        ws_manifests
            .iter()
            .copied()
            .filter(|ws| {
                path_in_dir(dir, &ws.dir)
                    && !ws.workspace_excludes.iter().any(|ex| {
                        let rel = if ws.dir.is_empty() {
                            dir
                        } else {
                            dir.strip_prefix(&ws.dir).and_then(|s| s.strip_prefix('/')).unwrap_or(dir)
                        };
                        matches_workspace_glob(ex, rel)
                    })
            })
            .max_by_key(|ws| ws.dir.len())
    };

    // Determine workspace member sets per workspace dir
    let mut ws_member_names: HashMap<String, HashSet<String>> = HashMap::new();
    for ws in &ws_manifests {
        let mut names = HashSet::new();
        if let Some(root_pkg) = &ws.package_name {
            names.insert(root_pkg.clone());
        }
        for m in &parsed_manifests {
            let Some(name) = &m.package_name else { continue };
            if !path_in_dir(&m.dir, &ws.dir) {
                continue;
            }
            let rel = if ws.dir.is_empty() {
                m.dir.as_str()
            } else {
                m.dir.strip_prefix(&ws.dir).and_then(|s| s.strip_prefix('/')).unwrap_or("")
            };
            let excluded = ws.workspace_excludes.iter().any(|ex| matches_workspace_glob(ex, rel));
            if !excluded && ws.workspace_members.iter().any(|pat| matches_workspace_glob(pat, rel)) {
                names.insert(name.clone());
            }
        }
        ws_member_names.insert(ws.dir.clone(), names);
    }

    let resolve_dep = |m: &ParsedCargoToml, dep_key: &str, spec: &CargoDepSpec| -> Option<String> {
        if let Some(p) = &spec.path
            && let Some(target_dir) = join_rel(&m.dir, p)
            && let Some(target_pkg) = dir_to_pkg.get(&target_dir)
        {
            return Some(target_pkg.clone());
        }
        let ws = enclosing_workspace(&m.dir);
        if spec.workspace
            && let Some(ws) = ws
        {
            let lookup = spec.package.as_deref().unwrap_or(dep_key);
            if let Some(ws_spec) = ws.workspace_deps.get(lookup) {
                if let Some(p) = &ws_spec.path
                    && let Some(target_dir) = join_rel(&ws.dir, p)
                    && let Some(target_pkg) = dir_to_pkg.get(&target_dir)
                {
                    return Some(target_pkg.clone());
                }
                let pkg_name = ws_spec.package.as_deref().unwrap_or(lookup);
                if pkg_to_dir.contains_key(pkg_name) {
                    return Some(pkg_name.to_string());
                }
            }
        }
        let pkg_name = spec.package.as_deref().unwrap_or(dep_key);
        if let Some(ws) = ws
            && ws_member_names.get(&ws.dir).is_some_and(|set| set.contains(pkg_name))
        {
            return Some(pkg_name.to_string());
        }
        None
    };

    let mut direct_normal: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut direct_dev: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut crates: Vec<CargoCrate> = Vec::new();

    for m in &parsed_manifests {
        let Some(pkg_name) = &m.package_name else { continue };
        for (dep_key, spec) in &m.normal_deps {
            if let Some(target) = resolve_dep(m, dep_key, spec)
                && target != *pkg_name
            {
                direct_normal.entry(pkg_name.clone()).or_default().insert(target);
            }
        }
        for (dep_key, spec) in &m.dev_deps {
            if let Some(target) = resolve_dep(m, dep_key, spec)
                && target != *pkg_name
            {
                direct_dev.entry(pkg_name.clone()).or_default().insert(target);
            }
        }

        let lib_ident = m.lib_name.clone().unwrap_or_else(|| pkg_name.replace('-', "_"));
        let lib_file = m
            .lib_path
            .as_deref()
            .and_then(|p| join_rel(&m.dir, p))
            .filter(|p| tree_files.contains(p))
            .or_else(|| join_rel(&m.dir, "src/lib.rs").filter(|p| tree_files.contains(p)));
        crates.push(CargoCrate {
            name: pkg_name.clone(),
            lib_ident,
            dir: m.dir.clone(),
            lib_file,
            mod_to_file: HashMap::new(),
            file_to_mod: HashMap::new(),
        });
    }

    // Sort crates by longest dir prefix first so nested crates own their own `.rs` files
    crates.sort_by(|a, b| b.dir.len().cmp(&a.dir.len()).then_with(|| a.name.cmp(&b.name)));

    // Map each `.rs` file in `tree_files` into its owning crate's module map
    let mut rs_files: Vec<&String> = tree_files.iter().filter(|f| !is_ignored_path(f) && is_rust_file(f)).collect();
    rs_files.sort();
    for f in rs_files {
        let Some(krate) = crates.iter_mut().find(|c| path_in_dir(f, &c.dir)) else { continue };
        let rel = if krate.dir.is_empty() {
            f.as_str()
        } else {
            f.strip_prefix(&krate.dir).and_then(|s| s.strip_prefix('/')).unwrap_or(f)
        };
        if krate.lib_file.as_deref() == Some(f.as_str()) || rel == "src/lib.rs" || rel == "src/main.rs" {
            krate.file_to_mod.insert((*f).clone(), Vec::new());
            continue;
        }
        if let Some(under_src) = rel.strip_prefix("src/") {
            let mod_path: Option<Vec<String>> = if let Some(dir_part) = under_src.strip_suffix("/mod.rs") {
                Some(dir_part.split('/').map(String::from).collect())
            } else if let Some(stem) = under_src.strip_suffix(".rs") {
                if stem == "lib" || stem == "main" || stem.starts_with("bin/") {
                    None
                } else {
                    Some(stem.split('/').map(String::from).collect())
                }
            } else {
                None
            };
            if let Some(mp) = mod_path
                && !mp.is_empty()
            {
                krate.mod_to_file.entry(mp.clone()).or_insert_with(|| (*f).clone());
                krate.file_to_mod.insert((*f).clone(), mp);
            }
        }
    }

    let closed_normal = transitive_closure(&direct_normal);
    let mut closed_dev = HashMap::new();
    for (k, devs) in &direct_dev {
        let mut all = BTreeSet::new();
        for d in devs {
            all.insert(d.clone());
            if let Some(trans) = closed_normal.get(d) {
                for t in trans {
                    if t != k {
                        all.insert(t.clone());
                    }
                }
            }
        }
        if !all.is_empty() {
            closed_dev.insert(k.clone(), all);
        }
    }

    CargoGraphData { crates, normal_deps: closed_normal, dev_deps: closed_dev }
}

fn build_js_workspace_graph(root: &Path, tree_files: &HashSet<String>, scanned: &mut usize) -> JsWorkspaceData {
    let mut pkg_json_paths: Vec<&String> = tree_files
        .iter()
        .filter(|f| !is_ignored_path(f) && (*f == "package.json" || f.ends_with("/package.json")))
        .collect();
    pkg_json_paths.sort();

    struct RawPkg {
        dir: String,
        name: Option<String>,
        workspaces: Vec<String>,
        workspace_excludes: Vec<String>,
        normal_deps: Vec<(String, String)>,
        dev_deps: Vec<(String, String)>,
        entry_file: Option<String>,
    }

    let mut raw_pkgs: Vec<RawPkg> = Vec::new();
    for pp in pkg_json_paths {
        if *scanned >= MAX_REPO_FILES_TO_SCAN {
            break;
        }
        *scanned += 1;
        let Some(text) = read_capped_file(root, pp) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let dir = parent_dir(pp).to_string();
        let name = v.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let mut workspaces = Vec::new();
        let mut workspace_excludes = Vec::new();
        let mut push_ws_pat = |item: &str| {
            if let Some(ex) = item.strip_prefix('!') {
                if !ex.is_empty() {
                    workspace_excludes.push(ex.to_string());
                }
            } else if !item.is_empty() {
                workspaces.push(item.to_string());
            }
        };
        if let Some(arr) = v.get("workspaces").and_then(Value::as_array) {
            for item in arr.iter().filter_map(Value::as_str) {
                push_ws_pat(item);
            }
        } else if let Some(arr) = v.get("workspaces").and_then(|w| w.get("packages")).and_then(Value::as_array) {
            for item in arr.iter().filter_map(Value::as_str) {
                push_ws_pat(item);
            }
        }
        if dir.is_empty()
            && tree_files.contains("pnpm-workspace.yaml")
            && let Some(yaml) = read_capped_file(root, "pnpm-workspace.yaml")
        {
            let (inc, exc) = parse_pnpm_workspace_yaml(&yaml);
            workspaces.extend(inc);
            workspace_excludes.extend(exc);
        }

        let mut normal_deps = Vec::new();
        for key in ["dependencies", "peerDependencies", "optionalDependencies"] {
            if let Some(obj) = v.get(key).and_then(Value::as_object) {
                for (dn, dv) in obj {
                    normal_deps.push((dn.clone(), dv.as_str().unwrap_or_default().to_string()));
                }
            }
        }
        let mut dev_deps = Vec::new();
        if let Some(obj) = v.get("devDependencies").and_then(Value::as_object) {
            for (dn, dv) in obj {
                dev_deps.push((dn.clone(), dv.as_str().unwrap_or_default().to_string()));
            }
        }

        let mut entry_file = None;
        let entry_candidates: Vec<&str> = [
            v.get("source").and_then(Value::as_str),
            v.get("module").and_then(Value::as_str),
            v.get("main").and_then(Value::as_str),
            v.get("exports").and_then(Value::as_str),
            v.get("exports").and_then(|e| e.get(".")).and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        .collect();
        for cand in entry_candidates {
            if let Some(joined) = join_rel(&dir, cand)
                && let Some(found) = probe_js_ts_file(&joined, tree_files)
            {
                entry_file = Some(found);
                break;
            }
        }
        if entry_file.is_none() {
            for fallback in ["src/index", "index"] {
                if let Some(joined) = join_rel(&dir, fallback)
                    && let Some(found) = probe_js_ts_file(&joined, tree_files)
                {
                    entry_file = Some(found);
                    break;
                }
            }
        }

        raw_pkgs.push(RawPkg { dir, name, workspaces, workspace_excludes, normal_deps, dev_deps, entry_file });
    }

    let mut dir_to_name: HashMap<String, String> = HashMap::new();
    let mut dir_to_pkg: HashMap<String, JsPackage> = HashMap::new();
    let mut pkg_dirs: Vec<(String, String)> = Vec::new();
    let has_workspaces = raw_pkgs.iter().any(|p| !p.workspaces.is_empty());

    let mut ws_records: Vec<(String, HashSet<String>)> = Vec::new();
    for ws in raw_pkgs.iter().filter(|p| !p.workspaces.is_empty()) {
        let mut members = HashSet::new();
        if ws.name.is_some() {
            members.insert(ws.dir.clone());
        }
        for cand in &raw_pkgs {
            if cand.dir == ws.dir || !path_in_dir(&cand.dir, &ws.dir) {
                continue;
            }
            let rel = if ws.dir.is_empty() {
                cand.dir.as_str()
            } else {
                cand.dir.strip_prefix(&ws.dir).and_then(|s| s.strip_prefix('/')).unwrap_or("")
            };
            let excluded = ws.workspace_excludes.iter().any(|pat| matches_workspace_glob(pat, rel));
            if !excluded && ws.workspaces.iter().any(|pat| matches_workspace_glob(pat, rel)) {
                members.insert(cand.dir.clone());
            }
        }
        ws_records.push((ws.dir.clone(), members));
    }

    let same_workspace = |a: &str, b: &str| ws_records.iter().any(|(_, set)| set.contains(a) && set.contains(b));

    for p in &raw_pkgs {
        let Some(name) = &p.name else { continue };
        // Register virtual workspace root only for its own explicit deps if it has code; register all other named packages.
        if has_workspaces && p.dir.is_empty() && !p.workspaces.is_empty() && p.entry_file.is_none() {
            continue;
        }
        dir_to_name.insert(p.dir.clone(), name.clone());
        dir_to_pkg.insert(
            p.dir.clone(),
            JsPackage { name: name.clone(), dir: p.dir.clone(), entry_file: p.entry_file.clone() },
        );
        pkg_dirs.push((p.dir.clone(), name.clone()));
    }
    pkg_dirs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.1.cmp(&b.1)));

    let mut direct_normal: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut direct_dev: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut explicit_links_by_dir: HashMap<String, HashMap<String, JsPackage>> = HashMap::new();
    // Returns target package directory so manifest dependency identities stay isolated across workspaces.
    let resolve_js_dep_dir = |pkg: &RawPkg,
                              dep_name: &str,
                              dep_ver: &str,
                              links: &mut HashMap<String, HashMap<String, JsPackage>>|
     -> Option<String> {
        if let Some(file_rel) = dep_ver.strip_prefix("file:").or_else(|| dep_ver.strip_prefix("link:"))
            && let Some(target_dir) = join_rel(&pkg.dir, file_rel)
            && let Some(target_pkg) = dir_to_pkg.get(&target_dir)
        {
            links.entry(pkg.dir.clone()).or_default().insert(dep_name.to_string(), target_pkg.clone());
            return Some(target_dir);
        }
        for cand in &raw_pkgs {
            if cand.name.as_deref() == Some(dep_name)
                && dir_to_name.contains_key(&cand.dir)
                && same_workspace(&pkg.dir, &cand.dir)
            {
                return Some(cand.dir.clone());
            }
        }
        None
    };

    for p in &raw_pkgs {
        if !dir_to_name.contains_key(&p.dir) {
            continue;
        }
        for (dn, dv) in &p.normal_deps {
            if let Some(target_dir) = resolve_js_dep_dir(p, dn, dv, &mut explicit_links_by_dir)
                && target_dir != p.dir
            {
                direct_normal.entry(p.dir.clone()).or_default().insert(target_dir);
            }
        }
        for (dn, dv) in &p.dev_deps {
            if let Some(target_dir) = resolve_js_dep_dir(p, dn, dv, &mut explicit_links_by_dir)
                && target_dir != p.dir
            {
                direct_dev.entry(p.dir.clone()).or_default().insert(target_dir);
            }
        }
    }

    // Build workspace-scoped package maps so bare imports in workspace A cannot resolve to a same-named package in workspace B.
    let mut visible_by_pkg_dir: HashMap<String, HashMap<String, JsPackage>> = HashMap::new();
    for p in &raw_pkgs {
        if !dir_to_name.contains_key(&p.dir) {
            continue;
        }
        let mut vis: HashMap<String, JsPackage> = HashMap::new();
        for (_, members) in &ws_records {
            if !members.contains(&p.dir) {
                continue;
            }
            for mdir in members {
                if mdir == &p.dir {
                    continue;
                }
                if let Some(mpkg) = dir_to_pkg.get(mdir) {
                    vis.insert(mpkg.name.clone(), mpkg.clone());
                }
            }
        }
        if let Some(links) = explicit_links_by_dir.get(&p.dir) {
            for (dep_key, target_pkg) in links {
                vis.insert(dep_key.clone(), target_pkg.clone());
                vis.insert(target_pkg.name.clone(), target_pkg.clone());
            }
        }
        if !vis.is_empty() {
            visible_by_pkg_dir.insert(p.dir.clone(), vis);
        }
    }

    let mut ws_root_pkgs: Vec<(String, HashMap<String, JsPackage>)> = Vec::new();
    for (ws_dir, members) in &ws_records {
        let mut map = HashMap::new();
        for mdir in members {
            if let Some(mpkg) = dir_to_pkg.get(mdir) {
                map.insert(mpkg.name.clone(), mpkg.clone());
            }
        }
        if !map.is_empty() {
            ws_root_pkgs.push((ws_dir.clone(), map));
        }
    }
    ws_root_pkgs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));

    let closed_normal = transitive_closure(&direct_normal);
    let mut closed_dev = HashMap::new();
    for (k, devs) in &direct_dev {
        let mut all = BTreeSet::new();
        for d in devs {
            all.insert(d.clone());
            if let Some(trans) = closed_normal.get(d) {
                for t in trans {
                    if t != k {
                        all.insert(t.clone());
                    }
                }
            }
        }
        if !all.is_empty() {
            closed_dev.insert(k.clone(), all);
        }
    }

    // Parse tsconfig.json / jsconfig.json path aliases
    let mut ts_aliases = Vec::new();
    let mut tsconfig_paths: Vec<&String> = tree_files
        .iter()
        .filter(|f| {
            !is_ignored_path(f)
                && (*f == "tsconfig.json"
                    || f.ends_with("/tsconfig.json")
                    || *f == "jsconfig.json"
                    || f.ends_with("/jsconfig.json"))
        })
        .collect();
    tsconfig_paths.sort();
    for tp in tsconfig_paths {
        if *scanned >= MAX_REPO_FILES_TO_SCAN {
            break;
        }
        *scanned += 1;
        if let Some(text) = read_capped_file(root, tp) {
            ts_aliases.extend(parse_tsconfig_aliases(parent_dir(tp), &text));
        }
    }
    ts_aliases.sort_by_key(|b| std::cmp::Reverse(b.config_dir.len()));

    JsWorkspaceData {
        visible_by_pkg_dir,
        ws_root_pkgs,
        pkg_dirs,
        pkg_dir_to_name: dir_to_name,
        normal_deps: closed_normal,
        dev_deps: closed_dev,
        ts_aliases,
    }
}

fn expand_barrel_reexports(
    raw_imports: &HashMap<String, Vec<ImportRef>>,
    reexports: &HashMap<String, Vec<ReexportEdge>>,
) -> HashMap<String, BTreeSet<String>> {
    let mut out: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (src, refs) in raw_imports {
        let entry = out.entry(src.clone()).or_default();
        for imp in refs {
            if imp.target == *src {
                continue;
            }
            entry.insert(imp.target.clone());
            // Follow re-export barrel chains (up to MAX_BARREL_HOPS) matching imported names
            let mut frontier: Vec<(String, Option<HashSet<String>>)> = vec![(imp.target.clone(), imp.names.clone())];
            let mut visited: HashSet<String> = HashSet::new();
            visited.insert(src.clone());
            visited.insert(imp.target.clone());
            for _ in 0..MAX_BARREL_HOPS {
                let mut next_frontier = Vec::new();
                for (cur, want_names) in frontier {
                    let Some(re_list) = reexports.get(&cur) else { continue };
                    for re in re_list {
                        if re.target == *src {
                            continue;
                        }
                        let matched_names: Option<Option<HashSet<String>>> = match (&want_names, &re.names) {
                            (None, _) => Some(None),
                            (Some(want), None) => Some(Some(want.clone())),
                            (Some(want), Some(avail)) => {
                                let inter: HashSet<String> = want.intersection(avail).cloned().collect();
                                (!inter.is_empty()).then_some(Some(inter))
                            }
                        };
                        if let Some(next_names) = matched_names {
                            entry.insert(re.target.clone());
                            if visited.insert(re.target.clone()) {
                                next_frontier.push((re.target.clone(), next_names));
                            }
                        }
                    }
                }
                if next_frontier.is_empty() {
                    break;
                }
                frontier = next_frontier;
            }
        }
    }
    out
}

fn build_repo_code_graph(local: &LocalRepoInfo, target_files: &HashSet<String>) -> RepoCodeGraph {
    let root = &local.root;
    let tree_files = &local.tree_files;
    let mut scanned = 0usize;

    let CargoGraphData { crates, normal_deps: crate_deps, dev_deps: crate_dev_deps } =
        build_cargo_graph(root, tree_files, &mut scanned);
    let crate_dirs: Vec<(String, String)> = crates.iter().map(|c| (c.dir.clone(), c.name.clone())).collect();
    let crates_by_name: HashMap<&str, &CargoCrate> = crates.iter().map(|c| (c.name.as_str(), c)).collect();

    let JsWorkspaceData {
        visible_by_pkg_dir,
        ws_root_pkgs,
        pkg_dirs,
        pkg_dir_to_name,
        normal_deps: pkg_deps,
        dev_deps: pkg_dev_deps,
        ts_aliases,
    } = build_js_workspace_graph(root, tree_files, &mut scanned);
    let empty_pkgs: HashMap<String, JsPackage> = HashMap::new();

    let (_, py_mod_to_file) = build_python_module_map(tree_files);

    let mut raw_imports: HashMap<String, Vec<ImportRef>> = HashMap::new();
    let mut reexports: HashMap<String, Vec<ReexportEdge>> = HashMap::new();
    let mut symbol_definers: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut target_file_identifiers: HashMap<String, HashSet<String>> = HashMap::new();

    let mut source_files: Vec<&String> = tree_files
        .iter()
        .filter(|f| !is_ignored_path(f) && (is_rust_file(f) || is_js_ts_file(f) || is_python_file(f)))
        .collect();
    // Prioritize open issues' target files first within the bounded scan budget, deterministically sorted.
    source_files.sort_by_key(|f| (!target_files.contains(f.as_str()), f.as_str()));

    for f in source_files {
        if scanned >= MAX_REPO_FILES_TO_SCAN {
            break;
        }
        scanned += 1;
        let Some(raw) = read_capped_file(root, f) else { continue };
        if target_files.contains(f.as_str()) {
            let idents = extract_code_file_identifiers(f, &raw);
            if !idents.is_empty() {
                target_file_identifiers.insert((*f).clone(), idents);
            }
        }
        let (imps, res, defs) = if is_rust_file(f) {
            let Some(krate) = crates.iter().find(|c| path_in_dir(f, &c.dir)) else { continue };
            let mut dep_map: HashMap<String, &CargoCrate> = HashMap::new();
            for dep_pkg in crate_deps.get(&krate.name).into_iter().flatten() {
                if let Some(&dc) = crates_by_name.get(dep_pkg.as_str()) {
                    dep_map.insert(dc.lib_ident.clone(), dc);
                }
            }
            if is_test_like_rust_path(f, &krate.dir) {
                for dep_pkg in crate_dev_deps.get(&krate.name).into_iter().flatten() {
                    if let Some(&dc) = crates_by_name.get(dep_pkg.as_str()) {
                        dep_map.insert(dc.lib_ident.clone(), dc);
                    }
                }
            }
            extract_rust_file_info(f, &raw, krate, &dep_map, tree_files)
        } else if is_js_ts_file(f) {
            // Choose the innermost scope across both named package directories and workspace roots.
            let scoped_pkgs =
                match (owner_in(&pkg_dirs, f), ws_root_pkgs.iter().find(|(ws_dir, _)| path_in_dir(f, ws_dir))) {
                    (Some((pdir, _)), Some((wdir, wmap))) if wdir.len() > pdir.len() => wmap,
                    (Some((pdir, _)), _) => visible_by_pkg_dir.get(pdir).unwrap_or(&empty_pkgs),
                    (None, Some((_, wmap))) => wmap,
                    (None, None) => &empty_pkgs,
                };
            extract_js_ts_file_info(f, &raw, tree_files, scoped_pkgs, &ts_aliases)
        } else if is_python_file(f) {
            extract_python_file_info(f, &raw, tree_files, &py_mod_to_file)
        } else {
            continue;
        };

        if !imps.is_empty() {
            raw_imports.insert((*f).clone(), imps);
        }
        if !res.is_empty() {
            reexports.insert((*f).clone(), res);
        }
        for sym in defs {
            symbol_definers.entry(sym).or_default().insert((*f).clone());
        }
    }

    let file_deps = expand_barrel_reexports(&raw_imports, &reexports);
    let mut file_to_unique_symbols: HashMap<String, Vec<String>> = HashMap::new();
    for (sym, files) in symbol_definers {
        if files.len() == 1
            && let Some(owner) = files.into_iter().next()
        {
            file_to_unique_symbols.entry(owner).or_default().push(sym);
        }
    }
    for syms in file_to_unique_symbols.values_mut() {
        syms.sort();
    }
    let has_any_unique_symbol = !file_to_unique_symbols.is_empty();

    RepoCodeGraph {
        file_deps,
        crate_dirs,
        crate_deps,
        crate_dev_deps,
        pkg_dirs,
        pkg_dir_to_name,
        pkg_deps,
        pkg_dev_deps,
        file_to_unique_symbols,
        target_file_identifiers,
        has_any_unique_symbol,
    }
}

fn is_ancestor_issue(candidate_ancestor: &str, descendant: &str, parent_of: &BTreeMap<String, Vec<String>>) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![descendant.to_string()];
    while let Some(cur) = stack.pop() {
        if let Some(parents) = parent_of.get(&cur) {
            for p in parents {
                if p == candidate_ancestor {
                    return true;
                }
                if seen.insert(p.clone()) {
                    stack.push(p.clone());
                }
            }
        }
    }
    false
}

struct ExpandedIssue {
    repo: String,
    number: u64,
    key: String,
    files: BTreeSet<String>,
    /// Map of crate name -> `true` if ALL touches in that crate are test/bench/example files.
    crates: BTreeMap<String, bool>,
    /// Map of JS workspace package dir -> `true` if ALL touches in that package are test files.
    pkgs: BTreeMap<String, bool>,
    defined_symbols: Vec<(String, String)>,
    body_candidates: HashSet<String>,
    file_identifiers: HashMap<String, HashSet<String>>,
}

fn owner_in<'a>(dirs: &'a [(String, String)], path: &str) -> Option<&'a (String, String)> {
    dirs.iter().find(|(d, _)| path_in_dir(path, d))
}

fn record_unit_touch(map: &mut BTreeMap<String, bool>, name: &str, is_test: bool) {
    map.entry(name.to_string()).and_modify(|all_test| *all_test &= is_test).or_insert(is_test);
}

pub(crate) fn analyze_repo_code_deps(
    local: &LocalRepoInfo,
    issues: &[RepoIssueCodeInput],
    parent_of: &BTreeMap<String, Vec<String>>,
) -> Vec<CodeDepEdge> {
    if issues.len() < 2 || local.tree_files.is_empty() || issues.iter().all(|i| i.paths.is_empty()) {
        return Vec::new();
    }

    let mut sorted_tree_files: Vec<&String> = local.tree_files.iter().filter(|f| !is_ignored_path(f)).collect();
    sorted_tree_files.sort();

    // Pre-expand each issue's target files so the single bounded repo scan captures target identifiers without reopening files.
    let mut issue_files: Vec<BTreeSet<String>> = Vec::with_capacity(issues.len());
    let mut all_target_files: HashSet<String> = HashSet::new();
    for issue in issues {
        let mut files: BTreeSet<String> = BTreeSet::new();
        for tp in &issue.paths {
            match tp {
                TargetPath::File(f) => {
                    files.insert(f.clone());
                    all_target_files.insert(f.clone());
                }
                TargetPath::Dir(d) => {
                    let mut count = 0usize;
                    for f in &sorted_tree_files {
                        if path_in_dir(f, d) {
                            files.insert((*f).clone());
                            all_target_files.insert((*f).clone());
                            count += 1;
                            if count >= MAX_DIR_FILES {
                                break;
                            }
                        }
                    }
                }
            }
        }
        issue_files.push(files);
    }

    let graph = build_repo_code_graph(local, &all_target_files);

    let mut expanded: Vec<ExpandedIssue> = Vec::with_capacity(issues.len());
    let mut any_issue_defines_symbols = false;

    for (issue, files) in issues.iter().zip(issue_files) {
        let mut crates: BTreeMap<String, bool> = BTreeMap::new();
        let mut pkgs: BTreeMap<String, bool> = BTreeMap::new();

        for tp in &issue.paths {
            match tp {
                TargetPath::File(f) => {
                    if let Some((cdir, cname)) = owner_in(&graph.crate_dirs, f) {
                        record_unit_touch(&mut crates, cname, is_test_like_rust_path(f, cdir));
                    }
                    if let Some((pdir, _)) = owner_in(&graph.pkg_dirs, f) {
                        record_unit_touch(&mut pkgs, pdir, is_test_like_js_path(f, pdir));
                    }
                }
                TargetPath::Dir(d) => {
                    // Match crates/packages enclosing `d` OR nested inside `d`
                    for (cdir, cname) in &graph.crate_dirs {
                        if path_in_dir(d, cdir) || path_in_dir(cdir, d) {
                            let is_test = path_in_dir(d, cdir) && is_test_like_rust_path(d, cdir);
                            record_unit_touch(&mut crates, cname, is_test);
                        }
                    }
                    for (pdir, _) in &graph.pkg_dirs {
                        if path_in_dir(d, pdir) || path_in_dir(pdir, d) {
                            let is_test = path_in_dir(d, pdir) && is_test_like_js_path(d, pdir);
                            record_unit_touch(&mut pkgs, pdir, is_test);
                        }
                    }
                }
            }
        }

        let mut defined_symbols = Vec::new();
        if graph.has_any_unique_symbol {
            for f in &files {
                if let Some(syms) = graph.file_to_unique_symbols.get(f) {
                    for s in syms {
                        defined_symbols.push((f.clone(), s.clone()));
                    }
                }
            }
        }
        if !defined_symbols.is_empty() {
            any_issue_defines_symbols = true;
        }

        expanded.push(ExpandedIssue {
            repo: issue.repo.clone(),
            number: issue.number,
            key: issue.key.clone(),
            files,
            crates,
            pkgs,
            defined_symbols,
            body_candidates: HashSet::new(),
            file_identifiers: HashMap::new(),
        });
    }

    // Reuse identifiers captured during the bounded scan without reopening files from disk.
    if any_issue_defines_symbols {
        for (idx, issue) in issues.iter().enumerate() {
            expanded[idx].body_candidates = extract_body_symbol_candidates(&issue.body);
            let target_files: Vec<String> = expanded[idx].files.iter().cloned().collect();
            for f in target_files {
                if let Some(idents) = graph.target_file_identifiers.get(&f) {
                    expanded[idx].file_identifiers.insert(f, idents.clone());
                }
            }
        }
    }

    expanded
        .sort_by(|a, b| a.repo.to_ascii_lowercase().cmp(&b.repo.to_ascii_lowercase()).then(a.number.cmp(&b.number)));

    let mut edges = Vec::new();
    for b in &expanded {
        for a in &expanded {
            if b.key == a.key {
                continue;
            }
            if is_ancestor_issue(&b.key, &a.key, parent_of) || is_ancestor_issue(&a.key, &b.key, parent_of) {
                continue;
            }
            let mut modules: BTreeSet<String> = BTreeSet::new();
            let mut symbols: BTreeSet<String> = BTreeSet::new();

            // 1. File/module import edges (direct + barrel re-exports)
            for bf in &b.files {
                let Some(deps) = graph.file_deps.get(bf) else { continue };
                for af in deps {
                    if af == bf || !a.files.contains(af) {
                        continue;
                    }
                    // Skip when both issues already touch both files (covered by fileConflict)
                    if a.files.contains(bf) && b.files.contains(af) {
                        continue;
                    }
                    modules.insert(format!("{bf} -> {af}"));
                }
            }

            // 2. Cargo workspace crate dependencies
            for (bc, &bc_test_only) in &b.crates {
                for ac in a.crates.keys() {
                    if bc == ac {
                        continue;
                    }
                    if a.crates.contains_key(bc) && b.crates.contains_key(ac) {
                        continue;
                    }
                    let normal_hit = graph.crate_deps.get(bc).is_some_and(|set| set.contains(ac));
                    let dev_hit = bc_test_only && graph.crate_dev_deps.get(bc).is_some_and(|set| set.contains(ac));
                    if normal_hit || dev_hit {
                        modules.insert(format!("crate {bc} -> {ac}"));
                    }
                }
            }

            // 3. JS/TS package.json workspace dependencies (keyed by package directory)
            for (bp_dir, &bp_test_only) in &b.pkgs {
                for ap_dir in a.pkgs.keys() {
                    if bp_dir == ap_dir {
                        continue;
                    }
                    if a.pkgs.contains_key(bp_dir) && b.pkgs.contains_key(ap_dir) {
                        continue;
                    }
                    let normal_hit = graph.pkg_deps.get(bp_dir).is_some_and(|set| set.contains(ap_dir));
                    let dev_hit =
                        bp_test_only && graph.pkg_dev_deps.get(bp_dir).is_some_and(|set| set.contains(ap_dir));
                    if normal_hit || dev_hit {
                        let bp_name = graph.pkg_dir_to_name.get(bp_dir).map_or(bp_dir.as_str(), String::as_str);
                        let ap_name = graph.pkg_dir_to_name.get(ap_dir).map_or(ap_dir.as_str(), String::as_str);
                        modules.insert(format!("workspace {bp_name} -> {ap_name}"));
                    }
                }
            }

            // 4. Unique symbol definitions in A's files referenced by B's target files or B's body
            for (af, sym) in &a.defined_symbols {
                if b.files.contains(af) {
                    continue;
                }
                let in_b_files = b.file_identifiers.iter().any(|(bf, idents)| bf != af && idents.contains(sym));
                let in_b_body = !in_b_files && b.body_candidates.contains(sym);
                if in_b_files || in_b_body {
                    symbols.insert(format!("{af}::{sym}"));
                }
            }

            if !modules.is_empty() || !symbols.is_empty() {
                edges.push(CodeDepEdge {
                    from: (b.repo.clone(), b.number),
                    to: (a.repo.clone(), a.number),
                    modules: modules.into_iter().take(MAX_REASONS_PER_EDGE).collect(),
                    symbols: symbols.into_iter().take(MAX_REASONS_PER_EDGE).collect(),
                });
            }
        }
    }

    edges.sort_by(|a, b| {
        a.from
            .0
            .to_ascii_lowercase()
            .cmp(&b.from.0.to_ascii_lowercase())
            .then(a.from.1.cmp(&b.from.1))
            .then_with(|| a.to.0.to_ascii_lowercase().cmp(&b.to.0.to_ascii_lowercase()))
            .then(a.to.1.cmp(&b.to.1))
    });
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_local_repo(root: &Path, files: &[(&str, &str)]) -> LocalRepoInfo {
        let mut tree_files = HashSet::new();
        let mut tree_dirs = HashSet::new();
        for (rel, content) in files {
            let full = root.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, content).unwrap();
            let mut prefix = *rel;
            while let Some((dir, _)) = prefix.rsplit_once('/') {
                tree_dirs.insert(dir.to_string());
                prefix = dir;
            }
            tree_files.insert((*rel).to_string());
        }
        LocalRepoInfo {
            root: root.to_path_buf(),
            default_branch: Some("main".to_string()),
            default_oid: None,
            worktrees: Vec::new(),
            tree_files,
            tree_dirs,
        }
    }

    fn issue_input(number: u64, paths: &[TargetPath], body: &str) -> RepoIssueCodeInput {
        RepoIssueCodeInput {
            repo: "acme/widgets".to_string(),
            number,
            key: format!("acme/widgets#{number}"),
            paths: paths.iter().cloned().collect(),
            body: body.to_string(),
        }
    }

    #[test]
    fn rust_intra_crate_mod_use_and_barrel_reexports_emit_code_dep_without_false_edges() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                ("Cargo.toml", "[package]\nname = \"acme-demo\"\nversion = \"0.1.0\"\n"),
                ("src/lib.rs", "pub mod storage;\npub mod service;\npub mod facade;\npub mod unrelated;\n"),
                ("src/storage.rs", "pub struct LedgerRecord { pub id: u64 }\npub fn write_ledger() {}\n"),
                (
                    "src/service.rs",
                    "use crate::storage::{self, LedgerRecord};\n// use crate::unrelated;\nlet _ = \"use crate::unrelated;\";\npub fn run_billing() { crate::storage::write_ledger(); }\n",
                ),
                (
                    "src/facade/mod.rs",
                    "mod alpha;\nmod beta;\npub use self::alpha::AlphaToken;\npub use self::beta::BetaToken;\n",
                ),
                ("src/facade/alpha.rs", "pub struct AlphaToken;\n"),
                ("src/facade/beta.rs", "pub struct BetaToken;\n"),
                ("src/consumer.rs", "use crate::facade::AlphaToken;\n"),
                ("src/unrelated.rs", "pub fn noop_unrelated() {}\n"),
                ("tests/smoke.rs", "use acme_demo::storage::write_ledger;\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("src/storage.rs".into())], ""),
            issue_input(2, &[TargetPath::File("src/service.rs".into())], ""),
            issue_input(3, &[TargetPath::File("src/unrelated.rs".into())], ""),
            issue_input(4, &[TargetPath::File("src/facade/alpha.rs".into())], ""),
            issue_input(5, &[TargetPath::File("src/facade/beta.rs".into())], ""),
            issue_input(6, &[TargetPath::File("src/consumer.rs".into())], ""),
            issue_input(7, &[TargetPath::File("tests/smoke.rs".into())], ""),
            issue_input(8, &[], "Update `LedgerRecord` handling for audit exports."),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // #2 (service.rs) -> #1 (storage.rs) via `use crate::storage` and symbols
        let e21 = find(2, 1).expect("#2 -> #1");
        assert!(e21.modules.contains(&"src/service.rs -> src/storage.rs".to_string()), "{e21:?}");
        assert!(e21.symbols.contains(&"src/storage.rs::LedgerRecord".to_string()), "{e21:?}");

        // #6 (consumer.rs) -> #4 (facade/alpha.rs) via named `pub use` re-export, NOT #5 (facade/beta.rs)
        let e64 = find(6, 4).expect("#6 -> #4 via barrel re-export");
        assert!(e64.modules.contains(&"src/consumer.rs -> src/facade/alpha.rs".to_string()), "{e64:?}");
        assert!(find(6, 5).is_none(), "named re-export must not leak sibling beta.rs: {edges:?}");

        // #7 (tests/smoke.rs) -> #1 (storage.rs) via integration test `use acme_demo::storage`
        let e71 = find(7, 1).expect("#7 -> #1");
        assert!(e71.modules.contains(&"tests/smoke.rs -> src/storage.rs".to_string()), "{e71:?}");

        // #8 (body-only) -> #1 (storage.rs) via backtick-quoted unique symbol `LedgerRecord`
        let e81 = find(8, 1).expect("#8 -> #1 via body symbol");
        assert!(e81.modules.is_empty(), "{e81:?}");
        assert_eq!(e81.symbols, vec!["src/storage.rs::LedgerRecord".to_string()]);

        // #3 (unrelated.rs) must have zero incident edges (comments and string literals ignored)
        assert!(edges.iter().all(|e| e.from.1 != 3 && e.to.1 != 3), "unrelated #3 must have no edges: {edges:?}");
    }

    #[test]
    fn cargo_workspace_dependencies_and_dev_dep_scoping() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"crates/*\"]\n[workspace.dependencies]\nacme-core = { path = \"crates/core\" }\n",
                ),
                (
                    "crates/core/Cargo.toml",
                    "[package]\nname = \"acme-core\"\n[dev-dependencies]\nacme-app = { path = \"../app\" }\n",
                ),
                ("crates/core/src/lib.rs", "pub fn core_kernel() {}\n"),
                ("crates/core/tests/e2e.rs", "use acme_app::boot_server;\n"),
                (
                    "crates/mid/Cargo.toml",
                    "[package]\nname = \"acme-mid\"\n[dependencies]\nacme-core.workspace = true\n",
                ),
                ("crates/mid/src/lib.rs", "pub fn mid_layer() {}\n"),
                (
                    "crates/app/Cargo.toml",
                    "[package]\nname = \"acme-app\"\n[dependencies]\nacme-mid = { path = \"../mid\" }\n",
                ),
                ("crates/app/src/lib.rs", "pub fn boot_server() {}\n"),
                ("crates/other/Cargo.toml", "[package]\nname = \"acme-other\"\n"),
                ("crates/other/src/lib.rs", "pub fn standalone_tool() {}\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("crates/core/src/lib.rs".into())], ""),
            issue_input(2, &[TargetPath::File("crates/app/src/lib.rs".into())], ""),
            issue_input(3, &[TargetPath::File("crates/core/tests/e2e.rs".into())], ""),
            issue_input(4, &[TargetPath::Dir("crates/other".into())], ""),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // Transitive workspace crate dep: app -> mid -> core
        let e21 = find(2, 1).expect("#2 (app) -> #1 (core)");
        assert!(e21.modules.contains(&"crate acme-app -> acme-core".to_string()), "{e21:?}");

        // Normal src file in core must NOT pick up reverse dev-dependency on app
        assert!(find(1, 2).is_none(), "dev-dependency must not emit reverse edge for non-test file: {edges:?}");

        // Test file in core DOES pick up dev-dependency on app
        let e32 = find(3, 2).expect("#3 (core/tests) -> #2 (app)");
        assert!(e32.modules.contains(&"crate acme-core -> acme-app".to_string()), "{e32:?}");

        // Sibling unrelated crate gets no edges
        assert!(edges.iter().all(|e| e.from.1 != 4 && e.to.1 != 4), "{edges:?}");
    }

    #[test]
    fn js_ts_workspaces_esm_and_python_imports_emit_code_dep() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                ("package.json", r#"{"private": true, "workspaces": ["packages/*"]}"#),
                ("packages/shared/package.json", r#"{"name": "@acme/shared", "main": "src/index.ts"}"#),
                (
                    "packages/shared/src/index.ts",
                    "export { verifySession } from './auth.js';\nexport { formatMoney } from './money.js';\n",
                ),
                ("packages/shared/src/auth.ts", "export function verifySession() { return true; }\n"),
                ("packages/shared/src/money.ts", "export function formatMoney() { return '$0'; }\n"),
                (
                    "packages/web/package.json",
                    r#"{"name": "@acme/web", "dependencies": {"@acme/shared": "workspace:*"}}"#,
                ),
                (
                    "packages/web/src/login.ts",
                    "import { verifySession } from '@acme/shared';\nexport const login = verifySession;\n",
                ),
                ("pyproject.toml", "[project]\nname = \"acme-py\"\n"),
                ("src/acmelib/__init__.py", "from .models import InvoiceLedger\n"),
                ("src/acmelib/models.py", "class InvoiceLedger:\n    pass\n"),
                ("src/acmelib/views.py", "from . import InvoiceLedger\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("packages/shared/src/auth.ts".into())], ""),
            issue_input(2, &[TargetPath::File("packages/shared/src/money.ts".into())], ""),
            issue_input(3, &[TargetPath::File("packages/web/src/login.ts".into())], ""),
            issue_input(4, &[TargetPath::File("src/acmelib/models.py".into())], ""),
            issue_input(5, &[TargetPath::File("src/acmelib/views.py".into())], ""),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // #3 (web/login.ts) -> #1 (shared/auth.ts) via workspace + barrel + ESM `.js` -> `.ts`
        let e31 = find(3, 1).expect("#3 -> #1");
        assert!(e31.modules.contains(&"workspace @acme/web -> @acme/shared".to_string()), "{e31:?}");
        assert!(
            e31.modules.contains(&"packages/web/src/login.ts -> packages/shared/src/auth.ts".to_string()),
            "{e31:?}"
        );

        // #3 -> #2 gets workspace dep (web depends on @acme/shared), NOT file-level barrel leak to money.ts
        let e32 = find(3, 2).expect("#3 -> #2 via workspace dep");
        assert!(e32.modules.contains(&"workspace @acme/web -> @acme/shared".to_string()), "{e32:?}");
        assert!(!e32.modules.iter().any(|m| m.contains("money.ts")), "barrel should not leak money.ts: {e32:?}");

        // #5 (views.py) -> #4 (models.py) via relative `__init__.py` re-export
        let e54 = find(5, 4).expect("#5 -> #4");
        assert!(e54.modules.contains(&"src/acmelib/views.py -> src/acmelib/models.py".to_string()), "{e54:?}");
        assert!(e54.symbols.contains(&"src/acmelib/models.py::InvoiceLedger".to_string()), "{e54:?}");
    }

    #[test]
    fn skips_parent_child_pairs_and_handles_multibyte_utf8_and_common_words() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                ("Cargo.toml", "[package]\nname = \"utf8-demo\"\n"),
                ("src/lib.rs", "pub mod alpha;\npub mod beta;\n"),
                ("src/alpha.rs", "// résumé naïve café\npub struct Config;\npub struct CustomBillingEngine;\n"),
                ("src/beta.rs", "use crate::alpha::CustomBillingEngine;\n"),
            ],
        );

        let mut parent_of = BTreeMap::new();
        parent_of.insert("acme/widgets#2".to_string(), vec!["acme/widgets#1".to_string()]);
        let issues = vec![
            issue_input(1, &[TargetPath::File("src/alpha.rs".into())], ""),
            issue_input(2, &[TargetPath::File("src/beta.rs".into())], ""),
            issue_input(3, &[], "Update the Config documentation for résumé exports."),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &parent_of);
        assert!(edges.is_empty(), "parent-child pair and stopword Config must not emit edges: {edges:?}");
    }

    #[test]
    fn common_symbol_stopwords_are_strictly_sorted() {
        for w in COMMON_SYMBOL_STOPWORDS.windows(2) {
            assert!(w[0] < w[1], "COMMON_SYMBOL_STOPWORDS out of order: {:?} >= {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn rust_pub_const_and_js_strings_and_excluded_workspaces_are_handled_precisely() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                ("Cargo.toml", "[package]\nname = \"const-demo\"\n"),
                ("src/lib.rs", "pub mod limits;\n"),
                (
                    "src/limits.rs",
                    "pub const DEFAULT_TIMEOUT_SECS: u64 = 30;\npub const fn build_retry_limit() -> u64 { 3 }\n",
                ),
                ("package.json", r#"{"private": true, "workspaces": ["packages/*", "!packages/excluded"]}"#),
                ("packages/excluded/package.json", r#"{"name": "lodash"}"#),
                ("packages/excluded/src/index.ts", "export function fakeLodash() {}\n"),
                ("tools/outside/package.json", r#"{"name": "chalk"}"#),
                ("tools/outside/src/index.ts", "export function fakeChalk() {}\n"),
                ("tools/local-util/package.json", r#"{"name": "local-util"}"#),
                ("tools/local-util/src/index.ts", "export function localHelper() {}\n"),
                (
                    "packages/web/package.json",
                    r#"{"name": "@acme/web", "dependencies": {"lodash": "^4.17.21", "chalk": "^5.0.0", "local-util": "file:../../tools/local-util"}}"#,
                ),
                (
                    "packages/web/src/app.ts",
                    "import _ from 'lodash';\nconst doc = 'require(\"./storage\")';\nconst tpl = `import('./storage')`;\n",
                ),
                ("packages/web/src/storage.ts", "export function realStorage() {}\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("src/limits.rs".into())], ""),
            issue_input(2, &[], "Tune `DEFAULT_TIMEOUT_SECS` and `build_retry_limit`."),
            issue_input(3, &[TargetPath::File("packages/web/src/app.ts".into())], ""),
            issue_input(4, &[TargetPath::File("packages/excluded/src/index.ts".into())], ""),
            issue_input(5, &[TargetPath::File("tools/outside/src/index.ts".into())], ""),
            issue_input(6, &[TargetPath::File("tools/local-util/src/index.ts".into())], ""),
            issue_input(7, &[TargetPath::File("packages/web/src/storage.ts".into())], ""),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // S4: pub const and pub const fn both emit symbol hints
        let e21 = find(2, 1).expect("#2 -> #1 via pub const + pub const fn");
        assert!(e21.symbols.contains(&"src/limits.rs::DEFAULT_TIMEOUT_SECS".to_string()), "{e21:?}");
        assert!(e21.symbols.contains(&"src/limits.rs::build_retry_limit".to_string()), "{e21:?}");

        // S2: excluded workspace pkg (#4) and outside-workspace pkg (#5) do NOT match registry deps,
        // while explicit file: dep (#6) DOES match
        assert!(find(3, 4).is_none(), "excluded workspace pkg must not match registry dep: {edges:?}");
        assert!(find(3, 5).is_none(), "outside-workspace pkg must not match registry dep: {edges:?}");
        let e36 = find(3, 6).expect("#3 -> #6 via explicit file: dep");
        assert!(e36.modules.contains(&"workspace @acme/web -> local-util".to_string()), "{e36:?}");

        // S3: require/import inside string or template literals must NOT emit import edge to #7
        assert!(find(3, 7).is_none(), "string/template literal require/import must be ignored: {edges:?}");
    }

    #[test]
    fn js_bare_imports_stay_workspace_scoped_and_regex_quotes_do_not_hide_require_and_standalone_rust_script_captures_symbols()
     {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                // S1: Rust file outside any Cargo package dir referencing a unique symbol in crates/core
                ("crates/core/Cargo.toml", "[package]\nname = \"acme-core\"\n"),
                ("crates/core/src/lib.rs", "pub struct StandaloneScriptSymbol;\n"),
                ("scripts/check.rs", "fn main() { let _ = StandaloneScriptSymbol; }\n"),
                // S2: Two separate JS workspaces: ws-a imports registry "foo", must NOT match ws-b's local "foo"
                ("ws-a/package.json", r#"{"private": true, "workspaces": ["packages/*"]}"#),
                ("ws-a/packages/web/package.json", r#"{"name": "@a/web", "dependencies": {"foo": "^1.0.0"}}"#),
                ("ws-a/packages/web/src/app.ts", "import { something } from 'foo';\n"),
                ("ws-b/package.json", r#"{"private": true, "workspaces": ["packages/*"]}"#),
                ("ws-b/packages/foo/package.json", r#"{"name": "foo"}"#),
                ("ws-b/packages/foo/src/index.ts", "export function localFooFromB() {}\n"),
                ("ws-b/packages/cli/package.json", r#"{"name": "@b/cli", "dependencies": {"foo": "workspace:*"}}"#),
                ("ws-b/packages/cli/src/index.ts", "import { localFooFromB } from 'foo';\n"),
                // S7: JS regex containing single quote before require('./storage') on next line and same line
                ("ws-a/packages/web/src/regex_caller.js", "const quote = /'/;\nconst s = require('./storage');\n"),
                ("ws-a/packages/web/src/storage.js", "export const STORAGE_KEY = 'k';\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("crates/core/src/lib.rs".into())], ""),
            issue_input(2, &[TargetPath::File("scripts/check.rs".into())], ""),
            issue_input(3, &[TargetPath::File("ws-a/packages/web/src/app.ts".into())], ""),
            issue_input(4, &[TargetPath::File("ws-b/packages/foo/src/index.ts".into())], ""),
            issue_input(5, &[TargetPath::File("ws-b/packages/cli/src/index.ts".into())], ""),
            issue_input(6, &[TargetPath::File("ws-a/packages/web/src/regex_caller.js".into())], ""),
            issue_input(7, &[TargetPath::File("ws-a/packages/web/src/storage.js".into())], ""),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // S1: standalone Rust script outside any Cargo crate dir still gets symbol-reference hint to #1
        let e21 = find(2, 1).expect("#2 (scripts/check.rs) -> #1 via StandaloneScriptSymbol");
        assert!(e21.symbols.contains(&"crates/core/src/lib.rs::StandaloneScriptSymbol".to_string()), "{e21:?}");

        // S2: ws-a's bare `import from 'foo'` must NOT match ws-b's local `foo` (#4),
        // while ws-b's own member #5 DOES match #4
        assert!(find(3, 4).is_none(), "cross-workspace bare import must not resolve to ws-b foo: {edges:?}");
        let e54 = find(5, 4).expect("#5 (@b/cli) -> #4 (ws-b foo)");
        assert!(
            e54.modules.contains(&"ws-b/packages/cli/src/index.ts -> ws-b/packages/foo/src/index.ts".to_string()),
            "{e54:?}"
        );

        // S7: regex literal `/'/` must not hide `require('./storage')` on the next line
        let e67 = find(6, 7).expect("#6 -> #7 via require('./storage') after /'/ regex");
        assert!(
            e67.modules
                .contains(&"ws-a/packages/web/src/regex_caller.js -> ws-a/packages/web/src/storage.js".to_string()),
            "{e67:?}"
        );
    }

    #[test]
    fn duplicate_pkg_names_nested_unnamed_workspace_and_if_regex_or_postfix_division_work() {
        let td = tempfile::tempdir().unwrap();
        let local = make_local_repo(
            td.path(),
            &[
                // S2: Both workspaces define a package named "web"; only ws-b's "web" depends on ws-b's "foo"
                ("ws-a/package.json", r#"{"private": true, "workspaces": ["packages/*"]}"#),
                ("ws-a/packages/web/package.json", r#"{"name": "web"}"#),
                ("ws-a/packages/web/src/index.ts", "export const WS_A_ONLY = 1;\n"),
                ("ws-b/package.json", r#"{"private": true, "workspaces": ["packages/*"]}"#),
                ("ws-b/packages/foo/package.json", r#"{"name": "foo"}"#),
                ("ws-b/packages/foo/src/index.ts", "export const WS_B_FOO = 2;\n"),
                ("ws-b/packages/web/package.json", r#"{"name": "web", "dependencies": {"foo": "workspace:*"}}"#),
                ("ws-b/packages/web/src/index.ts", "export const WS_B_WEB = 3;\n"),
                // S9: Unnamed nested workspace inside a named outer package
                ("outer/package.json", r#"{"name": "outer-pkg", "main": "index.js"}"#),
                ("outer/index.js", "export const OUTER_VAL = 1;\n"),
                ("outer/nested/package.json", r#"{"private": true, "workspaces": ["pkgs/*"]}"#),
                ("outer/nested/pkgs/core/package.json", r#"{"name": "nested-core", "main": "index.ts"}"#),
                ("outer/nested/pkgs/core/index.ts", "export function nestedCoreFn() {}\n"),
                ("outer/nested/run.ts", "import { nestedCoreFn } from 'nested-core';\n"),
                // S7 & S8: regex after `if (flag)` and division after `x++` before `require('./storage')`
                ("js/if_regex.js", "if (flag) /'/.test(value); require('./storage');\n"),
                ("js/postfix_div.js", "const value = x++ / require('./storage');\n"),
                ("js/storage.js", "module.exports = 42;\n"),
            ],
        );

        let issues = vec![
            issue_input(1, &[TargetPath::File("ws-a/packages/web/src/index.ts".into())], ""),
            issue_input(2, &[TargetPath::File("ws-b/packages/foo/src/index.ts".into())], ""),
            issue_input(3, &[TargetPath::File("ws-b/packages/web/src/index.ts".into())], ""),
            issue_input(4, &[TargetPath::File("outer/nested/run.ts".into())], ""),
            issue_input(5, &[TargetPath::File("outer/nested/pkgs/core/index.ts".into())], ""),
            issue_input(6, &[TargetPath::File("js/if_regex.js".into())], ""),
            issue_input(7, &[TargetPath::File("js/postfix_div.js".into())], ""),
            issue_input(8, &[TargetPath::File("js/storage.js".into())], ""),
        ];

        let edges = analyze_repo_code_deps(&local, &issues, &BTreeMap::new());
        let find = |from: u64, to: u64| edges.iter().find(|e| e.from.1 == from && e.to.1 == to);

        // S2: ws-a's "web" (#1) must NOT inherit ws-b's "web" -> "foo" (#2) dependency, while ws-b's "web" (#3) DOES
        assert!(find(1, 2).is_none(), "ws-a web must not inherit ws-b web->foo dep: {edges:?}");
        let e32 = find(3, 2).expect("ws-b web (#3) -> ws-b foo (#2)");
        assert!(e32.modules.contains(&"workspace web -> foo".to_string()), "{e32:?}");

        // S9: script at unnamed nested workspace root picks nested workspace scope over enclosing outer package
        let e45 = find(4, 5).expect("nested root script (#4) -> nested-core (#5)");
        assert!(e45.modules.contains(&"outer/nested/run.ts -> outer/nested/pkgs/core/index.ts".to_string()), "{e45:?}");

        // S7: `if (flag) /'/.test(value); require('./storage');` finds require('./storage')
        let e68 = find(6, 8).expect("if_regex (#6) -> storage (#8)");
        assert!(e68.modules.contains(&"js/if_regex.js -> js/storage.js".to_string()), "{e68:?}");

        // S8: `const value = x++ / require('./storage');` treats `/` as division and finds require('./storage')
        let e78 = find(7, 8).expect("postfix_div (#7) -> storage (#8)");
        assert!(e78.modules.contains(&"js/postfix_div.js -> js/storage.js".to_string()), "{e78:?}");
    }
}
