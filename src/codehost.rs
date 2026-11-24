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

impl ChangeRef {
    pub fn github(repo: &str, number: u64) -> ChangeRef {
        ChangeRef { forge: Forge::Github, repo: repo.into(), number }
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
