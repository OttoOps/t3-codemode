use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{BOARD, Card, LaunchArgs, card_text, chip};
use crate::config::{FlowConfig, Launch};
use crate::github::errors;
use crate::wf::Wf;

pub const EXTERNAL: &str = "blocked: external";
pub const LAUNCH: &str = "launch";
pub const SKIP: &str = "skip";
const PRIORITIES: [&str; 3] = ["high", "medium", "low"];
const MAX_PAGES: usize = 10;
const SKIPPED: &str = "flow_skip:";
const HELD: &str = "flow_done:";

pub const FLOW_TICKETS: &str = "query FlowTickets(";

macro_rules! issue_fields {
    () => {
        "number title url createdAt state
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

const TRACK: &str = "track:";
pub const BLOCKED_BY: &str = "blockedBy";
pub const PARENT: &str = "parent";

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

#[derive(Clone, Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
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
}

struct Reads {
    reads: Vec<Read>,
    covered: BTreeMap<String, String>,
}

#[derive(Clone)]
struct Ticket {
    label: String,
    thread: Option<String>,
    link: Option<String>,
    repo: Option<String>,
    issue: Option<String>,
    status: String,
}

#[derive(Clone)]
struct Target {
    to: IssueRef,
    state: Option<&'static str>,
}

#[derive(Clone)]
struct Read {
    issue: FlowIssue,
    state: Option<&'static str>,
    labels: Vec<String>,
    open_blockers: Vec<IssueRef>,
    more_blockers: bool,
    blockers: Vec<Target>,
    parent: Option<Target>,
    subs: Vec<Target>,
    more_subs: bool,
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
    Some(Target { to: issue_ref(v)?, state: state_of(v) })
}

fn targets(v: &Value) -> Vec<Target> {
    v["nodes"].as_array().into_iter().flatten().filter_map(target).collect()
}

fn more(v: &Value, listed: usize) -> bool {
    v["totalCount"].as_u64().is_some_and(|t| t > listed as u64)
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
    let mut names: Vec<String> = labels.iter().map(|l| l.trim().to_ascii_lowercase()).collect();
    names.sort();
    let mut held: Vec<String> = blockers
        .iter()
        .filter_map(|b| Some((issue_ref(b)?, b["state"] == "CLOSED")))
        .map(|(r, closed)| {
            format!("{}#{}:{}", r.repo.to_ascii_lowercase(), r.number, if closed { "closed" } else { "open" })
        })
        .collect();
    held.sort();
    let key = json!([names, held, more_blockers]).to_string();
    let (open, closed): (Vec<&Value>, Vec<&Value>) = blockers.into_iter().partition(|b| b["state"] != "CLOSED");
    let milestone = n["milestone"]["title"]
        .as_str()
        .map(|t| Milestone { title: t.to_string(), due_on: n["milestone"]["dueOn"].as_str().map(String::from) });
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
        subs,
        more_subs,
        labels,
        issue,
    })
}

fn skip_key(repo: &str, number: u64) -> String {
    format!("{SKIPPED}{}#{number}", repo.to_ascii_lowercase())
}

fn ticket_issue(covered: &BTreeMap<String, String>, label: &str) -> Option<(String, u64)> {
    let (name, n) = label.rsplit_once('#')?;
    let number: u64 = n.parse().ok()?;
    Some((covered.get(&name.to_ascii_lowercase())?.clone(), number))
}

fn ticket_repo(covered: &BTreeMap<String, String>, t: &Ticket) -> Option<(String, u64)> {
    if let Some((repo, n)) = t.issue.as_deref().and_then(|i| i.rsplit_once('#')) {
        return Some((repo.to_string(), n.parse().ok()?));
    }
    let (name, n) = t.label.rsplit_once('#')?;
    let known = t.repo.as_deref().filter(|r| r.rsplit_once('/').is_some_and(|(_, s)| s.eq_ignore_ascii_case(name)));
    match known {
        Some(repo) => Some((repo.to_string(), n.parse().ok()?)),
        None => ticket_issue(covered, &t.label),
    }
}

fn covers(covered: &BTreeMap<String, String>, repo: &str) -> bool {
    covered.values().any(|c| c.eq_ignore_ascii_case(repo))
}

fn node_key(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo.to_ascii_lowercase())
}

fn track_label(labels: &[String]) -> Option<String> {
    labels
        .iter()
        .filter_map(|l| {
            let (head, rest) = l.split_at_checked(TRACK.len())?;
            let rest = rest.trim();
            (head.eq_ignore_ascii_case(TRACK) && !rest.is_empty()).then(|| rest.to_string())
        })
        .min_by_key(|t| t.to_ascii_lowercase())
}

#[derive(Default)]
struct GraphBuild {
    nodes: BTreeMap<String, GraphNode>,
    edges: std::collections::BTreeSet<(String, &'static str, String)>,
    more_blockers: HashSet<String>,
    more_subs: HashSet<String>,
}

impl GraphBuild {
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
            self.edges.insert((key.clone(), BLOCKED_BY, to));
        }
        if let Some(p) = &r.parent {
            let to = self.node(&p.to, p.state);
            self.edges.insert((key.clone(), PARENT, to));
        }
        for s in &r.subs {
            let from = self.node(&s.to, s.state);
            self.edges.insert((from, PARENT, key.clone()));
        }
        if r.more_blockers {
            self.more_blockers.insert(key.clone());
        }
        if r.more_subs {
            self.more_subs.insert(key.clone());
        }
        let outside = |b: &IssueRef| !tracked.contains(&b.repo.to_ascii_lowercase());
        let external = r.labels.iter().any(|l| l.eq_ignore_ascii_case(EXTERNAL))
            || (!r.open_blockers.is_empty() && !r.more_blockers && r.open_blockers.iter().all(outside));
        let n = self.nodes.get_mut(&key).expect("node just added");
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
        for (from, _, to) in &self.edges {
            let (a, b) = (root(&mut up, from), root(&mut up, to));
            if a != b {
                let (lo, hi) = if order(&self.nodes, &a) <= order(&self.nodes, &b) { (a, b) } else { (b, a) };
                up.insert(hi, lo);
            }
        }
        let mut sorted: Vec<&(String, &'static str, String)> = self.edges.iter().collect();
        sorted.sort_by(|a, b| {
            order(&self.nodes, &a.0)
                .cmp(&order(&self.nodes, &b.0))
                .then(a.1.cmp(b.1))
                .then_with(|| order(&self.nodes, &a.2).cmp(&order(&self.nodes, &b.2)))
        });
        let sorted: Vec<(String, &'static str, String)> = sorted.into_iter().cloned().collect();
        let mut edges = Vec::new();
        for (from, kind, to) in &sorted {
            let (f, t) = (id(&self.nodes, from), id(&self.nodes, to));
            match *kind {
                BLOCKED_BY => {
                    self.nodes.get_mut(from).expect("edge end").blocked_by.push(t.clone());
                    self.nodes.get_mut(to).expect("edge end").unlocks.push(f.clone());
                }
                _ => {
                    let child = self.nodes.get_mut(from).expect("edge end");
                    child.parent = child.parent.take().or_else(|| Some(t.clone()));
                    self.nodes.get_mut(to).expect("edge end").sub_issues.push(f.clone());
                }
            }
            edges.push(GraphEdge { from: f, to: t, kind });
        }
        let mut keys: Vec<String> = self.nodes.keys().cloned().collect();
        keys.sort_by_key(|k| order(&self.nodes, k));
        let mut lanes: BTreeMap<String, String> = BTreeMap::new();
        for k in &keys {
            let comp = root(&mut up, k);
            let n = self.nodes.get_mut(k).expect("listed key");
            n.more_blockers = self.more_blockers.contains(k);
            n.more_sub_issues = self.more_subs.contains(k);
            let own = match (track_label(&n.labels), &n.milestone) {
                (Some(t), _) => Some((t, "label")),
                (None, Some(m)) => Some((m.title.clone(), "milestone")),
                (None, None) => None,
            };
            if let Some((t, by)) = own {
                lanes.entry(comp).or_insert_with(|| t.clone());
                (n.track, n.track_by) = (t, by);
            }
        }
        for k in &keys {
            let comp = root(&mut up, k);
            let lane = lanes.get(&comp).cloned().unwrap_or_else(|| self.nodes[&comp].label.clone());
            let n = self.nodes.get_mut(k).expect("listed key");
            if n.track_by.is_empty() {
                (n.track, n.track_by) = (lane, "component");
            }
        }
        let mut nodes: Vec<GraphNode> = self.nodes.into_values().collect();
        nodes.sort_by(|a, b| {
            a.repo.to_ascii_lowercase().cmp(&b.repo.to_ascii_lowercase()).then(a.number.cmp(&b.number))
        });
        Graph { nodes, edges }
    }
}

fn graph(
    covered: &BTreeMap<String, String>,
    reads: Vec<Read>,
    tickets: &[Ticket],
    lists: &BTreeMap<String, &'static str>,
) -> Graph {
    let tracked: HashSet<String> = covered.values().map(|r| r.to_ascii_lowercase()).collect();
    let mut g = GraphBuild::default();
    for r in reads {
        let list = lists.get(&node_key(&r.issue.repo, r.issue.number)).copied();
        g.read(r, list, &tracked);
    }
    for t in tickets {
        if let Some((repo, number)) = ticket_repo(covered, t) {
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
            format!("t{i}: repository(owner: $o{i}, name: $r{i}) {{ issue(number: $n{i}) {{ state }} }}")
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

impl Wf {
    fn open_tickets(&self) -> Result<Vec<Ticket>> {
        let mut out: Vec<Ticket> = self
            .store
            .children(BOARD, true)?
            .into_iter()
            .map(|l| {
                let link = l.link.clone().or_else(|| self.live_of(&l.child)?["link"].as_str().map(String::from));
                let v = self.node_view(&l);
                let repo = v["repo"].as_str().map(String::from);
                let status = v["status"].as_str().unwrap_or("unknown").to_string();
                Ticket { label: l.label, thread: Some(l.child), link, repo, issue: l.identity.issue, status }
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
                    let repo = self.place(None, &live.unwrap_or(Value::Null)).0.as_str().map(String::from);
                    Ticket {
                        label: p.label,
                        thread: p.thread,
                        link: p.link,
                        repo,
                        issue: p.identity.issue,
                        status: status.to_string(),
                    }
                }),
        );
        Ok(out)
    }

    async fn read_repo(&self, repo: &str, label: &str) -> Result<(String, Vec<Read>)> {
        let (owner, name) = repo.split_once('/').ok_or_else(|| anyhow!("flow: {repo:?} is not owner/name"))?;
        let mut after: Option<String> = None;
        let mut out = Vec::new();
        let mut full = repo.to_string();
        for _ in 0..MAX_PAGES {
            let vars = json!({"owner": owner, "name": name, "label": label, "after": after});
            let resp = self.github.graphql(FLOW_ISSUES, vars).await?;
            if let Some(e) = errors(&resp) {
                bail!("wf.flow: reading {repo}'s issues failed: {e}");
            }
            let r = &resp["data"]["repository"];
            if r.is_null() {
                bail!("wf.flow: GitHub returned no repository {repo}");
            }
            if let Some(n) = r["nameWithOwner"].as_str() {
                full = n.to_string();
            }
            for n in r["issues"]["nodes"].as_array().into_iter().flatten() {
                out.push(parse_node(&full, n)?);
            }
            let page = &r["issues"]["pageInfo"];
            match page["endCursor"].as_str() {
                Some(c) if page["hasNextPage"] == true => after = Some(c.to_string()),
                _ => return Ok((full, out)),
            }
        }
        bail!("wf.flow: {repo} has more than {} open issues labelled {label:?}", MAX_PAGES * 100)
    }

    async fn flow_reads(&self, cfg: &FlowConfig) -> Result<Reads> {
        let mut reads = Vec::new();
        let mut covered: BTreeMap<String, String> = BTreeMap::new();
        for repo in &cfg.repos {
            let (full, rs) = self.read_repo(repo, &cfg.opt_in.label).await?;
            let name = full.rsplit_once('/').map_or(full.as_str(), |(_, n)| n).to_ascii_lowercase();
            covered.insert(name, full);
            reads.extend(rs);
        }
        Ok(Reads { reads, covered })
    }

    async fn ticket_reads(&self, r: &Reads, tickets: &[Ticket], full: bool) -> Result<(HashSet<String>, Vec<Read>)> {
        let open: HashSet<String> = r.reads.iter().map(|x| node_key(&x.issue.repo, x.issue.number)).collect();
        let asks: Vec<(String, (String, u64))> = tickets
            .iter()
            .filter_map(|t| ticket_repo(&r.covered, t))
            .filter(|(repo, _)| full || covers(&r.covered, repo))
            .map(|(repo, number)| (node_key(&repo, number), (repo, number)))
            .filter(|(key, _)| !open.contains(key))
            .collect();
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
            for (i, (key, (repo, _))) in chunk.iter().enumerate() {
                let t = &resp["data"][format!("t{i}")];
                let issue = &t["issue"];
                if issue["state"] == "CLOSED" {
                    closed.insert(key.clone());
                }
                if full && issue.is_object() {
                    reads.push(parse_node(t["nameWithOwner"].as_str().unwrap_or(repo), issue)?);
                }
            }
        }
        Ok((closed, reads))
    }

    pub async fn flow(&self, with_graph: bool) -> Result<Value> {
        let Some(cfg) = self.config().flow.clone() else {
            let tickets = if with_graph { self.open_tickets()? } else { Vec::new() };
            let off = FlowView {
                enabled: false,
                launch: None,
                max_tickets: 0,
                free: 0,
                running: Vec::new(),
                queue: Vec::new(),
                blocked: Vec::new(),
                skipped: Vec::new(),
                graph: with_graph.then(|| graph(&BTreeMap::new(), Vec::new(), &tickets, &BTreeMap::new())),
            };
            return Ok(serde_json::to_value(off)?);
        };
        let reads = self.flow_reads(&cfg).await?;
        let tickets = self.open_tickets()?;
        let (closed, extra) = self.ticket_reads(&reads, &tickets, with_graph).await?;
        let covered = reads.covered.clone();
        let mut all = reads.reads.clone();
        all.extend(extra);
        let mut view = self.flow_build(&cfg, reads, tickets.clone(), &closed, false)?;
        if !with_graph {
            return Ok(serde_json::to_value(view)?);
        }
        let mut lists: BTreeMap<String, &'static str> = BTreeMap::new();
        for (name, list) in [("queue", &view.queue), ("skipped", &view.skipped)] {
            lists.extend(list.iter().map(|i| (node_key(&i.repo, i.number), name)));
        }
        lists.extend(view.blocked.iter().map(|b| (node_key(&b.issue.repo, b.issue.number), "blocked")));
        view.graph = Some(graph(&covered, all, &tickets, &lists));
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
        let open: HashSet<String> =
            tickets.iter().filter_map(|t| ticket_repo(&r.covered, t)).map(|(repo, n)| node_key(&repo, n)).collect();
        let running: Vec<FlowTicket> = tickets
            .into_iter()
            .filter_map(|t| {
                let (repo, number) = ticket_repo(&r.covered, &t)?;
                if !covers(&r.covered, &repo) || closed.contains(&node_key(&repo, number)) {
                    return None;
                }
                Some(FlowTicket {
                    url: format!("https://github.com/{repo}/issues/{number}"),
                    label: t.label,
                    repo,
                    number,
                    thread: t.thread,
                    link: t.link,
                })
            })
            .collect();
        let mut done: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for l in self.store.children(BOARD, false)?.into_iter().filter(|l| l.closed) {
            done.entry(l.label.to_ascii_lowercase()).or_default().push(l.child);
        }
        let opt_in = cfg.opt_in.label.to_ascii_lowercase();
        let (mut queue, mut blocked, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
        for r in r.reads {
            let has = |want: &str| r.labels.iter().any(|l| l.eq_ignore_ascii_case(want));
            if !has(&opt_in) || open.contains(&node_key(&r.issue.repo, r.issue.number)) {
                continue;
            }
            if r.issue.parent.as_ref().is_some_and(|p| open.contains(&node_key(&p.repo, p.number))) {
                continue;
            }
            let key = skip_key(&r.issue.repo, r.issue.number);
            let mut stored = self.store.meta_get(&key)?;
            let mut unheld = Vec::new();
            for t in done.get(&r.issue.label.to_ascii_lowercase()).into_iter().flatten() {
                let held = format!("{HELD}{t}");
                if self.store.meta_get(&held)?.is_none() {
                    unheld.push(held);
                }
            }
            if !unheld.is_empty() {
                if write {
                    self.store.meta_set(&key, &r.issue.key)?;
                    for held in &unheld {
                        self.store.meta_set(held, &self.now())?;
                    }
                }
                stored = Some(r.issue.key.clone());
            }
            let external = has(EXTERNAL);
            if external || !r.open_blockers.is_empty() || r.more_blockers {
                if write && stored.as_ref().is_some_and(|k| *k != r.issue.key) {
                    self.store.meta_del(&key)?;
                }
                blocked.push(BlockedIssue {
                    issue: r.issue,
                    blockers: r.open_blockers,
                    more_blockers: r.more_blockers,
                    external,
                });
                continue;
            }
            match stored {
                Some(k) if k == r.issue.key => skipped.push(r.issue),
                Some(_) => {
                    if write {
                        self.store.meta_del(&key)?;
                    }
                    queue.push(r.issue);
                }
                None => queue.push(r.issue),
            }
        }
        queue.sort_by(queue_order);
        skipped.sort_by(queue_order);
        blocked.sort_by(|a, b| queue_order(&a.issue, &b.issue));
        Ok(FlowView {
            enabled: true,
            launch: Some(cfg.launch),
            max_tickets: cfg.max_tickets,
            free: cfg.max_tickets.saturating_sub(running.len()),
            running,
            queue,
            blocked,
            skipped,
            graph: None,
        })
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

    pub(crate) async fn flow_pick(&self, most: Option<usize>) -> Result<Value> {
        let Some(cfg) = self.config().flow.clone() else {
            return Ok(json!({"launch": null, "free": 0, "picked": [], "offered": [],
                             "reason": "the flow is off: the config has no flow block"}));
        };
        let _pick = self.flow_lock.lock().await;
        if cfg.launch == Launch::Off {
            let _g = self.lock.lock().await;
            for (c, _) in self.flow_cards()? {
                self.retire_flow_card(&c, "flow.launch is off")?;
            }
            self.changed();
            return Ok(json!({"launch": cfg.launch, "free": 0, "picked": [], "offered": [],
                             "reason": "flow.launch is off, so nothing launches"}));
        }
        let reads = self.flow_reads(&cfg).await?;
        let (closed, _) = self.ticket_reads(&reads, &self.open_tickets()?, false).await?;
        let g = self.lock.lock().await;
        let covered = reads.covered.clone();
        let view = self.flow_build(&cfg, reads, self.open_tickets()?, &closed, true)?;
        let ready = |p: &FlowPick| view.queue.iter().any(|i| same(p, i));
        let mut offered: Vec<(FlowPick, String)> = Vec::new();
        let mut retired = Vec::new();
        for (c, p) in self.flow_cards()? {
            let note = if cfg.launch == Launch::Auto {
                "flow.launch is auto"
            } else if !ready(&p) {
                "no longer ready"
            } else if offered.len() >= view.free {
                "no free slot"
            } else {
                offered.push((p, c.id.clone()));
                continue;
            };
            if self.retire_flow_card(&c, note)? {
                retired.push(c.id.clone());
            }
        }
        if !retired.is_empty() {
            self.changed();
        }
        let room = view.free.saturating_sub(offered.len()).min(most.unwrap_or(usize::MAX));
        let picks: Vec<FlowIssue> =
            view.queue.iter().filter(|i| !offered.iter().any(|(p, _)| same(p, i))).take(room).cloned().collect();
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
                if open.iter().filter_map(|t| ticket_repo(&covered, t)).any(|(r, n)| node_key(&r, n) == parent) {
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
        let cfg = self.config().flow.clone().filter(|c| c.launch != Launch::Off);
        let fresh = match &cfg {
            Some(c) => {
                let reads = self.flow_reads(c).await?;
                let (closed, _) = self.ticket_reads(&reads, &self.open_tickets()?, false).await?;
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
