use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use serde_json::{Value, json};

use crate::clock::{Clock, iso, plus_minutes};

use crate::t3::{BoxFut, T3Client, T3Error};
use crate::tokens::{RunTokens, TokenSource};
use crate::usage::{Usage, UsageSource};

mod fault;

pub use fault::{Failure, Fault, Gate, Match, Nth, Rule, Target};
use fault::{Faults, Hit};

#[derive(Default)]
struct State {
    threads: BTreeMap<String, Value>,
    items: BTreeMap<String, Vec<Value>>,
    runs: BTreeMap<String, Vec<Value>>,
    prs: BTreeMap<String, Vec<Value>>,
    prs_before_list: BTreeMap<String, Vec<Value>>,
    projects: Vec<Value>,
    providers: Vec<Value>,
    launched: Vec<Value>,
    organized: Vec<Value>,
    interrupted: Vec<Value>,
    configured: Vec<Value>,
    forks: u64,
    sent: Vec<Value>,
    calls: Vec<String>,
    faults: Faults,
    pr_page: usize,
    short_list: usize,
    delay: Duration,
    clock: Option<Arc<dyn Clock>>,
    run_on_send: BTreeMap<String, i64>,
    run_ends: BTreeMap<String, (Timestamp, String)>,
    run_seq: BTreeMap<String, u64>,
}

impl State {
    fn now(&self) -> Option<Timestamp> {
        self.clock.as_ref().map(|c| c.now())
    }

    fn settle(&mut self) {
        let Some(now) = self.now() else { return };
        let ended: Vec<(String, Timestamp, String)> = self
            .run_ends
            .iter()
            .filter(|(_, (end, _))| *end <= now)
            .map(|(id, (end, status))| (id.clone(), *end, status.clone()))
            .collect();
        for (id, end, status) in ended {
            self.run_ends.remove(&id);
            if let Some(th) = self.threads.get_mut(&id) {
                th["status"] = json!(status);
                th["updatedAt"] = json!(iso(end));
            }
        }
    }

    fn touch(&mut self, id: &str) {
        if let Some(now) = self.now()
            && let Some(th) = self.threads.get_mut(id)
        {
            th["updatedAt"] = json!(iso(now));
        }
    }

    fn begin_run(&mut self, id: &str, minutes: Option<i64>) {
        let now = self.now();
        let Some(th) = self.threads.get_mut(id) else { return };
        let running = crate::wf::ACTIVE.contains(&th["status"].as_str().unwrap_or(""));
        if !running {
            let seq = self.run_seq.entry(id.to_string()).or_insert(1);
            *seq += 1;
            th["status"] = json!("running");
            th["latestRunId"] = json!(format!("run:{id}:{seq}"));
        }
        if let Some(now) = now {
            th["updatedAt"] = json!(iso(now));
        }
        match (minutes, now) {
            (Some(m), Some(now)) => {
                let from = self.run_ends.get(id).filter(|_| running).map_or(now, |(end, _)| (*end).max(now));
                self.run_ends.insert(id.to_string(), (plus_minutes(from, m as f64), "idle".into()));
            }
            _ => {
                self.run_ends.remove(id);
            }
        }
    }
}

#[derive(Default)]
pub struct FakeT3 {
    state: Mutex<State>,
    tools: Vec<Value>,
}

#[derive(Default)]
pub struct FakeRemotes(Mutex<BTreeMap<String, crate::board::Origin>>);

impl FakeRemotes {
    pub fn set(&self, root: &str, url: &str) {
        self.0.lock().unwrap().insert(root.into(), crate::board::Origin::Url(url.into()));
    }

    pub fn set_unavailable(&self, root: &str) {
        self.0.lock().unwrap().insert(root.into(), crate::board::Origin::Unavailable);
    }
}

impl crate::board::Remotes for FakeRemotes {
    fn origin<'a>(&'a self, root: &'a str) -> BoxFut<'a, Option<String>> {
        Box::pin(async move {
            match self.0.lock().unwrap().get(root).cloned() {
                Some(crate::board::Origin::Url(url)) => Some(url),
                _ => None,
            }
        })
    }

    fn origin_lookup<'a>(&'a self, root: &'a str) -> BoxFut<'a, crate::board::Origin> {
        Box::pin(async move { self.0.lock().unwrap().get(root).cloned().unwrap_or(crate::board::Origin::Absent) })
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

pub fn pull_request(repository: &str, number: u64, state: &str) -> Value {
    json!({
        "host": "github.com", "repository": repository, "number": number,
        "url": format!("https://github.com/{repository}/pull/{number}"), "source": "agent",
        "watching": state == "open", "state": state, "title": format!("PR {number}"),
        "headBranch": format!("fix/{number}"), "baseBranch": "main", "isDraft": false, "stack": null
    })
}

pub fn merge_request(host: &str, path: &str, iid: u64, state: &str) -> Value {
    json!({
        "host": host, "repository": path, "number": iid,
        "url": format!("https://{host}/{path}/-/merge_requests/{iid}"), "source": "agent",
        "watching": state == "open", "state": state, "title": format!("MR {iid}"),
        "headBranch": format!("fix/{iid}"), "baseBranch": "main", "isDraft": false, "stack": null
    })
}

pub fn provider(id: &str, driver: &str, models: &[&str]) -> Value {
    let models: Vec<Value> = models
        .iter()
        .map(|m| json!({"id": m, "options": [{"id": "effort", "values": ["low", "medium", "high"], "default": "medium"}]}))
        .collect();
    json!({"providerInstanceId": id, "driverKind": driver, "displayName": id, "models": models,
           "canRunChildTask": true, "canRunCrossProviderChildTask": true, "constraints": []})
}

pub fn disabled_provider(id: &str, driver: &str) -> Value {
    json!({"providerInstanceId": id, "driverKind": driver, "displayName": id, "models": [],
           "canRunChildTask": false, "canRunCrossProviderChildTask": false,
           "constraints": ["Provider instance is disabled."]})
}

pub fn usage_limit_item(run: &str, text: &str) -> Value {
    json!({"position": 9, "itemId": format!("error:{run}"), "runId": run, "type": "error", "status": "failed",
           "title": "Usage limit reached", "text": text, "createdBy": null, "creationSource": null})
}

#[derive(Default)]
pub struct FakeUsage {
    usage: Mutex<BTreeMap<String, Usage>>,
    fails: Mutex<bool>,
}

impl FakeUsage {
    pub fn set(&self, instance: &str, left: f64, resets_at: Option<&str>) {
        let u = Usage { left, resets_at: resets_at.map(String::from) };
        self.usage.lock().unwrap().insert(instance.into(), u);
    }

    pub fn set_fails(&self, fails: bool) {
        *self.fails.lock().unwrap() = fails;
    }
}

impl UsageSource for FakeUsage {
    fn usage<'a>(&'a self, instances: &'a [String]) -> BoxFut<'a, anyhow::Result<BTreeMap<String, Usage>>> {
        Box::pin(async move {
            if *self.fails.lock().unwrap() {
                anyhow::bail!("usage source unreachable (fake)");
            }
            let all = self.usage.lock().unwrap();
            Ok(instances.iter().filter_map(|i| all.get(i).map(|u| (i.clone(), u.clone()))).collect())
        })
    }
}

#[derive(Default)]
pub struct FakeTokens {
    runs: Mutex<BTreeMap<String, RunTokens>>,
    reads: Mutex<Vec<String>>,
    fails: Mutex<bool>,
    failing: Mutex<Vec<String>>,
}

impl FakeTokens {
    pub fn set(&self, run: &str, input: u64, output: u64) {
        self.runs.lock().unwrap().insert(run.into(), RunTokens { input, output });
    }

    pub fn set_fails(&self, fails: bool) {
        *self.fails.lock().unwrap() = fails;
    }

    pub fn set_run_fails(&self, run: &str, fails: bool) {
        let mut failing = self.failing.lock().unwrap();
        failing.retain(|r| r != run);
        if fails {
            failing.push(run.into());
        }
    }

    pub fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

impl TokenSource for FakeTokens {
    fn reports(&self) -> bool {
        true
    }

    fn tokens<'a>(&'a self, _thread: &'a str, run: &'a Value) -> BoxFut<'a, anyhow::Result<Option<RunTokens>>> {
        Box::pin(async move {
            let run = run["runId"].as_str().unwrap_or_default();
            self.reads.lock().unwrap().push(run.to_string());
            if *self.fails.lock().unwrap() || self.failing.lock().unwrap().iter().any(|r| r == run) {
                anyhow::bail!("token source unreachable (fake)");
            }
            Ok(self.runs.lock().unwrap().get(run).copied())
        })
    }
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
        let mut st = self.st();
        st.threads.remove(id);
        st.run_ends.remove(id);
    }

    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        self.st().clock = Some(clock);
    }

    pub fn run_on_send(&self, id: &str, minutes: i64) {
        self.st().run_on_send.insert(id.into(), minutes);
    }

    pub fn start_run(&self, id: &str, minutes: Option<i64>) {
        let mut st = self.st();
        st.settle();
        st.begin_run(id, minutes);
    }

    pub fn finish_run(&self, id: &str, status: &str) {
        let mut st = self.st();
        st.settle();
        st.run_ends.remove(id);
        if let Some(th) = st.threads.get_mut(id) {
            th["status"] = json!(status);
        }
        st.touch(id);
    }

    pub fn archive(&self, id: &str) {
        let mut st = self.st();
        st.settle();
        st.run_ends.remove(id);
        if let Some(th) = st.threads.get_mut(id) {
            th["archived"] = json!(true);
        }
        st.touch(id);
    }

    pub fn set_pr_page(&self, size: usize) {
        self.st().pr_page = size;
    }

    pub fn set_delay(&self, delay: Duration) {
        self.st().delay = delay;
    }

    pub fn fault(&self, rule: Rule) {
        self.st().faults.add(rule);
    }

    pub fn fail_tool(&self, tool: &str) {
        self.fault(Rule::on(tool).fail(Failure::down(T3_DOWN)));
    }

    pub fn clear_faults_on(&self, tool: &str) {
        self.st().faults.clear(Some(tool));
    }

    pub fn clear_faults(&self) {
        self.st().faults.clear(None);
    }

    pub fn set_jitter(&self, seed: Option<u64>) {
        self.st().faults.set_jitter(seed);
    }

    pub fn set_items(&self, id: &str, items: Vec<Value>) {
        self.st().items.insert(id.into(), items);
    }

    pub fn set_runs(&self, id: &str, runs: Vec<Value>) {
        self.st().runs.insert(id.into(), runs);
    }

    pub fn set_prs(&self, id: &str, prs: Vec<Value>) {
        self.st().prs.insert(id.into(), prs);
    }

    pub fn set_prs_before_list(&self, id: &str, prs: Vec<Value>) {
        self.st().prs_before_list.insert(id.into(), prs);
    }

    pub fn set_projects(&self, projects: Vec<Value>) {
        self.st().projects = projects;
    }

    pub fn set_providers(&self, providers: Vec<Value>) {
        self.st().providers = providers;
    }

    pub fn set_short_list(&self, models: usize) {
        self.st().short_list = models;
    }

    pub fn launched(&self) -> Vec<Value> {
        self.st().launched.clone()
    }

    pub fn organized(&self) -> Vec<Value> {
        self.st().organized.clone()
    }

    pub fn interrupted(&self) -> Vec<Value> {
        self.st().interrupted.clone()
    }

    pub fn sent(&self) -> Vec<Value> {
        self.st().sent.clone()
    }

    pub fn configured(&self) -> Vec<Value> {
        self.st().configured.clone()
    }

    pub fn thread(&self, id: &str) -> Option<Value> {
        let mut st = self.st();
        st.settle();
        st.threads.get(id).cloned()
    }

    pub fn calls(&self) -> Vec<String> {
        self.st().calls.clone()
    }

    fn handle(&self, tool: &str, args: &Value) -> Result<Value, T3Error> {
        let mut st = self.st();
        st.settle();
        match tool {
            "t3_thread_read" => {
                let id = args["threadId"].as_str().unwrap_or_default();
                let items = st.items.get(id).cloned().unwrap_or_default();
                let runs = st.runs.get(id).cloned().unwrap_or_default();
                match st.threads.get(id) {
                    Some(th) => {
                        let mut th = th.clone();
                        if let Some(prs) = st.prs.get(id) {
                            let open: Vec<&Value> = prs.iter().filter(|p| p["state"] == "open").collect();
                            th["openPullRequestCount"] = json!(open.len());
                            th["pullRequestCount"] = json!(prs.len());
                            th["pullRequests"] = json!(open);
                        }
                        Ok(json!({"thread": th, "recentRuns": runs, "items": items, "earlierPosition": null}))
                    }
                    None => Err(T3Error::Tool(format!("t3_thread_read: thread {id} not found"))),
                }
            }
            "t3_thread_send" => {
                st.sent.push(args.clone());
                let id = args["threadId"].as_str().unwrap_or_default();
                if let Some(&minutes) = st.run_on_send.get(id) {
                    st.begin_run(id, Some(minutes));
                } else if let Some(th) = st.threads.get_mut(id)
                    && th["status"] == "idle"
                    && th["latestRunId"].is_null()
                {
                    th["status"] = json!("running");
                    th["latestRunId"] = json!(format!("run:{id}:1"));
                }
                let n = st.sent.len();
                Ok(json!({"threadId": args["threadId"], "messageId": format!("msg:{n}"),
                          "runId": format!("run:{n}"), "status": "queued", "delivery": "queued"}))
            }
            "t3_thread_list" => {
                let wanted: Option<Vec<&str>> =
                    args["statuses"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect());
                let limit = args["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let start = args["cursor"].as_u64().unwrap_or(0) as usize;
                let matching: Vec<&Value> = st
                    .threads
                    .values()
                    .filter(|t| wanted.as_ref().is_none_or(|w| w.contains(&t["status"].as_str().unwrap_or(""))))
                    .collect();
                let threads: Vec<Value> = matching.iter().skip(start).take(limit).map(|t| (*t).clone()).collect();
                let next = (start.saturating_add(limit) < matching.len()).then(|| start + limit);
                Ok(json!({"threads": threads, "nextCursor": next, "total": matching.len()}))
            }
            "list_thread_pull_requests" => {
                let id = args["threadId"].as_str().unwrap_or_default();
                if let Some(prs) = st.prs_before_list.remove(id) {
                    st.prs.insert(id.into(), prs);
                }
                let wanted: Option<Vec<&str>> =
                    args["states"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect());
                let prs: Vec<Value> = st
                    .prs
                    .get(id)
                    .into_iter()
                    .flatten()
                    .filter(|p| wanted.as_ref().is_none_or(|w| w.contains(&p["state"].as_str().unwrap_or(""))))
                    .cloned()
                    .collect();
                let size = if st.pr_page == 0 { prs.len().max(1) } else { st.pr_page };
                let start: usize = args["cursor"].as_str().and_then(|c| c.parse().ok()).unwrap_or(0);
                let end = (start + size).min(prs.len());
                let next = (end < prs.len()).then(|| end.to_string());
                Ok(json!({"pullRequests": prs[start.min(end)..end], "chains": [], "nextCursor": next}))
            }
            "t3_project_list" => Ok(json!({"projects": st.projects, "nextCursor": null})),
            "orchestrator_capabilities" => {
                let search = args["search"].as_str();
                let providers: Vec<Value> = st
                    .providers
                    .iter()
                    .map(|p| {
                        let mut p = p.clone();
                        if search.is_none() && st.short_list > 0 {
                            let models: Vec<Value> =
                                p["models"].as_array().into_iter().flatten().take(st.short_list).cloned().collect();
                            p["models"] = json!(models);
                        }
                        if let Some(q) = search {
                            let models: Vec<Value> = p["models"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|m| m["id"].as_str().is_some_and(|id| id.contains(q)))
                                .cloned()
                                .collect();
                            p["models"] = json!(models);
                        }
                        p
                    })
                    .collect();
                Ok(json!({"parentThreadId": null, "runtimeMode": "full-access", "providers": providers}))
            }
            "t3_thread_launch" => {
                st.launched.push(args.clone());
                let id = format!("launch:{}", st.launched.len());
                let has_msg = args.get("message").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
                let init_status = if has_msg { "running" } else { "idle" };
                let mut th = thread(&id, init_status, "2026-11-17T12:00:00Z");
                if !has_msg {
                    th["latestRunId"] = Value::Null;
                }
                th["title"] = args["title"].clone();
                if let Some(p) = args.get("projectId").filter(|v| !v.is_null()) {
                    th["projectId"] = p.clone();
                }
                th["providerInstanceId"] = args["modelSelection"]["instanceId"].clone();
                th["model"] = args["modelSelection"]["model"].clone();
                st.threads.insert(id.clone(), th);
                let run_id = if has_msg { json!(format!("run:{id}:1")) } else { Value::Null };
                Ok(json!({"threadId": id, "link": format!("https://t3/{id}"), "projectId": args["projectId"],
                          "modelSelection": args["modelSelection"], "runId": run_id, "status": init_status}))
            }
            "t3_thread_transfer" if args["action"] == "fork" => {
                let id = args["threadId"].as_str().unwrap_or_default();
                let Some(src) = st.threads.get(id).cloned() else {
                    return Err(T3Error::Tool(format!("t3_thread_transfer: thread {id} not found")));
                };
                st.forks += 1;
                let new = format!("fork:{}", st.forks);
                let mut th = thread(&new, "idle", "2026-11-17T12:00:00Z");
                for k in ["providerInstanceId", "model", "projectId", "branch", "worktreePath"] {
                    th[k] = src[k].clone();
                }
                th["title"] = args.get("title").cloned().unwrap_or_else(|| src["title"].clone());
                th["latestRunId"] = Value::Null;
                th["forkedFrom"] = json!(id);
                st.threads.insert(new.clone(), th);
                Ok(json!({"sequence": st.forks, "targetThreadId": new}))
            }
            "t3_thread_configure" => {
                let id = args["threadId"].as_str().unwrap_or_default().to_string();
                st.configured.push(args.clone());
                let Some(th) = st.threads.get_mut(&id) else {
                    return Err(T3Error::Tool(format!("t3_thread_configure: thread {id} not found")));
                };
                let m = &args["modelSelection"];
                if m.is_object() {
                    th["providerInstanceId"] = m["instanceId"].clone();
                    th["model"] = m["model"].clone();
                }
                Ok(json!({"sequence": 1}))
            }
            "t3_thread_interrupt" => {
                st.interrupted.push(args.clone());
                let id = args["threadId"].as_str().unwrap_or_default().to_string();
                let Some(th) = st.threads.get_mut(&id) else {
                    return Err(T3Error::Tool(format!("t3_thread_interrupt: thread {id} not found")));
                };
                th["status"] = json!("interrupted");
                Ok(json!({"threadId": id, "interrupted": true}))
            }
            "t3_thread_organize" => {
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

const T3_DOWN: &str = "T3 unreachable (fake)";

fn t3_error(f: &Failure) -> T3Error {
    match f {
        Failure::Down(m) => T3Error::Unavailable(m.clone()),
        _ => T3Error::Tool(f.message().to_string()),
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
            tokio::task::yield_now().await;
            let (yields, hit) = {
                let mut st = self.st();
                st.calls.push(tool.to_string());
                let target: Vec<String> =
                    args["threadId"].as_str().or(args["search"].as_str()).map(String::from).into_iter().collect();
                (st.faults.yields(), st.faults.hit(&[tool], &target))
            };
            fault::jitter(yields).await;
            hit.pass_gates().await;
            if let Some(f) = hit.before() {
                return Err(t3_error(f));
            }
            let out = if tool == "sleep" { Ok(args) } else { self.handle(tool, &args) };
            match hit.after() {
                Some(f) => Err(t3_error(f)),
                None => out,
            }
        })
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<Value>, T3Error>> {
        Box::pin(async move { Ok(self.tools.clone()) })
    }
}

#[derive(Default)]
struct GhState {
    delay: Duration,
    repos: BTreeMap<String, Value>,
    repo_names: BTreeMap<String, String>,
    repo_aliases: BTreeMap<String, String>,
    default_branches: BTreeMap<String, String>,
    repo_labels: BTreeMap<String, Vec<(String, String, String)>>,
    prs: BTreeMap<(String, u64), Value>,
    issues: BTreeMap<(String, u64), Value>,
    issue_page: usize,
    pr_page: usize,
    pr_file_page: usize,
    calls: Vec<Value>,
    faults: Faults,
    queue: u64,
    merges: u64,
    label_seq: u64,
    unknown_reads: usize,
}

#[derive(Default)]
pub struct FakeGitHub(Mutex<GhState>);

pub fn github_pr(head: &str, merge_state: &str, queue: bool) -> Value {
    json!({
        "state": "OPEN", "isDraft": false, "headRefOid": head, "mergeStateStatus": merge_state,
        "reviewDecision": null, "isMergeQueueEnabled": queue, "isInMergeQueue": false, "mergeQueueEntry": null, "mergeCommit": null,
        "mergedAt": null, "closedAt": null, "timelineItems": {"nodes": []}, "commits": head_commit(head, json!([]))
    })
}

pub fn head_commit(head: &str, checks: Value) -> Value {
    json!({"nodes": [{"commit": {"oid": head, "statusCheckRollup": {
        "contexts": {"pageInfo": {"hasNextPage": false}, "nodes": checks}
    }}}]})
}

pub fn check_run(name: &str, conclusion: &str) -> Value {
    json!({"__typename": "CheckRun", "name": name, "status": "COMPLETED", "conclusion": conclusion})
}

pub fn github_issue(number: u64, title: &str, labels: &[&str], created_at: &str) -> Value {
    let labels: Vec<Value> = labels.iter().map(|l| json!({"name": l})).collect();
    json!({"number": number, "title": title, "body": "", "state": "OPEN", "createdAt": created_at,
           "labels": {"nodes": labels}, "milestone": null, "blockers": [], "parentRef": null})
}

fn issue_key(repo: &str, number: u64) -> (String, u64) {
    (repo.to_ascii_lowercase(), number)
}

fn gh_error(message: &str) -> Value {
    gh_typed_error("UNPROCESSABLE", message)
}

fn gh_typed_error(err_type: &str, message: &str) -> Value {
    json!({"data": null, "errors": [{"type": err_type, "message": message}]})
}

fn repo_node_id(repo_lower: &str) -> String {
    format!("R_{repo_lower}")
}

fn issue_node_id(repo_lower: &str, number: u64) -> String {
    format!("I_{repo_lower}#{number}")
}

fn ensure_repo_label_entry(st: &mut GhState, repo_lower: &str, name: &str, color: &str) -> (String, String) {
    if let Some(existing) =
        st.repo_labels.get(repo_lower).and_then(|list| list.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)))
    {
        return (existing.0.clone(), existing.1.clone());
    }
    st.label_seq += 1;
    let id = format!("LA_{repo_lower}_{}", st.label_seq);
    st.repo_labels.entry(repo_lower.to_string()).or_default().push((id.clone(), name.to_string(), color.to_string()));
    (id, name.to_string())
}

impl FakeGitHub {
    fn st(&self) -> std::sync::MutexGuard<'_, GhState> {
        self.0.lock().unwrap()
    }

    pub fn set_pr(&self, repo: &str, number: u64, pr: Value) {
        let mut st = self.st();
        let mut pr = pr;
        let key = repo.to_ascii_lowercase();
        pr["id"] = json!(format!("PR_{repo}_{number}"));
        pr["number"] = json!(number);
        if pr.get("baseRefName").is_none() {
            pr["baseRefName"] = json!("main");
        }
        if pr.get("headRefName").is_none() {
            pr["headRefName"] = json!(format!("feat/{number}"));
            pr["headRef"] = json!({"id": format!("REF_{repo}_{number}")});
            pr["isCrossRepository"] = json!(false);
        }
        st.repos.entry(key.clone()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.repo_names.entry(key.clone()).or_insert_with(|| repo.to_string());
        st.prs.insert((key, number), pr);
    }

    pub fn update_pr(&self, repo: &str, number: u64, patch: Value) {
        let mut st = self.st();
        let pr = st.prs.get_mut(&(repo.to_ascii_lowercase(), number)).expect("known pull request");
        for (k, v) in patch.as_object().expect("object patch") {
            pr[k] = v.clone();
        }
    }

    pub fn set_checks(&self, repo: &str, number: u64, checks: Value) {
        let head = self.pr(repo, number)["headRefOid"].as_str().unwrap_or_default().to_string();
        self.update_pr(repo, number, json!({"commits": head_commit(&head, checks)}));
    }

    pub fn pr(&self, repo: &str, number: u64) -> Value {
        self.st().prs[&(repo.to_ascii_lowercase(), number)].clone()
    }

    pub fn remove_pr(&self, repo: &str, number: u64) {
        self.st().prs.remove(&(repo.to_ascii_lowercase(), number));
    }

    pub fn set_default_branch(&self, repo: &str, branch: &str) {
        let mut st = self.st();
        let key = repo.to_ascii_lowercase();
        st.repos.entry(key.clone()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.repo_names.entry(key.clone()).or_insert_with(|| repo.to_string());
        st.default_branches.insert(key, branch.to_string());
    }

    pub fn set_pr_page(&self, size: usize) {
        self.st().pr_page = size;
    }

    pub fn set_pr_file_page(&self, size: usize) {
        self.st().pr_file_page = size;
    }

    pub fn drop_from_queue(&self, repo: &str, number: u64, reason: &str, at: &str) {
        self.update_pr(
            repo,
            number,
            json!({"isInMergeQueue": false, "mergeQueueEntry": null,
                   "timelineItems": {"nodes": [{"reason": reason, "createdAt": at}]}}),
        );
    }

    pub fn set_default_method(&self, repo: &str, method: &str) {
        self.st().repos.insert(repo.into(), json!({"viewerDefaultMergeMethod": method}));
    }

    pub fn alias_repo(&self, from: &str, to: &str) {
        self.st().repo_aliases.insert(from.to_ascii_lowercase(), to.to_string());
    }

    pub fn unalias_repo(&self, from: &str) {
        self.st().repo_aliases.remove(&from.to_ascii_lowercase());
    }

    pub fn set_delay(&self, delay: Duration) {
        self.st().delay = delay;
    }

    pub fn set_queue_length(&self, n: u64) {
        self.st().queue = n;
    }

    pub fn fault(&self, rule: Rule) {
        self.st().faults.add(rule);
    }

    pub fn unreachable(&self) {
        self.st().faults.add_first(Rule::on("*").fail(Failure::down(GH_DOWN)));
    }

    pub fn clear_faults_on(&self, operation: &str) {
        self.st().faults.clear(Some(operation));
    }

    pub fn clear_faults(&self) {
        self.st().faults.clear(None);
    }

    pub fn set_jitter(&self, seed: Option<u64>) {
        self.st().faults.set_jitter(seed);
    }

    pub fn set_unknown_reads(&self, n: usize) {
        self.st().unknown_reads = n;
    }

    pub fn add_repo(&self, repo: &str) {
        let mut st = self.st();
        let key = repo.to_ascii_lowercase();
        st.repos.entry(key.clone()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.repo_names.entry(key).or_insert_with(|| repo.to_string());
    }

    pub fn add_label(&self, repo: &str, name: &str, color: &str) {
        let mut st = self.st();
        let key = repo.to_ascii_lowercase();
        st.repos.entry(key.clone()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.repo_names.entry(key.clone()).or_insert_with(|| repo.to_string());
        ensure_repo_label_entry(&mut st, &key, name, color);
    }

    pub fn repo_labels(&self, repo: &str) -> Vec<String> {
        self.st()
            .repo_labels
            .get(&repo.to_ascii_lowercase())
            .map(|list| list.iter().map(|(_, n, _)| n.clone()).collect())
            .unwrap_or_default()
    }

    pub fn set_issue(&self, repo: &str, issue: Value) {
        let mut st = self.st();
        let mut issue = issue;
        let number = issue["number"].as_u64().expect("issue number");
        let key = repo.to_ascii_lowercase();
        issue["nameWithOwner"] = json!(repo);
        st.repos.entry(key.clone()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.repo_names.entry(key.clone()).or_insert_with(|| repo.to_string());
        let label_names: Vec<String> = issue["labels"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["name"].as_str().map(String::from))
            .collect();
        for name in label_names {
            ensure_repo_label_entry(&mut st, &key, &name, "ededed");
        }
        st.issues.insert(issue_key(repo, number), issue);
    }

    pub fn issue(&self, repo: &str, number: u64) -> Option<Value> {
        self.st().issues.get(&issue_key(repo, number)).cloned()
    }

    pub fn update_issue(&self, repo: &str, number: u64, patch: Value) {
        let mut st = self.st();
        let issue = st.issues.get_mut(&issue_key(repo, number)).expect("known issue");
        for (k, v) in patch.as_object().expect("object patch") {
            issue[k] = v.clone();
        }
    }

    pub fn set_blockers(&self, repo: &str, number: u64, blockers: &[(&str, u64)]) {
        let refs: Vec<Value> = blockers.iter().map(|(r, n)| json!([r, n])).collect();
        self.update_issue(repo, number, json!({"blockers": refs}));
    }

    pub fn set_parent(&self, repo: &str, number: u64, parent: (&str, u64)) {
        self.update_issue(repo, number, json!({"parentRef": [parent.0, parent.1]}));
    }

    pub fn set_milestone(&self, repo: &str, number: u64, title: &str, due_on: Option<&str>) {
        self.update_issue(repo, number, json!({"milestone": {"title": title, "dueOn": due_on}}));
    }

    pub fn set_body(&self, repo: &str, number: u64, body: &str) {
        self.update_issue(repo, number, json!({"body": body}));
    }

    pub fn close_issue(&self, repo: &str, number: u64) {
        self.update_issue(repo, number, json!({"state": "CLOSED"}));
    }

    pub fn set_issue_page(&self, size: usize) {
        self.st().issue_page = size;
    }

    pub fn calls(&self) -> Vec<Value> {
        self.st().calls.clone()
    }

    pub fn operations(&self) -> Vec<String> {
        self.calls().iter().map(|c| c["operation"].as_str().unwrap_or_default().to_string()).collect()
    }
}

const GH_DOWN: &str = "gh api graphql failed: gh: not logged in";

fn gh_failure(f: &Failure) -> anyhow::Result<Value> {
    match f {
        Failure::Down(m) => anyhow::bail!("{m}"),
        Failure::Refused(m) => Ok(gh_error(m)),
        Failure::Typed { kind, message } => Ok(gh_typed_error(kind, message)),
        Failure::Field { field, message } => {
            Ok(json!({"data": null, "errors": [{"path": [field], "message": message}]}))
        }
    }
}

fn gh_ops(op: &'static str, query: &str, variables: &Value) -> Vec<&'static str> {
    let mut ops = vec![op];
    if op == "flowIssues" && variables["label"].as_str().is_none() && !query.contains("states: CLOSED") {
        ops.push("flowIssues.unlabelledOpen");
    }
    ops
}

fn repo_names(st: &GhState, raw: &str) -> Vec<String> {
    let key = raw.to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&key).map_or_else(|| key.clone(), |c| c.to_ascii_lowercase());
    let mut out = vec![key, canonical.clone()];
    out.extend(
        st.repo_aliases.iter().filter(|(_, to)| to.eq_ignore_ascii_case(&canonical)).map(|(from, _)| from.clone()),
    );
    out.sort();
    out.dedup();
    out
}

fn pr_names(st: &GhState, repo: &str, number: u64) -> Vec<String> {
    repo_names(st, repo).into_iter().map(|r| format!("{r}#{number}")).collect()
}

fn gh_targets(st: &GhState, variables: &Value) -> Vec<String> {
    if let (Some(owner), Some(name)) = (variables["owner"].as_str(), variables["name"].as_str()) {
        let repo = format!("{owner}/{name}");
        return match variables["number"].as_u64() {
            Some(n) => pr_names(st, &repo, n),
            None => repo_names(st, &repo),
        };
    }
    let by = |key: &str, field: fn(&Value) -> &Value| {
        let id = variables[key].as_str()?;
        st.prs.iter().find(|(_, p)| field(p) == id).map(|((r, n), _)| pr_names(st, r, *n))
    };
    if let Some(names) = by("id", |p| &p["id"]).or_else(|| by("ref", |p| &p["headRef"]["id"])) {
        return names;
    }
    if let Some((repo, n)) =
        ["issueId", "labelableId"].iter().find_map(|k| variables[*k].as_str().and_then(parse_issue_node_id))
    {
        return pr_names(st, &repo, n);
    }
    variables["repoId"].as_str().and_then(|id| id.strip_prefix("R_")).map(|r| repo_names(st, r)).unwrap_or_default()
}

fn batch_targets(st: &GhState, variables: &Value) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for i in 0.. {
        let Some(owner) = variables[format!("o{i}")].as_str() else { break };
        let (Some(name), Some(n)) = (variables[format!("r{i}")].as_str(), variables[format!("n{i}")].as_u64()) else {
            break;
        };
        out.push((format!("p{i}"), pr_names(st, &format!("{owner}/{name}"), n)));
    }
    out
}

fn operation(query: &str) -> &'static str {
    if query.starts_with("query Reconcile(") {
        return "reconcile";
    }
    if query.starts_with("query RepoCanonical(") {
        return "repoCanonical";
    }
    if query.starts_with("query RepoId(") {
        return "repoId";
    }
    if query.starts_with("query IssueNode(") {
        return "issueNode";
    }
    if query.starts_with("query RepoLabel(") {
        return "repoLabel";
    }
    if query.starts_with("query FlowIssues(") {
        return "flowIssues";
    }
    if query.starts_with("query FlowOpenPrs(") {
        return "flowOpenPrs";
    }
    if query.starts_with("query FlowPrFiles(") {
        return "flowPrFiles";
    }
    if query.starts_with("query FlowBranchMerged(") {
        return "flowBranchMerged";
    }
    if query.starts_with(crate::flow::FLOW_TICKETS) {
        return "flowTickets";
    }
    for op in [
        "enqueuePullRequest",
        "mergePullRequest",
        "enablePullRequestAutoMerge",
        "deleteRef",
        "createLabel",
        "addLabelsToLabelable",
        "removeLabelsFromLabelable",
        "addBlockedBy",
        "removeBlockedBy",
        "addSubIssue",
        "removeSubIssue",
        "createIssue",
    ] {
        if query.contains(op) {
            return op;
        }
    }
    "read"
}

fn issue_node(st: &GhState, r: &Value) -> Value {
    let (Some(repo), Some(number)) = (r[0].as_str(), r[1].as_u64()) else { return Value::Null };
    let found = st.issues.get(&issue_key(repo, number));
    let repo = found.and_then(|i| i["nameWithOwner"].as_str()).unwrap_or(repo);
    json!({"number": number, "title": found.map_or(json!(format!("Issue {number}")), |i| i["title"].clone()),
           "url": format!("https://github.com/{repo}/issues/{number}"),
           "state": found.map_or(json!("OPEN"), |i| i["state"].clone()),
           "repository": {"nameWithOwner": repo}})
}

fn full_issue_node(st: &GhState, i: &Value) -> Value {
    let name = i["nameWithOwner"].as_str().unwrap_or_default();
    let number = i["number"].as_u64().unwrap_or_default();
    let blockers: Vec<Value> = i["blockers"].as_array().into_iter().flatten().map(|r| issue_node(st, r)).collect();
    let parent = issue_node(st, &i["parentRef"]);
    let mut subs: Vec<&Value> = st
        .issues
        .values()
        .filter(|c| {
            c["parentRef"][0].as_str().is_some_and(|r| r.eq_ignore_ascii_case(name))
                && c["parentRef"][1].as_u64() == Some(number)
        })
        .collect();
    subs.sort_by_key(|c| c["number"].as_u64());
    let subs: Vec<Value> =
        subs.into_iter().map(|c| issue_node(st, &json!([c["nameWithOwner"], c["number"]]))).collect();
    json!({"number": number, "title": i["title"], "body": i.get("body").cloned().unwrap_or_else(|| json!("")),
           "url": format!("https://github.com/{name}/issues/{number}"),
           "createdAt": i["createdAt"], "state": i["state"], "labels": i["labels"], "milestone": i["milestone"],
           "parent": parent, "blockedBy": {"totalCount": blockers.len(), "nodes": blockers},
           "subIssues": {"totalCount": subs.len(), "nodes": subs}})
}

fn pr_file_nodes(pr: &Value) -> Vec<Value> {
    if let Some(arr) = pr["files"]["nodes"].as_array() {
        arr.clone()
    } else if let Some(arr) = pr.get("files").and_then(Value::as_array) {
        arr.iter()
            .filter_map(|v| v.as_str().map(|p| json!({"path": p})).or_else(|| v.is_object().then(|| v.clone())))
            .collect()
    } else {
        Vec::new()
    }
}

fn pr_files_page(st: &GhState, pr: &Value, after: Option<&str>) -> Value {
    let all = pr_file_nodes(pr);
    let start: usize = after.and_then(|c| c.parse().ok()).unwrap_or(0);
    let size = if st.pr_file_page == 0 { 100 } else { st.pr_file_page };
    let end = (start + size).min(all.len());
    json!({
        "pageInfo": {"hasNextPage": end < all.len(), "endCursor": (end < all.len()).then(|| end.to_string())},
        "nodes": all[start.min(end)..end],
    })
}

fn full_pr_node(st: &GhState, display_repo: &str, number: u64, pr: &Value, with_files: bool) -> Value {
    let head_ref = pr["headRefName"].as_str().map(String::from).unwrap_or_else(|| format!("feat/{number}"));
    let base_ref = pr["baseRefName"].as_str().unwrap_or("main");
    let closing = if pr.get("closingIssuesReferences").is_some_and(Value::is_object) {
        pr["closingIssuesReferences"].clone()
    } else {
        json!({"nodes": []})
    };
    let mut node = json!({
        "number": pr["number"].as_u64().unwrap_or(number),
        "title": pr["title"].as_str().map(String::from).unwrap_or_else(|| format!("PR {number}")),
        "url": pr["url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{display_repo}/pull/{number}")),
        "body": pr["body"].as_str().unwrap_or(""),
        "baseRefName": base_ref,
        "headRefName": head_ref,
        "headRefOid": pr["headRefOid"].as_str().unwrap_or(""),
        "isCrossRepository": pr["isCrossRepository"].as_bool().unwrap_or(false),
        "closingIssuesReferences": closing,
    });
    if with_files {
        node["files"] = pr_files_page(st, pr, None);
    }
    node
}

fn open_prs_page(st: &GhState, repo: &str, display: &str, after: Option<&str>, with_files: bool) -> Value {
    let mut matching: Vec<(u64, &Value)> = st
        .prs
        .iter()
        .filter(|((r, _), p)| r.eq_ignore_ascii_case(repo) && p["state"] == "OPEN")
        .map(|((_, n), p)| (*n, p))
        .collect();
    matching.sort_by_key(|(n, _)| *n);
    let start: usize = after.and_then(|c| c.parse().ok()).unwrap_or(0);
    let size = if st.pr_page == 0 { 100 } else { st.pr_page };
    let end = (start + size).min(matching.len());
    let nodes: Vec<Value> =
        matching[start.min(end)..end].iter().map(|(n, p)| full_pr_node(st, display, *n, p, with_files)).collect();
    json!({
        "pageInfo": {"hasNextPage": end < matching.len(), "endCursor": (end < matching.len()).then(|| end.to_string())},
        "nodes": nodes,
    })
}

fn flow_pr_files(st: &GhState, variables: &Value) -> Value {
    let raw_repo = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())
        .to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&raw_repo).cloned();
    let repo = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(raw_repo);
    if !st.repos.contains_key(&repo) {
        return gh_error(&format!("Could not resolve to a Repository with the name '{repo}'."));
    }
    let number = variables["number"].as_u64().unwrap();
    let Some(pr) = st.prs.get(&(repo, number)) else {
        return json!({"data": {"repository": {"pullRequest": null}}});
    };
    let files = pr_files_page(st, pr, variables["after"].as_str());
    json!({"data": {"repository": {"pullRequest": {"files": files}}}})
}

fn flow_branch_merged(st: &GhState, variables: &Value) -> Value {
    let mut data = serde_json::Map::new();
    for i in 0.. {
        let Some(owner) = variables[format!("o{i}")].as_str() else { break };
        let raw_repo = format!("{owner}/{}", variables[format!("r{i}")].as_str().unwrap()).to_ascii_lowercase();
        let canonical = st.repo_aliases.get(&raw_repo).cloned();
        let repo = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(raw_repo);
        let branch = variables[format!("b{i}")].as_str().unwrap_or_default();
        let alias = format!("b{i}");
        if !st.repos.contains_key(&repo) {
            data.insert(alias, Value::Null);
            continue;
        }
        let mut merged: Vec<(u64, &Value)> = st
            .prs
            .iter()
            .filter(|((r, _), p)| {
                r.eq_ignore_ascii_case(&repo) && p["state"] == "MERGED" && p["headRefName"].as_str() == Some(branch)
            })
            .map(|((_, n), p)| (*n, p))
            .collect();
        merged.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
        let nodes: Vec<Value> = merged
            .into_iter()
            .take(10)
            .map(|(n, p)| {
                json!({
                    "number": n,
                    "isCrossRepository": p["isCrossRepository"].as_bool().unwrap_or(false),
                    "headRefOid": p["headRefOid"].as_str().unwrap_or("")
                })
            })
            .collect();
        data.insert(alias, json!({"pullRequests": {"nodes": nodes}}));
    }
    json!({"data": data})
}

fn flow_open_prs(st: &GhState, variables: &Value) -> Value {
    let raw_repo = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())
        .to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&raw_repo).cloned();
    let repo = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(raw_repo);
    if !st.repos.contains_key(&repo) {
        return gh_error(&format!("Could not resolve to a Repository with the name '{repo}'."));
    }
    let display = st.repo_names.get(&repo).cloned().or(canonical).unwrap_or_else(|| repo.clone());
    let default_branch = st.default_branches.get(&repo).map(String::as_str).unwrap_or("main");
    let with_files = variables.get("withPrFiles").and_then(Value::as_bool).unwrap_or(true);
    let prs = open_prs_page(st, &repo, &display, variables["after"].as_str(), with_files);
    json!({"data": {"repository": {
        "nameWithOwner": display,
        "defaultBranchRef": {"name": default_branch},
        "pullRequests": prs
    }}})
}

fn flow_issues(st: &GhState, query: &str, variables: &Value) -> Value {
    let raw_repo = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())
        .to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&raw_repo).cloned();
    let repo = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(raw_repo);
    if !st.repos.contains_key(&repo) {
        return gh_error(&format!("Could not resolve to a Repository with the name '{repo}'."));
    }
    let want_state = if query.contains("states: CLOSED") { "CLOSED" } else { "OPEN" };
    let label = variables["label"].as_str().map(str::to_ascii_lowercase);
    let mut matching: Vec<&Value> = st
        .issues
        .iter()
        .filter(|((r, _), i)| {
            *r == repo
                && i["state"] == want_state
                && label.as_ref().is_none_or(|want| {
                    i["labels"]["nodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|l| l["name"].as_str().is_some_and(|n| n.to_ascii_lowercase() == *want))
                })
        })
        .map(|(_, i)| i)
        .collect();
    if want_state == "CLOSED" {
        matching.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    } else {
        matching.sort_by(|a, b| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()));
    }
    let start: usize = variables["after"].as_str().and_then(|c| c.parse().ok()).unwrap_or(0);
    let size = if st.issue_page == 0 { 100 } else { st.issue_page };
    let end = (start + size).min(matching.len());
    let nodes: Vec<Value> = matching[start.min(end)..end].iter().map(|i| full_issue_node(st, i)).collect();
    let name = matching
        .first()
        .and_then(|i| i["nameWithOwner"].as_str())
        .or_else(|| st.repo_names.get(&repo).map(String::as_str))
        .or(canonical.as_deref())
        .unwrap_or(&repo)
        .to_string();
    let default_branch = st.default_branches.get(&repo).map(String::as_str).unwrap_or("main");
    let mut repository = json!({
        "nameWithOwner": name,
        "defaultBranchRef": {"name": default_branch},
        "issues": {
            "pageInfo": {"hasNextPage": end < matching.len(), "endCursor": (end < matching.len()).then(|| end.to_string())},
            "nodes": nodes
        }
    });
    if variables.get("withPrs").and_then(Value::as_bool).unwrap_or(false) {
        let with_files = variables.get("withPrFiles").and_then(Value::as_bool).unwrap_or(true);
        repository["pullRequests"] = open_prs_page(st, &repo, &name, None, with_files);
    }
    json!({"data": {"repository": repository}})
}

fn repo_canonical(st: &GhState, variables: &Value) -> Value {
    let raw = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap());
    let key = raw.to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&key).cloned();
    let lookup = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(key);
    if !st.repos.contains_key(&lookup) {
        return json!({"data": {"repository": null}, "errors": [{"type": "NOT_FOUND", "message": format!("Could not resolve to a Repository with the name '{raw}'.")}]});
    }
    let name = st
        .issues
        .iter()
        .find_map(|((r, _), i)| (*r == lookup).then(|| i["nameWithOwner"].as_str()).flatten())
        .or(canonical.as_deref())
        .unwrap_or(&raw)
        .to_string();
    json!({"data": {"repository": {"nameWithOwner": name}}})
}

fn flow_tickets(st: &GhState, variables: &Value) -> Value {
    let mut data = serde_json::Map::new();
    let mut errs = Vec::new();
    for i in 0.. {
        let Some(owner) = variables[format!("o{i}")].as_str() else { break };
        let raw_repo = format!("{owner}/{}", variables[format!("r{i}")].as_str().unwrap());
        let raw_key = raw_repo.to_ascii_lowercase();
        let canonical = st.repo_aliases.get(&raw_key).cloned();
        let repo = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(raw_key);
        let number = variables[format!("n{i}")].as_u64().unwrap();
        if number > i32::MAX as u64 {
            return gh_error(&format!("Variable $n{i} of type Int! was provided invalid value {number}"));
        }
        let alias = format!("t{i}");
        let issue = st.issues.get(&issue_key(&repo, number)).map(|i| full_issue_node(st, i));
        if issue.is_none() {
            errs.push(json!({"type": "NOT_FOUND", "path": [alias, "issue"], "message": "no issue"}));
        }
        let name = issue
            .as_ref()
            .and_then(|_| st.issues.get(&issue_key(&repo, number)).and_then(|v| v["nameWithOwner"].as_str()))
            .or(canonical.as_deref())
            .unwrap_or(&raw_repo);
        data.insert(alias, json!({"nameWithOwner": name, "issue": issue}));
    }
    let mut out = json!({"data": data});
    if !errs.is_empty() {
        out["errors"] = json!(errs);
    }
    out
}

fn resolve_repo_lookup(st: &GhState, owner: &str, name: &str) -> Option<(String, String)> {
    let raw = format!("{owner}/{name}");
    let key = raw.to_ascii_lowercase();
    let canonical = st.repo_aliases.get(&key).cloned();
    let lookup = canonical.as_deref().map(str::to_ascii_lowercase).unwrap_or(key);
    if !st.repos.contains_key(&lookup) {
        return None;
    }
    let display = st.repo_names.get(&lookup).cloned().or(canonical).unwrap_or(raw);
    Some((lookup, display))
}

fn find_repo_by_id(st: &GhState, repo_id: &str) -> Option<(String, String)> {
    let key = repo_id.strip_prefix("R_")?.to_ascii_lowercase();
    if !st.repos.contains_key(&key) {
        return None;
    }
    let display = st.repo_names.get(&key).cloned().unwrap_or_else(|| key.clone());
    Some((key, display))
}

fn parse_issue_node_id(node_id: &str) -> Option<(String, u64)> {
    let rest = node_id.strip_prefix("I_")?;
    let (repo, num) = rest.rsplit_once('#')?;
    Some((repo.to_ascii_lowercase(), num.parse().ok()?))
}

fn repo_id_query(st: &GhState, variables: &Value) -> Value {
    let (owner, name) = (variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap());
    let Some((lookup, display)) = resolve_repo_lookup(st, owner, name) else {
        return gh_typed_error(
            "NOT_FOUND",
            &format!("Could not resolve to a Repository with the name '{owner}/{name}'."),
        );
    };
    json!({"data": {"repository": {"id": repo_node_id(&lookup), "nameWithOwner": display}}})
}

fn issue_node_query(st: &mut GhState, variables: &Value) -> Value {
    let (owner, name) = (variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap());
    let number = variables["number"].as_u64().unwrap();
    if number > i32::MAX as u64 {
        return gh_error(&format!("Variable $number of type Int! was provided invalid value {number}"));
    }
    let Some((lookup, display)) = resolve_repo_lookup(st, owner, name) else {
        return gh_typed_error(
            "NOT_FOUND",
            &format!("Could not resolve to a Repository with the name '{owner}/{name}'."),
        );
    };
    let Some(i) = st.issues.get(&(lookup.clone(), number)).cloned() else {
        return json!({
            "data": {"repository": {"id": repo_node_id(&lookup), "nameWithOwner": display, "issue": null}},
            "errors": [{"type": "NOT_FOUND", "message": format!("Could not resolve to an Issue with the number of {number}.")}]
        });
    };
    let name_with_owner = i["nameWithOwner"].as_str().unwrap_or(&display).to_string();
    let parent = match (i["parentRef"][0].as_str(), i["parentRef"][1].as_u64()) {
        (Some(pr), Some(pn)) => {
            let p_low = pr.to_ascii_lowercase();
            let p_disp = st.issues.get(&(p_low.clone(), pn)).and_then(|pi| pi["nameWithOwner"].as_str()).unwrap_or(pr);
            json!({"id": issue_node_id(&p_low, pn), "number": pn, "repository": {"nameWithOwner": p_disp}})
        }
        _ => Value::Null,
    };
    let blockers: Vec<Value> = i["blockers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let br = r[0].as_str()?;
            let bn = r[1].as_u64()?;
            let b_low = br.to_ascii_lowercase();
            let b_disp = st.issues.get(&(b_low.clone(), bn)).and_then(|bi| bi["nameWithOwner"].as_str()).unwrap_or(br);
            Some(json!({"id": issue_node_id(&b_low, bn), "number": bn, "repository": {"nameWithOwner": b_disp}}))
        })
        .collect();
    let label_names: Vec<String> = i["labels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["name"].as_str().map(String::from))
        .collect();
    let labels: Vec<Value> = label_names
        .into_iter()
        .map(|lname| {
            let (lid, canon) = ensure_repo_label_entry(st, &lookup, &lname, "ededed");
            json!({"id": lid, "name": canon})
        })
        .collect();
    json!({
        "data": {
            "repository": {
                "id": repo_node_id(&lookup),
                "nameWithOwner": name_with_owner,
                "issue": {
                    "id": issue_node_id(&lookup, number),
                    "number": number,
                    "url": format!("https://github.com/{name_with_owner}/issues/{number}"),
                    "title": i["title"],
                    "parent": parent,
                    "blockedBy": {"nodes": blockers},
                    "labels": {"nodes": labels},
                }
            }
        }
    })
}

fn repo_label_query(st: &GhState, variables: &Value) -> Value {
    let (owner, name) = (variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap());
    let wanted = variables["label"].as_str().unwrap();
    let Some((lookup, _)) = resolve_repo_lookup(st, owner, name) else {
        return gh_typed_error(
            "NOT_FOUND",
            &format!("Could not resolve to a Repository with the name '{owner}/{name}'."),
        );
    };
    let found = st
        .repo_labels
        .get(&lookup)
        .and_then(|list| list.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(wanted)))
        .map(|(id, n, _)| json!({"id": id, "name": n}))
        .unwrap_or(Value::Null);
    json!({"data": {"repository": {"id": repo_node_id(&lookup), "label": found}}})
}

fn create_label_mut(st: &mut GhState, variables: &Value) -> Value {
    let repo_id = variables["repoId"].as_str().unwrap_or_default();
    let name = variables["name"].as_str().unwrap_or_default();
    let color = variables["color"].as_str().unwrap_or("ededed");
    let Some((lookup, _)) = find_repo_by_id(st, repo_id) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a Repository");
    };
    if st.repo_labels.get(&lookup).is_some_and(|list| list.iter().any(|(_, n, _)| n.eq_ignore_ascii_case(name))) {
        return gh_error("Name has already been taken");
    }
    let (id, canon) = ensure_repo_label_entry(st, &lookup, name, color);
    json!({"data": {"createLabel": {"label": {"id": id, "name": canon}}}})
}

fn add_labels_mut(st: &mut GhState, variables: &Value) -> Value {
    let labelable_id = variables["labelableId"].as_str().unwrap_or_default();
    let Some((repo, num)) = parse_issue_node_id(labelable_id) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    if !st.issues.contains_key(&(repo.clone(), num)) {
        return gh_typed_error("NOT_FOUND", "Could not resolve to an Issue");
    }
    let ids: Vec<&str> = variables["labelIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut resolved = Vec::new();
    for lid in ids {
        let Some((_, lname, _)) = st.repo_labels.get(&repo).and_then(|list| list.iter().find(|(id, _, _)| id == lid))
        else {
            return gh_typed_error("NOT_FOUND", &format!("Could not resolve to a Label {lid}"));
        };
        resolved.push(lname.clone());
    }
    let issue = st.issues.get_mut(&(repo, num)).expect("issue exists");
    let nodes = issue["labels"]["nodes"].as_array_mut().expect("labels.nodes array");
    for lname in resolved {
        if !nodes.iter().any(|n| n["name"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(&lname))) {
            nodes.push(json!({"name": lname}));
        }
    }
    json!({"data": {"addLabelsToLabelable": {"labelable": {"__typename": "Issue"}}}})
}

fn remove_labels_mut(st: &mut GhState, variables: &Value) -> Value {
    let labelable_id = variables["labelableId"].as_str().unwrap_or_default();
    let Some((repo, num)) = parse_issue_node_id(labelable_id) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    if !st.issues.contains_key(&(repo.clone(), num)) {
        return gh_typed_error("NOT_FOUND", "Could not resolve to an Issue");
    }
    let ids: Vec<&str> = variables["labelIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut resolved = Vec::new();
    for lid in ids {
        let Some((_, lname, _)) = st.repo_labels.get(&repo).and_then(|list| list.iter().find(|(id, _, _)| id == lid))
        else {
            return gh_typed_error("NOT_FOUND", &format!("Could not resolve to a Label {lid}"));
        };
        resolved.push(lname.clone());
    }
    let issue = st.issues.get_mut(&(repo, num)).expect("issue exists");
    let nodes = issue["labels"]["nodes"].as_array_mut().expect("labels.nodes array");
    nodes.retain(|n| !n["name"].as_str().is_some_and(|s| resolved.iter().any(|r| r.eq_ignore_ascii_case(s))));
    json!({"data": {"removeLabelsFromLabelable": {"labelable": {"__typename": "Issue"}}}})
}

fn add_blocked_by_mut(st: &mut GhState, variables: &Value) -> Value {
    let issue_id = variables["issueId"].as_str().unwrap_or_default();
    let blocker_id = variables["blockingIssueId"].as_str().unwrap_or_default();
    let (Some((repo, num)), Some((b_repo, b_num))) = (parse_issue_node_id(issue_id), parse_issue_node_id(blocker_id))
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    let Some(blocker_disp) =
        st.issues.get(&(b_repo.clone(), b_num)).map(|i| i["nameWithOwner"].as_str().unwrap_or(&b_repo).to_string())
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to blocking Issue");
    };
    let Some(issue) = st.issues.get_mut(&(repo.clone(), num)) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to Issue");
    };
    let blockers = issue["blockers"].as_array_mut().expect("blockers array");
    if blockers
        .iter()
        .any(|r| r[0].as_str().is_some_and(|s| s.eq_ignore_ascii_case(&b_repo)) && r[1].as_u64() == Some(b_num))
    {
        return gh_error("Issue is already blocked by this issue");
    }
    blockers.push(json!([blocker_disp, b_num]));
    json!({
        "data": {
            "addBlockedBy": {
                "issue": {"id": issue_id, "number": num},
                "blockingIssue": {"id": blocker_id, "number": b_num}
            }
        }
    })
}

fn remove_blocked_by_mut(st: &mut GhState, variables: &Value) -> Value {
    let issue_id = variables["issueId"].as_str().unwrap_or_default();
    let blocker_id = variables["blockingIssueId"].as_str().unwrap_or_default();
    let (Some((repo, num)), Some((b_repo, b_num))) = (parse_issue_node_id(issue_id), parse_issue_node_id(blocker_id))
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    let Some(issue) = st.issues.get_mut(&(repo, num)) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to Issue");
    };
    let blockers = issue["blockers"].as_array_mut().expect("blockers array");
    let before = blockers.len();
    blockers
        .retain(|r| !(r[0].as_str().is_some_and(|s| s.eq_ignore_ascii_case(&b_repo)) && r[1].as_u64() == Some(b_num)));
    if blockers.len() == before {
        return gh_error("Issue is not blocked by this issue");
    }
    json!({
        "data": {
            "removeBlockedBy": {
                "issue": {"id": issue_id, "number": num},
                "blockingIssue": {"id": blocker_id, "number": b_num}
            }
        }
    })
}

fn add_sub_issue_mut(st: &mut GhState, variables: &Value) -> Value {
    let parent_id = variables["issueId"].as_str().unwrap_or_default();
    let child_id = variables["subIssueId"].as_str().unwrap_or_default();
    let replace = variables["replaceParent"].as_bool().unwrap_or(false);
    let (Some((p_repo, p_num)), Some((c_repo, c_num))) =
        (parse_issue_node_id(parent_id), parse_issue_node_id(child_id))
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    let Some(parent_disp) =
        st.issues.get(&(p_repo.clone(), p_num)).map(|i| i["nameWithOwner"].as_str().unwrap_or(&p_repo).to_string())
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to parent Issue");
    };
    let Some(child) = st.issues.get_mut(&(c_repo, c_num)) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to sub-issue");
    };
    if let (Some(cur_repo), Some(cur_num)) = (child["parentRef"][0].as_str(), child["parentRef"][1].as_u64()) {
        if cur_repo.eq_ignore_ascii_case(&p_repo) && cur_num == p_num {
            return gh_error("Sub-issue is already a sub-issue of this issue");
        }
        if !replace {
            return gh_error("Sub-issue already has a parent");
        }
    }
    child["parentRef"] = json!([parent_disp, p_num]);
    json!({
        "data": {
            "addSubIssue": {
                "issue": {"id": parent_id, "number": p_num},
                "subIssue": {"id": child_id, "number": c_num}
            }
        }
    })
}

fn remove_sub_issue_mut(st: &mut GhState, variables: &Value) -> Value {
    let parent_id = variables["issueId"].as_str().unwrap_or_default();
    let child_id = variables["subIssueId"].as_str().unwrap_or_default();
    let (Some((p_repo, p_num)), Some((c_repo, c_num))) =
        (parse_issue_node_id(parent_id), parse_issue_node_id(child_id))
    else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a node");
    };
    let Some(child) = st.issues.get_mut(&(c_repo, c_num)) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to sub-issue");
    };
    let matches_parent = child["parentRef"][0].as_str().is_some_and(|r| r.eq_ignore_ascii_case(&p_repo))
        && child["parentRef"][1].as_u64() == Some(p_num);
    if !matches_parent {
        return gh_error("Issue is not a sub-issue of this issue");
    }
    child["parentRef"] = Value::Null;
    json!({
        "data": {
            "removeSubIssue": {
                "issue": {"id": parent_id, "number": p_num},
                "subIssue": {"id": child_id, "number": c_num}
            }
        }
    })
}

fn create_issue_mut(st: &mut GhState, variables: &Value) -> Value {
    let repo_id = variables["repoId"].as_str().unwrap_or_default();
    let title = variables["title"].as_str().unwrap_or_default();
    let body = variables["body"].as_str().unwrap_or_default();
    let Some((lookup, display)) = find_repo_by_id(st, repo_id) else {
        return gh_typed_error("NOT_FOUND", "Could not resolve to a Repository");
    };
    let next_num = st.issues.keys().filter(|(r, _)| *r == lookup).map(|(_, n)| *n).max().unwrap_or(0) + 1;
    let mut label_nodes = Vec::new();
    for lid in variables["labelIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if let Some((_, lname, _)) =
            st.repo_labels.get(&lookup).and_then(|list| list.iter().find(|(id, _, _)| id == lid))
        {
            label_nodes.push(json!({"name": lname}));
        }
    }
    let created_at = format!("2026-11-17T12:{:02}:{:02}Z", (next_num / 60) % 60, next_num % 60);
    let url = format!("https://github.com/{display}/issues/{next_num}");
    st.issues.insert(
        (lookup.clone(), next_num),
        json!({
            "number": next_num,
            "title": title,
            "body": body,
            "state": "OPEN",
            "createdAt": created_at,
            "labels": {"nodes": label_nodes},
            "milestone": null,
            "blockers": [],
            "parentRef": null,
            "nameWithOwner": display,
        }),
    );
    json!({
        "data": {
            "createIssue": {
                "issue": {
                    "id": issue_node_id(&lookup, next_num),
                    "number": next_num,
                    "url": url,
                    "title": title
                }
            }
        }
    })
}

impl crate::github::GitHub for FakeGitHub {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, anyhow::Result<Value>> {
        self.graphql_with_headers(query, variables, &[])
    }

    fn graphql_with_headers<'a>(
        &'a self,
        query: &'a str,
        variables: Value,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFut<'a, anyhow::Result<Value>> {
        Box::pin(async move {
            let delay = self.st().delay;
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            tokio::task::yield_now().await;
            let op = operation(query);
            let (yields, hit, per_pr) = {
                let mut st = self.st();
                let mut call = json!({"operation": op, "query": query, "variables": variables});
                if !headers.is_empty() {
                    let hdrs: serde_json::Map<String, Value> =
                        headers.iter().map(|(k, v)| ((*k).to_string(), json!(v))).collect();
                    call["headers"] = Value::Object(hdrs);
                }
                st.calls.push(call);
                let targets = gh_targets(&st, &variables);
                let hit = st.faults.hit(&gh_ops(op, query, &variables), &targets);
                let per_pr: Vec<(String, Hit)> = if op == "reconcile" && hit.before().is_none() {
                    batch_targets(&st, &variables)
                        .into_iter()
                        .map(|(alias, t)| (alias, st.faults.hit(&["reconcile.pr"], &t)))
                        .collect()
                } else {
                    Vec::new()
                };
                (st.faults.yields(), hit, per_pr)
            };
            fault::jitter(yields).await;
            let hits: Vec<&Hit> = std::iter::once(&hit).chain(per_pr.iter().map(|(_, h)| h)).collect();
            fault::pass_all(&hits).await;
            if let Some(f) = hit.before() {
                return gh_failure(f);
            }
            let failed: BTreeMap<String, Failure> =
                per_pr.into_iter().filter_map(|(alias, h)| h.fail.map(|f| (alias, f))).collect();
            let out = self.respond(op, query, &variables, &failed);
            match hit.after() {
                Some(f) => gh_failure(f),
                None => out,
            }
        })
    }
}

impl FakeGitHub {
    fn respond(
        &self,
        op: &'static str,
        query: &str,
        variables: &Value,
        failed: &BTreeMap<String, Failure>,
    ) -> anyhow::Result<Value> {
        let mut st = self.st();
        if op == "repoCanonical" {
            return Ok(repo_canonical(&st, variables));
        }
        if op == "repoId" {
            return Ok(repo_id_query(&st, variables));
        }
        if op == "issueNode" {
            return Ok(issue_node_query(&mut st, variables));
        }
        if op == "repoLabel" {
            return Ok(repo_label_query(&st, variables));
        }
        if op == "createLabel" {
            return Ok(create_label_mut(&mut st, variables));
        }
        if op == "addLabelsToLabelable" {
            return Ok(add_labels_mut(&mut st, variables));
        }
        if op == "removeLabelsFromLabelable" {
            return Ok(remove_labels_mut(&mut st, variables));
        }
        if op == "addBlockedBy" {
            return Ok(add_blocked_by_mut(&mut st, variables));
        }
        if op == "removeBlockedBy" {
            return Ok(remove_blocked_by_mut(&mut st, variables));
        }
        if op == "addSubIssue" {
            return Ok(add_sub_issue_mut(&mut st, variables));
        }
        if op == "removeSubIssue" {
            return Ok(remove_sub_issue_mut(&mut st, variables));
        }
        if op == "createIssue" {
            return Ok(create_issue_mut(&mut st, variables));
        }
        if op == "flowIssues" {
            return Ok(flow_issues(&st, query, variables));
        }
        if op == "flowOpenPrs" {
            return Ok(flow_open_prs(&st, variables));
        }
        if op == "flowPrFiles" {
            return Ok(flow_pr_files(&st, variables));
        }
        if op == "flowBranchMerged" {
            return Ok(flow_branch_merged(&st, variables));
        }
        if op == "flowTickets" {
            return Ok(flow_tickets(&st, variables));
        }
        if op == "reconcile" {
            let mut data = serde_json::Map::new();
            let mut errs = Vec::new();
            for i in 0.. {
                let Some(owner) = variables[format!("o{i}")].as_str() else { break };
                let repo = format!("{owner}/{}", variables[format!("r{i}")].as_str().unwrap()).to_ascii_lowercase();
                let number = variables[format!("n{i}")].as_u64().unwrap();
                let alias = format!("p{i}");
                if !st.repos.contains_key(&repo) {
                    errs.push(
                        json!({"type": "NOT_FOUND", "path": [alias], "message": format!("no repository {repo}")}),
                    );
                    data.insert(alias, Value::Null);
                    continue;
                }
                let mut pr = st.prs.get(&(repo.clone(), number)).cloned().unwrap_or(Value::Null);
                if let Some(Failure::Field { field, message }) = failed.get(&alias) {
                    errs.push(json!({"path": [alias, "pullRequest", field], "message": message}));
                    pr[field.as_str()] = Value::Null;
                } else if let Some(f) = failed.get(&alias) {
                    let mut err = json!({"path": [alias, "pullRequest"], "message": f.message()});
                    if let Failure::Typed { kind, .. } = f {
                        err["type"] = json!(kind);
                    }
                    errs.push(err);
                    pr = Value::Null;
                } else if pr.is_null() {
                    errs.push(
                        json!({"type": "NOT_FOUND", "path": [alias, "pullRequest"], "message": "no pull request"}),
                    );
                }
                let mut repository = st.repos[&repo].clone();
                repository["pullRequest"] = pr;
                data.insert(alias, repository);
            }
            let mut out = json!({"data": data});
            if !errs.is_empty() {
                out["errors"] = json!(errs);
            }
            return Ok(out);
        }
        if op == "read" {
            let repo = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())
                .to_ascii_lowercase();
            let Some(meta) = st.repos.get(&repo).cloned() else {
                return Ok(gh_error(&format!("Could not resolve to a Repository with the name '{repo}'.")));
            };
            let number = variables["number"].as_u64().unwrap();
            let mut pr = st.prs.get(&(repo, number)).cloned().unwrap_or(Value::Null);
            if st.unknown_reads > 0 && !pr.is_null() {
                st.unknown_reads -= 1;
                pr["mergeStateStatus"] = json!("UNKNOWN");
            }
            if pr["headRef"].is_object() && pr["headRef"]["target"].is_null() {
                pr["headRef"]["target"] = json!({"oid": pr["headRefOid"]});
            }
            let mut repository = meta;
            repository["pullRequest"] = pr;
            return Ok(json!({"data": {"repository": repository}}));
        }
        if op == "deleteRef" {
            let wanted = variables["ref"].as_str().unwrap_or_default().to_string();
            let Some(pr) = st.prs.values_mut().find(|p| p["headRef"]["id"] == wanted.as_str()) else {
                return Ok(gh_error("Could not resolve to a node"));
            };
            pr["headRef"] = Value::Null;
            return Ok(json!({"data": {"deleteRef": {"clientMutationId": null}}}));
        }
        let id = variables["id"].as_str().unwrap().to_string();
        let head = variables["head"].as_str().unwrap().to_string();
        let queue = st.queue;
        st.merges += 1;
        let merges = st.merges;
        let Some(pr) = st.prs.values_mut().find(|p| p["id"] == id.as_str()) else {
            return Ok(gh_error("Could not resolve to a node"));
        };
        if pr["headRefOid"] != head.as_str() {
            return Ok(gh_error("Head branch was modified. Review and try the merge again."));
        }
        match op {
            "enqueuePullRequest" => {
                let position = queue + 1;
                pr["isInMergeQueue"] = json!(true);
                pr["mergeQueueEntry"] = json!({"position": position});
                Ok(json!({"data": {"enqueuePullRequest": {"mergeQueueEntry": {"position": position}}}}))
            }
            "mergePullRequest" => {
                let oid = format!("{merges:040x}");
                pr["state"] = json!("MERGED");
                pr["mergeCommit"] = json!({"oid": oid});
                Ok(json!({"data": {"mergePullRequest": {"pullRequest": {"mergeCommit": {"oid": oid}}}}}))
            }
            _ => Ok(gh_error("the fake does not support this mutation")),
        }
    }
}

type MrKey = (String, String, u64);

#[derive(Default)]
struct GlState {
    mrs: BTreeMap<MrKey, Value>,
    calls: Vec<Value>,
    read_errors: BTreeMap<MrKey, String>,
    rest_status: BTreeMap<String, u16>,
    after: BTreeMap<String, Value>,
    trains: std::collections::BTreeSet<(String, String)>,
    merges: u64,
    down: bool,
}

#[derive(Default)]
pub struct FakeGitLab(Mutex<GlState>);

pub fn gitlab_mr(head: &str, detailed_merge_status: &str) -> Value {
    json!({
        "state": "opened", "draft": false, "sha": head, "detailed_merge_status": detailed_merge_status,
        "merged_at": null, "closed_at": null, "updated_at": null, "merge_commit_sha": null, "squash_commit_sha": null,
        "squash_on_merge": false, "squash": false, "head_pipeline": null, "pipelines": [], "train": null,
        "description": "", "diffs": [], "changes_count": "0"
    })
}

pub fn gitlab_pipeline(id: u64, sha: &str, git_ref: &str, status: &str) -> Value {
    json!({"id": id, "sha": sha, "ref": git_ref, "status": status, "source": "merge_request_event"})
}

fn mr_key(host: &str, path: &str, iid: u64) -> MrKey {
    (host.to_ascii_lowercase(), path.to_ascii_lowercase(), iid)
}

fn upper(v: &Value) -> Value {
    v.as_str().map_or(Value::Null, |s| json!(s.to_ascii_uppercase()))
}

pub fn gitlab_mr_id(iid: u64) -> u64 {
    1000 + iid
}

pub fn gitlab_gid(iid: u64) -> String {
    format!("gid://gitlab/MergeRequest/{}", gitlab_mr_id(iid))
}

fn mr_id(iid: u64, mr: &Value) -> u64 {
    mr["id"].as_u64().unwrap_or_else(|| gitlab_mr_id(iid))
}

const HIDDEN: &[&str] =
    &["pipelines", "train", "diffs", "branch_deleted", "branch_tip", "train_when_pipeline_succeeds", "cancel_fails"];

fn graphql_mr(iid: u64, mr: &Value) -> Value {
    let pipe = &mr["head_pipeline"];
    let car = &mr["train"];
    json!({
        "id": format!("gid://gitlab/MergeRequest/{}", mr_id(iid, mr)), "state": mr["state"], "draft": mr["draft"], "diffHeadSha": mr["sha"],
        "detailedMergeStatus": upper(&mr["detailed_merge_status"]), "mergedAt": mr["merged_at"],
        "squashOnMerge": mr["squash_on_merge"], "mergeCommitSha": mr["merge_commit_sha"],
        "headPipeline": if pipe.is_object() { json!({"status": upper(&pipe["status"]), "ref": pipe["ref"]}) } else { Value::Null },
        "mergeTrainCar": if car.is_object() { json!({"status": upper(&car["status"]), "index": car["index"]}) } else { Value::Null },
    })
}

fn rest_mr(iid: u64, mr: &Value) -> Value {
    let mut out = mr.clone();
    if let Some(o) = out.as_object_mut() {
        for k in HIDDEN {
            o.remove(*k);
        }
        o.insert("iid".into(), json!(iid));
        o.insert("id".into(), json!(mr_id(iid, mr)));
        o.entry("source_branch").or_insert_with(|| json!(format!("fix/{iid}")));
        o.entry("source_project_id").or_insert(json!(1));
        o.entry("target_project_id").or_insert(json!(1));
    }
    out
}

impl FakeGitLab {
    fn st(&self) -> std::sync::MutexGuard<'_, GlState> {
        self.0.lock().unwrap()
    }

    pub fn set_mr(&self, host: &str, path: &str, iid: u64, mr: Value) {
        self.st().mrs.insert(mr_key(host, path, iid), mr);
    }

    pub fn update_mr(&self, host: &str, path: &str, iid: u64, patch: Value) {
        let mut st = self.st();
        let mr = st.mrs.get_mut(&mr_key(host, path, iid)).expect("known merge request");
        for (k, v) in patch.as_object().expect("object patch") {
            mr[k] = v.clone();
        }
    }

    pub fn mr(&self, host: &str, path: &str, iid: u64) -> Value {
        self.st().mrs[&mr_key(host, path, iid)].clone()
    }

    pub fn remove_mr(&self, host: &str, path: &str, iid: u64) {
        self.st().mrs.remove(&mr_key(host, path, iid));
    }

    pub fn set_pipeline(&self, host: &str, path: &str, iid: u64, status: &str) {
        let mr = self.mr(host, path, iid);
        let sha = mr["sha"].as_str().unwrap_or_default().to_string();
        let pipe = gitlab_pipeline(100 + iid, &sha, &format!("refs/merge-requests/{iid}/head"), status);
        self.update_mr(host, path, iid, json!({"pipelines": [pipe.clone()], "head_pipeline": pipe}));
    }

    pub fn set_files(&self, host: &str, path: &str, iid: u64, files: &[&str]) {
        let diffs: Vec<Value> = files.iter().map(|f| json!({"old_path": f, "new_path": f})).collect();
        let count = if files.len() > 1000 { "1000+".to_string() } else { files.len().to_string() };
        self.update_mr(host, path, iid, json!({"diffs": diffs, "changes_count": count}));
    }

    pub fn join_train(&self, host: &str, path: &str, iid: u64, index: u64) {
        let mr = self.mr(host, path, iid);
        let train = gitlab_pipeline(900 + iid, &"e".repeat(40), &format!("refs/merge-requests/{iid}/train"), "running");
        let mut pipelines = vec![train.clone()];
        pipelines.extend(mr["pipelines"].as_array().cloned().unwrap_or_default());
        self.update_mr(
            host,
            path,
            iid,
            json!({"train": {"status": "fresh", "index": index}, "head_pipeline": train, "pipelines": pipelines,
                   "detailed_merge_status": "merge_request_blocked"}),
        );
    }

    pub fn merge(&self, host: &str, path: &str, iid: u64, commit: &str, at: &str) {
        let mr = self.mr(host, path, iid);
        let squash = mr["squash"] == true;
        let (merge_commit, squash_commit) =
            if squash { (Value::Null, json!(commit)) } else { (json!(commit), Value::Null) };
        let mut patch = json!({"state": "merged", "merged_at": at, "updated_at": at, "merge_commit_sha": merge_commit,
                               "squash_commit_sha": squash_commit, "detailed_merge_status": "not_open"});
        if mr["train"].is_object() {
            patch["train"] = json!({"status": "merged", "index": null});
        }
        self.update_mr(host, path, iid, patch);
    }

    pub fn close(&self, host: &str, path: &str, iid: u64, at: &str) {
        let patch = json!({"state": "closed", "closed_at": at, "updated_at": at, "detailed_merge_status": "not_open"});
        self.update_mr(host, path, iid, patch);
    }

    pub fn fail_read(&self, host: &str, path: &str, iid: u64, message: &str) {
        self.st().read_errors.insert(mr_key(host, path, iid), message.into());
    }

    pub fn clear_read_errors(&self) {
        self.st().read_errors.clear();
    }

    pub fn patch_after(&self, operation: &str, patch: Value) {
        self.st().after.insert(operation.into(), patch);
    }

    pub fn set_trains(&self, host: &str, path: &str, on: bool) {
        let key = (host.to_ascii_lowercase(), path.to_ascii_lowercase());
        let mut st = self.st();
        if on {
            st.trains.insert(key);
        } else {
            st.trains.remove(&key);
        }
    }

    pub fn fail_rest(&self, suffix: &str, status: u16) {
        self.st().rest_status.insert(suffix.into(), status);
    }

    pub fn set_down(&self, down: bool) {
        self.st().down = down;
    }

    pub fn calls(&self) -> Vec<Value> {
        self.st().calls.clone()
    }

    pub fn operations(&self) -> Vec<String> {
        self.st().calls.iter().filter_map(|c| c["operation"].as_str().map(String::from)).collect()
    }
}

fn page_of(items: &Value, query: &str) -> Value {
    let arg = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=')?.parse::<usize>().ok());
    let (per, page) = (arg("per_page").unwrap_or(20).max(1), arg("page").unwrap_or(1).max(1));
    let all = items.as_array().cloned().unwrap_or_default();
    json!(all.into_iter().skip((page - 1) * per).take(per).collect::<Vec<_>>())
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn project_of(path: &str) -> Option<(String, &str)> {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    let rest = path.strip_prefix("projects/")?;
    let (project, tail) = rest.split_once('/').unwrap_or((rest, ""));
    Some((decode(project).to_ascii_lowercase(), tail))
}

fn branch_of(st: &GlState, host: &str, project: &str, branch: &str) -> Option<MrKey> {
    st.mrs
        .iter()
        .find(|((h, p, iid), mr)| {
            h == host
                && p == project
                && mr["source_branch"].as_str().map_or(format!("fix/{iid}"), String::from) == branch
        })
        .map(|(k, _)| k.clone())
}

fn active_car(mr: &Value) -> bool {
    mr["train"]["status"].as_str().is_some_and(|s| ["idle", "fresh", "stale", "merging"].contains(&s))
}

fn rest_path(path: &str) -> Option<(String, u64, &str)> {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    let rest = path.strip_prefix("projects/")?;
    let (project, tail) = rest.split_once("/merge_requests/")?;
    let (iid, sub) = tail.split_once('/').unwrap_or((tail, ""));
    Some((project.replace("%2F", "/").to_ascii_lowercase(), iid.parse().ok()?, sub))
}

impl crate::gitlab::GitLab for FakeGitLab {
    fn graphql<'a>(
        &'a self,
        to: crate::gitlab::Target<'a>,
        query: &'a str,
        variables: Value,
    ) -> BoxFut<'a, anyhow::Result<Value>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            let mut st = self.st();
            let op = if query.starts_with("query Reconcile") {
                "reconcile"
            } else if query.starts_with("query MergeTrainCar") {
                "trainCar"
            } else {
                "unknown"
            };
            st.calls
                .push(json!({"operation": op, "host": to.host, "cli": to.cli, "query": query, "variables": variables}));
            if st.down {
                anyhow::bail!("{} api graphql failed: glab: not authenticated", to.cli);
            }
            if op == "trainCar" {
                let path = variables["p"].as_str().unwrap_or_default();
                let iid: u64 = variables["i"].as_str().and_then(|n| n.parse().ok()).unwrap_or_default();
                let key = mr_key(to.host, path, iid);
                if let Some(message) = st.read_errors.get(&key) {
                    return Ok(json!({"errors": [{"message": message}], "data": null}));
                }
                let car = st.mrs.get(&key).map_or(Value::Null, |mr| graphql_mr(iid, mr)["mergeTrainCar"].clone());
                return Ok(json!({"data": {"project": {"mergeRequest": {"mergeTrainCar": car}}}}));
            }
            if op != "reconcile" {
                return Ok(json!({"errors": [{"message": "the fake does not support this query"}]}));
            }
            let mut data = serde_json::Map::new();
            let mut errs = Vec::new();
            for i in 0.. {
                let Some(path) = variables[format!("p{i}")].as_str() else { break };
                let iid: u64 = variables[format!("i{i}")].as_str().and_then(|n| n.parse().ok()).unwrap();
                let alias = format!("m{i}");
                let key = mr_key(to.host, path, iid);
                if let Some(message) = st.read_errors.get(&key) {
                    errs.push(json!({"path": [alias, "mergeRequest"], "message": message}));
                    data.insert(alias, json!({"mergeRequest": null}));
                    continue;
                }
                let known = st.mrs.keys().any(|(h, p, _)| *h == key.0 && *p == key.1);
                let mr = st.mrs.get(&key).map_or(Value::Null, |mr| graphql_mr(iid, mr));
                data.insert(alias, if known { json!({"mergeRequest": mr}) } else { Value::Null });
            }
            let mut out = json!({"data": data});
            if !errs.is_empty() {
                out["errors"] = json!(errs);
            }
            Ok(out)
        })
    }

    fn get<'a>(
        &'a self,
        to: crate::gitlab::Target<'a>,
        path: &'a str,
    ) -> BoxFut<'a, anyhow::Result<crate::gitlab::Rest>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            let mut st = self.st();
            let parsed = rest_path(path);
            let project = project_of(path);
            let op = match (parsed.as_ref().map(|(_, _, sub)| *sub), project.as_ref().map(|(_, t)| *t)) {
                (Some(""), _) => "mergeRequest",
                (Some("pipelines"), _) => "pipelines",
                (Some("diffs"), _) => "diffs",
                (None, Some("")) => "project",
                (None, Some(t)) if t.starts_with("repository/branches/") => "branch",
                _ => "unknown",
            };
            let (bare, query) = path.split_once('?').unwrap_or((path, ""));
            st.calls.push(json!({"operation": op, "host": to.host, "cli": to.cli, "path": path}));
            if st.down {
                anyhow::bail!("{} api failed: glab: not authenticated", to.cli);
            }
            let missing = || crate::gitlab::Rest { status: 404, body: json!({"message": "404 Not found"}) };
            if let Some((_, status)) = st.rest_status.iter().find(|(s, _)| bare.ends_with(s.as_str())) {
                return Ok(crate::gitlab::Rest { status: *status, body: json!({"message": "fake failure"}) });
            }
            if let (Some((project, tail)), "project" | "branch") = (&project, op) {
                let host = to.host.to_ascii_lowercase();
                if !st.mrs.keys().any(|(h, p, _)| *h == host && p == project) {
                    return Ok(missing());
                }
                if op == "project" {
                    let trains = st.trains.contains(&(host, project.clone()));
                    let body = json!({"id": 1, "path_with_namespace": project, "merge_trains_enabled": trains});
                    return Ok(crate::gitlab::Rest { status: 200, body });
                }
                let branch = decode(tail.trim_start_matches("repository/branches/"));
                let Some(key) = branch_of(&st, &host, project, &branch) else { return Ok(missing()) };
                let mr = &st.mrs[&key];
                if mr["branch_deleted"] == true {
                    return Ok(missing());
                }
                let tip = mr["branch_tip"].as_str().or(mr["sha"].as_str()).unwrap_or_default();
                return Ok(crate::gitlab::Rest { status: 200, body: json!({"name": branch, "commit": {"id": tip}}) });
            }
            let Some((project, iid, _)) = parsed else { return Ok(missing()) };
            let key = mr_key(to.host, &project, iid);
            let Some(mr) = st.mrs.get(&key) else { return Ok(missing()) };
            let body = match op {
                "mergeRequest" => rest_mr(iid, mr),
                "pipelines" => page_of(&mr["pipelines"], query),
                "diffs" => page_of(&mr["diffs"], query),
                _ => return Ok(missing()),
            };
            if let Some(patch) = st.after.remove(op)
                && let Some(mr) = st.mrs.get_mut(&key)
            {
                for (k, v) in patch.as_object().into_iter().flatten() {
                    mr[k] = v.clone();
                }
            }
            Ok(crate::gitlab::Rest { status: 200, body })
        })
    }

    fn send<'a>(
        &'a self,
        to: crate::gitlab::Target<'a>,
        method: &'a str,
        path: &'a str,
        body: Value,
    ) -> BoxFut<'a, anyhow::Result<crate::gitlab::Rest>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            let mut st = self.st();
            let bare = path.split_once('?').map_or(path, |(p, _)| p);
            let project = project_of(path);
            let tail = project.as_ref().map(|(_, t)| *t).unwrap_or_default();
            let op = match (method, rest_path(path).as_ref().map(|(_, _, sub)| *sub)) {
                ("POST", _) if tail.starts_with("merge_trains/merge_requests/") => "joinTrain",
                ("DELETE", _) if tail.starts_with("repository/branches/") => "deleteBranch",
                ("PUT", Some("merge")) => "merge",
                ("POST", Some("cancel_auto_merge" | "cancel_merge_when_pipeline_succeeds")) => "cancelAutoMerge",
                _ => "unknown",
            };
            st.calls.push(
                json!({"operation": op, "host": to.host, "cli": to.cli, "method": method, "path": path, "body": body}),
            );
            if st.down {
                anyhow::bail!("{} api failed: glab: not authenticated", to.cli);
            }
            let rest = |status: u16, body: Value| Ok(crate::gitlab::Rest { status, body });
            let missing = || rest(404, json!({"message": "404 Not found"}));
            if let Some((_, status)) = st.rest_status.iter().find(|(s, _)| bare.ends_with(s.as_str())) {
                return rest(*status, json!({"message": "fake failure"}));
            }
            let host = to.host.to_ascii_lowercase();
            let Some((project, tail)) = project else { return missing() };
            if op == "deleteBranch" {
                let branch = decode(tail.trim_start_matches("repository/branches/"));
                let Some(key) = branch_of(&st, &host, &project, &branch) else { return missing() };
                let mr = st.mrs.get_mut(&key).unwrap();
                if mr["branch_deleted"] == true {
                    return missing();
                }
                mr["branch_deleted"] = json!(true);
                return rest(204, Value::Null);
            }
            let iid: u64 = match op {
                "joinTrain" => tail.trim_start_matches("merge_trains/merge_requests/").parse().unwrap_or_default(),
                _ => rest_path(path).map(|(_, iid, _)| iid).unwrap_or_default(),
            };
            let key = mr_key(&host, &project, iid);
            if op == "cancelAutoMerge" {
                let legacy = path.ends_with("/cancel_merge_when_pipeline_succeeds");
                let Some(mr) = st.mrs.get_mut(&key) else { return missing() };
                if mr["merge_when_pipeline_succeeds"] != true && mr["train_when_pipeline_succeeds"] != true {
                    return match legacy {
                        true => rest(
                            201,
                            json!({"status": "error", "http_status": 406, "message": "Can't cancel the automatic merge"}),
                        ),
                        false => rest(409, json!({"message": "409 Conflict"})),
                    };
                }
                if mr["cancel_fails"] == true {
                    return rest(201, json!({"status": "error", "message": "This merge request cannot be cancelled"}));
                }
                mr["merge_when_pipeline_succeeds"] = json!(false);
                mr["train_when_pipeline_succeeds"] = json!(false);
                return match legacy {
                    true => rest(201, json!({"status": "success"})),
                    false => rest(200, rest_mr(iid, mr)),
                };
            }
            if op == "unknown" {
                return missing();
            }
            st.merges += 1;
            let merges = st.merges;
            let trains = st.trains.contains(&(host.clone(), project.clone()));
            let index = st.mrs.iter().filter(|((h, p, _), m)| *h == host && *p == project && active_car(m)).count();
            let Some(mr) = st.mrs.get_mut(&key) else { return missing() };
            if mr["state"] != "opened" || mr["draft"] == true {
                return rest(405, json!({"message": "405 Method Not Allowed"}));
            }
            if body["sha"].as_str() != mr["sha"].as_str() {
                return rest(409, json!({"message": "SHA does not match HEAD of source branch"}));
            }
            let mergeable = mr["detailed_merge_status"] == "mergeable";
            let auto = body["auto_merge"] == true;
            let join = |mr: &mut Value| {
                let train =
                    gitlab_pipeline(900 + iid, &"e".repeat(40), &format!("refs/merge-requests/{iid}/train"), "running");
                mr["train"] = json!({"status": "fresh", "index": index});
                mr["head_pipeline"] = train;
                mr["detailed_merge_status"] = json!("merge_request_blocked");
            };
            if op == "joinTrain" {
                if !trains {
                    return rest(403, json!({"message": "403 Forbidden"}));
                }
                if active_car(mr) {
                    return rest(400, json!({"message": "The merge request is already on the merge train"}));
                }
                if mergeable {
                    join(mr);
                    return rest(201, json!({"id": 50 + iid, "merge_request": rest_mr(iid, mr), "status": "fresh"}));
                }
                if auto {
                    mr["train_when_pipeline_succeeds"] = json!(true);
                    return rest(202, json!({"message": "scheduled"}));
                }
                return rest(400, json!({"message": "The merge request is not mergeable"}));
            }
            if trains && auto && mergeable {
                join(mr);
                return rest(200, rest_mr(iid, mr));
            }
            if !mergeable {
                if auto && mr["detailed_merge_status"] == "ci_still_running" {
                    mr["merge_when_pipeline_succeeds"] = json!(true);
                    return rest(200, rest_mr(iid, mr));
                }
                return rest(405, json!({"message": "405 Method Not Allowed"}));
            }
            let commit = format!("{merges:040x}");
            let squash = body["squash"].as_bool().unwrap_or(mr["squash_on_merge"] == true);
            mr["state"] = json!("merged");
            mr["detailed_merge_status"] = json!("not_open");
            mr[if squash { "squash_commit_sha" } else { "merge_commit_sha" }] = json!(commit);
            if body["should_remove_source_branch"] == true {
                mr["branch_deleted"] = json!(true);
            }
            rest(200, rest_mr(iid, mr))
        })
    }
}

struct FakeTailscaleState {
    bin: Option<std::path::PathBuf>,
    status: Result<crate::tailscale::TailscaleStatus, String>,
    serve_status: Result<crate::tailscale::ServeStatus, String>,
    enable_err: Option<String>,
    disable_err: Option<String>,
    listener_owner: Option<crate::tailscale::ListenerOwner>,
    calls: Vec<String>,
}

impl Default for FakeTailscaleState {
    fn default() -> Self {
        Self {
            bin: None,
            status: Err("tailscale CLI not configured in fake".into()),
            serve_status: Ok(crate::tailscale::ServeStatus::default()),
            enable_err: None,
            disable_err: None,
            listener_owner: None,
            calls: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct FakeTailscale {
    state: Mutex<FakeTailscaleState>,
}

impl FakeTailscale {
    pub fn set_missing(&self) {
        let mut st = self.state.lock().unwrap();
        st.bin = None;
        st.status = Err("tailscale CLI not found".into());
        st.serve_status = Err("tailscale CLI not found".into());
    }

    pub fn set_status(&self, status: crate::tailscale::TailscaleStatus) {
        let mut st = self.state.lock().unwrap();
        if st.bin.is_none() {
            st.bin = Some(std::path::PathBuf::from("/fake/tailscale"));
        }
        st.status = Ok(status);
    }

    pub fn set_host_login(&self, login: &str) {
        self.set_status(crate::tailscale::TailscaleStatus {
            backend_state: Some("Running".into()),
            dns_name: Some("box.acme.ts.net".into()),
            cert_domains: vec!["box.acme.ts.net".into()],
            host_login: Some(login.into()),
            ipv4: None,
        });
    }

    pub fn set_status_error(&self, err: &str) {
        let mut st = self.state.lock().unwrap();
        if st.bin.is_none() {
            st.bin = Some(std::path::PathBuf::from("/fake/tailscale"));
        }
        st.status = Err(err.into());
    }

    pub fn set_serve_status(&self, serve: crate::tailscale::ServeStatus) {
        let mut st = self.state.lock().unwrap();
        if st.bin.is_none() {
            st.bin = Some(std::path::PathBuf::from("/fake/tailscale"));
        }
        st.serve_status = Ok(serve);
    }

    pub fn set_serve_status_error(&self, err: &str) {
        let mut st = self.state.lock().unwrap();
        if st.bin.is_none() {
            st.bin = Some(std::path::PathBuf::from("/fake/tailscale"));
        }
        st.serve_status = Err(err.into());
    }

    pub fn set_enable_error(&self, err: Option<&str>) {
        self.state.lock().unwrap().enable_err = err.map(String::from);
    }

    pub fn set_disable_error(&self, err: Option<&str>) {
        self.state.lock().unwrap().disable_err = err.map(String::from);
    }

    pub fn set_ipv4(&self, ip: Option<std::net::Ipv4Addr>) {
        if let Ok(status) = &mut self.state.lock().unwrap().status {
            status.ipv4 = ip;
        }
    }

    pub fn set_listener_owner(&self, owner: Option<crate::tailscale::ListenerOwner>) {
        self.state.lock().unwrap().listener_owner = owner;
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl crate::tailscale::TailscaleRunner for FakeTailscale {
    fn binary(&self) -> Option<std::path::PathBuf> {
        self.state.lock().unwrap().bin.clone()
    }

    fn status(&self) -> anyhow::Result<crate::tailscale::TailscaleStatus> {
        let mut st = self.state.lock().unwrap();
        st.calls.push("status".into());
        st.status.clone().map_err(anyhow::Error::msg)
    }

    fn serve_status(&self) -> anyhow::Result<crate::tailscale::ServeStatus> {
        let mut st = self.state.lock().unwrap();
        st.calls.push("serve status".into());
        st.serve_status.clone().map_err(anyhow::Error::msg)
    }

    fn serve_enable(&self, https_port: u16, local_port: u16) -> anyhow::Result<()> {
        let mut st = self.state.lock().unwrap();
        st.calls.push(format!("serve --bg --https={https_port} http://127.0.0.1:{local_port}"));
        if let Some(e) = &st.enable_err {
            return Err(anyhow::Error::msg(e.clone()));
        }
        Ok(())
    }

    fn serve_disable(&self, https_port: u16, mount: &str) -> anyhow::Result<()> {
        let mut st = self.state.lock().unwrap();
        st.calls.push(format!("serve --https={https_port} --set-path={mount} off"));
        if let Some(e) = &st.disable_err {
            return Err(anyhow::Error::msg(e.clone()));
        }
        Ok(())
    }

    fn listener_owner(&self, _addr: std::net::SocketAddrV4) -> Option<crate::tailscale::ListenerOwner> {
        self.state.lock().unwrap().listener_owner.clone()
    }

    fn cache_ttl(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// Write an executable script at `path` without holding a writable fd to it in
/// the calling process on Unix.
///
/// In a multithreaded test binary on Linux, calling `std::fs::write` opens a
/// writable fd in the test process; if another test thread concurrently forks a
/// subprocess, that child inherits the writable fd until its `execve` runs,
/// causing `execve` of `path` to fail with `ETXTBSY` (`os error 26`) even after
/// the parent closes its fd or renames the inode. Writing and `chmod`ing inside
/// a short-lived child process guarantees the test process never holds a
/// writable fd to the executable inode.
pub fn write_executable(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).unwrap();
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        const PRINTF_SCRIPT: &str = r#"tmp="$1.tmp.$$"; printf '%s' "$2" > "$tmp" && chmod -- 755 "$tmp" && mv -f -- "$tmp" "$1" || { s=$?; rm -f -- "$tmp"; exit "$s"; }"#;
        const CAT_SCRIPT: &str = r#"tmp="$1.tmp.$$"; cat > "$tmp" && chmod -- 755 "$tmp" && mv -f -- "$tmp" "$1" || { s=$?; rm -f -- "$tmp"; exit "$s"; }"#;

        let status = if body.len() <= 32_768 && !body.contains('\0') {
            Command::new("/bin/sh")
                .args(["-c", PRINTF_SCRIPT, "sh"])
                .arg(path)
                .arg(body)
                .stdin(Stdio::null())
                .status()
                .unwrap_or_else(|e| panic!("spawning /bin/sh to write {}: {e}", path.display()))
        } else {
            let mut child = Command::new("/bin/sh")
                .args(["-c", CAT_SCRIPT, "sh"])
                .arg(path)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap_or_else(|e| panic!("spawning /bin/sh to write {}: {e}", path.display()));
            child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
            child.wait().unwrap()
        };
        assert!(status.success(), "writing executable {} failed: {status}", path.display());
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body).unwrap();
    }
}

/// Copy an executable from `src` to `dst` without holding a writable fd to
/// `dst` in the calling process on Unix, avoiding `ETXTBSY` races across
/// parallel test threads.
pub fn copy_executable(src: &std::path::Path, dst: &std::path::Path) {
    if let Some(parent) = dst.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).unwrap();
    }
    #[cfg(unix)]
    {
        use std::process::{Command, Stdio};

        const COPY_SCRIPT: &str = r#"tmp="$2.tmp.$$"; cp -- "$1" "$tmp" && chmod -- 755 "$tmp" && mv -f -- "$tmp" "$2" || { s=$?; rm -f -- "$tmp"; exit "$s"; }"#;

        let status = Command::new("/bin/sh")
            .args(["-c", COPY_SCRIPT, "sh"])
            .arg(src)
            .arg(dst)
            .stdin(Stdio::null())
            .status()
            .unwrap_or_else(|e| panic!("spawning /bin/sh to copy {} to {}: {e}", src.display(), dst.display()));
        assert!(status.success(), "copying executable {} to {} failed: {status}", src.display(), dst.display());
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(src, dst).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn zero_delay_fakes_yield_so_joined_tasks_interleave() {
        use crate::github::GitHub;
        use futures_util::future::join;
        use std::sync::Arc;

        let t3 = FakeT3::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = {
            let log = log.clone();
            let t3 = &t3;
            async move {
                log.lock().unwrap().push("a:start");
                t3.call("echo", json!({})).await.unwrap();
                log.lock().unwrap().push("a:end");
            }
        };
        let b = {
            let log = log.clone();
            let t3 = &t3;
            async move {
                log.lock().unwrap().push("b:start");
                t3.call("echo", json!({})).await.unwrap();
                log.lock().unwrap().push("b:end");
            }
        };
        join(a, b).await;
        assert_eq!(*log.lock().unwrap(), vec!["a:start", "b:start", "a:end", "b:end"]);

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        let log = Arc::new(Mutex::new(Vec::new()));
        let vars = json!({"owner": "acme", "name": "widgets", "number": 7});
        let a = {
            let log = log.clone();
            let gh = &gh;
            let vars = vars.clone();
            async move {
                log.lock().unwrap().push("a:start");
                gh.graphql("query { repository { pullRequest { state } } }", vars).await.unwrap();
                log.lock().unwrap().push("a:end");
            }
        };
        let b = {
            let log = log.clone();
            let gh = &gh;
            async move {
                log.lock().unwrap().push("b:start");
                gh.graphql("query { repository { pullRequest { state } } }", vars).await.unwrap();
                log.lock().unwrap().push("b:end");
            }
        };
        join(a, b).await;
        assert_eq!(*log.lock().unwrap(), vec!["a:start", "b:start", "a:end", "b:end"]);
    }

    #[tokio::test]
    async fn a_rule_fires_on_the_nth_call_that_matches_its_op_and_target() {
        let t3 = FakeT3::new();
        t3.put(thread("mcp:a", "idle", "2026-11-17T12:00:00Z"));
        t3.put(thread("mcp:b", "idle", "2026-11-17T12:00:00Z"));
        t3.fault(Rule::on("t3_thread_read").target("mcp:a").call(2).fail(Failure::down("down")));
        let read = |id: &'static str| t3.call("t3_thread_read", json!({"threadId": id}));

        assert!(read("mcp:b").await.is_ok());
        assert!(read("mcp:a").await.is_ok());
        assert!(read("mcp:b").await.is_ok());
        assert!(matches!(read("mcp:a").await, Err(T3Error::Unavailable(m)) if m == "down"));
        assert!(read("mcp:a").await.is_ok());

        t3.fault(Rule::on("t3_thread_read").nth(Nth::From(2)).fail(Failure::refused("refused")));
        assert!(read("mcp:b").await.is_ok());
        assert!(matches!(read("mcp:b").await, Err(T3Error::Tool(m)) if m == "refused"));
        assert!(read("mcp:a").await.is_err());
        t3.clear_faults_on("t3_thread_read");
        assert!(read("mcp:a").await.is_ok());
    }

    #[tokio::test]
    async fn fail_after_effect_applies_the_call_then_returns_the_error() {
        use crate::github::GitHub;

        let t3 = FakeT3::new();
        t3.fault(Rule::on("t3_thread_send").call(1).fail_after_effect(Failure::down("lost reply")));

        let r = t3.call("t3_thread_send", json!({"threadId": "mcp:a", "message": "hi"})).await;

        assert!(matches!(r, Err(T3Error::Unavailable(_))), "{r:?}");
        assert_eq!(t3.sent().len(), 1);

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        let id = gh.pr("acme/widgets", 7)["id"].clone();
        gh.fault(Rule::on("mergePullRequest").pr("acme/widgets", 7).fail_after_effect(Failure::down("timeout")));
        let q = "mutation { mergePullRequest(input: {}) { clientMutationId } }";

        assert!(gh.graphql(q, json!({"id": id, "head": "1111111"})).await.is_err());
        assert_eq!(gh.pr("acme/widgets", 7)["state"], "MERGED");
    }

    #[tokio::test]
    async fn a_gate_parks_a_call_until_it_is_released() {
        let t3 = FakeT3::new();
        let gate = Gate::new();
        t3.fault(Rule::on("t3_thread_send").call(1).gate(&gate));
        let mut parked = Box::pin(t3.call("t3_thread_send", json!({"threadId": "mcp:a", "message": "hi"})));

        tokio::select! {
            _ = &mut parked => panic!("the gated call finished before its release"),
            _ = gate.arrived() => {}
        }
        assert!(t3.sent().is_empty());
        assert!(t3.call("t3_thread_send", json!({"threadId": "mcp:b", "message": "hi"})).await.is_ok());
        assert_eq!(t3.sent().len(), 1);
        gate.release();
        assert!(parked.await.is_ok());
        assert_eq!(t3.sent().len(), 2);
    }

    #[tokio::test]
    async fn a_gate_releases_a_parked_call_to_run_its_effect() {
        let t3 = FakeT3::new();
        let gate = Gate::new();
        t3.fault(Rule::on("t3_thread_send").gate(&gate));
        let send = t3.call("t3_thread_send", json!({"threadId": "mcp:a", "message": "hi"}));
        let release = async {
            gate.arrived().await;
            assert!(t3.sent().is_empty());
            gate.release();
        };

        let (r, ()) = futures_util::future::join(send, release).await;

        assert!(r.is_ok());
        assert_eq!(t3.sent().len(), 1);
        assert_eq!(gate.count(), 1);
    }

    #[tokio::test]
    async fn a_batch_read_fails_only_the_targeted_pull_request() {
        use crate::github::GitHub;

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        gh.set_pr("acme/widgets", 8, github_pr("2222222", "CLEAN", false));
        gh.fault(Rule::on("reconcile.pr").pr("acme/widgets", 8).call(1).fail(Failure::refused("transient")));
        let q = "query Reconcile($o0: String!) { }";
        let vars = json!({"o0": "acme", "r0": "widgets", "n0": 7, "o1": "acme", "r1": "widgets", "n1": 8});

        let first = gh.graphql(q, vars.clone()).await.unwrap();
        let second = gh.graphql(q, vars).await.unwrap();

        assert_eq!(first["data"]["p0"]["pullRequest"]["headRefOid"], "1111111");
        assert!(first["data"]["p1"]["pullRequest"].is_null());
        assert_eq!(first["errors"], json!([{"path": ["p1", "pullRequest"], "message": "transient"}]));
        assert_eq!(second["data"]["p1"]["pullRequest"]["headRefOid"], "2222222");
        assert!(second.get("errors").is_none());
    }

    #[tokio::test]
    async fn an_outage_wins_over_earlier_rules_and_spares_per_pr_counts() {
        use crate::github::GitHub;

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        gh.fault(Rule::on("reconcile").fail(Failure::refused("refused")));
        gh.fault(Rule::on("reconcile.pr").pr("acme/widgets", 7).call(1).fail(Failure::refused("transient")));
        gh.unreachable();
        let q = "query Reconcile($o0: String!) { }";
        let vars = json!({"o0": "acme", "r0": "widgets", "n0": 7});

        let down = gh.graphql(q, vars.clone()).await.unwrap_err();
        gh.clear_faults_on("*");
        gh.clear_faults_on("reconcile");
        let read = gh.graphql(q, vars).await.unwrap();

        assert_eq!(down.to_string(), GH_DOWN);
        assert_eq!(read["errors"], json!([{"path": ["p0", "pullRequest"], "message": "transient"}]));
    }

    #[tokio::test]
    async fn a_call_with_two_gates_arrives_at_both_before_either_releases() {
        let t3 = FakeT3::new();
        let (a, b) = (Gate::new(), Gate::new());
        t3.fault(Rule::on("t3_thread_send").gate(&a));
        t3.fault(Rule::on("t3_thread_send").target("mcp:a").gate(&b));
        let send = t3.call("t3_thread_send", json!({"threadId": "mcp:a", "message": "hi"}));
        let release = async {
            a.arrived().await;
            b.arrived().await;
            assert!(t3.sent().is_empty());
            b.release();
            a.release();
        };

        let (r, ()) = futures_util::future::join(send, release).await;

        assert!(r.is_ok());
        assert_eq!((a.count(), b.count()), (1, 1));
    }

    #[tokio::test]
    async fn a_batch_read_arrives_at_its_op_gate_and_pull_request_gate_before_waiting() {
        use crate::github::GitHub;

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        let (op, pr) = (Gate::new(), Gate::new());
        gh.fault(Rule::on("reconcile").gate(&op));
        gh.fault(Rule::on("reconcile.pr").pr("acme/widgets", 7).gate(&pr));
        let read = gh.graphql("query Reconcile($o0: String!) { }", json!({"o0": "acme", "r0": "widgets", "n0": 7}));
        let release = async {
            op.arrived().await;
            pr.arrived().await;
            pr.release();
            op.release();
        };

        let (r, ()) = futures_util::future::join(read, release).await;

        assert_eq!(r.unwrap()["data"]["p0"]["pullRequest"]["headRefOid"], "1111111");
    }

    #[tokio::test]
    async fn a_typed_failure_on_one_batched_pull_request_keeps_its_type() {
        use crate::github::GitHub;

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        gh.fault(Rule::on("reconcile.pr").pr("acme/widgets", 7).fail(Failure::typed("RATE_LIMITED", "slow down")));
        let vars = json!({"o0": "acme", "r0": "widgets", "n0": 7});

        let read = gh.graphql("query Reconcile($o0: String!) { }", vars).await.unwrap();

        assert_eq!(
            read["errors"],
            json!([{"path": ["p0", "pullRequest"], "message": "slow down", "type": "RATE_LIMITED"}])
        );
    }

    #[tokio::test]
    async fn a_pull_request_target_matches_across_a_repo_rename() {
        use crate::github::GitHub;

        let gh = FakeGitHub::default();
        gh.set_pr("acme/widgets", 7, github_pr("1111111", "CLEAN", false));
        gh.alias_repo("acme/gadgets", "acme/widgets");
        gh.fault(Rule::on("read").pr("acme/gadgets", 7).call(1).fail(Failure::refused("old name")));
        gh.fault(Rule::on("read").pr("acme/widgets", 7).call(2).fail(Failure::refused("new name")));
        let q = "query { repository { pullRequest { state } } }";
        let read = |name: &'static str| gh.graphql(q, json!({"owner": "acme", "name": name, "number": 7}));

        let first = read("widgets").await.unwrap();
        let second = read("gadgets").await.unwrap();

        assert_eq!(first["errors"][0]["message"], "old name");
        assert_eq!(second["errors"][0]["message"], "new name");
    }

    #[tokio::test]
    async fn seeded_jitter_yields_the_same_extra_steps_for_the_same_seed() {
        let mut a = fault::Faults::default();
        let mut b = fault::Faults::default();
        let mut c = fault::Faults::default();
        a.set_jitter(Some(7));
        b.set_jitter(Some(7));
        c.set_jitter(Some(8));
        let run = |f: &mut fault::Faults| (0..32).map(|_| f.yields()).collect::<Vec<_>>();

        let (ra, rb, rc) = (run(&mut a), run(&mut b), run(&mut c));

        assert_eq!(ra, rb);
        assert_ne!(ra, rc);
        assert!(ra.iter().all(|n| *n < 4) && ra.iter().any(|n| *n > 0));
        assert_eq!(run(&mut fault::Faults::default()), vec![0; 32]);
    }

    #[cfg(unix)]
    #[test]
    fn write_executable_and_copy_executable_preserve_content_and_exec() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("nested/script");
        let body = "#!/bin/sh\nprintf '%s\\n' '-n \\\\ hello'\n";
        write_executable(&script, body);
        assert_eq!(std::fs::read_to_string(&script).unwrap(), body);
        let out = Command::new(&script).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "-n \\\\ hello\n");

        // Overwrite an existing executable in place.
        write_executable(&script, "#!/bin/sh\nprintf 'v2'\n");
        let out = Command::new(&script).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "v2");

        // Large scripts (> 32 KiB) fall back to piping through stdin.
        let large_script = dir.path().join("large-script");
        let padding = "#".repeat(40_000);
        let large_body = format!("#!/bin/sh\n# {padding}\nprintf 'big-ok'\n");
        write_executable(&large_script, &large_body);
        assert_eq!(std::fs::read_to_string(&large_script).unwrap(), large_body);
        let out = Command::new(&large_script).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "big-ok");

        // Copy an executable and execute the copy.
        let copied = dir.path().join("copied/script");
        copy_executable(&script, &copied);
        let out = Command::new(&copied).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "v2");
    }
}
