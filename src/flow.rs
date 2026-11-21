use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::board::BOARD;
use crate::config::{FlowConfig, Launch};
use crate::github::errors;
use crate::wf::Wf;

pub const EXTERNAL: &str = "blocked: external";
const PRIORITIES: [&str; 3] = ["high", "medium", "low"];
const MAX_PAGES: usize = 10;

pub const FLOW_ISSUES: &str = "query FlowIssues($owner: String!, $name: String!, $label: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    issues(states: OPEN, labels: [$label], first: 100, after: $after, orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title url createdAt
        labels(first: 100) { nodes { name } }
        milestone { title dueOn }
        parent { number title url repository { nameWithOwner } }
        blockedBy(first: 100) { totalCount nodes { number title url state repository { nameWithOwner } } }
      }
    }
  }
}";

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
}

struct Ticket {
    label: String,
    thread: Option<String>,
    link: Option<String>,
}

struct Read {
    issue: FlowIssue,
    labels: Vec<String>,
    open_blockers: Vec<IssueRef>,
    more_blockers: bool,
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

fn parse_node(repo: &str, n: &Value) -> Result<Read> {
    let number = n["number"].as_u64().ok_or_else(|| anyhow!("GitHub returned an issue in {repo} with no number"))?;
    let labels: Vec<String> = n["labels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["name"].as_str())
        .map(String::from)
        .collect();
    let blockers: Vec<&Value> = n["blockedBy"]["nodes"].as_array().into_iter().flatten().collect();
    let more_blockers = n["blockedBy"]["totalCount"].as_u64().is_some_and(|t| t > blockers.len() as u64);
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
    };
    Ok(Read { more_blockers, open_blockers: open.into_iter().filter_map(issue_ref).collect(), labels, issue })
}

impl Wf {
    fn open_tickets(&self) -> Result<Vec<Ticket>> {
        let mut out: Vec<Ticket> = self
            .store
            .children(BOARD, true)?
            .into_iter()
            .map(|l| {
                let link = l.link.clone().or_else(|| self.live_of(&l.child)?["link"].as_str().map(String::from));
                Ticket { label: l.label, thread: Some(l.child), link }
            })
            .collect();
        let seen: HashSet<String> = out.iter().map(|t| t.label.to_ascii_lowercase()).collect();
        out.extend(
            self.store
                .pending_all()?
                .into_iter()
                .filter(|p| p.parent == BOARD && !seen.contains(&p.label.to_ascii_lowercase()))
                .map(|p| Ticket { label: p.label, thread: p.thread, link: p.link }),
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

    pub async fn flow(&self) -> Result<Value> {
        let Some(cfg) = self.config().flow.clone() else {
            let off = FlowView {
                enabled: false,
                launch: None,
                max_tickets: 0,
                free: 0,
                running: Vec::new(),
                queue: Vec::new(),
                blocked: Vec::new(),
            };
            return Ok(serde_json::to_value(off)?);
        };
        Ok(serde_json::to_value(self.flow_view(&cfg).await?)?)
    }

    async fn flow_view(&self, cfg: &FlowConfig) -> Result<FlowView> {
        let mut reads = Vec::new();
        let mut covered: BTreeMap<String, String> = BTreeMap::new();
        for repo in &cfg.repos {
            let (full, rs) = self.read_repo(repo, &cfg.opt_in.label).await?;
            let name = full.rsplit_once('/').map_or(full.as_str(), |(_, n)| n).to_ascii_lowercase();
            covered.insert(name, full);
            reads.extend(rs);
        }
        let tickets = self.open_tickets()?;
        let open: HashSet<String> = tickets.iter().map(|t| t.label.to_ascii_lowercase()).collect();
        let running: Vec<FlowTicket> = tickets
            .into_iter()
            .filter_map(|t| {
                let (name, n) = t.label.rsplit_once('#')?;
                let number: u64 = n.parse().ok()?;
                let repo = covered.get(&name.to_ascii_lowercase())?.clone();
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
        let opt_in = cfg.opt_in.label.to_ascii_lowercase();
        let (mut queue, mut blocked) = (Vec::new(), Vec::new());
        for r in reads {
            let has = |want: &str| r.labels.iter().any(|l| l.eq_ignore_ascii_case(want));
            if !has(&opt_in) || open.contains(&r.issue.label.to_ascii_lowercase()) {
                continue;
            }
            if r.issue.parent.as_ref().is_some_and(|p| open.contains(&p.label.to_ascii_lowercase())) {
                continue;
            }
            let external = has(EXTERNAL);
            if external || !r.open_blockers.is_empty() || r.more_blockers {
                blocked.push(BlockedIssue {
                    issue: r.issue,
                    blockers: r.open_blockers,
                    more_blockers: r.more_blockers,
                    external,
                });
            } else {
                queue.push(r.issue);
            }
        }
        queue.sort_by(queue_order);
        blocked.sort_by(|a, b| queue_order(&a.issue, &b.issue));
        Ok(FlowView {
            enabled: true,
            launch: Some(cfg.launch),
            max_tickets: cfg.max_tickets,
            free: cfg.max_tickets.saturating_sub(running.len()),
            running,
            queue,
            blocked,
        })
    }
}
