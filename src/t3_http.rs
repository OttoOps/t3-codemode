use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::clock::{Clock, iso, minutes_since, parse_ts};
use crate::config::home;
use crate::store::{Store, StoredToken};
use crate::t3::{BoxFut, T3Client, T3Error, T3Status, truncate_chars, unwrap_tool_result};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const LABEL: &str = "t3-codemode";
const REDIRECT: &str = "http://127.0.0.1/callback";
const SCOPES: &[&str] = &["orchestration:read", "orchestration:operate", "settings:write", "source-control:write"];
const REFRESH_MINUTES: f64 = 3.0 * 24.0 * 60.0;

pub fn t3_home() -> PathBuf {
    std::env::var("T3CODE_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".t3"))
}

struct Runtime {
    origin: String,
    pid: Option<u32>,
}

fn runtime() -> Result<Runtime, T3Error> {
    let path = t3_home().join("userdata").join("server-runtime.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| T3Error::Unavailable(format!("T3 Code is not running here: no readable {}", path.display())))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| T3Error::Unavailable(format!("{}: {e}", path.display())))?;
    let origin = std::env::var("T3CM_T3_ORIGIN")
        .ok()
        .or_else(|| v["origin"].as_str().map(String::from))
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", v["port"].as_u64().unwrap_or(3773)));
    Ok(Runtime { origin: origin.trim_end_matches('/').to_string(), pid: v["pid"].as_u64().map(|p| p as u32) })
}

pub fn find_t3_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("T3CM_T3_BIN") {
        return Some(PathBuf::from(p));
    }
    if let Some(pid) = runtime().ok().and_then(|r| r.pid)
        && let Ok(real) = std::fs::canonicalize(format!("/proc/{pid}/exe"))
        && real.file_name().is_some_and(|n| n == "t3")
    {
        return Some(real);
    }
    let dir = t3_home().join("wsl-runtime");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("sha256-"))
        .map(|e| e.path().join("t3"))
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("system randomness");
    URL_SAFE_NO_PAD.encode(b)
}

fn json_from_cli(out: &str) -> Option<Value> {
    let start = out.find(['{', '['])?;
    serde_json::from_str(&out[start..]).ok()
}

async fn run_t3(bin: &PathBuf, args: &[&str]) -> Result<String, T3Error> {
    let out = tokio::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| T3Error::Unavailable(format!("could not run {}: {e}", bin.display())))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(T3Error::Unavailable(format!(
            "`t3 {}` failed: {}",
            args[..2].join(" "),
            truncate_chars(err.trim(), 300)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

enum PostError {
    Status(u16, String),
    Transport(String),
}

struct Session {
    origin: String,
    token: String,
    id: Option<String>,
}

pub struct HttpT3 {
    http: reqwest::Client,
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    acquiring: tokio::sync::Mutex<()>,
    token: RwLock<Option<StoredToken>>,
    session: tokio::sync::Mutex<Option<Session>>,
    next_id: AtomicU64,
    status: Mutex<T3Status>,
    label: String,
}

fn parse_body(content_type: &str, raw: &str) -> Result<Value, String> {
    if content_type.contains("text/event-stream") {
        let last = raw.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).rfind(|l| !l.is_empty());
        return match last {
            Some(d) => serde_json::from_str(d).map_err(|e| e.to_string()),
            None => Err("empty event stream from /mcp".into()),
        };
    }
    if raw.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(raw).map_err(|e| e.to_string())
}

impl HttpT3 {
    pub fn new(store: Arc<Store>, clock: Arc<dyn Clock>) -> HttpT3 {
        let http =
            reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).no_proxy().build().expect("http client");
        let token = store.token_get().ok().flatten();
        let label = instance_label(store.path());
        HttpT3 {
            http,
            store,
            clock,
            acquiring: tokio::sync::Mutex::new(()),
            token: RwLock::new(token),
            session: tokio::sync::Mutex::new(None),
            next_id: AtomicU64::new(1),
            status: Mutex::new(T3Status::default()),
            label,
        }
    }

    fn token_fresh(&self, t: &StoredToken) -> bool {
        minutes_since(&*self.clock, Some(&t.expires_at)).is_some_and(|m| -m > REFRESH_MINUTES)
    }

    pub async fn ensure_token(&self, force: bool) -> Result<StoredToken, T3Error> {
        let seen = self.token.read().unwrap().clone();
        if let Some(t) = &seen
            && !force
            && self.token_fresh(t)
        {
            return Ok(t.clone());
        }
        let _g = self.acquiring.lock().await;
        let now = self.token.read().unwrap().clone();
        if let Some(t) = &now
            && (now != seen || (!force && self.token_fresh(t)))
        {
            return Ok(t.clone());
        }
        self.acquire().await
    }

    async fn acquire(&self) -> Result<StoredToken, T3Error> {
        let rt = runtime()?;
        let bin = find_t3_binary().ok_or_else(|| T3Error::Unavailable("no t3 binary found; set T3CM_T3_BIN".into()))?;
        let origin = rt.origin;
        let reg = self
            .post_json(
                &format!("{origin}/oauth/mcp/register"),
                &json!({"client_name": self.label, "redirect_uris": [REDIRECT]}),
            )
            .await?;
        let client_id =
            reg["client_id"].as_str().ok_or_else(|| T3Error::Unavailable("register: no client_id".into()))?.to_string();
        let mut args = vec!["auth", "pairing", "create", "--ttl", "5m", "--label", self.label.as_str(), "--json"];
        for s in SCOPES {
            args.extend(["--scope", s]);
        }
        let out = run_t3(&bin, &args).await?;
        let pairing =
            json_from_cli(&out).ok_or_else(|| T3Error::Unavailable("pairing create: no JSON output".into()))?;
        let code = ["credential", "token", "code"]
            .iter()
            .find_map(|k| pairing[*k].as_str())
            .ok_or_else(|| T3Error::Unavailable("pairing create: no pairing code in its output".into()))?
            .to_string();
        let verifier = random_b64(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_b64(12);
        let resource = format!("{origin}/mcp");
        let decision = self
            .post_json(
                &format!("{origin}/oauth/mcp/decision"),
                &json!({
                    "authorization": {"response_type": "code", "client_id": client_id, "redirect_uri": REDIRECT,
                        "code_challenge": challenge, "code_challenge_method": "S256", "state": state, "resource": resource},
                    "decision": {"_tag": "pairing-code", "access": "full-access", "code": code}
                }),
            )
            .await?;
        let redirect =
            decision["redirectTo"].as_str().ok_or_else(|| T3Error::Unavailable("decision: no redirectTo".into()))?;
        let url = reqwest::Url::parse(redirect)
            .or_else(|_| reqwest::Url::parse(&origin).and_then(|o| o.join(redirect)))
            .map_err(|e| T3Error::Unavailable(format!("decision: bad redirectTo: {e}")))?;
        let q = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
        if q("state").as_deref() != Some(state.as_str()) {
            return Err(T3Error::Unavailable("decision: state mismatch".into()));
        }
        let auth_code = q("code").ok_or_else(|| T3Error::Unavailable("decision: no authorization code".into()))?;
        let resp = self
            .http
            .post(format!("{origin}/oauth/mcp/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", auth_code.as_str()),
                ("redirect_uri", REDIRECT),
                ("client_id", client_id.as_str()),
                ("code_verifier", verifier.as_str()),
                ("resource", resource.as_str()),
            ])
            .send()
            .await
            .map_err(|e| T3Error::Unavailable(format!("token: {e}")))?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| T3Error::Unavailable(format!("token: {e}")))?;
        if !status.is_success() {
            return Err(T3Error::Unavailable(format!("token: HTTP {status}: {}", body["error"])));
        }
        let access =
            body["access_token"].as_str().ok_or_else(|| T3Error::Unavailable("token: no access_token".into()))?;
        let ttl = body["expires_in"].as_f64().unwrap_or(3600.0);
        let now = self.clock.now();
        let token = StoredToken {
            access_token: access.to_string(),
            client_id,
            expires_at: iso(crate::clock::plus_minutes(now, ttl / 60.0)),
            obtained_at: iso(now),
        };
        self.store.token_set(&token).map_err(|e| T3Error::Unavailable(format!("store: {e}")))?;
        *self.token.write().unwrap() = Some(token.clone());
        *self.session.lock().await = None;
        match revoke_older_sessions(&bin, &self.label).await {
            Ok(ids) if !ids.is_empty() => tracing::info!(revoked = ?ids, "revoked older t3-codemode sessions"),
            Ok(_) => {}
            Err(e) => tracing::warn!("could not revoke older t3-codemode sessions: {e}"),
        }
        tracing::info!(expires_at = %token.expires_at, "acquired a T3 OAuth session");
        Ok(token)
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<Value, T3Error> {
        let resp =
            self.http.post(url).json(body).send().await.map_err(|e| T3Error::Unavailable(format!("{url}: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(T3Error::Unavailable(format!("{url}: HTTP {status}: {}", truncate_chars(&text, 300))));
        }
        serde_json::from_str(&text).map_err(|e| T3Error::Unavailable(format!("{url}: {e}")))
    }

    async fn post_mcp(
        &self,
        origin: &str,
        token: &str,
        sid: Option<&str>,
        body: &Value,
    ) -> Result<(Option<String>, Value), PostError> {
        let mut req = self
            .http
            .post(format!("{origin}/mcp"))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .json(body);
        if let Some(s) = sid {
            req = req.header("Mcp-Session-Id", s);
        }
        let resp = req.send().await.map_err(|e| PostError::Transport(e.to_string()))?;
        let status = resp.status();
        let new_sid = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(String::from);
        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let raw = resp.text().await.map_err(|e| PostError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(PostError::Status(status.as_u16(), truncate_chars(&raw, 300)));
        }
        let v = parse_body(&ct, &raw).map_err(PostError::Transport)?;
        Ok((new_sid, v))
    }

    async fn session(&self, token: &StoredToken) -> Result<(String, Option<String>), PostError> {
        let mut guard = self.session.lock().await;
        if let Some(s) = guard.as_ref().filter(|s| s.token == token.access_token) {
            return Ok((s.origin.clone(), s.id.clone()));
        }
        let origin = runtime().map_err(|e| PostError::Transport(e.to_string()))?.origin;
        let init = json!({"jsonrpc": "2.0", "id": self.next_id.fetch_add(1, Ordering::Relaxed), "method": "initialize",
            "params": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                       "clientInfo": {"name": LABEL, "version": env!("CARGO_PKG_VERSION")}}});
        let (sid, res) = self.post_mcp(&origin, &token.access_token, None, &init).await?;
        if res.get("result").is_none() {
            return Err(PostError::Transport(format!(
                "MCP initialize failed: {}",
                truncate_chars(&res.to_string(), 300)
            )));
        }
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        self.post_mcp(&origin, &token.access_token, sid.as_deref(), &note).await?;
        *guard = Some(Session { origin: origin.clone(), token: token.access_token.clone(), id: sid.clone() });
        Ok((origin, sid))
    }

    fn note_status(&self, err: Option<&str>) {
        let mut st = self.status.lock().unwrap();
        st.connected = err.is_none();
        st.last_error = err.map(|e| self.redact(e.to_string()));
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, T3Error> {
        let mut reauthed = false;
        let mut reinit = false;
        loop {
            let token = match self.ensure_token(false).await {
                Ok(t) => t,
                Err(e) => {
                    self.note_status(Some(e.message()));
                    return Err(e);
                }
            };
            let attempt = async {
                let (origin, sid) = self.session(&token).await?;
                let body = json!({"jsonrpc": "2.0", "id": self.next_id.fetch_add(1, Ordering::Relaxed), "method": method, "params": params.clone()});
                self.post_mcp(&origin, &token.access_token, sid.as_deref(), &body).await.map(|(_, v)| v)
            };
            match attempt.await {
                Ok(v) => {
                    self.note_status(None);
                    if let Some(e) = v.get("error") {
                        let msg = e["message"].as_str().map(String::from).unwrap_or_else(|| e.to_string());
                        return Err(T3Error::Tool(msg));
                    }
                    return Ok(v["result"].clone());
                }
                Err(PostError::Status(401, _)) if !reauthed => {
                    reauthed = true;
                    *self.session.lock().await = None;
                    let stored = self.store.token_get().ok().flatten();
                    if stored.as_ref().is_some_and(|t| t.access_token != token.access_token) {
                        *self.token.write().unwrap() = stored;
                    } else {
                        let _ = self.store.token_clear();
                        *self.token.write().unwrap() = None;
                    }
                }
                Err(PostError::Status(400 | 404, _)) if !reinit => {
                    reinit = true;
                    *self.session.lock().await = None;
                }
                Err(e) => {
                    let msg = match e {
                        PostError::Status(code, body) => format!("T3 /mcp HTTP {code}: {body}"),
                        PostError::Transport(m) => format!("T3 is unreachable: {m}"),
                    };
                    let msg = self.redact(msg);
                    self.note_status(Some(&msg));
                    *self.session.lock().await = None;
                    return Err(T3Error::Unavailable(msg));
                }
            }
        }
    }

    pub fn token_expiry(&self) -> Option<String> {
        self.token.read().unwrap().as_ref().map(|t| t.expires_at.clone())
    }

    pub async fn force_acquire(&self) -> Result<StoredToken, T3Error> {
        self.ensure_token(true).await
    }
}

pub fn instance_label(store_path: &Path) -> String {
    let canon = std::fs::canonicalize(store_path).unwrap_or_else(|_| store_path.to_path_buf());
    let digest = Sha256::digest(canon.to_string_lossy().as_bytes());
    format!("{LABEL}@{}", hex8(&digest))
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

pub async fn revoke_older_sessions(bin: &PathBuf, label: &str) -> Result<Vec<String>, T3Error> {
    let out = run_t3(bin, &["auth", "session", "list", "--json"]).await?;
    let list = json_from_cli(&out).ok_or_else(|| T3Error::Unavailable("session list: no JSON output".into()))?;
    let items = match &list {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o.values().find_map(|v| v.as_array().cloned()).unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut ours: Vec<(String, String)> = items
        .iter()
        .filter(|s| {
            let l = s["client"]["label"].as_str().or_else(|| s["label"].as_str()).unwrap_or("");
            l == label
        })
        .filter_map(|s| Some((s["issuedAt"].as_str().unwrap_or("").to_string(), s["sessionId"].as_str()?.to_string())))
        .collect();
    ours.sort_by_key(|(at, _)| parse_ts(at).map(|t| t.as_second()).unwrap_or(0));
    ours.pop();
    let mut revoked = Vec::new();
    for (_, id) in ours {
        if run_t3(bin, &["auth", "session", "revoke", &id]).await.is_ok() {
            revoked.push(id);
        }
    }
    Ok(revoked)
}

impl T3Client for HttpT3 {
    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, T3Error>> {
        Box::pin(async move {
            let res = self.rpc("tools/call", json!({"name": tool, "arguments": args})).await.map_err(|e| match e {
                T3Error::Tool(m) => T3Error::Tool(format!("{tool}: {m}")),
                other => other,
            })?;
            unwrap_tool_result(tool, &res).map_err(|e| match e {
                T3Error::Tool(m) => T3Error::Tool(self.redact(m)),
                other => other,
            })
        })
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<Value>, T3Error>> {
        Box::pin(async move {
            let mut tools = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..20 {
                let params = match &cursor {
                    Some(c) => json!({"cursor": c}),
                    None => json!({}),
                };
                let res = self.rpc("tools/list", params).await?;
                tools.extend(res["tools"].as_array().cloned().unwrap_or_default());
                match res["nextCursor"].as_str() {
                    Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
                    _ => break,
                }
            }
            Ok(tools)
        })
    }

    fn status(&self) -> T3Status {
        let mut st = self.status.lock().unwrap().clone();
        st.origin = runtime().ok().map(|r| r.origin);
        st.token_expires_at = self.token_expiry();
        st
    }

    fn redact(&self, text: String) -> String {
        match self.token.read().unwrap().as_ref() {
            Some(t) if !t.access_token.is_empty() && text.contains(&t.access_token) => {
                text.replace(&t.access_token, "[redacted]")
            }
            _ => text,
        }
    }
}
