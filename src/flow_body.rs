use std::collections::{BTreeMap, HashSet};

use crate::board::{gh_name, parse_github_issue_only};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BodyRef {
    Issue {
        repo: String,
        number: u64,
    },
    Symbolic(String),
    /// Unquoted text after a comma following a structured ref on a line-start blocker directive
    /// (e.g. `After #42, Setup auth` vs `After #42, the board renders edges.`). Resolves when it
    /// matches a sibling title and is ignored as trailing prose when no candidate matches.
    OptionalSymbolic(String),
    Malformed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RefRole {
    Blocker,
    Parent,
    SubIssue,
}

impl RefRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Blocker => "blocker",
            Self::Parent => "parent",
            Self::SubIssue => "sub-issue",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedBody {
    pub(crate) blockers: Vec<BodyRef>,
    pub(crate) parents: Vec<BodyRef>,
    pub(crate) sub_issues: Vec<BodyRef>,
    pub(crate) track: Option<String>,
    pub(crate) touches: Vec<String>,
    pub(crate) declared_phase: Option<u32>,
}

impl ParsedBody {
    pub(crate) fn has_symbolic(&self) -> bool {
        self.blockers
            .iter()
            .chain(&self.parents)
            .any(|r| matches!(r, BodyRef::Symbolic(_) | BodyRef::OptionalSymbolic(_)))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) repo: String,
    pub(crate) number: u64,
    pub(crate) label: String,
    pub(crate) title: String,
    pub(crate) parent_key: Option<String>,
    pub(crate) declared_phase: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolveOutcome {
    Resolved(String, u64),
    NotFound,
    Ambiguous(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlowListKey {
    BlockedBy,
    Parent,
    Touches,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Quote,
    ListItem { content_col: usize },
}

#[derive(Clone, Debug)]
struct ActiveFence {
    containers: Vec<Container>,
    ch: u8,
    len: usize,
}

#[derive(Clone, Copy)]
struct LineCursor<'a> {
    rest: &'a str,
    virtual_spaces: usize,
    col: usize,
}

impl<'a> LineCursor<'a> {
    fn new(line: &'a str) -> Self {
        Self { rest: line, virtual_spaces: 0, col: 0 }
    }

    fn is_blank(&self) -> bool {
        self.rest.bytes().all(|b| b == b' ' || b == b'\t' || b == b'\r')
    }

    fn leading_cols(&self) -> (usize, usize) {
        let mut cols = self.virtual_spaces;
        let mut cur_col = self.col + self.virtual_spaces;
        let bytes = self.rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b' ' => {
                    cols += 1;
                    cur_col += 1;
                    i += 1;
                }
                b'\t' => {
                    let adv = 4 - (cur_col % 4);
                    cols += adv;
                    cur_col += adv;
                    i += 1;
                }
                _ => break,
            }
        }
        (cols, i)
    }

    fn consume_cols(&mut self, mut need: usize) -> bool {
        let use_virt = need.min(self.virtual_spaces);
        self.virtual_spaces -= use_virt;
        self.col += use_virt;
        need -= use_virt;
        let bytes = self.rest.as_bytes();
        let mut i = 0;
        while need > 0 && i < bytes.len() {
            match bytes[i] {
                b' ' => {
                    need -= 1;
                    self.col += 1;
                    i += 1;
                }
                b'\t' => {
                    let adv = 4 - (self.col % 4);
                    if adv <= need {
                        need -= adv;
                        self.col += adv;
                    } else {
                        self.col += need;
                        self.virtual_spaces = adv - need;
                        need = 0;
                    }
                    i += 1;
                }
                _ => return false,
            }
        }
        self.rest = &self.rest[i..];
        need == 0
    }

    fn consume_quote(&mut self) -> bool {
        let (cols, off) = self.leading_cols();
        if cols > 3 || self.rest.as_bytes().get(off) != Some(&b'>') {
            return false;
        }
        self.col += cols + 1;
        self.virtual_spaces = 0;
        self.rest = &self.rest[off + 1..];
        match self.rest.as_bytes().first() {
            Some(b' ') => {
                self.col += 1;
                self.rest = &self.rest[1..];
            }
            Some(b'\t') => {
                let adv = 4 - (self.col % 4);
                self.col += 1;
                self.virtual_spaces = adv - 1;
                self.rest = &self.rest[1..];
            }
            _ => {}
        }
        true
    }

    fn consume_list_marker(&mut self) -> Option<usize> {
        let (indent, off) = self.leading_cols();
        if indent > 3 {
            return None;
        }
        let after = &self.rest[off..];
        let bytes = after.as_bytes();
        let marker_len = match bytes.first().copied()? {
            b'-' | b'*' | b'+' if bytes.len() == 1 || matches!(bytes[1], b' ' | b'\t' | b'\r') => 1,
            b'0'..=b'9' => {
                let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
                if digits == 0 || digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
                    return None;
                }
                if bytes.len() == digits + 1 || matches!(bytes[digits + 1], b' ' | b'\t' | b'\r') {
                    digits + 1
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // Thematic breaks ("- - -", "* * *") are not list items.
        let mch = bytes[0];
        if matches!(mch, b'-' | b'*') {
            let rest_after = &after[1..];
            if rest_after.bytes().all(|b| b == mch || b == b' ' || b == b'\t' || b == b'\r')
                && rest_after.bytes().filter(|&b| b == mch).count() >= 2
            {
                return None;
            }
        }
        self.col += indent + marker_len;
        self.virtual_spaces = 0;
        self.rest = &after[marker_len..];
        if self.is_blank() {
            self.rest = "";
            return Some(indent + marker_len + 1);
        }
        let (after_sp, _) = self.leading_cols();
        if after_sp == 0 {
            return None;
        }
        let sp = if after_sp >= 5 { 1 } else { after_sp };
        self.consume_cols(sp);
        Some(indent + marker_len + sp)
    }
}

fn match_containers<'a>(raw_line: &'a str, containers: &[Container]) -> (usize, LineCursor<'a>) {
    let mut cursor = LineCursor::new(raw_line);
    let mut matched = 0;
    for &c in containers {
        match c {
            Container::Quote => {
                if !cursor.consume_quote() {
                    break;
                }
            }
            Container::ListItem { content_col } => {
                if cursor.is_blank() {
                    matched += 1;
                    continue;
                }
                if cursor.leading_cols().0 < content_col || !cursor.consume_cols(content_col) {
                    break;
                }
            }
        }
        matched += 1;
    }
    (matched, cursor)
}

fn fence_open(cursor: &LineCursor<'_>) -> Option<(u8, usize)> {
    let (cols, off) = cursor.leading_cols();
    if cols > 3 {
        return None;
    }
    let bytes = &cursor.rest.as_bytes()[off..];
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

fn fence_close(cursor: &LineCursor<'_>, open_ch: u8, open_len: usize) -> bool {
    let (cols, off) = cursor.leading_cols();
    if cols > 3 {
        return false;
    }
    let bytes = &cursor.rest.as_bytes()[off..];
    let len = bytes.iter().take_while(|b| **b == open_ch).count();
    if len < open_len {
        return false;
    }
    bytes[len..].iter().all(|b| *b == b' ' || *b == b'\t' || *b == b'\r')
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
            let mut started_comment = false;
            while i < bytes.len() {
                if bytes[i] == b'`' {
                    if let Some((_, end)) = find_inline_code_end(bytes, i) {
                        // Preserve inline code spans verbatim so `<!--` in backticks never starts a comment.
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
                    started_comment = true;
                    break;
                } else {
                    let ch = rest[i..].chars().next().expect("valid utf-8 boundary");
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
            if !started_comment {
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
            let ch = line[i..].chars().next().expect("valid utf-8 boundary");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < 3 && bytes[i] == b' ' {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
    }
    let level = i - start;
    if !(1..=6).contains(&level) {
        return None;
    }
    if i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\t' {
        return None;
    }
    let mut text = line[i..].trim();
    // Strip optional ATX closing hashes preceded by whitespace.
    let trimmed_end = text.trim_end_matches('#');
    if trimmed_end.len() < text.len()
        && (trimmed_end.is_empty() || trimmed_end.ends_with(' ') || trimmed_end.ends_with('\t'))
    {
        text = trimmed_end.trim_end();
    }
    Some((level, text))
}

fn is_dep_heading(title: &str) -> bool {
    let norm = title.trim().trim_end_matches(':').to_ascii_lowercase();
    matches!(norm.as_str(), "dependencies" | "depends on" | "blockers" | "blocked by" | "blocked-by" | "prerequisites")
}

fn is_placeholder(s: &str) -> bool {
    let lower =
        s.trim().trim_matches(|c| matches!(c, '`' | '"' | '\'' | '(' | ')' | '_' | '*')).trim().to_ascii_lowercase();
    matches!(lower.as_str(), "" | "none" | "-" | "--" | "—" | "–" | "n/a" | "na" | "nothing" | "no response")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlowFormSection {
    Parent,
    BlockedBy,
    Track,
    Touches,
}

fn flow_form_heading(title: &str) -> Option<FlowFormSection> {
    let norm = title.trim().trim_end_matches(':').to_ascii_lowercase();
    match norm.as_str() {
        "parent / epic" | "parent/epic" | "parent" => Some(FlowFormSection::Parent),
        "blocked by" | "blocked-by" => Some(FlowFormSection::BlockedBy),
        "track" => Some(FlowFormSection::Track),
        "affected files" | "touches" => Some(FlowFormSection::Touches),
        _ => None,
    }
}

fn strip_bullet_and_quote(line: &str) -> (usize, &str) {
    let mut s = line;
    let indent = s.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
    s = s.trim_start();
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start();
    }
    if let Some(rest) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")).or_else(|| s.strip_prefix("+ ")) {
        return (indent, rest.trim_start());
    }
    // Ordered list marker like "1. " or "2) "
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0
        && let Some(after) = s[digits..].strip_prefix(". ").or_else(|| s[digits..].strip_prefix(") "))
    {
        return (indent, after.trim_start());
    }
    (indent, s)
}

fn unwrap_bold_key(s: &str) -> String {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("**").and_then(|x| x.strip_suffix("**")) {
        return inner.trim().to_string();
    }
    if let Some(inner) = s.strip_prefix("__").and_then(|x| x.strip_suffix("__")) {
        return inner.trim().to_string();
    }
    s.to_string()
}

fn split_flow_kv(s: &str) -> Option<(String, &str)> {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("**").or_else(|| trimmed.strip_prefix("__")) {
        let marker = &trimmed[..2];
        if let Some(end) = rest.find(marker) {
            let inside = rest[..end].trim();
            let after = rest[end + 2..].trim_start();
            if let Some(key) = inside.strip_suffix(':') {
                return Some((key.trim().to_ascii_lowercase(), after));
            }
            if let Some(val) = after.strip_prefix(':') {
                return Some((inside.to_ascii_lowercase(), val.trim_start()));
            }
        }
    }
    let (key, val) = trimmed.split_once(':')?;
    let key = unwrap_bold_key(key).to_ascii_lowercase();
    if key.is_empty() || key.contains('#') || key.contains("http") || key.len() > 32 {
        return None;
    }
    Some((key, val.trim()))
}

fn strip_wrappers(s: &str) -> &str {
    let mut cur = s.trim();
    loop {
        let next = cur
            .strip_prefix('`')
            .and_then(|x| x.strip_suffix('`'))
            .or_else(|| cur.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
            .or_else(|| cur.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .or_else(|| cur.strip_prefix("**").and_then(|x| x.strip_suffix("**")))
            .map(str::trim);
        match next {
            Some(n) if n.len() < cur.len() => cur = n,
            _ => return cur,
        }
    }
}

fn parse_touches_items(raw: &str, out: &mut Vec<String>) {
    for part in raw.split([',', ';']) {
        let p = strip_wrappers(part).trim();
        if !p.is_empty() && !is_placeholder(p) && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
        }
    }
}

fn is_boundary_before(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    !prev.is_ascii_alphanumeric() && !matches!(prev, b'/' | b'-' | b'_' | b'.' | b'&')
}

fn is_boundary_after(bytes: &[u8], idx: usize) -> bool {
    match bytes.get(idx) {
        None => true,
        Some(b) => !b.is_ascii_alphanumeric() && *b != b'_' && *b != b'-',
    }
}

fn next_char(s: &str, i: usize) -> usize {
    i + s[i..].chars().next().map_or(1, char::len_utf8)
}

const MAX_GRAPHQL_ISSUE_NUMBER: u64 = i32::MAX as u64;

fn is_valid_issue_number(number: u64) -> bool {
    number > 0 && number <= MAX_GRAPHQL_ISSUE_NUMBER
}

fn is_opening_quote_byte(bytes: &[u8], i: usize) -> bool {
    match bytes[i] {
        b'"' => true,
        b'\'' => i == 0 || !bytes[i - 1].is_ascii_alphanumeric(),
        _ => false,
    }
}

fn extract_explicit_refs(default_repo: &str, segment: &str) -> (Vec<BodyRef>, Vec<String>) {
    let mut found = Vec::new();
    let mut malformed = Vec::new();
    let bytes = segment.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // 1. Markdown link: [text](url)
        if bytes[i] == b'['
            && let Some(close_br) = segment[i + 1..].find(']')
        {
            let after_br = i + 1 + close_br + 1;
            if bytes.get(after_br) == Some(&b'(')
                && let Some(close_p) = segment[after_br + 1..].find(')')
            {
                let url = segment[after_br + 1..after_br + 1 + close_p].trim();
                if let Some(issue) = parse_github_issue_only(url)
                    && let Some((owner, repo, number)) = issue.github_parts()
                {
                    if is_valid_issue_number(number) {
                        found.push(BodyRef::Issue { repo: format!("{owner}/{repo}"), number });
                    } else {
                        malformed.push(url.to_string());
                    }
                    i = after_br + 1 + close_p + 1;
                    continue;
                }
            }
        }

        // 2. Bare GitHub issue URL: http(s)://(www.)?github.com/owner/repo/issues/N
        if (segment[i..].len() >= 19)
            && (segment[i..].get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
                || segment[i..].get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")))
        {
            let mut end = i;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && !matches!(bytes[end], b')' | b']' | b'>') {
                end += 1;
            }
            let raw_url = segment[i..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if let Some(issue) = parse_github_issue_only(raw_url)
                && let Some((owner, repo, number)) = issue.github_parts()
            {
                if is_valid_issue_number(number) {
                    found.push(BodyRef::Issue { repo: format!("{owner}/{repo}"), number });
                } else {
                    malformed.push(raw_url.to_string());
                }
                i = end;
                continue;
            }
            // Skip past non-issue URL so URL fragments like https://example.com/#123 don't match #123.
            i = end;
            continue;
        }

        // 3. Cross-repo ref: owner/repo#N (or malformed owner/repo#...)
        if bytes[i] == b'#' {
            // Check backwards for owner/repo immediately before '#'.
            let mut prefix_start = i;
            while prefix_start > 0
                && (bytes[prefix_start - 1].is_ascii_alphanumeric()
                    || matches!(bytes[prefix_start - 1], b'/' | b'-' | b'_' | b'.'))
            {
                prefix_start -= 1;
            }
            let prefix = &segment[prefix_start..i];
            let mut num_end = i + 1;
            while num_end < bytes.len() && (bytes[num_end].is_ascii_alphanumeric() || bytes[num_end] == b'_') {
                num_end += 1;
            }
            let num_part = &segment[i + 1..num_end];

            if !prefix.is_empty() {
                let parts: Vec<&str> = prefix.split('/').collect();
                if parts.len() == 2 && gh_name(parts[0]) && gh_name(parts[1]) && is_boundary_before(bytes, prefix_start)
                {
                    if let Ok(number) = num_part.parse::<u64>()
                        && is_valid_issue_number(number)
                        && is_boundary_after(bytes, num_end)
                    {
                        found.push(BodyRef::Issue { repo: format!("{}/{}", parts[0], parts[1]), number });
                    } else {
                        malformed.push(segment[prefix_start..num_end].to_string());
                    }
                    i = num_end;
                    continue;
                }
                i += 1;
                continue;
            }

            // 4. Same-repo ref: #N (or malformed #0 / #abc / out-of-range / bare #)
            if is_boundary_before(bytes, i) {
                if !num_part.is_empty() && num_part.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(number) = num_part.parse::<u64>()
                        && is_valid_issue_number(number)
                    {
                        found.push(BodyRef::Issue { repo: default_repo.to_string(), number });
                    } else {
                        malformed.push(segment[i..num_end].to_string());
                    }
                    i = num_end;
                    continue;
                } else if num_part.is_empty() || num_part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    // Reject markdown heading hashes like "##", only flag "#N" / "#foo" / bare "#"
                    if segment[i..].starts_with("##") {
                        while i < bytes.len() && bytes[i] == b'#' {
                            i += 1;
                        }
                        continue;
                    }
                    malformed.push(segment[i..num_end].to_string());
                    i = num_end;
                    continue;
                }
            }
        }

        i = next_char(segment, i);
    }

    (found, malformed)
}

fn split_top_level(raw: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut in_quote: Option<char> = None;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = in_quote {
            cur.push(ch);
            if ch == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        match ch {
            '"' => {
                in_quote = Some(ch);
                cur.push(ch);
            }
            '\'' if i == 0 || !chars[i - 1].is_alphanumeric() => {
                in_quote = Some(ch);
                cur.push(ch);
            }
            '(' => {
                paren += 1;
                cur.push(ch);
            }
            ')' => {
                paren = paren.saturating_sub(1);
                cur.push(ch);
            }
            '[' => {
                bracket += 1;
                cur.push(ch);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                cur.push(ch);
            }
            ',' | ';' if paren == 0 && bracket == 0 => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
        i += 1;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

fn looks_like_structured_ref_piece(s: &str) -> bool {
    let t = strip_wrappers(s).trim();
    if t.is_empty() {
        return false;
    }
    t.contains('#')
        || t.to_ascii_lowercase().contains("github.com/")
        || extract_phase_number(t).is_some()
        || (s.trim().starts_with('"') && s.trim().ends_with('"'))
        || (s.trim().starts_with('\'') && s.trim().ends_with('\''))
}

fn split_and_clauses(segment: &str) -> Vec<String> {
    // Split on top-level " and " or " & " only when both sides look like structured refs
    // (explicit #N, URL, phase ref, or quoted title) so an unquoted title containing "and" stays intact.
    let bytes = segment.as_bytes();
    let mut split_points = Vec::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i = next_char(segment, i);
            continue;
        }
        match b {
            b'"' | b'\'' if is_opening_quote_byte(bytes, i) => in_quote = Some(b),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            _ if paren == 0 && bracket == 0 => {
                if bytes[i..].len() >= 5 && bytes[i..i + 5].eq_ignore_ascii_case(b" and ") {
                    split_points.push((i, i + 5));
                    i += 5;
                    continue;
                }
                if bytes[i..].starts_with(b" & ") {
                    split_points.push((i, i + 3));
                    i += 3;
                    continue;
                }
            }
            _ => {}
        }
        i = next_char(segment, i);
    }
    if split_points.is_empty() {
        return vec![segment.to_string()];
    }
    let mut pieces = Vec::new();
    let mut prev = 0;
    for (s, e) in split_points {
        pieces.push(segment[prev..s].trim().to_string());
        prev = e;
    }
    pieces.push(segment[prev..].trim().to_string());
    if pieces.iter().all(|p| looks_like_structured_ref_piece(p)) { pieces } else { vec![segment.to_string()] }
}

fn parse_ref_list(default_repo: &str, raw: &str, strip_backticks: bool) -> Vec<BodyRef> {
    let cleaned_owned;
    let src = if strip_backticks {
        cleaned_owned = raw.replace('`', "");
        cleaned_owned.as_str()
    } else {
        raw
    };
    let trimmed = src.trim().trim_end_matches(['.', '!', '?']).trim();
    if is_placeholder(trimmed) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (seg_idx, seg) in split_top_level(trimmed).into_iter().enumerate() {
        let seg = if seg_idx > 0 {
            let t = seg.trim_start();
            strip_leading_kw(t, "and").or_else(|| t.strip_prefix('&')).map_or(seg.as_str(), str::trim_start).to_string()
        } else {
            seg
        };
        for piece in split_and_clauses(&seg) {
            let piece = piece.trim().trim_end_matches(['.', '!', '?']).trim();
            if piece.is_empty() || is_placeholder(piece) {
                continue;
            }
            let (explicit, malformed) = extract_explicit_refs(default_repo, piece);
            if !explicit.is_empty() || !malformed.is_empty() {
                out.extend(explicit);
                for m in malformed {
                    out.push(BodyRef::Malformed(m));
                }
            } else {
                let sym = strip_wrappers(piece).trim();
                if !sym.is_empty() && !is_placeholder(sym) {
                    out.push(BodyRef::Symbolic(sym.to_string()));
                }
            }
        }
    }
    out
}

fn push_unique_ref(list: &mut Vec<BodyRef>, item: BodyRef) {
    for existing in list.iter_mut() {
        match (&*existing, &item) {
            (BodyRef::Issue { repo: r1, number: n1 }, BodyRef::Issue { repo: r2, number: n2 })
                if n1 == n2 && r1.eq_ignore_ascii_case(r2) =>
            {
                return;
            }
            (BodyRef::Symbolic(a), BodyRef::Symbolic(b) | BodyRef::OptionalSymbolic(b))
                if a.eq_ignore_ascii_case(b) =>
            {
                return;
            }
            (BodyRef::OptionalSymbolic(a), BodyRef::OptionalSymbolic(b)) if a.eq_ignore_ascii_case(b) => {
                return;
            }
            (BodyRef::OptionalSymbolic(a), BodyRef::Symbolic(b)) if a.eq_ignore_ascii_case(b) => {
                *existing = item;
                return;
            }
            (BodyRef::Malformed(a), BodyRef::Malformed(b)) if a.eq_ignore_ascii_case(b) => {
                return;
            }
            _ => {}
        }
    }
    list.push(item);
}

fn strip_leading_kw<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let bytes = s.as_bytes();
    let klen = kw.len();
    if bytes.len() < klen || !s.get(..klen).is_some_and(|p| p.eq_ignore_ascii_case(kw)) {
        return None;
    }
    let after = &s[klen..];
    if after.is_empty() || after.starts_with([' ', '\t', ':']) {
        return Some(after);
    }
    None
}

fn match_phase_of(s: &str) -> Option<(u32, &str)> {
    // Matches "Phase P<k> of <rest>" or "Phase <k> of <rest>" (case-insensitive)
    let after_phase = strip_leading_kw(s, "phase")?.trim_start();
    let after_p = after_phase
        .strip_prefix('P')
        .or_else(|| after_phase.strip_prefix('p'))
        .filter(|r| r.as_bytes().first().is_some_and(u8::is_ascii_digit))
        .unwrap_or(after_phase);
    let digits = after_p.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let k: u32 = after_p[..digits].parse().ok()?;
    let after_num = after_p[digits..].trim_start();
    let after_of = strip_leading_kw(after_num, "of")?;
    let rest = after_of.trim_start().strip_prefix(':').unwrap_or(after_of).trim_start();
    Some((k, rest))
}

fn is_negated_prefix(prefix: &str) -> bool {
    let lower = prefix.trim_end().to_ascii_lowercase();
    for neg in ["not", "no longer", "never", "doesn't", "does not", "don't", "do not", "isn't", "is not", "without"] {
        if let Some(before) = lower.strip_suffix(neg)
            && (before.is_empty() || before.as_bytes().last().is_some_and(|b| !b.is_ascii_alphanumeric()))
        {
            return true;
        }
    }
    false
}

fn starts_with_explicit_ref(s: &str) -> bool {
    let t = s.trim_start().trim_start_matches("**").trim_start_matches("__").trim_start();
    if t.is_empty() {
        return false;
    }
    if t.starts_with('#') {
        return t.as_bytes().get(1).is_some_and(u8::is_ascii_alphanumeric);
    }
    if t.starts_with('[') && t.contains("](") && t.to_ascii_lowercase().contains("github.com/") {
        return true;
    }
    if t.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
        || t.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"))
    {
        return t.to_ascii_lowercase().contains("github.com/");
    }
    let token = t.split_whitespace().next().unwrap_or_default();
    token.split_once('#').is_some_and(|(prefix, _)| {
        prefix.split_once('/').is_some_and(|(o, r)| !r.contains('/') && gh_name(o) && gh_name(r))
    })
}

fn starts_with_quoted(s: &str) -> bool {
    let t = s.trim_start().trim_start_matches("**").trim_start_matches("__").trim_start();
    t.starts_with(['"', '\''])
}

fn starts_with_explicit_or_phase_or_quote(s: &str) -> bool {
    if starts_with_explicit_ref(s) || starts_with_quoted(s) {
        return true;
    }
    let t = s.trim_start().trim_start_matches("**").trim_start_matches("__").trim_start();
    extract_leading_phase(t).is_some()
}

fn strip_colon_and_bold(s: &str) -> (bool, &str) {
    let s = s.trim_start();
    let s = s.strip_prefix("**").or_else(|| s.strip_prefix("__")).unwrap_or(s).trim_start();
    let (has_colon, s) = match s.strip_prefix(':') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, s),
    };
    let s = s.strip_prefix("**").or_else(|| s.strip_prefix("__")).unwrap_or(s).trim_start();
    (has_colon, s)
}

fn extract_leading_phase(s: &str) -> Option<(u32, usize)> {
    let t = s.trim_start();
    let offset = s.len() - t.len();
    if let Some(after_phase) = strip_leading_kw(t, "phase") {
        let trimmed = after_phase.trim_start();
        let after_p = trimmed
            .strip_prefix('P')
            .or_else(|| trimmed.strip_prefix('p'))
            .filter(|r| r.as_bytes().first().is_some_and(u8::is_ascii_digit))
            .unwrap_or(trimmed);
        let digits = after_p.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && is_boundary_after(after_p.as_bytes(), digits)
            && let Ok(k) = after_p[..digits].parse::<u32>()
        {
            let consumed = offset + (t.len() - after_p.len()) + digits;
            return Some((k, consumed));
        }
    }
    if let Some(after_p) = t.strip_prefix('P').or_else(|| t.strip_prefix('p')) {
        let digits = after_p.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && is_boundary_after(after_p.as_bytes(), digits)
            && let Ok(k) = after_p[..digits].parse::<u32>()
        {
            return Some((k, offset + 1 + digits));
        }
    }
    None
}

fn extract_phase_number(s: &str) -> Option<u32> {
    let t = strip_wrappers(s).trim();
    extract_leading_phase(t).map(|(k, _)| k)
}

fn slice_directive_tail(rest: &str, inside_parens: bool) -> (&str, &str) {
    // Capture up to closing ')' (if opened in parens), sentence break ". " before another directive
    // or non-continuation clause, or ';' before another directive keyword.
    let bytes = rest.as_bytes();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' if is_opening_quote_byte(bytes, i) => in_quote = Some(b),
            b'(' => paren += 1,
            b')' => {
                if paren == 0 && inside_parens {
                    return (&rest[..i], &rest[i + 1..]);
                }
                paren = paren.saturating_sub(1);
            }
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'|' if paren == 0 && bracket == 0 => {
                return (&rest[..i], &rest[i + 1..]);
            }
            b'.' if paren == 0 && bracket == 0 && bytes.get(i + 1).is_some_and(u8::is_ascii_whitespace) => {
                return (&rest[..i], &rest[i + 1..]);
            }
            b';' if paren == 0 && bracket == 0 => {
                let after = rest[i + 1..].trim_start();
                if starts_with_directive_keyword(after) {
                    return (&rest[..i], after);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (rest, "")
}

fn starts_with_directive_keyword(s: &str) -> bool {
    let s = s.trim_start().trim_start_matches('*');
    strip_leading_kw(s, "depends on").is_some()
        || strip_leading_kw(s, "blocked by").is_some()
        || strip_leading_kw(s, "blocked-by").is_some()
        || strip_leading_kw(s, "part of").is_some()
        || strip_leading_kw(s, "after").is_some()
        || match_phase_of(s).is_some()
}

fn target_starts_with_ref_or_backticked_ref(s: &str) -> bool {
    let trimmed = s.trim_start().trim_start_matches("**").trim_start_matches("__").trim_start();
    if starts_with_explicit_or_phase_or_quote(trimmed) {
        return true;
    }
    let bytes = trimmed.as_bytes();
    if bytes.first() == Some(&b'`')
        && let Some((content_start, end)) = find_inline_code_end(bytes, 0)
    {
        let inner = trimmed[content_start..end - content_start].trim();
        if starts_with_explicit_or_phase_or_quote(inner) {
            return true;
        }
    }
    let stripped = strip_inline_code(trimmed);
    starts_with_explicit_or_phase_or_quote(stripped.trim_start())
}

fn contains_prose_directive_cue(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    let unbold = trimmed.strip_prefix("**").or_else(|| trimmed.strip_prefix("__")).unwrap_or(trimmed);
    if match_phase_of(unbold).is_some()
        || strip_leading_kw(unbold, "depends on").is_some()
        || strip_leading_kw(unbold, "blocked by").is_some()
        || strip_leading_kw(unbold, "blocked-by").is_some()
        || strip_leading_kw(unbold, "part of").is_some()
    {
        return true;
    }
    if let Some(after_kw) = strip_leading_kw(unbold, "after") {
        let (has_colon, rest) = strip_colon_and_bold(after_kw);
        if has_colon || target_starts_with_ref_or_backticked_ref(rest) {
            return true;
        }
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < trimmed.len() {
        if bytes[i] == b'`' {
            if let Some((_, end)) = find_inline_code_end(bytes, i) {
                i = end;
            } else {
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
            }
            continue;
        }
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            i = next_char(trimmed, i);
            continue;
        }
        let rest_here = &trimmed[i..];
        if match_phase_of(rest_here).is_some() {
            return true;
        }
        for kw in ["depends on", "blocked by", "blocked-by", "part of", "after"] {
            if let Some(after_kw) = strip_leading_kw(rest_here, kw) {
                let t = after_kw.trim_start();
                let has_colon = t.starts_with(':');
                let target_src = t.strip_prefix(':').unwrap_or(t).trim_start();
                if target_starts_with_ref_or_backticked_ref(target_src)
                    || (has_colon && kw != "after")
                    || (kw != "after" && is_negated_prefix(&strip_inline_code(&trimmed[..i])))
                {
                    return true;
                }
            }
        }
        i = next_char(trimmed, i);
    }
    false
}

fn sanitize_form_refs(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some((content_start, end)) = find_inline_code_end(bytes, i) {
                let tick_len = content_start - i;
                let inner = &raw[content_start..end - tick_len];
                if contains_prose_directive_cue(inner) {
                    out.push(' ');
                } else {
                    out.push_str(inner);
                }
                i = end;
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                out.push_str(&raw[start..i]);
            }
        } else {
            let ch = raw[i..].chars().next().expect("valid utf-8 boundary");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn capture_inline_target(rest: &str, inside_parens: bool) -> (&str, &str) {
    // Used for mid-sentence inline directives: capture consecutive refs separated by ',', ';', 'and', '&',
    // optional parenthetical annotation "(Phase 1)", stopping before unrelated prose words.
    let (tail, after_tail) = slice_directive_tail(rest, inside_parens);
    // If tail starts with a quoted title or explicit ref or phase ref, scan ref-list items cleanly.
    let mut end = 0usize;
    let mut pos = 0usize;
    let bytes = tail.as_bytes();
    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let slice = &tail[pos..];
        let bold_skip = if slice.starts_with("**") || slice.starts_with("__") { 2 } else { 0 };
        let inner = &slice[bold_skip..];
        let inner_pos = pos + bold_skip;
        let item_end = if let Some(q @ (b'"' | b'\'')) = bytes.get(inner_pos).copied() {
            tail[inner_pos + 1..].find(q as char).map(|k| inner_pos + 1 + k + 1)
        } else if let Some((_, plen)) = extract_leading_phase(inner) {
            Some(inner_pos + plen)
        } else if inner.starts_with('[')
            && let Some(cb) = inner[1..].find(']')
            && inner[1 + cb + 1..].starts_with('(')
            && let Some(cp) = inner[1 + cb + 2..].find(')')
            && parse_github_issue_only(inner[1 + cb + 2..1 + cb + 2 + cp].trim()).is_some()
        {
            Some(inner_pos + 1 + cb + 2 + cp + 1)
        } else if inner.starts_with('#')
            || inner.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
            || inner.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"))
            || {
                let tok = inner.split_whitespace().next().unwrap_or_default();
                tok.split_once('#').is_some_and(|(pre, _)| {
                    pre.split_once('/').is_some_and(|(o, r)| !r.contains('/') && gh_name(o) && gh_name(r))
                })
            }
        {
            let mut k = inner_pos;
            while k < bytes.len() && !bytes[k].is_ascii_whitespace() && !matches!(bytes[k], b',' | b';' | b')') {
                k = next_char(tail, k);
            }
            Some(k)
        } else {
            None
        };

        let Some(mut next_pos) = item_end else {
            break;
        };
        if bold_skip == 2 && (tail[next_pos..].starts_with("**") || tail[next_pos..].starts_with("__")) {
            next_pos += 2;
        }
        // Consume optional trailing parenthetical annotation like "(Phase 1)"
        let after_item = tail[next_pos..].trim_start();
        if after_item.starts_with('(')
            && let Some(cp) = after_item.find(')')
        {
            next_pos = tail.len() - after_item.len() + cp + 1;
        }
        end = next_pos;
        pos = next_pos;
        // Look for list separator: ',', ';', ' and ', ' & '
        let after_ws = tail[pos..].trim_start();
        let ws_skip = tail[pos..].len() - after_ws.len();
        if let Some(r) = after_ws.strip_prefix(',').or_else(|| after_ws.strip_prefix(';')) {
            let after_sep = r.trim_start();
            // Optional "and" after comma: ", and #3"
            let sep_advance = if let Some(and_rest) = strip_leading_kw(after_sep, "and") {
                tail.len() - and_rest.len()
            } else {
                tail.len() - after_sep.len()
            };
            if starts_with_explicit_or_phase_or_quote(&tail[sep_advance..]) {
                pos = sep_advance;
                continue;
            }
            break;
        } else if let Some(and_rest) = strip_leading_kw(after_ws, "and").or_else(|| after_ws.strip_prefix('&')) {
            let after_and = and_rest.trim_start();
            if starts_with_explicit_or_phase_or_quote(after_and) {
                pos = pos + ws_skip + (after_ws.len() - after_and.len());
                continue;
            }
            break;
        } else if ws_skip > 0 && starts_with_explicit_ref(after_ws) {
            pos += ws_skip;
            continue;
        } else {
            break;
        }
    }
    if end == 0 || end == tail.len() { (tail, after_tail) } else { (&tail[..end], &rest[end..]) }
}

fn scan_unquoted_item_end(tail: &str, start: usize) -> usize {
    let bytes = tail.as_bytes();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut in_quote: Option<u8> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i = next_char(tail, i);
            continue;
        }
        match b {
            b'"' | b'\'' if is_opening_quote_byte(bytes, i) => in_quote = Some(b),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b',' | b';' if paren == 0 && bracket == 0 => return i,
            _ => {}
        }
        i = next_char(tail, i);
    }
    tail.len()
}

fn parse_line_blocker_refs<'a>(default_repo: &str, rest: &'a str) -> (Vec<BodyRef>, &'a str) {
    // Line-start blocker directive parser:
    // - When the directive starts with a structured ref (#N, owner/repo#N, URL, quoted title, or
    //   bare Phase N), consume structured items (including space-separated `#42 #43`) as hard refs,
    //   and emit any comma/semicolon-separated unquoted text after them as `OptionalSymbolic` so
    //   real sibling titles (`After #42, Setup auth`, `Depends on #42, Setup auth`) resolve while
    //   trailing prose (`Depends on #42, please review before merging.`, `After #42, the board
    //   renders edges.`) never becomes a false hard blocker.
    // - When the directive starts with an unquoted title (e.g. `Depends on Setup auth, Do database
    //   migration`), parse the whole directive tail with `parse_ref_list` (no prose denylist).
    let (tail, after_tail) = slice_directive_tail(rest, false);
    let starts_structured = starts_with_explicit_ref(tail)
        || starts_with_quoted(tail)
        || extract_leading_phase(tail)
            .is_some_and(|(_, plen)| !tail[plen..].trim_start().starts_with([':', '-', '—', '–']));
    if !starts_structured {
        return (parse_ref_list(default_repo, tail, false), after_tail);
    }

    let mut out = Vec::new();
    let mut end = 0usize;
    let mut pos = 0usize;
    let bytes = tail.as_bytes();
    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let slice = &tail[pos..];
        let bold_skip = if slice.starts_with("**") || slice.starts_with("__") { 2 } else { 0 };
        let inner = &slice[bold_skip..];
        let inner_pos = pos + bold_skip;
        let structured_end = if let Some(q @ (b'"' | b'\'')) = bytes.get(inner_pos).copied() {
            tail[inner_pos + 1..].find(q as char).map(|k| inner_pos + 1 + k + 1)
        } else if let Some((_, plen)) = extract_leading_phase(inner)
            && !inner[plen..].trim_start().starts_with([':', '-', '—', '–'])
        {
            Some(inner_pos + plen)
        } else if inner.starts_with('[')
            && let Some(cb) = inner[1..].find(']')
            && inner[1 + cb + 1..].starts_with('(')
            && let Some(cp) = inner[1 + cb + 2..].find(')')
            && parse_github_issue_only(inner[1 + cb + 2..1 + cb + 2 + cp].trim()).is_some()
        {
            Some(inner_pos + 1 + cb + 2 + cp + 1)
        } else if starts_with_explicit_ref(inner) {
            let mut k = inner_pos;
            while k < bytes.len() && !bytes[k].is_ascii_whitespace() && !matches!(bytes[k], b',' | b';' | b')') {
                k = next_char(tail, k);
            }
            Some(k)
        } else {
            None
        };

        if let Some(mut next_pos) = structured_end {
            if bold_skip == 2 && (tail[next_pos..].starts_with("**") || tail[next_pos..].starts_with("__")) {
                next_pos += 2;
            }
            let after_item = tail[next_pos..].trim_start();
            if after_item.starts_with('(')
                && let Some(cp) = after_item.find(')')
            {
                next_pos = tail.len() - after_item.len() + cp + 1;
            }
            for r in parse_ref_list(default_repo, &tail[pos..next_pos], false) {
                push_unique_ref(&mut out, r);
            }
            end = next_pos;
            pos = next_pos;
        } else {
            let next_pos = scan_unquoted_item_end(tail, pos);
            let raw_item = tail[pos..next_pos].trim().trim_end_matches(['.', '!', '?']).trim();
            if raw_item.is_empty() {
                break;
            }
            if extract_leading_phase(raw_item).is_some() {
                let sym = strip_wrappers(raw_item).trim();
                if !sym.is_empty() && !is_placeholder(sym) {
                    push_unique_ref(&mut out, BodyRef::Symbolic(sym.to_string()));
                }
            } else {
                let sym = strip_wrappers(raw_item).trim();
                if !sym.is_empty() && !is_placeholder(sym) {
                    push_unique_ref(&mut out, BodyRef::OptionalSymbolic(sym.to_string()));
                }
            }
            end = pos + tail[pos..next_pos].trim_end().len();
            pos = next_pos;
        }

        let after_ws = tail[pos..].trim_start();
        let ws_skip = tail[pos..].len() - after_ws.len();
        if after_ws.is_empty() {
            break;
        }
        if let Some(r) = after_ws.strip_prefix(',').or_else(|| after_ws.strip_prefix(';')) {
            let after_sep = r.trim_start();
            if after_sep.is_empty() || starts_with_directive_keyword(after_sep) {
                break;
            }
            let candidate = strip_leading_kw(after_sep, "and")
                .or_else(|| after_sep.strip_prefix('&'))
                .map_or(after_sep, str::trim_start);
            if candidate.is_empty() || starts_with_directive_keyword(candidate) {
                break;
            }
            pos = tail.len() - candidate.len();
            continue;
        } else if let Some(and_rest) = strip_leading_kw(after_ws, "and").or_else(|| after_ws.strip_prefix('&')) {
            let after_and = and_rest.trim_start();
            if starts_with_explicit_or_phase_or_quote(after_and) {
                pos = tail.len() - after_and.len();
                continue;
            }
            break;
        } else if ws_skip > 0 && starts_with_explicit_ref(after_ws) {
            pos += ws_skip;
            continue;
        } else {
            break;
        }
    }
    let rem = if end == 0 || end == tail.len() { after_tail } else { &rest[end..] };
    (out, rem)
}

fn scan_prose_segment(default_repo: &str, segment: &str, line_start: bool, parsed: &mut ParsedBody) -> Option<RefRole> {
    let mut s = segment.trim();
    if s.is_empty() {
        return None;
    }

    // 1. Line-start structured directives (can take unquoted sibling titles too!).
    if line_start {
        let unbold = s.strip_prefix("**").or_else(|| s.strip_prefix("__")).unwrap_or(s);
        if let Some((k, rest)) = match_phase_of(unbold) {
            parsed.declared_phase = parsed.declared_phase.or(Some(k));
            let (target, rem) = if starts_with_explicit_ref(rest) || starts_with_quoted(rest) {
                capture_inline_target(rest, false)
            } else {
                slice_directive_tail(rest, false)
            };
            for r in parse_ref_list(default_repo, target, false) {
                push_unique_ref(&mut parsed.parents, r);
            }
            if !rem.trim().is_empty() {
                scan_prose_segment(default_repo, rem, false, parsed);
            }
            return None;
        }
        for (kw, role) in [
            ("depends on", RefRole::Blocker),
            ("blocked by", RefRole::Blocker),
            ("blocked-by", RefRole::Blocker),
            ("part of", RefRole::Parent),
        ] {
            if let Some(after_kw) = strip_leading_kw(unbold, kw) {
                let (has_colon, rest) = strip_colon_and_bold(after_kw);
                if rest.is_empty() {
                    return has_colon.then_some(role);
                }
                let (refs, rem) = if role == RefRole::Parent {
                    let (target, rem) = if starts_with_explicit_ref(rest) || starts_with_quoted(rest) {
                        capture_inline_target(rest, false)
                    } else {
                        slice_directive_tail(rest, false)
                    };
                    (parse_ref_list(default_repo, target, false), rem)
                } else {
                    parse_line_blocker_refs(default_repo, rest)
                };
                let dest = match role {
                    RefRole::Blocker => &mut parsed.blockers,
                    RefRole::Parent => &mut parsed.parents,
                    RefRole::SubIssue => &mut parsed.sub_issues,
                };
                for r in refs {
                    push_unique_ref(dest, r);
                }
                if !rem.trim().is_empty() {
                    scan_prose_segment(default_repo, rem, false, parsed);
                }
                return None;
            }
        }
        if let Some(after_kw) = strip_leading_kw(unbold, "after") {
            let (has_colon, rest) = strip_colon_and_bold(after_kw);
            if rest.is_empty() {
                return has_colon.then_some(RefRole::Blocker);
            }
            if has_colon || starts_with_explicit_or_phase_or_quote(rest) {
                let (refs, rem) = parse_line_blocker_refs(default_repo, rest);
                for r in refs {
                    push_unique_ref(&mut parsed.blockers, r);
                }
                if !rem.trim().is_empty() {
                    scan_prose_segment(default_repo, rem, false, parsed);
                }
                return None;
            }
        }
    }

    // 2. Inline scan across the segment for explicit-ref / phase / quoted / colon directives.
    let mut i = 0;
    let mut paren_depth = 0usize;
    while i < s.len() {
        let bytes = s.as_bytes();
        let b = bytes[i];
        if b == b'(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if b == b')' {
            paren_depth = paren_depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            i = next_char(s, i);
            continue;
        }
        let rest_here = &s[i..];
        let prefix = &s[..i];

        // "Phase P?<k> of ..."
        if let Some((k, rest)) = match_phase_of(rest_here) {
            if !is_negated_prefix(prefix) {
                parsed.declared_phase = parsed.declared_phase.or(Some(k));
                if starts_with_explicit_or_phase_or_quote(rest) {
                    let (target, rem) = capture_inline_target(rest, paren_depth > 0);
                    for r in parse_ref_list(default_repo, target, false) {
                        push_unique_ref(&mut parsed.parents, r);
                    }
                    s = rem;
                    i = 0;
                    paren_depth = 0;
                    continue;
                }
            }
            i += 5;
            continue;
        }

        // Inline "Depends on", "Blocked by", "Blocked-by", "After", "Part of"
        let matched = [
            ("depends on", RefRole::Blocker),
            ("blocked by", RefRole::Blocker),
            ("blocked-by", RefRole::Blocker),
            ("part of", RefRole::Parent),
            ("after", RefRole::Blocker),
        ]
        .into_iter()
        .find_map(|(kw, role)| strip_leading_kw(rest_here, kw).map(|after| (kw, role, after)));

        if let Some((kw, role, after_kw)) = matched {
            if !is_negated_prefix(prefix) {
                let trimmed = after_kw.trim_start();
                let has_colon = trimmed.starts_with(':');
                let target_src = trimmed.strip_prefix(':').unwrap_or(trimmed).trim_start();
                if starts_with_explicit_or_phase_or_quote(target_src) || (has_colon && kw != "after") {
                    let (target, rem) = if starts_with_explicit_or_phase_or_quote(target_src) {
                        capture_inline_target(target_src, paren_depth > 0)
                    } else {
                        slice_directive_tail(target_src, paren_depth > 0)
                    };
                    let refs = parse_ref_list(default_repo, target, false);
                    let dest = match role {
                        RefRole::Blocker => &mut parsed.blockers,
                        RefRole::Parent => &mut parsed.parents,
                        RefRole::SubIssue => &mut parsed.sub_issues,
                    };
                    for r in refs {
                        push_unique_ref(dest, r);
                    }
                    s = rem;
                    i = 0;
                    paren_depth = 0;
                    continue;
                }
            }
            i += kw.len();
            continue;
        }

        let ch = rest_here.chars().next().expect("valid utf-8");
        i += ch.len_utf8();
    }
    None
}

fn parse_tasklist_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let after_bullet =
        trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ "))?;
    let after_box = after_bullet
        .strip_prefix("[ ] ")
        .or_else(|| after_bullet.strip_prefix("[x] "))
        .or_else(|| after_bullet.strip_prefix("[X] "))?;
    Some(after_box.trim_start())
}

fn extract_leading_issue_ref(default_repo: &str, text: &str) -> Option<BodyRef> {
    let t = text.trim_start().trim_start_matches("**").trim_start_matches("__").trim_start();
    if t.starts_with('[')
        && let Some(cb) = t[1..].find(']')
        && t[1 + cb + 1..].starts_with('(')
        && let Some(cp) = t[1 + cb + 2..].find(')')
    {
        let url = t[1 + cb + 2..1 + cb + 2 + cp].trim();
        if let Some(issue) = parse_github_issue_only(url)
            && let Some((owner, repo, number)) = issue.github_parts()
        {
            return Some(if is_valid_issue_number(number) {
                BodyRef::Issue { repo: format!("{owner}/{repo}"), number }
            } else {
                BodyRef::Malformed(url.to_string())
            });
        }
    }
    let token = t.split_whitespace().next()?;
    let clean = token.trim_end_matches([',', ';', ':', '.', ')', '*', '_']);
    let (explicit, malformed) = extract_explicit_refs(default_repo, clean);
    if let Some(first) = explicit.into_iter().next() {
        return Some(first);
    }
    if let Some(m) = malformed.into_iter().next() {
        return Some(BodyRef::Malformed(m));
    }
    None
}

pub(crate) struct FlowBodyLayout {
    pub(crate) section: Option<(usize, usize)>,
    pub(crate) unclosed_fence: Option<(u8, usize)>,
    pub(crate) unclosed_comment: bool,
}

pub(crate) fn inspect_flow_body(body: &str) -> FlowBodyLayout {
    let mut containers: Vec<Container> = Vec::new();
    let mut fence: Option<ActiveFence> = None;
    let mut in_comment = false;
    let mut flow_start: Option<(usize, usize)> = None;
    let mut section: Option<(usize, usize)> = None;
    let mut total_lines = 0usize;

    for (idx, raw_line) in body.lines().enumerate() {
        total_lines = idx + 1;
        if let Some(active) = fence.as_ref() {
            let (matched, cursor) = match_containers(raw_line, &active.containers);
            if matched == active.containers.len() {
                if fence_close(&cursor, active.ch, active.len) {
                    containers = active.containers.clone();
                    fence = None;
                }
                continue;
            }
            containers = active.containers[..matched].to_vec();
            fence = None;
        }
        let was_in_comment = in_comment;
        let line = strip_html_comments(raw_line, &mut in_comment);
        if line.trim().is_empty() {
            if !was_in_comment && !raw_line.trim().is_empty() {
                let (matched, _) = match_containers(raw_line, &containers);
                containers.truncate(matched);
            }
            continue;
        }
        let (matched, mut cursor) = match_containers(&line, &containers);
        containers.truncate(matched);
        if !cursor.is_blank() {
            loop {
                if cursor.consume_quote() {
                    containers.push(Container::Quote);
                    continue;
                }
                if let Some(content_col) = cursor.consume_list_marker() {
                    containers.push(Container::ListItem { content_col });
                    continue;
                }
                break;
            }
            if !in_comment
                && !cursor.is_blank()
                && let Some((ch, len)) = fence_open(&cursor)
            {
                fence = Some(ActiveFence { containers: containers.clone(), ch, len });
                continue;
            }
        }
        if containers.is_empty()
            && section.is_none()
            && let Some((level, heading)) = parse_heading(&line)
        {
            if let Some((start_idx, start_level)) = flow_start {
                if level <= start_level {
                    section = Some((start_idx, idx));
                }
            } else if (level == 2 || level == 3) && heading.eq_ignore_ascii_case("flow") {
                flow_start = Some((idx, level));
            }
        }
    }
    let section = section.or_else(|| flow_start.map(|(start, _)| (start, total_lines)));
    let unclosed_fence = fence.filter(|f| f.containers.is_empty()).map(|f| (f.ch, f.len));
    FlowBodyLayout { section, unclosed_fence, unclosed_comment: in_comment }
}

pub(crate) fn parse_body(default_repo: &str, body: &str) -> ParsedBody {
    let mut parsed = ParsedBody::default();
    let mut containers: Vec<Container> = Vec::new();
    let mut fence: Option<ActiveFence> = None;
    let mut in_comment = false;
    let mut flow_level: Option<usize> = None;
    let mut flow_list_key: Option<FlowListKey> = None;
    let mut flow_form_section: Option<FlowFormSection> = None;
    let mut pending_prose_role: Option<RefRole> = None;
    let mut section_role: Option<RefRole> = None;

    for raw_line in body.lines() {
        if let Some(active) = fence.as_ref() {
            let (matched, cursor) = match_containers(raw_line, &active.containers);
            if matched == active.containers.len() {
                if fence_close(&cursor, active.ch, active.len) {
                    containers = active.containers.clone();
                    fence = None;
                }
                continue;
            }
            // Enclosing blockquote or list item ended, closing the unclosed fence.
            containers = active.containers[..matched].to_vec();
            fence = None;
        }
        let was_in_comment = in_comment;
        let line = strip_html_comments(raw_line, &mut in_comment);
        if line.trim().is_empty() {
            if !was_in_comment && !raw_line.trim().is_empty() {
                // A comment opener at lower indentation (e.g. `<!--` at col 0) can close an
                // enclosing list item or blockquote without pushing markers from inside the comment.
                let (matched, _) = match_containers(raw_line, &containers);
                containers.truncate(matched);
            }
            flow_list_key = None;
            pending_prose_role = None;
            continue;
        }
        let (matched, mut cursor) = match_containers(&line, &containers);
        containers.truncate(matched);
        if !cursor.is_blank() {
            loop {
                if cursor.consume_quote() {
                    containers.push(Container::Quote);
                    continue;
                }
                if let Some(content_col) = cursor.consume_list_marker() {
                    containers.push(Container::ListItem { content_col });
                    continue;
                }
                break;
            }
            if !in_comment
                && !cursor.is_blank()
                && let Some((ch, len)) = fence_open(&cursor)
            {
                fence = Some(ActiveFence { containers: containers.clone(), ch, len });
                continue;
            }
        }

        if let Some((level, heading)) = parse_heading(&line) {
            flow_list_key = None;
            pending_prose_role = None;
            if let Some(fl) = flow_level
                && level <= fl
            {
                flow_level = None;
            }
            if level <= 3 {
                flow_form_section = None;
            }
            if (level == 2 || level == 3) && heading.eq_ignore_ascii_case("flow") {
                flow_level = Some(level);
                section_role = None;
                continue;
            }
            if level == 3
                && flow_level.is_none()
                && let Some(form_sec) = flow_form_heading(heading)
            {
                flow_form_section = Some(form_sec);
                section_role = match form_sec {
                    FlowFormSection::Parent => Some(RefRole::Parent),
                    FlowFormSection::BlockedBy => Some(RefRole::Blocker),
                    FlowFormSection::Track | FlowFormSection::Touches => None,
                };
                continue;
            }
            section_role = is_dep_heading(heading).then_some(RefRole::Blocker);
            let clean_heading = strip_inline_code(heading);
            scan_prose_segment(default_repo, &clean_heading, false, &mut parsed);
            continue;
        }

        if let Some(form_sec) = flow_form_section {
            let (_, raw_stripped) = strip_bullet_and_quote(&line);
            if is_placeholder(raw_stripped) {
                continue;
            }
            match form_sec {
                FlowFormSection::Track => {
                    let t = strip_wrappers(raw_stripped).trim();
                    if !t.is_empty() && !is_placeholder(t) && parsed.track.is_none() {
                        parsed.track = Some(t.to_string());
                    }
                    continue;
                }
                FlowFormSection::Touches => {
                    parse_touches_items(raw_stripped, &mut parsed.touches);
                    continue;
                }
                FlowFormSection::Parent | FlowFormSection::BlockedBy => {
                    let dest = match form_sec {
                        FlowFormSection::Parent => &mut parsed.parents,
                        FlowFormSection::BlockedBy => &mut parsed.blockers,
                        FlowFormSection::Track | FlowFormSection::Touches => unreachable!(),
                    };
                    let clean = strip_inline_code(&line);
                    let trimmed = clean.trim();
                    if pending_prose_role.is_none() && !trimmed.starts_with('|') {
                        if let Some(task_rest) = parse_tasklist_item(&line) {
                            let task_clean = strip_inline_code(task_rest);
                            if contains_prose_directive_cue(task_rest) {
                                pending_prose_role = scan_prose_segment(default_repo, &task_clean, true, &mut parsed);
                            } else {
                                let form_text = sanitize_form_refs(task_rest);
                                for r in parse_ref_list(default_repo, &form_text, false) {
                                    push_unique_ref(dest, r);
                                }
                            }
                            continue;
                        }
                        if !contains_prose_directive_cue(raw_stripped) {
                            let form_text = sanitize_form_refs(raw_stripped);
                            for r in parse_ref_list(default_repo, &form_text, false) {
                                push_unique_ref(dest, r);
                            }
                            continue;
                        }
                    }
                }
            }
        }

        if flow_level.is_some() {
            let (indent, stripped) = strip_bullet_and_quote(&line);
            if let Some((key, val)) = split_flow_kv(stripped) {
                flow_list_key = None;
                match key.as_str() {
                    "parent" => {
                        if val.is_empty() {
                            flow_list_key = Some(FlowListKey::Parent);
                        } else {
                            for r in parse_ref_list(default_repo, val, true) {
                                push_unique_ref(&mut parsed.parents, r);
                            }
                        }
                    }
                    "blocked by" | "blocked-by" | "depends on" => {
                        if val.is_empty() {
                            flow_list_key = Some(FlowListKey::BlockedBy);
                        } else {
                            for r in parse_ref_list(default_repo, val, true) {
                                push_unique_ref(&mut parsed.blockers, r);
                            }
                        }
                    }
                    "track" => {
                        let t = strip_wrappers(val).trim();
                        if !t.is_empty() && !is_placeholder(t) && parsed.track.is_none() {
                            parsed.track = Some(t.to_string());
                        }
                    }
                    "touches" => {
                        if val.is_empty() {
                            flow_list_key = Some(FlowListKey::Touches);
                        } else {
                            parse_touches_items(val, &mut parsed.touches);
                        }
                    }
                    "priority" => {}
                    _ => {}
                }
                continue;
            }
            if indent > 0
                && let Some(active) = flow_list_key
            {
                match active {
                    FlowListKey::Parent => {
                        for r in parse_ref_list(default_repo, stripped, true) {
                            push_unique_ref(&mut parsed.parents, r);
                        }
                    }
                    FlowListKey::BlockedBy => {
                        for r in parse_ref_list(default_repo, stripped, true) {
                            push_unique_ref(&mut parsed.blockers, r);
                        }
                    }
                    FlowListKey::Touches => parse_touches_items(stripped, &mut parsed.touches),
                }
            }
            continue;
        }

        // Outside ## Flow: strip inline code spans before parsing prose & tasklists.
        let clean = strip_inline_code(&line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Markdown table rows: scan each cell independently as inline text (never line-start).
        if trimmed.starts_with('|') {
            for cell in trimmed.split('|') {
                let cell = cell.trim();
                if !cell.is_empty() && !cell.bytes().all(|b| matches!(b, b'-' | b':' | b' ')) {
                    scan_prose_segment(default_repo, cell, false, &mut parsed);
                }
            }
            continue;
        }

        // Tasklist items: "- [ ] #N" or "- [x] #N"
        if let Some(task_rest) = parse_tasklist_item(&clean) {
            let role = pending_prose_role.or(section_role);
            if starts_with_directive_keyword(task_rest) {
                pending_prose_role = scan_prose_segment(default_repo, task_rest, true, &mut parsed);
            } else if let Some(item_ref) = extract_leading_issue_ref(default_repo, task_rest) {
                match role {
                    Some(RefRole::Blocker) => push_unique_ref(&mut parsed.blockers, item_ref),
                    Some(RefRole::Parent) => push_unique_ref(&mut parsed.parents, item_ref),
                    _ => push_unique_ref(&mut parsed.sub_issues, item_ref),
                }
            } else if let Some(r) = role
                && starts_with_explicit_or_phase_or_quote(task_rest)
            {
                let dest = match r {
                    RefRole::Blocker => &mut parsed.blockers,
                    RefRole::Parent => &mut parsed.parents,
                    RefRole::SubIssue => &mut parsed.sub_issues,
                };
                for item in parse_ref_list(default_repo, task_rest, false) {
                    push_unique_ref(dest, item);
                }
            }
            continue;
        }

        let is_bullet = trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ");
        let (_, stripped) = strip_bullet_and_quote(&clean);
        if is_bullet
            && let Some(role) = pending_prose_role
            && !starts_with_directive_keyword(stripped)
        {
            let dest = match role {
                RefRole::Blocker => &mut parsed.blockers,
                RefRole::Parent => &mut parsed.parents,
                RefRole::SubIssue => &mut parsed.sub_issues,
            };
            for item in parse_ref_list(default_repo, stripped, false) {
                push_unique_ref(dest, item);
            }
            continue;
        }
        pending_prose_role = scan_prose_segment(default_repo, stripped, true, &mut parsed);
    }

    parsed
}

fn normalize_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_space = true;
    for ch in strip_wrappers(raw).chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim_end().to_string()
}

fn strip_conventional_prefix(title: &str) -> Option<&str> {
    let (head, rest) = title.split_once(':')?;
    let kind = head.split_once('(').map_or(head, |(k, _)| k).trim().trim_end_matches('!');
    if matches!(
        kind.to_ascii_lowercase().as_str(),
        "feat" | "fix" | "chore" | "docs" | "refactor" | "test" | "perf" | "build" | "ci" | "style" | "revert"
    ) {
        Some(rest.trim())
    } else {
        None
    }
}

fn title_contains_phase(title: &str, target: u32) -> bool {
    let bytes = title.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            i = next_char(title, i);
            continue;
        }
        if let Some((k, len)) = extract_leading_phase(&title[i..]) {
            if k == target {
                return true;
            }
            i += len.max(1);
            continue;
        }
        i = next_char(title, i);
    }
    false
}

fn contains_word_bounded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs = start + pos;
        let end = abs + needle.len();
        let before_ok = abs == 0 || haystack.as_bytes()[abs - 1] == b' ';
        let after_ok = end == haystack.len() || haystack.as_bytes()[end] == b' ';
        if before_ok && after_ok {
            return true;
        }
        start = next_char(haystack, abs);
    }
    false
}

fn parse_phase_query(raw: &str) -> Option<(u32, String)> {
    let t = strip_wrappers(raw).trim();
    let (k, consumed) = extract_leading_phase(t)?;
    let rest = t[consumed..].trim().trim_start_matches([':', '-', '—', '–', '.']).trim();
    let subtitle = normalize_title(rest);
    Some((k, subtitle))
}

fn resolve_in_scope(query: &str, scope: &[&Candidate], optional: bool) -> ResolveOutcome {
    if scope.is_empty() {
        return ResolveOutcome::NotFound;
    }
    let q_norm = normalize_title(query);
    if q_norm.is_empty() {
        return ResolveOutcome::NotFound;
    }

    let decide = |matches: Vec<&Candidate>| -> ResolveOutcome {
        match matches.len() {
            0 => ResolveOutcome::NotFound,
            1 => ResolveOutcome::Resolved(matches[0].repo.clone(), matches[0].number),
            _ => {
                let mut labels: Vec<String> = matches.into_iter().map(|c| c.label.clone()).collect();
                labels.sort();
                labels.dedup();
                ResolveOutcome::Ambiguous(labels)
            }
        }
    };

    // Tier 1: exact normalized title match (full or conventional-commit-stripped).
    let exact: Vec<&Candidate> = scope
        .iter()
        .copied()
        .filter(|c| {
            normalize_title(&c.title) == q_norm
                || strip_conventional_prefix(&c.title).is_some_and(|s| normalize_title(s) == q_norm)
        })
        .collect();
    if !exact.is_empty() {
        return decide(exact);
    }

    // Tier 2: phase reference match (e.g. "Phase 1", "P1", "Phase P1: foo").
    if let Some((phase_num, subtitle)) = parse_phase_query(query) {
        let phase_hits: Vec<&Candidate> = scope
            .iter()
            .copied()
            .filter(|c| c.declared_phase == Some(phase_num) || title_contains_phase(&c.title, phase_num))
            .collect();
        if !phase_hits.is_empty() {
            if !subtitle.is_empty() {
                let narrowed: Vec<&Candidate> = phase_hits
                    .iter()
                    .copied()
                    .filter(|c| contains_word_bounded(&normalize_title(&c.title), &subtitle))
                    .collect();
                return decide(narrowed);
            }
            return decide(phase_hits);
        }
    }

    // Tier 3: word-bounded substring match on normalized title (guard against tiny/generic tokens).
    // For optional trailing segments after an explicit ref, require a multi-word phrase so a
    // single prose word never collides with an unrelated title.
    let alnum_len = q_norm.chars().filter(|c| c.is_alphanumeric()).count();
    let generic = matches!(
        q_norm.as_str(),
        "this" | "that" | "work" | "issue" | "task" | "item" | "step" | "part" | "epic" | "phase" | "next" | "prev"
    );
    if alnum_len >= 4 && !generic && (!optional || q_norm.contains(' ')) {
        let sub_hits: Vec<&Candidate> =
            scope.iter().copied().filter(|c| contains_word_bounded(&normalize_title(&c.title), &q_norm)).collect();
        if !sub_hits.is_empty() {
            return decide(sub_hits);
        }
    }

    ResolveOutcome::NotFound
}

fn resolve_symbolic_inner(
    query: &str,
    role: RefRole,
    me_repo: &str,
    me_number: u64,
    me_parent_key: Option<&str>,
    candidates: &[Candidate],
    optional: bool,
) -> ResolveOutcome {
    let same_repo: Vec<&Candidate> =
        candidates.iter().filter(|c| c.repo.eq_ignore_ascii_case(me_repo) && c.number != me_number).collect();

    if role == RefRole::Blocker
        && let Some(parent) = me_parent_key
    {
        let epic_siblings: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| {
                c.parent_key.as_deref().is_some_and(|p| p == parent)
                    && !(c.repo.eq_ignore_ascii_case(me_repo) && c.number == me_number)
            })
            .collect();
        let epic_result = resolve_in_scope(query, &epic_siblings, optional);
        match epic_result {
            ResolveOutcome::Resolved(_, _) | ResolveOutcome::Ambiguous(_) => return epic_result,
            ResolveOutcome::NotFound => {
                // Bare phase refs (e.g. "Phase 1", "P1") on an issue inside an epic must NOT
                // accidentally bind to a different epic's Phase 1 in the same repo.
                if parse_phase_query(query).is_some_and(|(_, sub)| sub.is_empty()) {
                    return ResolveOutcome::NotFound;
                }
            }
        }
    }

    resolve_in_scope(query, &same_repo, optional)
}

pub(crate) fn resolve_symbolic(
    query: &str,
    role: RefRole,
    me_repo: &str,
    me_number: u64,
    me_parent_key: Option<&str>,
    candidates: &[Candidate],
) -> ResolveOutcome {
    resolve_symbolic_inner(query, role, me_repo, me_number, me_parent_key, candidates, false)
}

pub(crate) fn resolve_symbolic_optional(
    query: &str,
    me_repo: &str,
    me_number: u64,
    me_parent_key: Option<&str>,
    candidates: &[Candidate],
) -> ResolveOutcome {
    resolve_symbolic_inner(query, RefRole::Blocker, me_repo, me_number, me_parent_key, candidates, true)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CycleComponent {
    pub(crate) cycle: Vec<String>,
    pub(crate) all_nodes: Vec<String>,
}

pub(crate) fn find_cycles(nodes: &[String], edges: &[(String, String)]) -> Vec<CycleComponent> {
    // Deterministic Tarjan's SCC over sorted node keys and sorted adjacency lists.
    // Returns each cycle component (SCC of size >= 2, or size 1 with a self-loop) as an ordered
    // representative cycle path (with all SCC members represented cleanly).
    let idx_of: BTreeMap<&str, usize> = nodes.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let n = nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut self_loops: HashSet<usize> = HashSet::new();
    for (from, to) in edges {
        let (Some(&u), Some(&v)) = (idx_of.get(from.as_str()), idx_of.get(to.as_str())) else {
            continue;
        };
        if u == v {
            self_loops.insert(u);
        } else if !adj[u].contains(&v) {
            adj[u].push(v);
        }
    }
    for list in &mut adj {
        list.sort_unstable();
    }

    struct Tarjan<'a> {
        adj: &'a [Vec<usize>],
        index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        sccs: Vec<Vec<usize>>,
    }
    impl Tarjan<'_> {
        fn strongconnect(&mut self, v: usize) {
            self.indices[v] = Some(self.index);
            self.lowlink[v] = self.index;
            self.index += 1;
            self.stack.push(v);
            self.on_stack[v] = true;

            for &w in &self.adj[v] {
                if self.indices[w].is_none() {
                    self.strongconnect(w);
                    self.lowlink[v] = self.lowlink[v].min(self.lowlink[w]);
                } else if self.on_stack[w] {
                    self.lowlink[v] = self.lowlink[v].min(self.indices[w].expect("visited"));
                }
            }

            if self.lowlink[v] == self.indices[v].expect("visited") {
                let mut comp = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                self.sccs.push(comp);
            }
        }
    }

    let mut t = Tarjan {
        adj: &adj,
        index: 0,
        indices: vec![None; n],
        lowlink: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    for v in 0..n {
        if t.indices[v].is_none() {
            t.strongconnect(v);
        }
    }

    let mut out = Vec::new();
    for comp in t.sccs {
        if comp.len() == 1 {
            let u = comp[0];
            if self_loops.contains(&u) {
                let id = nodes[u].clone();
                out.push(CycleComponent { cycle: vec![id.clone()], all_nodes: vec![id] });
            }
            continue;
        }
        // Order a deterministic shortest cycle starting from the lowest node in `comp`,
        // keeping `cycle` as an exact closed path and `all_nodes` as every node in the SCC.
        let members: HashSet<usize> = comp.iter().copied().collect();
        let start = comp[0];
        let mut parent: BTreeMap<usize, usize> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::new();
        let mut end_node = None;
        for &next in &adj[start] {
            if members.contains(&next) && !parent.contains_key(&next) {
                parent.insert(next, start);
                queue.push_back(next);
            }
        }
        while let Some(cur) = queue.pop_front() {
            if adj[cur].contains(&start) {
                end_node = Some(cur);
                break;
            }
            for &next in &adj[cur] {
                if members.contains(&next) && next != start && !parent.contains_key(&next) {
                    parent.insert(next, cur);
                    queue.push_back(next);
                }
            }
        }
        let mut cycle_idx = Vec::new();
        if let Some(mut cur) = end_node {
            let mut rev = vec![cur];
            while let Some(&p) = parent.get(&cur) {
                rev.push(p);
                if p == start {
                    break;
                }
                cur = p;
            }
            rev.reverse();
            cycle_idx.extend(rev);
        }
        let mut all_idx = cycle_idx.clone();
        for u in comp {
            if !all_idx.contains(&u) {
                all_idx.push(u);
            }
        }
        out.push(CycleComponent {
            cycle: cycle_idx.into_iter().map(|i| nodes[i].clone()).collect(),
            all_nodes: all_idx.into_iter().map(|i| nodes[i].clone()).collect(),
        });
    }
    out.sort_by(|a, b| a.all_nodes.cmp(&b.all_nodes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "acme/widgets";

    #[test]
    fn parses_canonical_flow_block_and_skips_fenced_code_and_comments() {
        let body = r#"
Some intro text mentioning `Depends on #99` and ``Blocked by #98`` in inline code.
<!-- Depends on #97 -->

```markdown
## Flow
- Parent: #999
- Blocked by: #888
- Track: fake-track
Depends on #777
- [ ] #666
```

~~~text
Blocked by #555
~~~

## Scope (Phase 2 of #254)

Depends on #255 (Phase 1) and acme/tools#12 (sync).

## Flow
- Parent: #254
- Blocked by: #255, acme/tools#12
- Track: flow-dag
- Priority: high
- Touches: `src/flow.rs`, `src/board.html`, `tests/all/flow.rs`
"#;
        let p = parse_body(REPO, body);
        assert_eq!(
            p.blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 255 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 12 },
            ]
        );
        assert_eq!(p.parents, vec![BodyRef::Issue { repo: REPO.into(), number: 254 }]);
        assert_eq!(p.track.as_deref(), Some("flow-dag"));
        assert_eq!(p.touches, vec!["src/flow.rs", "src/board.html", "tests/all/flow.rs"]);
        assert_eq!(p.declared_phase, Some(2));
        assert!(p.sub_issues.is_empty());
    }

    #[test]
    fn parses_prose_directives_tasklists_and_cross_repo_urls() {
        let body = r#"
Depends on #1, #2
Blocked by https://github.com/acme/tools/issues/3
After acme/tools#4
Part of #10
Phase P3 of #10
This does not depend on #99 and is no longer blocked by #98.
After deploying, check the logs.
The UI depends on the config file.

| Phase | Issue | Depends on | Summary |
|---|---|---|---|
| Phase 1 | #10 | - | Table header must not emit unresolved ref |

## Tasks
- [ ] #20 Child task A
- [x] acme/tools#21 Child task B
- [ ] Write unit tests for #999

## Dependencies
- [ ] #30 Prerequisite issue
"#;
        let p = parse_body(REPO, body);
        assert_eq!(
            p.blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 1 },
                BodyRef::Issue { repo: REPO.into(), number: 2 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 3 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 4 },
                BodyRef::Issue { repo: REPO.into(), number: 30 },
            ]
        );
        assert_eq!(p.parents, vec![BodyRef::Issue { repo: REPO.into(), number: 10 }]);
        assert_eq!(
            p.sub_issues,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 20 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 21 },
            ]
        );
        assert_eq!(p.declared_phase, Some(3));
    }

    #[test]
    fn resolves_sibling_phase_and_title_references_and_flags_ambiguity() {
        let cands = vec![
            Candidate {
                repo: REPO.into(),
                number: 11,
                label: "widgets#11".into(),
                title: "feat: zero-config graph".into(),
                parent_key: Some("acme/widgets#10".into()),
                declared_phase: Some(1),
            },
            Candidate {
                repo: REPO.into(),
                number: 12,
                label: "widgets#12".into(),
                title: "feat: Phase 2 API wiring".into(),
                parent_key: Some("acme/widgets#10".into()),
                declared_phase: Some(2),
            },
            Candidate {
                repo: REPO.into(),
                number: 21,
                label: "widgets#21".into(),
                title: "Phase 1: auth setup".into(),
                parent_key: Some("acme/widgets#20".into()),
                declared_phase: Some(1),
            },
        ];

        // Inside epic #10, "Phase 1" unambiguously resolves to #11 (not epic #20's #21).
        assert_eq!(
            resolve_symbolic("Phase 1", RefRole::Blocker, REPO, 12, Some("acme/widgets#10"), &cands),
            ResolveOutcome::Resolved(REPO.into(), 11)
        );
        // Outside any epic, "Phase 1" is ambiguous across #11 and #21.
        assert_eq!(
            resolve_symbolic("Phase 1", RefRole::Blocker, REPO, 30, None, &cands),
            ResolveOutcome::Ambiguous(vec!["widgets#11".into(), "widgets#21".into()])
        );
        // Exact conventional-stripped title resolves unambiguously.
        assert_eq!(
            resolve_symbolic("zero-config graph", RefRole::Blocker, REPO, 12, Some("acme/widgets#10"), &cands),
            ResolveOutcome::Resolved(REPO.into(), 11)
        );
        // Nonexistent phase in epic stays unresolved instead of binding across epics.
        assert_eq!(
            resolve_symbolic("Phase 2", RefRole::Blocker, REPO, 22, Some("acme/widgets#20"), &cands),
            ResolveOutcome::NotFound
        );
    }

    #[test]
    fn finds_self_loops_and_multi_node_cycles_deterministically() {
        let nodes = vec!["a#1".into(), "a#2".into(), "a#3".into(), "a#4".into(), "a#5".into()];
        let edges = vec![
            ("a#1".into(), "a#2".into()),
            ("a#2".into(), "a#1".into()),
            ("a#2".into(), "a#5".into()),
            ("a#5".into(), "a#2".into()),
            ("a#3".into(), "a#3".into()),
            ("a#4".into(), "a#1".into()),
        ];
        assert_eq!(
            find_cycles(&nodes, &edges),
            vec![
                CycleComponent {
                    cycle: vec!["a#1".into(), "a#2".into()],
                    all_nodes: vec!["a#1".into(), "a#2".into(), "a#5".into()],
                },
                CycleComponent { cycle: vec!["a#3".into()], all_nodes: vec!["a#3".into()] },
            ]
        );
    }

    #[test]
    fn handles_multibyte_utf8_and_multiline_lists_without_panicking() {
        let body = "Résumé — 🚀 Depends on **#1** and acme/tools#2 — café.\n\nBlocked by:\n- #3\n- Phase 1 — café\n\n## Flow\n- Track: `café-track`\n- Blocked by:\n  - #4\n- Touches:\n  - `src/flow.rs`\n";
        let p = parse_body(REPO, body);
        assert_eq!(
            p.blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 1 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 2 },
                BodyRef::Issue { repo: REPO.into(), number: 3 },
                BodyRef::Symbolic("Phase 1 — café".into()),
                BodyRef::Issue { repo: REPO.into(), number: 4 },
            ]
        );
        assert_eq!(p.track.as_deref(), Some("café-track"));
        assert_eq!(p.touches, vec!["src/flow.rs"]);
    }

    #[test]
    fn skips_fences_inside_blockquotes_and_stops_at_trailing_prose_after_explicit_refs() {
        let body = r#"
> Quoted example:
> ```markdown
> Depends on #42
> ## Flow
> - Blocked by: #43
> ```
> > ~~~text
> > Blocked by #44
> > ~~~

- Indented fence in list:
    ```
    Depends on #45
    ```

Depends on #10, which adds OAuth support.
Blocked by #11, #12, because they wire the API.
Part of #1, tracking the OAuth epic.
Phase P2 of #1, covering follow-up work.
After: #13, which ships the migration.
"#;
        let p = parse_body(REPO, body);
        let hard_blockers: Vec<&BodyRef> =
            p.blockers.iter().filter(|r| !matches!(r, BodyRef::OptionalSymbolic(_))).collect();
        assert_eq!(
            hard_blockers,
            vec![
                &BodyRef::Issue { repo: REPO.into(), number: 10 },
                &BodyRef::Issue { repo: REPO.into(), number: 11 },
                &BodyRef::Issue { repo: REPO.into(), number: 12 },
                &BodyRef::Issue { repo: REPO.into(), number: 13 },
            ]
        );
        assert_eq!(p.parents, vec![BodyRef::Issue { repo: REPO.into(), number: 1 }]);
        assert_eq!(p.declared_phase, Some(2));
    }

    #[test]
    fn enforces_list_fence_boundaries_and_closing_indent_and_mixed_line_refs() {
        // S5a: unclosed fence inside a list item ends when the list item ends.
        let list_unclosed = "\
- Example:
  ```
  Depends on #99

Depends on #42
";
        assert_eq!(parse_body(REPO, list_unclosed).blockers, vec![BodyRef::Issue { repo: REPO.into(), number: 42 }]);

        // S5b: unclosed fence right after "- " ends at sibling list item.
        let list_sibling = "\
- ```
  Depends on #98
- Depends on #43
";
        assert_eq!(parse_body(REPO, list_sibling).blockers, vec![BodyRef::Issue { repo: REPO.into(), number: 43 }]);

        // S5c: four-space-indented backticks inside a top-level fence do NOT close it early.
        let indented_inner = "\
```markdown
    ```
    Depends on #97
```
Depends on #44
";
        assert_eq!(parse_body(REPO, indented_inner).blockers, vec![BodyRef::Issue { repo: REPO.into(), number: 44 }]);

        // S8: list markers inside multiline HTML comments never leak into fence container tracking.
        let comment_list_fence = "\
<!--
- Example
  -->
  ```
Depends on #42
  ```
Depends on #43
";
        assert_eq!(
            parse_body(REPO, comment_list_fence).blockers,
            vec![BodyRef::Issue { repo: REPO.into(), number: 43 }]
        );

        // S6 + S11: space-separated explicit refs, colonless `After #42, Setup auth`, and
        // unquoted title lists (`Do database migration`) stay intact without any prose denylist,
        // while trailing unquoted prose after explicit refs stays soft (`OptionalSymbolic`).
        let mixed = "\
Depends on #42 #43
Blocked by #44, Setup auth
After #45, Setup auth
After #46, the board renders edges.
Depends on Setup auth, Do database migration
Depends on #47, please review before merging.
After: #48, Phase 1: Auth setup
";
        assert_eq!(
            parse_body(REPO, mixed).blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 42 },
                BodyRef::Issue { repo: REPO.into(), number: 43 },
                BodyRef::Issue { repo: REPO.into(), number: 44 },
                BodyRef::Symbolic("Setup auth".into()),
                BodyRef::Issue { repo: REPO.into(), number: 45 },
                BodyRef::Issue { repo: REPO.into(), number: 46 },
                BodyRef::OptionalSymbolic("the board renders edges".into()),
                BodyRef::Symbolic("Do database migration".into()),
                BodyRef::Issue { repo: REPO.into(), number: 47 },
                BodyRef::OptionalSymbolic("please review before merging".into()),
                BodyRef::Issue { repo: REPO.into(), number: 48 },
                BodyRef::Symbolic("Phase 1: Auth setup".into()),
            ]
        );
        // Standalone colonless After keeps Setup auth as an optional sibling reference.
        assert_eq!(
            parse_body(REPO, "After #42, Setup auth\n").blockers,
            vec![BodyRef::Issue { repo: REPO.into(), number: 42 }, BodyRef::OptionalSymbolic("Setup auth".into())]
        );
        // Standalone pure-title list preserves titles starting with "Do" (no clause-word denylist).
        assert_eq!(
            parse_body(REPO, "Depends on Setup auth, Do database migration\n").blockers,
            vec![BodyRef::Symbolic("Setup auth".into()), BodyRef::Symbolic("Do database migration".into())]
        );

        // Apostrophes inside words must not start a single-quoted string and swallow later blockers.
        assert_eq!(
            parse_body(REPO, "Depends on #1, Rotate team's keys, #2\n").blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 1 },
                BodyRef::OptionalSymbolic("Rotate team's keys".into()),
                BodyRef::Issue { repo: REPO.into(), number: 2 },
            ]
        );
        assert_eq!(
            parse_body(REPO, "Depends on Rotate team's keys, Fix user's login\n").blockers,
            vec![BodyRef::Symbolic("Rotate team's keys".into()), BodyRef::Symbolic("Fix user's login".into())]
        );

        // `<!--` inside inline backticks must not start an HTML comment that hides real directives.
        assert_eq!(
            parse_body(REPO, "Example `<!--` token.\nDepends on #2\n").blockers,
            vec![BodyRef::Issue { repo: REPO.into(), number: 2 }]
        );

        // Out-of-range GraphQL Int issue numbers are flagged malformed without dropping valid siblings.
        assert_eq!(
            parse_body(REPO, "Depends on #9999999999999, acme/tools#3000000000, #2\n").blockers,
            vec![
                BodyRef::Malformed("#9999999999999".into()),
                BodyRef::Malformed("acme/tools#3000000000".into()),
                BodyRef::Issue { repo: REPO.into(), number: 2 },
            ]
        );

        // Qualified phase subtitle with no match must stay NotFound rather than binding another Phase 1.
        let reporting_only = vec![Candidate {
            repo: REPO.into(),
            number: 11,
            label: "widgets#11".into(),
            title: "Phase 1: Reporting".into(),
            parent_key: Some("acme/widgets#10".into()),
            declared_phase: Some(1),
        }];
        assert_eq!(
            resolve_symbolic(
                "Phase 1: Authentication",
                RefRole::Blocker,
                REPO,
                12,
                Some("acme/widgets#10"),
                &reporting_only
            ),
            ResolveOutcome::NotFound
        );
        assert_eq!(
            resolve_symbolic("Phase 1", RefRole::Blocker, REPO, 12, Some("acme/widgets#10"), &reporting_only),
            ResolveOutcome::Resolved(REPO.into(), 11)
        );
    }

    #[test]
    fn parses_github_issue_form_flow_fields_and_ignores_no_response() {
        let filled = "\
### Problem or use case

Sync parsed links to GitHub.

### Parent / Epic

#254

### Blocked by

#255, acme/tools#12

### Track

flow-dag

### Affected files

`src/web.rs`, src/board.html
";
        let parsed = parse_body(REPO, filled);
        assert_eq!(parsed.parents, vec![BodyRef::Issue { repo: REPO.into(), number: 254 }]);
        assert_eq!(
            parsed.blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 255 },
                BodyRef::Issue { repo: "acme/tools".into(), number: 12 },
            ]
        );
        assert_eq!(parsed.track.as_deref(), Some("flow-dag"));
        assert_eq!(parsed.touches, vec!["src/web.rs", "src/board.html"]);

        let blank = "\
### Problem or use case

Just a normal bug report.

### Parent / Epic

_No response_

### Blocked by

_No response_

### Track

_No response_

### Affected files

_No response_
";
        let empty = parse_body(REPO, blank);
        assert!(empty.parents.is_empty(), "{empty:?}");
        assert!(empty.blockers.is_empty(), "{empty:?}");
        assert!(empty.track.is_none(), "{empty:?}");
        assert!(empty.touches.is_empty(), "{empty:?}");

        // Explicit directives inside ### Blocked by keep their prose semantics (e.g. Part of -> parent),
        // while structured tasklist lists, backticked refs, and bare sibling titles stay blockers.
        let mixed = "\
### Blocked by

- [ ] Part of #10
- [ ] Tracked here (part of #18)
- [ ] Not after `#19`
- [ ] Not after **`#23`**
- [ ] #11, #12
- [ ] `#16`
- [ ] #21 (see the `blocked by:` field)
#22 (see the `blocked by:` field and `part of #99`)
Setup auth
Clean up after migration
Add `sync-links` endpoint
Part of #13
Phase 1 of #14
Not blocked by #15
Not after `#20`
Not after **`#24`**
Part of:
- #17
";
        let mixed_parsed = parse_body(REPO, mixed);
        assert_eq!(
            mixed_parsed.parents,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 10 },
                BodyRef::Issue { repo: REPO.into(), number: 18 },
                BodyRef::Issue { repo: REPO.into(), number: 13 },
                BodyRef::Issue { repo: REPO.into(), number: 14 },
                BodyRef::Issue { repo: REPO.into(), number: 17 },
            ],
            "{mixed_parsed:?}"
        );
        assert_eq!(
            mixed_parsed.blockers,
            vec![
                BodyRef::Issue { repo: REPO.into(), number: 11 },
                BodyRef::Issue { repo: REPO.into(), number: 12 },
                BodyRef::Issue { repo: REPO.into(), number: 16 },
                BodyRef::Issue { repo: REPO.into(), number: 21 },
                BodyRef::Issue { repo: REPO.into(), number: 22 },
                BodyRef::Symbolic("Setup auth".into()),
                BodyRef::Symbolic("Clean up after migration".into()),
                BodyRef::Symbolic("Add sync-links endpoint".into()),
            ],
            "{mixed_parsed:?}"
        );
        assert_eq!(mixed_parsed.declared_phase, Some(1));
    }
}
