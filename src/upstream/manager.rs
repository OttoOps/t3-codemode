use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{Shared, join_all};
use jiff::{SignedDuration, Timestamp};
use serde_json::Value;
use tokio::time::Instant;

use super::config::resolve;
use super::{
    BACKOFF_MAX, BACKOFF_MIN, BoxFut, CONNECT_TIMEOUT, Connector, McpConnector, Secret, T3_NAMESPACE, ToolDef,
    ToolFilter, Upstream, UpstreamConfig, UpstreamError, UpstreamState, UpstreamStatus, qualified, redact_json,
    redact_secrets, server_info,
};

type Conn = Arc<dyn Upstream>;
type ConnectFut = Shared<BoxFut<'static, Result<Conn, UpstreamError>>>;

pub const MAX_TIMEOUTS: u32 = 3;
const BACKSTOP_GRACE: Duration = Duration::from_millis(250);

pub fn connect_budget(config: &UpstreamConfig) -> Duration {
    CONNECT_TIMEOUT.max(Duration::from_millis(config.timeout_ms))
}

pub struct Redactor {
    secrets: Vec<Secret>,
    conns: Vec<Conn>,
}

impl Redactor {
    pub fn apply(&self, text: String) -> String {
        let mut out = redact_secrets(text, &self.secrets);
        for c in &self.conns {
            out = c.redact(out);
        }
        out
    }
}

pub struct UpstreamManager {
    inner: Arc<Inner>,
}

struct Inner {
    connector: Arc<dyn Connector>,
    slots: Vec<Slot>,
    generation: AtomicU64,
    closed: AtomicBool,
}

struct Slot {
    config: UpstreamConfig,
    filter: ToolFilter,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    conn: Option<Conn>,
    connecting: Option<ConnectFut>,
    tools: Option<Vec<ToolDef>>,
    state: UpstreamState,
    failures: u32,
    timeouts: u32,
    restarts: u32,
    connect_started: Option<Instant>,
    last_error: Option<String>,
    retry_at: Option<(Instant, String)>,
    secrets: Vec<Secret>,
    server: Option<Value>,
}

fn same(a: &Conn, b: &Conn) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

fn with_message(e: &UpstreamError, message: String) -> UpstreamError {
    match e {
        UpstreamError::Tool(_) => UpstreamError::Tool(message),
        UpstreamError::Unavailable(_) => UpstreamError::Unavailable(message),
        UpstreamError::Timeout(_) => UpstreamError::Timeout(message),
        UpstreamError::Denied(_) => UpstreamError::Denied(message),
        UpstreamError::UnknownTool(_) => UpstreamError::UnknownTool(message),
        UpstreamError::UnknownNamespace(_) => UpstreamError::UnknownNamespace(message),
        UpstreamError::Config(_) => UpstreamError::Config(message),
    }
}

fn backoff_delay(failures: u32) -> Duration {
    if failures <= 1 {
        return Duration::ZERO;
    }
    let shift = (failures - 2).min(16);
    BACKOFF_MIN.saturating_mul(1 << shift).min(BACKOFF_MAX)
}

fn iso_ms(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn filtered(filter: &ToolFilter, tools: Vec<ToolDef>) -> Vec<ToolDef> {
    let mut seen = BTreeSet::new();
    tools.into_iter().filter(|t| seen.insert(t.name.clone())).filter(|t| filter.allows(&t.name)).collect()
}

impl Slot {
    fn lock(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn name(&self) -> &str {
        &self.config.name
    }
}

impl Inner {
    fn enabled(&self) -> impl Iterator<Item = (usize, &Slot)> {
        self.slots.iter().enumerate().filter(|(_, s)| s.config.enabled)
    }

    fn find(&self, ns: &str) -> Option<usize> {
        self.enabled().find(|(_, s)| s.name() == ns).map(|(i, _)| i)
    }

    fn redactor(&self) -> Redactor {
        let mut secrets: Vec<Secret> = Vec::new();
        let mut conns: Vec<Conn> = Vec::new();
        for slot in &self.slots {
            let st = slot.lock();
            secrets.extend(st.secrets.iter().cloned());
            conns.extend(st.conn.iter().cloned());
        }
        secrets.sort_by_key(|s| std::cmp::Reverse(s.expose().len()));
        Redactor { secrets, conns }
    }

    fn redact(&self, text: String) -> String {
        self.redactor().apply(text)
    }

    fn redact_tool(r: &Redactor, tool: ToolDef) -> ToolDef {
        let f = |s: String| r.apply(s);
        ToolDef {
            description: r.apply(tool.description),
            input_schema: redact_json(tool.input_schema, &f),
            output_schema: tool.output_schema.map(|v| redact_json(v, &f)),
            annotations: tool.annotations.map(|v| redact_json(v, &f)),
            name: tool.name,
        }
    }

    fn redact_error(&self, e: UpstreamError) -> UpstreamError {
        let message = self.redact(e.message().to_string());
        with_message(&e, message)
    }

    fn remember_secrets(&self, idx: usize, secrets: &[Secret]) {
        let mut st = self.slots[idx].lock();
        for s in secrets {
            if !st.secrets.iter().any(|known| known == s) {
                st.secrets.push(s.clone());
            }
        }
    }

    fn set_tools(&self, idx: usize, tools: Vec<ToolDef>) -> usize {
        let slot = &self.slots[idx];
        let r = self.redactor();
        let tools: Vec<ToolDef> =
            filtered(&slot.filter, tools).into_iter().map(|t| Inner::redact_tool(&r, t)).collect();
        let count = tools.len();
        let mut st = slot.lock();
        if st.tools.as_ref() != Some(&tools) {
            st.tools = Some(tools);
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        count
    }

    fn install(&self, idx: usize, conn: Conn, tools: Vec<ToolDef>) -> bool {
        let count = self.set_tools(idx, tools);
        let r = self.redactor();
        let server = server_info(conn.status().server, &|s| r.apply(s));
        let mut st = self.slots[idx].lock();
        if self.closed.load(Ordering::SeqCst) {
            st.connecting = None;
            return false;
        }
        tracing::info!(upstream = %self.slots[idx].name(), tools = count, "upstream connected");
        st.conn = Some(conn);
        st.connecting = None;
        st.connect_started = None;
        st.state = UpstreamState::Ready;
        st.failures = 0;
        st.timeouts = 0;
        st.retry_at = None;
        st.last_error = None;
        st.server = server;
        true
    }

    fn fail(&self, idx: usize, expected: Option<&Conn>, err: UpstreamError) -> (UpstreamError, Option<Conn>) {
        let err = self.redact_error(err);
        let mut st = self.slots[idx].lock();
        if let Some(conn) = expected
            && !st.conn.as_ref().is_some_and(|c| same(c, conn))
        {
            return (err, None);
        }
        let dropped = st.conn.take();
        st.connecting = None;
        st.connect_started = None;
        st.timeouts = 0;
        st.state = UpstreamState::Backoff;
        st.restarts = st.restarts.saturating_add(1);
        st.failures = st.failures.saturating_add(1);
        let delay = backoff_delay(st.failures);
        let wall = Timestamp::now().checked_add(SignedDuration::try_from(delay).unwrap_or_default());
        st.retry_at = Some((Instant::now() + delay, iso_ms(wall.unwrap_or_else(|_| Timestamp::now()))));
        st.last_error = Some(err.message().to_string());
        tracing::warn!(
            upstream = %self.slots[idx].name(),
            restarts = st.restarts,
            retry_at = %st.retry_at.as_ref().map(|r| r.1.as_str()).unwrap_or_default(),
            "upstream unavailable: {}",
            err.message()
        );
        (err, dropped)
    }

    async fn fail_and_close(&self, idx: usize, expected: Option<&Conn>, err: UpstreamError) -> UpstreamError {
        let (err, dropped) = self.fail(idx, expected, err);
        if let Some(conn) = dropped {
            conn.shutdown().await;
        }
        err
    }

    async fn attempt(&self, idx: usize) -> Result<(Conn, Vec<ToolDef>), UpstreamError> {
        let config = &self.slots[idx].config;
        let budget = connect_budget(config);
        let deadline = Instant::now() + budget;
        let timed_out =
            || UpstreamError::Unavailable(format!("{}: connect timed out after {} s", config.name, budget.as_secs()));
        let spec = resolve(config)?;
        self.remember_secrets(idx, &spec.secrets);
        let conn =
            tokio::time::timeout_at(deadline, self.connector.connect(&spec)).await.map_err(|_| timed_out())??;
        match tokio::time::timeout_at(deadline, conn.list_tools()).await {
            Ok(Ok(tools)) => Ok((conn, tools)),
            Ok(Err(e)) => {
                conn.shutdown().await;
                Err(e)
            }
            Err(_) => {
                conn.shutdown().await;
                Err(timed_out())
            }
        }
    }

    async fn connect_task(self: Arc<Inner>, idx: usize) -> Result<Conn, UpstreamError> {
        match self.attempt(idx).await {
            Ok((conn, tools)) => {
                if self.install(idx, conn.clone(), tools) {
                    Ok(conn)
                } else {
                    conn.shutdown().await;
                    Err(self.shutting_down(idx))
                }
            }
            Err(e) => Err(self.fail_and_close(idx, None, e).await),
        }
    }

    fn shutting_down(&self, idx: usize) -> UpstreamError {
        UpstreamError::Unavailable(format!("{}: the service is shutting down", self.slots[idx].name()))
    }

    fn fail_fast(&self, idx: usize, st: &SlotState) -> Option<UpstreamError> {
        if st.state != UpstreamState::Backoff {
            return None;
        }
        let (at, iso) = st.retry_at.as_ref()?;
        if Instant::now() >= *at {
            return None;
        }
        let last = st.last_error.clone().unwrap_or_default();
        Some(UpstreamError::Unavailable(format!("{} is unavailable (retry at {iso}): {last}", self.slots[idx].name())))
    }

    fn pending(self: &Arc<Inner>, idx: usize) -> Result<Result<Conn, ConnectFut>, UpstreamError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(self.shutting_down(idx));
        }
        let mut st = self.slots[idx].lock();
        if let Some(conn) = &st.conn {
            if conn.is_alive() {
                return Ok(Ok(conn.clone()));
            }
            let dead = st.conn.take().expect("checked above");
            st.restarts = st.restarts.saturating_add(1);
            st.last_error = dead.status().last_error.map(|e| self.redact_unlocked(&st, e));
            tracing::info!(upstream = %self.slots[idx].name(), "upstream exited; reconnecting");
            tokio::spawn(async move { dead.shutdown().await });
        }
        if let Some(fut) = &st.connecting {
            return Ok(Err(fut.clone()));
        }
        if let Some(e) = self.fail_fast(idx, &st) {
            return Err(e);
        }
        let name = self.slots[idx].name().to_string();
        let handle = tokio::spawn(self.clone().connect_task(idx));
        let task: BoxFut<'static, Result<Conn, UpstreamError>> = Box::pin(async move {
            handle.await.unwrap_or_else(|e| Err(UpstreamError::Unavailable(format!("{name}: connect failed: {e}"))))
        });
        let fut = task.shared();
        st.state = UpstreamState::Connecting;
        st.connecting = Some(fut.clone());
        st.connect_started = Some(Instant::now());
        Ok(Err(fut))
    }

    fn redact_unlocked(&self, st: &SlotState, text: String) -> String {
        redact_secrets(text, &st.secrets)
    }

    fn run_deadline(&self, idx: usize, budget: Duration) -> Option<Instant> {
        let st = self.slots[idx].lock();
        let now = Instant::now();
        if st.conn.as_ref().is_some_and(|c| c.is_alive()) {
            return Some(now + budget);
        }
        if st.tools.is_some() {
            return None;
        }
        if st.connecting.is_some() {
            return st.connect_started.map(|t| t + budget);
        }
        if self.fail_fast(idx, &st).is_some() {
            return None;
        }
        Some(now + budget)
    }

    fn note_success(&self, idx: usize, conn: &Conn) {
        let mut st = self.slots[idx].lock();
        if st.conn.as_ref().is_some_and(|c| same(c, conn)) {
            st.timeouts = 0;
        }
    }

    async fn note_timeout(&self, idx: usize, conn: &Conn) {
        let count = {
            let mut st = self.slots[idx].lock();
            if !st.conn.as_ref().is_some_and(|c| same(c, conn)) {
                return;
            }
            st.timeouts = st.timeouts.saturating_add(1);
            st.timeouts
        };
        if count >= MAX_TIMEOUTS {
            let name = self.slots[idx].name();
            let e = UpstreamError::Unavailable(format!("{name}: {count} requests in a row timed out; restarting it"));
            self.fail_and_close(idx, Some(conn), e).await;
        }
    }

    async fn ensure(self: &Arc<Inner>, idx: usize) -> Result<Conn, UpstreamError> {
        match self.pending(idx)? {
            Ok(conn) => Ok(conn),
            Err(fut) => fut.await,
        }
    }

    async fn relist(&self, idx: usize, conn: &Conn) -> Result<usize, UpstreamError> {
        let listed = match tokio::time::timeout(CONNECT_TIMEOUT, conn.list_tools()).await {
            Ok(r) => r,
            Err(_) => Err(UpstreamError::Timeout(format!(
                "{}: tools/list timed out after {} s",
                self.slots[idx].name(),
                CONNECT_TIMEOUT.as_secs()
            ))),
        };
        match listed {
            Ok(tools) => {
                self.note_success(idx, conn);
                Ok(self.set_tools(idx, tools))
            }
            Err(e) if e.restarts_connection() => Err(self.fail_and_close(idx, Some(conn), e).await),
            Err(e @ UpstreamError::Timeout(_)) => {
                self.note_timeout(idx, conn).await;
                Err(self.redact_error(e))
            }
            Err(e) => Err(self.redact_error(e)),
        }
    }

    async fn relist_if_changed(&self, idx: usize, conn: &Conn) {
        if conn.tools_changed() {
            let _ = self.relist(idx, conn).await;
        }
    }

    async fn warm(self: &Arc<Inner>, idx: usize) {
        if let Ok(conn) = self.ensure(idx).await {
            self.relist_if_changed(idx, &conn).await;
        }
    }

    async fn refresh_one(self: &Arc<Inner>, idx: usize) -> Result<usize, String> {
        let live = self.slots[idx].lock().conn.clone();
        let result = match live {
            Some(conn) => self.relist(idx, &conn).await,
            None => self.ensure(idx).await.map(|_| self.slots[idx].lock().tools.as_ref().map_or(0, Vec::len)),
        };
        result.map_err(|e| e.message().to_string())
    }

    fn known_tool(&self, idx: usize, tool: &str) -> bool {
        self.slots[idx].lock().tools.as_ref().is_some_and(|ts| ts.iter().any(|t| t.name == tool))
    }

    async fn call(self: &Arc<Inner>, ns: &str, tool: &str, args: Value) -> Result<Value, UpstreamError> {
        let Some(idx) = self.find(ns) else {
            let mut names = vec![T3_NAMESPACE.to_string()];
            names.extend(self.enabled().map(|(_, s)| s.name().to_string()));
            return Err(UpstreamError::UnknownNamespace(format!(
                "unknown namespace {ns:?}; namespaces are {}",
                names.join(", ")
            )));
        };
        let slot = &self.slots[idx];
        let name = qualified(ns, tool);
        if !slot.filter.allows(tool) {
            return Err(UpstreamError::Denied(format!(
                "{name} is not allowed by the t3-codemode config for upstream {ns}; do not retry it"
            )));
        }
        let conn = self.ensure(idx).await.map_err(|e| self.redact_error(e))?;
        self.relist_if_changed(idx, &conn).await;
        if !self.known_tool(idx, tool) {
            return Err(UpstreamError::UnknownTool(format!(
                "unknown tool {name}; use the search tool to find the right name"
            )));
        }
        let limit = Duration::from_millis(slot.config.timeout_ms) + BACKSTOP_GRACE;
        let result = match tokio::time::timeout(limit, conn.call(tool, args)).await {
            Ok(r) => r,
            Err(_) => Err(UpstreamError::Timeout(format!("{name}: timed out after {} ms", slot.config.timeout_ms))),
        };
        match result {
            Ok(v) => {
                self.note_success(idx, &conn);
                self.relist_if_changed(idx, &conn).await;
                Ok(v)
            }
            Err(e) if e.restarts_connection() => Err(self.fail_and_close(idx, Some(&conn), e).await),
            Err(e @ UpstreamError::Timeout(_)) => {
                self.note_timeout(idx, &conn).await;
                Err(self.redact_error(e))
            }
            Err(e) => {
                self.note_success(idx, &conn);
                self.relist_if_changed(idx, &conn).await;
                Err(self.redact_error(e))
            }
        }
    }

    fn status_of(&self, slot: &Slot) -> UpstreamStatus {
        let st = slot.lock();
        let enabled = slot.config.enabled;
        let exited = st.conn.as_ref().filter(|c| !c.is_alive()).map(|c| c.status().last_error);
        let state = match (&exited, enabled) {
            (_, false) => UpstreamState::Disabled,
            (Some(_), true) => UpstreamState::Idle,
            (None, true) => st.state,
        };
        let last_error = match exited {
            Some(e) => e.map(|e| self.redact_unlocked(&st, e)),
            None => st.last_error.clone(),
        };
        UpstreamStatus {
            enabled,
            state,
            tools: st.tools.as_ref().map_or(0, Vec::len),
            restarts: st.restarts,
            last_error,
            retry_at: if st.state == UpstreamState::Backoff {
                st.retry_at.as_ref().map(|(_, s)| s.clone())
            } else {
                None
            },
            server: st.server.clone(),
            ..UpstreamStatus::new(slot.name(), slot.config.transport)
        }
    }

    fn cached(&self) -> Vec<(String, Vec<ToolDef>)> {
        self.enabled().map(|(_, s)| (s.name().to_string(), s.lock().tools.clone().unwrap_or_default())).collect()
    }
}

impl UpstreamManager {
    pub fn new(configs: Vec<UpstreamConfig>) -> UpstreamManager {
        UpstreamManager::with_connector(configs, Arc::new(McpConnector))
    }

    pub fn with_connector(configs: Vec<UpstreamConfig>, connector: Arc<dyn Connector>) -> UpstreamManager {
        let slots = configs
            .into_iter()
            .map(|config| {
                let state = SlotState {
                    state: if config.enabled { UpstreamState::Idle } else { UpstreamState::Disabled },
                    ..SlotState::default()
                };
                Slot { filter: ToolFilter::new(&config.allow, &config.deny), config, state: Mutex::new(state) }
            })
            .collect();
        UpstreamManager {
            inner: Arc::new(Inner { connector, slots, generation: AtomicU64::new(0), closed: AtomicBool::new(false) }),
        }
    }

    pub fn empty() -> UpstreamManager {
        UpstreamManager::new(Vec::new())
    }

    pub fn namespaces(&self) -> Vec<String> {
        self.inner.enabled().map(|(_, s)| s.name().to_string()).collect()
    }

    pub fn is_namespace(&self, ns: &str) -> bool {
        self.inner.find(ns).is_some()
    }

    pub async fn tools(&self) -> Vec<(String, Vec<ToolDef>)> {
        let indexes: Vec<usize> = self.inner.enabled().map(|(i, _)| i).collect();
        join_all(indexes.into_iter().map(|i| self.inner.warm(i))).await;
        self.inner.cached()
    }

    pub async fn tools_for_run(&self, budget: Duration) -> Vec<(String, Vec<ToolDef>)> {
        let mut waits = Vec::new();
        for (idx, _) in self.inner.enabled() {
            let deadline = self.inner.run_deadline(idx, budget);
            let inner = self.inner.clone();
            let handle = tokio::spawn(async move { inner.warm(idx).await });
            if let Some(deadline) = deadline {
                waits.push(tokio::time::timeout_at(deadline, handle));
            }
        }
        join_all(waits).await;
        self.inner.cached()
    }

    pub fn cached_tools(&self) -> Vec<(String, Vec<ToolDef>)> {
        self.inner.cached()
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::SeqCst)
    }

    pub async fn refresh(&self) -> Vec<(String, Result<usize, String>)> {
        let targets: Vec<(usize, String)> = self.inner.enabled().map(|(i, s)| (i, s.name().to_string())).collect();
        let results = join_all(targets.iter().map(|(i, _)| self.inner.refresh_one(*i))).await;
        targets.into_iter().map(|(_, n)| n).zip(results).collect()
    }

    pub async fn call(&self, ns: &str, tool: &str, args: Value) -> Result<Value, UpstreamError> {
        self.inner.call(ns, tool, args).await
    }

    pub fn status(&self) -> Vec<UpstreamStatus> {
        self.inner.slots.iter().map(|s| self.inner.status_of(s)).collect()
    }

    pub fn redact(&self, text: String) -> String {
        self.inner.redact(text)
    }

    pub fn redactor(&self) -> Redactor {
        self.inner.redactor()
    }

    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let mut live: Vec<Conn> = Vec::new();
        for slot in &self.inner.slots {
            let mut st = slot.lock();
            if let Some(conn) = st.conn.take() {
                live.push(conn);
                st.state = UpstreamState::Idle;
            }
            st.connecting = None;
        }
        join_all(live.iter().map(|c| c.shutdown())).await;
    }
}
