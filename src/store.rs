use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::LinkParams;

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
  handle TEXT NOT NULL, queued TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS waiting (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, owner TEXT NOT NULL, id TEXT NOT NULL, text TEXT NOT NULL,
  from_label TEXT, link TEXT, opened_at TEXT NOT NULL, resolved_at TEXT, note TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS waiting_open ON waiting (owner, id) WHERE resolved_at IS NULL;
CREATE TABLE IF NOT EXISTS asks (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, thread TEXT NOT NULL, open INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS merge_waits (
  repo TEXT NOT NULL, pr INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY (repo, pr));
"#;

const KEEP_MESSAGES: i64 = 5000;
const UNSETTLED: &str = "json_extract(data, '$.settledAt') IS NULL";

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
}

impl Link {
    pub fn closed_since(&self) -> Option<&str> {
        match (self.closed_at.as_deref(), self.restored_at.as_deref()) {
            (Some(c), Some(r)) if r > c => Some(r),
            (c, _) => c,
        }
    }
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboxItem {
    pub id: i64,
    pub to: String,
    pub text: String,
    pub handle: String,
    pub queued: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct WaitingItem {
    pub id: String,
    pub text: String,
    pub from: Option<String>,
    pub link: Option<String>,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredToken {
    pub access_token: String,
    pub client_id: String,
    pub expires_at: String,
    pub obtained_at: String,
}

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

fn handle_seq(handle: &str) -> Option<i64> {
    handle.strip_prefix('m')?.parse().ok()
}

impl Store {
    pub fn open(dir: &Path) -> Result<Store> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        crate::platform::private_dir(dir)?;
        let path = dir.join("store.db");
        let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        crate::platform::private_file(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let has_data: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('waiting') WHERE name = 'data'", [], |r| r.get(0))?;
        if has_data == 0 {
            conn.execute_batch("ALTER TABLE waiting ADD COLUMN data TEXT")?;
        }
        Ok(Store { conn: Mutex::new(conn), path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(f(&conn)?)
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

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional())
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
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }

    pub fn link_get(&self, child: &str) -> Result<Option<Link>> {
        Ok(self.links_where("WHERE child = ?1", [child])?.pop())
    }

    pub fn links_open(&self) -> Result<Vec<Link>> {
        self.links_where("WHERE closed = 0", [])
    }

    pub fn children(&self, parent: &str, open_only: bool) -> Result<Vec<Link>> {
        if open_only {
            self.links_where("WHERE parent = ?1 AND closed = 0", [parent])
        } else {
            self.links_where("WHERE parent = ?1", [parent])
        }
    }

    pub fn link_save(&self, ln: &Link) -> Result<()> {
        let data = serde_json::to_string(ln)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO links (child, parent, closed, data) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (child) DO UPDATE SET parent = excluded.parent, closed = excluded.closed, data = excluded.data",
                params![ln.child, ln.parent, ln.closed as i64, data],
            )
        })?;
        Ok(())
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

    pub fn msg_new(&self, from: &str, to: &str, kind: &str, body: &str, at: &str) -> Result<String> {
        let seq = self.with(|c| {
            c.execute(
                "INSERT INTO messages (from_id, to_id, kind, body, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![from, to, kind, body, at],
            )?;
            let seq = c.last_insert_rowid();
            if seq % 100 == 0 {
                c.execute("DELETE FROM messages WHERE seq <= ?1", [seq - KEEP_MESSAGES])?;
            }
            Ok(seq)
        })?;
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
        self.with(|c| {
            c.execute(
                "INSERT INTO outbox (to_id, text, handle, queued) VALUES (?1, ?2, ?3, ?4)",
                params![to, text, handle, queued],
            )
        })?;
        Ok(())
    }

    pub fn outbox_all(&self) -> Result<Vec<OutboxItem>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, to_id, text, handle, queued FROM outbox ORDER BY id")?;
            st.query_map([], |r| {
                Ok(OutboxItem { id: r.get(0)?, to: r.get(1)?, text: r.get(2)?, handle: r.get(3)?, queued: r.get(4)? })
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

    pub fn waiting_resolve_seq(&self, owner: &str, seq: i64, note: Option<&str>, at: &str) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE waiting SET resolved_at = ?3, note = ?4 WHERE owner = ?1 AND seq = ?2 AND resolved_at IS NULL",
                params![owner, seq, at, note],
            )
        })
        .map(|n| n > 0)
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

    pub fn asks_open(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where("WHERE open = 1 ORDER BY seq")
    }

    pub fn asks_closed_unsettled(&self) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!("WHERE open = 0 AND {UNSETTLED} ORDER BY seq"))
    }

    pub fn asks_unsettled(&self, limit: usize) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!(
            "WHERE {UNSETTLED} ORDER BY COALESCE(json_extract(data, '$.restoredAt'), json_extract(data, '$.at')) DESC, \
             seq DESC LIMIT {limit}"
        ))
    }

    pub fn asks_settled(&self, limit: usize) -> Result<Vec<(i64, Value)>> {
        self.asks_where(&format!(
            "WHERE NOT {UNSETTLED} ORDER BY json_extract(data, '$.settledAt') DESC, seq DESC LIMIT {limit}"
        ))
    }

    pub fn asks_settled_count(&self) -> Result<usize> {
        let n: i64 =
            self.with(|c| c.query_row(&format!("SELECT COUNT(*) FROM asks WHERE NOT {UNSETTLED}"), [], |r| r.get(0)))?;
        Ok(n as usize)
    }
}
