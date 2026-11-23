use std::collections::HashMap;
use std::fmt;

use anyhow::bail;
use serde::Deserialize;
use serde_json::Value;

use crate::board::CARD_KINDS;
use crate::store::{BoardContext, ContextPut, ContextSpec};
use crate::wf::Wf;

pub const MAX_CONTEXTS: usize = 100;
pub const STATUSES: &[&str] = &["running", "waiting", "blocked", "done", "paused", "pr-green", "pr-review", "alert"];
const MAX_ID: usize = 64;
const MAX_NAME: usize = 60;
const MAX_PROJECTS: usize = 100;
const MAX_PROJECT: usize = 200;
const MAX_QUERY: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub spec: Option<ContextSpec>,
}

#[derive(Debug)]
pub enum ContextError {
    Invalid(String),
    Missing(String),
    Taken(String),
    Store(anyhow::Error),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::Invalid(m) | ContextError::Missing(m) | ContextError::Taken(m) => f.write_str(m),
            ContextError::Store(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for ContextError {
    fn from(e: anyhow::Error) -> Self {
        ContextError::Store(e)
    }
}

fn invalid(msg: impl Into<String>) -> ContextError {
    ContextError::Invalid(msg.into())
}

pub fn same_name(a: &str, b: &str) -> bool {
    unicase::eq(a, b)
}

pub fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    unicase::UniCase::new(a).cmp(&unicase::UniCase::new(b))
}

fn check_id(id: &str) -> Result<(), ContextError> {
    let ok =
        !id.is_empty() && id.len() <= MAX_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok { Ok(()) } else { Err(invalid(format!("context ids are 1 to {MAX_ID} letters, digits, - or _"))) }
}

fn check_name(name: &str) -> Result<String, ContextError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("a context needs a name"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(invalid(format!("context names are at most {MAX_NAME} characters")));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("context names are one line of text"));
    }
    if name.eq_ignore_ascii_case("all") {
        return Err(invalid("\"All\" is the built-in context that clears every filter; pick another name"));
    }
    Ok(name.to_string())
}

fn pick(values: &[String], allowed: &[&str], what: &str) -> Result<Vec<String>, ContextError> {
    if let Some(bad) = values.iter().find(|v| !allowed.contains(&v.as_str())) {
        return Err(invalid(format!("unknown {what} {bad:?}; use {}", allowed.join(", "))));
    }
    Ok(allowed.iter().filter(|a| values.iter().any(|v| v == *a)).map(|a| a.to_string()).collect())
}

fn check_spec(spec: ContextSpec) -> Result<ContextSpec, ContextError> {
    let mut projects: Vec<String> = spec.projects.iter().map(|p| p.trim().to_string()).collect();
    if projects.iter().any(|p| p.is_empty() || p.chars().count() > MAX_PROJECT || p.chars().any(char::is_control)) {
        return Err(invalid(format!("each project is one line of 1 to {MAX_PROJECT} characters")));
    }
    projects.sort();
    projects.dedup();
    if projects.len() > MAX_PROJECTS {
        return Err(invalid(format!("a context holds at most {MAX_PROJECTS} projects")));
    }
    let q = spec.q.map(|q| q.trim().to_string()).filter(|q| !q.is_empty());
    if q.as_ref().is_some_and(|q| q.chars().count() > MAX_QUERY || q.chars().any(char::is_control)) {
        return Err(invalid(format!("the search is one line of at most {MAX_QUERY} characters")));
    }
    Ok(ContextSpec {
        projects,
        statuses: pick(&spec.statuses, STATUSES, "status")?,
        kinds: pick(&spec.kinds, CARD_KINDS, "card kind")?,
        q,
    })
}

#[derive(Clone)]
struct Place {
    id: String,
    repo: String,
    title: String,
}

impl Place {
    fn of(item: &Value) -> Option<Place> {
        let s = |v: &Value| v.as_str().unwrap_or_default().trim().to_string();
        let p = Place { id: s(&item["project"]["id"]), repo: s(&item["repo"]), title: s(&item["project"]["title"]) };
        (!p.id.is_empty() || !p.repo.is_empty() || !p.title.is_empty()).then_some(p)
    }

    fn hit(&self, v: &str) -> bool {
        if !self.id.is_empty() && v == self.id {
            return true;
        }
        let low = v.to_lowercase();
        (!self.repo.is_empty() && low == self.repo.to_lowercase())
            || (!self.title.is_empty() && low == self.title.to_lowercase())
    }
}

pub struct ProjectFilter {
    want: Vec<String>,
    known: HashMap<String, Place>,
}

impl ProjectFilter {
    pub fn new<'a>(want: Vec<String>, board: impl IntoIterator<Item = &'a Value>) -> ProjectFilter {
        let mut known: HashMap<String, Place> = HashMap::new();
        for p in board.into_iter().filter_map(Place::of).filter(|p| !p.id.is_empty()) {
            let k = known.entry(p.id.clone()).or_insert_with(|| p.clone());
            if k.repo.is_empty() {
                k.repo = p.repo;
            }
            if k.title.is_empty() {
                k.title = p.title;
            }
        }
        ProjectFilter { want, known }
    }

    pub fn keeps(&self, item: &Value) -> bool {
        Place::of(item).is_some_and(|p| self.want.iter().any(|v| self.matches(&p, v)))
    }

    fn matches(&self, p: &Place, v: &str) -> bool {
        if p.hit(v) {
            return true;
        }
        if !p.id.is_empty() {
            return self.known.get(&p.id).is_some_and(|k| k.hit(v));
        }
        !p.repo.is_empty() && self.known.get(v).is_some_and(|k| k.repo.to_lowercase() == p.repo.to_lowercase())
    }
}

impl Wf {
    pub fn project_filter(
        &self,
        context: Option<&str>,
        projects: Option<Vec<String>>,
    ) -> anyhow::Result<Option<ProjectFilter>> {
        let want: Vec<String> = match (context, projects) {
            (Some(_), Some(_)) => bail!("give a context or projects, not both"),
            (Some(key), None) => {
                let all = self.contexts()?;
                let found = all.iter().find(|c| c.id == key).or_else(|| all.iter().find(|c| same_name(&c.name, key)));
                let Some(c) = found else { bail!("no saved context with the id or name {key:?}") };
                c.spec.projects.clone()
            }
            (None, Some(p)) => p.into_iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect(),
            (None, None) => return Ok(None),
        };
        if want.is_empty() {
            return Ok(None);
        }
        Ok(Some(ProjectFilter::new(want, &self.board_places()?)))
    }

    pub fn contexts(&self) -> anyhow::Result<Vec<BoardContext>> {
        self.store.contexts_all()
    }

    pub fn put_context(&self, id: &str, body: ContextBody) -> Result<(bool, BoardContext, u64), ContextError> {
        check_id(id)?;
        if body.name.is_none() && body.spec.is_none() {
            return Err(invalid("give a name, a spec or both"));
        }
        let name = body.name.as_deref().map(check_name).transpose()?;
        let spec = body.spec.map(check_spec).transpose()?;
        let _in_order = self.context_writes.lock().unwrap_or_else(|e| e.into_inner());
        let out = match self.store.context_put(id, name.as_deref(), spec.as_ref(), &self.now(), MAX_CONTEXTS)? {
            ContextPut::Created(c) => (true, c),
            ContextPut::Updated(c) => (false, c),
            ContextPut::NameTaken(name) => return Err(ContextError::Taken(format!("a context named {name:?} exists"))),
            ContextPut::Full => return Err(invalid(format!("the board keeps at most {MAX_CONTEXTS} contexts"))),
            ContextPut::Missing => {
                return Err(ContextError::Missing(format!("no context {id:?}; give a name and a spec to create one")));
            }
        };
        Ok((out.0, out.1, self.changed()))
    }

    pub fn delete_context(&self, id: &str) -> Result<u64, ContextError> {
        check_id(id)?;
        let _in_order = self.context_writes.lock().unwrap_or_else(|e| e.into_inner());
        if !self.store.context_delete(id)? {
            return Err(ContextError::Missing(format!("no context {id:?}")));
        }
        Ok(self.changed())
    }
}
