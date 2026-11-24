use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{BOARD, Issue, LaunchArgs, gh_name, parse_github_issue_only};
use crate::clock::local_clock;
use crate::flow::{BLOCKED_BY, CODE_DEP, FILE_CONFLICT, PARENT, SOURCE_BODY, canonical_dismissed_edge_key};
use crate::flow_body;
use crate::github::{
    self, EnsureLabelOutcome, GraphQlError, IssueNode, IssueNodeRef, LabelNode, LabelSpec, LinkOutcome, RepoNode,
    UnlinkOutcome,
};
use crate::store::NewIssue;
use crate::wf::{NO_CALLER, Wf};

const MAX_TITLE: usize = 300;
const MAX_NOTE: usize = 2000;
const MAX_LABEL: usize = 50;
pub(crate) const MAX_GRAPHQL_ISSUE_NUMBER: u64 = i32::MAX as u64;

const COLOR_TRACK: &str = "1d76db";
const COLOR_PRIORITY_HIGH: &str = "b60205";
const COLOR_PRIORITY_MEDIUM: &str = "fbca04";
const COLOR_PRIORITY_LOW: &str = "0e8a16";
const COLOR_QUEUE: &str = "0e8a16";
const COLOR_DEFAULT: &str = "ededed";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueArgs {
    pub url: String,
    pub title: Option<String>,
    pub note: Option<String>,
    #[serde(default, rename = "blockedBy")]
    pub blocked_by: Option<Vec<String>>,
    pub parent: Option<String>,
    pub track: Option<String>,
    pub priority: Option<String>,
    pub sync: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIssuesArgs {
    pub repo: Option<String>,
    pub parent: Option<String>,
    pub track: Option<String>,
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub queue: Option<bool>,
    pub report: Option<bool>,
    pub sync: Option<bool>,
    pub issues: Vec<FileIssueItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIssueItem {
    pub id: Option<String>,
    pub repo: Option<String>,
    pub title: String,
    pub body: Option<String>,
    #[serde(default, rename = "blockedBy")]
    pub blocked_by: Vec<String>,
    pub parent: Option<String>,
    pub track: Option<String>,
    pub priority: Option<String>,
    #[serde(default)]
    pub touches: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub queue: Option<bool>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReport {
    #[serde(rename = "ref")]
    pub reference: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    pub(crate) op: Option<&'static str>,
    #[serde(skip)]
    pub(crate) permission: bool,
}

impl LinkReport {
    fn ok(reference: String, status: &'static str) -> Self {
        Self { reference, status, error: None, op: None, permission: false }
    }

    fn err(reference: String, op: &'static str, err: &GraphQlError) -> Self {
        Self { reference, status: "error", error: Some(err.message.clone()), op: Some(op), permission: err.permission }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLinkResult {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLinkError {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub op: &'static str,
    pub message: String,
    pub permission: bool,
    pub conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLinksResponse {
    pub ok: bool,
    pub total: usize,
    pub synced: usize,
    pub already_linked: usize,
    pub failed: usize,
    pub results: Vec<SyncLinkResult>,
    pub errors: Vec<SyncLinkError>,
    pub flow: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelReport {
    pub name: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WiringError {
    pub op: &'static str,
    pub target: String,
    pub message: String,
    pub permission: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WiringReport {
    pub ok: bool,
    pub blocked_by: Vec<LinkReport>,
    pub parent: Option<LinkReport>,
    pub labels: Vec<LabelReport>,
    pub errors: Vec<WiringError>,
}

#[derive(Clone, Debug)]
pub(crate) struct DesiredLabel {
    pub(crate) name: String,
    pub(crate) alt_name: Option<String>,
    pub(crate) color: &'static str,
    pub(crate) description: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct WiringPlan {
    pub(crate) blocked_by: Vec<(String, u64)>,
    pub(crate) parent: Option<(String, u64)>,
    pub(crate) replace_parent: bool,
    pub(crate) labels: Vec<DesiredLabel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowEdgeArgs {
    pub action: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(default)]
    pub replace_parent: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowEdgeResponse {
    pub ok: bool,
    pub action: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_parent: Option<String>,
    pub flow: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_error: Option<String>,
}

fn deserialize_patch_string<'de, D>(de: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(de).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowIssueUpdateArgs {
    pub issue: String,
    #[serde(default, deserialize_with = "deserialize_patch_string")]
    pub track: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_patch_string")]
    pub priority: Option<Option<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowIssueUpdateResponse {
    pub ok: bool,
    pub issue: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Option<String>>,
    pub labels: Vec<LabelReport>,
    pub errors: Vec<WiringError>,
    pub flow: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FlowMeta {
    pub(crate) parent: Option<String>,
    pub(crate) blocked_by: Vec<String>,
    pub(crate) track: Option<String>,
    pub(crate) priority: Option<&'static str>,
    pub(crate) touches: Vec<String>,
}

impl FlowMeta {
    fn is_empty(&self) -> bool {
        self.parent.is_none()
            && self.blocked_by.is_empty()
            && self.track.is_none()
            && self.priority.is_none()
            && self.touches.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BatchRef {
    Local(usize),
    External(String, u64),
}

struct PlannedIssue {
    input_index: usize,
    id: Option<String>,
    repo: String,
    title: String,
    body: Option<String>,
    blocked_by: Vec<BatchRef>,
    parent: Option<BatchRef>,
    track: Option<String>,
    priority: Option<&'static str>,
    touches: Vec<String>,
    labels: Vec<String>,
    queue: bool,
    note: Option<String>,
}

pub(crate) struct WireCaches {
    pub(crate) nodes: HashMap<(String, u64), IssueNode>,
    pub(crate) labels: HashMap<(String, String), LabelNode>,
}

impl WireCaches {
    pub(crate) fn new() -> Self {
        Self { nodes: HashMap::new(), labels: HashMap::new() }
    }
}

pub fn issue_key(i: &Issue) -> String {
    i.slug().to_ascii_lowercase()
}

pub(crate) fn node_cache_key(repo: &str, number: u64) -> (String, u64) {
    (repo.to_ascii_lowercase(), number)
}

pub(crate) fn ref_slug(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

fn format_flow_ref(item_repo: &str, target_repo: &str, number: u64) -> String {
    if item_repo.eq_ignore_ascii_case(target_repo) { format!("#{number}") } else { format!("{target_repo}#{number}") }
}

fn is_track_prefix_label(name: &str) -> bool {
    name.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("track:")) && !name[6..].trim().is_empty()
}

fn is_priority_prefix_label(name: &str) -> bool {
    let Some(head) = name.get(..9) else { return false };
    if !head.eq_ignore_ascii_case("priority:") {
        return false;
    }
    matches!(name[9..].trim().to_ascii_lowercase().as_str(), "high" | "medium" | "low")
}

fn priority_color(priority: &str) -> &'static str {
    match priority {
        "high" => COLOR_PRIORITY_HIGH,
        "medium" => COLOR_PRIORITY_MEDIUM,
        _ => COLOR_PRIORITY_LOW,
    }
}

fn text_for(method: &str, s: Option<String>, what: &str, max: usize) -> Result<Option<String>> {
    let Some(s) = s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else { return Ok(None) };
    if s.chars().count() > max {
        bail!("{method}: {what} is over {max} chars");
    }
    Ok(Some(s))
}

fn parse_repo_name(raw: &str, method: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    let Some((owner, name)) = trimmed.split_once('/').filter(|(o, n)| gh_name(o) && gh_name(n)) else {
        bail!("{method}: {field} {raw:?} is not an owner/name repository");
    };
    Ok(format!("{owner}/{name}"))
}

pub(crate) fn parse_github_ref(raw: &str, default_repo: &str, method: &str, field: &str) -> Result<(String, u64)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("{method}: {field} must not be empty");
    }
    if let Some(num_str) = trimmed
        .strip_prefix('#')
        .or_else(|| (!trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit())).then_some(trimmed))
    {
        let number = num_str
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0 && n <= MAX_GRAPHQL_ISSUE_NUMBER)
            .ok_or_else(|| anyhow!("{method}: {field} {raw:?} is not a valid GitHub issue number"))?;
        return Ok((default_repo.to_string(), number));
    }
    let issue = parse_github_issue_only(trimmed).ok_or_else(|| {
        anyhow!(
            "{method}: {field} {raw:?} is not a GitHub issue; use #N, <owner>/<repo>#<n>, or https://github.com/<owner>/<repo>/issues/<n>"
        )
    })?;
    let (owner, repo, number) = issue.github_parts().expect("GitHub issue has owner, repo and number");
    if number == 0 || number > MAX_GRAPHQL_ISSUE_NUMBER {
        bail!("{method}: {field} {raw:?} has an out-of-range GitHub issue number");
    }
    Ok((format!("{owner}/{repo}"), number))
}

pub(crate) fn normalize_priority(raw: Option<String>, method: &str) -> Result<Option<&'static str>> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let lower = trimmed.to_ascii_lowercase();
    let val = lower.strip_prefix("priority:").map_or(lower.as_str(), str::trim);
    match val {
        "high" => Ok(Some("high")),
        "medium" => Ok(Some("medium")),
        "low" => Ok(Some("low")),
        _ => bail!("{method}: priority must be \"high\", \"medium\", or \"low\", got {raw:?}"),
    }
}

pub(crate) fn normalize_track(raw: Option<String>, method: &str) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        bail!("{method}: track must be a single line");
    }
    let slug =
        if trimmed.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("track:")) { trimmed[6..].trim() } else { trimmed };
    if slug.is_empty() {
        bail!("{method}: track must not be empty");
    }
    let label = format!("track: {slug}");
    if label.chars().count() > MAX_LABEL {
        bail!("{method}: track label {label:?} is over {MAX_LABEL} chars");
    }
    Ok(Some(slug.to_string()))
}

fn normalize_label(raw: &str, method: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        bail!("{method}: label {raw:?} must be a single line");
    }
    if trimmed.chars().count() > MAX_LABEL {
        bail!("{method}: label {trimmed:?} is over {MAX_LABEL} chars");
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_touches(raw: Vec<String>, method: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in raw {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains('\n') || trimmed.contains('\r') || trimmed.contains(',') || trimmed.contains(';') {
            bail!("{method}: touches path {entry:?} must be a single path without newlines, commas, or semicolons");
        }
        if !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

pub(crate) fn desired_labels(
    track: Option<&str>,
    priority: Option<&str>,
    opt_in_label: Option<&str>,
    extra: &[String],
) -> Vec<DesiredLabel> {
    let mut out: Vec<DesiredLabel> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |item: DesiredLabel| {
        let key = item.name.to_ascii_lowercase();
        let alt_key = item.alt_name.as_deref().map(str::to_ascii_lowercase);
        if seen.contains(&key) || alt_key.as_ref().is_some_and(|a| seen.contains(a)) {
            return;
        }
        seen.insert(key);
        if let Some(a) = alt_key {
            seen.insert(a);
        }
        out.push(item);
    };
    if let Some(slug) = track {
        push(DesiredLabel {
            name: format!("track: {slug}"),
            alt_name: Some(format!("track:{slug}")),
            color: COLOR_TRACK,
            description: Some("Flow track"),
        });
    }
    if let Some(p) = priority {
        push(DesiredLabel {
            name: format!("priority: {p}"),
            alt_name: Some(format!("priority:{p}")),
            color: priority_color(p),
            description: Some("Flow priority"),
        });
    }
    if let Some(opt) = opt_in_label {
        push(DesiredLabel {
            name: opt.to_string(),
            alt_name: None,
            color: COLOR_QUEUE,
            description: Some("Flow queue opt-in"),
        });
    }
    for l in extra {
        push(DesiredLabel { name: l.clone(), alt_name: None, color: COLOR_DEFAULT, description: None });
    }
    out
}

pub(crate) fn render_flow_block(meta: &FlowMeta) -> String {
    if meta.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## Flow".to_string()];
    if let Some(parent) = &meta.parent {
        lines.push(format!("- Parent: {parent}"));
    }
    if !meta.blocked_by.is_empty() {
        lines.push(format!("- Blocked by: {}", meta.blocked_by.join(", ")));
    }
    if let Some(track) = &meta.track {
        lines.push(format!("- Track: {track}"));
    }
    if let Some(priority) = meta.priority {
        lines.push(format!("- Priority: {priority}"));
    }
    if !meta.touches.is_empty() {
        let formatted: Vec<String> =
            meta.touches.iter().map(|p| if p.contains('`') { p.clone() } else { format!("`{p}`") }).collect();
        lines.push(format!("- Touches: {}", formatted.join(", ")));
    }
    lines.join("\n")
}

fn remaining_body_outside_replaced_flow(raw: &str, will_write_flow: bool) -> String {
    if !will_write_flow {
        return raw.to_string();
    }
    let trimmed = raw.trim_end();
    if let Some((start, end)) = flow_body::inspect_flow_body(trimmed).section {
        let lines: Vec<&str> = trimmed.lines().collect();
        let prefix = lines[..start].join("\n");
        let suffix = lines[end..].join("\n");
        let mut out = String::new();
        if !prefix.trim().is_empty() {
            out.push_str(prefix.trim_end());
        }
        if !suffix.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(suffix.trim_start_matches(['\n', '\r']));
        }
        return out;
    }
    trimmed.to_string()
}

pub(crate) fn merge_flow_body(body: Option<&str>, meta: &FlowMeta) -> String {
    let raw = body.unwrap_or_default();
    if meta.is_empty() {
        return raw.to_string();
    }
    let block = render_flow_block(meta);
    if raw.trim().is_empty() {
        return block;
    }
    let trimmed = raw.trim_end();
    let layout = flow_body::inspect_flow_body(trimmed);
    if let Some((start, end)) = layout.section {
        let lines: Vec<&str> = trimmed.lines().collect();
        let prefix = lines[..start].join("\n");
        let suffix = lines[end..].join("\n");
        let prefix_trimmed = prefix.trim_end();
        let suffix_trimmed = suffix.trim_start_matches(['\n', '\r']);
        let mut out = String::new();
        if !prefix_trimmed.is_empty() {
            out.push_str(prefix_trimmed);
            out.push_str("\n\n");
        }
        out.push_str(&block);
        if !suffix_trimmed.is_empty() {
            out.push_str("\n\n");
            out.push_str(suffix_trimmed.trim_end());
        }
        return out;
    }
    let mut out = trimmed.to_string();
    if let Some((ch, len)) = layout.unclosed_fence {
        out.push('\n');
        for _ in 0..len {
            out.push(ch as char);
        }
    }
    if layout.unclosed_comment {
        out.push_str("\n-->");
    }
    out.push_str("\n\n");
    out.push_str(&block);
    out
}

fn seq_of(id: &str) -> Result<i64> {
    id.strip_prefix('n')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| anyhow!("no new issue {id:?}; new issue ids look like n3"))
}

pub fn issue_view(seq: i64, i: &NewIssue) -> Value {
    let mut v = json!(i);
    v["id"] = json!(format!("n{seq}"));
    v["atLocal"] = json!(local_clock(&i.at));
    v["dismissed"] = json!(i.dismissed_at.is_some());
    v["dismissedAtLocal"] = json!(i.dismissed_at.as_deref().map(local_clock));
    v
}

fn matches_github_ticket(ticket_issue: Option<&str>, ticket_label: &str, issue: &Issue) -> bool {
    match ticket_issue {
        Some(raw) => parse_github_issue_only(raw).is_some_and(|gh| gh.slug().eq_ignore_ascii_case(&issue.slug())),
        None => ticket_label.eq_ignore_ascii_case(&issue.label()),
    }
}

fn github_ticket_key(ticket_issue: Option<&str>, ticket_label: &str) -> Option<String> {
    match ticket_issue {
        Some(raw) => parse_github_issue_only(raw).map(|gh| gh.slug().to_ascii_lowercase()),
        None => Some(ticket_label.to_ascii_lowercase()),
    }
}

fn topo_sort_plan(items: &[PlannedIssue]) -> Result<Vec<usize>> {
    let n = items.len();
    // Creation order: every local parent and local blocker must be created strictly before the item
    // that references it. Use a min-heap over input index to keep ties in caller order.
    let mut in_degree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut dep_edges: Vec<(String, String)> = Vec::new();
    let mut node_names: Vec<String> = Vec::with_capacity(n);

    for (idx, item) in items.iter().enumerate() {
        let name = item.id.clone().unwrap_or_else(|| format!("#{}", idx + 1));
        node_names.push(name);
    }

    for (idx, item) in items.iter().enumerate() {
        let mut deps: Vec<usize> = Vec::new();
        if let Some(BatchRef::Local(p)) = item.parent {
            deps.push(p);
        }
        for b in &item.blocked_by {
            if let BatchRef::Local(b_idx) = b {
                deps.push(*b_idx);
            }
        }
        deps.sort_unstable();
        deps.dedup();
        for dep in deps {
            dep_edges.push((node_names[idx].clone(), node_names[dep].clone()));
            dependents[dep].push(idx);
            in_degree[idx] += 1;
        }
    }

    let mut ready: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            ready.push(Reverse(idx));
        }
    }
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(u)) = ready.pop() {
        order.push(u);
        for &v in &dependents[u] {
            in_degree[v] -= 1;
            if in_degree[v] == 0 {
                ready.push(Reverse(v));
            }
        }
    }
    if order.len() < n {
        let cycles = flow_body::find_cycles(&node_names, &dep_edges);
        let detail = if let Some(first) = cycles.first() {
            let mut path = first.cycle.clone();
            if let Some(head) = first.cycle.first().cloned() {
                path.push(head);
            }
            path.join(" -> ")
        } else {
            "cycle in local parent/blockedBy references".to_string()
        };
        bail!("wf.fileIssues: dependency cycle detected ({detail}); no issues were created");
    }
    Ok(order)
}

fn validate_batch(args: FileIssuesArgs) -> Result<(Vec<PlannedIssue>, bool, Option<bool>)> {
    if args.issues.is_empty() {
        bail!("wf.fileIssues: issues must not be empty");
    }
    let default_repo = args.repo.as_deref().map(|r| parse_repo_name(r, "wf.fileIssues", "repo")).transpose()?;
    let default_track = normalize_track(args.track, "wf.fileIssues")?;
    let default_priority = normalize_priority(args.priority, "wf.fileIssues")?;
    let default_parent = args.parent.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let default_queue = args.queue.unwrap_or(false);
    let sync = args.sync.unwrap_or(true);
    let mut default_labels = Vec::new();
    for raw in &args.labels {
        if let Some(l) = normalize_label(raw, "wf.fileIssues")? {
            default_labels.push(l);
        }
    }

    // Pass 1: validate local IDs and build case-insensitive lookup map.
    let mut id_map: HashMap<String, usize> = HashMap::new();
    let mut clean_ids: Vec<Option<String>> = Vec::with_capacity(args.issues.len());
    for (idx, item) in args.issues.iter().enumerate() {
        let id = match &item.id {
            Some(raw) => {
                let t = raw.trim();
                if t.is_empty() {
                    bail!("wf.fileIssues: issues[{idx}].id must not be empty");
                }
                let key = t.to_ascii_lowercase();
                if let Some(&prev) = id_map.get(&key) {
                    bail!("wf.fileIssues: duplicate local id {t:?} (issues[{prev}] and issues[{idx}])");
                }
                id_map.insert(key, idx);
                Some(t.to_string())
            }
            None => None,
        };
        clean_ids.push(id);
    }

    let resolve_batch_ref = |raw: &str, item_repo: &str, item_label: &str, field: &str| -> Result<BatchRef> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("wf.fileIssues: {item_label} has an empty {field} reference");
        }
        if let Some(&local_idx) = id_map.get(&trimmed.to_ascii_lowercase()) {
            return Ok(BatchRef::Local(local_idx));
        }
        match parse_github_ref(trimmed, item_repo, "wf.fileIssues", field) {
            Ok((repo, num)) => Ok(BatchRef::External(repo, num)),
            Err(_) => bail!(
                "wf.fileIssues: {item_label} references unknown {field} {raw:?} (not a local id in this batch and not a GitHub issue ref)"
            ),
        }
    };

    // Pass 2: validate every field and resolve local/external refs.
    let mut planned = Vec::with_capacity(args.issues.len());
    for (idx, item) in args.issues.into_iter().enumerate() {
        let id = clean_ids[idx].clone();
        let item_label = id.as_ref().map_or_else(|| format!("issues[{idx}]"), |s| format!("item {s:?}"));
        let repo = match item.repo.as_deref() {
            Some(r) => parse_repo_name(r, "wf.fileIssues", &format!("{item_label}.repo"))?,
            None => default_repo.clone().ok_or_else(|| {
                anyhow!("wf.fileIssues: {item_label} needs a repo (set repo on the item or top level)")
            })?,
        };
        let title = text_for("wf.fileIssues", Some(item.title), &format!("{item_label}.title"), MAX_TITLE)?
            .ok_or_else(|| anyhow!("wf.fileIssues: {item_label} must have a non-empty title"))?;
        let note = text_for("wf.fileIssues", item.note, &format!("{item_label}.note"), MAX_NOTE)?;
        let track = match item.track {
            Some(t) => normalize_track(Some(t), "wf.fileIssues")?,
            None => default_track.clone(),
        };
        let priority = match item.priority {
            Some(p) => normalize_priority(Some(p), "wf.fileIssues")?,
            None => default_priority,
        };
        let touches = normalize_touches(item.touches, "wf.fileIssues")?;
        let queue = item.queue.unwrap_or(default_queue);

        let mut labels = default_labels.clone();
        for raw in &item.labels {
            if let Some(l) = normalize_label(raw, "wf.fileIssues")?
                && !labels.iter().any(|existing| existing.eq_ignore_ascii_case(&l))
            {
                labels.push(l);
            }
        }

        let parent = match item.parent.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            Some(raw) => Some(resolve_batch_ref(&raw, &repo, &item_label, "parent")?),
            None => match &default_parent {
                Some(raw) => {
                    let resolved = resolve_batch_ref(raw, &repo, &item_label, "parent")?;
                    // Top-level parent pointing to this item's own local id means this item IS the parent.
                    if resolved == BatchRef::Local(idx) { None } else { Some(resolved) }
                }
                None => None,
            },
        };

        let mut blocked_by: Vec<BatchRef> = Vec::new();
        for raw in &item.blocked_by {
            let resolved = resolve_batch_ref(raw, &repo, &item_label, "blockedBy")?;
            let dup = blocked_by.iter().any(|existing| match (existing, &resolved) {
                (BatchRef::Local(a), BatchRef::Local(b)) => a == b,
                (BatchRef::External(r1, n1), BatchRef::External(r2, n2)) => n1 == n2 && r1.eq_ignore_ascii_case(r2),
                _ => false,
            });
            if !dup {
                blocked_by.push(resolved);
            }
        }

        planned.push(PlannedIssue {
            input_index: idx,
            id,
            repo,
            title,
            body: item.body,
            blocked_by,
            parent,
            track,
            priority,
            touches,
            labels,
            queue,
            note,
        });
    }

    Ok((planned, sync, args.report))
}

impl Wf {
    fn ticket_for(&self, issue: &Issue) -> Result<Option<Value>> {
        if let Some(l) = self
            .store
            .children(BOARD, false)?
            .into_iter()
            .find(|l| matches_github_ticket(l.identity.issue.as_deref(), &l.label, issue))
        {
            let link = l.link.clone().or_else(|| self.live_of(&l.child)?["link"].as_str().map(String::from));
            return Ok(Some(json!({"label": l.label, "thread": l.child, "link": link, "done": l.closed})));
        }
        Ok(self
            .store
            .pending_all()?
            .into_iter()
            .find(|p| p.parent == BOARD && matches_github_ticket(p.identity.issue.as_deref(), &p.label, issue))
            .map(|p| json!({"label": p.label, "thread": p.thread, "link": p.link, "done": false})))
    }

    fn ticket_keys(&self) -> Result<HashSet<String>> {
        let mut out: HashSet<String> = self
            .store
            .children(BOARD, false)?
            .into_iter()
            .filter_map(|l| github_ticket_key(l.identity.issue.as_deref(), &l.label))
            .collect();
        out.extend(
            self.store
                .pending_all()?
                .into_iter()
                .filter(|p| p.parent == BOARD)
                .filter_map(|p| github_ticket_key(p.identity.issue.as_deref(), &p.label)),
        );
        Ok(out)
    }

    fn already_known(&self, issue: &Issue) -> Result<Option<Value>> {
        let url = issue.reference();
        if let Some(t) = self.ticket_for(issue)? {
            let message = format!("{url} already has a ticket on the board ({}); nothing added", t["label"]);
            return Ok(Some(json!({"added": false, "reason": "ticket", "ticket": t, "message": message})));
        }
        if let Some((seq, i)) = self.store.new_issue_by_key(&issue_key(issue))? {
            let message = match i.dismissed_at {
                Some(_) => format!("{url} was already reported as n{seq} and is dismissed; nothing added"),
                None => format!("{url} is already on the board's new issues as n{seq}; nothing added"),
            };
            return Ok(Some(
                json!({"added": false, "reason": "reported", "issue": issue_view(seq, &i), "message": message}),
            ));
        }
        Ok(None)
    }

    async fn thread_meta(&self, thread: &str) -> (Option<String>, Option<String>) {
        let th =
            self.t3.call("t3_thread_read", json!({"threadId": thread, "limit": 1, "maxCharsPerItem": 1})).await.ok();
        let field = |k: &str| th.as_ref().and_then(|r| r["thread"][k].as_str()).map(String::from);
        (field("link"), field("title"))
    }

    async fn backfill_thread(&self, key: &str) -> Result<()> {
        let Some((_, i)) = self.store.new_issue_by_key(key)? else { return Ok(()) };
        if i.thread_link.is_some() && i.thread_title.is_some() {
            return Ok(());
        }
        let (link, title) = self.thread_meta(&i.thread).await;
        let _g = self.lock.lock().await;
        let Some((seq, mut i)) = self.store.new_issue_by_key(key)? else { return Ok(()) };
        let before = (i.thread_link.clone(), i.thread_title.clone());
        i.thread_link = i.thread_link.or(link);
        i.thread_title = i.thread_title.or(title);
        if (i.thread_link.clone(), i.thread_title.clone()) != before {
            self.store.new_issue_update(seq, &i)?;
            self.changed();
        }
        Ok(())
    }

    async fn record_board_issue(
        &self,
        me: &str,
        issue: &Issue,
        title: Option<String>,
        note: Option<String>,
        cached_thread_meta: Option<&(Option<String>, Option<String>)>,
    ) -> Result<Value> {
        let (owner, repo, number) = issue.github_parts().expect("GitHub issue has owner, repo and number");
        self.backfill_thread(&issue_key(issue)).await?;
        if let Some(v) = self.already_known(issue)? {
            return Ok(v);
        }
        let (thread_link, thread_title) = match cached_thread_meta {
            Some(meta) => meta.clone(),
            None => self.thread_meta(me).await,
        };
        let entry = NewIssue {
            url: issue.reference().to_string(),
            repo: format!("{owner}/{repo}"),
            number,
            label: issue.label(),
            title,
            note,
            thread: me.to_string(),
            thread_link,
            thread_title,
            at: self.now(),
            dismissed_at: None,
            restored_at: None,
        };
        let _g = self.lock.lock().await;
        if let Some(v) = self.already_known(issue)? {
            return Ok(v);
        }
        let seq = self
            .store
            .new_issue_add(&issue_key(issue), &entry)?
            .ok_or_else(|| anyhow!("wf.issue: {} was reported at the same moment; nothing added", entry.url))?;
        let message = format!("{} is on the board's new issues as n{seq}", entry.url);
        Ok(json!({"added": true, "issue": issue_view(seq, &entry), "message": message}))
    }

    pub(crate) async fn resolve_issue_cached(
        &self,
        caches: &mut WireCaches,
        repo: &str,
        number: u64,
    ) -> std::result::Result<IssueNode, GraphQlError> {
        let key = node_cache_key(repo, number);
        if let Some(found) = caches.nodes.get(&key) {
            return Ok(found.clone());
        }
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| GraphQlError::new(format!("{repo:?} is not an owner/name repository"), false))?;
        let node = github::read_issue_node(&*self.github, owner, name, number).await?;
        caches.nodes.insert(node_cache_key(&node.repo, node.number), node.clone());
        caches.nodes.insert(key, node.clone());
        Ok(node)
    }

    pub(crate) async fn wire_issue_node(
        &self,
        issue_node: &IssueNode,
        plan: &WiringPlan,
        caches: &mut WireCaches,
    ) -> WiringReport {
        let mut errors = Vec::new();
        let mut parent_report = None;
        let mut blocked_reports = Vec::with_capacity(plan.blocked_by.len());
        let mut label_reports = Vec::with_capacity(plan.labels.len());

        // 1. Parent (sub-issue relationship: parent.addSubIssue(child))
        if let Some((p_repo, p_num)) = &plan.parent {
            let target = ref_slug(p_repo, *p_num);
            let already =
                issue_node.parent.as_ref().is_some_and(|p| p.number == *p_num && p.repo.eq_ignore_ascii_case(p_repo));
            if already {
                let canon = issue_node.parent.as_ref().map_or_else(|| target.clone(), |p| ref_slug(&p.repo, p.number));
                parent_report = Some(LinkReport::ok(canon, "alreadyLinked"));
            } else {
                match self.resolve_issue_cached(caches, p_repo, *p_num).await {
                    Err(e) => {
                        errors.push(WiringError {
                            op: "readIssue",
                            target: target.clone(),
                            message: e.message.clone(),
                            permission: e.permission,
                        });
                        parent_report = Some(LinkReport::err(target, "readIssue", &e));
                    }
                    Ok(parent_node) => {
                        let canon = ref_slug(&parent_node.repo, parent_node.number);
                        match github::add_sub_issue(&*self.github, &parent_node.id, &issue_node.id, plan.replace_parent)
                            .await
                        {
                            Ok(LinkOutcome::Linked) => {
                                parent_report = Some(LinkReport::ok(canon, "linked"));
                            }
                            Ok(LinkOutcome::AlreadyLinked) => {
                                parent_report = Some(LinkReport::ok(canon, "alreadyLinked"));
                            }
                            Err(e) => {
                                errors.push(WiringError {
                                    op: "addSubIssue",
                                    target: canon.clone(),
                                    message: e.message.clone(),
                                    permission: e.permission,
                                });
                                parent_report = Some(LinkReport::err(canon, "addSubIssue", &e));
                            }
                        }
                    }
                }
            }
        }

        // 2. Blockers (addBlockedBy)
        for (b_repo, b_num) in &plan.blocked_by {
            let target = ref_slug(b_repo, *b_num);
            if let Some(existing) =
                issue_node.blocked_by.iter().find(|b| b.number == *b_num && b.repo.eq_ignore_ascii_case(b_repo))
            {
                blocked_reports.push(LinkReport::ok(ref_slug(&existing.repo, existing.number), "alreadyLinked"));
                continue;
            }
            match self.resolve_issue_cached(caches, b_repo, *b_num).await {
                Err(e) => {
                    errors.push(WiringError {
                        op: "readIssue",
                        target: target.clone(),
                        message: e.message.clone(),
                        permission: e.permission,
                    });
                    blocked_reports.push(LinkReport::err(target, "readIssue", &e));
                }
                Ok(blocker_node) => {
                    let canon = ref_slug(&blocker_node.repo, blocker_node.number);
                    match github::add_blocked_by(&*self.github, &issue_node.id, &blocker_node.id).await {
                        Ok(LinkOutcome::Linked) => {
                            blocked_reports.push(LinkReport::ok(canon, "linked"));
                        }
                        Ok(LinkOutcome::AlreadyLinked) => {
                            blocked_reports.push(LinkReport::ok(canon, "alreadyLinked"));
                        }
                        Err(e) => {
                            errors.push(WiringError {
                                op: "addBlockedBy",
                                target: canon.clone(),
                                message: e.message.clone(),
                                permission: e.permission,
                            });
                            blocked_reports.push(LinkReport::err(canon, "addBlockedBy", &e));
                        }
                    }
                }
            }
        }

        // 3. Labels (ensure_label + batched addLabelsToLabelable)
        if !plan.labels.is_empty() {
            let (owner, name) = issue_node.repo.split_once('/').unwrap_or(("", issue_node.repo.as_str()));
            let repo_node = RepoNode {
                id: issue_node.repo_id.clone(),
                owner: owner.to_string(),
                name: name.to_string(),
                name_with_owner: issue_node.repo.clone(),
            };
            let mut to_add: Vec<(LabelNode, bool)> = Vec::new();
            let repo_key = issue_node.repo.to_ascii_lowercase();
            for desired in &plan.labels {
                let already = issue_node.labels.iter().find(|l| {
                    l.name.eq_ignore_ascii_case(&desired.name)
                        || desired.alt_name.as_deref().is_some_and(|alt| l.name.eq_ignore_ascii_case(alt))
                });
                if let Some(existing) = already {
                    label_reports.push(LabelReport {
                        name: existing.name.clone(),
                        status: "alreadyApplied",
                        error: None,
                    });
                    continue;
                }
                let cache_key = (repo_key.clone(), desired.name.to_ascii_lowercase());
                if let Some(cached) = caches.labels.get(&cache_key) {
                    to_add.push((cached.clone(), false));
                    continue;
                }
                let spec = LabelSpec {
                    name: &desired.name,
                    alt_name: desired.alt_name.as_deref(),
                    color: desired.color,
                    description: desired.description,
                };
                match github::ensure_label(&*self.github, &repo_node, &spec).await {
                    Ok(EnsureLabelOutcome::Existing(lbl)) => {
                        caches.labels.insert(cache_key, lbl.clone());
                        to_add.push((lbl, false));
                    }
                    Ok(EnsureLabelOutcome::Created(lbl)) => {
                        caches.labels.insert(cache_key, lbl.clone());
                        to_add.push((lbl, true));
                    }
                    Err(e) => {
                        errors.push(WiringError {
                            op: "ensureLabel",
                            target: desired.name.clone(),
                            message: e.message.clone(),
                            permission: e.permission,
                        });
                        label_reports.push(LabelReport {
                            name: desired.name.clone(),
                            status: "error",
                            error: Some(e.message),
                        });
                    }
                }
            }
            if !to_add.is_empty() {
                let ids: Vec<String> = to_add.iter().map(|(l, _)| l.id.clone()).collect();
                match github::add_labels(&*self.github, &issue_node.id, &ids).await {
                    Ok(()) => {
                        for (lbl, created) in to_add {
                            label_reports.push(LabelReport {
                                name: lbl.name,
                                status: if created { "createdAndAdded" } else { "added" },
                                error: None,
                            });
                        }
                    }
                    Err(e) => {
                        errors.push(WiringError {
                            op: "addLabels",
                            target: ref_slug(&issue_node.repo, issue_node.number),
                            message: e.message.clone(),
                            permission: e.permission,
                        });
                        for (lbl, _) in to_add {
                            label_reports.push(LabelReport {
                                name: lbl.name,
                                status: "error",
                                error: Some(e.message.clone()),
                            });
                        }
                    }
                }
            }
        }

        WiringReport {
            ok: errors.is_empty(),
            blocked_by: blocked_reports,
            parent: parent_report,
            labels: label_reports,
            errors,
        }
    }

    async fn wire_existing_issue(&self, repo: &str, number: u64, plan: &WiringPlan) -> WiringReport {
        let mut caches = WireCaches::new();
        match self.resolve_issue_cached(&mut caches, repo, number).await {
            Ok(issue_node) => self.wire_issue_node(&issue_node, plan, &mut caches).await,
            Err(e) => {
                let target = ref_slug(repo, number);
                let blocked_by =
                    plan.blocked_by.iter().map(|(r, n)| LinkReport::err(ref_slug(r, *n), "readIssue", &e)).collect();
                let parent = plan.parent.as_ref().map(|(r, n)| LinkReport::err(ref_slug(r, *n), "readIssue", &e));
                let labels = plan
                    .labels
                    .iter()
                    .map(|l| LabelReport { name: l.name.clone(), status: "error", error: Some(e.message.clone()) })
                    .collect();
                WiringReport {
                    ok: false,
                    blocked_by,
                    parent,
                    labels,
                    errors: vec![WiringError { op: "readIssue", target, message: e.message, permission: e.permission }],
                }
            }
        }
    }

    pub async fn sync_flow_links(&self) -> Result<SyncLinksResponse> {
        let initial = self.flow_view(true).await?;
        let body_edges: Vec<(String, String, &'static str)> = initial
            .graph
            .as_ref()
            .map(|g| {
                g.edges
                    .iter()
                    .filter(|e| e.source == SOURCE_BODY && (e.kind == BLOCKED_BY || e.kind == PARENT))
                    .map(|e| (e.from.clone(), e.to.clone(), e.kind))
                    .collect()
            })
            .unwrap_or_default();

        if body_edges.is_empty() {
            return Ok(SyncLinksResponse {
                ok: true,
                total: 0,
                synced: 0,
                already_linked: 0,
                failed: 0,
                results: Vec::new(),
                errors: Vec::new(),
                flow: Some(serde_json::to_value(initial)?),
                refresh_error: None,
            });
        }

        struct FromGroup {
            from_id: String,
            repo: String,
            number: u64,
            parents: Vec<(usize, String, u64, String)>,
            blockers: Vec<(usize, String, u64, String)>,
        }

        let mut slots: Vec<Option<SyncLinkResult>> = vec![None; body_edges.len()];
        let mut error_slots: Vec<Option<SyncLinkError>> = vec![None; body_edges.len()];
        let mut groups: Vec<FromGroup> = Vec::new();
        let mut group_idx: HashMap<(String, u64), usize> = HashMap::new();

        let parse_edge_endpoint = |raw: &str| -> Option<(String, u64)> {
            let issue = parse_github_issue_only(raw)?;
            let (owner, repo, number) = issue.github_parts()?;
            (number > 0 && number <= MAX_GRAPHQL_ISSUE_NUMBER).then(|| (format!("{owner}/{repo}"), number))
        };

        let record_err = |slots: &mut [Option<SyncLinkResult>],
                          error_slots: &mut [Option<SyncLinkError>],
                          idx: usize,
                          from: String,
                          to: String,
                          kind: &'static str,
                          op: &'static str,
                          message: String,
                          permission: bool,
                          conflict: bool,
                          existing_parent: Option<String>| {
            slots[idx] = Some(SyncLinkResult {
                from: from.clone(),
                to: to.clone(),
                kind,
                status: "error",
                error: Some(message.clone()),
                permission,
                conflict,
                existing_parent: existing_parent.clone(),
            });
            error_slots[idx] =
                Some(SyncLinkError { from, to, kind, op, message, permission, conflict, existing_parent });
        };

        for (idx, (from_id, to_id, kind)) in body_edges.iter().enumerate() {
            let default_op = if *kind == PARENT { "addSubIssue" } else { "addBlockedBy" };
            let Some((from_repo, from_num)) = parse_edge_endpoint(from_id) else {
                record_err(
                    &mut slots,
                    &mut error_slots,
                    idx,
                    from_id.clone(),
                    to_id.clone(),
                    kind,
                    default_op,
                    format!("invalid source issue {from_id:?}"),
                    false,
                    false,
                    None,
                );
                continue;
            };
            let Some((to_repo, to_num)) = parse_edge_endpoint(to_id) else {
                record_err(
                    &mut slots,
                    &mut error_slots,
                    idx,
                    from_id.clone(),
                    to_id.clone(),
                    kind,
                    default_op,
                    format!("invalid target issue {to_id:?}"),
                    false,
                    false,
                    None,
                );
                continue;
            };
            if from_num == to_num && from_repo.eq_ignore_ascii_case(&to_repo) {
                let msg = if *kind == PARENT {
                    "issue cannot be its own parent".to_string()
                } else {
                    "issue cannot be blocked by itself".to_string()
                };
                record_err(
                    &mut slots,
                    &mut error_slots,
                    idx,
                    from_id.clone(),
                    to_id.clone(),
                    kind,
                    default_op,
                    msg,
                    false,
                    false,
                    None,
                );
                continue;
            }
            let key = node_cache_key(&from_repo, from_num);
            let gi = match group_idx.get(&key) {
                Some(&gi) => gi,
                None => {
                    let gi = groups.len();
                    group_idx.insert(key, gi);
                    groups.push(FromGroup {
                        from_id: from_id.clone(),
                        repo: from_repo,
                        number: from_num,
                        parents: Vec::new(),
                        blockers: Vec::new(),
                    });
                    gi
                }
            };
            if *kind == PARENT {
                groups[gi].parents.push((idx, to_repo, to_num, to_id.clone()));
            } else {
                groups[gi].blockers.push((idx, to_repo, to_num, to_id.clone()));
            }
        }

        let primary_parent_of: HashMap<String, String> = initial
            .graph
            .as_ref()
            .map(|g| {
                g.nodes
                    .iter()
                    .filter_map(|n| n.parent.as_ref().map(|p| (n.id.to_ascii_lowercase(), p.clone())))
                    .collect()
            })
            .unwrap_or_default();

        let mut caches = WireCaches::new();
        for mut group in groups {
            let graph_primary = primary_parent_of.get(&group.from_id.to_ascii_lowercase()).cloned();
            let has_primary_write = match &graph_primary {
                Some(primary) => {
                    if let Some(pos) = group.parents.iter().position(|(_, r, n, to_id)| {
                        to_id.eq_ignore_ascii_case(primary) || ref_slug(r, *n).eq_ignore_ascii_case(primary)
                    }) {
                        if pos > 0 {
                            let primary_edge = group.parents.remove(pos);
                            group.parents.insert(0, primary_edge);
                        }
                        true
                    } else {
                        // The graph's primary parent is already native on GitHub (or not among the body edges);
                        // never promote a conflicting body candidate into a write.
                        false
                    }
                }
                None => !group.parents.is_empty(),
            };

            let mut from_node = match self.resolve_issue_cached(&mut caches, &group.repo, group.number).await {
                Ok(n) => n,
                Err(e) => {
                    for (idx, _, _, to_id) in group.parents {
                        record_err(
                            &mut slots,
                            &mut error_slots,
                            idx,
                            group.from_id.clone(),
                            to_id,
                            PARENT,
                            "readIssue",
                            e.message.clone(),
                            e.permission,
                            false,
                            None,
                        );
                    }
                    for (idx, _, _, to_id) in group.blockers {
                        record_err(
                            &mut slots,
                            &mut error_slots,
                            idx,
                            group.from_id.clone(),
                            to_id,
                            BLOCKED_BY,
                            "readIssue",
                            e.message.clone(),
                            e.permission,
                            false,
                            None,
                        );
                    }
                    continue;
                }
            };

            let from_canon = ref_slug(&from_node.repo, from_node.number);
            let (primary_parents, secondary_parents) =
                if has_primary_write { group.parents.split_at(1) } else { (&[][..], &group.parents[..]) };
            let plan = WiringPlan {
                blocked_by: group.blockers.iter().map(|(_, r, n, _)| (r.clone(), *n)).collect(),
                parent: primary_parents.first().map(|(_, r, n, _)| (r.clone(), *n)),
                replace_parent: false,
                labels: Vec::new(),
            };
            let report = self.wire_issue_node(&from_node, &plan, &mut caches).await;

            let mut primary_target = graph_primary;
            if let (Some((idx, p_repo, p_num, _)), Some(lr)) = (primary_parents.first(), report.parent) {
                primary_target = Some(lr.reference.clone());
                if lr.status == "error" {
                    let msg = lr.error.unwrap_or_else(|| "GitHub returned an error".to_string());
                    let existing = from_node.parent.as_ref().map(|p| ref_slug(&p.repo, p.number));
                    let conflict = !lr.permission && (github::is_parent_conflict_message(&msg) || existing.is_some());
                    record_err(
                        &mut slots,
                        &mut error_slots,
                        *idx,
                        from_canon.clone(),
                        lr.reference,
                        PARENT,
                        lr.op.unwrap_or("addSubIssue"),
                        msg,
                        lr.permission,
                        conflict,
                        conflict.then_some(existing).flatten(),
                    );
                } else {
                    if lr.status == "linked"
                        && let Some(p_node) = caches.nodes.get(&node_cache_key(p_repo, *p_num)).cloned()
                    {
                        from_node.parent =
                            Some(IssueNodeRef { id: p_node.id, repo: p_node.repo, number: p_node.number });
                    }
                    slots[*idx] = Some(SyncLinkResult {
                        from: from_canon.clone(),
                        to: lr.reference,
                        kind: PARENT,
                        status: lr.status,
                        error: None,
                        permission: false,
                        conflict: false,
                        existing_parent: None,
                    });
                }
            }

            // Secondary parent candidates conflict with the node's existing or primary parent and are never written,
            // unless GitHub already acquired that candidate as the native parent before we read the issue.
            for (idx, p_repo, p_num, _) in secondary_parents {
                if let Some(p) =
                    from_node.parent.as_ref().filter(|p| p.number == *p_num && p.repo.eq_ignore_ascii_case(p_repo))
                {
                    slots[*idx] = Some(SyncLinkResult {
                        from: from_canon.clone(),
                        to: ref_slug(&p.repo, p.number),
                        kind: PARENT,
                        status: "alreadyLinked",
                        error: None,
                        permission: false,
                        conflict: false,
                        existing_parent: None,
                    });
                    continue;
                }
                let target = ref_slug(p_repo, *p_num);
                let existing = from_node.parent.as_ref().map(|p| ref_slug(&p.repo, p.number));
                let msg = match (&existing, &primary_target) {
                    (Some(ex), _) => format!("issue already has parent {ex}; skipped conflicting parent {target}"),
                    (None, Some(pri)) => {
                        format!("skipped conflicting parent {target}; primary parent {pri} takes precedence")
                    }
                    (None, None) => format!("skipped conflicting parent {target}"),
                };
                record_err(
                    &mut slots,
                    &mut error_slots,
                    *idx,
                    from_canon.clone(),
                    target,
                    PARENT,
                    "addSubIssue",
                    msg,
                    false,
                    true,
                    existing,
                );
            }

            for ((idx, b_repo, b_num, _), lr) in group.blockers.iter().zip(report.blocked_by) {
                if lr.status == "error" {
                    let msg = lr.error.unwrap_or_else(|| "GitHub returned an error".to_string());
                    record_err(
                        &mut slots,
                        &mut error_slots,
                        *idx,
                        from_canon.clone(),
                        lr.reference,
                        BLOCKED_BY,
                        lr.op.unwrap_or("addBlockedBy"),
                        msg,
                        lr.permission,
                        false,
                        None,
                    );
                } else {
                    if lr.status == "linked"
                        && let Some(b_node) = caches.nodes.get(&node_cache_key(b_repo, *b_num)).cloned()
                    {
                        from_node.blocked_by.push(IssueNodeRef {
                            id: b_node.id,
                            repo: b_node.repo,
                            number: b_node.number,
                        });
                    }
                    slots[*idx] = Some(SyncLinkResult {
                        from: from_canon.clone(),
                        to: lr.reference,
                        kind: BLOCKED_BY,
                        status: lr.status,
                        error: None,
                        permission: false,
                        conflict: false,
                        existing_parent: None,
                    });
                }
            }

            caches.nodes.insert(node_cache_key(&from_node.repo, from_node.number), from_node.clone());
            caches.nodes.insert(node_cache_key(&group.repo, group.number), from_node);
        }

        let results: Vec<SyncLinkResult> = slots.into_iter().flatten().collect();
        let errors: Vec<SyncLinkError> = error_slots.into_iter().flatten().collect();
        let synced = results.iter().filter(|r| r.status == "linked").count();
        let already_linked = results.iter().filter(|r| r.status == "alreadyLinked").count();
        let failed = results.iter().filter(|r| r.status == "error").count();
        let (flow, refresh_error) = match self.flow_view(true).await {
            Ok(fresh) => (Some(serde_json::to_value(fresh)?), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };

        Ok(SyncLinksResponse {
            ok: errors.is_empty(),
            total: results.len(),
            synced,
            already_linked,
            failed,
            results,
            errors,
            flow,
            refresh_error,
        })
    }

    pub(crate) fn clear_dismissed_edge(
        &self,
        kind: &str,
        from_repo: &str,
        from_num: u64,
        to_repo: &str,
        to_num: u64,
    ) -> Result<bool> {
        let from_canon = self.canonical_github_repo(from_repo);
        let to_canon = self.canonical_github_repo(to_repo);
        let (cf, ct) = canonical_dismissed_edge_key(kind, &from_canon, from_num, &to_canon, to_num);
        let mut removed = self.store.flow_dismissed_edge_remove(&cf, kind, &ct)?;
        let (rf, rt) = canonical_dismissed_edge_key(kind, from_repo, from_num, to_repo, to_num);
        if (rf != cf || rt != ct) && self.store.flow_dismissed_edge_remove(&rf, kind, &rt)? {
            removed = true;
        }
        Ok(removed)
    }

    pub(crate) async fn apply_track_and_priority(
        &self,
        issue_node: &IssueNode,
        track_edit: Option<Option<&str>>,
        priority_edit: Option<Option<&str>>,
        caches: &mut WireCaches,
    ) -> (Vec<LabelReport>, Vec<WiringError>) {
        let mut label_reports = Vec::new();
        let mut errors = Vec::new();

        // Attach replacements before removing old labels so an add/ensure failure never drops the prior value.
        let desired = desired_labels(track_edit.flatten(), priority_edit.flatten(), None, &[]);
        let mut track_add_ok = true;
        let mut prio_add_ok = true;
        if !desired.is_empty() {
            let plan = WiringPlan { blocked_by: Vec::new(), parent: None, replace_parent: false, labels: desired };
            let report = self.wire_issue_node(issue_node, &plan, caches).await;
            for lr in &report.labels {
                if lr.status == "error" {
                    if is_track_prefix_label(&lr.name) {
                        track_add_ok = false;
                    }
                    if is_priority_prefix_label(&lr.name) {
                        prio_add_ok = false;
                    }
                }
            }
            label_reports.extend(report.labels);
            errors.extend(report.errors);
        }

        let mut to_remove: Vec<&LabelNode> = Vec::new();
        for lbl in &issue_node.labels {
            if let Some(want_track) = track_edit
                && track_add_ok
                && is_track_prefix_label(&lbl.name)
            {
                let cur = lbl.name[6..].trim();
                if want_track.is_none_or(|w| !cur.eq_ignore_ascii_case(w)) {
                    to_remove.push(lbl);
                    continue;
                }
            }
            if let Some(want_prio) = priority_edit
                && prio_add_ok
                && is_priority_prefix_label(&lbl.name)
            {
                let cur = lbl.name[9..].trim();
                if want_prio.is_none_or(|w| !cur.eq_ignore_ascii_case(w)) {
                    to_remove.push(lbl);
                }
            }
        }
        if !to_remove.is_empty() {
            let ids: Vec<String> = to_remove.iter().map(|l| l.id.clone()).collect();
            match github::remove_labels(&*self.github, &issue_node.id, &ids).await {
                Ok(()) => {
                    for lbl in to_remove {
                        label_reports.push(LabelReport { name: lbl.name.clone(), status: "removed", error: None });
                    }
                }
                Err(e) => {
                    errors.push(WiringError {
                        op: "removeLabels",
                        target: ref_slug(&issue_node.repo, issue_node.number),
                        message: e.message.clone(),
                        permission: e.permission,
                    });
                    for lbl in to_remove {
                        label_reports.push(LabelReport {
                            name: lbl.name.clone(),
                            status: "error",
                            error: Some(e.message.clone()),
                        });
                    }
                }
            }
        }
        (label_reports, errors)
    }

    pub async fn edit_flow_edge(&self, args: FlowEdgeArgs) -> Result<FlowEdgeResponse> {
        let action = args.action.trim().to_string();
        let kind = args.kind.trim().to_string();
        if !matches!(action.as_str(), "add" | "remove" | "confirm" | "dismiss" | "restore") {
            bail!(
                "edit_flow_edge: unknown action {action:?}; use \"add\", \"remove\", \"confirm\", \"dismiss\", or \"restore\""
            );
        }
        if !matches!(kind.as_str(), BLOCKED_BY | PARENT | FILE_CONFLICT | CODE_DEP) {
            bail!(
                "edit_flow_edge: unknown kind {kind:?}; use \"blockedBy\", \"parent\", \"fileConflict\", or \"codeDep\""
            );
        }
        if matches!(action.as_str(), "add" | "remove") && !matches!(kind.as_str(), BLOCKED_BY | PARENT) {
            bail!("edit_flow_edge: action {action:?} only applies to \"blockedBy\" or \"parent\"");
        }

        let from_issue = parse_github_issue_only(args.from.trim()).ok_or_else(|| {
            anyhow!(
                "edit_flow_edge: from {:?} is not a GitHub issue; use <owner>/<repo>#<n> or a GitHub issue URL",
                args.from
            )
        })?;
        let (f_owner, f_repo, from_num) = from_issue.github_parts().expect("GitHub issue has parts");
        if from_num == 0 || from_num > MAX_GRAPHQL_ISSUE_NUMBER {
            bail!("edit_flow_edge: from {:?} has an out-of-range issue number", args.from);
        }
        let from_repo = format!("{f_owner}/{f_repo}");
        let (to_repo, to_num) = parse_github_ref(&args.to, &from_repo, "edit_flow_edge", "to")?;
        if from_num == to_num && from_repo.eq_ignore_ascii_case(&to_repo) {
            bail!("edit_flow_edge: issue cannot link to itself");
        }

        let mut from_canon = ref_slug(&from_repo, from_num);
        let mut to_canon = ref_slug(&to_repo, to_num);
        let mut ok = true;
        let status: &'static str;
        let mut error_msg: Option<String> = None;
        let mut permission = false;
        let mut conflict = false;
        let mut existing_parent: Option<String> = None;

        match action.as_str() {
            "dismiss" => {
                let fc = self.canonical_github_repo(&from_repo);
                let tc = self.canonical_github_repo(&to_repo);
                let (fk, tk) = canonical_dismissed_edge_key(&kind, &fc, from_num, &tc, to_num);
                let inserted = self.store.flow_dismissed_edge_add(&fk, &kind, &tk, &self.now())?;
                from_canon = ref_slug(&fc, from_num);
                to_canon = ref_slug(&tc, to_num);
                status = if inserted { "dismissed" } else { "alreadyDismissed" };
            }
            "restore" => {
                let fc = self.canonical_github_repo(&from_repo);
                let tc = self.canonical_github_repo(&to_repo);
                let removed = self.clear_dismissed_edge(&kind, &from_repo, from_num, &to_repo, to_num)?;
                from_canon = ref_slug(&fc, from_num);
                to_canon = ref_slug(&tc, to_num);
                status = if removed { "restored" } else { "notDismissed" };
            }
            "add" | "confirm" => {
                let mut caches = WireCaches::new();
                match self.resolve_issue_cached(&mut caches, &from_repo, from_num).await {
                    Err(e) => {
                        ok = false;
                        status = "error";
                        error_msg = Some(e.message);
                        permission = e.permission;
                    }
                    Ok(from_node) => {
                        from_canon = ref_slug(&from_node.repo, from_node.number);
                        if kind == PARENT {
                            let plan = WiringPlan {
                                blocked_by: Vec::new(),
                                parent: Some((to_repo.clone(), to_num)),
                                replace_parent: args.replace_parent,
                                labels: Vec::new(),
                            };
                            let report = self.wire_issue_node(&from_node, &plan, &mut caches).await;
                            let lr = report.parent.expect("parent plan produces parent report");
                            to_canon = lr.reference;
                            if lr.status == "error" {
                                ok = false;
                                status = "error";
                                let msg = lr.error.unwrap_or_else(|| "GitHub returned an error".to_string());
                                let existing = from_node.parent.as_ref().map(|p| ref_slug(&p.repo, p.number));
                                conflict =
                                    !lr.permission && (github::is_parent_conflict_message(&msg) || existing.is_some());
                                existing_parent = conflict.then_some(existing).flatten();
                                permission = lr.permission;
                                error_msg = Some(msg);
                            } else {
                                status = lr.status;
                                let (canon_to_repo, canon_to_num) = caches
                                    .nodes
                                    .get(&node_cache_key(&to_repo, to_num))
                                    .map(|n| (n.repo.clone(), n.number))
                                    .unwrap_or((to_repo.clone(), to_num));
                                let _ = self.clear_dismissed_edge(
                                    PARENT,
                                    &from_node.repo,
                                    from_node.number,
                                    &canon_to_repo,
                                    canon_to_num,
                                );
                                let _ = self.clear_dismissed_edge(PARENT, &from_repo, from_num, &to_repo, to_num);
                            }
                        } else {
                            // blockedBy (native or body) OR confirming advisory fileConflict/codeDep -> native blockedBy
                            let plan = WiringPlan {
                                blocked_by: vec![(to_repo.clone(), to_num)],
                                parent: None,
                                replace_parent: false,
                                labels: Vec::new(),
                            };
                            let report = self.wire_issue_node(&from_node, &plan, &mut caches).await;
                            let lr = report.blocked_by.into_iter().next().expect("blockedBy plan produces report");
                            to_canon = lr.reference;
                            if lr.status == "error" {
                                ok = false;
                                status = "error";
                                permission = lr.permission;
                                error_msg = lr.error;
                            } else {
                                status = lr.status;
                                let (canon_to_repo, canon_to_num) = caches
                                    .nodes
                                    .get(&node_cache_key(&to_repo, to_num))
                                    .map(|n| (n.repo.clone(), n.number))
                                    .unwrap_or((to_repo.clone(), to_num));
                                let _ = self.clear_dismissed_edge(
                                    BLOCKED_BY,
                                    &from_node.repo,
                                    from_node.number,
                                    &canon_to_repo,
                                    canon_to_num,
                                );
                                let _ = self.clear_dismissed_edge(BLOCKED_BY, &from_repo, from_num, &to_repo, to_num);
                                if matches!(kind.as_str(), FILE_CONFLICT | CODE_DEP) {
                                    let (fk, tk) = canonical_dismissed_edge_key(
                                        &kind,
                                        &from_node.repo,
                                        from_node.number,
                                        &canon_to_repo,
                                        canon_to_num,
                                    );
                                    let _ = self.store.flow_dismissed_edge_add(&fk, &kind, &tk, &self.now());
                                }
                            }
                        }
                    }
                }
            }
            "remove" => {
                let mut caches = WireCaches::new();
                match (
                    self.resolve_issue_cached(&mut caches, &from_repo, from_num).await,
                    self.resolve_issue_cached(&mut caches, &to_repo, to_num).await,
                ) {
                    (Err(e), _) | (_, Err(e)) => {
                        ok = false;
                        status = "error";
                        error_msg = Some(e.message);
                        permission = e.permission;
                    }
                    (Ok(from_node), Ok(to_node)) => {
                        from_canon = ref_slug(&from_node.repo, from_node.number);
                        to_canon = ref_slug(&to_node.repo, to_node.number);
                        let res = if kind == PARENT {
                            github::remove_sub_issue(&*self.github, &to_node.id, &from_node.id).await
                        } else {
                            github::remove_blocked_by(&*self.github, &from_node.id, &to_node.id).await
                        };
                        match res {
                            Ok(UnlinkOutcome::Removed) => status = "removed",
                            Ok(UnlinkOutcome::NotLinked) => status = "notLinked",
                            Err(e) => {
                                ok = false;
                                status = "error";
                                permission = e.permission;
                                error_msg = Some(e.message);
                            }
                        }
                    }
                }
            }
            _ => unreachable!("validated action"),
        }

        let (flow, refresh_error) = match self.flow_view(true).await {
            Ok(fresh) => (Some(serde_json::to_value(fresh)?), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        Ok(FlowEdgeResponse {
            ok,
            action,
            from: from_canon,
            to: to_canon,
            kind,
            status,
            error: error_msg,
            permission,
            conflict,
            existing_parent,
            flow,
            refresh_error,
        })
    }

    pub async fn update_flow_issue(&self, args: FlowIssueUpdateArgs) -> Result<FlowIssueUpdateResponse> {
        if args.track.is_none() && args.priority.is_none() {
            bail!("update_flow_issue: specify track and/or priority");
        }
        let parsed = parse_github_issue_only(args.issue.trim()).ok_or_else(|| {
            anyhow!(
                "update_flow_issue: issue {:?} is not a GitHub issue; use <owner>/<repo>#<n> or a GitHub issue URL",
                args.issue
            )
        })?;
        let (owner, repo, number) = parsed.github_parts().expect("GitHub issue has parts");
        if number == 0 || number > MAX_GRAPHQL_ISSUE_NUMBER {
            bail!("update_flow_issue: issue {:?} has an out-of-range issue number", args.issue);
        }
        let full_repo = format!("{owner}/{repo}");
        let track_edit: Option<Option<String>> = match args.track {
            None => None,
            Some(opt) => Some(normalize_track(opt, "update_flow_issue")?),
        };
        let priority_edit: Option<Option<&'static str>> = match args.priority {
            None => None,
            Some(opt) => Some(normalize_priority(opt, "update_flow_issue")?),
        };

        let mut caches = WireCaches::new();
        let (issue_canon, labels, errors) = match self.resolve_issue_cached(&mut caches, &full_repo, number).await {
            Err(e) => {
                let target = ref_slug(&full_repo, number);
                (
                    target.clone(),
                    Vec::new(),
                    vec![WiringError { op: "readIssue", target, message: e.message, permission: e.permission }],
                )
            }
            Ok(node) => {
                let canon = ref_slug(&node.repo, node.number);
                let (lbls, errs) = self
                    .apply_track_and_priority(
                        &node,
                        track_edit.as_ref().map(|o| o.as_deref()),
                        priority_edit,
                        &mut caches,
                    )
                    .await;
                (canon, lbls, errs)
            }
        };

        let (flow, refresh_error) = match self.flow_view(true).await {
            Ok(fresh) => (Some(serde_json::to_value(fresh)?), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        Ok(FlowIssueUpdateResponse {
            ok: errors.is_empty(),
            issue: issue_canon,
            track: track_edit,
            priority: priority_edit.map(|o| o.map(String::from)),
            labels,
            errors,
            flow,
            refresh_error,
        })
    }

    pub async fn report_issue(&self, me: &str, a: IssueArgs) -> Result<Value> {
        let issue = parse_github_issue_only(&a.url).ok_or_else(|| {
            anyhow!(
                "wf.issue: {:?} is not a GitHub issue; use https://github.com/<owner>/<repo>/issues/<n> or <owner>/<repo>#<n>",
                a.url
            )
        })?;
        let (owner, repo, number) = issue.github_parts().expect("GitHub issue has owner, repo and number");
        let full_repo = format!("{owner}/{repo}");
        let title = text_for("wf.issue", a.title, "title", MAX_TITLE)?;
        let note = text_for("wf.issue", a.note, "note", MAX_NOTE)?;
        let track = normalize_track(a.track, "wf.issue")?;
        let priority = normalize_priority(a.priority, "wf.issue")?;

        let parent = match a.parent.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            Some(raw) => {
                let parsed = parse_github_ref(&raw, &full_repo, "wf.issue", "parent")?;
                if parsed.1 == number && parsed.0.eq_ignore_ascii_case(&full_repo) {
                    bail!("wf.issue: issue cannot be its own parent");
                }
                Some(parsed)
            }
            None => None,
        };

        let mut blocked_by: Vec<(String, u64)> = Vec::new();
        for raw in a.blocked_by.unwrap_or_default() {
            let parsed = parse_github_ref(&raw, &full_repo, "wf.issue", "blockedBy")?;
            if parsed.1 == number && parsed.0.eq_ignore_ascii_case(&full_repo) {
                bail!("wf.issue: issue cannot be blocked by itself");
            }
            if !blocked_by.iter().any(|(r, n)| *n == parsed.1 && r.eq_ignore_ascii_case(&parsed.0)) {
                blocked_by.push(parsed);
            }
        }

        let labels = desired_labels(track.as_deref(), priority, None, &[]);
        let has_wiring = !blocked_by.is_empty() || parent.is_some() || !labels.is_empty();
        let should_sync = a.sync.unwrap_or(has_wiring) && has_wiring;
        if should_sync && (number == 0 || number > MAX_GRAPHQL_ISSUE_NUMBER) {
            bail!("wf.issue: issue number #{number} is out of range for GitHub GraphQL");
        }

        if let Some(t) = &title {
            self.note_issue_title(&full_repo, number, t);
        }
        let mut out = self.record_board_issue(me, &issue, title, note, None).await?;
        if should_sync {
            let plan = WiringPlan { blocked_by, parent, replace_parent: false, labels };
            let report = self.wire_existing_issue(&full_repo, number, &plan).await;
            if !report.ok {
                let summary: Vec<String> = report
                    .errors
                    .iter()
                    .map(|e| {
                        if e.permission {
                            format!("{} ({}) [permission]: {}", e.op, e.target, e.message)
                        } else {
                            format!("{} ({}): {}", e.op, e.target, e.message)
                        }
                    })
                    .collect();
                if let Some(msg) = out["message"].as_str() {
                    out["message"] = json!(format!("{msg}; GitHub wiring failed: {}", summary.join("; ")));
                }
            }
            out["wiring"] = serde_json::to_value(&report)?;
        }
        Ok(out)
    }

    pub async fn file_issues(&self, caller: Option<&str>, a: FileIssuesArgs) -> Result<Value> {
        let (planned, sync, report_opt) = validate_batch(a)?;
        let should_report = match (report_opt, caller) {
            (Some(true), None) => bail!("{NO_CALLER}"),
            (Some(flag), _) => flag,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        let order = topo_sort_plan(&planned)?;

        // Pre-flight: resolve every target repository id and every external issue reference before
        // creating any issue so bad repos, nonexistent/guessed #N refs (which could collide with
        // newly allocated numbers and create self-cycles), or an unreachable gh fail with zero
        // partial writes.
        let mut repos: HashMap<String, RepoNode> = HashMap::new();
        for &idx in &order {
            let repo_name = &planned[idx].repo;
            let key = repo_name.to_ascii_lowercase();
            if repos.contains_key(&key) {
                continue;
            }
            let (owner, name) = repo_name.split_once('/').expect("validated owner/name");
            let node = github::read_repo(&*self.github, owner, name)
                .await
                .map_err(|e| anyhow!("wf.fileIssues: cannot read repository {repo_name}: {}", e.message))?;
            repos.insert(key, node);
        }

        let mut caches = WireCaches::new();
        for &idx in &order {
            let item = &planned[idx];
            let item_label =
                item.id.as_ref().map_or_else(|| format!("issues[{}]", item.input_index), |s| format!("item {s:?}"));
            let ext_refs = item.parent.iter().chain(&item.blocked_by).filter_map(|r| match r {
                BatchRef::External(repo, num) => Some((repo.as_str(), *num)),
                BatchRef::Local(_) => None,
            });
            for (ext_repo, ext_num) in ext_refs {
                if let Err(e) = self.resolve_issue_cached(&mut caches, ext_repo, ext_num).await {
                    bail!(
                        "wf.fileIssues: external issue reference {ext_repo}#{ext_num} does not exist or cannot be read ({}); use a local id for issues created in this batch; no issues were created",
                        e.message
                    );
                }
            }
            if let Some(raw_body) = item.body.as_deref()
                && !raw_body.trim().is_empty()
            {
                let will_write_flow = item.parent.is_some()
                    || !item.blocked_by.is_empty()
                    || item.track.is_some()
                    || item.priority.is_some()
                    || !item.touches.is_empty();
                let remaining = remaining_body_outside_replaced_flow(raw_body, will_write_flow);
                let repo_name = &repos[&item.repo.to_ascii_lowercase()].name_with_owner;
                let parsed = flow_body::parse_body(repo_name, &remaining);
                for body_ref in parsed.parents.iter().chain(&parsed.blockers).chain(&parsed.sub_issues) {
                    match body_ref {
                        flow_body::BodyRef::Issue { repo, number } => {
                            if let Err(e) = self.resolve_issue_cached(&mut caches, repo, *number).await {
                                bail!(
                                    "wf.fileIssues: {item_label} body references nonexistent or unreadable issue {repo}#{number} ({}); declare batch dependencies in parent and blockedBy; no issues were created",
                                    e.message
                                );
                            }
                        }
                        flow_body::BodyRef::Symbolic(raw) => {
                            bail!(
                                "wf.fileIssues: {item_label} body contains symbolic dependency {raw:?}; declare dependencies in parent and blockedBy instead; no issues were created"
                            );
                        }
                        flow_body::BodyRef::Malformed(raw) => {
                            bail!(
                                "wf.fileIssues: {item_label} body contains malformed dependency {raw:?}; declare dependencies in parent and blockedBy instead; no issues were created"
                            );
                        }
                        flow_body::BodyRef::OptionalSymbolic(_) => {}
                    }
                }
            }
        }

        let opt_in_label = self
            .config()
            .flow
            .as_ref()
            .and_then(|f| f.opt_in.as_ref())
            .and_then(|o| o.label.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let cached_thread_meta = match (should_report, caller) {
            (true, Some(me)) => Some(self.thread_meta(me).await),
            _ => None,
        };

        let mut created_by_idx: HashMap<usize, github::CreatedIssue> = HashMap::new();
        let mut out_order: Vec<String> = Vec::with_capacity(order.len());
        let mut out_issues: Vec<Value> = Vec::with_capacity(order.len());
        let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
        let mut all_errors: Vec<Value> = Vec::new();

        for &idx in &order {
            let item = &planned[idx];
            let repo_node = &repos[&item.repo.to_ascii_lowercase()];

            let resolved_parent = item.parent.as_ref().map(|r| match r {
                BatchRef::Local(p_idx) => {
                    let c = &created_by_idx[p_idx];
                    (c.repo.clone(), c.number)
                }
                BatchRef::External(r, n) => (r.clone(), *n),
            });
            let mut resolved_blockers: Vec<(String, u64)> = Vec::with_capacity(item.blocked_by.len());
            for r in &item.blocked_by {
                let target = match r {
                    BatchRef::Local(b_idx) => {
                        let c = &created_by_idx[b_idx];
                        (c.repo.clone(), c.number)
                    }
                    BatchRef::External(repo, n) => (repo.clone(), *n),
                };
                if !resolved_blockers.iter().any(|(er, en)| *en == target.1 && er.eq_ignore_ascii_case(&target.0)) {
                    resolved_blockers.push(target);
                }
            }

            let flow_meta = FlowMeta {
                parent: resolved_parent.as_ref().map(|(r, n)| format_flow_ref(&repo_node.name_with_owner, r, *n)),
                blocked_by: resolved_blockers
                    .iter()
                    .map(|(r, n)| format_flow_ref(&repo_node.name_with_owner, r, *n))
                    .collect(),
                track: item.track.clone(),
                priority: item.priority,
                touches: item.touches.clone(),
            };
            let final_body = merge_flow_body(item.body.as_deref(), &flow_meta);

            let created = match github::create_issue(&*self.github, repo_node, &item.title, &final_body, &[]).await {
                Ok(c) => c,
                Err(e) => {
                    let item_name = item.id.as_deref().unwrap_or(item.title.as_str());
                    if out_issues.is_empty() {
                        bail!(
                            "wf.fileIssues: creating {item_name:?} in {} failed: {}",
                            repo_node.name_with_owner,
                            e.message
                        );
                    }
                    all_errors.push(json!({
                        "issue": item_name,
                        "op": "createIssue",
                        "target": repo_node.name_with_owner,
                        "message": e.message,
                        "permission": e.permission,
                    }));
                    break;
                }
            };

            let created_node = IssueNode {
                id: created.id.clone(),
                repo_id: repo_node.id.clone(),
                repo: created.repo.clone(),
                number: created.number,
                url: created.url.clone(),
                title: created.title.clone(),
                parent: None,
                blocked_by: Vec::new(),
                labels: Vec::new(),
            };
            caches.nodes.insert(node_cache_key(&created.repo, created.number), created_node.clone());

            let item_opt_in = item.queue.then_some(opt_in_label.as_deref()).flatten();
            let labels = desired_labels(item.track.as_deref(), item.priority, item_opt_in, &item.labels);

            let wiring = if sync {
                let plan = WiringPlan {
                    blocked_by: resolved_blockers.clone(),
                    parent: resolved_parent.clone(),
                    replace_parent: false,
                    labels,
                };
                if plan.blocked_by.is_empty() && plan.parent.is_none() && plan.labels.is_empty() {
                    None
                } else {
                    let report = self.wire_issue_node(&created_node, &plan, &mut caches).await;
                    let issue_slug = ref_slug(&created.repo, created.number);
                    for err in &report.errors {
                        all_errors.push(json!({
                            "issue": issue_slug,
                            "op": err.op,
                            "target": err.target,
                            "message": err.message,
                            "permission": err.permission,
                        }));
                    }
                    Some(report)
                }
            } else {
                None
            };

            let short_label = {
                let name = created.repo.rsplit_once('/').map_or(created.repo.as_str(), |(_, n)| n);
                format!("{name}#{}", created.number)
            };

            let reported = if should_report
                && let Some(me) = caller
                && let Some(parsed_issue) = parse_github_issue_only(&created.url)
            {
                match self
                    .record_board_issue(
                        me,
                        &parsed_issue,
                        Some(created.title.clone()),
                        item.note.clone(),
                        cached_thread_meta.as_ref(),
                    )
                    .await
                {
                    Ok(v) => Some(v),
                    Err(e) => {
                        let msg = e.to_string();
                        all_errors.push(json!({
                            "issue": ref_slug(&created.repo, created.number),
                            "op": "reportBoard",
                            "target": created.url,
                            "message": msg,
                            "permission": false,
                        }));
                        Some(json!({"added": false, "error": msg}))
                    }
                }
            } else {
                None
            };

            let applied_labels: Vec<String> = wiring
                .as_ref()
                .map(|w| w.labels.iter().filter(|l| l.status != "error").map(|l| l.name.clone()).collect())
                .unwrap_or_default();

            if let Some(id) = &item.id {
                by_id.insert(
                    id.clone(),
                    json!({
                        "repo": created.repo,
                        "number": created.number,
                        "label": short_label,
                        "url": created.url,
                    }),
                );
                out_order.push(id.clone());
            } else {
                out_order.push(short_label.clone());
            }

            out_issues.push(json!({
                "id": item.id,
                "inputIndex": item.input_index,
                "repo": created.repo,
                "number": created.number,
                "label": short_label,
                "url": created.url,
                "title": created.title,
                "body": final_body,
                "parent": resolved_parent.as_ref().map(|(r, n)| ref_slug(r, *n)),
                "blockedBy": resolved_blockers.iter().map(|(r, n)| ref_slug(r, *n)).collect::<Vec<_>>(),
                "labels": applied_labels,
                "reported": reported,
                "wiring": wiring,
            }));

            created_by_idx.insert(idx, created);
        }

        Ok(json!({
            "ok": all_errors.is_empty(),
            "order": out_order,
            "issues": out_issues,
            "byId": by_id,
            "errors": all_errors,
        }))
    }

    fn worker_coverage(&self) -> Result<Vec<(String, u8, Value)>> {
        let mut out = Vec::new();
        for t in self.store.children(BOARD, false)? {
            for w in self.store.children(&t.child, false)? {
                let Some(issue) = w.identity.issue.clone() else { continue };
                let v = self.node_view(&w);
                let pr = &v["pr"];
                let (rank, kind) = if pr["state"] == "merged" {
                    (0, "merged")
                } else if pr["url"].is_string() && pr["state"] != "closed" {
                    (1, "pr")
                } else if !w.closed {
                    (2, "worker")
                } else {
                    continue;
                };
                out.push((
                    issue,
                    rank,
                    json!({
                        "kind": kind, "ticket": t.label, "ticketThread": t.child, "worker": w.label,
                        "thread": w.child, "link": v["link"], "status": v["status"],
                        "pr": if rank < 2 { pr.clone() } else { Value::Null },
                    }),
                ));
            }
        }
        out.sort_by_key(|(_, rank, _)| *rank);
        Ok(out)
    }

    pub fn new_issues(&self) -> Result<(Vec<Value>, Vec<Value>)> {
        let tickets = self.ticket_keys()?;
        let coverage = self.worker_coverage()?;
        let (mut open, mut dismissed) = (Vec::new(), Vec::new());
        for (seq, i) in self.store.new_issues()?.into_iter().rev() {
            let slug = format!("{}#{}", i.repo, i.number).to_ascii_lowercase();
            if tickets.contains(&slug) || tickets.contains(&i.label.to_ascii_lowercase()) {
                continue;
            }
            let mut v = issue_view(seq, &i);
            v["coveredBy"] = json!(
                coverage
                    .iter()
                    .filter(|(issue, ..)| self.same_issue_slug(issue, &slug))
                    .map(|(.., c)| c)
                    .collect::<Vec<_>>()
            );
            if i.dismissed_at.is_some() { dismissed.push(v) } else { open.push(v) }
        }
        Ok((open, dismissed))
    }

    fn new_issue_row(&self, id: &str) -> Result<(i64, NewIssue)> {
        let seq = seq_of(id)?;
        let i = self.store.new_issue_get(seq)?.ok_or_else(|| anyhow!("no new issue {id:?}"))?;
        Ok((seq, i))
    }

    pub async fn launch_new_issue(&self, id: &str) -> Result<Value> {
        let (_, i) = self.new_issue_row(id)?;
        let args = LaunchArgs {
            issue: i.url,
            parent_issue: None,
            part: None,
            project: None,
            title: i.title,
            model: None,
            effort: None,
        };
        let mut r = self.launch_ticket(args).await?;
        r["id"] = json!(id);
        Ok(r)
    }

    pub async fn dismiss_new_issue(&self, id: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let (seq, mut i) = self.new_issue_row(id)?;
        if i.dismissed_at.is_some() {
            bail!("new issue {id} is already dismissed");
        }
        i.dismissed_at = Some(self.now());
        self.store.new_issue_update(seq, &i)?;
        self.changed();
        Ok(json!({"id": id, "dismissed": true}))
    }

    pub async fn restore_new_issue(&self, id: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let (seq, mut i) = self.new_issue_row(id)?;
        if i.dismissed_at.is_none() {
            bail!("new issue {id} is not dismissed");
        }
        i.dismissed_at = None;
        i.restored_at = Some(self.now());
        self.store.new_issue_update(seq, &i)?;
        self.changed();
        Ok(json!({"id": id, "dismissed": false}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_body::{BodyRef, parse_body};

    #[test]
    fn flow_block_round_trips_through_flow_body_parser() {
        let meta = FlowMeta {
            parent: Some("#254".into()),
            blocked_by: vec!["#255".into(), "Other/lib#12".into()],
            track: Some("flow-dag".into()),
            priority: Some("high"),
            touches: vec!["src/issues.rs".into(), "src/github.rs".into()],
        };
        let body = merge_flow_body(Some("## Goal\n\nWire native links."), &meta);
        assert!(body.contains("## Flow\n- Parent: #254\n- Blocked by: #255, Other/lib#12\n- Track: flow-dag\n- Priority: high\n- Touches: `src/issues.rs`, `src/github.rs`"), "{body}");

        let parsed = parse_body("Acme/widgets", &body);
        assert_eq!(parsed.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 254 }]);
        assert_eq!(
            parsed.blockers,
            vec![
                BodyRef::Issue { repo: "Acme/widgets".into(), number: 255 },
                BodyRef::Issue { repo: "Other/lib".into(), number: 12 },
            ]
        );
        assert_eq!(parsed.track.as_deref(), Some("flow-dag"));
        assert_eq!(parsed.touches, vec!["src/issues.rs", "src/github.rs"]);
    }

    #[test]
    fn merge_flow_body_replaces_existing_flow_section_and_ignores_fenced_examples() {
        let meta = FlowMeta {
            parent: Some("#10".into()),
            blocked_by: vec!["#11".into()],
            track: Some("auth".into()),
            priority: Some("medium"),
            touches: vec!["src/auth.rs".into()],
        };

        // Existing ## Flow section between two sections is replaced in place.
        let with_old = "## Goal\nDo work.\n\n## Flow\n- Parent: P0\n- Blocked by: P1\n\n## Notes\nKeep this.";
        let replaced = merge_flow_body(Some(with_old), &meta);
        assert!(!replaced.contains("Parent: P0"), "{replaced}");
        assert!(replaced.contains("## Notes\nKeep this."), "{replaced}");
        let parsed = parse_body("Acme/widgets", &replaced);
        assert_eq!(parsed.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);
        assert_eq!(parsed.track.as_deref(), Some("auth"));
        assert_eq!(parsed.touches, vec!["src/auth.rs"]);

        // A ## Flow inside a fenced code block is NOT replaced; real block is appended.
        let with_fence = "## Example\n```markdown\n## Flow\n- Parent: #999\n```\n";
        let appended = merge_flow_body(Some(with_fence), &meta);
        assert!(appended.contains("- Parent: #999"), "fenced example must stay intact:\n{appended}");
        let parsed = parse_body("Acme/widgets", &appended);
        assert_eq!(parsed.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);

        // Unclosed top-level code fence or HTML comment is closed before appending ## Flow.
        let unclosed_fence = "## Notes\n```rust\nfn main() {}";
        let fixed_fence = merge_flow_body(Some(unclosed_fence), &meta);
        let parsed_fence = parse_body("Acme/widgets", &fixed_fence);
        assert_eq!(parsed_fence.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed_fence.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);

        let unclosed_comment = "## Notes\n<!-- unfinished draft comment";
        let fixed_comment = merge_flow_body(Some(unclosed_comment), &meta);
        let parsed_comment = parse_body("Acme/widgets", &fixed_comment);
        assert_eq!(parsed_comment.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed_comment.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);

        // Trailing non-breaking space on a closing fence trims away before inspection so no spurious fence is added.
        let nbsp_fence = "```\nexample\n```\u{00A0}";
        let fixed_nbsp = merge_flow_body(Some(nbsp_fence), &meta);
        let parsed_nbsp = parse_body("Acme/widgets", &fixed_nbsp);
        assert_eq!(parsed_nbsp.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed_nbsp.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);

        // A ## Flow heading inside a list item is NOT treated as a top-level Flow section, so
        // trailing prose outside the list is preserved instead of overwritten.
        let list_heading = "- Example\n  ## Flow\n  example metadata\n\nOutside description";
        let merged_list = merge_flow_body(Some(list_heading), &meta);
        assert!(merged_list.contains("Outside description"), "prose after list heading must stay:\n{merged_list}");
        let parsed_list = parse_body("Acme/widgets", &merged_list);
        assert_eq!(parsed_list.parents, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 10 }]);
        assert_eq!(parsed_list.blockers, vec![BodyRef::Issue { repo: "Acme/widgets".into(), number: 11 }]);
    }

    #[test]
    fn normalize_track_handles_multibyte_unicode_without_panicking() {
        assert_eq!(normalize_track(Some("café-é".into()), "wf.issue").unwrap().as_deref(), Some("café-é"));
        assert_eq!(normalize_track(Some("caféé".into()), "wf.issue").unwrap().as_deref(), Some("caféé"));
        assert_eq!(normalize_track(Some("Track: café-é".into()), "wf.issue").unwrap().as_deref(), Some("café-é"));
    }

    #[test]
    fn topo_sort_orders_parents_and_blockers_and_rejects_cycles() {
        let args: FileIssuesArgs = serde_json::from_value(json!({
            "repo": "Acme/widgets",
            "issues": [
                {"id": "P2", "title": "Phase 2", "parent": "E", "blockedBy": ["P1"]},
                {"id": "P1", "title": "Phase 1", "parent": "E", "blockedBy": ["P0"]},
                {"id": "P0", "title": "Phase 0", "parent": "E"},
                {"id": "E", "title": "Epic"}
            ]
        }))
        .unwrap();
        let (planned, _, _) = validate_batch(args).unwrap();
        let order = topo_sort_plan(&planned).unwrap();
        let ids: Vec<&str> = order.iter().map(|&i| planned[i].id.as_deref().unwrap()).collect();
        assert_eq!(ids, vec!["E", "P0", "P1", "P2"]);

        // Mixed parent + blocker cycle is rejected.
        let cyclic: FileIssuesArgs = serde_json::from_value(json!({
            "repo": "Acme/widgets",
            "issues": [
                {"id": "A", "title": "A", "parent": "B"},
                {"id": "B", "title": "B", "blockedBy": ["A"]}
            ]
        }))
        .unwrap();
        let (planned, _, _) = validate_batch(cyclic).unwrap();
        let err = topo_sort_plan(&planned).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
    }
}
