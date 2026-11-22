use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{BoardRemoteConfig, Config, with_defaults};
use crate::pools::PoolConfig;
use crate::{field, json_patch, steps};

const EMPTY_FILE: &str = "{}\n";
pub const MEMBER_CHECK_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(String);

impl Revision {
    fn of(bytes: Option<&[u8]>) -> Revision {
        Revision(match bytes {
            Some(bytes) => Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect(),
            None => "absent".into(),
        })
    }
}

#[derive(Debug)]
pub struct Stale {
    path: PathBuf,
}

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} changed on disk since it was read, so it was not written; try again", self.path.display())
    }
}

impl std::error::Error for Stale {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    fn from_error(error: &anyhow::Error) -> FieldError {
        FieldError { field: field::of(error).to_string(), message: format!("{error:#}") }
    }
}

#[derive(Debug, Serialize)]
pub struct Rejected {
    pub errors: Vec<FieldError>,
}

impl Rejected {
    fn at(field: String, message: impl Into<String>) -> Rejected {
        Rejected { errors: vec![FieldError { field, message: message.into() }] }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, FieldError { field, message }) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            if !message.starts_with(field.as_str()) {
                write!(f, "{field}: ")?;
            }
            f.write_str(message)?;
        }
        Ok(())
    }
}

impl std::error::Error for Rejected {}

#[derive(Clone, Debug)]
pub enum Edit {
    Set { path: Vec<String>, value: Value },
    Unset { path: Vec<String> },
}

#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    target: PathBuf,
    text: Option<String>,
    revision: Revision,
}

#[derive(Debug)]
pub struct Draft<'a> {
    document: &'a Document,
    text: Option<String>,
    config: Config,
}

impl Document {
    pub fn read(path: &Path) -> Result<Document> {
        let target = resolve(path)?;
        let text = match std::fs::read_to_string(&target) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let revision = Revision::of(text.as_deref().map(str::as_bytes));
        Ok(Document { path: path.to_path_buf(), target, text, revision })
    }

    pub fn location(path: &Path) -> PathBuf {
        steps::real(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
    }

    pub fn revision(&self) -> &Revision {
        &self.revision
    }

    pub fn exists(&self) -> bool {
        self.text.is_some()
    }

    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or(EMPTY_FILE)
    }

    pub fn value(&self) -> Result<Value, Rejected> {
        parse(self.text())
    }

    pub fn config(&self) -> Result<Config, Rejected> {
        check(self.value()?)
    }

    pub fn check_revision(&self, seen: &Revision) -> Result<(), Stale> {
        if *seen == self.revision { Ok(()) } else { Err(self.stale()) }
    }

    fn stale(&self) -> Stale {
        Stale { path: self.path.clone() }
    }

    pub fn edit(&self, edits: &[Edit]) -> Result<Draft<'_>, Rejected> {
        let mut new = self.value()?;
        for edit in edits {
            let merged = with_defaults(new.clone());
            // Deserialized without validation, so a reference a later edit resolves keeps serde's defaults.
            let in_effect = serde_json::from_value::<Config>(merged.clone()).map_or(merged, |c| effective(&c));
            let current = with_inherited(in_effect);
            match edit {
                Edit::Set { path, value } => set(&mut new, Some(&current), path, 0, value)?,
                Edit::Unset { path } => {
                    if !unset(&mut new, Some(&current), path) && lookup(&current, path).is_none() {
                        return Err(Rejected::at(field::display(path), "no such key in the config"));
                    }
                }
            }
        }
        self.rewrite(new)
    }

    pub fn rewrite(&self, new: Value) -> Result<Draft<'_>, Rejected> {
        let old = self.value()?;
        if new == old {
            return Ok(Draft { document: self, text: self.text.clone(), config: check(new)? });
        }
        self.replace(json_patch::patch(self.text(), &old, &new))
    }

    pub fn replace(&self, text: String) -> Result<Draft<'_>, Rejected> {
        let config = check(parse(&text)?)?;
        Ok(Draft { document: self, text: Some(text), config })
    }
}

impl Draft<'_> {
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn is_changed(&self) -> bool {
        self.text != self.document.text
    }

    pub fn changed_pools(&self) -> Vec<&str> {
        let before = self.document.value().unwrap_or_default();
        self.config
            .pools
            .iter()
            .filter(|(name, pool)| {
                PoolConfig::deserialize(&before["pools"][name.as_str()]).ok().as_ref() != Some(*pool)
            })
            .map(|(name, _)| name.as_str())
            .collect()
    }

    pub fn save(self) -> Result<Revision> {
        let Document { path, target, text: before, revision } = self.document;
        let stale = || anyhow::Error::new(self.document.stale());
        if resolve(path)? != *target {
            return Err(stale());
        }
        if !self.is_changed() {
            return if Document::read(path)?.revision == *revision { Ok(revision.clone()) } else { Err(stale()) };
        }
        let after = self.text.as_deref().unwrap_or(EMPTY_FILE).as_bytes();
        if let Some(dir) = target.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let _lock = steps::lock_beside(target)?;
        if !steps::replace_unchanged(target, before.as_deref().map(str::as_bytes), after)? {
            return Err(stale());
        }
        Ok(Revision::of(Some(after)))
    }
}

pub fn check_members(config: &Config, pools: &[&str], caps: &Value) -> Result<(), Rejected> {
    let errors: Vec<FieldError> = pools
        .iter()
        .filter_map(|name| crate::pools::check_members(name, config.pools.get(*name)?, caps).err())
        .map(|e| FieldError::from_error(&e))
        .collect();
    if errors.is_empty() { Ok(()) } else { Err(Rejected { errors }) }
}

pub fn effective(config: &Config) -> Value {
    let mut out = json!(config);
    if let Some(remote) = &config.board.remote {
        out["board"]["remote"] = remote_value(remote);
    }
    for (name, pool) in &config.pools {
        let shown = &mut out["pools"][name];
        shown["mixed"] = json!(pool.mixed);
        shown["driver"] = json!(pool.driver);
        shown["model"] = json!(pool.model);
        shown["effort"] = json!(pool.effort);
        shown["moveOnLimit"] = json!(pool.move_on_limit);
        for (i, member) in pool.members.iter().enumerate() {
            shown["members"][i]["model"] = json!(member.model);
            shown["members"][i]["effort"] = json!(member.effort);
        }
    }
    out
}

fn remote_value(remote: &BoardRemoteConfig) -> Value {
    json!({
        "mode": remote.mode,
        "origin": remote.origin,
        "allowUsers": remote.allow_users,
        "requirePairing": remote.require_pairing,
    })
}

pub fn lookup(effective: &Value, path: &[String]) -> Option<Value> {
    let find = |all: &Value| path.iter().try_fold(all, |at, key| child(at, key)).cloned();
    find(effective).or_else(|| find(&with_inherited(effective.clone())))
}

fn with_inherited(mut all: Value) -> Value {
    if let Some(remote) = all.pointer_mut("/board/remote").filter(|v| v.is_null()) {
        *remote = remote_value(&BoardRemoteConfig::default());
    }
    let wake = all.get("wake").cloned().unwrap_or_default();
    let roles = all.get_mut("roles").and_then(Value::as_object_mut);
    for role in roles.into_iter().flat_map(|roles| roles.values_mut()) {
        let children = role.get_mut("children").and_then(Value::as_object_mut);
        for policy in children.into_iter().flat_map(|c| c.values_mut()).filter_map(Value::as_object_mut) {
            let own = policy.entry("wake").or_insert(Value::Null);
            if own.is_null() {
                *own = wake.clone();
            }
        }
    }
    all
}

fn child<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key),
        Value::Array(items) => items.get(index(key)?),
        _ => None,
    }
}

fn index(key: &str) -> Option<usize> {
    key.parse().ok().filter(|i: &usize| i.to_string() == key)
}

// A merge replaces arrays whole, so an edit inside an inherited array starts from a copy of it.
fn inherited(current: Option<&Value>) -> Value {
    current.filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!({}))
}

fn set(slot: &mut Value, current: Option<&Value>, path: &[String], at: usize, value: &Value) -> Result<(), Rejected> {
    let Some(key) = path.get(at) else {
        *slot = value.clone();
        return Ok(());
    };
    if slot.is_null() {
        *slot = inherited(current);
    }
    let next = match slot {
        Value::Object(map) => map.entry(key.clone()).or_insert(Value::Null),
        Value::Array(items) => {
            let len = items.len();
            let Some(i) = index(key).filter(|i| *i <= len) else {
                let message = format!("not an index of this list; use 0 to {len}, where {len} appends");
                return Err(Rejected::at(field::display(&path[..=at]), message));
            };
            if i == len {
                items.push(Value::Null);
            }
            &mut items[i]
        }
        _ => {
            let message = format!("must be a JSON object to hold {}", field::segment(key));
            return Err(Rejected::at(field::display(&path[..at]), message));
        }
    };
    set(next, current.and_then(|v| child(v, key)), path, at + 1, value)
}

fn unset(slot: &mut Value, current: Option<&Value>, path: &[String]) -> bool {
    let Some((key, rest)) = path.split_first() else { return false };
    if slot.is_null() {
        let mut fresh = inherited(current);
        let removed = unset(&mut fresh, current, path);
        if removed {
            *slot = fresh;
        }
        return removed;
    }
    let below = current.and_then(|v| child(v, key));
    match slot {
        Value::Object(map) if rest.is_empty() => map.shift_remove(key).is_some(),
        Value::Object(map) => {
            if let Some(value) = map.get_mut(key) {
                return unset(value, below, rest);
            }
            let mut value = Value::Null;
            let removed = unset(&mut value, below, rest);
            if removed {
                map.insert(key.clone(), value);
            }
            removed
        }
        Value::Array(items) => match index(key).filter(|i| *i < items.len()) {
            Some(i) if rest.is_empty() => {
                items.remove(i);
                true
            }
            Some(i) => unset(&mut items[i], below, rest),
            None => false,
        },
        _ => false,
    }
}

fn parse(text: &str) -> Result<Value, Rejected> {
    serde_json::from_str(text).map_err(|e| Rejected::at(String::new(), format!("not valid JSON: {e}")))
}

fn check(value: Value) -> Result<Config, Rejected> {
    Config::from_value(value).map_err(|e| Rejected { errors: vec![FieldError::from_error(&e)] })
}

fn resolve(path: &Path) -> Result<PathBuf> {
    if path.is_symlink() && !path.exists() {
        bail!("{} is a symlink to a file that does not exist; create that file or remove the link", path.display());
    }
    Ok(steps::real(path))
}
