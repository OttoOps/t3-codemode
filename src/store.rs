use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::LinkParams;
use crate::tokens::RunTokens;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS kv (
  ns TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL,
  PRIMARY KEY (ns, key));
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS token (
  id INTEGER PRIMARY KEY CHECK (id = 1), access_token TEXT NOT NULL, client_id TEXT NOT NULL,
  expires_at TEXT NOT NULL, obtained_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS nodes (thread TEXT PRIMARY KEY, role TEXT NOT NULL, at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS links (
  child TEXT PRIMARY KEY, parent TEXT NOT NULL, closed INTEGER NOT NULL, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS links_parent ON links (parent);
CREATE TABLE IF NOT EXISTS pending (
  parent TEXT NOT NULL, label TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY (parent, label));
CREATE TABLE IF NOT EXISTS messages (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, from_id TEXT NOT NULL, to_id TEXT NOT NULL,
  kind TEXT NOT NULL, body TEXT NOT NULL, at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS messages_from ON messages (from_id, seq);
CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT, to_id TEXT NOT NULL, text TEXT NOT NULL,
  handle TEXT NOT NULL, queued TEXT NOT NULL, allow_closed INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS waiting (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, owner TEXT NOT NULL, id TEXT NOT NULL, text TEXT NOT NULL,
  from_label TEXT, link TEXT, opened_at TEXT NOT NULL, resolved_at TEXT, note TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS waiting_open ON waiting (owner, id) WHERE resolved_at IS NULL;
CREATE TABLE IF NOT EXISTS asks (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, thread TEXT NOT NULL, open INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS merge_waits (
  repo TEXT NOT NULL, pr INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY (repo, pr));
CREATE TABLE IF NOT EXISTS contexts (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pool_members (
  pool TEXT NOT NULL, instance TEXT NOT NULL, credit INTEGER NOT NULL, PRIMARY KEY (pool, instance));
CREATE TABLE IF NOT EXISTS instance_limits (instance TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pool_picks (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, pool TEXT NOT NULL, instance TEXT NOT NULL, at TEXT NOT NULL,
  handle TEXT, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS pool_picks_pool ON pool_picks (pool, seq);
CREATE INDEX IF NOT EXISTS pool_picks_handle ON pool_picks (handle) WHERE handle IS NOT NULL;
CREATE TABLE IF NOT EXISTS limit_stops (
  thread TEXT NOT NULL, run TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY (thread, run));
CREATE TABLE IF NOT EXISTS run_tokens (
  thread TEXT NOT NULL, run TEXT NOT NULL, instance TEXT NOT NULL, driver TEXT, at TEXT NOT NULL,
  input INTEGER, output INTEGER, PRIMARY KEY (thread, run));
CREATE INDEX IF NOT EXISTS run_tokens_at ON run_tokens (at);
CREATE TABLE IF NOT EXISTS new_issues (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL UNIQUE, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pr_seen (
  repo TEXT NOT NULL, pr INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY (repo, pr));
CREATE TABLE IF NOT EXISTS instance_folders (
  instance TEXT PRIMARY KEY, driver TEXT NOT NULL, folder TEXT NOT NULL, at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS slot_resolutions (
  key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, data TEXT NOT NULL, at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pr_events (
  repo TEXT NOT NULL, pr INTEGER NOT NULL, event TEXT NOT NULL, ticket TEXT NOT NULL, at TEXT NOT NULL,
  PRIMARY KEY (repo, pr, event, ticket));
CREATE TABLE IF NOT EXISTS pairing_tokens (
  token_hash TEXT PRIMARY KEY, label TEXT, created_at TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT);
CREATE INDEX IF NOT EXISTS pairing_tokens_expires ON pairing_tokens (expires_at);
CREATE TABLE IF NOT EXISTS board_sessions (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, token_hash TEXT NOT NULL UNIQUE, label TEXT,
  created_at TEXT NOT NULL, last_seen_at TEXT NOT NULL, expires_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS board_sessions_expires ON board_sessions (expires_at);
CREATE TABLE IF NOT EXISTS flow_dismissed_edges (
  from_key TEXT NOT NULL, kind TEXT NOT NULL, to_key TEXT NOT NULL, at TEXT NOT NULL,
  PRIMARY KEY (from_key, kind, to_key));
"#;

const KEEP_MESSAGES: i64 = 5000;
pub const KEEP_PICKS: i64 = 2000;
const UNSETTLED: &str = "json_extract(data, '$.settledAt') IS NULL";
const REGULAR_ASK: &str = "COALESCE(json_extract(data, '$.kind'), '') != 'triageDag'";
const TRIAGE_ASK: &str = "json_extract(data, '$.kind') = 'triageDag'";

pub type WaitingRow = (WaitingItem, Option<Value>, Option<(String, Option<String>)>, i64);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub child: String,
    pub parent: String,
    pub label: String,
    pub role: String,
    pub parent_role: Option<String>,
    pub params: LinkParams,
    pub link: Option<String>,
    pub opened: String,
    pub last_up: String,
    pub last_seen: Option<String>,
    pub last_nudge: Option<String>,
    pub nudges: i64,
    pub escalated: bool,
    pub reported: Vec<String>,
    pub gate: Option<String>,
    pub gate_since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_pr: Option<(String, u64)>,
    pub closed: bool,
    pub closed_at: Option<String>,
    pub closed_how: Option<String>,
    pub unread: Vec<String>,
    pub bad_run: Option<String>,
    #[serde(default)]
    pub settled_at: Option<String>,
    #[serde(default)]
    pub restored_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<String>,
    #[serde(flatten)]
    pub identity: Identity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<PrBrief>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<LinkPr>,
    #[serde(skip)]
    pub base: LinkBase,
}

/// The stored row a [`Link`] was read from or last saved as.
///
/// [`Store::link_save`] compares it with the row it finds, so a save keeps the fields another
/// process (`t3cm stdio` beside `t3cm serve`) changed since, and writes only the fields this copy
/// changed. It never takes part in equality.
#[derive(Default)]
pub struct LinkBase(Mutex<Option<String>>);

impl LinkBase {
    fn of(raw: &str) -> LinkBase {
        LinkBase(Mutex::new(Some(raw.to_string())))
    }

    fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, raw: String) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(raw);
    }
}

impl Clone for LinkBase {
    fn clone(&self) -> LinkBase {
        LinkBase(Mutex::new(self.get()))
    }
}

impl std::fmt::Debug for LinkBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkBase")
    }
}

impl PartialEq for LinkBase {
    fn eq(&self, _: &LinkBase) -> bool {
        true
    }
}

fn merge_fields(base: &str, ours: &str, theirs: &str) -> Result<String> {
    let (Value::Object(base), Value::Object(ours), Value::Object(theirs)) =
        (serde_json::from_str(base)?, serde_json::from_str(ours)?, serde_json::from_str(theirs)?)
    else {
        return Ok(ours.to_string());
    };
    let mut out = theirs.clone();
    let keys: std::collections::BTreeSet<&String> = base.keys().chain(ours.keys()).collect();
    let set = |row: &serde_json::Map<String, Value>, key: &str| row.get(key).filter(|v| !v.is_null()).cloned();
    for key in keys {
        if set(&base, key) == set(&ours, key) {
            continue;
        }
        let merged = match (base.get(key), ours.get(key), out.get(key)) {
            (Some(Value::Array(b)), Some(Value::Array(o)), Some(Value::Array(t)))
                if SET_FIELDS.contains(&key.as_str()) =>
            {
                let mut items: Vec<Value> = t.iter().filter(|v| o.contains(v) || !b.contains(v)).cloned().collect();
                items.extend(o.iter().filter(|v| !b.contains(v) && !t.contains(v)).cloned());
                Some(Value::Array(items))
            }
            (_, ours, _) => ours.cloned(),
        };
        match merged {
            Some(v) => out.insert(key.clone(), v),
            None => out.remove(key),
        };
    }
    let closes = |row: &serde_json::Map<String, Value>| row.get("closed") == Some(&Value::Bool(true));
    let closer = match (closes(&base), closes(&ours), closes(&theirs)) {
        (false, true, _) => Some(&ours),
        (false, false, true) => Some(&theirs),
        _ => None,
    };
    if let Some(closer) = closer {
        for key in CLOSE_FIELDS {
            match closer.get(*key) {
                Some(v) => out.insert(key.to_string(), v.clone()),
                None => out.remove(*key),
            };
        }
    }
    Ok(Value::Object(out).to_string())
}

const SET_FIELDS: &[&str] = &["unread", "reported"];
const CLOSE_FIELDS: &[&str] = &["gate", "gateSince", "gatePr"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrBrief {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkPr {
    pub repo: String,
    pub number: u64,
    pub head: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_ci: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub followups: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl Identity {
    pub fn or(self, other: Identity) -> Identity {
        Identity {
            issue: self.issue.or(other.issue),
            parent_issue: self.parent_issue.or(other.parent_issue),
            part: self.part.or(other.part),
        }
    }
}

impl Link {
    pub fn closed_since(&self) -> Option<&str> {
        match (self.closed_at.as_deref(), self.restored_at.as_deref()) {
            (Some(c), Some(r)) if r > c => Some(r),
            (c, _) => c,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunTokenRecord {
    pub thread: String,
    pub run: String,
    pub instance: String,
    pub driver: Option<String>,
    pub at: String,
    pub tokens: Option<RunTokens>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Bind {
    Kept(Link),
    Saved(Link),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub parent: String,
    pub label: String,
    pub role: String,
    pub params: LinkParams,
    pub at: String,
    pub reported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(flatten)]
    pub identity: Identity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<PrBrief>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Message {
    pub handle: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub at: String,
    pub body: String,
}

impl Message {
    pub fn head(&self, max: usize) -> String {
        let flat = self.body.split_whitespace().collect::<Vec<_>>().join(" ");
        flat.chars().take(max).collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeWait {
    pub repo: String,
    pub pr: u64,
    pub head: String,
    pub pull_request_id: String,
    pub method: Option<String>,
    pub thread: Option<String>,
    pub label: Option<String>,
    pub merge_state: String,
    pub at: String,
    #[serde(default)]
    pub status: WaitStatus,
    #[serde(default)]
    pub approval: String,
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advancing: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WaitStatus {
    #[default]
    Waiting,
    Sent,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrSeen {
    pub repo: String,
    pub pr: u64,
    pub state: String,
    pub head: String,
    pub queued: bool,
    pub since: String,
    pub tickets: Vec<(String, String)>,
    pub at: String,
    #[serde(default)]
    pub left: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<String>,
    #[serde(default)]
    pub rev: u64,
}

pub struct Wake<'a> {
    pub repo: &'a str,
    pub pr: u64,
    pub event: Option<&'a str>,
    pub ticket: &'a str,
    pub from: &'a str,
    pub kind: &'a str,
    pub body: &'a str,
    pub at: &'a str,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub until: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimitStop {
    pub limit: bool,
    pub seen_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PickRecord {
    pub pool: String,
    pub instance_id: String,
    pub model: String,
    pub strategy: String,
    pub reason: String,
    pub skipped: Value,
    pub at: String,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewIssue {
    pub url: String,
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub title: Option<String>,
    pub note: Option<String>,
    pub thread: String,
    pub thread_link: Option<String>,
    pub thread_title: Option<String>,
    pub at: String,
    #[serde(default)]
    pub dismissed_at: Option<String>,
    #[serde(default)]
    pub restored_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboxItem {
    pub id: i64,
    pub to: String,
    pub text: String,
    pub handle: String,
    pub queued: String,
    pub allow_closed: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct WaitingItem {
    pub id: String,
    pub text: String,
    pub from: Option<String>,
    pub link: Option<String>,
    pub at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContextSpec {
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardContext {
    pub id: String,
    pub name: String,
    pub spec: ContextSpec,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContextPut {
    Created(BoardContext),
    Updated(BoardContext),
    NameTaken(String),
    Full,
    Missing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredToken {
    pub access_token: String,
    pub client_id: String,
    pub expires_at: String,
    pub obtained_at: String,
}

pub struct Rebind<'a> {
    pub old: &'a str,
    pub new: &'a str,
    pub link: Option<&'a str>,
    pub board: &'a str,
    pub card: Option<(i64, &'a str)>,
    pub pick: i64,
    pub summary: &'a str,
    pub at: &'a str,
}

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
    reach: std::sync::OnceLock<std::sync::Arc<crate::reach::Reach>>,
    #[cfg(feature = "test-faults")]
    writes: std::sync::Arc<Writes>,
}

/// Panic payload a store unwinds with when [`Store::set_fault`] crashes it.
#[cfg(feature = "test-faults")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreCrash {
    pub write: usize,
}

#[cfg(feature = "test-faults")]
#[derive(Default)]
struct Writes {
    seen: std::sync::atomic::AtomicUsize,
    crash_at: std::sync::atomic::AtomicUsize,
    crashed: std::sync::atomic::AtomicUsize,
}

#[cfg(feature = "test-faults")]
impl Writes {
    fn hook(conn: &mut Connection) -> Result<std::sync::Arc<Writes>> {
        use std::sync::atomic::Ordering::SeqCst;
        let writes = std::sync::Arc::new(Writes::default());
        let w = writes.clone();
        conn.commit_hook(Some(move || {
            let n = w.seen.fetch_add(1, SeqCst) + 1;
            let crash = w.crashed.load(SeqCst) != 0 || n == w.crash_at.load(SeqCst);
            if crash {
                let _ = w.crashed.compare_exchange(0, n, SeqCst, SeqCst);
            }
            crash
        }))?;
        Ok(writes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceFolder {
    pub instance: String,
    pub driver: String,
    pub folder: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSession {
    #[serde(skip)]
    pub seq: i64,
    pub id: String,
    pub label: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissedEdgeRow {
    pub from_key: String,
    pub kind: String,
    pub to_key: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedeemResult {
    Redeemed(BoardSession),
    NotFound,
    AlreadyUsed,
    Expired,
}

fn is_unexpired(now_iso: &str, expires_at: &str) -> bool {
    matches!(
        (crate::clock::parse_ts(now_iso), crate::clock::parse_ts(expires_at)),
        (Some(now), Some(exp)) if now < exp
    )
}

fn handle_seq(handle: &str) -> Option<i64> {
    handle.strip_prefix('m')?.parse().ok()
}

impl Store {
    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(SCHEMA)?;
        let has_data: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('waiting') WHERE name = 'data'", [], |r| r.get(0))?;
        if has_data == 0 {
            conn.execute_batch("ALTER TABLE waiting ADD COLUMN data TEXT")?;
        }
        let has_allow_closed: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('outbox') WHERE name = 'allow_closed'", [], |r| {
                r.get(0)
            })?;
        if has_allow_closed == 0 {
            conn.execute_batch("ALTER TABLE outbox ADD COLUMN allow_closed INTEGER NOT NULL DEFAULT 0")?;
        }
        Ok(())
    }

    pub fn open(dir: &Path) -> Result<Store> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        crate::platform::private_dir(dir)?;
        let path = dir.join("store.db");
        let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        crate::platform::private_file(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init_schema(&conn)?;
        Self::build(conn, path)
    }

    #[cfg_attr(not(feature = "test-faults"), allow(unused_mut))]
    fn build(mut conn: Connection, path: PathBuf) -> Result<Store> {
        #[cfg(feature = "test-faults")]
        let writes = Writes::hook(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
            path,
            reach: std::sync::OnceLock::new(),
            #[cfg(feature = "test-faults")]
            writes,
        })
    }

    /// Open a private in-memory SQLite store with the same schema and migrations as [`Store::open`].
    ///
    /// For single-process use only: SQLite `:memory:` databases belong to a single connection, do
    /// not persist across reopen, and cannot be shared with a peer handle or second process. Use
    /// [`Store::open`] for crash recovery, reopen, and multi-process contention tests.
    pub fn open_memory() -> Result<Store> {
        let conn = Connection::open_in_memory().context("opening in-memory store")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init_schema(&conn)?;
        Self::build(conn, PathBuf::from(":memory:"))
    }

    /// Count this store's field-merged link saves in `reach`, once; later calls keep the first.
    pub fn report_to(&self, reach: std::sync::Arc<crate::reach::Reach>) {
        let _ = self.reach.set(reach);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.lock();
        let out = f(&conn);
        self.crashed();
        Ok(out?)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        self.crashed();
        conn
    }

    fn commit(&self, tx: rusqlite::Transaction) -> Result<()> {
        let out = tx.commit();
        self.crashed();
        Ok(out?)
    }

    #[inline]
    fn crashed(&self) {
        #[cfg(feature = "test-faults")]
        {
            let write = self.writes.crashed.load(std::sync::atomic::Ordering::SeqCst);
            if write != 0 {
                std::panic::panic_any(StoreCrash { write });
            }
        }
    }

    /// Crash this store at its `write`-th committed write since it opened (1-based), or never.
    ///
    /// The crashing write rolls back, and that call and every later one on this store panic with
    /// a [`StoreCrash`] payload, the way a process that dies mid-write loses the write and stops.
    /// Reopen the same directory to recover.
    #[cfg(feature = "test-faults")]
    pub fn set_fault(&self, write: Option<usize>) {
        self.writes.crash_at.store(write.unwrap_or(0), std::sync::atomic::Ordering::SeqCst);
    }

    /// Committed writes since this store opened, the index [`Store::set_fault`] counts in.
    #[cfg(feature = "test-faults")]
    pub fn writes(&self) -> usize {
        self.writes.seen.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn kv_get(&self, ns: &str, key: &str) -> Result<Option<Value>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT value FROM kv WHERE ns = ?1 AND key = ?2", params![ns, key], |r| r.get(0)).optional()
        })?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn kv_set(&self, ns: &str, key: &str, value: &Value, at: &str) -> Result<()> {
        let s = value.to_string();
        self.with(|c| {
            c.execute(
                "INSERT INTO kv (ns, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (ns, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![ns, key, s, at],
            )
        })?;
        Ok(())
    }

    pub fn kv_del(&self, ns: &str, key: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM kv WHERE ns = ?1 AND key = ?2", params![ns, key]))? > 0)
    }

    pub fn kv_list(&self, ns: &str, prefix: &str) -> Result<Vec<String>> {
        self.with(|c| {
            let mut st =
                c.prepare("SELECT key FROM kv WHERE ns = ?1 AND substr(key, 1, length(?2)) = ?2 ORDER BY key")?;
            st.query_map(params![ns, prefix], |r| r.get(0))?.collect()
        })
    }

    pub fn meta_prefixed(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT key, value FROM meta WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")?;
            st.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional())
    }

    pub fn meta_del(&self, key: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM meta WHERE key = ?1", [key]))? > 0)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
        })?;
        Ok(())
    }

    pub fn instance_folder(&self, instance: &str) -> Result<Option<InstanceFolder>> {
        self.with(|c| {
            c.query_row(
                "SELECT instance, driver, folder, at FROM instance_folders WHERE instance = ?1",
                [instance],
                |r| Ok(InstanceFolder { instance: r.get(0)?, driver: r.get(1)?, folder: r.get(2)?, at: r.get(3)? }),
            )
            .optional()
        })
    }

    pub fn instance_folders(&self) -> Result<Vec<InstanceFolder>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT instance, driver, folder, at FROM instance_folders ORDER BY instance")?;
            st.query_map([], |r| {
                Ok(InstanceFolder { instance: r.get(0)?, driver: r.get(1)?, folder: r.get(2)?, at: r.get(3)? })
            })?
            .collect()
        })
    }

    pub fn instance_folder_set(&self, f: &InstanceFolder) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO instance_folders (instance, driver, folder, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (instance) DO UPDATE SET driver = excluded.driver, folder = excluded.folder, at = excluded.at",
                params![f.instance, f.driver, f.folder, f.at],
            )
        })?;
        Ok(())
    }

    pub fn slot_resolution(&self, key: &str) -> Result<Option<(String, Value)>> {
        let row: Option<(String, String)> = self.with(|c| {
            c.query_row("SELECT fingerprint, data FROM slot_resolutions WHERE key = ?1", [key], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })?;
        Ok(match row {
            Some((fp, data)) => Some((fp, serde_json::from_str(&data)?)),
            None => None,
        })
    }

    pub fn slot_resolution_set(&self, key: &str, fingerprint: &str, data: &Value, at: &str) -> Result<()> {
        let s = data.to_string();
        self.with(|c| {
            c.execute(
                "INSERT INTO slot_resolutions (key, fingerprint, data, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (key) DO UPDATE SET fingerprint = excluded.fingerprint, data = excluded.data, at = excluded.at",
                params![key, fingerprint, s, at],
            )
        })?;
        Ok(())
    }

    pub fn token_get(&self) -> Result<Option<StoredToken>> {
        self.with(|c| {
            c.query_row("SELECT access_token, client_id, expires_at, obtained_at FROM token WHERE id = 1", [], |r| {
                Ok(StoredToken {
                    access_token: r.get(0)?,
                    client_id: r.get(1)?,
                    expires_at: r.get(2)?,
                    obtained_at: r.get(3)?,
                })
            })
            .optional()
        })
    }

    pub fn token_set(&self, t: &StoredToken) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO token (id, access_token, client_id, expires_at, obtained_at) VALUES (1, ?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET access_token = excluded.access_token, client_id = excluded.client_id,
                 expires_at = excluded.expires_at, obtained_at = excluded.obtained_at",
                params![t.access_token, t.client_id, t.expires_at, t.obtained_at],
            )
        })?;
        Ok(())
    }

    pub fn token_clear(&self) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM token", []))?;
        Ok(())
    }

    pub fn node_role(&self, thread: &str) -> Result<Option<String>> {
        self.with(|c| c.query_row("SELECT role FROM nodes WHERE thread = ?1", [thread], |r| r.get(0)).optional())
    }

    pub fn node_set(&self, thread: &str, role: &str, at: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO nodes (thread, role, at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread) DO UPDATE SET role = excluded.role, at = excluded.at",
                params![thread, role, at],
            )
        })?;
        Ok(())
    }

    fn links_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Link>> {
        let raws: Vec<String> = self.with(|c| {
            let mut st = c.prepare(&format!("SELECT data FROM links {sql} ORDER BY rowid"))?;
            st.query_map(p, |r| r.get(0))?.collect()
        })?;
        raws.iter()
            .map(|s| {
                let ln: Link = serde_json::from_str(s)?;
                Ok(Link { base: LinkBase::of(s), ..ln })
            })
            .collect()
    }

    pub fn link_get(&self, child: &str) -> Result<Option<Link>> {
        Ok(self.links_where("WHERE child = ?1", [child])?.pop())
    }

    pub fn links_open(&self) -> Result<Vec<Link>> {
        self.links_where("WHERE closed = 0", [])
    }

    pub fn links_all(&self) -> Result<Vec<Link>> {
        self.links_where("", [])
    }

    pub fn children(&self, parent: &str, open_only: bool) -> Result<Vec<Link>> {
        if open_only {
            self.links_where("WHERE parent = ?1 AND closed = 0", [parent])
        } else {
            self.links_where("WHERE parent = ?1", [parent])
        }
    }

    /// Save a link, merged field by field with any change another process made since `ln` was read.
    pub fn link_save(&self, ln: &Link) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved = Self::save_link(&tx, ln)?;
        self.commit(tx)?;
        if let Some(saved) = saved {
            self.saved(ln, saved);
        }
        Ok(())
    }

    /// Save a closing link, drop its own held wakes when `purge`, and queue `last` (to, text, handle)
    /// in one transaction, so a crash never closes the link without its last wake.
    /// Returns how many held wakes it dropped, or `None` when the link moved away since it was read.
    pub fn link_close(
        &self,
        ln: &Link,
        purge: bool,
        last: Option<(&str, &str, &str)>,
        at: &str,
    ) -> Result<Option<usize>> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(saved) = Self::save_link(&tx, ln)? else { return Ok(None) };
        let dropped = if purge { tx.execute("DELETE FROM outbox WHERE to_id = ?1", [&ln.child])? } else { 0 };
        if let Some((to, text, handle)) = last {
            tx.execute(
                "INSERT INTO outbox (to_id, text, handle, queued) VALUES (?1, ?2, ?3, ?4)",
                params![to, text, handle, at],
            )?;
        }
        self.commit(tx)?;
        self.saved(ln, saved);
        Ok(Some(dropped))
    }

    fn save_link(tx: &rusqlite::Transaction, ln: &Link) -> Result<Option<(String, bool)>> {
        let ours = serde_json::to_string(ln)?;
        let theirs: Option<String> =
            tx.query_row("SELECT data FROM links WHERE child = ?1", [&ln.child], |r| r.get(0)).optional()?;
        let (data, merged) = match (ln.base.get(), theirs) {
            (Some(_), None) => return Ok(None),
            (Some(base), Some(theirs)) if base != theirs => (merge_fields(&base, &ours, &theirs)?, true),
            _ => (ours.clone(), false),
        };
        let row: Link = serde_json::from_str(&data)?;
        tx.execute(
            "INSERT INTO links (child, parent, closed, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (child) DO UPDATE SET parent = excluded.parent, closed = excluded.closed, data = excluded.data",
            params![row.child, row.parent, row.closed as i64, data],
        )?;
        Ok(Some((ours, merged)))
    }

    fn saved(&self, ln: &Link, (ours, merged): (String, bool)) {
        ln.base.set(ours);
        if merged && let Some(reach) = self.reach.get() {
            reach.hit(crate::reach::ReachPoint::LinkSaveMerged);
        }
    }

    pub fn link_bind(
        &self,
        child: &str,
        clear_pending: bool,
        build: impl FnOnce(Option<Link>) -> Bind,
    ) -> Result<Bind> {
        self.link_bind_launched(child, clear_pending, None, |current, _| build(current))
    }

    pub fn link_bind_launched(
        &self,
        child: &str,
        clear_pending: bool,
        launched_by: Option<&str>,
        build: impl FnOnce(Option<Link>, Option<&Pending>) -> Bind,
    ) -> Result<Bind> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> =
            tx.query_row("SELECT data FROM links WHERE child = ?1", [child], |r| r.get(0)).optional()?;
        let current = raw.map(|s| serde_json::from_str(&s)).transpose()?;
        let launch = match launched_by {
            Some(parent) => {
                let mut st = tx.prepare("SELECT data FROM pending WHERE parent = ?1 ORDER BY rowid")?;
                let raws: Vec<String> = st.query_map([parent], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                drop(st);
                raws.iter()
                    .map(|s| serde_json::from_str::<Pending>(s))
                    .collect::<serde_json::Result<Vec<_>>>()?
                    .into_iter()
                    .find(|p| p.thread.as_deref() == Some(child))
            }
            None => None,
        };
        let bind = build(current, launch.as_ref());
        if let Bind::Saved(ln) = &bind {
            tx.execute(
                "INSERT INTO links (child, parent, closed, data) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (child) DO UPDATE SET parent = excluded.parent, closed = excluded.closed, data = excluded.data",
                params![ln.child, ln.parent, ln.closed as i64, serde_json::to_string(ln)?],
            )?;
            tx.execute(
                "INSERT INTO nodes (thread, role, at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread) DO UPDATE SET role = excluded.role, at = excluded.at",
                params![ln.child, ln.role, ln.last_up],
            )?;
            if clear_pending {
                tx.execute("DELETE FROM pending WHERE parent = ?1 AND label = ?2", params![ln.parent, ln.label])?;
            }
            if let Some(p) = &launch {
                tx.execute("DELETE FROM pending WHERE parent = ?1 AND label = ?2", params![p.parent, p.label])?;
            }
        }
        self.commit(tx)?;
        Ok(bind)
    }

    pub fn pending_put(&self, p: &Pending) -> Result<()> {
        let data = serde_json::to_string(p)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO pending (parent, label, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (parent, label) DO UPDATE SET data = excluded.data",
                params![p.parent, p.label, data],
            )
        })?;
        Ok(())
    }

    pub fn pending_remove(&self, parent: &str, label: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM pending WHERE parent = ?1 AND label = ?2", params![parent, label]))?
            > 0)
    }

    pub fn pending_all(&self) -> Result<Vec<Pending>> {
        let raws: Vec<String> = self.with(|c| {
            let mut st = c.prepare("SELECT data FROM pending ORDER BY rowid")?;
            st.query_map([], |r| r.get(0))?.collect()
        })?;
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }

    pub fn merge_wait_put(&self, w: &MergeWait) -> Result<()> {
        let data = serde_json::to_string(w)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO merge_waits (repo, pr, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (repo, pr) DO UPDATE SET data = excluded.data",
                params![w.repo, w.pr as i64, data],
            )
        })?;
        Ok(())
    }

    pub fn merge_wait_remove(&self, repo: &str, pr: u64) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM merge_waits WHERE repo = ?1 AND pr = ?2", params![repo, pr as i64]))?
            > 0)
    }

    pub fn merge_waits(&self) -> Result<Vec<MergeWait>> {
        let raws: Vec<String> = self.with(|c| {
            let mut st = c.prepare("SELECT data FROM merge_waits ORDER BY rowid")?;
            st.query_map([], |r| r.get(0))?.collect()
        })?;
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }

    pub fn merge_wait_rows(&self) -> Result<Vec<(String, u64, MergeWait)>> {
        let rows: Vec<(String, i64, String)> = self.with(|c| {
            let mut st = c.prepare("SELECT repo, pr, data FROM merge_waits ORDER BY rowid")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
        })?;
        rows.into_iter().map(|(repo, pr, s)| Ok((repo, pr as u64, serde_json::from_str(&s)?))).collect()
    }

    pub fn merge_wait_get(&self, repo: &str, pr: u64) -> Result<Option<MergeWait>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT data FROM merge_waits WHERE repo = ?1 AND pr = ?2", params![repo, pr as i64], |r| {
                r.get(0)
            })
            .optional()
        })?;
        raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }

    pub fn merge_wait_swap(&self, w: &MergeWait) -> Result<bool> {
        let data = serde_json::to_string(w)?;
        let sql = "UPDATE merge_waits SET data = ?3 WHERE repo = ?1 AND pr = ?2 \
                   AND COALESCE(json_extract(data, '$.approval'), '') = ?4";
        Ok(self.with(|c| c.execute(sql, params![w.repo, w.pr as i64, data, w.approval]))? > 0)
    }

    pub fn merge_wait_retire(&self, w: &MergeWait) -> Result<bool> {
        let sql = "DELETE FROM merge_waits WHERE repo = ?1 AND pr = ?2 \
                   AND COALESCE(json_extract(data, '$.approval'), '') = ?3";
        Ok(self.with(|c| c.execute(sql, params![w.repo, w.pr as i64, w.approval]))? > 0)
    }

    /// Save a pull request's last-seen state under a revision no earlier save in this store has used,
    /// so a row deleted and tracked again never repeats one.
    pub fn pr_seen_put(&self, p: &PrSeen) -> Result<()> {
        let data = serde_json::to_string(p)?;
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let rev: i64 = tx.query_row(
            "INSERT INTO meta (key, value) VALUES ('pr_seen_rev',
                 (SELECT COALESCE(MAX(CAST(json_extract(data, '$.rev') AS INTEGER)), 0) + 1 FROM pr_seen))
             ON CONFLICT (key) DO UPDATE SET value = CAST(value AS INTEGER) + 1
             RETURNING CAST(value AS INTEGER)",
            [],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO pr_seen (repo, pr, data) VALUES (?1, ?2, json_set(?3, '$.rev', ?4))
             ON CONFLICT (repo, pr) DO UPDATE SET data = excluded.data",
            params![p.repo, p.pr as i64, data, rev],
        )?;
        self.commit(tx)
    }

    pub fn pr_seen_all(&self) -> Result<Vec<PrSeen>> {
        let raws: Vec<String> = self.with(|c| {
            let mut st = c.prepare("SELECT data FROM pr_seen ORDER BY rowid")?;
            st.query_map([], |r| r.get(0))?.collect()
        })?;
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }

    pub fn pr_seen_get(&self, repo: &str, pr: u64) -> Result<Option<PrSeen>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT data FROM pr_seen WHERE repo = ?1 AND pr = ?2", params![repo, pr as i64], |r| r.get(0))
                .optional()
        })?;
        raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }

    pub fn pr_seen_any_queued(&self) -> Result<bool> {
        let sql = "SELECT EXISTS (SELECT 1 FROM pr_seen WHERE json_extract(data, '$.queued') = 1 \
                   AND json_extract(data, '$.state') = 'OPEN')";
        self.with(|c| c.query_row(sql, [], |r| r.get(0)))
    }

    pub fn pr_seen_remove(&self, repo: &str, pr: u64) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM pr_seen WHERE repo = ?1 AND pr = ?2", params![repo, pr as i64]))?;
        Ok(())
    }

    pub fn pr_event_seen(&self, repo: &str, pr: u64, event: &str, ticket: &str) -> Result<bool> {
        let sql = "SELECT EXISTS (SELECT 1 FROM pr_events WHERE repo = ?1 AND pr = ?2 AND event = ?3 AND ticket = ?4)";
        self.with(|c| c.query_row(sql, params![repo, pr as i64, event, ticket], |r| r.get(0)))
    }

    pub fn pr_events_all(&self) -> Result<Vec<(String, u64, String, String)>> {
        let rows: Vec<(String, i64, String, String)> = self.with(|c| {
            let mut st = c.prepare("SELECT repo, pr, event, ticket FROM pr_events ORDER BY rowid")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect()
        })?;
        Ok(rows.into_iter().map(|(repo, pr, event, ticket)| (repo, pr as u64, event, ticket)).collect())
    }

    pub fn pr_event_mark(&self, repo: &str, pr: u64, event: &str, ticket: &str, at: &str) -> Result<bool> {
        let sql = "INSERT OR IGNORE INTO pr_events (repo, pr, event, ticket, at) VALUES (?1, ?2, ?3, ?4, ?5)";
        Ok(self.with(|c| c.execute(sql, params![repo, pr as i64, event, ticket, at]))? > 0)
    }

    pub fn pr_event_wake(&self, w: &Wake, text: impl FnOnce(&str) -> String) -> Result<Option<String>> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(event) = w.event {
            let fresh = tx.execute(
                "INSERT OR IGNORE INTO pr_events (repo, pr, event, ticket, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![w.repo, w.pr as i64, event, w.ticket, w.at],
            )? > 0;
            if !fresh {
                return Ok(None);
            }
        }
        let handle = Self::msg_insert(&tx, w.from, w.ticket, w.kind, w.body, w.at)?;
        tx.execute(
            "INSERT INTO outbox (to_id, text, handle, queued) VALUES (?1, ?2, ?3, ?4)",
            params![w.ticket, text(&handle), handle, w.at],
        )?;
        self.commit(tx)?;
        Ok(Some(handle))
    }

    pub fn msg_new(&self, from: &str, to: &str, kind: &str, body: &str, at: &str) -> Result<String> {
        self.with(|c| Self::msg_insert(c, from, to, kind, body, at))
    }

    fn msg_insert(c: &Connection, from: &str, to: &str, kind: &str, body: &str, at: &str) -> rusqlite::Result<String> {
        c.execute(
            "INSERT INTO messages (from_id, to_id, kind, body, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![from, to, kind, body, at],
        )?;
        let seq = c.last_insert_rowid();
        if seq % 100 == 0 {
            c.execute("DELETE FROM messages WHERE seq <= ?1", [seq - KEEP_MESSAGES])?;
        }
        Ok(format!("m{seq}"))
    }

    fn msg_row(r: &rusqlite::Row) -> rusqlite::Result<Message> {
        Ok(Message {
            handle: format!("m{}", r.get::<_, i64>(0)?),
            from: r.get(1)?,
            to: r.get(2)?,
            kind: r.get(3)?,
            body: r.get(4)?,
            at: r.get(5)?,
        })
    }

    pub fn msg_get(&self, handle: &str) -> Result<Option<Message>> {
        let Some(seq) = handle_seq(handle) else { return Ok(None) };
        self.with(|c| {
            c.query_row("SELECT seq, from_id, to_id, kind, body, at FROM messages WHERE seq = ?1", [seq], Self::msg_row)
                .optional()
        })
    }

    pub fn msg_last_from(&self, from: &str) -> Result<Option<Message>> {
        self.with(|c| {
            c.query_row(
                "SELECT seq, from_id, to_id, kind, body, at FROM messages WHERE from_id = ?1 ORDER BY seq DESC LIMIT 1",
                [from],
                Self::msg_row,
            )
            .optional()
        })
    }

    pub fn msgs_recent(&self, kinds: &[&str], to: &[String], limit: usize) -> Result<Vec<Message>> {
        if kinds.is_empty() || to.is_empty() {
            return Ok(Vec::new());
        }
        let marks = |n: usize, from: usize| (from..from + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT seq, from_id, to_id, kind, body, at FROM messages WHERE kind IN ({}) AND to_id IN ({}) \
             ORDER BY seq DESC LIMIT {limit}",
            marks(kinds.len(), 1),
            marks(to.len(), kinds.len() + 1)
        );
        let args: Vec<&str> = kinds.iter().copied().chain(to.iter().map(String::as_str)).collect();
        self.with(|c| {
            let mut st = c.prepare(&sql)?;
            st.query_map(rusqlite::params_from_iter(args), Self::msg_row)?.collect()
        })
    }

    pub fn outbox_push(&self, to: &str, text: &str, handle: &str, queued: &str) -> Result<()> {
        self.outbox_push_with(to, text, handle, queued, false)
    }

    pub fn outbox_push_with(&self, to: &str, text: &str, handle: &str, queued: &str, allow_closed: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO outbox (to_id, text, handle, queued, allow_closed) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![to, text, handle, queued, allow_closed as i64],
            )
        })?;
        Ok(())
    }

    pub fn outbox_all(&self) -> Result<Vec<OutboxItem>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, to_id, text, handle, queued, allow_closed FROM outbox ORDER BY id")?;
            st.query_map([], |r| {
                Ok(OutboxItem {
                    id: r.get(0)?,
                    to: r.get(1)?,
                    text: r.get(2)?,
                    handle: r.get(3)?,
                    queued: r.get(4)?,
                    allow_closed: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect()
        })
    }

    pub fn outbox_drop_to(&self, to: &str) -> Result<usize> {
        self.with(|c| c.execute("DELETE FROM outbox WHERE to_id = ?1", [to]))
    }

    pub fn outbox_delete(&self, ids: &[i64]) -> Result<()> {
        self.with(|c| {
            for id in ids {
                c.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
            }
            Ok(())
        })
    }

    pub fn rebind(&self, m: &Rebind) -> Result<bool> {
        let Rebind { old, new, link, board, at, .. } = *m;
        fn read<T: serde::de::DeserializeOwned>(
            tx: &rusqlite::Transaction,
            sql: &str,
            p: impl rusqlite::Params,
        ) -> Result<Vec<(i64, T)>> {
            let mut st = tx.prepare(sql)?;
            let rows: Vec<(i64, String)> =
                st.query_map(p, |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows.into_iter().map(|(k, d)| Ok((k, serde_json::from_str(&d)?))).collect()
        }
        fn save(tx: &rusqlite::Transaction, ln: &Link) -> Result<()> {
            tx.execute(
                "INSERT INTO links (child, parent, closed, data) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (child) DO UPDATE SET parent = excluded.parent, closed = excluded.closed, data = excluded.data",
                params![ln.child, ln.parent, ln.closed as i64, serde_json::to_string(ln)?],
            )?;
            Ok(())
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut rebound = false;
        let own: Vec<(i64, Link)> = read(&tx, "SELECT rowid, data FROM links WHERE child = ?1 AND closed = 0", [old])?;
        if let Some((_, ln)) = own.into_iter().next() {
            let moved = Link {
                child: new.into(),
                link: link.map(String::from).or(ln.link.clone()),
                last_nudge: None,
                nudges: 0,
                escalated: false,
                bad_run: None,
                settled_at: None,
                ..ln
            };
            tx.execute("DELETE FROM links WHERE child = ?1", [old])?;
            save(&tx, &moved)?;
            rebound = true;
        }
        let children: Vec<(i64, Link)> =
            read(&tx, "SELECT rowid, data FROM links WHERE parent = ?1 AND closed = 0", [old])?;
        for (_, mut ln) in children {
            ln.parent = new.into();
            save(&tx, &ln)?;
            rebound = true;
        }
        tx.execute(
            "INSERT INTO nodes (thread, role, at) SELECT ?2, role, ?3 FROM nodes WHERE thread = ?1
             ON CONFLICT (thread) DO UPDATE SET role = excluded.role, at = excluded.at",
            params![old, new, at],
        )?;
        let pending: Vec<(i64, Pending)> = read(&tx, "SELECT rowid, data FROM pending", [])?;
        for (_, p) in pending {
            let mut q = p.clone();
            if q.parent == old {
                q.parent = new.into();
            }
            if q.thread.as_deref() == Some(old) {
                q.thread = Some(new.into());
                q.link = link.map(String::from).or(q.link);
                rebound = true;
            }
            if q != p {
                tx.execute("DELETE FROM pending WHERE parent = ?1 AND label = ?2", params![p.parent, p.label])?;
                tx.execute(
                    "INSERT INTO pending (parent, label, data) VALUES (?1, ?2, ?3)
                     ON CONFLICT (parent, label) DO UPDATE SET data = excluded.data",
                    params![q.parent, q.label, serde_json::to_string(&q)?],
                )?;
            }
        }
        let cards: Vec<(i64, Value)> = read(
            &tx,
            "SELECT seq, data FROM waiting WHERE owner = ?1 AND resolved_at IS NULL AND data IS NOT NULL",
            [board],
        )?;
        for (seq, mut d) in cards {
            let ticket = d["ticket"] == old && d["move"].is_null();
            let worker = d["worker"]["thread"] == old;
            if !ticket && !worker {
                continue;
            }
            if ticket {
                d["ticket"] = Value::from(new);
            }
            if worker {
                d["worker"]["thread"] = Value::from(new);
                if let Some(l) = link {
                    d["worker"]["link"] = Value::from(l);
                }
            }
            let relink = worker || d["worker"].is_null();
            tx.execute(
                "UPDATE waiting SET data = ?2, link = COALESCE(?3, link) WHERE seq = ?1",
                params![seq, d.to_string(), link.filter(|_| relink)],
            )?;
        }
        tx.execute("UPDATE outbox SET to_id = ?2 WHERE to_id = ?1", params![old, new])?;
        let waits: Vec<(i64, MergeWait)> = read(&tx, "SELECT rowid, data FROM merge_waits", [])?;
        for (row, mut w) in waits.into_iter().filter(|(_, w)| w.thread.as_deref() == Some(old)) {
            w.thread = Some(new.into());
            tx.execute("UPDATE merge_waits SET data = ?2 WHERE rowid = ?1", params![row, serde_json::to_string(&w)?])?;
        }
        let seen: Vec<(i64, PrSeen)> = read(&tx, "SELECT rowid, data FROM pr_seen", [])?;
        for (row, mut p) in seen.into_iter().filter(|(_, p)| p.tickets.iter().any(|(t, _)| t == old)) {
            for (t, _) in p.tickets.iter_mut().filter(|(t, _)| t == old) {
                *t = new.into();
            }
            tx.execute("UPDATE pr_seen SET data = ?2 WHERE rowid = ?1", params![row, serde_json::to_string(&p)?])?;
        }
        if let Some((card, resolution)) = m.card {
            let resolved = tx.execute(
                "UPDATE waiting SET resolved_at = ?3, note = ?4 WHERE owner = ?1 AND seq = ?2 AND resolved_at IS NULL",
                params![board, card, at, resolution],
            )?;
            if resolved == 0 {
                anyhow::bail!("its card was already answered");
            }
        }
        let handle = Self::msg_insert(&tx, board, board, "note", m.summary, at)?;
        let pick: Option<String> =
            tx.query_row("SELECT data FROM pool_picks WHERE seq = ?1", [m.pick], |r| r.get(0)).optional()?;
        if let Some(raw) = pick {
            let p = PickRecord {
                handle: Some(handle.clone()),
                thread: Some(new.into()),
                moved_from: Some(old.into()),
                ..serde_json::from_str(&raw)?
            };
            tx.execute(
                "UPDATE pool_picks SET handle = ?2, data = ?3 WHERE seq = ?1",
                params![m.pick, handle, serde_json::to_string(&p)?],
            )?;
        }
        self.commit(tx)?;
        Ok(rebound)
    }

    pub fn outbox_retarget(&self, from: &str, to: &str) -> Result<usize> {
        self.with(|c| c.execute("UPDATE outbox SET to_id = ?2 WHERE to_id = ?1", params![from, to]))
    }

    pub fn pending_retarget(&self, from: &str, to: &str) -> Result<Vec<String>> {
        let moved: Vec<Pending> = self.pending_all()?.into_iter().filter(|p| p.parent == from).collect();
        for p in &moved {
            self.pending_remove(&p.parent, &p.label)?;
            self.pending_put(&Pending { parent: to.to_string(), ..p.clone() })?;
        }
        Ok(moved.into_iter().map(|p| p.label).collect())
    }

    pub fn waiting_open(&self, owner: &str) -> Result<Vec<WaitingItem>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id, text, from_label, link, opened_at FROM waiting
                 WHERE owner = ?1 AND resolved_at IS NULL ORDER BY seq",
            )?;
            st.query_map([owner], |r| {
                Ok(WaitingItem { id: r.get(0)?, text: r.get(1)?, from: r.get(2)?, link: r.get(3)?, at: r.get(4)? })
            })?
            .collect()
        })
    }

    pub fn waiting_open_data(&self, owner: &str) -> Result<Vec<(WaitingItem, Option<Value>)>> {
        Ok(self.waiting_data(owner, false)?.into_iter().map(|(w, d, _, _)| (w, d)).collect())
    }

    pub fn waiting_data(&self, owner: &str, all: bool) -> Result<Vec<WaitingRow>> {
        type Row = (WaitingItem, Option<String>, Option<String>, Option<String>, i64);
        let rows: Vec<Row> = self.with(|c| {
            let mut st = c.prepare(
                "SELECT id, text, from_label, link, opened_at, data, resolved_at, note, seq FROM waiting
                 WHERE owner = ?1 AND (?2 OR resolved_at IS NULL) ORDER BY seq",
            )?;
            st.query_map(params![owner, all], |r| {
                Ok((
                    WaitingItem { id: r.get(0)?, text: r.get(1)?, from: r.get(2)?, link: r.get(3)?, at: r.get(4)? },
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })?
            .collect()
        })?;
        Ok(rows
            .into_iter()
            .map(|(w, d, at, note, seq)| (w, d.and_then(|s| serde_json::from_str(&s).ok()), at.map(|a| (a, note)), seq))
            .collect())
    }

    pub fn waiting_insert(&self, owner: &str, item: &WaitingItem, data: Option<&Value>) -> Result<()> {
        let data = data.map(Value::to_string);
        self.with(|c| {
            c.execute(
                "INSERT INTO waiting (owner, id, text, from_label, link, opened_at, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![owner, item.id, item.text, item.from, item.link, item.at, data],
            )
        })?;
        Ok(())
    }

    pub fn waiting_set_data(&self, owner: &str, seq: i64, data: &Value) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE waiting SET data = ?3 WHERE owner = ?1 AND seq = ?2",
                params![owner, seq, data.to_string()],
            )
        })?;
        Ok(())
    }

    pub fn waiting_retarget(
        &self,
        owner: &str,
        seq: i64,
        from_label: Option<&str>,
        link: Option<&str>,
        data: &Value,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE waiting SET from_label = ?3, link = ?4, data = ?5 WHERE owner = ?1 AND seq = ?2",
                params![owner, seq, from_label, link, data.to_string()],
            )
        })?;
        Ok(())
    }

    pub fn waiting_resolve_seq(&self, owner: &str, seq: i64, note: Option<&str>, at: &str) -> Result<bool> {
        self.with(|c| Self::resolve_seq(c, owner, seq, note, at)).map(|n| n > 0)
    }

    fn resolve_seq(c: &Connection, owner: &str, seq: i64, note: Option<&str>, at: &str) -> rusqlite::Result<usize> {
        c.execute(
            "UPDATE waiting SET resolved_at = ?3, note = ?4 WHERE owner = ?1 AND seq = ?2 AND resolved_at IS NULL",
            params![owner, seq, at, note],
        )
    }

    pub fn waiting_resolve_releasing(
        &self,
        owner: &str,
        seqs: &[i64],
        tickets: &[&str],
        note: &str,
        at: &str,
    ) -> Result<usize> {
        let release = "UPDATE links SET data = json_set(data, '$.gate', NULL, '$.gateSince', NULL, '$.gatePr', NULL) \
                       WHERE child = ?1 AND closed = 0 AND json_extract(data, '$.gate') = 'merge' \
                       AND NOT EXISTS (SELECT 1 FROM waiting w WHERE w.owner = ?2 AND w.resolved_at IS NULL \
                       AND json_extract(w.data, '$.kind') = 'gate' AND json_extract(w.data, '$.ticket') = ?1 \
                       AND json_extract(w.data, '$.worker') IS NULL)";
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut resolved = 0;
        for seq in seqs {
            resolved += Self::resolve_seq(&tx, owner, *seq, Some(note), at)?;
        }
        for ticket in tickets {
            tx.execute(release, params![ticket, owner])?;
        }
        self.commit(tx)?;
        Ok(resolved)
    }

    pub fn waiting_resolve(&self, owner: &str, id: &str, note: Option<&str>, at: &str) -> Result<Option<String>> {
        self.with(|c| {
            let text: Option<String> = c
                .query_row(
                    "SELECT text FROM waiting WHERE owner = ?1 AND id = ?2 AND resolved_at IS NULL",
                    params![owner, id],
                    |r| r.get(0),
                )
                .optional()?;
            if text.is_some() {
                c.execute(
                    "UPDATE waiting SET resolved_at = ?3, note = ?4 WHERE owner = ?1 AND id = ?2 AND resolved_at IS NULL",
                    params![owner, id, at, note],
                )?;
            }
            Ok(text)
        })
    }

    pub fn ask_insert(&self, thread: &str, data: &Value) -> Result<i64> {
        self.with(|c| {
            c.execute("INSERT INTO asks (thread, open, data) VALUES (?1, 1, ?2)", params![thread, data.to_string()])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn ask_update(&self, seq: i64, open: bool, data: &Value) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE asks SET open = ?2, data = ?3 WHERE seq = ?1",
                params![seq, open as i64, data.to_string()],
            )
        })?;
        Ok(())
    }

    pub fn ask_update_if_open_unsettled(&self, seq: i64, open: bool, data: &Value) -> Result<bool> {
        Ok(self.with(|c| {
            c.execute(
                &format!("UPDATE asks SET open = ?2, data = ?3 WHERE seq = ?1 AND open = 1 AND {UNSETTLED}"),
                params![seq, open as i64, data.to_string()],
            )
        })? > 0)
    }

    pub fn ask_is_open_unsettled(&self, seq: i64) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT EXISTS (SELECT 1 FROM asks WHERE seq = ?1 AND open = 1 AND {UNSETTLED})"),
                [seq],
                |r| r.get(0),
            )
        })
    }

    pub fn triage_thread(&self, thread: &str) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT EXISTS (SELECT 1 FROM asks WHERE thread = ?1 AND {TRIAGE_ASK})"),
                params![thread],
                |r| r.get(0),
            )
        })
    }

    pub fn triage_live(&self) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                &format!(
                    "SELECT EXISTS (SELECT 1 FROM asks WHERE {TRIAGE_ASK} AND \
                     (open = 1 OR COALESCE(json_extract(data, '$.threadStopped'), 0) = 0))"
                ),
                [],
                |r| r.get(0),
            )
        })
    }

    pub fn triage_unstopped_closed(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!(
            "WHERE open = 0 AND {TRIAGE_ASK} AND COALESCE(json_extract(data, '$.threadStopped'), 0) = 0 ORDER BY seq"
        ))
    }

    pub fn triage_mark_stopped(&self, seq: i64, archived: bool) -> Result<bool> {
        Ok(self.with(|c| {
            let sql = if archived {
                format!(
                    "UPDATE asks SET data = json_set(data, '$.threadStopped', json('true'), '$.archived', json('true')) \
                     WHERE seq = ?1 AND {TRIAGE_ASK} AND COALESCE(json_extract(data, '$.threadStopped'), 0) = 0"
                )
            } else {
                format!(
                    "UPDATE asks SET data = json_set(data, '$.threadStopped', json('true')) \
                     WHERE seq = ?1 AND {TRIAGE_ASK} AND COALESCE(json_extract(data, '$.threadStopped'), 0) = 0"
                )
            };
            c.execute(&sql, [seq])
        })? > 0)
    }

    fn asks_where(&self, sql: &str) -> Result<Vec<(i64, Value)>> {
        let rows: Vec<(i64, String)> = self.with(|c| {
            let mut st = c.prepare(&format!("SELECT seq, data FROM asks {sql}"))?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })?;
        rows.into_iter().map(|(seq, d)| Ok((seq, serde_json::from_str(&d)?))).collect()
    }

    pub fn ask_get(&self, seq: i64) -> Result<Option<(bool, Value)>> {
        let row: Option<(bool, String)> = self.with(|c| {
            c.query_row("SELECT open, data FROM asks WHERE seq = ?1", [seq], |r| Ok((r.get(0)?, r.get(1)?))).optional()
        })?;
        row.map(|(open, d)| Ok((open, serde_json::from_str(&d)?))).transpose()
    }

    pub fn ask_of_thread(&self, thread: &str) -> Result<Option<(i64, bool, Value)>> {
        let row: Option<(i64, bool, String)> = self.with(|c| {
            c.query_row(
                "SELECT seq, open, data FROM asks WHERE thread = ?1 ORDER BY seq DESC LIMIT 1",
                params![thread],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })?;
        row.map(|(seq, open, d)| Ok((seq, open, serde_json::from_str(&d)?))).transpose()
    }

    pub fn ask_thread(&self, thread: &str) -> Result<bool> {
        self.with(|c| {
            c.query_row("SELECT EXISTS (SELECT 1 FROM asks WHERE thread = ?1)", params![thread], |r| r.get(0))
        })
    }

    pub fn asks_open(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where("WHERE open = 1 ORDER BY seq")
    }

    pub fn asks_closed_unsettled(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!("WHERE open = 0 AND {UNSETTLED} AND {REGULAR_ASK} ORDER BY seq"))
    }

    pub fn asks_unsettled(&self, limit: usize) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!(
            "WHERE {UNSETTLED} AND {REGULAR_ASK} ORDER BY COALESCE(json_extract(data, '$.restoredAt'), json_extract(data, '$.at')) DESC, \
             seq DESC LIMIT {limit}"
        ))
    }

    pub fn asks_settled(&self, limit: usize) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!(
            "WHERE NOT {UNSETTLED} AND {REGULAR_ASK} ORDER BY json_extract(data, '$.settledAt') DESC, seq DESC LIMIT {limit}"
        ))
    }

    pub fn asks_settled_count(&self) -> Result<usize> {
        let n: i64 = self.with(|c| {
            c.query_row(&format!("SELECT COUNT(*) FROM asks WHERE NOT {UNSETTLED} AND {REGULAR_ASK}"), [], |r| r.get(0))
        })?;
        Ok(n as usize)
    }

    pub fn triage_open(&self) -> Result<Option<(i64, Value)>> {
        Ok(self.asks_where(&format!("WHERE open = 1 AND {TRIAGE_ASK} ORDER BY seq DESC LIMIT 1"))?.pop())
    }

    pub fn triage_latest_active(&self) -> Result<Option<(i64, bool, Value)>> {
        let row: Option<(i64, bool, String)> = self.with(|c| {
            c.query_row(
                &format!(
                    "SELECT seq, open, data FROM asks WHERE {UNSETTLED} AND {TRIAGE_ASK} ORDER BY seq DESC LIMIT 1"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })?;
        row.map(|(seq, open, d)| Ok((seq, open, serde_json::from_str(&d)?))).transpose()
    }

    pub fn triage_get(&self, seq: i64) -> Result<Option<(bool, Value)>> {
        let row: Option<(bool, String)> = self.with(|c| {
            c.query_row(&format!("SELECT open, data FROM asks WHERE seq = ?1 AND {TRIAGE_ASK}"), [seq], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })?;
        row.map(|(open, d)| Ok((open, serde_json::from_str(&d)?))).transpose()
    }

    pub fn triage_unsettled_closed(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!("WHERE open = 0 AND {UNSETTLED} AND {TRIAGE_ASK} ORDER BY seq"))
    }

    pub fn flow_dismissed_edge_add(&self, from_key: &str, kind: &str, to_key: &str, at: &str) -> Result<bool> {
        Ok(self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO flow_dismissed_edges (from_key, kind, to_key, at) VALUES (?1, ?2, ?3, ?4)",
                params![from_key, kind, to_key, at],
            )
        })? > 0)
    }

    pub fn flow_dismissed_edge_remove(&self, from_key: &str, kind: &str, to_key: &str) -> Result<bool> {
        Ok(self.with(|c| {
            c.execute(
                "DELETE FROM flow_dismissed_edges WHERE from_key = ?1 AND kind = ?2 AND to_key = ?3",
                params![from_key, kind, to_key],
            )
        })? > 0)
    }

    pub fn flow_dismissed_edges(&self) -> Result<Vec<DismissedEdgeRow>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT from_key, kind, to_key, at FROM flow_dismissed_edges ORDER BY from_key, kind, to_key",
            )?;
            st.query_map([], |r| {
                Ok(DismissedEdgeRow { from_key: r.get(0)?, kind: r.get(1)?, to_key: r.get(2)?, at: r.get(3)? })
            })?
            .collect()
        })
    }

    fn contexts_in(c: &Connection) -> rusqlite::Result<Vec<BoardContext>> {
        let mut st = c.prepare("SELECT data FROM contexts")?;
        let raws: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut all: Vec<BoardContext> = raws.iter().filter_map(|s| serde_json::from_str(s).ok()).collect();
        all.sort_by(|a, b| crate::contexts::name_order(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(all)
    }

    pub fn contexts_all(&self) -> Result<Vec<BoardContext>> {
        self.with(Self::contexts_in)
    }

    pub fn context_put(
        &self,
        id: &str,
        name: Option<&str>,
        spec: Option<&ContextSpec>,
        at: &str,
        max: usize,
    ) -> Result<ContextPut> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let all = Self::contexts_in(&tx)?;
        let old = all.iter().find(|c| c.id == id);
        let (name, spec) = match (old, name, spec) {
            (_, Some(n), Some(s)) => (n.to_string(), s.clone()),
            (Some(o), n, s) => (n.unwrap_or(&o.name).to_string(), s.unwrap_or(&o.spec).clone()),
            (None, _, _) => return Ok(ContextPut::Missing),
        };
        if let Some(other) = all.iter().find(|c| c.id != id && crate::contexts::same_name(&c.name, &name)) {
            return Ok(ContextPut::NameTaken(other.name.clone()));
        }
        if old.is_none() && all.len() >= max {
            return Ok(ContextPut::Full);
        }
        let saved = BoardContext {
            id: id.to_string(),
            name,
            spec,
            created_at: old.map_or(at, |o| &o.created_at).to_string(),
            updated_at: at.to_string(),
        };
        tx.execute(
            "INSERT INTO contexts (id, data) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            params![saved.id, serde_json::to_string(&saved)?],
        )?;
        self.commit(tx)?;
        Ok(if old.is_some() { ContextPut::Updated(saved) } else { ContextPut::Created(saved) })
    }

    pub fn context_delete(&self, id: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM contexts WHERE id = ?1", [id]))? > 0)
    }

    pub fn limit_set(&self, instance: &str, limit: &Limit) -> Result<()> {
        let s = serde_json::to_string(limit)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO instance_limits (instance, data) VALUES (?1, ?2)
                 ON CONFLICT (instance) DO UPDATE SET data = excluded.data",
                params![instance, s],
            )
        })?;
        Ok(())
    }

    pub fn limit_clear(&self, instance: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM instance_limits WHERE instance = ?1", [instance]))? > 0)
    }

    pub fn limits(&self) -> Result<BTreeMap<String, Limit>> {
        let rows: Vec<(String, String)> = self.with(|c| {
            let mut st = c.prepare("SELECT instance, data FROM instance_limits ORDER BY instance")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })?;
        rows.into_iter().map(|(k, v)| Ok((k, serde_json::from_str(&v)?))).collect()
    }

    pub fn limit_stop(&self, thread: &str, run: &str) -> Result<Option<LimitStop>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT data FROM limit_stops WHERE thread = ?1 AND run = ?2", [thread, run], |r| r.get(0))
                .optional()
        })?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn limit_stop_put(&self, thread: &str, run: &str, stop: &LimitStop) -> Result<()> {
        let s = serde_json::to_string(stop)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO limit_stops (thread, run, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread, run) DO UPDATE SET data = excluded.data",
                params![thread, run, s],
            )
        })?;
        Ok(())
    }

    pub fn run_tokens_seen(&self, thread: &str, run: &str) -> Result<bool> {
        let n: i64 = self.with(|c| {
            c.query_row("SELECT COUNT(*) FROM run_tokens WHERE thread = ?1 AND run = ?2", [thread, run], |r| r.get(0))
        })?;
        Ok(n > 0)
    }

    pub fn run_tokens_prune(&self, before: &str) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM run_tokens WHERE at < ?1", [before]))?;
        Ok(())
    }

    pub fn run_tokens_first_at(&self) -> Result<Option<String>> {
        self.with(|c| c.query_row("SELECT MIN(at) FROM run_tokens WHERE input IS NOT NULL", [], |r| r.get(0)))
    }

    pub fn run_tokens_put(&self, r: &RunTokenRecord) -> Result<()> {
        let (input, output) = r.tokens.map_or((None, None), |t| (Some(t.input as i64), Some(t.output as i64)));
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO run_tokens (thread, run, instance, driver, at, input, output)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![r.thread, r.run, r.instance, r.driver, r.at, input, output],
            )
        })?;
        Ok(())
    }

    pub fn run_tokens(&self, since: Option<&str>) -> Result<Vec<RunTokenRecord>> {
        type Row = (String, String, String, Option<String>, String, Option<i64>, Option<i64>);
        let rows: Vec<Row> = self.with(|c| {
            let mut st = c.prepare(
                "SELECT thread, run, instance, driver, at, input, output FROM run_tokens
                 WHERE ?1 IS NULL OR at >= ?1 ORDER BY at",
            )?;
            st.query_map([since], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })?
            .collect()
        })?;
        Ok(rows
            .into_iter()
            .map(|(thread, run, instance, driver, at, input, output)| RunTokenRecord {
                thread,
                run,
                instance,
                driver,
                at,
                tokens: input.zip(output).map(|(i, o)| RunTokens { input: i.max(0) as u64, output: o.max(0) as u64 }),
            })
            .collect())
    }

    pub fn pool_credits(&self, pool: &str) -> Result<BTreeMap<String, i64>> {
        let rows: Vec<(String, i64)> = self.with(|c| {
            let mut st = c.prepare("SELECT instance, credit FROM pool_members WHERE pool = ?1")?;
            st.query_map([pool], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })?;
        Ok(rows.into_iter().collect())
    }

    pub fn pool_credits_put(&self, pool: &str, credits: &BTreeMap<String, i64>) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for (instance, credit) in credits {
            tx.execute(
                "INSERT INTO pool_members (pool, instance, credit) VALUES (?1, ?2, ?3)
                 ON CONFLICT (pool, instance) DO UPDATE SET credit = excluded.credit",
                params![pool, instance, credit],
            )?;
        }
        self.commit(tx)?;
        Ok(())
    }

    pub fn pick_add(&self, p: &PickRecord) -> Result<i64> {
        let s = serde_json::to_string(p)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO pool_picks (pool, instance, at, handle, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.pool, p.instance_id, p.at, p.handle, s],
            )?;
            let seq = c.last_insert_rowid();
            if seq % 100 == 0 {
                c.execute("DELETE FROM pool_picks WHERE seq <= ?1", [seq - KEEP_PICKS])?;
            }
            Ok(seq)
        })
    }

    pub fn pick_launched(&self, seq: i64, handle: Option<&str>, thread: &str) -> Result<()> {
        self.pick_bind(seq, handle, thread, None)
    }

    pub fn pick_bind(&self, seq: i64, handle: Option<&str>, thread: &str, moved_from: Option<&str>) -> Result<()> {
        let Some(mut p) = self.pick_get(seq)? else { return Ok(()) };
        p.handle = handle.map(String::from);
        p.thread = Some(thread.to_string());
        p.moved_from = moved_from.map(String::from);
        let s = serde_json::to_string(&p)?;
        self.with(|c| {
            c.execute("UPDATE pool_picks SET handle = ?2, data = ?3 WHERE seq = ?1", params![seq, handle, s])
        })?;
        Ok(())
    }

    pub fn pick_get(&self, seq: i64) -> Result<Option<PickRecord>> {
        let raw: Option<String> =
            self.with(|c| c.query_row("SELECT data FROM pool_picks WHERE seq = ?1", [seq], |r| r.get(0)).optional())?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn pick_by_thread(&self, thread: &str) -> Result<Option<PickRecord>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row(
                "SELECT data FROM pool_picks WHERE json_extract(data, '$.thread') = ?1 ORDER BY seq DESC LIMIT 1",
                [thread],
                |r| r.get(0),
            )
            .optional()
        })?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn picks(&self, pool: &str, limit: usize) -> Result<Vec<PickRecord>> {
        let raws: Vec<String> = self.with(|c| {
            let mut st = c.prepare("SELECT data FROM pool_picks WHERE pool = ?1 ORDER BY seq DESC LIMIT ?2")?;
            st.query_map(params![pool, limit as i64], |r| r.get(0))?.collect()
        })?;
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }

    pub fn pick_by_handle(&self, handle: &str) -> Result<Option<PickRecord>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT data FROM pool_picks WHERE handle = ?1", [handle], |r| r.get(0)).optional()
        })?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn pick_stats(&self, pool: &str) -> Result<BTreeMap<String, (u64, Option<String>)>> {
        let rows: Vec<(String, i64, Option<String>)> = self.with(|c| {
            let mut st =
                c.prepare("SELECT instance, COUNT(*), MAX(at) FROM pool_picks WHERE pool = ?1 GROUP BY instance")?;
            st.query_map([pool], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
        })?;
        Ok(rows.into_iter().map(|(i, n, at)| (i, (n.max(0) as u64, at))).collect())
    }

    pub fn new_issue_add(&self, key: &str, issue: &NewIssue) -> Result<Option<i64>> {
        let s = serde_json::to_string(issue)?;
        self.with(|c| {
            let n = c.execute("INSERT OR IGNORE INTO new_issues (key, data) VALUES (?1, ?2)", params![key, s])?;
            Ok((n > 0).then(|| c.last_insert_rowid()))
        })
    }

    fn new_issue_where<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<(i64, NewIssue)>> {
        let rows: Vec<(i64, String)> = self.with(|c| {
            let mut st = c.prepare(&format!("SELECT seq, data FROM new_issues {sql}"))?;
            st.query_map(p, |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })?;
        rows.into_iter().map(|(seq, d)| Ok((seq, serde_json::from_str(&d)?))).collect()
    }

    pub fn new_issue_by_key(&self, key: &str) -> Result<Option<(i64, NewIssue)>> {
        Ok(self.new_issue_where("WHERE key = ?1", [key])?.into_iter().next())
    }

    pub fn new_issue_get(&self, seq: i64) -> Result<Option<NewIssue>> {
        Ok(self.new_issue_where("WHERE seq = ?1", [seq])?.into_iter().next().map(|(_, i)| i))
    }

    pub fn new_issues(&self) -> Result<Vec<(i64, NewIssue)>> {
        self.new_issue_where("ORDER BY seq", [])
    }

    pub fn new_issue_update(&self, seq: i64, issue: &NewIssue) -> Result<()> {
        let s = serde_json::to_string(issue)?;
        self.with(|c| c.execute("UPDATE new_issues SET data = ?2 WHERE seq = ?1", params![seq, s]))?;
        Ok(())
    }

    pub fn new_issue_remove(&self, key: &str) -> Result<bool> {
        Ok(self.with(|c| c.execute("DELETE FROM new_issues WHERE key = ?1", [key]))? > 0)
    }

    pub fn pairing_token_insert(
        &self,
        token_hash: &str,
        label: Option<&str>,
        created_at: &str,
        expires_at: &str,
        prune_before: &str,
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM pairing_tokens WHERE expires_at <= ?1", [prune_before])?;
        tx.execute(
            "INSERT INTO pairing_tokens (token_hash, label, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, label, created_at, expires_at],
        )?;
        self.commit(tx)?;
        Ok(())
    }

    pub fn pairing_token_redeem(
        &self,
        token_hash: &str,
        session_hash: &str,
        fallback_label: Option<&str>,
        now_iso: &str,
        session_expires_at: &str,
    ) -> Result<RedeemResult> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(Option<String>, String, Option<String>)> = tx
            .query_row(
                "SELECT label, expires_at, used_at FROM pairing_tokens WHERE token_hash = ?1",
                [token_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((token_label, expires_at, used_at)) = row else {
            return Ok(RedeemResult::NotFound);
        };
        if used_at.is_some() {
            return Ok(RedeemResult::AlreadyUsed);
        }
        if !is_unexpired(now_iso, &expires_at) {
            return Ok(RedeemResult::Expired);
        }
        let updated = tx.execute(
            "UPDATE pairing_tokens SET used_at = ?2 WHERE token_hash = ?1 AND used_at IS NULL",
            params![token_hash, now_iso],
        )?;
        if updated != 1 {
            return Ok(RedeemResult::AlreadyUsed);
        }
        let label = token_label.or_else(|| fallback_label.map(str::to_string));
        tx.execute(
            "INSERT INTO board_sessions (token_hash, label, created_at, last_seen_at, expires_at)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            params![session_hash, label, now_iso, session_expires_at],
        )?;
        let seq = tx.last_insert_rowid();
        self.commit(tx)?;
        Ok(RedeemResult::Redeemed(BoardSession {
            seq,
            id: crate::pairing::format_session_id(seq),
            label,
            created_at: now_iso.to_string(),
            last_seen_at: now_iso.to_string(),
            expires_at: session_expires_at.to_string(),
        }))
    }

    pub fn board_session_verify(&self, token_hash: &str, now_iso: &str) -> Result<Option<BoardSession>> {
        self.with(|c| {
            let row: Option<BoardSession> = c
                .query_row(
                    "SELECT seq, label, created_at, last_seen_at, expires_at FROM board_sessions WHERE token_hash = ?1",
                    [token_hash],
                    |r| {
                        let seq: i64 = r.get(0)?;
                        Ok(BoardSession {
                            seq,
                            id: crate::pairing::format_session_id(seq),
                            label: r.get(1)?,
                            created_at: r.get(2)?,
                            last_seen_at: r.get(3)?,
                            expires_at: r.get(4)?,
                        })
                    },
                )
                .optional()?;
            let Some(mut session) = row else {
                return Ok(None);
            };
            if !is_unexpired(now_iso, &session.expires_at) {
                return Ok(None);
            }
            if session.last_seen_at != now_iso {
                c.execute("UPDATE board_sessions SET last_seen_at = ?2 WHERE seq = ?1", params![session.seq, now_iso])?;
                session.last_seen_at = now_iso.to_string();
            }
            Ok(Some(session))
        })
    }

    pub fn board_session_is_active(&self, seq: i64, now_iso: &str) -> Result<bool> {
        let exp: Option<String> = self.with(|c| {
            c.query_row("SELECT expires_at FROM board_sessions WHERE seq = ?1", [seq], |r| r.get(0)).optional()
        })?;
        Ok(exp.is_some_and(|e| is_unexpired(now_iso, &e)))
    }

    pub fn board_sessions_active(&self, now_iso: &str) -> Result<Vec<BoardSession>> {
        let rows: Vec<BoardSession> = self.with(|c| {
            let mut st =
                c.prepare("SELECT seq, label, created_at, last_seen_at, expires_at FROM board_sessions ORDER BY seq")?;
            st.query_map([], |r| {
                let seq: i64 = r.get(0)?;
                Ok(BoardSession {
                    seq,
                    id: crate::pairing::format_session_id(seq),
                    label: r.get(1)?,
                    created_at: r.get(2)?,
                    last_seen_at: r.get(3)?,
                    expires_at: r.get(4)?,
                })
            })?
            .collect()
        })?;
        Ok(rows.into_iter().filter(|s| is_unexpired(now_iso, &s.expires_at)).collect())
    }

    pub fn board_session_revoke(&self, seq: i64, now_iso: &str) -> Result<bool> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exp: Option<String> =
            tx.query_row("SELECT expires_at FROM board_sessions WHERE seq = ?1", [seq], |r| r.get(0)).optional()?;
        tx.execute("DELETE FROM board_sessions WHERE seq = ?1", [seq])?;
        self.commit(tx)?;
        Ok(exp.is_some_and(|e| is_unexpired(now_iso, &e)))
    }

    pub fn board_session_revoke_all(&self, now_iso: &str) -> Result<usize> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exps: Vec<String> = {
            let mut st = tx.prepare("SELECT expires_at FROM board_sessions")?;
            st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        let active = exps.iter().filter(|e| is_unexpired(now_iso, e)).count();
        tx.execute("DELETE FROM board_sessions", [])?;
        self.commit(tx)?;
        Ok(active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_objects(store: &Store) -> Vec<(String, String, String, String)> {
        store
            .with(|c| {
                let mut st = c.prepare(
                    "SELECT type, name, tbl_name, sql FROM sqlite_master \
                     WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
                )?;
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect()
            })
            .unwrap()
    }

    fn table_columns(store: &Store, table: &str) -> Vec<String> {
        store
            .with(|c| {
                let mut st = c.prepare(&format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid"))?;
                st.query_map([], |r| r.get(0))?.collect()
            })
            .unwrap()
    }

    #[test]
    fn open_memory_matches_open_schema_and_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let wal = Store::open(dir.path()).unwrap();
        let mem = Store::open_memory().unwrap();

        assert_eq!(mem.path(), Path::new(":memory:"));
        assert_eq!(schema_objects(&mem), schema_objects(&wal));
        assert_eq!(table_columns(&mem, "waiting"), table_columns(&wal, "waiting"));
        assert_eq!(table_columns(&mem, "outbox"), table_columns(&wal, "outbox"));
        assert!(table_columns(&mem, "waiting").iter().any(|c| c == "data"));
        assert!(table_columns(&mem, "outbox").iter().any(|c| c == "allow_closed"));
    }

    #[test]
    fn open_memory_is_isolated_and_supports_core_and_transactional_ops() {
        let a = Store::open_memory().unwrap();
        let b = Store::open_memory().unwrap();

        a.meta_set("k", "v").unwrap();
        assert_eq!(a.meta_get("k").unwrap().as_deref(), Some("v"));
        assert_eq!(b.meta_get("k").unwrap(), None);

        let link = Link {
            child: "mcp:w1".into(),
            parent: "mcp:t1".into(),
            label: "#41 PR 1/1".into(),
            role: "worker".into(),
            parent_role: Some("ticket".into()),
            params: LinkParams { down: 45, miss: 2, stall: 240 },
            link: None,
            opened: "2026-11-17T12:00:00Z".into(),
            last_up: "2026-11-17T12:00:00Z".into(),
            last_seen: None,
            last_nudge: None,
            nudges: 0,
            escalated: false,
            reported: Vec::new(),
            gate: None,
            gate_since: None,
            gate_pr: None,
            closed: false,
            closed_at: None,
            closed_how: None,
            unread: Vec::new(),
            bad_run: None,
            settled_at: None,
            restored_at: None,
            base: Default::default(),
            project: None,
            paused_at: None,
            identity: Identity::default(),
            brief: None,
            pr: None,
        };
        assert!(matches!(a.link_bind("mcp:w1", false, |_| Bind::Saved(link.clone())).unwrap(), Bind::Saved(_)));
        assert_eq!(a.links_all().unwrap().len(), 1);
        assert!(b.links_all().unwrap().is_empty());

        let wait = MergeWait {
            repo: "acme/widgets".into(),
            pr: 7,
            head: "1111111".into(),
            pull_request_id: "PR_acme/widgets_7".into(),
            method: None,
            thread: Some("mcp:t1".into()),
            label: Some("widgets#7".into()),
            merge_state: "CLEAN".into(),
            at: "2026-11-17T12:00:00Z".into(),
            status: WaitStatus::Waiting,
            approval: "appr-1".into(),
            failures: 0,
            last_error: None,
            advancing: None,
        };
        a.merge_wait_put(&wait).unwrap();
        assert_eq!(a.merge_wait_rows().unwrap(), vec![("acme/widgets".into(), 7, wait)]);

        let wake = Wake {
            repo: "acme/widgets",
            pr: 7,
            event: Some("merged"),
            ticket: "mcp:t1",
            from: "github",
            kind: "event",
            body: "acme/widgets#7 merged",
            at: "2026-11-17T12:01:00Z",
        };
        let handle = a.pr_event_wake(&wake, |h| format!("[t3cm event from=github msg={h}]\nmerged")).unwrap().unwrap();
        assert_eq!(a.msg_get(&handle).unwrap().unwrap().body, "acme/widgets#7 merged");
        assert_eq!(a.pr_events_all().unwrap(), vec![("acme/widgets".into(), 7, "merged".into(), "mcp:t1".into())]);
        assert!(b.pr_events_all().unwrap().is_empty());
    }

    #[cfg(feature = "test-faults")]
    fn crash_of(f: impl FnOnce()) -> Option<StoreCrash> {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).err()?;
        payload.downcast_ref::<StoreCrash>().copied()
    }

    #[cfg(feature = "test-faults")]
    #[test]
    fn set_fault_counts_only_commits_and_crashes_at_the_nth_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let base = store.writes();
        store.kv_set("ns", "a", &serde_json::json!(1), "t").unwrap();
        assert_eq!(store.kv_get("ns", "a").unwrap(), Some(serde_json::json!(1)));
        assert_eq!(store.writes(), base + 1, "a read is not a write");
        let wake = Wake {
            repo: "acme/widgets",
            pr: 7,
            event: Some("merged"),
            ticket: "mcp:t1",
            from: "github",
            kind: "event",
            body: "merged",
            at: "t",
        };
        store.pr_event_wake(&wake, |h| h.to_string()).unwrap();
        assert_eq!(store.writes(), base + 2, "a transaction is one write");

        store.set_fault(Some(base + 4));
        store.kv_set("ns", "b", &serde_json::json!(2), "t").unwrap();
        assert_eq!(
            crash_of(|| drop(store.kv_set("ns", "c", &serde_json::json!(3), "t"))),
            Some(StoreCrash { write: base + 4 })
        );
        assert_eq!(crash_of(|| drop(store.kv_get("ns", "a"))), Some(StoreCrash { write: base + 4 }));
        drop(store);

        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.kv_list("ns", "").unwrap(), ["a", "b"], "the crashing write rolled back");
        assert_eq!(reopened.pr_events_all().unwrap().len(), 1);
        assert_eq!(reopened.writes(), 0);
    }

    #[cfg(feature = "test-faults")]
    #[test]
    fn a_crash_inside_a_transaction_rolls_back_every_statement() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.set_fault(Some(store.writes() + 1));
        let wake = Wake {
            repo: "acme/widgets",
            pr: 7,
            event: Some("merged"),
            ticket: "mcp:t1",
            from: "github",
            kind: "event",
            body: "merged",
            at: "t",
        };
        assert!(crash_of(|| drop(store.pr_event_wake(&wake, |h| h.to_string()))).is_some());
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert!(reopened.pr_events_all().unwrap().is_empty());
        assert!(reopened.outbox_all().unwrap().is_empty());
    }

    #[test]
    fn a_pr_seen_row_tracked_again_never_repeats_a_revision() {
        let store = Store::open_memory().unwrap();
        let seen = |state: &str| PrSeen {
            repo: "acme/widgets".into(),
            pr: 7,
            state: state.into(),
            head: "a".repeat(40),
            ..PrSeen::default()
        };
        let rev = |store: &Store| store.pr_seen_get("acme/widgets", 7).unwrap().unwrap().rev;
        store.pr_seen_put(&seen("OPEN")).unwrap();
        store.pr_seen_put(&seen("MERGED")).unwrap();
        let merged = rev(&store);
        store.pr_seen_remove("acme/widgets", 7).unwrap();
        store.pr_seen_put(&seen("OPEN")).unwrap();
        store.pr_seen_put(&seen("MERGED")).unwrap();
        assert!(rev(&store) > merged, "a MERGED row tracked again reused revision {merged}");
    }

    #[test]
    fn pr_seen_revisions_continue_past_rows_saved_before_the_sequence() {
        let store = Store::open_memory().unwrap();
        let old = serde_json::json!({"repo": "acme/widgets", "pr": 7, "state": "OPEN", "head": "a", "queued": false,
                                     "since": "t", "tickets": [], "at": "t", "rev": 41});
        store
            .with(|c| {
                c.execute("INSERT INTO pr_seen (repo, pr, data) VALUES ('acme/widgets', 7, ?1)", [old.to_string()])
            })
            .unwrap();
        let mut next = store.pr_seen_get("acme/widgets", 8).unwrap().unwrap_or_default();
        next.repo = "acme/widgets".into();
        next.pr = 8;
        store.pr_seen_put(&next).unwrap();
        assert_eq!(store.pr_seen_get("acme/widgets", 8).unwrap().unwrap().rev, 42);
    }

    #[test]
    fn merge_fields_keeps_both_sides_changes_and_merges_set_fields() {
        let base = serde_json::json!({"gate": null, "nudges": 0, "unread": ["m1", "m2"], "reported": []});
        let ours = serde_json::json!({"gate": null, "nudges": 1, "unread": ["m2"], "reported": ["stall"]});
        let theirs = serde_json::json!({"gate": "review", "nudges": 0, "unread": ["m1", "m2", "m3"], "reported": []});
        let merged: Value =
            serde_json::from_str(&merge_fields(&base.to_string(), &ours.to_string(), &theirs.to_string()).unwrap())
                .unwrap();
        assert_eq!(
            merged,
            serde_json::json!({"gate": "review", "nudges": 1, "unread": ["m2", "m3"], "reported": ["stall"]})
        );
    }

    #[test]
    fn a_null_field_left_out_on_save_keeps_a_peers_value() {
        let base = serde_json::json!({"closed": false, "gatePr": null, "nudges": 0});
        let ours = serde_json::json!({"closed": false, "nudges": 1});
        let theirs = serde_json::json!({"closed": false, "gatePr": {"repo": "acme/widgets", "number": 7}, "nudges": 0});
        let merged: Value =
            serde_json::from_str(&merge_fields(&base.to_string(), &ours.to_string(), &theirs.to_string()).unwrap())
                .unwrap();
        assert_eq!(
            merged,
            serde_json::json!({"closed": false, "gatePr": {"repo": "acme/widgets", "number": 7}, "nudges": 1})
        );
    }

    #[test]
    fn a_close_merged_with_a_peers_gate_clears_the_gate() {
        let base = serde_json::json!({"closed": false, "gate": null});
        let ours = serde_json::json!({"closed": true, "gate": null, "closedHow": "handoff"});
        let theirs = serde_json::json!({"closed": false, "gate": "review", "gateSince": "t"});
        let merged: Value =
            serde_json::from_str(&merge_fields(&base.to_string(), &ours.to_string(), &theirs.to_string()).unwrap())
                .unwrap();
        assert_eq!(merged, serde_json::json!({"closed": true, "gate": null, "closedHow": "handoff"}));
    }

    #[test]
    fn a_stale_gate_save_after_a_peer_closed_keeps_the_close_clean() {
        let base = serde_json::json!({"closed": false, "gate": null});
        let ours = serde_json::json!({"closed": false, "gate": "review", "gateSince": "t"});
        let theirs = serde_json::json!({"closed": true, "gate": null, "closedHow": "handoff"});
        let merged: Value =
            serde_json::from_str(&merge_fields(&base.to_string(), &ours.to_string(), &theirs.to_string()).unwrap())
                .unwrap();
        assert_eq!(merged, serde_json::json!({"closed": true, "gate": null, "closedHow": "handoff"}));
    }

    #[test]
    fn a_save_after_the_row_moved_away_does_not_bring_it_back() {
        let store = Store::open_memory().unwrap();
        let raw = serde_json::json!({"child": "mcp:old", "parent": "mcp:t", "label": "w", "role": "worker",
            "parentRole": null, "params": {"down": 45, "miss": 2, "stall": 240}, "link": null, "opened": "t",
            "lastUp": "t", "lastSeen": null, "lastNudge": null, "nudges": 0, "escalated": false, "reported": [],
            "gate": null, "gateSince": null, "closed": false, "closedAt": null, "closedHow": null, "unread": [],
            "badRun": null});
        store
            .with(|c| {
                c.execute(
                    "INSERT INTO links (child, parent, closed, data) VALUES ('mcp:old', 'mcp:t', 0, ?1)",
                    [raw.to_string()],
                )
            })
            .unwrap();
        let mut ln = store.link_get("mcp:old").unwrap().unwrap();
        store.with(|c| c.execute("DELETE FROM links WHERE child = 'mcp:old'", [])).unwrap();
        ln.nudges = 1;
        store.link_save(&ln).unwrap();
        assert!(store.link_get("mcp:old").unwrap().is_none());
        ln.closed = true;
        assert_eq!(store.link_close(&ln, true, Some(("mcp:t", "bye", "m1")), "t").unwrap(), None);
        assert!(store.outbox_all().unwrap().is_empty(), "a close of a moved link queued its wake");
    }
}
