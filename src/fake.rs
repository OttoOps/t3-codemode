use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::t3::{BoxFut, T3Client, T3Error};

#[derive(Default)]
struct State {
    threads: BTreeMap<String, Value>,
    items: BTreeMap<String, Vec<Value>>,
    projects: Vec<Value>,
    launched: Vec<Value>,
    organized: Vec<Value>,
    sent: Vec<Value>,
    calls: Vec<String>,
    send_fails: bool,
    organize_fails: bool,
    delay: Duration,
}

#[derive(Default)]
pub struct FakeT3 {
    state: Mutex<State>,
    tools: Vec<Value>,
}

#[derive(Default)]
pub struct FakeRemotes(Mutex<BTreeMap<String, String>>);

impl FakeRemotes {
    pub fn set(&self, root: &str, url: &str) {
        self.0.lock().unwrap().insert(root.into(), url.into());
    }
}

impl crate::board::Remotes for FakeRemotes {
    fn origin<'a>(&'a self, root: &'a str) -> BoxFut<'a, Option<String>> {
        Box::pin(async move { self.0.lock().unwrap().get(root).cloned() })
    }
}

#[derive(Default)]
pub struct FakeDesktop {
    pub running: std::sync::atomic::AtomicBool,
    pub opened: Mutex<Vec<String>>,
}

impl crate::desktop::Desktop for FakeDesktop {
    fn open<'a>(&'a self, app_link: &'a str) -> BoxFut<'a, bool> {
        Box::pin(async move {
            let running = self.running.load(std::sync::atomic::Ordering::SeqCst);
            if running {
                self.opened.lock().unwrap().push(app_link.to_string());
            }
            running
        })
    }
}

pub fn project(id: &str, root: &str) -> Value {
    json!({"id": id, "title": id, "workspaceRoot": root, "defaultModelSelection": null, "scripts": [],
           "createdAt": "2026-11-01T00:00:00Z", "updatedAt": "2026-11-01T00:00:00Z", "deletedAt": null})
}

pub fn thread(id: &str, status: &str, updated: &str) -> Value {
    json!({
        "threadId": id, "status": status, "updatedAt": updated, "link": format!("https://t3/{id}"),
        "pendingBackgroundTasks": [], "blockedOnThreadWait": false, "pendingRequestCount": 0,
        "latestRunId": format!("run:{id}:1"), "archived": false, "title": id
    })
}

impl FakeT3 {
    pub fn new() -> FakeT3 {
        FakeT3::default()
    }

    pub fn with_tools(tools: Vec<Value>) -> FakeT3 {
        FakeT3 { tools, ..FakeT3::default() }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub fn put(&self, thread: Value) {
        let id = thread["threadId"].as_str().expect("threadId").to_string();
        self.st().threads.insert(id, thread);
    }

    pub fn update(&self, id: &str, patch: Value) {
        let mut st = self.st();
        let th = st.threads.get_mut(id).expect("known thread");
        for (k, v) in patch.as_object().expect("object patch") {
            th[k] = v.clone();
        }
    }

    pub fn remove(&self, id: &str) {
        self.st().threads.remove(id);
    }

    pub fn set_send_fails(&self, fails: bool) {
        self.st().send_fails = fails;
    }

    pub fn set_organize_fails(&self, fails: bool) {
        self.st().organize_fails = fails;
    }

    pub fn set_delay(&self, delay: Duration) {
        self.st().delay = delay;
    }

    pub fn set_items(&self, id: &str, items: Vec<Value>) {
        self.st().items.insert(id.into(), items);
    }

    pub fn set_projects(&self, projects: Vec<Value>) {
        self.st().projects = projects;
    }

    pub fn launched(&self) -> Vec<Value> {
        self.st().launched.clone()
    }

    pub fn organized(&self) -> Vec<Value> {
        self.st().organized.clone()
    }

    pub fn sent(&self) -> Vec<Value> {
        self.st().sent.clone()
    }

    pub fn calls(&self) -> Vec<String> {
        self.st().calls.clone()
    }

    fn handle(&self, tool: &str, args: &Value) -> Result<Value, T3Error> {
        let mut st = self.st();
        st.calls.push(tool.to_string());
        match tool {
            "t3_thread_read" => {
                let id = args["threadId"].as_str().unwrap_or_default();
                let items = st.items.get(id).cloned().unwrap_or_default();
                match st.threads.get(id) {
                    Some(th) => Ok(json!({"thread": th, "items": items, "earlierPosition": null})),
                    None => Err(T3Error::Tool(format!("t3_thread_read: thread {id} not found"))),
                }
            }
            "t3_thread_send" => {
                if st.send_fails {
                    return Err(T3Error::Unavailable("T3 unreachable (fake)".into()));
                }
                st.sent.push(args.clone());
                let n = st.sent.len();
                Ok(json!({"threadId": args["threadId"], "messageId": format!("msg:{n}"),
                          "runId": format!("run:{n}"), "status": "queued", "delivery": "queued"}))
            }
            "t3_thread_list" => {
                let wanted: Option<Vec<&str>> =
                    args["statuses"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect());
                let limit = args["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let threads: Vec<Value> = st
                    .threads
                    .values()
                    .filter(|t| wanted.as_ref().is_none_or(|w| w.contains(&t["status"].as_str().unwrap_or(""))))
                    .take(limit)
                    .cloned()
                    .collect();
                Ok(json!({"threads": threads}))
            }
            "t3_project_list" => Ok(json!({"projects": st.projects, "nextCursor": null})),
            "t3_thread_launch" => {
                st.launched.push(args.clone());
                let id = format!("launch:{}", st.launched.len());
                let mut th = thread(&id, "running", "2026-11-17T12:00:00Z");
                th["title"] = args["title"].clone();
                st.threads.insert(id.clone(), th);
                Ok(json!({"threadId": id, "link": format!("https://t3/{id}"), "projectId": args["projectId"],
                          "modelSelection": args["modelSelection"], "runId": format!("run:{id}:1"), "status": "running"}))
            }
            "t3_thread_organize" => {
                if st.organize_fails {
                    return Err(T3Error::Unavailable("T3 unreachable (fake)".into()));
                }
                st.organized.push(args.clone());
                let id = args["threadId"].as_str().unwrap_or_default().to_string();
                if args["action"] == "archive"
                    && let Some(th) = st.threads.get_mut(&id)
                {
                    th["archived"] = json!(true);
                }
                Ok(Value::Null)
            }
            "echo" => Ok(args.clone()),
            "fail" => {
                Err(T3Error::Tool(format!("fail: {}", args["message"].as_str().unwrap_or("the fake tool failed"))))
            }
            other => Err(T3Error::Tool(format!("fake T3 has no tool {other}"))),
        }
    }
}

impl T3Client for FakeT3 {
    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, T3Error>> {
        Box::pin(async move {
            let delay = self.st().delay;
            let delay = match tool {
                "sleep" => Duration::from_millis(args["ms"].as_u64().unwrap_or(0)),
                _ => delay,
            };
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if tool == "sleep" {
                self.st().calls.push("sleep".into());
                return Ok(args);
            }
            self.handle(tool, &args)
        })
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<Value>, T3Error>> {
        Box::pin(async move { Ok(self.tools.clone()) })
    }
}
