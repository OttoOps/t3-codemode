use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{
    BoxFut, Connector, ResolvedUpstream, ToolDef, Transport, Upstream, UpstreamError, UpstreamState, UpstreamStatus,
    redact_secrets,
};

pub fn tool(name: &str, description: &str, input_schema: Value) -> ToolDef {
    ToolDef { name: name.into(), description: description.into(), input_schema, output_schema: None, annotations: None }
}

pub struct FakeUpstream {
    name: String,
    tools: Mutex<Vec<ToolDef>>,
    replies: Mutex<BTreeMap<String, Result<Value, UpstreamError>>>,
    calls: Mutex<Vec<(String, Value)>>,
    down: AtomicBool,
    changed: AtomicBool,
    delay: Mutex<Duration>,
    shutdowns: AtomicUsize,
    secrets: Mutex<Vec<super::Secret>>,
    exited: AtomicBool,
    list_delay: Mutex<Duration>,
    server: Mutex<Value>,
}

impl FakeUpstream {
    pub fn new(name: &str, tools: Vec<ToolDef>) -> Arc<FakeUpstream> {
        Arc::new(FakeUpstream {
            name: name.into(),
            tools: Mutex::new(tools),
            replies: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
            down: AtomicBool::new(false),
            changed: AtomicBool::new(false),
            delay: Mutex::new(Duration::ZERO),
            shutdowns: AtomicUsize::new(0),
            secrets: Mutex::new(Vec::new()),
            exited: AtomicBool::new(false),
            list_delay: Mutex::new(Duration::ZERO),
            server: Mutex::new(json!({"name": format!("fake-{name}"), "version": "0"})),
        })
    }

    pub fn reply(&self, tool: &str, result: Result<Value, UpstreamError>) {
        self.replies.lock().unwrap().insert(tool.into(), result);
    }

    pub fn set_tools(&self, tools: Vec<ToolDef>) {
        *self.tools.lock().unwrap() = tools;
        self.changed.store(true, Ordering::SeqCst);
    }

    pub fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    pub fn set_delay(&self, d: Duration) {
        *self.delay.lock().unwrap() = d;
    }

    pub fn set_exited(&self, exited: bool) {
        self.exited.store(exited, Ordering::SeqCst);
    }

    pub fn set_list_delay(&self, d: Duration) {
        *self.list_delay.lock().unwrap() = d;
    }

    pub fn set_server(&self, server: Value) {
        *self.server.lock().unwrap() = server;
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn shutdowns(&self) -> usize {
        self.shutdowns.load(Ordering::SeqCst)
    }

    fn unavailable(&self) -> UpstreamError {
        UpstreamError::Unavailable(format!("{}: connection lost", self.name))
    }
}

impl Upstream for FakeUpstream {
    fn name(&self) -> &str {
        &self.name
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>> {
        Box::pin(async move {
            let delay = *self.list_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if self.down.load(Ordering::SeqCst) {
                return Err(self.unavailable());
            }
            Ok(self.tools.lock().unwrap().clone())
        })
    }

    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>> {
        Box::pin(async move {
            let delay = *self.delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if self.down.load(Ordering::SeqCst) {
                return Err(self.unavailable());
            }
            self.calls.lock().unwrap().push((tool.to_string(), args.clone()));
            if let Some(r) = self.replies.lock().unwrap().get(tool) {
                return r.clone();
            }
            if self.tools.lock().unwrap().iter().any(|t| t.name == tool) {
                Ok(json!({"server": self.name, "tool": tool, "args": args}))
            } else {
                Err(UpstreamError::Tool(format!("{}.{tool}: unknown tool", self.name)))
            }
        })
    }

    fn status(&self) -> UpstreamStatus {
        let up = !self.down.load(Ordering::SeqCst);
        UpstreamStatus {
            state: if up { UpstreamState::Ready } else { UpstreamState::Backoff },
            tools: self.tools.lock().unwrap().len(),
            last_error: self.exited.load(Ordering::SeqCst).then(|| format!("{}: process exited", self.name)),
            server: Some(self.server.lock().unwrap().clone()),
            ..UpstreamStatus::new(&self.name, Transport::Stdio)
        }
    }

    fn tools_changed(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }

    fn is_alive(&self) -> bool {
        !self.exited.load(Ordering::SeqCst)
    }

    fn redact(&self, text: String) -> String {
        redact_secrets(text, &self.secrets.lock().unwrap())
    }

    fn shutdown(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
        })
    }
}

#[derive(Default)]
pub struct FakeConnector {
    upstreams: Mutex<BTreeMap<String, Arc<FakeUpstream>>>,
    refuse: Mutex<BTreeMap<String, String>>,
    connects: Mutex<BTreeMap<String, usize>>,
    specs: Mutex<BTreeMap<String, ResolvedUpstream>>,
}

impl FakeConnector {
    pub fn new() -> Arc<FakeConnector> {
        Arc::new(FakeConnector::default())
    }

    pub fn add(&self, up: Arc<FakeUpstream>) {
        self.upstreams.lock().unwrap().insert(up.name.clone(), up);
    }

    pub fn refuse(&self, name: &str, message: Option<&str>) {
        let mut r = self.refuse.lock().unwrap();
        match message {
            Some(m) => r.insert(name.into(), m.into()),
            None => r.remove(name),
        };
    }

    pub fn connects(&self, name: &str) -> usize {
        self.connects.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    pub fn last_spec(&self, name: &str) -> Option<ResolvedUpstream> {
        self.specs.lock().unwrap().get(name).cloned()
    }
}

impl Connector for FakeConnector {
    fn connect<'a>(&'a self, spec: &'a ResolvedUpstream) -> BoxFut<'a, Result<Arc<dyn Upstream>, UpstreamError>> {
        Box::pin(async move {
            *self.connects.lock().unwrap().entry(spec.name.clone()).or_default() += 1;
            self.specs.lock().unwrap().insert(spec.name.clone(), spec.clone());
            if let Some(m) = self.refuse.lock().unwrap().get(&spec.name) {
                return Err(UpstreamError::Unavailable(spec.redact(format!("{}: {m}", spec.name))));
            }
            let up = self
                .upstreams
                .lock()
                .unwrap()
                .get(&spec.name)
                .cloned()
                .ok_or_else(|| UpstreamError::Unavailable(format!("{}: no fake registered", spec.name)))?;
            *up.secrets.lock().unwrap() = spec.secrets.clone();
            up.set_down(false);
            up.set_exited(false);
            let up: Arc<dyn Upstream> = up;
            Ok(up)
        })
    }
}
