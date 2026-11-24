use std::sync::Arc;

use anyhow::Result;

use crate::t3::BoxFut;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Forge {
    Github,
    Gitlab { host: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChangeRef {
    pub forge: Forge,
    pub repo: String,
    pub number: u64,
}

impl Forge {
    pub fn name(&self) -> &'static str {
        match self {
            Forge::Github => "GitHub",
            Forge::Gitlab { .. } => "GitLab",
        }
    }

    pub fn sender(&self) -> &'static str {
        match self {
            Forge::Github => "github",
            Forge::Gitlab { .. } => "gitlab",
        }
    }
}

impl ChangeRef {
    pub fn github(repo: &str, number: u64) -> ChangeRef {
        ChangeRef { forge: Forge::Github, repo: repo.into(), number }
    }

    pub fn gitlab(host: &str, path: &str, number: u64) -> ChangeRef {
        ChangeRef { forge: Forge::Gitlab { host: host.to_ascii_lowercase() }, repo: path.to_ascii_lowercase(), number }
    }

    pub fn of_key(key: &str, number: u64) -> ChangeRef {
        match key.split_once('/') {
            Some((host, path)) if path.contains('/') => ChangeRef::gitlab(host, path, number),
            _ => ChangeRef::github(key, number),
        }
    }

    pub fn key(&self) -> String {
        match &self.forge {
            Forge::Github => self.repo.to_ascii_lowercase(),
            Forge::Gitlab { host } => format!("{host}/{}", self.repo).to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    Closed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Open => "OPEN",
            State::Merged => "MERGED",
            State::Closed => "CLOSED",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rollup {
    pub failing: Vec<String>,
    pub pending: Vec<String>,
    pub settled: Vec<String>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: Option<String>,
    pub state: State,
    pub draft: bool,
    pub head: String,
    pub merge_state: String,
    pub review_decision: Option<String>,
    pub checks: Option<Rollup>,
    pub queue: bool,
    pub queued: bool,
    pub position: Option<u64>,
    pub commit: Option<String>,
    pub ended: Option<String>,
    pub removal: Option<(String, String)>,
    pub method: Option<String>,
    pub closing_issue: Option<String>,
}

impl Change {
    pub fn commit(&self) -> &str {
        self.commit.as_deref().unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRequest {
    pub author: Option<String>,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    pub change: Change,
    pub body: String,
    pub files: Vec<String>,
    pub more_files: bool,
    pub change_requests: Vec<ChangeRequest>,
}

#[derive(Debug)]
pub enum Read {
    Found(Box<Change>),
    NotFound(String),
    Failed(String),
}

#[derive(Debug)]
pub enum Viewed {
    Found(Box<Detail>),
    NotFound,
    Refused(String),
}

#[derive(Clone, Copy, Debug)]
pub enum Landing<'a> {
    Queue,
    Merge { method: &'a str },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Landed {
    Queued { position: Option<u64> },
    Merged { commit: Option<String> },
    Refused(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Branch {
    Unread(String),
    NotMerged(String),
    Fork(String),
    Gone(String),
    TipMoved { branch: String, tip: String },
    Deleted(String),
    Failed { branch: String, why: String },
}

pub trait CodeHost: Send + Sync {
    fn read<'a>(&'a self, refs: &'a [ChangeRef]) -> BoxFut<'a, Result<Vec<Read>>>;

    fn view<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Result<Viewed>>;

    fn merge<'a>(
        &'a self,
        r: &'a ChangeRef,
        id: &'a str,
        head: &'a str,
        how: Landing<'a>,
    ) -> BoxFut<'a, Result<Landed>>;

    fn delete_branch<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Branch>;

    fn url(&self, r: &ChangeRef) -> String;
}

pub struct Router {
    github: Arc<dyn CodeHost>,
    gitlab: Arc<dyn CodeHost>,
}

impl Router {
    pub fn new(github: Arc<dyn CodeHost>, gitlab: Arc<dyn CodeHost>) -> Router {
        Router { github, gitlab }
    }

    fn of(&self, r: &ChangeRef) -> &dyn CodeHost {
        match r.forge {
            Forge::Github => &*self.github,
            Forge::Gitlab { .. } => &*self.gitlab,
        }
    }
}

impl CodeHost for Router {
    fn read<'a>(&'a self, refs: &'a [ChangeRef]) -> BoxFut<'a, Result<Vec<Read>>> {
        Box::pin(async move {
            let (hub, lab): (Vec<ChangeRef>, Vec<ChangeRef>) =
                refs.iter().cloned().partition(|r| r.forge == Forge::Github);
            let mut hub = if hub.is_empty() { Vec::new() } else { self.github.read(&hub).await? }.into_iter();
            let mut lab = if lab.is_empty() { Vec::new() } else { self.gitlab.read(&lab).await? }.into_iter();
            Ok(refs
                .iter()
                .map(|r| {
                    let next = if r.forge == Forge::Github { hub.next() } else { lab.next() };
                    next.unwrap_or_else(|| Read::Failed(format!("{} returned no read", r.forge.name())))
                })
                .collect())
        })
    }

    fn view<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Result<Viewed>> {
        self.of(r).view(r)
    }

    fn merge<'a>(
        &'a self,
        r: &'a ChangeRef,
        id: &'a str,
        head: &'a str,
        how: Landing<'a>,
    ) -> BoxFut<'a, Result<Landed>> {
        self.of(r).merge(r, id, head, how)
    }

    fn delete_branch<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Branch> {
        self.of(r).delete_branch(r)
    }

    fn url(&self, r: &ChangeRef) -> String {
        self.of(r).url(r)
    }
}
