use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::board::parse_github_issue_only;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_GRAPHQL_ISSUE_NUMBER: u64 = i32::MAX as u64;
const EXCLUDED_BRANCH_PREFIXES: &[&str] = &["pr", "pull", "pulls", "release", "releases", "v", "tag", "tags"];
const WELL_KNOWN_ROOT_FILES: &[&str] = &["Makefile", "Dockerfile", "Justfile", "Procfile", "Gemfile", "Rakefile"];

#[derive(Clone, Debug, Default)]
pub(crate) struct OpenPr {
    pub(crate) repo: String,
    pub(crate) number: u64,
    pub(crate) body: String,
    pub(crate) base_ref_name: String,
    pub(crate) head_ref_name: String,
    pub(crate) head_ref_oid: String,
    pub(crate) is_cross_repository: bool,
    pub(crate) closing_issues: Vec<(String, u64)>,
    pub(crate) files: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RepoPrs {
    pub(crate) default_branch: Option<String>,
    pub(crate) prs: Vec<OpenPr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeEntry {
    pub(crate) path: PathBuf,
    pub(crate) branch: Option<String>,
    pub(crate) head_oid: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LocalRepoInfo {
    pub(crate) root: PathBuf,
    pub(crate) default_branch: Option<String>,
    pub(crate) default_oid: Option<String>,
    pub(crate) worktrees: Vec<WorktreeEntry>,
    pub(crate) tree_files: HashSet<String>,
    pub(crate) tree_dirs: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrStackEdge {
    pub(crate) from: (String, u64),
    pub(crate) to: (String, u64),
    pub(crate) branch: Option<String>,
    pub(crate) base_branch: Option<String>,
    pub(crate) pr: Option<u64>,
    pub(crate) base_pr: Option<u64>,
    pub(crate) part: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileConflictEdge {
    pub(crate) from: (String, u64),
    pub(crate) to: (String, u64),
    pub(crate) files: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ActivePartLink {
    pub(crate) scope: String,
    pub(crate) part_num: u32,
    pub(crate) part_total: u32,
    pub(crate) issue: (String, u64),
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveBranchLink {
    pub(crate) repo: Option<String>,
    pub(crate) pr_number: Option<u64>,
    pub(crate) branch: Option<String>,
    pub(crate) worktree_path: Option<PathBuf>,
    pub(crate) issue: (String, u64),
}

#[derive(Clone, Debug)]
pub(crate) struct IssueFileInput<'a> {
    pub(crate) repo: &'a str,
    pub(crate) number: u64,
    pub(crate) touches: &'a [String],
    pub(crate) body: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TargetPath {
    File(String),
    Dir(String),
}

fn issue_key(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo.to_ascii_lowercase())
}

fn canonicalize_repo(aliases: &HashMap<String, String>, repo: &str) -> String {
    let mut cur = repo.to_string();
    for _ in 0..4 {
        let Some(next) = aliases.get(&cur.to_ascii_lowercase()) else { break };
        if next.eq_ignore_ascii_case(&cur) {
            break;
        }
        cur = next.clone();
    }
    cur
}

pub(crate) fn parse_open_pr(repo: &str, v: &Value) -> Option<OpenPr> {
    let number = v["number"].as_u64()?;
    let base_ref_name = v["baseRefName"].as_str().unwrap_or_default().to_string();
    let head_ref_name = v["headRefName"].as_str().unwrap_or_default().to_string();
    let head_ref_oid = v["headRefOid"].as_str().unwrap_or_default().to_string();
    let is_cross_repository = v["isCrossRepository"].as_bool().unwrap_or(false);
    let body = v["body"].as_str().unwrap_or_default().to_string();
    let mut closing_issues = Vec::new();
    for n in v["closingIssuesReferences"]["nodes"].as_array().into_iter().flatten() {
        let Some(num) = n["number"].as_u64().filter(|&x| x > 0 && x <= MAX_GRAPHQL_ISSUE_NUMBER) else {
            continue;
        };
        let irepo = n["repository"]["nameWithOwner"].as_str().unwrap_or(repo).to_string();
        if !closing_issues.iter().any(|(r, k): &(String, u64)| *k == num && r.eq_ignore_ascii_case(&irepo)) {
            closing_issues.push((irepo, num));
        }
    }
    let mut files = Vec::new();
    for f in v["files"]["nodes"].as_array().into_iter().flatten() {
        if let Some(raw) = f["path"].as_str()
            && let Some((norm, _)) = normalize_repo_path(raw)
            && !files.iter().any(|x| x == &norm)
        {
            files.push(norm);
        }
    }
    Some(OpenPr {
        repo: repo.to_string(),
        number,
        body,
        base_ref_name,
        head_ref_name,
        head_ref_oid,
        is_cross_repository,
        closing_issues,
        files,
    })
}

pub(crate) fn parse_part(raw: &str) -> Option<(u32, u32)> {
    let (a, b) = raw.trim().split_once('/')?;
    let n: u32 = a.trim().parse().ok()?;
    let total: u32 = b.trim().parse().ok()?;
    (n >= 1 && n <= total && total >= 2).then_some((n, total))
}

pub(crate) fn parse_branch_issue(branch: &str) -> Option<u64> {
    let segs: Vec<&str> = branch.trim().split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() < 2 {
        return None;
    }
    for pair in segs.windows(2) {
        let prefix = pair[0];
        let tail = pair[1];
        if !prefix.bytes().any(|b| b.is_ascii_alphabetic())
            || EXCLUDED_BRANCH_PREFIXES.iter().any(|ex| prefix.eq_ignore_ascii_case(ex))
        {
            continue;
        }
        let rest = if tail.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("issue-")) { &tail[6..] } else { tail };
        if rest.is_empty() || rest.starts_with('0') {
            continue;
        }
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        let after = rest.as_bytes().get(digits).copied();
        if after.is_some_and(|b| !matches!(b, b'-' | b'_' | b'.')) {
            continue;
        }
        if let Ok(num) = rest[..digits].parse::<u64>()
            && (1..=MAX_GRAPHQL_ISSUE_NUMBER).contains(&num)
        {
            return Some(num);
        }
    }
    None
}

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

fn strip_html_comments(line: &str, in_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        if *in_comment {
            if let Some(end) = rest.find("-->") {
                *in_comment = false;
                rest = &rest[end + 3..];
            } else {
                break;
            }
        } else {
            let bytes = rest.as_bytes();
            let mut i = 0;
            let mut started = false;
            while i < bytes.len() {
                if bytes[i] == b'`' {
                    if let Some((_, end)) = find_inline_code_end(bytes, i) {
                        out.push_str(&rest[i..end]);
                        i = end;
                    } else {
                        let start = i;
                        while i < bytes.len() && bytes[i] == b'`' {
                            i += 1;
                        }
                        out.push_str(&rest[start..i]);
                    }
                } else if rest[i..].starts_with("<!--") {
                    *in_comment = true;
                    rest = &rest[i + 4..];
                    started = true;
                    break;
                } else {
                    let ch = rest[i..].chars().next().expect("valid utf-8");
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
            if !started {
                break;
            }
        }
    }
    out
}

fn strip_inline_code(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some((_, end)) = find_inline_code_end(bytes, i) {
                out.push(' ');
                i = end;
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                out.push_str(&line[start..i]);
            }
        } else {
            let ch = line[i..].chars().next().expect("valid utf-8");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let ch = *bytes.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = bytes.iter().take_while(|b| **b == ch).count();
    if len < 3 {
        return None;
    }
    if ch == b'`' && bytes[len..].contains(&b'`') {
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

fn parse_single_issue_token(default_repo: &str, raw: &str) -> Option<(String, u64)> {
    let t = raw.trim().trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '*' | '_'));
    let t = t.trim_end_matches([',', ';', ':', '.', '!', '?']);
    if t.is_empty() {
        return None;
    }
    if let Some(num_str) = t.strip_prefix('#') {
        if num_str.is_empty() || num_str.starts_with('0') || !num_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let num: u64 = num_str.parse().ok()?;
        return (num <= MAX_GRAPHQL_ISSUE_NUMBER).then(|| (default_repo.to_string(), num));
    }
    if let Some(issue) = parse_github_issue_only(t)
        && let Some((owner, repo, num)) = issue.github_parts()
        && (1..=MAX_GRAPHQL_ISSUE_NUMBER).contains(&num)
    {
        return Some((format!("{owner}/{repo}"), num));
    }
    None
}

fn match_pr_ref_keyword(s: &str) -> Option<usize> {
    const KWS: &[&str] = &[
        "references",
        "resolves",
        "resolved",
        "resolve",
        "closes",
        "closed",
        "close",
        "fixes",
        "fixed",
        "fix",
        "refs",
        "ref",
    ];
    for kw in KWS {
        if s.len() >= kw.len() && s[..kw.len()].eq_ignore_ascii_case(kw) {
            let after = &s[kw.len()..];
            if after.starts_with([' ', '\t', ':']) {
                return Some(kw.len());
            }
        }
    }
    None
}

pub(crate) fn extract_pr_body_issue_refs(default_repo: &str, body: &str) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut in_comment = false;

    for raw_line in body.lines() {
        if let Some((ch, len)) = fence {
            if fence_closes(raw_line, ch, len) {
                fence = None;
            }
            continue;
        }
        let line = strip_html_comments(raw_line, &mut in_comment);
        if !in_comment && let Some(f) = fence_marker(&line) {
            fence = Some(f);
            continue;
        }
        let clean = strip_inline_code(&line);
        let bytes = clean.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
                i += clean[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let Some(klen) = match_pr_ref_keyword(&clean[i..]) else {
                i += clean[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            let mut pos = i + klen;
            while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
                pos += 1;
            }
            if bytes.get(pos) == Some(&b':') {
                pos += 1;
            }
            while pos < bytes.len() {
                while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b',' | b';') {
                    pos += 1;
                }
                if pos >= bytes.len() {
                    break;
                }
                let rest = &clean[pos..];
                if rest.len() >= 4
                    && rest[..3].eq_ignore_ascii_case("and")
                    && rest.as_bytes().get(3).is_some_and(|b| matches!(b, b' ' | b'\t'))
                {
                    pos += 3;
                    continue;
                }
                if let Some(after) = rest.strip_prefix('&')
                    && after.starts_with([' ', '\t'])
                {
                    pos += 1;
                    continue;
                }
                // Check markdown link `[text](url)`
                if rest.starts_with('[')
                    && let Some(cb) = rest[1..].find(']')
                    && rest[1 + cb + 1..].starts_with('(')
                    && let Some(cp) = rest[1 + cb + 2..].find(')')
                {
                    let url = &rest[1 + cb + 2..1 + cb + 2 + cp];
                    if let Some((r, n)) = parse_single_issue_token(default_repo, url) {
                        if !out.iter().any(|(er, en)| *en == n && er.eq_ignore_ascii_case(&r)) {
                            out.push((r, n));
                        }
                        pos += 1 + cb + 2 + cp + 1;
                        continue;
                    }
                }
                let tok_end =
                    rest.find(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | ')')).unwrap_or(rest.len());
                let tok = &rest[..tok_end];
                if let Some((r, n)) = parse_single_issue_token(default_repo, tok) {
                    if !out.iter().any(|(er, en)| *en == n && er.eq_ignore_ascii_case(&r)) {
                        out.push((r, n));
                    }
                    pos += tok_end;
                    continue;
                }
                break;
            }
            i = pos.max(i + klen);
        }
    }
    out
}

fn strip_line_suffix(s: &str) -> &str {
    // Strip `#L10` / `#L10-L20`
    let s = if let Some((head, tail)) = s.rsplit_once("#L")
        && !head.is_empty()
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'L' | b'C'))
    {
        head
    } else {
        s
    };
    // Strip `:123` / `:123-456` / `:123:45`
    if let Some((head, tail)) = s.rsplit_once(':')
        && !head.is_empty()
        && !tail.is_empty()
        && tail.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && tail.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':'))
    {
        // Handle `:line:col` recursively once
        if let Some((head2, tail2)) = head.rsplit_once(':')
            && !head2.is_empty()
            && !tail2.is_empty()
            && tail2.bytes().all(|b| b.is_ascii_digit())
        {
            return head2;
        }
        return head;
    }
    s
}

/// Normalize a candidate repo-relative path. Returns `(normalized, ended_with_slash)`.
pub(crate) fn normalize_repo_path(raw: &str) -> Option<(String, bool)> {
    let mut s = raw.trim();
    loop {
        let next = s
            .strip_prefix('`')
            .and_then(|x| x.strip_suffix('`'))
            .or_else(|| s.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
            .or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .or_else(|| s.strip_prefix("**").and_then(|x| x.strip_suffix("**")))
            .or_else(|| s.strip_prefix('(').and_then(|x| x.strip_suffix(')')))
            .or_else(|| s.strip_prefix('[').and_then(|x| x.strip_suffix(']')))
            .or_else(|| s.strip_prefix('<').and_then(|x| x.strip_suffix('>')))
            .map(str::trim);
        match next {
            Some(n) if n.len() < s.len() => s = n,
            _ => break,
        }
    }
    s = s.trim_end_matches([',', ';', '.', '!', '?', ':', ')', ']', '`', '"', '\'']);
    s = s.trim_start_matches(['(', '[', '`', '"', '\'']);
    s = strip_line_suffix(s);
    if s.is_empty() || s.contains("://") || s.starts_with('/') || s.starts_with('~') {
        return None;
    }
    if s.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        return None;
    }
    let owned = s.replace('\\', "/");
    let trimmed = owned.strip_prefix("./").unwrap_or(&owned);
    let is_dir = trimmed.ends_with('/');
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains("//") || trimmed.contains('#') || trimmed.contains('?') {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b'@' | b'~'))
    {
        return None;
    }
    Some((trimmed.to_string(), is_dir))
}

fn looks_like_repo_path(path: &str, is_dir: bool) -> bool {
    if is_dir {
        return path.bytes().any(|b| b.is_ascii_alphabetic());
    }
    if path.contains('/') {
        let segs: Vec<&str> = path.split('/').collect();
        // Reject pure numeric ratios like "1/4" or "5/10"
        if segs.iter().all(|s| s.bytes().all(|b| b.is_ascii_digit())) {
            return false;
        }
        return segs.iter().any(|s| s.bytes().any(|b| b.is_ascii_alphabetic()));
    }
    if WELL_KNOWN_ROOT_FILES.contains(&path) {
        return true;
    }
    let Some((stem, ext)) = path.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && stem.bytes().any(|b| b.is_ascii_alphabetic())
        && (1..=10).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

pub(crate) fn extract_body_file_paths(body: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut in_comment = false;

    let mut push_candidate = |raw: &str| {
        if let Some((norm, is_dir)) = normalize_repo_path(raw)
            && looks_like_repo_path(&norm, is_dir)
            && seen.insert(norm.clone())
        {
            out.push((norm, is_dir));
        }
    };

    for raw_line in body.lines() {
        if let Some((ch, len)) = fence {
            if fence_closes(raw_line, ch, len) {
                fence = None;
            }
            continue;
        }
        let line = strip_html_comments(raw_line, &mut in_comment);
        if !in_comment && let Some(f) = fence_marker(&line) {
            fence = Some(f);
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut plain = String::with_capacity(line.len());
        while i < bytes.len() {
            if bytes[i] == b'`'
                && let Some((content_start, end)) = find_inline_code_end(bytes, i)
            {
                let tick_len = content_start - i;
                let inner = &line[content_start..end - tick_len];
                for part in inner.split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';')) {
                    push_candidate(part);
                }
                plain.push(' ');
                i = end;
                continue;
            }
            let ch = line[i..].chars().next().expect("valid utf-8");
            plain.push(ch);
            i += ch.len_utf8();
        }
        for tok in plain.split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';')) {
            if tok.contains("://") || tok.contains("github.com/") {
                continue;
            }
            // If markdown link `[label](target)`, check both label and target separately
            if let Some(open) = tok.find("](") {
                let label = &tok[..open];
                let target = &tok[open + 2..];
                push_candidate(label);
                if !target.contains("://") && !target.contains("github.com/") {
                    push_candidate(target);
                }
                continue;
            }
            push_candidate(tok);
        }
    }
    out
}

pub(crate) fn parse_worktree_porcelain(stdout: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut head_oid: Option<String> = None;
    let mut skip = false;

    let flush = |out: &mut Vec<WorktreeEntry>,
                 path: &mut Option<PathBuf>,
                 branch: &mut Option<String>,
                 head_oid: &mut Option<String>,
                 skip: &mut bool| {
        if let Some(p) = path.take()
            && !*skip
            && p.is_dir()
        {
            out.push(WorktreeEntry { path: p, branch: branch.take(), head_oid: head_oid.take() });
        } else {
            *branch = None;
            *head_oid = None;
        }
        *skip = false;
    };

    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut out, &mut path, &mut branch, &mut head_oid, &mut skip);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut out, &mut path, &mut branch, &mut head_oid, &mut skip);
            path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            let oid = rest.trim();
            if !oid.is_empty() && !oid.bytes().all(|b| b == b'0') {
                head_oid = Some(oid.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("branch ") {
            let b = rest.trim();
            branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        } else if line == "bare" || line.starts_with("prunable") {
            skip = true;
        }
    }
    flush(&mut out, &mut path, &mut branch, &mut head_oid, &mut skip);
    out
}

async fn git_output(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    let run = tokio::process::Command::from(crate::platform::git())
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(GIT_TIMEOUT, run).await.ok()?.ok()
}

async fn git_stdout(dir: &Path, args: &[&str]) -> Option<String> {
    let out = git_output(dir, args).await?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

async fn git_ok(dir: &Path, args: &[&str]) -> bool {
    git_output(dir, args).await.is_some_and(|o| o.status.success())
}

fn safe_git_ref(r: &str) -> bool {
    !r.is_empty() && !r.starts_with('-') && !r.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
}

async fn resolve_commit_oid(dir: &Path, rev: &str) -> Option<String> {
    if !safe_git_ref(rev) {
        return None;
    }
    let query = format!("{rev}^{{commit}}");
    let s = git_stdout(dir, &["rev-parse", "--verify", "--quiet", &query]).await?;
    let oid = s.trim();
    (!oid.is_empty()).then(|| oid.to_string())
}

async fn resolve_default_branch(dir: &Path, gh_default: Option<&str>) -> (Option<String>, Option<String>) {
    if let Some(name) = gh_default.filter(|n| safe_git_ref(n)) {
        for cand in [format!("refs/remotes/origin/{name}"), format!("refs/heads/{name}")] {
            if let Some(oid) = resolve_commit_oid(dir, &cand).await {
                return (Some(name.to_string()), Some(oid));
            }
        }
    }
    if let Some(sym) = git_stdout(dir, &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]).await {
        let sym = sym.trim();
        let name = sym.strip_prefix("refs/remotes/origin/").unwrap_or(sym).to_string();
        if let Some(oid) = resolve_commit_oid(dir, sym).await {
            return (Some(name), Some(oid));
        }
    }
    for name in ["main", "master"] {
        for cand in [format!("refs/remotes/origin/{name}"), format!("refs/heads/{name}")] {
            if let Some(oid) = resolve_commit_oid(dir, &cand).await {
                return (Some(name.to_string()), Some(oid));
            }
        }
    }
    (gh_default.map(String::from), None)
}

pub(crate) async fn inspect_local_repo(root: &Path, gh_default: Option<&str>) -> Option<LocalRepoInfo> {
    if !root.is_dir() {
        return None;
    }
    if !git_ok(root, &["rev-parse", "--git-dir"]).await {
        return None;
    }
    let (default_branch, default_oid) = resolve_default_branch(root, gh_default).await;
    let worktrees = git_stdout(root, &["worktree", "list", "--porcelain"])
        .await
        .map(|s| parse_worktree_porcelain(&s))
        .unwrap_or_default();
    let mut tree_files = HashSet::new();
    let mut tree_dirs = HashSet::new();
    if let Some(ls) = git_stdout(root, &["ls-files", "-z"]).await {
        for entry in ls.split('\0') {
            if entry.is_empty() {
                continue;
            }
            let norm = entry.replace('\\', "/");
            let mut prefix = norm.as_str();
            while let Some((dir, _)) = prefix.rsplit_once('/') {
                if !tree_dirs.insert(dir.to_string()) {
                    break;
                }
                prefix = dir;
            }
            tree_files.insert(norm);
        }
    }
    Some(LocalRepoInfo { root: root.to_path_buf(), default_branch, default_oid, worktrees, tree_files, tree_dirs })
}

async fn resolve_branch_oid(root: &Path, branch: &str, fallback_oid: Option<&str>) -> Option<String> {
    if safe_git_ref(branch) {
        for cand in [format!("refs/heads/{branch}"), format!("refs/remotes/origin/{branch}")] {
            if let Some(oid) = resolve_commit_oid(root, &cand).await {
                return Some(oid);
            }
        }
    }
    if let Some(oid) = fallback_oid.filter(|o| safe_git_ref(o)) {
        return resolve_commit_oid(root, oid).await;
    }
    None
}

async fn is_ahead_of_default(root: &Path, head_oid: &str, default_oid: &str) -> bool {
    if head_oid == default_oid || !safe_git_ref(head_oid) || !safe_git_ref(default_oid) {
        return false;
    }
    let Some(mb) = git_stdout(root, &["merge-base", head_oid, default_oid]).await else {
        return false;
    };
    let mb = mb.trim();
    !mb.is_empty() && !mb.eq_ignore_ascii_case(head_oid)
}

async fn is_strict_ancestor(root: &Path, ancestor_oid: &str, descendant_oid: &str) -> bool {
    if ancestor_oid.eq_ignore_ascii_case(descendant_oid) || !safe_git_ref(ancestor_oid) || !safe_git_ref(descendant_oid)
    {
        return false;
    }
    git_ok(root, &["merge-base", "--is-ancestor", ancestor_oid, descendant_oid]).await
}

async fn worktree_modified_files(wt: &Path, base_oid: Option<&str>) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut add_lines = |raw: &str| {
        for line in raw.lines() {
            if let Some((norm, _)) = normalize_repo_path(line)
                && seen.insert(norm.clone())
            {
                files.push(norm);
            }
        }
    };
    if let Some(base) = base_oid.filter(|b| safe_git_ref(b)) {
        let range = format!("{base}...HEAD");
        if let Some(diff) = git_stdout(wt, &["diff", "--name-only", &range]).await {
            add_lines(&diff);
        }
    }
    if let Some(diff) = git_stdout(wt, &["diff", "--name-only", "HEAD"]).await {
        add_lines(&diff);
    }
    if let Some(untracked) = git_stdout(wt, &["ls-files", "--others", "--exclude-standard"]).await {
        add_lines(&untracked);
    }
    files
}

fn canonical_path(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
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

fn prune_parent_epics(issues: &mut Vec<(String, u64)>, parent_of: &BTreeMap<String, Vec<String>>) {
    if issues.len() <= 1 {
        return;
    }
    let keys: Vec<String> = issues.iter().map(|(r, n)| issue_key(r, *n)).collect();
    let mut keep = vec![true; issues.len()];
    for i in 0..issues.len() {
        for j in 0..issues.len() {
            if i != j && is_ancestor_issue(&keys[i], &keys[j], parent_of) {
                keep[i] = false;
                break;
            }
        }
    }
    if keep.iter().any(|&k| k) {
        let mut idx = 0;
        issues.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }
}

struct BranchCandidate {
    branch: String,
    base_ref_name: Option<String>,
    pr_number: Option<u64>,
    is_cross_repository: bool,
    local_oid: Option<String>,
    worktree_path: Option<PathBuf>,
    pr_files: Vec<String>,
    issues: Vec<(String, u64)>,
}

pub(crate) struct SourceControlInput<'a> {
    pub(crate) aliases: &'a HashMap<String, String>,
    pub(crate) parent_of: &'a BTreeMap<String, Vec<String>>,
    pub(crate) closed_issues: &'a HashSet<String>,
    pub(crate) open_prs: &'a BTreeMap<String, RepoPrs>,
    pub(crate) local_repos: &'a BTreeMap<String, LocalRepoInfo>,
    pub(crate) active_parts: &'a [ActivePartLink],
    pub(crate) active_branches: &'a [ActiveBranchLink],
    pub(crate) open_issues: &'a [IssueFileInput<'a>],
}

pub(crate) struct SourceControlAnalysis {
    pub(crate) pr_stacks: Vec<PrStackEdge>,
    pub(crate) file_conflicts: Vec<FileConflictEdge>,
}

fn push_resolved_issue(out: &mut Vec<(String, u64)>, aliases: &HashMap<String, String>, repo: String, number: u64) {
    let canon = canonicalize_repo(aliases, &repo);
    if !out.iter().any(|(r, n)| *n == number && r.eq_ignore_ascii_case(&canon)) {
        out.push((canon, number));
    }
}

fn resolve_pr_issues(
    pr: &OpenPr,
    aliases: &HashMap<String, String>,
    active_branches: &[ActiveBranchLink],
    parent_of: &BTreeMap<String, Vec<String>>,
) -> Vec<(String, u64)> {
    let pr_repo = canonicalize_repo(aliases, &pr.repo);
    let mut out: Vec<(String, u64)> = Vec::new();

    for (r, n) in &pr.closing_issues {
        push_resolved_issue(&mut out, aliases, r.clone(), *n);
    }
    for link in active_branches {
        let repo_ok = link.repo.as_deref().is_none_or(|r| canonicalize_repo(aliases, r).eq_ignore_ascii_case(&pr_repo));
        if !repo_ok {
            continue;
        }
        let by_pr = link.pr_number == Some(pr.number);
        let by_branch = !pr.is_cross_repository
            && !pr.head_ref_name.is_empty()
            && link.branch.as_deref() == Some(pr.head_ref_name.as_str());
        if by_pr || by_branch {
            push_resolved_issue(&mut out, aliases, link.issue.0.clone(), link.issue.1);
        }
    }
    if out.is_empty() {
        for (r, n) in extract_pr_body_issue_refs(&pr_repo, &pr.body) {
            push_resolved_issue(&mut out, aliases, r, n);
        }
        if let Some(num) = parse_branch_issue(&pr.head_ref_name) {
            push_resolved_issue(&mut out, aliases, pr_repo, num);
        }
    }
    prune_parent_epics(&mut out, parent_of);
    out
}

fn resolve_worktree_issues(
    repo: &str,
    wt: &WorktreeEntry,
    aliases: &HashMap<String, String>,
    active_branches: &[ActiveBranchLink],
    parent_of: &BTreeMap<String, Vec<String>>,
) -> Vec<(String, u64)> {
    let canon_repo = canonicalize_repo(aliases, repo);
    let wt_canon = canonical_path(&wt.path);
    let mut out: Vec<(String, u64)> = Vec::new();

    for link in active_branches {
        let by_wt = link.worktree_path.as_deref().is_some_and(|p| canonical_path(p) == wt_canon);
        let by_branch = wt.branch.as_deref().is_some_and(|b| link.branch.as_deref() == Some(b))
            && link.repo.as_deref().is_none_or(|r| canonicalize_repo(aliases, r).eq_ignore_ascii_case(&canon_repo));
        if by_wt || by_branch {
            push_resolved_issue(&mut out, aliases, link.issue.0.clone(), link.issue.1);
        }
    }
    if out.is_empty()
        && let Some(b) = wt.branch.as_deref()
        && let Some(num) = parse_branch_issue(b)
    {
        push_resolved_issue(&mut out, aliases, canon_repo, num);
    }
    prune_parent_epics(&mut out, parent_of);
    out
}

fn merge_pr_stack_edge(slot: &mut PrStackEdge, incoming: PrStackEdge) {
    if slot.branch.is_none() {
        slot.branch = incoming.branch;
    }
    if slot.base_branch.is_none() {
        slot.base_branch = incoming.base_branch;
    }
    if slot.pr.is_none() {
        slot.pr = incoming.pr;
    }
    if slot.base_pr.is_none() {
        slot.base_pr = incoming.base_pr;
    }
    if slot.part.is_none() {
        slot.part = incoming.part;
    }
}

fn reachable(adj: &BTreeMap<String, Vec<String>>, start: &str, target: &str) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(cur) = stack.pop() {
        if let Some(nexts) = adj.get(&cur) {
            for n in nexts {
                if n == target {
                    return true;
                }
                if seen.insert(n.clone()) {
                    stack.push(n.clone());
                }
            }
        }
    }
    false
}

fn overlap_paths(a: &BTreeSet<TargetPath>, b: &BTreeSet<TargetPath>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for pa in a {
        for pb in b {
            match (pa, pb) {
                (TargetPath::File(fa), TargetPath::File(fb)) if fa == fb => {
                    out.insert(fa.clone());
                }
                (TargetPath::Dir(d), TargetPath::File(f)) | (TargetPath::File(f), TargetPath::Dir(d))
                    if f.len() > d.len() && f.starts_with(d.as_str()) && f.as_bytes()[d.len()] == b'/' =>
                {
                    out.insert(f.clone());
                }
                (TargetPath::Dir(da), TargetPath::Dir(db)) => {
                    if da == db
                        || (da.len() > db.len() && da.starts_with(db.as_str()) && da.as_bytes()[db.len()] == b'/')
                    {
                        out.insert(format!("{da}/"));
                    } else if db.len() > da.len() && db.starts_with(da.as_str()) && db.as_bytes()[da.len()] == b'/' {
                        out.insert(format!("{db}/"));
                    }
                }
                _ => {}
            }
        }
    }
    out.into_iter().collect()
}

pub(crate) async fn analyze_source_control(input: SourceControlInput<'_>) -> SourceControlAnalysis {
    let mut repo_keys: BTreeSet<String> = BTreeSet::new();
    for r in input.open_prs.keys() {
        repo_keys.insert(r.to_ascii_lowercase());
    }
    for r in input.local_repos.keys() {
        repo_keys.insert(r.to_ascii_lowercase());
    }
    for o in input.open_issues {
        repo_keys.insert(o.repo.to_ascii_lowercase());
    }

    let mut direct_edges: BTreeMap<(String, String), PrStackEdge> = BTreeMap::new();
    let mut ancestry_edges: BTreeMap<(String, String), PrStackEdge> = BTreeMap::new();
    let mut issue_extra_files: BTreeMap<String, BTreeSet<TargetPath>> = BTreeMap::new();

    for repo_key in &repo_keys {
        let repo_prs = input.open_prs.get(repo_key);
        let local = input.local_repos.get(repo_key);
        let repo_display = repo_prs
            .and_then(|rp| rp.prs.first().map(|p| p.repo.clone()))
            .or_else(|| {
                input.open_issues.iter().find(|o| o.repo.eq_ignore_ascii_case(repo_key)).map(|o| o.repo.to_string())
            })
            .unwrap_or_else(|| repo_key.clone());
        let default_branch = repo_prs
            .and_then(|rp| rp.default_branch.clone())
            .or_else(|| local.and_then(|l| l.default_branch.clone()))
            .unwrap_or_else(|| "main".to_string());

        let mut candidates: Vec<BranchCandidate> = Vec::new();
        let mut wt_used: Vec<bool> = vec![false; local.map_or(0, |l| l.worktrees.len())];

        for pr in repo_prs.map(|rp| rp.prs.as_slice()).unwrap_or(&[]) {
            let mut wt_path = None;
            let mut wt_oid = None;
            if let Some(l) = local
                && !pr.is_cross_repository
                && !pr.head_ref_name.is_empty()
            {
                for (wi, wt) in l.worktrees.iter().enumerate() {
                    if wt.branch.as_deref() == Some(pr.head_ref_name.as_str()) {
                        wt_used[wi] = true;
                        wt_path = Some(wt.path.clone());
                        wt_oid = wt.head_oid.clone();
                        break;
                    }
                }
            }
            let local_oid = if let Some(l) = local {
                match wt_oid {
                    Some(oid) => Some(oid),
                    None => {
                        let fallback = (!pr.head_ref_oid.is_empty()).then_some(pr.head_ref_oid.as_str());
                        if pr.is_cross_repository {
                            match fallback {
                                Some(o) => resolve_commit_oid(&l.root, o).await,
                                None => None,
                            }
                        } else {
                            resolve_branch_oid(&l.root, &pr.head_ref_name, fallback).await
                        }
                    }
                }
            } else {
                None
            };
            let issues = resolve_pr_issues(pr, input.aliases, input.active_branches, input.parent_of);
            candidates.push(BranchCandidate {
                branch: pr.head_ref_name.clone(),
                base_ref_name: Some(pr.base_ref_name.clone()),
                pr_number: Some(pr.number),
                is_cross_repository: pr.is_cross_repository,
                local_oid,
                worktree_path: wt_path,
                pr_files: pr.files.clone(),
                issues,
            });
        }

        if let Some(l) = local {
            for (wi, wt) in l.worktrees.iter().enumerate() {
                if wt_used[wi] {
                    continue;
                }
                let Some(branch) = wt.branch.as_deref() else { continue };
                if branch == default_branch {
                    continue;
                }
                let issues =
                    resolve_worktree_issues(&repo_display, wt, input.aliases, input.active_branches, input.parent_of);
                if !issues.is_empty() && issues.iter().all(|(r, n)| input.closed_issues.contains(&issue_key(r, *n))) {
                    continue;
                }
                let local_oid = match wt.head_oid.clone() {
                    Some(o) => Some(o),
                    None => resolve_branch_oid(&l.root, branch, None).await,
                };
                candidates.push(BranchCandidate {
                    branch: branch.to_string(),
                    base_ref_name: None,
                    pr_number: None,
                    is_cross_repository: false,
                    local_oid,
                    worktree_path: Some(wt.path.clone()),
                    pr_files: Vec::new(),
                    issues,
                });
            }
        }

        // Track stack base OID for each worktree candidate so worktree file diff compares against its stack base.
        let mut stack_base_oid: Vec<Option<String>> = vec![None; candidates.len()];

        // 1. Direct baseRefName PR stacks.
        for b_idx in 0..candidates.len() {
            let Some(base_ref) = candidates[b_idx].base_ref_name.as_deref() else { continue };
            if base_ref.is_empty() || base_ref == default_branch {
                continue;
            }
            for a_idx in 0..candidates.len() {
                if a_idx == b_idx || candidates[a_idx].is_cross_repository || candidates[a_idx].pr_number.is_none() {
                    continue;
                }
                if candidates[a_idx].branch != base_ref || candidates[a_idx].branch == default_branch {
                    continue;
                }
                if stack_base_oid[b_idx].is_none() {
                    stack_base_oid[b_idx] = candidates[a_idx].local_oid.clone();
                }
                for (rb, nb) in &candidates[b_idx].issues {
                    for (ra, na) in &candidates[a_idx].issues {
                        let kb = issue_key(rb, *nb);
                        let ka = issue_key(ra, *na);
                        if kb == ka {
                            continue;
                        }
                        let edge = PrStackEdge {
                            from: (rb.clone(), *nb),
                            to: (ra.clone(), *na),
                            branch: (!candidates[b_idx].branch.is_empty()).then(|| candidates[b_idx].branch.clone()),
                            base_branch: Some(candidates[a_idx].branch.clone()),
                            pr: candidates[b_idx].pr_number,
                            base_pr: candidates[a_idx].pr_number,
                            part: None,
                        };
                        direct_edges
                            .entry((kb, ka))
                            .and_modify(|slot| merge_pr_stack_edge(slot, edge.clone()))
                            .or_insert(edge);
                    }
                }
            }
        }

        // 2. Local git branch ancestry (`git merge-base`).
        if let Some(l) = local
            && let Some(default_oid) = l.default_oid.as_deref()
        {
            let mut ahead = vec![false; candidates.len()];
            for (idx, c) in candidates.iter().enumerate() {
                if let Some(oid) = c.local_oid.as_deref() {
                    ahead[idx] = is_ahead_of_default(&l.root, oid, default_oid).await;
                }
            }
            for a_idx in 0..candidates.len() {
                if !ahead[a_idx] {
                    continue;
                }
                let Some(oid_a) = candidates[a_idx].local_oid.as_deref() else { continue };
                for b_idx in 0..candidates.len() {
                    if a_idx == b_idx {
                        continue;
                    }
                    let Some(oid_b) = candidates[b_idx].local_oid.as_deref() else { continue };
                    if !is_strict_ancestor(&l.root, oid_a, oid_b).await {
                        continue;
                    }
                    // Prefer the closest ancestor as base_oid for worktree diff
                    if let Some(prev_oid) = stack_base_oid[b_idx].as_deref() {
                        if is_strict_ancestor(&l.root, prev_oid, oid_a).await {
                            stack_base_oid[b_idx] = Some(oid_a.to_string());
                        }
                    } else {
                        stack_base_oid[b_idx] = Some(oid_a.to_string());
                    }
                    for (rb, nb) in &candidates[b_idx].issues {
                        for (ra, na) in &candidates[a_idx].issues {
                            let kb = issue_key(rb, *nb);
                            let ka = issue_key(ra, *na);
                            if kb == ka {
                                continue;
                            }
                            let edge = PrStackEdge {
                                from: (rb.clone(), *nb),
                                to: (ra.clone(), *na),
                                branch: (!candidates[b_idx].branch.is_empty())
                                    .then(|| candidates[b_idx].branch.clone()),
                                base_branch: (!candidates[a_idx].branch.is_empty())
                                    .then(|| candidates[a_idx].branch.clone()),
                                pr: candidates[b_idx].pr_number,
                                base_pr: candidates[a_idx].pr_number,
                                part: None,
                            };
                            ancestry_edges
                                .entry((kb, ka))
                                .and_modify(|slot| merge_pr_stack_edge(slot, edge.clone()))
                                .or_insert(edge);
                        }
                    }
                }
            }
        }

        // 3. Collect modified files from open PRs and active local worktrees for each linked issue in this repo.
        let default_oid = local.and_then(|l| l.default_oid.as_deref());
        for (idx, c) in candidates.iter().enumerate() {
            let mut c_files: Vec<String> = c.pr_files.clone();
            if let Some(wt) = c.worktree_path.as_deref() {
                let base = stack_base_oid[idx].as_deref().or(default_oid);
                for f in worktree_modified_files(wt, base).await {
                    if !c_files.iter().any(|x| x == &f) {
                        c_files.push(f);
                    }
                }
            }
            if c_files.is_empty() {
                continue;
            }
            for (r, n) in &c.issues {
                if !r.eq_ignore_ascii_case(&repo_display) && !r.eq_ignore_ascii_case(repo_key) {
                    continue;
                }
                let entry = issue_extra_files.entry(issue_key(r, *n)).or_default();
                for f in &c_files {
                    entry.insert(TargetPath::File(f.clone()));
                }
            }
        }
    }

    // Transitive reduction on ancestry-only edges so a 3-deep commit stack A -> B -> C doesn't emit C -> A.
    let mut combined_adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (from, to) in direct_edges.keys().chain(ancestry_edges.keys()) {
        combined_adj.entry(from.clone()).or_default().push(to.clone());
    }
    let mut merged_stacks: BTreeMap<(String, String), PrStackEdge> = direct_edges;
    for ((from, to), edge) in ancestry_edges {
        if merged_stacks.contains_key(&(from.clone(), to.clone())) {
            merged_stacks.entry((from, to)).and_modify(|slot| merge_pr_stack_edge(slot, edge));
            continue;
        }
        let redundant = combined_adj.get(&from).is_some_and(|mids| {
            mids.iter().any(|mid| {
                mid != &from
                    && mid != &to
                    && reachable(&combined_adj, mid, &to)
                    && !reachable(&combined_adj, mid, &from)
            })
        });
        if !redundant {
            merged_stacks.insert((from, to), edge);
        }
    }

    // 4. Sequential t3-ticket `part: n/N` chains from active links.
    type PartGroup = BTreeMap<u32, Vec<(String, u64)>>;
    let mut part_groups: BTreeMap<(String, u32), PartGroup> = BTreeMap::new();
    for p in input.active_parts {
        let issues = part_groups.entry((p.scope.clone(), p.part_total)).or_default().entry(p.part_num).or_default();
        if !issues.iter().any(|(r, n)| *n == p.issue.1 && r.eq_ignore_ascii_case(&p.issue.0)) {
            issues.push(p.issue.clone());
        }
    }
    for ((_, total), by_part) in part_groups {
        let ordered: Vec<(u32, Vec<(String, u64)>)> = by_part.into_iter().collect();
        for pair in ordered.windows(2) {
            let (prev_n, prev_issues) = &pair[0];
            let (curr_n, curr_issues) = &pair[1];
            let label = format!("{prev_n}/{total} -> {curr_n}/{total}");
            for (rc, nc) in curr_issues {
                for (rp, np) in prev_issues {
                    let kc = issue_key(rc, *nc);
                    let kp = issue_key(rp, *np);
                    if kc == kp || input.closed_issues.contains(&kp) {
                        continue;
                    }
                    let edge = PrStackEdge {
                        from: (rc.clone(), *nc),
                        to: (rp.clone(), *np),
                        branch: None,
                        base_branch: None,
                        pr: None,
                        base_pr: None,
                        part: Some(label.clone()),
                    };
                    merged_stacks
                        .entry((kc, kp))
                        .and_modify(|slot| merge_pr_stack_edge(slot, edge.clone()))
                        .or_insert(edge);
                }
            }
        }
    }

    // 5. File-conflict edges across open issues in each repo.
    type RepoIssuePaths = ((String, u64), String, BTreeSet<TargetPath>);
    let mut by_repo: BTreeMap<String, Vec<RepoIssuePaths>> = BTreeMap::new();
    let mut seen_open: HashSet<String> = HashSet::new();
    for item in input.open_issues {
        let canon = canonicalize_repo(input.aliases, item.repo);
        let rkey = canon.to_ascii_lowercase();
        let ikey = issue_key(&canon, item.number);
        if !seen_open.insert(ikey.clone()) {
            continue;
        }
        let local = input.local_repos.get(&rkey).filter(|l| !l.tree_files.is_empty());
        let mut paths = issue_extra_files.remove(&ikey).unwrap_or_default();

        // Add Touches: paths (verified against local tree when present, else syntactically valid repo paths).
        for raw in item.touches {
            let Some((norm, is_dir)) = normalize_repo_path(raw) else { continue };
            if !looks_like_repo_path(&norm, is_dir) {
                continue;
            }
            if let Some(l) = local {
                if !is_dir && l.tree_files.contains(&norm) {
                    paths.insert(TargetPath::File(norm));
                } else if l.tree_dirs.contains(&norm) {
                    paths.insert(TargetPath::Dir(norm));
                }
            } else if is_dir {
                paths.insert(TargetPath::Dir(norm));
            } else {
                paths.insert(TargetPath::File(norm));
            }
        }

        // Add body-extracted repo-relative paths verified against the local repository tree.
        if let Some(l) = local {
            for (norm, is_dir) in extract_body_file_paths(item.body) {
                if !is_dir && l.tree_files.contains(&norm) {
                    paths.insert(TargetPath::File(norm));
                } else if l.tree_dirs.contains(&norm) {
                    paths.insert(TargetPath::Dir(norm));
                }
            }
        }

        if !paths.is_empty() {
            by_repo.entry(rkey).or_default().push(((canon, item.number), ikey, paths));
        }
    }

    let mut file_conflicts = Vec::new();
    for (_, mut items) in by_repo {
        items.sort_by(|a, b| a.0.0.to_ascii_lowercase().cmp(&b.0.0.to_ascii_lowercase()).then(a.0.1.cmp(&b.0.1)));
        for i in 0..items.len() {
            for j in (i + 1)..items.len() {
                let (ref issue_a, ref key_a, ref paths_a) = items[i];
                let (ref issue_b, ref key_b, ref paths_b) = items[j];
                if is_ancestor_issue(key_a, key_b, input.parent_of) || is_ancestor_issue(key_b, key_a, input.parent_of)
                {
                    continue;
                }
                let shared = overlap_paths(paths_a, paths_b);
                if !shared.is_empty() {
                    file_conflicts.push(FileConflictEdge { from: issue_a.clone(), to: issue_b.clone(), files: shared });
                }
            }
        }
    }

    let mut pr_stacks: Vec<PrStackEdge> = merged_stacks.into_values().collect();
    pr_stacks.sort_by(|a, b| {
        a.from
            .0
            .to_ascii_lowercase()
            .cmp(&b.from.0.to_ascii_lowercase())
            .then(a.from.1.cmp(&b.from.1))
            .then_with(|| a.to.0.to_ascii_lowercase().cmp(&b.to.0.to_ascii_lowercase()))
            .then(a.to.1.cmp(&b.to.1))
    });
    file_conflicts.sort_by(|a, b| {
        a.from
            .0
            .to_ascii_lowercase()
            .cmp(&b.from.0.to_ascii_lowercase())
            .then(a.from.1.cmp(&b.from.1))
            .then_with(|| a.to.0.to_ascii_lowercase().cmp(&b.to.0.to_ascii_lowercase()))
            .then(a.to.1.cmp(&b.to.1))
    });

    SourceControlAnalysis { pr_stacks, file_conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_issue_numbers_and_rejects_non_issue_prefixes() {
        assert_eq!(parse_branch_issue("feat/257-pr-stack-file-conflict"), Some(257));
        assert_eq!(parse_branch_issue("fix/42"), Some(42));
        assert_eq!(parse_branch_issue("chore/19_cleanup"), Some(19));
        assert_eq!(parse_branch_issue("alice/feat/101-login"), Some(101));
        assert_eq!(parse_branch_issue("t3/issue-112-pool-routing"), Some(112));
        assert_eq!(parse_branch_issue("main"), None);
        assert_eq!(parse_branch_issue("pr/230"), None);
        assert_eq!(parse_branch_issue("pull/99-test"), None);
        assert_eq!(parse_branch_issue("release/2026-11-24"), None);
        assert_eq!(parse_branch_issue("feat/0-invalid"), None);
        assert_eq!(parse_branch_issue("feat/42abc"), None);
        assert_eq!(parse_branch_issue("2026/11-notes"), None);
    }

    #[test]
    fn extracts_pr_body_refs_outside_fences_code_and_comments() {
        let body = "\
Summary of PR.
Refs #257, acme/tools#12 and https://github.com/acme/widgets/issues/33
<!-- Refs #99 -->
`Refs #88`
```md
Refs #77
```
Closes: #44
";
        let refs = extract_pr_body_issue_refs("acme/widgets", body);
        assert_eq!(
            refs,
            vec![
                ("acme/widgets".into(), 257),
                ("acme/tools".into(), 12),
                ("acme/widgets".into(), 33),
                ("acme/widgets".into(), 44),
            ]
        );
    }

    #[test]
    fn normalizes_and_extracts_body_file_paths() {
        assert_eq!(normalize_repo_path("`./src/flow.rs:743-758`"), Some(("src/flow.rs".into(), false)));
        assert_eq!(normalize_repo_path("src/board.html#L10-L20"), Some(("src/board.html".into(), false)));
        assert_eq!(normalize_repo_path("docs/src/"), Some(("docs/src".into(), true)));
        assert_eq!(normalize_repo_path("../secret.txt"), None);
        assert_eq!(normalize_repo_path("/etc/passwd"), None);
        assert_eq!(normalize_repo_path("https://github.com/acme/widgets/issues/1"), None);

        let body = "\
Changes `src/board.html:4499` and `tests/all/flow.rs`, plus Cargo.toml.
Also see docs/src/board.md and [link](https://github.com/acme/widgets/issues/1).
Part 1/4 of acme/widgets#254.
```rust
let ignored = \"src/ignored.rs\";
```
<!-- src/commented.rs -->
";
        let found: Vec<String> = extract_body_file_paths(body).into_iter().map(|(p, _)| p).collect();
        assert!(found.contains(&"src/board.html".to_string()), "{found:?}");
        assert!(found.contains(&"tests/all/flow.rs".to_string()), "{found:?}");
        assert!(found.contains(&"Cargo.toml".to_string()), "{found:?}");
        assert!(found.contains(&"docs/src/board.md".to_string()), "{found:?}");
        assert!(!found.contains(&"src/ignored.rs".to_string()), "{found:?}");
        assert!(!found.contains(&"src/commented.rs".to_string()), "{found:?}");
        assert!(!found.contains(&"1/4".to_string()), "{found:?}");
    }

    #[test]
    fn parses_worktree_porcelain_and_skips_bare_and_prunable() {
        let dir = tempfile::tempdir().unwrap();
        let main_wt = dir.path().join("main");
        let feat_wt = dir.path().join("feat");
        let bare_wt = dir.path().join("bare");
        let prunable_wt = dir.path().join("prunable");
        std::fs::create_dir_all(&main_wt).unwrap();
        std::fs::create_dir_all(&feat_wt).unwrap();
        std::fs::create_dir_all(&bare_wt).unwrap();
        std::fs::create_dir_all(&prunable_wt).unwrap();
        let out = format!(
            "worktree {}\nHEAD 1111111\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD 2222222\nbranch refs/heads/feat/20-stack\n\n\
             worktree {}\nbare\n\n\
             worktree {}\nHEAD 3333333\nbranch refs/heads/feat/30-old\nprunable gitdir file points to non-existent location\n",
            main_wt.display(),
            feat_wt.display(),
            bare_wt.display(),
            prunable_wt.display(),
        );
        let parsed = parse_worktree_porcelain(&out);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert_eq!(parsed[1].branch.as_deref(), Some("feat/20-stack"));
        assert_eq!(parsed[1].head_oid.as_deref(), Some("2222222"));
    }
}
