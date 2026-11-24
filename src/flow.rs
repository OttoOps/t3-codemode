use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{BOARD, Card, LaunchArgs, card_text, chip};
use crate::config::{FlowConfig, Launch, OptIn};
use crate::flow_body::{self, BodyRef, Candidate, ParsedBody, RefRole, ResolveOutcome};
use crate::github::errors;
use crate::wf::Wf;

pub const EXTERNAL: &str = "blocked: external";
pub const BLOCKED_LABEL: &str = "blocked";
pub const LAUNCH: &str = "launch";
pub const SKIP: &str = "skip";
const PRIORITIES: [&str; 3] = ["high", "medium", "low"];
const SKIPPED: &str = "flow_skip:";
const HELD: &str = "flow_done:";

pub const FLOW_TICKETS: &str = "query FlowTickets(";

macro_rules! issue_fields {
    () => {
        "number title url body createdAt state
        labels(first: 100) { nodes { name } }
        milestone { title dueOn }
        parent { number title url state repository { nameWithOwner } }
        blockedBy(first: 100) { totalCount nodes { number title url state repository { nameWithOwner } } }
        subIssues(first: 100) { totalCount nodes { number title url state repository { nameWithOwner } } }"
    };
}

pub const FLOW_ISSUES: &str = concat!(
    "query FlowIssues($owner: String!, $name: String!, $label: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    issues(states: OPEN, labels: [$label], first: 100, after: $after, orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        ",
    issue_fields!(),
    "
      }
    }
  }
}"
);

pub const FLOW_OPEN_ISSUES: &str = concat!(
    "query FlowIssues($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    issues(states: OPEN, first: 100, after: $after, orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        ",
    issue_fields!(),
    "
      }
    }
  }
}"
);

const TRACK: &str = "track:";
const TRACK_SUFFIX: &str = "-track";
const AREA: &str = "area:";
pub const BLOCKED_BY: &str = "blockedBy";
pub const PARENT: &str = "parent";
pub const SOURCE_GITHUB: &str = "github";
pub const SOURCE_BODY: &str = "body";
#[allow(dead_code)]
pub const SOURCE_GIT: &str = "git";
#[allow(dead_code)]
pub const SOURCE_CODE: &str = "code";
pub const WARN_CYCLE: &str = "cycle";
pub const WARN_UNRESOLVED: &str = "unresolved";
pub const WARN_BLOCKED_LABEL: &str = "blockedLabel";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRef {
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub url: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    pub title: String,
    pub due_on: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowIssue {
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub url: String,
    pub title: String,
    pub priority: Option<&'static str>,
    pub milestone: Option<Milestone>,
    pub created_at: String,
    pub parent: Option<IssueRef>,
    pub unblocked_by: Vec<IssueRef>,
    #[serde(skip)]
    pub(crate) key: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowPick {
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub url: String,
    pub title: String,
    pub key: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedIssue {
    #[serde(flatten)]
    pub issue: FlowIssue,
    pub blockers: Vec<IssueRef>,
    pub more_blockers: bool,
    pub external: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked_label: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowTicket {
    pub label: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub thread: Option<String>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowView {
    pub enabled: bool,
    pub launch: Option<Launch>,
    pub max_tickets: usize,
    pub free: usize,
    pub running: Vec<FlowTicket>,
    pub queue: Vec<FlowIssue>,
    pub blocked: Vec<BlockedIssue>,
    pub skipped: Vec<FlowIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<Graph>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphWarning {
    pub kind: &'static str,
    pub message: String,
    pub nodes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeWarning {
    pub kind: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub warnings: Vec<GraphWarning>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub source: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTicket {
    pub thread: Option<String>,
    pub link: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub url: String,
    pub title: Option<String>,
    pub state: Option<&'static str>,
    pub tracked: bool,
    pub labels: Vec<String>,
    pub priority: Option<&'static str>,
    pub milestone: Option<Milestone>,
    pub track: String,
    pub track_by: &'static str,
    pub parent: Option<String>,
    pub sub_issues: Vec<String>,
    pub more_sub_issues: bool,
    pub blocked_by: Vec<String>,
    pub more_blockers: bool,
    pub unlocks: Vec<String>,
    pub flow_list: Option<&'static str>,
    pub ready: bool,
    pub external: bool,
    pub ticket: Option<GraphTicket>,
    pub warnings: Vec<NodeWarning>,
}

#[derive(Clone, Default)]
struct CoveredRepos {
    all: Vec<String>,
    by_short: BTreeMap<String, Option<String>>,
}

impl CoveredRepos {
    fn insert(&mut self, full: String) {
        if self.all.iter().any(|r| r.eq_ignore_ascii_case(&full)) {
            return;
        }
        let short = full.rsplit_once('/').map_or(full.as_str(), |(_, n)| n).to_ascii_lowercase();
        self.by_short
            .entry(short)
            .and_modify(|slot| {
                if slot.as_deref().is_some_and(|prev| !prev.eq_ignore_ascii_case(&full)) {
                    *slot = None;
                }
            })
            .or_insert_with(|| Some(full.clone()));
        self.all.push(full);
    }

    fn contains(&self, repo: &str) -> bool {
        self.all.iter().any(|c| c.eq_ignore_ascii_case(repo))
    }

    fn resolve_short(&self, short: &str) -> Option<String> {
        self.by_short.get(&short.to_ascii_lowercase()).and_then(Clone::clone)
    }

    fn is_ambiguous_short(&self, short: &str) -> bool {
        matches!(self.by_short.get(&short.to_ascii_lowercase()), Some(None))
    }

    fn tracked_set(&self) -> HashSet<String> {
        self.all.iter().map(|r| r.to_ascii_lowercase()).collect()
    }
}

struct Reads {
    reads: Vec<Read>,
    covered: CoveredRepos,
    unreadable: CoveredRepos,
    unreadable_projects: HashSet<String>,
    has_unavailable_discovery: bool,
    aliases: std::collections::HashMap<String, String>,
}

struct ResolvedRepo {
    repo: String,
    explicit: bool,
    project_ids: Vec<String>,
}

struct ResolvedRepos {
    repos: Vec<ResolvedRepo>,
    unavailable_projects: Vec<String>,
}

impl Reads {
    fn capacity_repos(&self) -> CoveredRepos {
        let mut out = self.covered.clone();
        for repo in &self.unreadable.all {
            out.insert(repo.clone());
        }
        out
    }
}

#[derive(Clone)]
struct Ticket {
    label: String,
    thread: Option<String>,
    link: Option<String>,
    repo: Option<String>,
    project: Option<String>,
    issue: Option<String>,
    status: String,
}

#[derive(Clone)]
struct Target {
    to: IssueRef,
    state: Option<&'static str>,
    source: &'static str,
}

#[derive(Clone)]
struct Read {
    issue: FlowIssue,
    state: Option<&'static str>,
    labels: Vec<String>,
    body_track: Option<String>,
    #[allow(dead_code)] // PR 5 (#257) uses Touches: for fileConflict edges
    body_touches: Vec<String>,
    body_parsed: ParsedBody,
    open_blockers: Vec<IssueRef>,
    more_blockers: bool,
    blockers: Vec<Target>,
    parent: Option<Target>,
    extra_parents: Vec<Target>,
    subs: Vec<Target>,
    more_subs: bool,
    unresolved_blockers: Vec<String>,
    warnings: Vec<NodeWarning>,
}

fn label_of(repo: &str, number: u64) -> String {
    let name = repo.rsplit_once('/').map_or(repo, |(_, n)| n);
    format!("{name}#{number}")
}

fn issue_ref(v: &Value) -> Option<IssueRef> {
    let repo = v["repository"]["nameWithOwner"].as_str()?.to_string();
    let number = v["number"].as_u64()?;
    Some(IssueRef {
        label: label_of(&repo, number),
        url: v["url"].as_str().map_or_else(|| format!("https://github.com/{repo}/issues/{number}"), String::from),
        title: v["title"].as_str().map(String::from),
        repo,
        number,
    })
}

fn priority(labels: &[String]) -> Option<&'static str> {
    labels
        .iter()
        .filter_map(|l| {
            let rest = l.to_ascii_lowercase().strip_prefix("priority:")?.trim().to_string();
            PRIORITIES.iter().position(|p| *p == rest)
        })
        .min()
        .map(|i| PRIORITIES[i])
}

fn rank(p: Option<&str>) -> usize {
    p.and_then(|p| PRIORITIES.iter().position(|q| *q == p)).unwrap_or(PRIORITIES.len())
}

fn due(i: &FlowIssue) -> Option<&str> {
    i.milestone.as_ref().and_then(|m| m.due_on.as_deref())
}

pub fn queue_order(a: &FlowIssue, b: &FlowIssue) -> Ordering {
    let by_due = match (due(a), due(b)) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    rank(a.priority)
        .cmp(&rank(b.priority))
        .then(by_due)
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.repo.to_ascii_lowercase().cmp(&b.repo.to_ascii_lowercase()))
        .then(a.number.cmp(&b.number))
}

fn state_of(v: &Value) -> Option<&'static str> {
    match v["state"].as_str() {
        Some("OPEN") => Some("open"),
        Some("CLOSED") => Some("closed"),
        _ => None,
    }
}

fn target(v: &Value) -> Option<Target> {
    Some(Target { to: issue_ref(v)?, state: state_of(v), source: SOURCE_GITHUB })
}

fn targets(v: &Value) -> Vec<Target> {
    v["nodes"].as_array().into_iter().flatten().filter_map(target).collect()
}

fn more(v: &Value, listed: usize) -> bool {
    v["totalCount"].as_u64().is_some_and(|t| t > listed as u64)
}

fn has_blocked_label(labels: &[String]) -> bool {
    labels.iter().any(|l| l.trim().eq_ignore_ascii_case(BLOCKED_LABEL))
}

fn source_rank(source: &str) -> u8 {
    match source {
        SOURCE_GITHUB => 0,
        SOURCE_BODY => 1,
        SOURCE_GIT => 2,
        SOURCE_CODE => 3,
        _ => 4,
    }
}

fn compute_skip_key(
    labels: &[String],
    blockers: &[Target],
    more_blockers: bool,
    unresolved_blockers: &[String],
) -> String {
    let mut names: Vec<String> = labels.iter().map(|l| l.trim().to_ascii_lowercase()).collect();
    names.sort();
    let mut held: Vec<String> = blockers
        .iter()
        .map(|b| {
            format!(
                "{}#{}:{}",
                b.to.repo.to_ascii_lowercase(),
                b.to.number,
                if b.state == Some("closed") { "closed" } else { "open" }
            )
        })
        .collect();
    held.sort();
    held.dedup();
    if unresolved_blockers.is_empty() {
        json!([names, held, more_blockers]).to_string()
    } else {
        let mut unres: Vec<String> = unresolved_blockers.iter().map(|u| u.trim().to_ascii_lowercase()).collect();
        unres.sort();
        unres.dedup();
        json!([names, held, more_blockers, unres]).to_string()
    }
}

fn parse_node(repo: &str, n: &Value) -> Result<Read> {
    let number = n["number"].as_u64().ok_or_else(|| anyhow!("GitHub returned an issue in {repo} with no number"))?;
    let labels: Vec<String> = n["labels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["name"].as_str())
        .map(String::from)
        .collect();
    let all_blockers = targets(&n["blockedBy"]);
    let subs = targets(&n["subIssues"]);
    let more_subs = more(&n["subIssues"], n["subIssues"]["nodes"].as_array().map_or(0, Vec::len));
    let blockers: Vec<&Value> = n["blockedBy"]["nodes"].as_array().into_iter().flatten().collect();
    let more_blockers = n["blockedBy"]["totalCount"].as_u64().is_some_and(|t| t > blockers.len() as u64);
    let key = compute_skip_key(&labels, &all_blockers, more_blockers, &[]);
    let (open, closed): (Vec<&Value>, Vec<&Value>) = blockers.into_iter().partition(|b| b["state"] != "CLOSED");
    let milestone = n["milestone"]["title"]
        .as_str()
        .map(|t| Milestone { title: t.to_string(), due_on: n["milestone"]["dueOn"].as_str().map(String::from) });
    let body_text = n["body"].as_str().unwrap_or_default();
    let body_parsed = flow_body::parse_body(repo, body_text);
    let body_track = body_parsed.track.clone();
    let body_touches = body_parsed.touches.clone();
    let issue = FlowIssue {
        repo: repo.to_string(),
        number,
        label: label_of(repo, number),
        url: n["url"].as_str().map_or_else(|| format!("https://github.com/{repo}/issues/{number}"), String::from),
        title: n["title"].as_str().unwrap_or_default().to_string(),
        priority: priority(&labels),
        milestone,
        created_at: n["createdAt"].as_str().unwrap_or_default().to_string(),
        parent: issue_ref(&n["parent"]),
        unblocked_by: closed.into_iter().filter_map(issue_ref).collect(),
        key,
    };
    Ok(Read {
        more_blockers,
        open_blockers: open.into_iter().filter_map(issue_ref).collect(),
        state: state_of(n),
        blockers: all_blockers,
        parent: target(&n["parent"]),
        extra_parents: Vec::new(),
        subs,
        more_subs,
        labels,
        body_track,
        body_touches,
        body_parsed,
        unresolved_blockers: Vec::new(),
        warnings: Vec::new(),
        issue,
    })
}

fn skip_key(repo: &str, number: u64) -> String {
    format!("{SKIPPED}{}#{number}", repo.to_ascii_lowercase())
}

fn ticket_issue(covered: &CoveredRepos, label: &str) -> Option<(String, u64)> {
    let (name, n) = label.rsplit_once('#')?;
    let number: u64 = n.parse().ok()?;
    Some((covered.resolve_short(name)?, number))
}

fn ticket_repo(covered: &CoveredRepos, t: &Ticket) -> Option<(String, u64)> {
    if let Some(issue) = t.issue.as_deref() {
        let parsed = crate::board::parse_github_issue_only(issue)?;
        let (owner, repo, number) = parsed.github_parts()?;
        return Some((format!("{owner}/{repo}"), number));
    }
    let (name, n) = t.label.rsplit_once('#')?;
    let known = t.repo.as_deref().filter(|r| r.rsplit_once('/').is_some_and(|(_, s)| s.eq_ignore_ascii_case(name)));
    match known {
        Some(repo) => Some((repo.to_string(), n.parse().ok()?)),
        None => ticket_issue(covered, &t.label),
    }
}

fn canonicalize_repo(aliases: &std::collections::HashMap<String, String>, repo: &str) -> String {
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

fn ticket_repo_canonical(
    covered: &CoveredRepos,
    aliases: &std::collections::HashMap<String, String>,
    t: &Ticket,
) -> Option<(String, u64)> {
    let (repo, number) = ticket_repo(covered, t)?;
    Some((canonicalize_repo(aliases, &repo), number))
}

fn node_key(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo.to_ascii_lowercase())
}

fn prefix_label(labels: &[String], prefix: &str) -> Option<String> {
    labels
        .iter()
        .filter_map(|l| {
            let (head, rest) = l.split_at_checked(prefix.len())?;
            let rest = rest.trim();
            (head.eq_ignore_ascii_case(prefix) && !rest.is_empty()).then(|| rest.to_string())
        })
        .min_by_key(|t| t.to_ascii_lowercase())
}

fn suffix_track_label(labels: &[String]) -> Option<String> {
    labels
        .iter()
        .filter_map(|l| {
            let trimmed = l.trim();
            let split = trimmed.len().checked_sub(TRACK_SUFFIX.len())?;
            let (stem, tail) = trimmed.split_at_checked(split)?;
            let stem = stem.trim();
            (tail.eq_ignore_ascii_case(TRACK_SUFFIX) && !stem.is_empty()).then(|| stem.to_string())
        })
        .min_by_key(|t| t.to_ascii_lowercase())
}

fn explicit_track(labels: &[String], body_track: Option<&str>) -> Option<(String, &'static str)> {
    if let Some(t) = prefix_label(labels, TRACK) {
        return Some((t, "label"));
    }
    if let Some(t) = suffix_track_label(labels) {
        return Some((t, "label"));
    }
    if let Some(t) = prefix_label(labels, AREA) {
        return Some((t, "label"));
    }
    if let Some(t) = body_track.map(str::trim).filter(|s| !s.is_empty()) {
        return Some((t.to_string(), "label"));
    }
    None
}

fn resolve_epic(
    start: &str,
    parent_of: &BTreeMap<String, String>,
    explicit: &BTreeMap<String, String>,
    nodes: &BTreeMap<String, GraphNode>,
) -> (Option<String>, String) {
    let mut seen: Vec<String> = vec![start.to_string()];
    let mut cur = start;
    let mut inherited_track: Option<String> = None;
    while let Some(p) = parent_of.get(cur) {
        if let Some(t) = explicit.get(p)
            && inherited_track.is_none()
        {
            inherited_track = Some(t.clone());
        }
        if let Some(pos) = seen.iter().position(|x| x == p) {
            let root = seen[pos..]
                .iter()
                .min_by_key(|k| {
                    let n = &nodes[*k];
                    (n.repo.to_ascii_lowercase(), n.number)
                })
                .expect("non-empty cycle")
                .clone();
            return (inherited_track, root);
        }
        seen.push(p.clone());
        cur = p;
    }
    (inherited_track, cur.to_string())
}

fn warn_kind_rank(kind: &str) -> u8 {
    match kind {
        WARN_CYCLE => 0,
        WARN_UNRESOLVED => 1,
        WARN_BLOCKED_LABEL => 2,
        _ => 3,
    }
}

#[derive(Default)]
struct GraphBuild {
    nodes: BTreeMap<String, GraphNode>,
    edges: BTreeMap<(String, &'static str, String), &'static str>,
    body_tracks: BTreeMap<String, String>,
    more_blockers: HashSet<String>,
    more_subs: HashSet<String>,
    warnings: Vec<GraphWarning>,
}

impl GraphBuild {
    fn add_edge(&mut self, from: String, kind: &'static str, to: String, source: &'static str) {
        self.edges
            .entry((from, kind, to))
            .and_modify(|cur| {
                if source_rank(source) < source_rank(cur) {
                    *cur = source;
                }
            })
            .or_insert(source);
    }

    fn node(&mut self, r: &IssueRef, state: Option<&'static str>) -> String {
        let key = node_key(&r.repo, r.number);
        let n = self.nodes.entry(key.clone()).or_insert_with(|| GraphNode {
            id: format!("{}#{}", r.repo, r.number),
            repo: r.repo.clone(),
            number: r.number,
            label: r.label.clone(),
            url: r.url.clone(),
            title: None,
            state: None,
            tracked: false,
            labels: Vec::new(),
            priority: None,
            milestone: None,
            track: String::new(),
            track_by: "",
            parent: None,
            sub_issues: Vec::new(),
            more_sub_issues: false,
            blocked_by: Vec::new(),
            more_blockers: false,
            unlocks: Vec::new(),
            flow_list: None,
            ready: false,
            external: false,
            ticket: None,
            warnings: Vec::new(),
        });
        n.title = n.title.take().or_else(|| r.title.clone());
        n.state = n.state.or(state);
        key
    }

    fn read(&mut self, r: Read, list: Option<&'static str>, tracked: &HashSet<String>) {
        let i = &r.issue;
        let me = IssueRef {
            repo: i.repo.clone(),
            number: i.number,
            label: i.label.clone(),
            url: i.url.clone(),
            title: None,
        };
        let key = self.node(&me, r.state);
        for b in &r.blockers {
            let to = self.node(&b.to, b.state);
            self.add_edge(key.clone(), BLOCKED_BY, to, b.source);
        }
        if let Some(p) = &r.parent {
            let to = self.node(&p.to, p.state);
            self.add_edge(key.clone(), PARENT, to, p.source);
        }
        for p in &r.extra_parents {
            let to = self.node(&p.to, p.state);
            self.add_edge(key.clone(), PARENT, to, p.source);
        }
        for s in &r.subs {
            let from = self.node(&s.to, s.state);
            self.add_edge(from, PARENT, key.clone(), s.source);
        }
        if r.more_blockers {
            self.more_blockers.insert(key.clone());
        }
        if r.more_subs {
            self.more_subs.insert(key.clone());
        }
        if let Some(track) = r.body_track {
            self.body_tracks.insert(key.clone(), track);
        }
        let outside = |b: &IssueRef| !tracked.contains(&b.repo.to_ascii_lowercase());
        let has_external = r.labels.iter().any(|l| l.eq_ignore_ascii_case(EXTERNAL));
        let external = has_external
            || (!r.open_blockers.is_empty()
                && !r.more_blockers
                && r.unresolved_blockers.is_empty()
                && r.open_blockers.iter().all(outside));
        let mut node_warnings = r.warnings;
        if r.state != Some("closed")
            && has_blocked_label(&r.labels)
            && !has_external
            && r.open_blockers.is_empty()
            && !r.more_blockers
        {
            node_warnings.push(NodeWarning {
                kind: WARN_BLOCKED_LABEL,
                message: "Labelled \"blocked\" with no open blocker linked".to_string(),
                raw: None,
            });
        }
        let n = self.nodes.get_mut(&key).expect("node just added");
        let node_id = n.id.clone();
        let node_label = n.label.clone();
        for w in &node_warnings {
            if !n.warnings.contains(w) {
                n.warnings.push(w.clone());
                let graph_w = GraphWarning {
                    kind: w.kind,
                    message: format!("{node_label}: {}", w.message),
                    nodes: vec![node_id.clone()],
                    raw: w.raw.clone(),
                };
                if !self.warnings.contains(&graph_w) {
                    self.warnings.push(graph_w);
                }
            }
        }
        n.title = Some(i.title.clone());
        n.state = r.state.or(n.state);
        n.tracked = true;
        n.priority = i.priority;
        n.milestone = i.milestone.clone();
        n.flow_list = list.or(n.flow_list);
        n.ready = n.flow_list == Some("queue");
        n.external = external;
        n.labels = r.labels;
    }

    fn ticket(&mut self, repo: &str, number: u64, t: &Ticket) {
        let r = IssueRef {
            label: label_of(repo, number),
            url: format!("https://github.com/{repo}/issues/{number}"),
            repo: repo.to_string(),
            number,
            title: None,
        };
        let key = self.node(&r, None);
        let n = self.nodes.get_mut(&key).expect("node just added");
        n.tracked = true;
        n.ticket = Some(GraphTicket { thread: t.thread.clone(), link: t.link.clone(), status: t.status.clone() });
    }

    fn finish(mut self) -> Graph {
        let id = |nodes: &BTreeMap<String, GraphNode>, k: &str| nodes[k].id.clone();
        let mut up: BTreeMap<String, String> = self.nodes.keys().map(|k| (k.clone(), k.clone())).collect();
        fn root(up: &mut BTreeMap<String, String>, k: &str) -> String {
            let mut k = k.to_string();
            while up[&k] != k {
                let next = up[&up[&k]].clone();
                up.insert(k.clone(), next.clone());
                k = next;
            }
            k
        }
        let order = |nodes: &BTreeMap<String, GraphNode>, k: &str| {
            let n = &nodes[k];
            (n.repo.to_ascii_lowercase(), n.number)
        };
        for (from, _, to) in self.edges.keys() {
            let (a, b) = (root(&mut up, from), root(&mut up, to));
            if a != b {
                let (lo, hi) = if order(&self.nodes, &a) <= order(&self.nodes, &b) { (a, b) } else { (b, a) };
                up.insert(hi, lo);
            }
        }
        let mut sorted: Vec<(String, &'static str, String, &'static str)> =
            self.edges.into_iter().map(|((f, k, t), s)| (f, k, t, s)).collect();
        sorted.sort_by(|a, b| {
            order(&self.nodes, &a.0)
                .cmp(&order(&self.nodes, &b.0))
                .then(a.1.cmp(b.1))
                .then_with(|| order(&self.nodes, &a.2).cmp(&order(&self.nodes, &b.2)))
        });
        let mut edges = Vec::new();
        let mut parent_of: BTreeMap<String, (u8, String)> = BTreeMap::new();
        let mut blocked_pairs: Vec<(String, String)> = Vec::new();
        let mut parent_pairs: Vec<(String, String)> = Vec::new();
        let mut all_pairs: Vec<(String, String)> = Vec::new();
        for (from, kind, to, source) in &sorted {
            let (f, t) = (id(&self.nodes, from), id(&self.nodes, to));
            all_pairs.push((from.clone(), to.clone()));
            match *kind {
                BLOCKED_BY => {
                    blocked_pairs.push((from.clone(), to.clone()));
                    self.nodes.get_mut(from).expect("edge end").blocked_by.push(t.clone());
                    self.nodes.get_mut(to).expect("edge end").unlocks.push(f.clone());
                }
                _ => {
                    parent_pairs.push((from.clone(), to.clone()));
                    let sr = source_rank(source);
                    if from != to {
                        parent_of
                            .entry(from.clone())
                            .and_modify(|cur| {
                                if sr < cur.0 {
                                    *cur = (sr, to.clone());
                                }
                            })
                            .or_insert_with(|| (sr, to.clone()));
                    }
                    self.nodes.get_mut(to).expect("edge end").sub_issues.push(f.clone());
                }
            }
            edges.push(GraphEdge { from: f, to: t, kind, source });
        }
        let parent_of: BTreeMap<String, String> = parent_of.into_iter().map(|(k, (_, v))| (k, v)).collect();
        for (child, parent) in &parent_of {
            let pid = id(&self.nodes, parent);
            self.nodes.get_mut(child).expect("edge end").parent = Some(pid);
        }
        // Preserve self-parent edge in `node.parent` if it was the only parent edge.
        for (from, kind, to, _) in &sorted {
            if *kind == PARENT && from == to {
                let pid = id(&self.nodes, to);
                let child = self.nodes.get_mut(from).expect("edge end");
                if child.parent.is_none() {
                    child.parent = Some(pid);
                }
            }
        }
        let mut keys: Vec<String> = self.nodes.keys().cloned().collect();
        keys.sort_by_key(|k| order(&self.nodes, k));

        let mut short_repos: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut repo_names: BTreeMap<String, String> = BTreeMap::new();
        for k in &keys {
            let repo = &self.nodes[k].repo;
            repo_names.entry(repo.to_ascii_lowercase()).or_insert_with(|| repo.clone());
            let short = repo.rsplit_once('/').map_or(repo.as_str(), |(_, s)| s).to_ascii_lowercase();
            short_repos
                .entry(short)
                .and_modify(|slot| {
                    if slot.as_deref().is_some_and(|prev| !prev.eq_ignore_ascii_case(repo)) {
                        *slot = None;
                    }
                })
                .or_insert_with(|| Some(repo.clone()));
        }
        let node_lane = |nodes: &BTreeMap<String, GraphNode>, k: &str| -> String {
            let n = &nodes[k];
            let short = n.repo.rsplit_once('/').map_or(n.repo.as_str(), |(_, s)| s).to_ascii_lowercase();
            if short_repos.get(&short).is_some_and(Option::is_none) { n.id.clone() } else { n.label.clone() }
        };

        // Detect cycles (blockedBy, parent, and mixed graph cycles) deterministically.
        let mut covered_cycle_sets: Vec<HashSet<String>> = Vec::new();
        for (prefix, pairs, check_subset) in [
            ("Dependency cycle", &blocked_pairs, false),
            ("Parent cycle", &parent_pairs, false),
            ("Graph cycle", &all_pairs, true),
        ] {
            for comp in flow_body::find_cycles(&keys, pairs) {
                let comp_set: HashSet<String> = comp.all_nodes.iter().cloned().collect();
                if check_subset && covered_cycle_sets.iter().any(|prev| comp_set.is_subset(prev)) {
                    continue;
                }
                let path_labels: Vec<String> = comp.cycle.iter().map(|k| node_lane(&self.nodes, k)).collect();
                let extra_labels: Vec<String> = comp
                    .all_nodes
                    .iter()
                    .filter(|k| !comp.cycle.contains(k))
                    .map(|k| node_lane(&self.nodes, k))
                    .collect();
                let mut message = if path_labels.len() == 1 {
                    format!("{prefix}: {} depends on itself", path_labels[0])
                } else {
                    format!("{prefix}: {} -> {}", path_labels.join(" -> "), path_labels[0])
                };
                if !extra_labels.is_empty() {
                    message.push_str(&format!(" (and {})", extra_labels.join(", ")));
                }
                let node_ids: Vec<String> = comp.all_nodes.iter().map(|k| id(&self.nodes, k)).collect();
                let nw = NodeWarning { kind: WARN_CYCLE, message: message.clone(), raw: None };
                for k in &comp.all_nodes {
                    let n = self.nodes.get_mut(k).expect("cycle node");
                    if !n.warnings.contains(&nw) {
                        n.warnings.push(nw.clone());
                    }
                }
                let gw = GraphWarning { kind: WARN_CYCLE, message, nodes: node_ids, raw: None };
                if !self.warnings.contains(&gw) {
                    self.warnings.push(gw);
                }
                covered_cycle_sets.push(comp_set);
            }
        }

        // Tier 1: track:<name>, *-track, area:*, or ## Flow Track: hook.
        let mut explicit: BTreeMap<String, String> = BTreeMap::new();
        for k in &keys {
            let body_t = self.body_tracks.get(k).map(String::as_str);
            let n = self.nodes.get_mut(k).expect("listed key");
            n.more_blockers = self.more_blockers.contains(k);
            n.more_sub_issues = self.more_subs.contains(k);
            if let Some((t, by)) = explicit_track(&n.labels, body_t) {
                explicit.insert(k.clone(), t.clone());
                (n.track, n.track_by) = (t, by);
            }
        }

        // Tier 2: parent epic (inheriting nearest ancestor's explicit track, else root epic label).
        let mut claimed_epics: HashSet<String> = HashSet::new();
        for k in &keys {
            if !self.nodes[k].track_by.is_empty() || !parent_of.contains_key(k) {
                continue;
            }
            let (inherited, epic_root) = resolve_epic(k, &parent_of, &explicit, &self.nodes);
            let lane = match inherited {
                Some(t) => t,
                None => {
                    claimed_epics.insert(epic_root.clone());
                    node_lane(&self.nodes, &epic_root)
                }
            };
            let n = self.nodes.get_mut(k).expect("listed key");
            (n.track, n.track_by) = (lane, "epic");
        }
        for k in &keys {
            if self.nodes[k].track_by.is_empty() && claimed_epics.contains(k) {
                let lane = node_lane(&self.nodes, k);
                let n = self.nodes.get_mut(k).expect("listed key");
                (n.track, n.track_by) = (lane, "epic");
            }
        }

        // Tier 3: milestone.
        for k in &keys {
            let n = self.nodes.get_mut(k).expect("listed key");
            if n.track_by.is_empty()
                && let Some(m) = &n.milestone
            {
                let title = m.title.trim();
                if !title.is_empty() {
                    (n.track, n.track_by) = (title.to_string(), "milestone");
                }
            }
        }

        // Tier 4: connected component only when component size >= 2.
        let mut comp_size: BTreeMap<String, usize> = BTreeMap::new();
        for k in &keys {
            let comp = root(&mut up, k);
            *comp_size.entry(comp).or_insert(0) += 1;
        }
        let tier_rank = |by: &str| match by {
            "label" => 0u8,
            "epic" => 1,
            "milestone" => 2,
            _ => 3,
        };
        let mut comp_lanes: BTreeMap<String, (u8, String)> = BTreeMap::new();
        for k in &keys {
            let comp = root(&mut up, k);
            if comp_size[&comp] < 2 {
                continue;
            }
            let n = &self.nodes[k];
            if !n.track_by.is_empty() {
                let r = tier_rank(n.track_by);
                comp_lanes
                    .entry(comp)
                    .and_modify(|cur| {
                        if r < cur.0 {
                            *cur = (r, n.track.clone());
                        }
                    })
                    .or_insert_with(|| (r, n.track.clone()));
            }
        }
        for k in &keys {
            let comp = root(&mut up, k);
            if comp_size[&comp] < 2 || !self.nodes[k].track_by.is_empty() {
                continue;
            }
            let lane = comp_lanes.get(&comp).map(|(_, t)| t.clone()).unwrap_or_else(|| node_lane(&self.nodes, &comp));
            let n = self.nodes.get_mut(k).expect("listed key");
            (n.track, n.track_by) = (lane, "component");
        }

        // Tier 5: shared per-repo Independent lane for singleton unlinked issues.
        for k in &keys {
            let n = self.nodes.get_mut(k).expect("listed key");
            if n.track_by.is_empty() {
                let repo = repo_names.get(&n.repo.to_ascii_lowercase()).map(String::as_str).unwrap_or(&n.repo);
                let lane = if repo.is_empty() { "Independent".to_string() } else { format!("{repo}: Independent") };
                (n.track, n.track_by) = (lane, "independent");
            }
        }

        let mut nodes: Vec<GraphNode> = self.nodes.into_values().collect();
        for n in &mut nodes {
            n.warnings.sort_by(|a, b| {
                warn_kind_rank(a.kind).cmp(&warn_kind_rank(b.kind)).then_with(|| a.message.cmp(&b.message))
            });
        }
        nodes.sort_by(|a, b| {
            a.repo.to_ascii_lowercase().cmp(&b.repo.to_ascii_lowercase()).then(a.number.cmp(&b.number))
        });
        let mut warnings = self.warnings;
        warnings.sort_by(|a, b| {
            warn_kind_rank(a.kind)
                .cmp(&warn_kind_rank(b.kind))
                .then_with(|| a.nodes.cmp(&b.nodes))
                .then_with(|| a.message.cmp(&b.message))
        });
        Graph { nodes, edges, warnings }
    }
}

fn graph(
    covered: &CoveredRepos,
    aliases: &std::collections::HashMap<String, String>,
    reads: Vec<Read>,
    tickets: &[Ticket],
    lists: &BTreeMap<String, &'static str>,
) -> Graph {
    let tracked = covered.tracked_set();
    let mut g = GraphBuild::default();
    for r in reads {
        let list = lists.get(&node_key(&r.issue.repo, r.issue.number)).copied();
        g.read(r, list, &tracked);
    }
    for t in tickets {
        if let Some((repo, number)) = ticket_repo_canonical(covered, aliases, t) {
            g.ticket(&repo, number, t);
        }
    }
    g.finish()
}

pub fn why(i: &FlowIssue) -> String {
    let mut s = format!("priority: {}", i.priority.unwrap_or("none"));
    if !i.unblocked_by.is_empty() {
        let by: Vec<String> = i
            .unblocked_by
            .iter()
            .map(|r| if r.repo.eq_ignore_ascii_case(&i.repo) { format!("#{}", r.number) } else { r.label.clone() })
            .collect();
        s.push_str(&format!(", unblocked by {}", by.join(", ")));
    }
    s
}

fn refs_query(asks: &[(String, u64)]) -> (String, Value) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    let mut vars = serde_json::Map::new();
    for (i, (repo, number)) in asks.iter().enumerate() {
        let (owner, name) = repo.split_once('/').unwrap_or_default();
        params.push(format!("$o{i}: String!, $r{i}: String!, $n{i}: Int!"));
        fields.push(format!(
            "t{i}: repository(owner: $o{i}, name: $r{i}) {{ nameWithOwner issue(number: $n{i}) {{ number title url state }} }}"
        ));
        vars.insert(format!("o{i}"), json!(owner));
        vars.insert(format!("r{i}"), json!(name));
        vars.insert(format!("n{i}"), json!(number));
    }
    (format!("{FLOW_TICKETS}{}) {{ {} }}", params.join(", "), fields.join(" ")), Value::Object(vars))
}

fn tickets_query(asks: &[(String, u64)], full: bool) -> (String, Value) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    let mut vars = serde_json::Map::new();
    for (i, (repo, number)) in asks.iter().enumerate() {
        let (owner, name) = repo.split_once('/').unwrap_or_default();
        params.push(format!("$o{i}: String!, $r{i}: String!, $n{i}: Int!"));
        fields.push(if full {
            format!(
                "t{i}: repository(owner: $o{i}, name: $r{i}) {{ nameWithOwner issue(number: $n{i}) {{ {} }} }}",
                issue_fields!()
            )
        } else {
            format!("t{i}: repository(owner: $o{i}, name: $r{i}) {{ nameWithOwner issue(number: $n{i}) {{ state }} }}")
        });
        vars.insert(format!("o{i}"), json!(owner));
        vars.insert(format!("r{i}"), json!(name));
        vars.insert(format!("n{i}"), json!(number));
    }
    (format!("{FLOW_TICKETS}{}) {{ {} }}", params.join(", "), fields.join(" ")), Value::Object(vars))
}

fn same(p: &FlowPick, i: &FlowIssue) -> bool {
    p.repo.eq_ignore_ascii_case(&i.repo) && p.number == i.number
}

fn picked(i: &FlowIssue) -> Value {
    json!({"label": i.label, "url": i.url, "title": i.title, "priority": i.priority, "why": why(i)})
}

enum RepoReadError {
    Transport(anyhow::Error),
    Graphql(anyhow::Error),
}

enum FlowMetaOp {
    SetSkip(String, String),
    SetHeld(String),
    DelSkip(String),
}

struct FlowPlan {
    view: FlowView,
    meta_ops: Vec<FlowMetaOp>,
}

fn matches_opt_in(opt_in: Option<&OptIn>, r: &Read) -> bool {
    let Some(opt) = opt_in else {
        return true;
    };
    if let Some(label) = opt.label.as_deref() {
        let want = label.trim();
        if !r.labels.iter().any(|l| l.eq_ignore_ascii_case(want)) {
            return false;
        }
    }
    if let Some(milestone) = opt.milestone.as_deref() {
        let want = milestone.trim();
        if !r.issue.milestone.as_ref().is_some_and(|m| m.title.eq_ignore_ascii_case(want)) {
            return false;
        }
    }
    true
}

fn plan_flow(
    cfg: &FlowConfig,
    r: Reads,
    tickets: Vec<Ticket>,
    closed: &HashSet<String>,
    done: &BTreeMap<String, Vec<String>>,
    mut meta_get: impl FnMut(&str) -> Result<Option<String>>,
    write: bool,
) -> Result<FlowPlan> {
    let max_tickets = cfg.max_tickets.unwrap_or(0);
    let capacity_repos = r.capacity_repos();
    let aliases = &r.aliases;
    let excluded = |repo: &str| cfg.exclude_repos.iter().any(|ex| ex.eq_ignore_ascii_case(repo));
    let open: HashSet<String> = tickets
        .iter()
        .filter_map(|t| ticket_repo_canonical(&capacity_repos, aliases, t))
        .map(|(repo, n)| node_key(&repo, n))
        .collect();
    let mut running: Vec<FlowTicket> = Vec::new();
    let mut extra_running = 0usize;
    for t in tickets {
        let in_unreadable_project = t.project.as_deref().is_some_and(|p| r.unreadable_projects.contains(p));
        let Some((repo, number)) = ticket_repo_canonical(&capacity_repos, aliases, &t) else {
            let ambiguous_legacy = t.issue.is_none()
                && t.repo.is_none()
                && t.label
                    .rsplit_once('#')
                    .is_some_and(|(name, num)| num.parse::<u64>().is_ok() && capacity_repos.is_ambiguous_short(name));
            let unplaced_under_unavailable = r.has_unavailable_discovery && t.project.is_none();
            if in_unreadable_project || ambiguous_legacy || unplaced_under_unavailable {
                extra_running += 1;
            }
            continue;
        };
        let unmatched_under_unavailable = r.has_unavailable_discovery && t.project.is_none() && !excluded(&repo);
        if (!capacity_repos.contains(&repo) && !in_unreadable_project && !unmatched_under_unavailable)
            || closed.contains(&node_key(&repo, number))
        {
            continue;
        }
        running.push(FlowTicket {
            url: format!("https://github.com/{repo}/issues/{number}"),
            label: t.label,
            repo,
            number,
            thread: t.thread,
            link: t.link,
        });
    }
    let (mut queue, mut blocked, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    let mut meta_ops = Vec::new();
    let mut skip_overlay: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut marked_held = HashSet::new();
    for r in r.reads {
        let has = |want: &str| r.labels.iter().any(|l| l.eq_ignore_ascii_case(want));
        if !matches_opt_in(cfg.opt_in.as_ref(), &r) || open.contains(&node_key(&r.issue.repo, r.issue.number)) {
            continue;
        }
        if r.issue.parent.as_ref().is_some_and(|p| open.contains(&node_key(&p.repo, p.number))) {
            continue;
        }
        let key = skip_key(&r.issue.repo, r.issue.number);
        let mut stored = match skip_overlay.get(&key) {
            Some(v) => v.clone(),
            None => meta_get(&key)?,
        };
        let mut unheld = Vec::new();
        let slug_key = node_key(&r.issue.repo, r.issue.number);
        let label_key = r.issue.label.to_ascii_lowercase();
        for t in done.get(&slug_key).into_iter().chain(done.get(&label_key)).flatten() {
            let held = format!("{HELD}{t}");
            if (!write || !marked_held.contains(&held)) && meta_get(&held)?.is_none() {
                unheld.push(held);
            }
        }
        if !unheld.is_empty() {
            if write {
                skip_overlay.insert(key.clone(), Some(r.issue.key.clone()));
                meta_ops.push(FlowMetaOp::SetSkip(key.clone(), r.issue.key.clone()));
                for held in &unheld {
                    marked_held.insert(held.clone());
                    meta_ops.push(FlowMetaOp::SetHeld(held.clone()));
                }
            }
            stored = Some(r.issue.key.clone());
        }
        let external = has(EXTERNAL);
        let blocked_label = has_blocked_label(&r.labels);
        if external
            || blocked_label
            || !r.open_blockers.is_empty()
            || r.more_blockers
            || !r.unresolved_blockers.is_empty()
        {
            if write && stored.as_ref().is_some_and(|k| *k != r.issue.key) {
                skip_overlay.insert(key.clone(), None);
                meta_ops.push(FlowMetaOp::DelSkip(key));
            }
            blocked.push(BlockedIssue {
                issue: r.issue,
                blockers: r.open_blockers,
                more_blockers: r.more_blockers,
                external,
                blocked_label,
                unresolved: r.unresolved_blockers,
            });
            continue;
        }
        match stored {
            Some(k) if k == r.issue.key => skipped.push(r.issue),
            Some(_) => {
                if write {
                    skip_overlay.insert(key.clone(), None);
                    meta_ops.push(FlowMetaOp::DelSkip(key));
                }
                queue.push(r.issue);
            }
            None => queue.push(r.issue),
        }
    }
    queue.sort_by(queue_order);
    skipped.sort_by(queue_order);
    blocked.sort_by(|a, b| queue_order(&a.issue, &b.issue));
    let view = FlowView {
        enabled: true,
        launch: Some(cfg.launch),
        max_tickets,
        free: max_tickets.saturating_sub(running.len() + extra_running),
        running,
        queue,
        blocked,
        skipped,
        graph: None,
    };
    Ok(FlowPlan { view, meta_ops })
}

struct PickPlan {
    offered: Vec<(FlowPick, String)>,
    retire: Vec<(usize, &'static str)>,
    picks: Vec<FlowIssue>,
}

fn plan_picks(
    launch: Launch,
    queue: &[FlowIssue],
    cards: &[(String, FlowPick)],
    free: usize,
    most: Option<usize>,
) -> PickPlan {
    let ready = |p: &FlowPick| queue.iter().any(|i| same(p, i));
    let mut offered: Vec<(FlowPick, String)> = Vec::new();
    let mut retire = Vec::new();
    for (idx, (id, p)) in cards.iter().enumerate() {
        let note = if launch == Launch::Auto {
            "flow.launch is auto"
        } else if !ready(p) {
            "no longer ready"
        } else if offered.len() >= free {
            "no free slot"
        } else {
            offered.push((p.clone(), id.clone()));
            continue;
        };
        retire.push((idx, note));
    }
    let room = free.saturating_sub(offered.len()).min(most.unwrap_or(usize::MAX));
    let picks: Vec<FlowIssue> =
        queue.iter().filter(|i| !offered.iter().any(|(p, _)| same(p, i))).take(room).cloned().collect();
    PickPlan { offered, retire, picks }
}

struct ResolvedBodyRefs {
    blockers: Vec<(String, u64)>,
    parents: Vec<(String, u64)>,
    subs: Vec<(String, u64)>,
}

fn disp_issue_ref(home_repo: &str, repo: &str, number: u64) -> String {
    if repo.eq_ignore_ascii_case(home_repo) { format!("#{number}") } else { format!("{repo}#{number}") }
}

fn push_read_warning(warnings: &mut Vec<NodeWarning>, w: NodeWarning) {
    if !warnings.contains(&w) {
        warnings.push(w);
    }
}

fn push_unresolved_blocker(unresolved: &mut Vec<String>, raw: String) {
    if !unresolved.iter().any(|x| x.eq_ignore_ascii_case(&raw)) {
        unresolved.push(raw);
    }
}

fn seed_known_issue(known: &mut BTreeMap<String, (IssueRef, Option<&'static str>)>, r: &Read) {
    let key = node_key(&r.issue.repo, r.issue.number);
    let iref = IssueRef {
        repo: r.issue.repo.clone(),
        number: r.issue.number,
        label: r.issue.label.clone(),
        url: r.issue.url.clone(),
        title: Some(r.issue.title.clone()),
    };
    known.insert(key, (iref, r.state));
    for t in r.blockers.iter().chain(r.parent.iter()).chain(r.subs.iter()) {
        let tk = node_key(&t.to.repo, t.to.number);
        known
            .entry(tk)
            .and_modify(|cur| {
                if cur.1.is_none() && t.state.is_some() {
                    cur.1 = t.state;
                }
                if cur.0.title.is_none() && t.to.title.is_some() {
                    cur.0.title = t.to.title.clone();
                }
            })
            .or_insert_with(|| (t.to.clone(), t.state));
    }
}

fn read_primary_parent_key(
    r: &Read,
    aliases: &std::collections::HashMap<String, String>,
    inferred_parents: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(p) = &r.parent {
        return Some(node_key(&canonicalize_repo(aliases, &p.to.repo), p.to.number));
    }
    for pref in &r.body_parsed.parents {
        if let BodyRef::Issue { repo, number } = pref {
            return Some(node_key(&canonicalize_repo(aliases, repo), *number));
        }
    }
    inferred_parents.get(&node_key(&r.issue.repo, r.issue.number)).cloned()
}

fn record_unresolved_ref(
    warnings: &mut Vec<NodeWarning>,
    unresolved_blockers: &mut Vec<String>,
    role: RefRole,
    raw: String,
    reason: Option<String>,
) {
    let role_str = role.label();
    let message = match reason {
        Some(detail) => detail,
        None => format!("Unresolved {role_str} reference \"{raw}\""),
    };
    if role == RefRole::Blocker {
        push_unresolved_blocker(unresolved_blockers, raw.clone());
    }
    push_read_warning(warnings, NodeWarning { kind: WARN_UNRESOLVED, message, raw: Some(raw) });
}

fn build_candidates(
    all_reads: &[&Read],
    aliases: &std::collections::HashMap<String, String>,
    inferred_parents: &BTreeMap<String, String>,
) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in all_reads {
        if r.state == Some("closed") {
            continue;
        }
        let k = node_key(&r.issue.repo, r.issue.number);
        if !seen.insert(k) {
            continue;
        }
        out.push(Candidate {
            repo: r.issue.repo.clone(),
            number: r.issue.number,
            label: r.issue.label.clone(),
            title: r.issue.title.clone(),
            parent_key: read_primary_parent_key(r, aliases, inferred_parents),
            declared_phase: r.body_parsed.declared_phase,
        });
    }
    out
}

impl Wf {
    async fn resolve_body_dependencies(&self, reads: &mut Reads, extra: &mut [Read], all_open: bool) -> Result<()> {
        let has_any_body_refs = reads.reads.iter().chain(extra.iter()).any(|r| {
            !r.body_parsed.blockers.is_empty()
                || !r.body_parsed.parents.is_empty()
                || !r.body_parsed.sub_issues.is_empty()
        });
        if !has_any_body_refs {
            return Ok(());
        }

        // If we read only optIn-labelled issues (all_open == false) and any issue has a symbolic
        // sibling reference, fetch open issues for those repos so symbolic matching sees unlabelled siblings.
        let mut symbolic_pool: Vec<Read> = Vec::new();
        if !all_open {
            let mut need_repos: Vec<String> = Vec::new();
            for r in reads.reads.iter().chain(extra.iter()) {
                if r.body_parsed.has_symbolic() && !need_repos.iter().any(|x| x.eq_ignore_ascii_case(&r.issue.repo)) {
                    need_repos.push(r.issue.repo.clone());
                }
            }
            for repo in need_repos {
                match self.read_repo(&repo, None).await {
                    Ok((full, rs)) => {
                        self.record_github_repo_alias(&repo, &full);
                        symbolic_pool.extend(rs);
                    }
                    Err(RepoReadError::Transport(e)) => return Err(e),
                    Err(RepoReadError::Graphql(e)) => {
                        tracing::warn!("flow: could not read open siblings in {repo} for body resolution: {e:#}");
                    }
                }
            }
            reads.aliases = self.repo_aliases_snapshot();
        }

        let aliases = reads.aliases.clone();
        let mut known: BTreeMap<String, (IssueRef, Option<&'static str>)> = BTreeMap::new();
        for r in reads.reads.iter().chain(extra.iter()).chain(symbolic_pool.iter()) {
            seed_known_issue(&mut known, r);
        }

        // Collect parent links implied by tasklist sub-issues ("- [ ] #N") so sibling phase resolution
        // knows epic membership even when only the parent epic lists its children.
        let mut inferred_parents: BTreeMap<String, (String, u64, String)> = BTreeMap::new();
        for r in reads.reads.iter().chain(extra.iter()).chain(symbolic_pool.iter()) {
            let p_repo = r.issue.repo.clone();
            let p_num = r.issue.number;
            let p_key = node_key(&p_repo, p_num);
            for sref in &r.body_parsed.sub_issues {
                if let BodyRef::Issue { repo, number } = sref {
                    let canon = canonicalize_repo(&aliases, repo);
                    let ck = node_key(&canon, *number);
                    inferred_parents
                        .entry(ck)
                        .and_modify(|cur| {
                            if (p_repo.to_ascii_lowercase(), p_num) < (cur.0.to_ascii_lowercase(), cur.1) {
                                *cur = (p_repo.clone(), p_num, p_key.clone());
                            }
                        })
                        .or_insert_with(|| (p_repo.clone(), p_num, p_key.clone()));
                }
            }
        }
        let mut inferred_parent_keys: BTreeMap<String, String> =
            inferred_parents.iter().map(|(k, (_, _, pk))| (k.clone(), pk.clone())).collect();

        // Pass 1: resolve parent refs (explicit + symbolic) so Pass 2 knows every node's epic.
        let mut pool_refs: Vec<&Read> = reads.reads.iter().chain(extra.iter()).chain(symbolic_pool.iter()).collect();
        let initial_candidates = build_candidates(&pool_refs, &aliases, &inferred_parent_keys);

        let total = reads.reads.len() + extra.len();
        let mut resolved: Vec<ResolvedBodyRefs> = (0..total)
            .map(|_| ResolvedBodyRefs { blockers: Vec::new(), parents: Vec::new(), subs: Vec::new() })
            .collect();

        let mut resolved_parent_keys: BTreeMap<String, String> = inferred_parent_keys.clone();
        for idx in 0..total {
            let r = if idx < reads.reads.len() { &mut reads.reads[idx] } else { &mut extra[idx - reads.reads.len()] };
            let me_key = node_key(&r.issue.repo, r.issue.number);
            let me_repo = r.issue.repo.clone();
            let me_num = r.issue.number;
            let parents_in = r.body_parsed.parents.clone();
            let subs_in = r.body_parsed.sub_issues.clone();
            for pref in parents_in {
                match pref {
                    BodyRef::Issue { repo, number } => {
                        let canon = canonicalize_repo(&aliases, &repo);
                        resolved_parent_keys.entry(me_key.clone()).or_insert_with(|| node_key(&canon, number));
                        resolved[idx].parents.push((canon, number));
                    }
                    BodyRef::Symbolic(raw) => {
                        match flow_body::resolve_symbolic(
                            &raw,
                            RefRole::Parent,
                            &me_repo,
                            me_num,
                            None,
                            &initial_candidates,
                        ) {
                            ResolveOutcome::Resolved(repo, number) => {
                                let canon = canonicalize_repo(&aliases, &repo);
                                resolved_parent_keys.entry(me_key.clone()).or_insert_with(|| node_key(&canon, number));
                                resolved[idx].parents.push((canon, number));
                            }
                            ResolveOutcome::Ambiguous(matches) => {
                                let msg =
                                    format!("Ambiguous parent reference \"{raw}\" (matches {})", matches.join(", "));
                                record_unresolved_ref(
                                    &mut r.warnings,
                                    &mut r.unresolved_blockers,
                                    RefRole::Parent,
                                    raw,
                                    Some(msg),
                                );
                            }
                            ResolveOutcome::NotFound => {
                                record_unresolved_ref(
                                    &mut r.warnings,
                                    &mut r.unresolved_blockers,
                                    RefRole::Parent,
                                    raw,
                                    None,
                                );
                            }
                        }
                    }
                    BodyRef::Malformed(raw) => {
                        record_unresolved_ref(&mut r.warnings, &mut r.unresolved_blockers, RefRole::Parent, raw, None);
                    }
                }
            }
            for sref in subs_in {
                match sref {
                    BodyRef::Issue { repo, number } => {
                        let canon = canonicalize_repo(&aliases, &repo);
                        resolved[idx].subs.push((canon, number));
                    }
                    BodyRef::Symbolic(raw) | BodyRef::Malformed(raw) => {
                        record_unresolved_ref(
                            &mut r.warnings,
                            &mut r.unresolved_blockers,
                            RefRole::SubIssue,
                            raw,
                            None,
                        );
                    }
                }
            }
        }
        inferred_parent_keys.extend(resolved_parent_keys);

        // Pass 2: resolve blocker refs (explicit + symbolic) with full epic membership.
        pool_refs = reads.reads.iter().chain(extra.iter()).chain(symbolic_pool.iter()).collect();
        let candidates = build_candidates(&pool_refs, &aliases, &inferred_parent_keys);
        for idx in 0..total {
            let r = if idx < reads.reads.len() { &mut reads.reads[idx] } else { &mut extra[idx - reads.reads.len()] };
            let me_repo = r.issue.repo.clone();
            let me_num = r.issue.number;
            let me_parent = read_primary_parent_key(r, &aliases, &inferred_parent_keys);
            let blockers_in = r.body_parsed.blockers.clone();
            for bref in blockers_in {
                match bref {
                    BodyRef::Issue { repo, number } => {
                        let canon = canonicalize_repo(&aliases, &repo);
                        resolved[idx].blockers.push((canon, number));
                    }
                    BodyRef::Symbolic(raw) => {
                        match flow_body::resolve_symbolic(
                            &raw,
                            RefRole::Blocker,
                            &me_repo,
                            me_num,
                            me_parent.as_deref(),
                            &candidates,
                        ) {
                            ResolveOutcome::Resolved(repo, number) => {
                                let canon = canonicalize_repo(&aliases, &repo);
                                resolved[idx].blockers.push((canon, number));
                            }
                            ResolveOutcome::Ambiguous(matches) => {
                                let msg =
                                    format!("Ambiguous blocker reference \"{raw}\" (matches {})", matches.join(", "));
                                record_unresolved_ref(
                                    &mut r.warnings,
                                    &mut r.unresolved_blockers,
                                    RefRole::Blocker,
                                    raw,
                                    Some(msg),
                                );
                            }
                            ResolveOutcome::NotFound => {
                                record_unresolved_ref(
                                    &mut r.warnings,
                                    &mut r.unresolved_blockers,
                                    RefRole::Blocker,
                                    raw,
                                    None,
                                );
                            }
                        }
                    }
                    BodyRef::Malformed(raw) => {
                        record_unresolved_ref(&mut r.warnings, &mut r.unresolved_blockers, RefRole::Blocker, raw, None);
                    }
                }
            }
        }

        // Pass 3: batch-fetch any referenced numeric issues whose state/title isn't already known.
        let mut asks_map: BTreeMap<String, (String, u64)> = BTreeMap::new();
        for res in &resolved {
            for (repo, number) in res.blockers.iter().chain(&res.parents).chain(&res.subs) {
                let k = node_key(repo, *number);
                if known.get(&k).is_none_or(|(_, st)| st.is_none()) {
                    asks_map.entry(k).or_insert_with(|| (repo.clone(), *number));
                }
            }
        }
        let asks: Vec<(String, u64)> = asks_map.into_values().collect();
        let mut missing_refs: HashSet<String> = HashSet::new();
        for chunk in asks.chunks(50) {
            let (query, vars) = refs_query(chunk);
            let resp = self.github.graphql(&query, vars).await?;
            if !resp["data"].is_object() {
                bail!(
                    "wf.flow: reading referenced issues failed: {}",
                    errors(&resp).unwrap_or_else(|| "no data".into())
                );
            }
            for (i, (query_repo, number)) in chunk.iter().enumerate() {
                let t = &resp["data"][format!("t{i}")];
                let actual = t["nameWithOwner"].as_str().filter(|s| s.contains('/'));
                if let Some(actual) = actual {
                    self.record_github_repo_alias(query_repo, actual);
                }
                let final_repo = actual.unwrap_or(query_repo);
                let issue = &t["issue"];
                if issue.is_object() {
                    let iref = IssueRef {
                        repo: final_repo.to_string(),
                        number: *number,
                        label: label_of(final_repo, *number),
                        url: issue["url"]
                            .as_str()
                            .map_or_else(|| format!("https://github.com/{final_repo}/issues/{number}"), String::from),
                        title: issue["title"].as_str().map(String::from),
                    };
                    let st = state_of(issue);
                    known.insert(node_key(final_repo, *number), (iref.clone(), st));
                    if !final_repo.eq_ignore_ascii_case(query_repo) {
                        known.insert(node_key(query_repo, *number), (iref, st));
                    }
                } else {
                    missing_refs.insert(node_key(query_repo, *number));
                    missing_refs.insert(node_key(final_repo, *number));
                }
            }
        }
        reads.aliases = self.repo_aliases_snapshot();
        let aliases = reads.aliases.clone();

        // Pass 4: merge resolved body links into each Read, preserving native GitHub precedence.
        let mut sub_parent_fill: BTreeMap<String, IssueRef> = BTreeMap::new();
        for (idx, res) in resolved.into_iter().enumerate() {
            let r = if idx < reads.reads.len() { &mut reads.reads[idx] } else { &mut extra[idx - reads.reads.len()] };
            let home = r.issue.repo.clone();
            let me_ref = IssueRef {
                repo: r.issue.repo.clone(),
                number: r.issue.number,
                label: r.issue.label.clone(),
                url: r.issue.url.clone(),
                title: Some(r.issue.title.clone()),
            };

            let lookup_target = |repo: &str, number: u64| -> Target {
                let canon = canonicalize_repo(&aliases, repo);
                let k = node_key(&canon, number);
                if let Some((iref, st)) = known.get(&k) {
                    Target { to: iref.clone(), state: *st, source: SOURCE_BODY }
                } else {
                    Target {
                        to: IssueRef {
                            label: label_of(&canon, number),
                            url: format!("https://github.com/{canon}/issues/{number}"),
                            title: None,
                            repo: canon,
                            number,
                        },
                        state: None,
                        source: SOURCE_BODY,
                    }
                }
            };

            // Merge blockers
            let mut seen_blockers: HashSet<String> =
                r.blockers.iter().map(|b| node_key(&canonicalize_repo(&aliases, &b.to.repo), b.to.number)).collect();
            for (repo, number) in res.blockers {
                let canon = canonicalize_repo(&aliases, &repo);
                let k = node_key(&canon, number);
                if missing_refs.contains(&k) {
                    let disp = disp_issue_ref(&home, &canon, number);
                    let msg = format!("Unresolved blocker reference {disp} (issue not found)");
                    record_unresolved_ref(
                        &mut r.warnings,
                        &mut r.unresolved_blockers,
                        RefRole::Blocker,
                        disp,
                        Some(msg),
                    );
                    continue;
                }
                if seen_blockers.insert(k) {
                    r.blockers.push(lookup_target(&canon, number));
                }
            }
            // Re-partition open_blockers & unblocked_by from merged blockers
            r.open_blockers.clear();
            r.issue.unblocked_by.clear();
            for b in &r.blockers {
                if b.state == Some("closed") {
                    r.issue.unblocked_by.push(b.to.clone());
                } else {
                    r.open_blockers.push(b.to.clone());
                }
            }

            // Merge parents
            for (repo, number) in res.parents {
                let canon = canonicalize_repo(&aliases, &repo);
                let k = node_key(&canon, number);
                if missing_refs.contains(&k) {
                    let disp = disp_issue_ref(&home, &canon, number);
                    let msg = format!("Unresolved parent reference {disp} (issue not found)");
                    record_unresolved_ref(
                        &mut r.warnings,
                        &mut r.unresolved_blockers,
                        RefRole::Parent,
                        disp,
                        Some(msg),
                    );
                    continue;
                }
                let tgt = lookup_target(&canon, number);
                match &r.parent {
                    None => {
                        r.issue.parent = Some(tgt.to.clone());
                        r.parent = Some(tgt);
                    }
                    Some(existing) if node_key(&existing.to.repo, existing.to.number) == k => {}
                    Some(_) => {
                        if !r.extra_parents.iter().any(|p| node_key(&p.to.repo, p.to.number) == k) {
                            r.extra_parents.push(tgt);
                        }
                    }
                }
            }

            // Merge tasklist sub-issues
            let mut seen_subs: HashSet<String> =
                r.subs.iter().map(|s| node_key(&canonicalize_repo(&aliases, &s.to.repo), s.to.number)).collect();
            for (repo, number) in res.subs {
                let canon = canonicalize_repo(&aliases, &repo);
                let k = node_key(&canon, number);
                if missing_refs.contains(&k) {
                    let disp = disp_issue_ref(&home, &canon, number);
                    let msg = format!("Unresolved sub-issue reference {disp} (issue not found)");
                    record_unresolved_ref(
                        &mut r.warnings,
                        &mut r.unresolved_blockers,
                        RefRole::SubIssue,
                        disp,
                        Some(msg),
                    );
                    continue;
                }
                if seen_subs.insert(k.clone()) {
                    r.subs.push(lookup_target(&canon, number));
                }
                sub_parent_fill
                    .entry(k)
                    .and_modify(|cur| {
                        if (me_ref.repo.to_ascii_lowercase(), me_ref.number)
                            < (cur.repo.to_ascii_lowercase(), cur.number)
                        {
                            *cur = me_ref.clone();
                        }
                    })
                    .or_insert_with(|| me_ref.clone());
            }

            r.unresolved_blockers.sort_by_key(|u| u.to_ascii_lowercase());
            r.warnings.sort_by(|a, b| {
                warn_kind_rank(a.kind).cmp(&warn_kind_rank(b.kind)).then_with(|| a.message.cmp(&b.message))
            });
            r.issue.key = compute_skip_key(&r.labels, &r.blockers, r.more_blockers, &r.unresolved_blockers);
        }

        // Propagate parent ref onto child reads that only had a parent via an epic's tasklist.
        for r in reads.reads.iter_mut().chain(extra.iter_mut()) {
            if r.issue.parent.is_none()
                && let Some(p) = sub_parent_fill.get(&node_key(&r.issue.repo, r.issue.number))
            {
                r.issue.parent = Some(p.clone());
            }
        }

        Ok(())
    }

    fn open_tickets(&self) -> Result<Vec<Ticket>> {
        let mut out: Vec<Ticket> = self
            .store
            .children(BOARD, true)?
            .into_iter()
            .map(|l| {
                let live = self.live_of(&l.child);
                let link = l.link.clone().or_else(|| live.as_ref()?["link"].as_str().map(String::from));
                let project =
                    l.project.clone().or_else(|| live.as_ref().and_then(|v| v["project"].as_str().map(String::from)));
                let v = self.node_view(&l);
                let repo = v["repo"].as_str().map(String::from);
                let status = v["status"].as_str().unwrap_or("unknown").to_string();
                Ticket { label: l.label, thread: Some(l.child), link, repo, project, issue: l.identity.issue, status }
            })
            .collect();
        let seen: HashSet<String> = out.iter().map(|t| t.label.to_ascii_lowercase()).collect();
        out.extend(
            self.store
                .pending_all()?
                .into_iter()
                .filter(|p| p.parent == BOARD && !seen.contains(&p.label.to_ascii_lowercase()))
                .map(|p| {
                    let live = p.thread.as_deref().and_then(|t| self.live_of(t));
                    let status = if p.thread.is_none() { "launching" } else { chip(live.as_ref()) };
                    let project = p
                        .project
                        .clone()
                        .or_else(|| live.as_ref().and_then(|v| v["project"].as_str().map(String::from)));
                    let repo =
                        self.place(project.as_deref(), &live.unwrap_or(Value::Null)).0.as_str().map(String::from);
                    Ticket {
                        label: p.label,
                        thread: p.thread,
                        link: p.link,
                        repo,
                        project,
                        issue: p.identity.issue,
                        status: status.to_string(),
                    }
                }),
        );
        Ok(out)
    }

    async fn read_repo(
        &self,
        repo: &str,
        label: Option<&str>,
    ) -> std::result::Result<(String, Vec<Read>), RepoReadError> {
        let (owner, name) =
            repo.split_once('/').ok_or_else(|| RepoReadError::Graphql(anyhow!("flow: {repo:?} is not owner/name")))?;
        let mut after: Option<String> = None;
        let mut seen_cursors: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        let mut full = repo.to_string();
        let query = if label.is_some() { FLOW_ISSUES } else { FLOW_OPEN_ISSUES };
        loop {
            let vars = match label {
                Some(label) => json!({"owner": owner, "name": name, "label": label, "after": after}),
                None => json!({"owner": owner, "name": name, "after": after}),
            };
            let resp = self.github.graphql(query, vars).await.map_err(RepoReadError::Transport)?;
            if let Some(e) = errors(&resp) {
                return Err(RepoReadError::Graphql(anyhow!("wf.flow: reading {repo}'s issues failed: {e}")));
            }
            let r = &resp["data"]["repository"];
            if r.is_null() {
                return Err(RepoReadError::Graphql(anyhow!("wf.flow: GitHub returned no repository {repo}")));
            }
            if let Some(n) = r["nameWithOwner"].as_str() {
                full = n.to_string();
            }
            for n in r["issues"]["nodes"].as_array().into_iter().flatten() {
                out.push(parse_node(&full, n).map_err(RepoReadError::Graphql)?);
            }
            let page = &r["issues"]["pageInfo"];
            match page["endCursor"].as_str() {
                Some(c) if page["hasNextPage"] == true => {
                    if !seen_cursors.insert(c.to_string()) {
                        return Err(RepoReadError::Graphql(anyhow!(
                            "wf.flow: GitHub repeated page cursor {c:?} while reading {repo}"
                        )));
                    }
                    after = Some(c.to_string());
                }
                _ => return Ok((full, out)),
            }
        }
    }

    async fn resolve_repos(&self, cfg: Option<&FlowConfig>) -> Result<ResolvedRepos> {
        let mut resolved = match cfg.and_then(|c| c.repos.as_ref()) {
            Some(explicit) => ResolvedRepos {
                repos: explicit
                    .iter()
                    .map(|r| ResolvedRepo { repo: r.clone(), explicit: true, project_ids: Vec::new() })
                    .collect(),
                unavailable_projects: Vec::new(),
            },
            None => {
                let discovered = self.discover_github_repos().await?;
                ResolvedRepos {
                    repos: discovered
                        .repos
                        .into_iter()
                        .map(|(repo, project_ids)| ResolvedRepo { repo, explicit: false, project_ids })
                        .collect(),
                    unavailable_projects: discovered.unavailable_projects,
                }
            }
        };
        if let Some(cfg) = cfg {
            for repo in &cfg.include_repos {
                if let Some(slot) = resolved.repos.iter_mut().find(|r| r.repo.eq_ignore_ascii_case(repo)) {
                    slot.explicit = true;
                } else {
                    resolved.repos.push(ResolvedRepo { repo: repo.clone(), explicit: true, project_ids: Vec::new() });
                }
            }
            if !cfg.exclude_repos.is_empty() {
                resolved.repos.retain(|r| !cfg.exclude_repos.iter().any(|ex| ex.eq_ignore_ascii_case(&r.repo)));
            }
        }
        Ok(resolved)
    }

    async fn flow_reads(&self, cfg: Option<&FlowConfig>, all_open: bool) -> Result<Reads> {
        let label = (!all_open)
            .then(|| {
                cfg.and_then(|c| c.opt_in.as_ref())
                    .and_then(|o| o.label.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .flatten();
        let is_excluded =
            |name: &str| cfg.is_some_and(|c| c.exclude_repos.iter().any(|ex| ex.eq_ignore_ascii_case(name)));
        let ResolvedRepos { repos, unavailable_projects } = self.resolve_repos(cfg).await?;
        let has_unavailable_discovery = !unavailable_projects.is_empty();
        let mut reads = Vec::new();
        let mut seen_issues: HashSet<String> = HashSet::new();
        let mut covered = CoveredRepos::default();
        let mut unreadable = CoveredRepos::default();
        let mut unreadable_projects: HashSet<String> = unavailable_projects.into_iter().collect();
        let mut succeeded = 0usize;
        let mut first_discovered_err: Option<anyhow::Error> = None;
        for ResolvedRepo { repo, explicit, project_ids } in repos {
            match self.read_repo(&repo, label).await {
                Ok((full, rs)) => {
                    self.record_github_repo_alias(&repo, &full);
                    if is_excluded(&full) {
                        continue;
                    }
                    succeeded += 1;
                    covered.insert(full);
                    for r in rs {
                        if seen_issues.insert(node_key(&r.issue.repo, r.issue.number)) {
                            reads.push(r);
                        }
                    }
                }
                Err(RepoReadError::Transport(e)) => return Err(e),
                Err(RepoReadError::Graphql(e)) if explicit => return Err(e),
                Err(RepoReadError::Graphql(e)) => {
                    tracing::warn!("flow: skipping discovered repo {repo}: {e:#}");
                    unreadable.insert(repo.clone());
                    let canonical = self.canonical_github_repo(&repo);
                    if !canonical.eq_ignore_ascii_case(&repo) {
                        unreadable.insert(canonical);
                    }
                    unreadable_projects.extend(project_ids);
                    if first_discovered_err.is_none() {
                        first_discovered_err = Some(e);
                    }
                }
            }
        }
        if succeeded == 0
            && let Some(e) = first_discovered_err
        {
            return Err(e);
        }
        let aliases = self.repo_aliases_snapshot();
        Ok(Reads { reads, covered, unreadable, unreadable_projects, has_unavailable_discovery, aliases })
    }

    async fn ticket_reads(&self, r: &Reads, tickets: &[Ticket], full: bool) -> Result<(HashSet<String>, Vec<Read>)> {
        let aliases = self.repo_aliases_snapshot();
        let open: HashSet<String> = r.reads.iter().map(|x| node_key(&x.issue.repo, x.issue.number)).collect();
        let mut asks: Vec<(String, (String, u64))> = Vec::new();
        let mut seen_asks: HashSet<String> = HashSet::new();
        for t in tickets {
            let Some((raw_repo, number)) = ticket_repo(&r.covered, t) else { continue };
            let canon_repo = canonicalize_repo(&aliases, &raw_repo);
            let canon_key = node_key(&canon_repo, number);
            if open.contains(&canon_key) {
                continue;
            }
            let unreadable = r.unreadable.contains(&raw_repo) || r.unreadable.contains(&canon_repo);
            let query_repo = if r.covered.contains(&canon_repo) {
                canon_repo.clone()
            } else if full || (t.issue.is_some() && !unreadable) {
                raw_repo.clone()
            } else {
                continue;
            };
            let ask_key = node_key(&query_repo, number);
            if seen_asks.insert(ask_key) {
                asks.push((raw_repo, (query_repo, number)));
            }
        }
        let mut closed = HashSet::new();
        let mut reads = Vec::new();
        for chunk in asks.chunks(50) {
            let issues: Vec<(String, u64)> = chunk.iter().map(|(_, i)| i.clone()).collect();
            let (query, vars) = tickets_query(&issues, full);
            let resp = self.github.graphql(&query, vars).await?;
            if !resp["data"].is_object() {
                bail!(
                    "wf.flow: reading the running tickets' issues failed: {}",
                    errors(&resp).unwrap_or_else(|| "no data".into())
                );
            }
            for (i, (raw_repo, (query_repo, number))) in chunk.iter().enumerate() {
                let t = &resp["data"][format!("t{i}")];
                let actual = t["nameWithOwner"].as_str().filter(|s| s.contains('/'));
                if let Some(actual) = actual {
                    self.record_github_repo_alias(raw_repo, actual);
                    if !query_repo.eq_ignore_ascii_case(raw_repo) {
                        self.record_github_repo_alias(query_repo, actual);
                    }
                }
                let final_repo = actual.unwrap_or(query_repo);
                let issue = &t["issue"];
                if issue["state"] == "CLOSED" {
                    closed.insert(node_key(final_repo, *number));
                }
                if full && issue.is_object() {
                    reads.push(parse_node(final_repo, issue)?);
                }
            }
        }
        Ok((closed, reads))
    }

    pub async fn flow(&self, with_graph: bool) -> Result<Value> {
        let cfg = self.config().flow.clone();
        let queue_enabled = cfg.as_ref().is_some_and(FlowConfig::queue_enabled);
        if !queue_enabled && !with_graph {
            let off = FlowView {
                enabled: false,
                launch: None,
                max_tickets: 0,
                free: 0,
                running: Vec::new(),
                queue: Vec::new(),
                blocked: Vec::new(),
                skipped: Vec::new(),
                graph: None,
            };
            return Ok(serde_json::to_value(off)?);
        }
        let mut reads = self.flow_reads(cfg.as_ref(), with_graph).await?;
        let tickets = self.open_tickets()?;
        let (closed, mut extra) = self.ticket_reads(&reads, &tickets, with_graph).await?;
        let aliases = self.repo_aliases_snapshot();
        reads.aliases = aliases;
        self.resolve_body_dependencies(&mut reads, &mut extra, with_graph).await?;
        let aliases = reads.aliases.clone();
        let covered = reads.covered.clone();
        let mut all = reads.reads.clone();
        all.extend(extra);
        let mut view = match cfg.as_ref().filter(|c| c.queue_enabled()) {
            Some(c) => self.flow_build(c, reads, tickets.clone(), &closed, false)?,
            None => FlowView {
                enabled: false,
                launch: None,
                max_tickets: 0,
                free: 0,
                running: Vec::new(),
                queue: Vec::new(),
                blocked: Vec::new(),
                skipped: Vec::new(),
                graph: None,
            },
        };
        if !with_graph {
            return Ok(serde_json::to_value(view)?);
        }
        let mut lists: BTreeMap<String, &'static str> = BTreeMap::new();
        for (name, list) in [("queue", &view.queue), ("skipped", &view.skipped)] {
            lists.extend(list.iter().map(|i| (node_key(&i.repo, i.number), name)));
        }
        lists.extend(view.blocked.iter().map(|b| (node_key(&b.issue.repo, b.issue.number), "blocked")));
        view.graph = Some(graph(&covered, &aliases, all, &tickets, &lists));
        Ok(serde_json::to_value(view)?)
    }

    fn flow_build(
        &self,
        cfg: &FlowConfig,
        r: Reads,
        tickets: Vec<Ticket>,
        closed: &HashSet<String>,
        write: bool,
    ) -> Result<FlowView> {
        let mut done: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for l in self.store.children(BOARD, false)?.into_iter().filter(|l| l.closed) {
            let key = match l.identity.issue.as_deref() {
                Some(issue) => {
                    let Some(gh) = crate::board::parse_github_issue_only(issue) else { continue };
                    let canon = canonicalize_repo(&r.aliases, &gh.scope);
                    format!("{}#{}", canon.to_ascii_lowercase(), gh.key)
                }
                None => l.label.to_ascii_lowercase(),
            };
            done.entry(key).or_default().push(l.child);
        }
        let plan = plan_flow(cfg, r, tickets, closed, &done, |k| self.store.meta_get(k), write)?;
        if write {
            for op in plan.meta_ops {
                match op {
                    FlowMetaOp::SetSkip(k, v) => self.store.meta_set(&k, &v)?,
                    FlowMetaOp::SetHeld(k) => self.store.meta_set(&k, &self.now())?,
                    FlowMetaOp::DelSkip(k) => {
                        self.store.meta_del(&k)?;
                    }
                }
            }
        }
        Ok(plan.view)
    }

    fn flow_cards(&self) -> Result<Vec<(Card, FlowPick)>> {
        Ok(self.cards()?.into_iter().filter_map(|c| c.flow.clone().map(|p| (c, p))).collect())
    }

    fn retire_flow_card(&self, c: &Card, note: &str) -> Result<bool> {
        self.store.waiting_resolve_seq(BOARD, c.seq, Some(note), &self.now())
    }

    fn flow_card(&self, i: &FlowIssue) -> Result<String> {
        let pick = FlowPick {
            repo: i.repo.clone(),
            number: i.number,
            label: i.label.clone(),
            url: i.url.clone(),
            title: i.title.clone(),
            key: i.key.clone(),
        };
        let body = format!("Launch {} next? ({}), here: {}", i.label, why(i), i.url);
        let data = json!({"kind": "flow", "ticket": null, "handle": null, "options": [LAUNCH, SKIP], "pr": null,
                          "flow": pick});
        let id = self.alloc_waiting(BOARD, &card_text(&body), Some(i.label.clone()), None, Some(&data))?;
        self.changed();
        Ok(id)
    }

    fn flow_event(&self, body: &str) {
        match self.store.msg_new(BOARD, BOARD, "event", body, &self.now()) {
            Ok(_) => {
                self.changed();
            }
            Err(e) => tracing::warn!("flow: recording {body:?} failed: {e:#}"),
        }
    }

    pub(crate) fn flow_soon(&self) {
        self.flow_due.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) async fn flow_if_due(&self) {
        if !self.flow_due.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        match self.flow_pick(None).await {
            Ok(v) if v["picked"].as_array().is_some_and(|p| !p.is_empty()) => tracing::info!(pick = %v, "flow pick"),
            Ok(_) => {}
            Err(e) => tracing::warn!("flow pick failed: {e:#}"),
        }
    }

    pub async fn flow_next(&self) -> Result<Value> {
        self.flow_pick(Some(1)).await
    }

    fn retire_all_flow_cards(&self, note: &str) -> Result<Vec<String>> {
        let mut retired = Vec::new();
        for (c, _) in self.flow_cards()? {
            if self.retire_flow_card(&c, note)? {
                retired.push(c.id);
            }
        }
        if !retired.is_empty() {
            self.changed();
        }
        Ok(retired)
    }

    pub(crate) async fn flow_pick(&self, most: Option<usize>) -> Result<Value> {
        let Some(cfg) = self.config().flow.clone().filter(FlowConfig::queue_enabled) else {
            let _pick = self.flow_lock.lock().await;
            let _g = self.lock.lock().await;
            self.retire_all_flow_cards("flow.launch is off")?;
            return Ok(json!({"launch": null, "free": 0, "picked": [], "offered": [],
                             "reason": "the flow is off: the config has no flow launch settings"}));
        };
        let _pick = self.flow_lock.lock().await;
        if cfg.launch == Launch::Off {
            let _g = self.lock.lock().await;
            self.retire_all_flow_cards("flow.launch is off")?;
            return Ok(json!({"launch": cfg.launch, "free": 0, "picked": [], "offered": [],
                             "reason": "flow.launch is off, so nothing launches"}));
        }
        let mut reads = self.flow_reads(Some(&cfg), false).await?;
        let (closed, _) = self.ticket_reads(&reads, &self.open_tickets()?, false).await?;
        reads.aliases = self.repo_aliases_snapshot();
        self.resolve_body_dependencies(&mut reads, &mut [], false).await?;
        let aliases = reads.aliases.clone();
        let g = self.lock.lock().await;
        let covered = reads.capacity_repos();
        let view = self.flow_build(&cfg, reads, self.open_tickets()?, &closed, true)?;
        let flow_cards = self.flow_cards()?;
        let card_refs: Vec<(String, FlowPick)> = flow_cards.iter().map(|(c, p)| (c.id.clone(), p.clone())).collect();
        let PickPlan { offered, retire, picks } = plan_picks(cfg.launch, &view.queue, &card_refs, view.free, most);
        let mut retired = Vec::new();
        for (idx, note) in retire {
            let (c, _) = &flow_cards[idx];
            if self.retire_flow_card(c, note)? {
                retired.push(c.id.clone());
            }
        }
        if !retired.is_empty() {
            self.changed();
        }
        let mut out = json!({"launch": cfg.launch, "free": view.free, "retired": retired,
            "offered": offered.iter().map(|(p, id)| json!({"label": p.label, "card": id})).collect::<Vec<_>>()});
        if picks.is_empty() {
            out["picked"] = json!([]);
            out["reason"] =
                json!(if view.free <= offered.len() { "no free ticket slot" } else { "no ready issue to launch" });
            return Ok(out);
        }
        let mut done = Vec::new();
        if cfg.launch == Launch::Ask {
            for i in &picks {
                let mut v = picked(i);
                v["card"] = json!(self.flow_card(i)?);
                done.push(v);
            }
            out["picked"] = json!(done);
            return Ok(out);
        }
        drop(g);
        for i in &picks {
            let mut v = picked(i);
            if let Some(p) = &i.parent {
                let parent = node_key(&p.repo, p.number);
                let open = self.open_tickets()?;
                if open
                    .iter()
                    .filter_map(|t| ticket_repo_canonical(&covered, &aliases, t))
                    .any(|(r, n)| node_key(&r, n) == parent)
                {
                    v["reason"] = json!(format!("its parent {} has a ticket now", p.label));
                    done.push(v);
                    continue;
                }
            }
            let title = Some(i.title.clone()).filter(|t| !t.trim().is_empty());
            let args = LaunchArgs {
                issue: i.url.clone(),
                parent_issue: i.parent.as_ref().map(|p| p.url.clone()),
                part: None,
                project: None,
                title,
                model: None,
                effort: None,
            };
            match self.launch_unlocked(args).await {
                Ok(r) if r["ok"] == true => {
                    self.flow_event(&format!("Flow launched {} ({})", i.label, why(i)));
                    v["launched"] = r;
                }
                Ok(r) if r["retryable"] == true => {
                    let e = r["error"].as_str().unwrap_or("transient launch failure");
                    self.flow_event(&format!("Flow deferred {} for retry: {e}", i.label));
                    v["launched"] = r;
                }
                Ok(r) => {
                    self.store.meta_set(&skip_key(&i.repo, i.number), &i.key)?;
                    let e = r["error"].as_str().unwrap_or("launch failed");
                    self.flow_event(&format!(
                        "Flow could not launch {}, so it skips it until its labels or blockers change: {e}",
                        i.label
                    ));
                    v["launched"] = r;
                }
                Err(e) => {
                    self.store.meta_set(&skip_key(&i.repo, i.number), &i.key)?;
                    self.flow_event(&format!(
                        "Flow could not launch {}, so it skips it until its labels or blockers change: {e:#}",
                        i.label
                    ));
                    v["error"] = json!(format!("{e:#}"));
                }
            }
            done.push(v);
        }
        out["picked"] = json!(done);
        Ok(out)
    }

    pub(crate) async fn answer_flow(&self, card: &Card, pick: &FlowPick, text: &str) -> Result<Value> {
        let word = text.trim().to_ascii_lowercase();
        if word != LAUNCH && word != SKIP {
            bail!("card {} takes \"{LAUNCH}\" or \"{SKIP}\"", card.id);
        }
        let held = self.flow_lock.lock().await;
        let cfg = self.config().flow.clone().filter(|c| c.queue_enabled() && c.launch != Launch::Off);
        let fresh = match &cfg {
            Some(c) => {
                let mut reads = self.flow_reads(Some(c), false).await?;
                let (closed, _) = self.ticket_reads(&reads, &self.open_tickets()?, false).await?;
                reads.aliases = self.repo_aliases_snapshot();
                self.resolve_body_dependencies(&mut reads, &mut [], false).await?;
                Some((reads, closed))
            }
            None => None,
        };
        let seen = fresh.as_ref().and_then(|(r, _)| {
            r.reads
                .iter()
                .find(|x| x.issue.repo.eq_ignore_ascii_case(&pick.repo) && x.issue.number == pick.number)
                .map(|x| x.issue.key.clone())
        });
        let g = self.lock.lock().await;
        let view = match (&cfg, fresh) {
            (Some(c), Some((reads, closed))) => Some(self.flow_build(c, reads, self.open_tickets()?, &closed, true)?),
            _ => None,
        };
        let current = view.as_ref().and_then(|v| v.queue.iter().find(|i| same(pick, i))).cloned();
        let holds = match &view {
            Some(v) => self
                .flow_cards()?
                .into_iter()
                .filter(|(_, p)| v.queue.iter().any(|i| same(p, i)))
                .take(v.free)
                .any(|(c, _)| c.seq == card.seq),
            None => false,
        };
        let refused = match (&view, &current) {
            _ if word == SKIP => None,
            (None, _) => Some("the flow is off".to_string()),
            (_, None) => Some(format!("{} is no longer ready", pick.label)),
            _ if !holds => Some("no ticket slot is free".to_string()),
            _ => None,
        };
        let note = match &refused {
            Some(why) => format!("not launched: {why}"),
            None => format!("answered: {word}"),
        };
        if !self.store.waiting_resolve_seq(BOARD, card.seq, Some(&note), &self.now())? {
            bail!("no open board card {:?}; it was already answered", card.id);
        }
        self.changed();
        if let Some(why) = refused {
            bail!("card {} is resolved and nothing launched: {why}", card.id);
        }
        if let (true, Some(i)) = (word == LAUNCH, current) {
            drop(g);
            let title = Some(i.title.clone()).filter(|t| !t.trim().is_empty());
            let args = LaunchArgs {
                issue: i.url.clone(),
                parent_issue: i.parent.as_ref().map(|p| p.url.clone()),
                part: None,
                project: None,
                title,
                model: None,
                effort: None,
            };
            let launched = self.launch_unlocked(args).await?;
            return Ok(json!({"card": card.id, "launched": launched}));
        }
        let key = seen.as_deref().unwrap_or(&pick.key);
        self.store.meta_set(&skip_key(&pick.repo, pick.number), key)?;
        drop(g);
        drop(held);
        let next = match self.flow_pick(None).await {
            Ok(v) => v,
            Err(e) => json!({"error": format!("{e:#}")}),
        };
        Ok(json!({"card": card.id, "skipped": pick.label, "next": next}))
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use crate::config::OptIn;
    use arbitrary::Unstructured;
    use arbtest::arbtest;

    const REPOS: &[&str] = &["acme/widgets", "acme/tools", "acme/gadgets"];
    const MIXED_CASE_REPOS: &[&str] = &["acme/widgets", "Acme/Widgets", "acme/tools", "ACME/TOOLS", "acme/gadgets"];
    const DUE_DATES: &[Option<&str>] =
        &[None, None, Some("2026-11-01T00:00:00Z"), Some("2026-11-15T00:00:00Z"), Some("2026-12-01T00:00:00Z")];
    const CREATED_AT: &[&str] =
        &["2026-10-01T09:00:00Z", "2026-10-02T09:00:00Z", "2026-10-03T09:00:00Z", "2026-10-04T09:00:00Z"];
    const PRIORITY_OPTS: &[Option<&'static str>] = &[Some("high"), Some("medium"), Some("low"), None, Some("urgent")];

    fn prop<P>(budget_ms: u64, property: P)
    where
        P: FnMut(&mut Unstructured<'_>) -> arbitrary::Result<()>,
    {
        let mut test = arbtest(property).size_min(32).size_max(4096);
        if std::env::var_os("ARBTEST_BUDGET_MS").is_none() {
            test = test.budget_ms(budget_ms);
        }
        test.run();
    }

    fn shuffle<T>(u: &mut Unstructured<'_>, items: &mut [T]) -> arbitrary::Result<()> {
        for i in (1..items.len()).rev() {
            let j = u.int_in_range(0..=i)?;
            items.swap(i, j);
        }
        Ok(())
    }

    fn issue_ref_for(repo: &str, number: u64) -> IssueRef {
        IssueRef {
            repo: repo.to_string(),
            number,
            label: label_of(repo, number),
            url: format!("https://github.com/{repo}/issues/{number}"),
            title: Some(format!("Issue {number}")),
        }
    }

    fn gen_issue(u: &mut Unstructured<'_>, repo: &str, number: u64) -> arbitrary::Result<FlowIssue> {
        let prio = *u.choose(PRIORITY_OPTS)?;
        let milestone = match u.int_in_range(0..=3u8)? {
            0 => None,
            _ => Some(Milestone {
                title: format!("m{}", u.int_in_range(0..=3u8)?),
                due_on: (*u.choose(DUE_DATES)?).map(String::from),
            }),
        };
        let created_at = (*u.choose(CREATED_AT)?).to_string();
        let parent =
            if u.int_in_range(0..=3u8)? == 0 { Some(issue_ref_for(repo, u.int_in_range(1..=20u64)?)) } else { None };
        Ok(FlowIssue {
            repo: repo.to_string(),
            number,
            label: label_of(repo, number),
            url: format!("https://github.com/{repo}/issues/{number}"),
            title: format!("T{}", u.int_in_range(0..=99u8)?),
            priority: prio,
            milestone,
            created_at,
            parent,
            unblocked_by: Vec::new(),
            key: format!("k-{repo}-{number}-{}", u.int_in_range(0..=9u8)?),
        })
    }

    fn gen_distinct_issues(u: &mut Unstructured<'_>, max_n: usize) -> arbitrary::Result<Vec<FlowIssue>> {
        let n = u.int_in_range(0..=max_n)?;
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let repo = *u.choose(MIXED_CASE_REPOS)?;
            let number = u.int_in_range(1..=40u64)?;
            if seen.insert(node_key(repo, number)) {
                out.push(gen_issue(u, repo, number)?);
            }
        }
        Ok(out)
    }

    fn issue_ids(list: &[FlowIssue]) -> Vec<(String, u64)> {
        list.iter().map(|i| (i.repo.clone(), i.number)).collect()
    }

    fn blocked_ids(list: &[BlockedIssue]) -> Vec<(String, u64, bool, bool, bool)> {
        list.iter()
            .map(|b| (b.issue.repo.clone(), b.issue.number, b.external, b.more_blockers, b.blocked_label))
            .collect()
    }

    fn assert_sorted_by_queue_order(list: &[FlowIssue]) {
        for pair in list.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert_ne!(queue_order(a, b), Ordering::Greater, "out of order: {a:?} > {b:?}");
            let (ra, rb) = (rank(a.priority), rank(b.priority));
            assert!(ra <= rb);
            if ra == rb {
                match (due(a), due(b)) {
                    (None, Some(_)) => panic!("missing due_on preceded dated milestone: {a:?} before {b:?}"),
                    (Some(da), Some(db)) => assert!(da <= db),
                    _ => {}
                }
                if due(a) == due(b) {
                    assert!(a.created_at <= b.created_at);
                    if a.created_at == b.created_at {
                        let (la, lb) = (a.repo.to_ascii_lowercase(), b.repo.to_ascii_lowercase());
                        assert!(la <= lb);
                        if la == lb {
                            assert!(a.number <= b.number);
                        }
                    }
                }
            }
        }
    }

    fn gen_any_issue(u: &mut Unstructured<'_>, max_num: u64) -> arbitrary::Result<FlowIssue> {
        let repo = *u.choose(MIXED_CASE_REPOS)?;
        let number = u.int_in_range(1..=max_num)?;
        gen_issue(u, repo, number)
    }

    #[test]
    fn queue_order_is_a_strict_weak_order_invariant_to_permutations() {
        prop(40, |u| {
            // Strict weak ordering axioms on arbitrary triples.
            let a = gen_any_issue(u, 10)?;
            let b = gen_any_issue(u, 10)?;
            let c = gen_any_issue(u, 10)?;

            assert_eq!(queue_order(&a, &a), Ordering::Equal);
            assert_eq!(queue_order(&a, &b), queue_order(&b, &a).reverse());
            let (ab, bc, ac) = (queue_order(&a, &b), queue_order(&b, &c), queue_order(&a, &c));
            if ab != Ordering::Greater && bc != Ordering::Greater {
                assert_ne!(ac, Ordering::Greater, "transitivity of <= failed");
                if ab == Ordering::Less || bc == Ordering::Less {
                    assert_eq!(ac, Ordering::Less, "strict transitivity failed");
                } else {
                    assert_eq!(ac, Ordering::Equal, "equivalence transitivity failed");
                }
            }
            if node_key(&a.repo, a.number) != node_key(&b.repo, b.number) {
                assert_ne!(ab, Ordering::Equal, "distinct issues must not tie");
            }

            // Non-key fields never affect ordering.
            let mut a_mut = a.clone();
            a_mut.title = "completely-different-title".into();
            a_mut.label = "other#999".into();
            a_mut.url = "https://example.invalid/x".into();
            a_mut.parent = Some(issue_ref_for("acme/other", 77));
            a_mut.unblocked_by = vec![issue_ref_for("acme/other", 88)];
            a_mut.key = "different-key".into();
            if let Some(ref mut m) = a_mut.milestone {
                m.title = "renamed-milestone".into();
            }
            assert_eq!(queue_order(&a, &a_mut), Ordering::Equal);
            assert_eq!(queue_order(&a_mut, &b), ab);

            // Permutation invariance on unordered distinct issue inputs.
            let issues = gen_distinct_issues(u, 16)?;
            let mut sorted_a = issues.clone();
            sorted_a.sort_by(queue_order);
            for _ in 0..2 {
                let mut permuted = issues.clone();
                shuffle(u, &mut permuted)?;
                permuted.sort_by(queue_order);
                assert_eq!(issue_ids(&permuted), issue_ids(&sorted_a));
            }

            // Label-permutation invariance of `priority(&labels)`.
            let label_pool = [
                "flow",
                "Priority: HIGH",
                "priority:  medium ",
                "priority:low",
                "priority: unknown",
                "blocked: external",
                "bug",
            ];
            let label_n = u.int_in_range(0..=7usize)?;
            let mut labels: Vec<String> = (0..label_n)
                .map(|_| u.choose(&label_pool).map(|s| (*s).to_string()))
                .collect::<arbitrary::Result<_>>()?;
            let p_orig = priority(&labels);
            shuffle(u, &mut labels)?;
            assert_eq!(priority(&labels), p_orig);

            // Stability on equal comparison keys preserves original relative order.
            let dup_n = u.int_in_range(2..=5usize)?;
            let mut dups: Vec<FlowIssue> = (0..dup_n)
                .map(|idx| {
                    let mut x = a.clone();
                    x.title = format!("tag-{idx}");
                    x
                })
                .collect();
            dups.push(b);
            dups.sort_by(queue_order);
            let tags: Vec<&str> = dups.iter().filter_map(|x| x.title.strip_prefix("tag-")).collect();
            let expected_tags: Vec<String> = (0..dup_n).map(|idx| idx.to_string()).collect();
            let expected_refs: Vec<&str> = expected_tags.iter().map(String::as_str).collect();
            assert_eq!(tags, expected_refs);
            Ok(())
        });
    }

    #[test]
    fn queue_order_enforces_stable_priority_due_age_and_tie_hierarchy() {
        prop(40, |u| {
            let mut issues = gen_distinct_issues(u, 18)?;
            issues.sort_by(queue_order);
            assert_sorted_by_queue_order(&issues);
            for pair in issues.windows(2) {
                assert_eq!(queue_order(&pair[0], &pair[1]), Ordering::Less);
            }

            // Explicit pairwise dominance across hierarchy levels.
            let mut hi = gen_any_issue(u, 50)?;
            let mut lo = gen_any_issue(u, 50)?;
            let p_pair = *u.choose(&[
                (Some("high"), Some("medium")),
                (Some("high"), Some("low")),
                (Some("high"), None),
                (Some("medium"), Some("low")),
                (Some("medium"), None),
                (Some("low"), None),
            ])?;
            hi.priority = p_pair.0;
            lo.priority = p_pair.1;
            assert_eq!(queue_order(&hi, &lo), Ordering::Less);

            // Equal priority: dated milestone strictly beats undated/missing milestone regardless of age.
            lo.priority = hi.priority;
            hi.milestone = Some(Milestone { title: "m1".into(), due_on: Some("2026-11-10T00:00:00Z".into()) });
            lo.milestone =
                if u.arbitrary::<bool>()? { None } else { Some(Milestone { title: "m2".into(), due_on: None }) };
            assert_eq!(queue_order(&hi, &lo), Ordering::Less);

            // Equal priority and due_on: earlier created_at wins regardless of repo and number.
            lo.milestone = hi.milestone.clone();
            hi.created_at = "2026-10-01T00:00:00Z".into();
            lo.created_at = "2026-10-02T00:00:00Z".into();
            assert_eq!(queue_order(&hi, &lo), Ordering::Less);
            Ok(())
        });
    }

    #[derive(Clone)]
    struct FlowScenario {
        cfg: FlowConfig,
        covered: CoveredRepos,
        reads: Vec<Read>,
        tickets: Vec<Ticket>,
        closed: HashSet<String>,
        stored_skips: BTreeMap<String, String>,
        unheld_done: BTreeMap<String, Vec<String>>,
    }

    fn gen_flow_scenario(u: &mut Unstructured<'_>, allow_zero_cap: bool) -> arbitrary::Result<FlowScenario> {
        let repo_count = u.int_in_range(1..=REPOS.len())?;
        let repos: Vec<String> = REPOS[..repo_count].iter().map(|s| (*s).to_string()).collect();
        let mut covered = CoveredRepos::default();
        for r in &repos {
            covered.insert(r.clone());
        }
        let max_tickets = if allow_zero_cap { u.int_in_range(0..=6usize)? } else { u.int_in_range(1..=6usize)? };
        let launch = *u.choose(&[Launch::Ask, Launch::Auto, Launch::Off])?;
        let opt_in = match u.int_in_range(0..=3u8)? {
            0 => None,
            1 => Some(OptIn { label: Some("flow".into()), milestone: None }),
            2 => Some(OptIn { label: None, milestone: Some("v0.1".into()) }),
            _ => Some(OptIn { label: Some("flow".into()), milestone: Some("v0.1".into()) }),
        };
        let cfg = FlowConfig {
            repos: Some(repos.clone()),
            include_repos: Vec::new(),
            exclude_repos: Vec::new(),
            opt_in,
            max_tickets: Some(max_tickets),
            launch,
        };

        let n = u.int_in_range(0..=16usize)?;
        let mut seen = HashSet::new();
        let mut reads = Vec::with_capacity(n);
        let mut issue_keys = Vec::new();
        for _ in 0..n {
            let repo = u.choose(&repos)?.clone();
            let number = u.int_in_range(1..=30u64)?;
            let nk = node_key(&repo, number);
            if !seen.insert(nk.clone()) {
                continue;
            }
            issue_keys.push((repo.clone(), number));
        }

        let mut stored_skips = BTreeMap::new();
        let mut unheld_done: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut closed_thread_seq = 0usize;

        for (repo, number) in &issue_keys {
            let mut labels = Vec::new();
            if u.int_in_range(0..=4u8)? != 0 {
                labels.push((*u.choose(&["flow", "Flow", "FLOW"])?).to_string());
            }
            match u.int_in_range(0..=4u8)? {
                0 => labels.push((*u.choose(&["priority: high", "Priority: High"])?).to_string()),
                1 => labels.push("priority: medium".into()),
                2 => labels.push("priority: low".into()),
                _ => {}
            }
            if u.int_in_range(0..=5u8)? == 0 {
                labels.push((*u.choose(&[EXTERNAL, "Blocked: External"])?).to_string());
            }
            if u.int_in_range(0..=7u8)? == 0 {
                labels.push((*u.choose(&[BLOCKED_LABEL, "Blocked"])?).to_string());
            }

            let open_blocker_n = if u.int_in_range(0..=3u8)? == 0 { u.int_in_range(1..=2usize)? } else { 0 };
            let mut open_blockers = Vec::new();
            let mut blockers = Vec::new();
            for _ in 0..open_blocker_n {
                let brepo = *u.choose(&["acme/widgets", "acme/tools", "acme/outside"])?;
                let bnum = u.int_in_range(100..=120u64)?;
                let br = issue_ref_for(brepo, bnum);
                open_blockers.push(br.clone());
                blockers.push(Target { to: br, state: Some("open"), source: SOURCE_GITHUB });
            }
            let mut unblocked_by = Vec::new();
            if u.int_in_range(0..=3u8)? == 0 {
                let cr = issue_ref_for(repo, u.int_in_range(200..=210u64)?);
                unblocked_by.push(cr.clone());
                blockers.push(Target { to: cr, state: Some("closed"), source: SOURCE_GITHUB });
            }
            let more_blockers = u.int_in_range(0..=9u8)? == 0;
            let unresolved_blockers = if u.int_in_range(0..=9u8)? == 0 { vec!["Phase 9".into()] } else { Vec::new() };

            let parent_ref = if !issue_keys.is_empty() && u.int_in_range(0..=4u8)? == 0 {
                let (pr, pn) = u.choose(&issue_keys)?;
                if pr != repo || pn != number { Some(issue_ref_for(pr, *pn)) } else { None }
            } else {
                None
            };

            let milestone = if u.arbitrary::<bool>()? {
                Some(Milestone {
                    title: format!("v0.{}", u.int_in_range(1..=5u8)?),
                    due_on: (*u.choose(DUE_DATES)?).map(String::from),
                })
            } else {
                None
            };
            let key = format!("key-{repo}-{number}");
            let issue = FlowIssue {
                repo: repo.clone(),
                number: *number,
                label: label_of(repo, *number),
                url: format!("https://github.com/{repo}/issues/{number}"),
                title: format!("Issue {number}"),
                priority: priority(&labels),
                milestone,
                created_at: (*u.choose(CREATED_AT)?).to_string(),
                parent: parent_ref.clone(),
                unblocked_by,
                key: key.clone(),
            };

            match u.int_in_range(0..=7u8)? {
                0 => {
                    stored_skips.insert(skip_key(repo, *number), key.clone());
                }
                1 => {
                    stored_skips.insert(skip_key(repo, *number), "stale-key".into());
                }
                _ => {}
            }
            if u.int_in_range(0..=7u8)? == 0 {
                closed_thread_seq += 1;
                let child = format!("mcp:closed-{closed_thread_seq}");
                let map_key =
                    if u.arbitrary::<bool>()? { node_key(repo, *number) } else { issue.label.to_ascii_lowercase() };
                unheld_done.entry(map_key).or_default().push(child);
            }

            reads.push(Read {
                issue,
                state: Some("open"),
                labels,
                body_track: None,
                body_touches: Vec::new(),
                body_parsed: ParsedBody::default(),
                open_blockers,
                more_blockers,
                blockers,
                parent: parent_ref.map(|to| Target { to, state: Some("open"), source: SOURCE_GITHUB }),
                extra_parents: Vec::new(),
                subs: Vec::new(),
                more_subs: false,
                unresolved_blockers,
                warnings: Vec::new(),
            });
        }

        let ticket_n = u.int_in_range(0..=7usize)?;
        let mut tickets = Vec::with_capacity(ticket_n);
        let mut closed = HashSet::new();
        for t_idx in 0..ticket_n {
            let (trepo, tnum) = if !issue_keys.is_empty() && u.int_in_range(0..=2u8)? != 0 {
                let (r, num) = u.choose(&issue_keys)?;
                (r.clone(), *num)
            } else {
                let r = *u.choose(&["acme/widgets", "acme/tools", "acme/uncovered"])?;
                (r.to_string(), u.int_in_range(1..=40u64)?)
            };
            let label = label_of(&trepo, tnum);
            let (repo_field, issue_field) = match u.int_in_range(0..=2u8)? {
                0 => (None, Some(format!("{trepo}#{tnum}"))),
                1 => (Some(trepo.clone()), None),
                _ => (None, None),
            };
            let t = Ticket {
                label,
                thread: if u.arbitrary::<bool>()? { Some(format!("mcp:t{t_idx}")) } else { None },
                link: None,
                repo: repo_field,
                project: None,
                issue: issue_field,
                status: "running".into(),
            };
            if u.int_in_range(0..=4u8)? == 0
                && let Some((resolved_repo, resolved_num)) = ticket_repo(&covered, &t)
            {
                closed.insert(node_key(&resolved_repo, resolved_num));
            }
            tickets.push(t);
        }

        Ok(FlowScenario { cfg, covered, reads, tickets, closed, stored_skips, unheld_done })
    }

    #[test]
    fn plan_flow_is_permutation_invariant_and_enforces_exclusion_invariants() {
        prop(50, |u| {
            let sc = gen_flow_scenario(u, false)?;
            let write = u.arbitrary::<bool>()?;

            let plan = plan_flow(
                &sc.cfg,
                Reads {
                    reads: sc.reads.clone(),
                    covered: sc.covered.clone(),
                    unreadable: CoveredRepos::default(),
                    unreadable_projects: HashSet::new(),
                    has_unavailable_discovery: false,
                    aliases: std::collections::HashMap::new(),
                },
                sc.tickets.clone(),
                &sc.closed,
                &sc.unheld_done,
                |k| Ok(sc.stored_skips.get(k).cloned()),
                write,
            )
            .unwrap();
            if !write {
                assert!(plan.meta_ops.is_empty());
            }

            // Permutation invariance over unordered reads AND unordered tickets.
            let mut perm_reads = sc.reads.clone();
            shuffle(u, &mut perm_reads)?;
            let mut perm_tickets = sc.tickets.clone();
            shuffle(u, &mut perm_tickets)?;
            let perm_plan = plan_flow(
                &sc.cfg,
                Reads {
                    reads: perm_reads,
                    covered: sc.covered.clone(),
                    unreadable: CoveredRepos::default(),
                    unreadable_projects: HashSet::new(),
                    has_unavailable_discovery: false,
                    aliases: std::collections::HashMap::new(),
                },
                perm_tickets,
                &sc.closed,
                &sc.unheld_done,
                |k| Ok(sc.stored_skips.get(k).cloned()),
                write,
            )
            .unwrap();
            assert_eq!(issue_ids(&plan.view.queue), issue_ids(&perm_plan.view.queue));
            assert_eq!(issue_ids(&plan.view.skipped), issue_ids(&perm_plan.view.skipped));
            assert_eq!(blocked_ids(&plan.view.blocked), blocked_ids(&perm_plan.view.blocked));
            assert_eq!(plan.view.free, perm_plan.view.free);
            let mut run_a: Vec<(String, u64)> = plan.view.running.iter().map(|t| (t.repo.clone(), t.number)).collect();
            let mut run_b: Vec<(String, u64)> =
                perm_plan.view.running.iter().map(|t| (t.repo.clone(), t.number)).collect();
            run_a.sort();
            run_b.sort();
            assert_eq!(run_a, run_b);

            // Sortedness of all three output lists.
            assert_sorted_by_queue_order(&plan.view.queue);
            assert_sorted_by_queue_order(&plan.view.skipped);
            let blocked_issues: Vec<FlowIssue> = plan.view.blocked.iter().map(|b| b.issue.clone()).collect();
            assert_sorted_by_queue_order(&blocked_issues);

            // Exclusion & exact disjoint partition invariants.
            let open_set: HashSet<String> =
                sc.tickets.iter().filter_map(|t| ticket_repo(&sc.covered, t)).map(|(r, n)| node_key(&r, n)).collect();
            let q_set: HashSet<String> = plan.view.queue.iter().map(|i| node_key(&i.repo, i.number)).collect();
            let b_set: HashSet<String> =
                plan.view.blocked.iter().map(|b| node_key(&b.issue.repo, b.issue.number)).collect();
            let s_set: HashSet<String> = plan.view.skipped.iter().map(|i| node_key(&i.repo, i.number)).collect();

            assert_eq!(q_set.len(), plan.view.queue.len());
            assert_eq!(b_set.len(), plan.view.blocked.len());
            assert_eq!(s_set.len(), plan.view.skipped.len());
            assert!(q_set.is_disjoint(&b_set));
            assert!(q_set.is_disjoint(&s_set));
            assert!(b_set.is_disjoint(&s_set));

            let del_ops: HashSet<String> = plan
                .meta_ops
                .iter()
                .filter_map(|op| match op {
                    FlowMetaOp::DelSkip(k) => Some(k.clone()),
                    _ => None,
                })
                .collect();

            for r in &sc.reads {
                let nk = node_key(&r.issue.repo, r.issue.number);
                let has_opt_in = matches_opt_in(sc.cfg.opt_in.as_ref(), r);
                let in_progress = open_set.contains(&nk);
                let parent_in_progress =
                    r.issue.parent.as_ref().is_some_and(|p| open_set.contains(&node_key(&p.repo, p.number)));

                if !has_opt_in || in_progress || parent_in_progress {
                    assert!(!q_set.contains(&nk), "{nk} must be excluded from queue");
                    assert!(!b_set.contains(&nk), "{nk} must be excluded from blocked");
                    assert!(!s_set.contains(&nk), "{nk} must be excluded from skipped");
                    continue;
                }

                let external = r.labels.iter().any(|l| l.eq_ignore_ascii_case(EXTERNAL));
                let blocked_label = has_blocked_label(&r.labels);
                let is_blocked = external
                    || blocked_label
                    || !r.open_blockers.is_empty()
                    || r.more_blockers
                    || !r.unresolved_blockers.is_empty();
                if is_blocked {
                    assert!(b_set.contains(&nk), "blocked issue {nk} missing from blocked");
                    assert!(!q_set.contains(&nk) && !s_set.contains(&nk));
                    continue;
                }

                let sk = skip_key(&r.issue.repo, r.issue.number);
                let has_unheld = sc.unheld_done.contains_key(&nk)
                    || sc.unheld_done.contains_key(&r.issue.label.to_ascii_lowercase());
                let exact_skip = sc.stored_skips.get(&sk).is_some_and(|k| *k == r.issue.key);
                let stale_skip = sc.stored_skips.get(&sk).is_some_and(|k| *k != r.issue.key);

                if has_unheld || exact_skip {
                    assert!(s_set.contains(&nk), "skipped issue {nk} missing from skipped");
                    assert!(!q_set.contains(&nk) && !b_set.contains(&nk));
                } else {
                    assert!(q_set.contains(&nk), "ready issue {nk} missing from queue");
                    assert!(!b_set.contains(&nk) && !s_set.contains(&nk));
                    if write && stale_skip {
                        assert!(del_ops.contains(&sk), "stale skip {sk} should be cleaned up on write");
                    }
                }
            }
            Ok(())
        });
    }

    #[test]
    fn plan_flow_and_plan_picks_enforce_capacity_caps() {
        prop(50, |u| {
            let mut sc = gen_flow_scenario(u, true)?;
            sc.cfg.launch = *u.choose(&[Launch::Ask, Launch::Auto])?;

            let plan = plan_flow(
                &sc.cfg,
                Reads {
                    reads: sc.reads.clone(),
                    covered: sc.covered.clone(),
                    unreadable: CoveredRepos::default(),
                    unreadable_projects: HashSet::new(),
                    has_unavailable_discovery: false,
                    aliases: std::collections::HashMap::new(),
                },
                sc.tickets.clone(),
                &sc.closed,
                &sc.unheld_done,
                |k| Ok(sc.stored_skips.get(k).cloned()),
                false,
            )
            .unwrap();
            let view = &plan.view;

            // Independently derive expected running tickets from inputs (covered and not closed).
            let expected_running: Vec<(String, u64)> = sc
                .tickets
                .iter()
                .filter_map(|t| {
                    let (repo, number) = ticket_repo(&sc.covered, t)?;
                    if !sc.covered.contains(&repo) || sc.closed.contains(&node_key(&repo, number)) {
                        return None;
                    }
                    Some((repo, number))
                })
                .collect();
            let expected_ambiguous = sc
                .tickets
                .iter()
                .filter(|t| {
                    ticket_repo(&sc.covered, t).is_none()
                        && t.issue.is_none()
                        && t.repo.is_none()
                        && t.label.rsplit_once('#').is_some_and(|(name, num)| {
                            num.parse::<u64>().is_ok() && sc.covered.is_ambiguous_short(name)
                        })
                })
                .count();
            let actual_running: Vec<(String, u64)> =
                view.running.iter().map(|rt| (rt.repo.clone(), rt.number)).collect();
            assert_eq!(actual_running, expected_running);

            // Running count and free-slot capacity bounds derived from input tickets.
            let max_tickets = sc.cfg.max_tickets.unwrap_or(0);
            let occupied = expected_running.len() + expected_ambiguous;
            let expected_free = max_tickets.saturating_sub(occupied);
            assert_eq!(view.max_tickets, max_tickets);
            assert_eq!(view.free, expected_free);
            assert!(view.free <= max_tickets);
            if occupied >= max_tickets {
                assert_eq!(view.free, 0);
            } else {
                assert_eq!(occupied + view.free, max_tickets);
            }

            // Generate existing flow cards (ready, stale, duplicates, or non-existent).
            let card_n = u.int_in_range(0..=8usize)?;
            let mut cards = Vec::with_capacity(card_n);
            for idx in 0..card_n {
                let pick = if !view.queue.is_empty() && u.int_in_range(0..=2u8)? != 0 {
                    let q = u.choose(&view.queue)?;
                    FlowPick {
                        repo: q.repo.clone(),
                        number: q.number,
                        label: q.label.clone(),
                        url: q.url.clone(),
                        title: q.title.clone(),
                        key: q.key.clone(),
                    }
                } else {
                    let repo = (*u.choose(REPOS)?).to_string();
                    let number = u.int_in_range(50..=70u64)?;
                    FlowPick {
                        label: label_of(&repo, number),
                        url: format!("https://github.com/{repo}/issues/{number}"),
                        title: format!("Stale {number}"),
                        key: "stale".into(),
                        repo,
                        number,
                    }
                };
                cards.push((format!("a{idx}"), pick));
            }
            let most = *u.choose(&[None, Some(0), Some(1), Some(2), Some(5)])?;
            let pp = plan_picks(sc.cfg.launch, &view.queue, &cards, view.free, most);

            // Every card is either kept or retired exactly once.
            assert_eq!(pp.offered.len() + pp.retire.len(), cards.len());
            let retired_idxs: HashSet<usize> = pp.retire.iter().map(|(i, _)| *i).collect();
            assert_eq!(retired_idxs.len(), pp.retire.len());

            // Strict capacity caps anchored to the input-derived free budget.
            assert!(pp.offered.len() <= expected_free);
            assert!(pp.offered.len() + pp.picks.len() <= expected_free);
            assert!(pp.offered.len() + pp.picks.len() <= max_tickets);
            assert!(pp.picks.len() <= most.unwrap_or(usize::MAX));
            assert!(pp.picks.len() <= view.queue.len());
            if expected_running.len() <= max_tickets {
                assert!(expected_running.len() + pp.offered.len() + pp.picks.len() <= max_tickets);
            }
            if expected_free == 0 {
                assert!(pp.offered.is_empty() && pp.picks.is_empty());
            }

            if sc.cfg.launch == Launch::Auto {
                assert!(pp.offered.is_empty());
                assert_eq!(pp.retire.len(), cards.len());
                assert!(pp.retire.iter().all(|(_, note)| *note == "flow.launch is auto"));
                let want = view.free.min(most.unwrap_or(usize::MAX)).min(view.queue.len());
                assert_eq!(issue_ids(&pp.picks), issue_ids(&view.queue[..want]));
            } else {
                for (p, _) in &pp.offered {
                    assert!(view.queue.iter().any(|i| same(p, i)), "kept non-ready card {p:?}");
                }
                for (idx, note) in &pp.retire {
                    let (_, p) = &cards[*idx];
                    if *note == "no longer ready" {
                        assert!(!view.queue.iter().any(|i| same(p, i)));
                    }
                }
                // Picks are disjoint from kept offered cards and are the exact highest-priority prefix.
                for pick in &pp.picks {
                    assert!(!pp.offered.iter().any(|(p, _)| same(p, pick)));
                }
                let room = view.free.saturating_sub(pp.offered.len()).min(most.unwrap_or(usize::MAX));
                let expected: Vec<FlowIssue> = view
                    .queue
                    .iter()
                    .filter(|i| !pp.offered.iter().any(|(p, _)| same(p, i)))
                    .take(room)
                    .cloned()
                    .collect();
                assert_eq!(issue_ids(&pp.picks), issue_ids(&expected));
            }
            Ok(())
        });
    }

    #[test]
    fn body_track_hook_and_parent_cycles_resolve_deterministically() {
        let mut g = GraphBuild::default();
        let tracked: HashSet<String> = ["acme/widgets".into()].into_iter().collect();
        let make = |n: u64, labels: &[&str], body_track: Option<&str>, parent: Option<u64>| Read {
            issue: FlowIssue {
                repo: "acme/widgets".into(),
                number: n,
                label: label_of("acme/widgets", n),
                url: format!("https://github.com/acme/widgets/issues/{n}"),
                title: format!("Issue {n}"),
                priority: None,
                milestone: None,
                created_at: "2026-10-01T00:00:00Z".into(),
                parent: parent.map(|p| issue_ref_for("acme/widgets", p)),
                unblocked_by: Vec::new(),
                key: format!("k-{n}"),
            },
            state: Some("open"),
            labels: labels.iter().map(|s| (*s).to_string()).collect(),
            body_track: body_track.map(String::from),
            body_touches: Vec::new(),
            body_parsed: ParsedBody::default(),
            open_blockers: Vec::new(),
            more_blockers: false,
            blockers: Vec::new(),
            parent: parent.map(|p| Target {
                to: issue_ref_for("acme/widgets", p),
                state: Some("open"),
                source: SOURCE_GITHUB,
            }),
            extra_parents: Vec::new(),
            subs: Vec::new(),
            more_subs: false,
            unresolved_blockers: Vec::new(),
            warnings: Vec::new(),
        };
        g.read(make(1, &[], Some("flow-dag"), None), None, &tracked);
        g.read(make(2, &["track: explicit"], Some("ignored-body"), None), None, &tracked);
        // Parent cycle 3 <-> 4 terminates deterministically at lowest issue number.
        g.read(make(3, &[], None, Some(4)), None, &tracked);
        g.read(make(4, &[], None, Some(3)), None, &tracked);
        let built = g.finish();
        let by_id = |id: &str| built.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!((by_id("acme/widgets#1").track.as_str(), by_id("acme/widgets#1").track_by), ("flow-dag", "label"));
        assert_eq!((by_id("acme/widgets#2").track.as_str(), by_id("acme/widgets#2").track_by), ("explicit", "label"));
        assert_eq!((by_id("acme/widgets#3").track.as_str(), by_id("acme/widgets#3").track_by), ("widgets#3", "epic"));
        assert_eq!((by_id("acme/widgets#4").track.as_str(), by_id("acme/widgets#4").track_by), ("widgets#3", "epic"));
    }
}
