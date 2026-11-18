use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::client_stdio::{
    RpcError, accept_initialize, call_outcome, handshake_error, initialize_params, into_reply, is_list_changed,
    list_pages, messages, redact_error, response_id, server_request_reply,
};
use super::{
    BoxFut, ERROR_CHARS, MAX_MESSAGE_BYTES, ResolvedTransport, ResolvedUpstream, Secret, ToolDef, Transport, Upstream,
    UpstreamError, UpstreamState, UpstreamStatus, is_loopback_host, qualified, redact_secrets,
};
use crate::t3::truncate_chars;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DELETE_TIMEOUT: Duration = Duration::from_secs(2);
const BODY_CHARS: usize = 300;
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";
const ERROR_BODY_BYTES: usize = 16 * 1024;
const SERVER_REPLIES: usize = 8;

enum PostError {
    SessionGone,
    Failed(UpstreamError),
}

fn is_loopback(url: &reqwest::Url) -> bool {
    is_loopback_host(url.host_str().unwrap_or_default())
}

fn reqwest_error(e: reqwest::Error) -> String {
    error_chain(&e.without_url())
}

fn too_large() -> String {
    format!("the response is over {MAX_MESSAGE_BYTES} bytes")
}

async fn read_limited(resp: reqwest::Response, cap: usize, strict: bool) -> Result<Vec<u8>, String> {
    if strict && resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(too_large());
    }
    let mut stream = resp.bytes_stream();
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(reqwest_error)?;
        if out.len() + chunk.len() > cap {
            if strict {
                return Err(too_large());
            }
            out.extend_from_slice(&chunk[..cap - out.len()]);
            break;
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let next = s.to_string();
        if !msg.contains(&next) {
            msg.push_str(": ");
            msg.push_str(&next);
        }
        source = s.source();
    }
    msg
}

struct Inner {
    ns: String,
    url: reqwest::Url,
    headers: HeaderMap,
    secrets: Vec<Secret>,
    timeout: Duration,
    http: reqwest::Client,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
    server: Mutex<Option<Value>>,
    next_id: AtomicU64,
    changed: AtomicBool,
    last_error: Mutex<Option<String>>,
    tools: AtomicUsize,
    reinit: tokio::sync::Mutex<()>,
    replies: Arc<tokio::sync::Semaphore>,
}

impl Inner {
    fn unavailable(&self, msg: String) -> UpstreamError {
        UpstreamError::Unavailable(truncate_chars(&redact_secrets(msg, &self.secrets), ERROR_CHARS))
    }

    fn request(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        let mut req = self.http.request(method, self.url.clone()).headers(self.headers.clone());
        if let Some(v) = self.version.lock().unwrap().clone() {
            req = req.header(VERSION_HEADER, v);
        }
        if let Some(s) = self.session.lock().unwrap().clone() {
            req = req.header(SESSION_HEADER, s);
        }
        req
    }

    async fn post(self: &Arc<Self>, body: &Value, id: Option<u64>) -> Result<Option<Value>, PostError> {
        let ns = &self.ns;
        let had_session = self.session.lock().unwrap().is_some();
        let resp = self
            .request(reqwest::Method::POST)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| PostError::Failed(self.unavailable(format!("{ns}: request failed: {}", reqwest_error(e)))))?;
        let status = resp.status();
        match status.as_u16() {
            401 => return Err(PostError::Failed(self.unavailable(format!("{ns}: HTTP 401 unauthorized")))),
            403 => return Err(PostError::Failed(self.unavailable(format!("{ns}: HTTP 403 forbidden")))),
            404 if had_session => return Err(PostError::SessionGone),
            _ => {}
        }
        if !status.is_success() {
            let raw = read_limited(resp, ERROR_BODY_BYTES, false).await.unwrap_or_default();
            let text = String::from_utf8_lossy(&raw).into_owned();
            return Err(PostError::Failed(self.unavailable(format!(
                "{ns}: HTTP {}: {}",
                status.as_u16(),
                truncate_chars(&redact_secrets(text, &self.secrets), BODY_CHARS)
            ))));
        }
        if let Some(sid) = resp.headers().get(SESSION_HEADER).and_then(|v| v.to_str().ok()).filter(|s| !s.is_empty()) {
            *self.session.lock().unwrap() = Some(sid.to_string());
        }
        let Some(id) = id else { return Ok(None) };
        let content_type =
            resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
        if content_type.starts_with("text/event-stream") {
            return self.read_events(resp, id).await.map(Some);
        }
        let raw = read_limited(resp, MAX_MESSAGE_BYTES, true)
            .await
            .map_err(|e| PostError::Failed(self.unavailable(format!("{ns}: reading the response failed: {e}"))))?;
        let v: Value = serde_json::from_slice(&raw)
            .map_err(|e| PostError::Failed(self.unavailable(format!("{ns}: response is not JSON: {e}"))))?;
        match self.take(v, id) {
            Some(msg) => Ok(Some(msg)),
            None => Err(PostError::Failed(self.unavailable(format!("{ns}: the response had no reply")))),
        }
    }

    fn take(self: &Arc<Self>, v: Value, id: u64) -> Option<Value> {
        let mut found = None;
        for msg in messages(v) {
            if found.is_none() && response_id(&msg) == Some(id) {
                found = Some(msg);
            } else {
                self.incoming(msg);
            }
        }
        found
    }

    fn incoming(self: &Arc<Self>, msg: Value) {
        if let Some(reply) = server_request_reply(&msg) {
            let Ok(permit) = self.replies.clone().try_acquire_owned() else {
                tracing::debug!(upstream = %self.ns, "dropped a reply to a server request: too many in flight");
                return;
            };
            let inner = self.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(inner.timeout, inner.post(&reply, None)).await;
                drop(permit);
            });
        } else if is_list_changed(&msg) {
            self.changed.store(true, Ordering::SeqCst);
        }
    }

    async fn read_events(self: &Arc<Self>, resp: reqwest::Response, id: u64) -> Result<Value, PostError> {
        let ns = &self.ns;
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut data = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                PostError::Failed(self.unavailable(format!("{ns}: event stream failed: {}", reqwest_error(e))))
            })?;
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_MESSAGE_BYTES || data.len() > MAX_MESSAGE_BYTES {
                return Err(PostError::Failed(self.unavailable(format!("{ns}: event stream: {}", too_large()))));
            }
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let raw: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&raw);
                let line = line.trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    if let Some(found) = self.event(&std::mem::take(&mut data), id) {
                        return Ok(found);
                    }
                } else if let Some(d) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(d.strip_prefix(' ').unwrap_or(d));
                    if data.len() > MAX_MESSAGE_BYTES {
                        return Err(PostError::Failed(
                            self.unavailable(format!("{ns}: event stream: {}", too_large())),
                        ));
                    }
                }
            }
        }
        if let Some(found) = self.event(&data, id) {
            return Ok(found);
        }
        Err(PostError::Failed(self.unavailable(format!("{ns}: the event stream ended without a reply"))))
    }

    fn event(self: &Arc<Self>, data: &str, id: u64) -> Option<Value> {
        if data.trim().is_empty() {
            return None;
        }
        let v: Value = serde_json::from_str(data).ok()?;
        self.take(v, id)
    }

    async fn rpc(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, RpcError> {
        let mut resent = false;
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params.clone()});
            let used = self.session.lock().unwrap().clone();
            let mut guard = CancelGuard { inner: Some(self.clone()), id, armed: method != "initialize" };
            let posted = self.post(&body, Some(id)).await;
            guard.armed = false;
            match posted {
                Ok(Some(msg)) => return into_reply(msg),
                Ok(None) => {
                    return Err(RpcError::Failed(self.unavailable(format!("{}: {method}: empty response", self.ns))));
                }
                Err(PostError::SessionGone) if !resent => {
                    resent = true;
                    self.reinitialize(used).await.map_err(RpcError::Failed)?;
                }
                Err(PostError::SessionGone) => {
                    return Err(RpcError::Failed(
                        self.unavailable(format!("{}: HTTP 404: session not found", self.ns)),
                    ));
                }
                Err(PostError::Failed(e)) => return Err(RpcError::Failed(e)),
            }
        }
    }

    async fn reinitialize(self: &Arc<Self>, stale: Option<String>) -> Result<(), UpstreamError> {
        let _guard = self.reinit.lock().await;
        if *self.session.lock().unwrap() != stale {
            return Ok(());
        }
        *self.session.lock().unwrap() = None;
        *self.version.lock().unwrap() = None;
        self.initialize().await
    }

    async fn initialize(self: &Arc<Self>) -> Result<(), UpstreamError> {
        let ns = &self.ns;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": initialize_params()});
        let msg = match self.post(&body, Some(id)).await {
            Ok(Some(msg)) => msg,
            Ok(None) => return Err(self.unavailable(format!("{ns}: initialize: empty response"))),
            Err(PostError::SessionGone) => return Err(self.unavailable(format!("{ns}: initialize: HTTP 404"))),
            Err(PostError::Failed(e)) => return Err(e),
        };
        let result = into_reply(msg).map_err(|e| handshake_error(ns, "initialize", e))?;
        let (version, server) = accept_initialize(ns, &result)?;
        *self.version.lock().unwrap() = Some(version);
        *self.server.lock().unwrap() = server;
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        match self.post(&note, None).await {
            Ok(_) => Ok(()),
            Err(PostError::SessionGone) => Err(self.unavailable(format!("{ns}: notifications/initialized: HTTP 404"))),
            Err(PostError::Failed(e)) => Err(e),
        }
    }

    async fn timed<T>(&self, label: &str, fut: impl Future<Output = Result<T, RpcError>>) -> Result<T, RpcError> {
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(r) => r,
            Err(_) => Err(RpcError::Failed(UpstreamError::Timeout(format!(
                "{label}: timed out after {} ms",
                self.timeout.as_millis()
            )))),
        }
    }

    fn finish<T>(&self, r: Result<T, UpstreamError>) -> Result<T, UpstreamError> {
        r.map_err(|e| {
            let e = redact_error(e, &self.secrets);
            *self.last_error.lock().unwrap() = Some(e.message().to_string());
            e
        })
    }
}

struct CancelGuard {
    inner: Option<Arc<Inner>>,
    id: u64,
    armed: bool,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take().filter(|_| self.armed) else { return };
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let note = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": self.id, "reason": "timed out"}});
        rt.spawn(async move {
            let _ = tokio::time::timeout(DELETE_TIMEOUT, inner.post(&note, None)).await;
        });
    }
}

pub struct HttpClient {
    name: String,
    inner: Arc<Inner>,
}

impl HttpClient {
    pub async fn connect(spec: &ResolvedUpstream) -> Result<HttpClient, UpstreamError> {
        let ns = &spec.name;
        let ResolvedTransport::Http { url, headers } = &spec.transport else {
            return Err(UpstreamError::Config(format!("{ns}: not an http upstream")));
        };
        let url =
            reqwest::Url::parse(url.expose()).map_err(|e| UpstreamError::Config(format!("{ns}: invalid url: {e}")))?;
        let mut map = HeaderMap::new();
        for (key, value) in headers {
            let name = HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| UpstreamError::Config(format!("{ns}: invalid header name {key:?}")))?;
            let mut value = HeaderValue::from_str(value.expose())
                .map_err(|_| UpstreamError::Config(format!("{ns}: header {key} has an invalid value")))?;
            value.set_sensitive(true);
            map.append(name, value);
        }
        let mut builder =
            reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT).redirect(reqwest::redirect::Policy::none());
        if is_loopback(&url) {
            builder = builder.no_proxy();
        }
        let http = builder.build().map_err(|e| {
            UpstreamError::Unavailable(format!("{ns}: cannot build the http client: {}", reqwest_error(e)))
        })?;
        let inner = Arc::new(Inner {
            ns: ns.clone(),
            url,
            headers: map,
            secrets: spec.secrets.clone(),
            timeout: spec.timeout,
            http,
            session: Mutex::new(None),
            version: Mutex::new(None),
            server: Mutex::new(None),
            next_id: AtomicU64::new(1),
            changed: AtomicBool::new(false),
            last_error: Mutex::new(None),
            tools: AtomicUsize::new(0),
            reinit: tokio::sync::Mutex::new(()),
            replies: Arc::new(tokio::sync::Semaphore::new(SERVER_REPLIES)),
        });
        let label = format!("{ns}: initialize");
        match inner.timed(&label, async { inner.initialize().await.map_err(RpcError::Failed) }).await {
            Ok(()) => Ok(HttpClient { name: ns.clone(), inner }),
            Err(e) => {
                let e = match handshake_error(ns, "initialize", e) {
                    UpstreamError::Timeout(m) => UpstreamError::Unavailable(m),
                    e => e,
                };
                Err(redact_error(e, &spec.secrets))
            }
        }
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.session.lock().unwrap().clone()
    }
}

impl Upstream for HttpClient {
    fn name(&self) -> &str {
        &self.name
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>> {
        Box::pin(async move {
            let inner = &self.inner;
            let label = format!("{}: tools/list", self.name);
            let r = list_pages(&self.name, |params| inner.timed(&label, inner.rpc("tools/list", params))).await;
            if let Ok(tools) = &r {
                inner.tools.store(tools.len(), Ordering::Relaxed);
            }
            inner.finish(r)
        })
    }

    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>> {
        Box::pin(async move {
            let inner = &self.inner;
            let label = qualified(&self.name, tool);
            let r = inner.timed(&label, inner.rpc("tools/call", json!({"name": tool, "arguments": args}))).await;
            inner.finish(call_outcome(&self.name, tool, r))
        })
    }

    fn status(&self) -> UpstreamStatus {
        UpstreamStatus {
            state: UpstreamState::Ready,
            tools: self.inner.tools.load(Ordering::Relaxed),
            last_error: self.inner.last_error.lock().unwrap().clone(),
            server: self.inner.server.lock().unwrap().clone(),
            ..UpstreamStatus::new(&self.name, Transport::Http)
        }
    }

    fn tools_changed(&self) -> bool {
        self.inner.changed.swap(false, Ordering::SeqCst)
    }

    fn redact(&self, text: String) -> String {
        redact_secrets(text, &self.inner.secrets)
    }

    fn shutdown(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            if self.inner.session.lock().unwrap().is_none() {
                return;
            }
            let delete = self.inner.request(reqwest::Method::DELETE).send();
            let _ = tokio::time::timeout(DELETE_TIMEOUT, delete).await;
            *self.inner.session.lock().unwrap() = None;
        })
    }
}
