use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::board::parse_github_issue_only;
pub(crate) use crate::flow_code::CodeDepEdge;
use crate::flow_code::RepoIssueCodeInput;

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
    pub(crate) files_next_cursor: Option<String>,
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
    pub(crate) base_repo: Option<String>,
    pub(crate) base_oid: Option<String>,
    pub(crate) pr: Option<u64>,
    pub(crate) base_pr: Option<u64>,
    pub(crate) part: Option<String>,
    pub(crate) direct: bool,
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
pub(crate) enum TargetPath {
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
            && let Some(norm) = clean_exact_repo_path(raw)
            && !files.iter().any(|x| x == &norm)
        {
            files.push(norm);
        }
    }
    let files_page = &v["files"]["pageInfo"];
    let files_next_cursor =
        (files_page["hasNextPage"] == true).then(|| files_page["endCursor"].as_str().map(String::from)).flatten();
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
        files_next_cursor,
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
        let rest =
            if tail.len() >= 6 && tail.as_bytes()[..6].eq_ignore_ascii_case(b"issue-") { &tail[6..] } else { tail };
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
    const KWS: &[&[u8]] = &[
        b"references",
        b"resolves",
        b"resolved",
        b"resolve",
        b"closes",
        b"closed",
        b"close",
        b"fixes",
        b"fixed",
        b"fix",
        b"refs",
        b"ref",
    ];
    let bytes = s.as_bytes();
    for kw in KWS {
        if bytes.len() >= kw.len()
            && bytes[..kw.len()].eq_ignore_ascii_case(kw)
            && bytes.get(kw.len()).is_some_and(|b| matches!(b, b' ' | b'\t' | b':'))
        {
            return Some(kw.len());
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
                let rest_bytes = rest.as_bytes();
                if rest_bytes.len() >= 4
                    && rest_bytes[..3].eq_ignore_ascii_case(b"and")
                    && matches!(rest_bytes[3], b' ' | b'\t')
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

/// Clean an exact repo-relative path from GitHub or Git (`-z`), preserving spaces and non-ASCII characters.
pub(crate) fn clean_exact_repo_path(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let owned = raw.replace('\\', "/");
    let trimmed = owned.strip_prefix("./").unwrap_or(&owned).trim_end_matches('/');
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    if trimmed.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    if trimmed.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return None;
    }
    Some(trimmed.to_string())
}

/// Normalize a candidate repo-relative path from issue prose. Returns `(normalized, ended_with_slash)`.
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

async fn git_stdout_stdin(dir: &Path, args: &[&str], input: &str) -> Option<String> {
    let mut child = tokio::process::Command::from(crate::platform::git())
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let bytes = input.as_bytes().to_vec();
    let run = async move {
        let write = async move {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        };
        let (_, out) = tokio::join!(write, child.wait_with_output());
        out
    };
    let out = tokio::time::timeout(GIT_TIMEOUT, run).await.ok()?.ok()?;
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

pub(crate) async fn inspect_local_repo(
    root: &Path,
    gh_default: Option<&str>,
    with_tree: bool,
) -> Option<LocalRepoInfo> {
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
    if with_tree && let Some(ls) = git_stdout(root, &["ls-files", "-z"]).await {
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

pub(crate) async fn refresh_local_repo_worktrees(prev: &LocalRepoInfo, gh_default: Option<&str>) -> LocalRepoInfo {
    let preferred = gh_default.or(prev.default_branch.as_deref());
    let (default_branch, default_oid) = resolve_default_branch(&prev.root, preferred).await;
    let worktrees = git_stdout(&prev.root, &["worktree", "list", "--porcelain"])
        .await
        .map(|s| parse_worktree_porcelain(&s))
        .unwrap_or_else(|| prev.worktrees.clone());
    LocalRepoInfo {
        root: prev.root.clone(),
        default_branch,
        default_oid,
        worktrees,
        tree_files: prev.tree_files.clone(),
        tree_dirs: prev.tree_dirs.clone(),
    }
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

#[derive(Default)]
struct AncestryGraph {
    in_graph: HashSet<String>,
    ahead: HashSet<String>,
    strict_ancestors: HashMap<String, HashSet<String>>,
}

impl AncestryGraph {
    fn is_ahead(&self, oid: &str) -> bool {
        self.ahead.contains(&oid.to_ascii_lowercase())
    }

    fn in_graph(&self, oid: &str) -> bool {
        self.in_graph.contains(&oid.to_ascii_lowercase())
    }

    fn is_strict_ancestor(&self, ancestor_oid: &str, descendant_oid: &str) -> bool {
        self.strict_ancestors
            .get(&descendant_oid.to_ascii_lowercase())
            .is_some_and(|ancs| ancs.contains(&ancestor_oid.to_ascii_lowercase()))
    }
}

async fn compute_candidate_ancestry(root: &Path, default_oid: &str, candidate_oids: &[&str]) -> AncestryGraph {
    if !safe_git_ref(default_oid) {
        return AncestryGraph::default();
    }
    let default_low = default_oid.to_ascii_lowercase();
    let mut unique_oids: Vec<String> = Vec::new();
    let mut seen_oids: HashSet<String> = HashSet::new();
    for oid in candidate_oids {
        if !safe_git_ref(oid) {
            continue;
        }
        let low = oid.to_ascii_lowercase();
        if low != default_low && seen_oids.insert(low.clone()) {
            unique_oids.push(low);
        }
    }
    if unique_oids.is_empty() {
        return AncestryGraph::default();
    }
    let mut stdin_buf = format!("^{default_low}\n");
    for oid in &unique_oids {
        stdin_buf.push_str(oid);
        stdin_buf.push('\n');
    }
    let Some(stdout) = git_stdout_stdin(root, &["rev-list", "--parents", "--stdin"], &stdin_buf).await else {
        return AncestryGraph::default();
    };
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut oids: Vec<String> = Vec::new();
    let mut id_of: HashMap<String, usize> = HashMap::new();
    for line in stdout.lines() {
        let toks: Vec<String> = line.split_ascii_whitespace().map(str::to_ascii_lowercase).collect();
        let Some(first) = toks.first() else { continue };
        if id_of.contains_key(first) {
            continue;
        }
        let idx = oids.len();
        id_of.insert(first.clone(), idx);
        oids.push(first.clone());
        rows.push(toks);
    }
    let n = oids.len();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut touches_default: Vec<bool> = vec![false; n];
    for (u, toks) in rows.iter().enumerate() {
        for parent in toks.iter().skip(1) {
            if let Some(&v) = id_of.get(parent) {
                edges[u].push(v);
            } else {
                touches_default[u] = true;
            }
        }
    }
    let mut is_cand = vec![false; n];
    let mut cand_nodes = Vec::new();
    for oid in &unique_oids {
        if let Some(&u) = id_of.get(oid)
            && !is_cand[u]
        {
            is_cand[u] = true;
            cand_nodes.push(u);
        }
    }
    let mut ahead = HashSet::new();
    let mut strict_ancestors: HashMap<String, HashSet<String>> = HashMap::new();
    let mut visited = vec![false; n];
    let mut touched = Vec::new();
    let mut stack = Vec::new();
    for u in cand_nodes {
        visited[u] = true;
        touched.push(u);
        stack.push(u);
        let mut has_base = touches_default[u];
        let mut ancs = HashSet::new();
        while let Some(cur) = stack.pop() {
            for &nxt in &edges[cur] {
                if !visited[nxt] {
                    visited[nxt] = true;
                    touched.push(nxt);
                    has_base |= touches_default[nxt];
                    if is_cand[nxt] {
                        ancs.insert(oids[nxt].clone());
                    }
                    stack.push(nxt);
                }
            }
        }
        for idx in touched.drain(..) {
            visited[idx] = false;
        }
        if has_base {
            ahead.insert(oids[u].clone());
        }
        if !ancs.is_empty() {
            strict_ancestors.insert(oids[u].clone(), ancs);
        }
    }
    let in_graph = oids.into_iter().collect();
    AncestryGraph { in_graph, ahead, strict_ancestors }
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
    let mut add_nul = |raw: &str| {
        for entry in raw.split('\0') {
            if let Some(norm) = clean_exact_repo_path(entry)
                && seen.insert(norm.clone())
            {
                files.push(norm);
            }
        }
    };
    if let Some(base) = base_oid.filter(|b| safe_git_ref(b)) {
        let range = format!("{base}...HEAD");
        if let Some(diff) = git_stdout(wt, &["diff", "--name-only", "-z", &range]).await {
            add_nul(&diff);
        }
    }
    if let Some(diff) = git_stdout(wt, &["diff", "--name-only", "-z", "HEAD"]).await {
        add_nul(&diff);
    }
    if let Some(untracked) = git_stdout(wt, &["ls-files", "--others", "--exclude-standard", "-z"]).await {
        add_nul(&untracked);
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
    pub(crate) include_file_conflicts: bool,
    pub(crate) include_code_deps: bool,
}

pub(crate) struct SourceControlAnalysis {
    pub(crate) pr_stacks: Vec<PrStackEdge>,
    pub(crate) file_conflicts: Vec<FileConflictEdge>,
    pub(crate) code_deps: Vec<CodeDepEdge>,
    pub(crate) static_issue_paths: BTreeMap<String, BTreeSet<TargetPath>>,
    pub(crate) analyzed_issues: HashSet<String>,
}

pub(crate) struct LiveConflictInput<'a> {
    pub(crate) repo: &'a str,
    pub(crate) candidate: u64,
    pub(crate) running: &'a [u64],
    pub(crate) aliases: &'a HashMap<String, String>,
    pub(crate) parent_of: &'a BTreeMap<String, Vec<String>>,
    pub(crate) closed_issues: &'a HashSet<String>,
    pub(crate) repo_prs: Option<&'a RepoPrs>,
    pub(crate) local: Option<&'a LocalRepoInfo>,
    pub(crate) active_branches: &'a [ActiveBranchLink],
    pub(crate) static_paths: &'a BTreeMap<String, BTreeSet<TargetPath>>,
}

struct CandidateResolveCtx<'a> {
    aliases: &'a HashMap<String, String>,
    parent_of: &'a BTreeMap<String, Vec<String>>,
    closed_issues: &'a HashSet<String>,
    active_branches: &'a [ActiveBranchLink],
}

pub(crate) fn extract_static_issue_paths(
    touches: &[String],
    body: &str,
    local: Option<&LocalRepoInfo>,
) -> BTreeSet<TargetPath> {
    let verified_local = local.filter(|l| !l.tree_files.is_empty());
    let mut paths = BTreeSet::new();

    // Add Touches: paths (verified against local tree when present, else syntactically valid repo paths).
    for raw in touches {
        let Some((norm, is_dir)) = normalize_repo_path(raw) else { continue };
        if !looks_like_repo_path(&norm, is_dir) {
            continue;
        }
        if let Some(l) = verified_local {
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
    if let Some(l) = verified_local {
        for (norm, is_dir) in extract_body_file_paths(body) {
            if !is_dir && l.tree_files.contains(&norm) {
                paths.insert(TargetPath::File(norm));
            } else if l.tree_dirs.contains(&norm) {
                paths.insert(TargetPath::Dir(norm));
            }
        }
    }
    paths
}

async fn collect_repo_candidates(
    repo_display: &str,
    default_branch: &str,
    repo_prs: Option<&RepoPrs>,
    local: Option<&LocalRepoInfo>,
    ctx: &CandidateResolveCtx<'_>,
) -> Vec<BranchCandidate> {
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
        let issues = resolve_pr_issues(pr, ctx.aliases, ctx.active_branches, ctx.parent_of);
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
            let issues = resolve_worktree_issues(repo_display, wt, ctx.aliases, ctx.active_branches, ctx.parent_of);
            if !issues.is_empty() && issues.iter().all(|(r, n)| ctx.closed_issues.contains(&issue_key(r, *n))) {
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
    candidates
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
    if slot.base_repo.is_none() {
        slot.base_repo = incoming.base_repo;
    }
    if slot.base_oid.is_none() {
        slot.base_oid = incoming.base_oid;
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
    slot.direct |= incoming.direct;
}

pub(crate) fn reduce_ancestry_stacks(edges: Vec<PrStackEdge>) -> Vec<PrStackEdge> {
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &edges {
        let from = issue_key(&e.from.0, e.from.1);
        let to = issue_key(&e.to.0, e.to.1);
        let list = adj.entry(from).or_default();
        if !list.contains(&to) {
            list.push(to);
        }
    }
    let mut kept: Vec<PrStackEdge> = Vec::with_capacity(edges.len());
    for e in edges {
        if e.direct {
            kept.push(e);
            continue;
        }
        let from = issue_key(&e.from.0, e.from.1);
        let to = issue_key(&e.to.0, e.to.1);
        let redundant = adj.get(&from).is_some_and(|mids| {
            mids.iter()
                .any(|mid| mid != &from && mid != &to && reachable(&adj, mid, &to) && !reachable(&adj, mid, &from))
        });
        if !redundant {
            kept.push(e);
        }
    }
    kept
}

pub(crate) async fn is_local_branch_covered_by_merged_commit(root: &Path, local_oid: &str, merged_oid: &str) -> bool {
    if merged_oid.is_empty() || local_oid.is_empty() {
        return false;
    }
    if merged_oid.eq_ignore_ascii_case(local_oid) {
        return true;
    }
    let Some(resolved) = resolve_commit_oid(root, merged_oid).await else {
        return false;
    };
    is_strict_ancestor(root, local_oid, &resolved).await
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
    let resolve_ctx = CandidateResolveCtx {
        aliases: input.aliases,
        parent_of: input.parent_of,
        closed_issues: input.closed_issues,
        active_branches: input.active_branches,
    };

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

        let candidates = collect_repo_candidates(&repo_display, &default_branch, repo_prs, local, &resolve_ctx).await;

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
                            base_repo: Some(repo_display.clone()),
                            base_oid: candidates[a_idx].local_oid.clone(),
                            pr: candidates[b_idx].pr_number,
                            base_pr: candidates[a_idx].pr_number,
                            part: None,
                            direct: true,
                        };
                        direct_edges
                            .entry((kb, ka))
                            .and_modify(|slot| merge_pr_stack_edge(slot, edge.clone()))
                            .or_insert(edge);
                    }
                }
            }
        }

        // 2. Local git branch ancestry via a single batched `git rev-list --parents --stdin` traversal.
        if let Some(l) = local
            && let Some(default_oid) = l.default_oid.as_deref()
        {
            let cand_oids: Vec<&str> = candidates.iter().filter_map(|c| c.local_oid.as_deref()).collect();
            let ancestry = compute_candidate_ancestry(&l.root, default_oid, &cand_oids).await;
            for a_idx in 0..candidates.len() {
                let Some(oid_a) = candidates[a_idx].local_oid.as_deref() else { continue };
                if !ancestry.is_ahead(oid_a) {
                    continue;
                }
                for b_idx in 0..candidates.len() {
                    if a_idx == b_idx {
                        continue;
                    }
                    let Some(oid_b) = candidates[b_idx].local_oid.as_deref() else { continue };
                    if !ancestry.is_strict_ancestor(oid_a, oid_b) {
                        continue;
                    }
                    // Prefer the closest ancestor as base_oid for worktree diff
                    if let Some(prev_oid) = stack_base_oid[b_idx].as_deref() {
                        if ancestry.is_strict_ancestor(prev_oid, oid_a) || !ancestry.in_graph(prev_oid) {
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
                                base_repo: Some(repo_display.clone()),
                                base_oid: Some(oid_a.to_string()),
                                pr: candidates[b_idx].pr_number,
                                base_pr: candidates[a_idx].pr_number,
                                part: None,
                                direct: false,
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
        if input.include_file_conflicts || input.include_code_deps {
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
    }

    // Merge direct and ancestry edges without premature transitive reduction (reduction runs after endpoint verification).
    let mut merged_stacks: BTreeMap<(String, String), PrStackEdge> = direct_edges;
    for (pair, edge) in ancestry_edges {
        merged_stacks.entry(pair).and_modify(|slot| merge_pr_stack_edge(slot, edge.clone())).or_insert(edge);
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
                        base_repo: None,
                        base_oid: None,
                        pr: None,
                        base_pr: None,
                        part: Some(label.clone()),
                        direct: true,
                    };
                    merged_stacks
                        .entry((kc, kp))
                        .and_modify(|slot| merge_pr_stack_edge(slot, edge.clone()))
                        .or_insert(edge);
                }
            }
        }
    }

    // 5. File-conflict and advisory code-dependency edges across open issues in each repo.
    let mut file_conflicts = Vec::new();
    let mut code_deps = Vec::new();
    let mut static_issue_paths: BTreeMap<String, BTreeSet<TargetPath>> = BTreeMap::new();
    let mut analyzed_issues: HashSet<String> = HashSet::new();
    if input.include_file_conflicts || input.include_code_deps {
        type RepoIssuePaths = ((String, u64), String, BTreeSet<TargetPath>);
        let mut by_repo: BTreeMap<String, Vec<RepoIssuePaths>> = BTreeMap::new();
        let mut by_repo_code: BTreeMap<String, Vec<RepoIssueCodeInput>> = BTreeMap::new();
        for item in input.open_issues {
            let canon = canonicalize_repo(input.aliases, item.repo);
            let rkey = canon.to_ascii_lowercase();
            let ikey = issue_key(&canon, item.number);
            if !analyzed_issues.insert(ikey.clone()) {
                continue;
            }
            let local = input.local_repos.get(&rkey);
            let static_paths = extract_static_issue_paths(item.touches, item.body, local);
            let mut paths = issue_extra_files.remove(&ikey).unwrap_or_default();
            if !static_paths.is_empty() {
                paths.extend(static_paths.iter().cloned());
                static_issue_paths.insert(ikey.clone(), static_paths);
            }
            if input.include_code_deps {
                by_repo_code.entry(rkey.clone()).or_default().push(RepoIssueCodeInput {
                    repo: canon.clone(),
                    number: item.number,
                    key: ikey.clone(),
                    paths: paths.clone(),
                    body: item.body.to_string(),
                });
            }
            if input.include_file_conflicts && !paths.is_empty() {
                by_repo.entry(rkey).or_default().push(((canon, item.number), ikey, paths));
            }
        }

        for (_, mut items) in by_repo {
            items.sort_by(|a, b| a.0.0.to_ascii_lowercase().cmp(&b.0.0.to_ascii_lowercase()).then(a.0.1.cmp(&b.0.1)));
            for i in 0..items.len() {
                for j in (i + 1)..items.len() {
                    let (ref issue_a, ref key_a, ref paths_a) = items[i];
                    let (ref issue_b, ref key_b, ref paths_b) = items[j];
                    if is_ancestor_issue(key_a, key_b, input.parent_of)
                        || is_ancestor_issue(key_b, key_a, input.parent_of)
                    {
                        continue;
                    }
                    let shared = overlap_paths(paths_a, paths_b);
                    if !shared.is_empty() {
                        file_conflicts.push(FileConflictEdge {
                            from: issue_a.clone(),
                            to: issue_b.clone(),
                            files: shared,
                        });
                    }
                }
            }
        }

        if input.include_code_deps {
            for (rkey, repo_issues) in by_repo_code {
                if repo_issues.len() < 2 || repo_issues.iter().all(|i| i.paths.is_empty()) {
                    continue;
                }
                let Some(local) = input.local_repos.get(&rkey) else { continue };
                if local.tree_files.is_empty() {
                    continue;
                }
                let local_owned = local.clone();
                let parent_owned = input.parent_of.clone();
                if let Ok(found) = tokio::task::spawn_blocking(move || {
                    crate::flow_code::analyze_repo_code_deps(&local_owned, &repo_issues, &parent_owned)
                })
                .await
                {
                    code_deps.extend(found);
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
    code_deps.sort_by(|a, b| {
        a.from
            .0
            .to_ascii_lowercase()
            .cmp(&b.from.0.to_ascii_lowercase())
            .then(a.from.1.cmp(&b.from.1))
            .then_with(|| a.to.0.to_ascii_lowercase().cmp(&b.to.0.to_ascii_lowercase()))
            .then(a.to.1.cmp(&b.to.1))
    });

    SourceControlAnalysis { pr_stacks, file_conflicts, code_deps, static_issue_paths, analyzed_issues }
}

pub(crate) async fn live_repo_conflicts_with_running(input: LiveConflictInput<'_>) -> Vec<(String, u64, Vec<String>)> {
    let canon_repo = canonicalize_repo(input.aliases, input.repo);
    let repo_key = canon_repo.to_ascii_lowercase();
    let cand_key = issue_key(&canon_repo, input.candidate);
    let running_nums: BTreeSet<u64> = input
        .running
        .iter()
        .copied()
        .filter(|&n| {
            if n == input.candidate {
                return false;
            }
            let rk = issue_key(&canon_repo, n);
            !is_ancestor_issue(&cand_key, &rk, input.parent_of) && !is_ancestor_issue(&rk, &cand_key, input.parent_of)
        })
        .collect();
    if running_nums.is_empty() {
        return Vec::new();
    }
    let relevant: HashSet<String> =
        std::iter::once(cand_key.clone()).chain(running_nums.iter().map(|&n| issue_key(&canon_repo, n))).collect();

    let default_branch = input
        .repo_prs
        .and_then(|rp| rp.default_branch.clone())
        .or_else(|| input.local.and_then(|l| l.default_branch.clone()))
        .unwrap_or_else(|| "main".to_string());
    let ctx = CandidateResolveCtx {
        aliases: input.aliases,
        parent_of: input.parent_of,
        closed_issues: input.closed_issues,
        active_branches: input.active_branches,
    };
    let candidates = collect_repo_candidates(&canon_repo, &default_branch, input.repo_prs, input.local, &ctx).await;

    let mut stack_base_oid: Vec<Option<String>> = vec![None; candidates.len()];
    for (b_idx, cand_b) in candidates.iter().enumerate() {
        let Some(base_ref) = cand_b.base_ref_name.as_deref() else { continue };
        if base_ref.is_empty() || base_ref == default_branch {
            continue;
        }
        for (a_idx, cand_a) in candidates.iter().enumerate() {
            if a_idx == b_idx || cand_a.is_cross_repository || cand_a.pr_number.is_none() {
                continue;
            }
            if cand_a.branch != base_ref || cand_a.branch == default_branch {
                continue;
            }
            if stack_base_oid[b_idx].is_none() {
                stack_base_oid[b_idx] = cand_a.local_oid.clone();
            }
        }
    }
    if let Some(l) = input.local
        && let Some(default_oid) = l.default_oid.as_deref()
    {
        let cand_oids: Vec<&str> = candidates.iter().filter_map(|c| c.local_oid.as_deref()).collect();
        let ancestry = compute_candidate_ancestry(&l.root, default_oid, &cand_oids).await;
        for (a_idx, cand_a) in candidates.iter().enumerate() {
            let Some(oid_a) = cand_a.local_oid.as_deref() else { continue };
            if !ancestry.is_ahead(oid_a) {
                continue;
            }
            for (b_idx, cand_b) in candidates.iter().enumerate() {
                if a_idx == b_idx {
                    continue;
                }
                let Some(oid_b) = cand_b.local_oid.as_deref() else { continue };
                if !ancestry.is_strict_ancestor(oid_a, oid_b) {
                    continue;
                }
                if let Some(prev_oid) = stack_base_oid[b_idx].as_deref() {
                    if ancestry.is_strict_ancestor(prev_oid, oid_a) || !ancestry.in_graph(prev_oid) {
                        stack_base_oid[b_idx] = Some(oid_a.to_string());
                    }
                } else {
                    stack_base_oid[b_idx] = Some(oid_a.to_string());
                }
            }
        }
    }

    let mut live_paths: BTreeMap<String, BTreeSet<TargetPath>> = BTreeMap::new();
    for k in &relevant {
        if let Some(paths) = input.static_paths.get(k) {
            live_paths.insert(k.clone(), paths.clone());
        }
    }
    let default_oid = input.local.and_then(|l| l.default_oid.as_deref());
    for (idx, c) in candidates.iter().enumerate() {
        let matched: Vec<String> = c
            .issues
            .iter()
            .filter(|(r, _)| r.eq_ignore_ascii_case(&canon_repo) || r.eq_ignore_ascii_case(&repo_key))
            .map(|(r, n)| issue_key(r, *n))
            .filter(|k| relevant.contains(k))
            .collect();
        if matched.is_empty() {
            continue;
        }
        let mut c_files = c.pr_files.clone();
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
        for k in matched {
            let entry = live_paths.entry(k).or_default();
            for f in &c_files {
                entry.insert(TargetPath::File(f.clone()));
            }
        }
    }

    let Some(cand_paths) = live_paths.get(&cand_key) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rnum in running_nums {
        let rk = issue_key(&canon_repo, rnum);
        if let Some(r_paths) = live_paths.get(&rk) {
            let shared = overlap_paths(cand_paths, r_paths);
            if !shared.is_empty() {
                out.push((canon_repo.clone(), rnum, shared));
            }
        }
    }
    out
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
        assert_eq!(parse_branch_issue("feat/résumé-12"), None);
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
    fn pr_body_ref_scanner_handles_multibyte_utf8_without_panicking() {
        let body = "\
Résumé of changes.
Fixes nação and #12
Refs #14 and naïve
Élan closes #19
";
        let refs = extract_pr_body_issue_refs("acme/widgets", body);
        assert_eq!(refs, vec![("acme/widgets".into(), 14), ("acme/widgets".into(), 19)]);
    }

    #[test]
    fn clean_exact_repo_path_preserves_spaces_and_unicode_and_rejects_traversal() {
        assert_eq!(clean_exact_repo_path("docs/Release Notes.md"), Some("docs/Release Notes.md".into()));
        assert_eq!(clean_exact_repo_path("docs/café (v2)/[id].md"), Some("docs/café (v2)/[id].md".into()));
        assert_eq!(clean_exact_repo_path("./docs/Release Notes.md"), Some("docs/Release Notes.md".into()));
        assert_eq!(clean_exact_repo_path("docs\\Release Notes.md"), Some("docs/Release Notes.md".into()));
        assert_eq!(clean_exact_repo_path(""), None);
        assert_eq!(clean_exact_repo_path("/etc/passwd"), None);
        assert_eq!(clean_exact_repo_path("../secret.txt"), None);
        assert_eq!(clean_exact_repo_path("docs/bad\nname.md"), None);
    }

    #[tokio::test]
    async fn compute_candidate_ancestry_batches_strict_ancestry_and_ahead_checks() {
        let td = tempfile::tempdir().unwrap();
        let repo = td.path();
        let git = |args: &[&str]| {
            let out = crate::platform::git()
                .current_dir(repo)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-b", "main"]);
        std::fs::write(repo.join("f.txt"), "0\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "c0"]);
        let behind = git(&["rev-parse", "HEAD"]);
        std::fs::write(repo.join("f.txt"), "1\n").unwrap();
        git(&["commit", "-am", "main"]);
        let main_oid = git(&["rev-parse", "HEAD"]);

        git(&["checkout", "-b", "a"]);
        std::fs::write(repo.join("f.txt"), "a\n").unwrap();
        git(&["commit", "-am", "a"]);
        let a_oid = git(&["rev-parse", "HEAD"]);

        git(&["checkout", "-b", "b"]);
        std::fs::write(repo.join("f.txt"), "b\n").unwrap();
        git(&["commit", "-am", "b"]);
        let b_oid = git(&["rev-parse", "HEAD"]);

        git(&["checkout", "-b", "c"]);
        std::fs::write(repo.join("f.txt"), "c\n").unwrap();
        git(&["commit", "-am", "c"]);
        let c_oid = git(&["rev-parse", "HEAD"]);

        git(&["checkout", "-b", "sibling", "main"]);
        std::fs::write(repo.join("s.txt"), "s\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "sibling"]);
        let sib_oid = git(&["rev-parse", "HEAD"]);

        git(&["checkout", "--orphan", "orphan"]);
        git(&["rm", "-rf", "."]);
        std::fs::write(repo.join("o.txt"), "o\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "orphan"]);
        let orphan_oid = git(&["rev-parse", "HEAD"]);

        let graph = compute_candidate_ancestry(
            repo,
            &main_oid,
            &[&behind, &main_oid, &a_oid, &a_oid, &b_oid, &c_oid, &sib_oid, &orphan_oid],
        )
        .await;

        assert!(!graph.is_ahead(&behind));
        assert!(!graph.is_ahead(&main_oid));
        assert!(!graph.is_ahead(&orphan_oid));
        assert!(graph.is_ahead(&a_oid));
        assert!(graph.is_ahead(&b_oid));
        assert!(graph.is_ahead(&c_oid));
        assert!(graph.is_ahead(&sib_oid));

        assert!(graph.is_strict_ancestor(&a_oid, &b_oid));
        assert!(graph.is_strict_ancestor(&a_oid, &c_oid));
        assert!(graph.is_strict_ancestor(&b_oid, &c_oid));
        assert!(!graph.is_strict_ancestor(&a_oid, &a_oid));
        assert!(!graph.is_strict_ancestor(&b_oid, &a_oid));
        assert!(!graph.is_strict_ancestor(&a_oid, &sib_oid));
        assert!(!graph.is_strict_ancestor(&sib_oid, &c_oid));
        assert!(!graph.is_strict_ancestor(&orphan_oid, &c_oid));
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
