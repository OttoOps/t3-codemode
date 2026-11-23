use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::t3::{BoxFut, T3Client, T3Error};
use crate::tokens::{RunTokens, TokenSource};
use crate::usage::{Usage, UsageSource};

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
    configured: Vec<Value>,
    forks: u64,
    sent: Vec<Value>,
    calls: Vec<String>,
    send_fails: bool,
    prs_fail: bool,
    list_fails: bool,
    projects_fail: bool,
    read_fails: bool,
    caps_fail: bool,
    search_fail: bool,
    pr_page: usize,
    short_list: usize,
    organize_fails: bool,
    launch_fails: bool,
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

pub fn pull_request(repository: &str, number: u64, state: &str) -> Value {
    json!({
        "host": "github.com", "repository": repository, "number": number,
        "url": format!("https://github.com/{repository}/pull/{number}"), "source": "agent",
        "watching": state == "open", "state": state, "title": format!("PR {number}"),
        "headBranch": format!("fix/{number}"), "baseBranch": "main", "isDraft": false, "stack": null
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
        self.st().threads.remove(id);
    }

    pub fn set_send_fails(&self, fails: bool) {
        self.st().send_fails = fails;
    }

    pub fn set_list_fails(&self, fails: bool) {
        self.st().list_fails = fails;
    }

    pub fn set_prs_fail(&self, fails: bool) {
        self.st().prs_fail = fails;
    }

    pub fn set_projects_fail(&self, fails: bool) {
        self.st().projects_fail = fails;
    }

    pub fn set_pr_page(&self, size: usize) {
        self.st().pr_page = size;
    }

    pub fn set_read_fails(&self, fails: bool) {
        self.st().read_fails = fails;
    }

    pub fn set_launch_fails(&self, fails: bool) {
        self.st().launch_fails = fails;
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

    pub fn set_caps_fail(&self, fails: bool) {
        self.st().caps_fail = fails;
    }

    pub fn set_search_fail(&self, fails: bool) {
        self.st().search_fail = fails;
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

    pub fn sent(&self) -> Vec<Value> {
        self.st().sent.clone()
    }

    pub fn configured(&self) -> Vec<Value> {
        self.st().configured.clone()
    }

    pub fn thread(&self, id: &str) -> Option<Value> {
        self.st().threads.get(id).cloned()
    }

    pub fn calls(&self) -> Vec<String> {
        self.st().calls.clone()
    }

    fn handle(&self, tool: &str, args: &Value) -> Result<Value, T3Error> {
        let mut st = self.st();
        st.calls.push(tool.to_string());
        match tool {
            "t3_thread_read" => {
                if st.read_fails {
                    return Err(T3Error::Unavailable("T3 unreachable (fake)".into()));
                }
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
                if st.send_fails {
                    return Err(T3Error::Unavailable("T3 unreachable (fake)".into()));
                }
                st.sent.push(args.clone());
                let n = st.sent.len();
                Ok(json!({"threadId": args["threadId"], "messageId": format!("msg:{n}"),
                          "runId": format!("run:{n}"), "status": "queued", "delivery": "queued"}))
            }
            "t3_thread_list" if st.list_fails => Err(T3Error::Unavailable("T3 unreachable (fake)".into())),
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
                if st.prs_fail {
                    return Err(T3Error::Unavailable("T3 unreachable (fake)".into()));
                }
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
            "t3_project_list" if st.projects_fail => Err(T3Error::Unavailable("T3 unreachable (fake)".into())),
            "t3_project_list" => Ok(json!({"projects": st.projects, "nextCursor": null})),
            "orchestrator_capabilities" if st.caps_fail || (st.search_fail && args["search"].is_string()) => {
                Err(T3Error::Unavailable("T3 unreachable (fake)".into()))
            }
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
            "t3_thread_launch" if st.launch_fails => Err(T3Error::Tool("t3_thread_launch: launch refused".into())),
            "t3_thread_launch" => {
                st.launched.push(args.clone());
                let id = format!("launch:{}", st.launched.len());
                let mut th = thread(&id, "running", "2026-11-17T12:00:00Z");
                th["title"] = args["title"].clone();
                th["providerInstanceId"] = args["modelSelection"]["instanceId"].clone();
                th["model"] = args["modelSelection"]["model"].clone();
                st.threads.insert(id.clone(), th);
                Ok(json!({"threadId": id, "link": format!("https://t3/{id}"), "projectId": args["projectId"],
                          "modelSelection": args["modelSelection"], "runId": format!("run:{id}:1"), "status": "running"}))
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

#[derive(Default)]
struct GhState {
    repos: BTreeMap<String, Value>,
    prs: BTreeMap<(String, u64), Value>,
    issues: BTreeMap<(String, u64), Value>,
    issue_page: usize,
    calls: Vec<Value>,
    errors: BTreeMap<String, String>,
    read_errors: BTreeMap<(String, u64), String>,
    field_errors: BTreeMap<(String, u64), (String, String)>,
    down: bool,
    queue: u64,
    merges: u64,
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
    json!({"number": number, "title": title, "state": "OPEN", "createdAt": created_at,
           "labels": {"nodes": labels}, "milestone": null, "blockers": [], "parentRef": null})
}

fn issue_key(repo: &str, number: u64) -> (String, u64) {
    (repo.to_ascii_lowercase(), number)
}

fn gh_error(message: &str) -> Value {
    json!({"data": null, "errors": [{"type": "UNPROCESSABLE", "message": message}]})
}

impl FakeGitHub {
    fn st(&self) -> std::sync::MutexGuard<'_, GhState> {
        self.0.lock().unwrap()
    }

    pub fn set_pr(&self, repo: &str, number: u64, pr: Value) {
        let mut st = self.st();
        let mut pr = pr;
        pr["id"] = json!(format!("PR_{repo}_{number}"));
        st.repos.entry(repo.into()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.prs.insert((repo.into(), number), pr);
    }

    pub fn update_pr(&self, repo: &str, number: u64, patch: Value) {
        let mut st = self.st();
        let pr = st.prs.get_mut(&(repo.into(), number)).expect("known pull request");
        for (k, v) in patch.as_object().expect("object patch") {
            pr[k] = v.clone();
        }
    }

    pub fn set_checks(&self, repo: &str, number: u64, checks: Value) {
        let head = self.pr(repo, number)["headRefOid"].as_str().unwrap_or_default().to_string();
        self.update_pr(repo, number, json!({"commits": head_commit(&head, checks)}));
    }

    pub fn pr(&self, repo: &str, number: u64) -> Value {
        self.st().prs[&(repo.into(), number)].clone()
    }

    pub fn remove_pr(&self, repo: &str, number: u64) {
        self.st().prs.remove(&(repo.into(), number));
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

    pub fn set_queue_length(&self, n: u64) {
        self.st().queue = n;
    }

    pub fn fail_operation(&self, operation: &str, message: &str) {
        self.st().errors.insert(operation.into(), message.into());
    }

    pub fn fail_read(&self, repo: &str, number: u64, message: &str) {
        self.st().read_errors.insert((repo.into(), number), message.into());
    }

    pub fn fail_field(&self, repo: &str, number: u64, field: &str, message: &str) {
        self.st().field_errors.insert((repo.into(), number), (field.into(), message.into()));
    }

    pub fn clear_read_errors(&self) {
        let mut st = self.st();
        st.read_errors.clear();
        st.field_errors.clear();
    }

    pub fn set_unknown_reads(&self, n: usize) {
        self.st().unknown_reads = n;
    }

    pub fn set_down(&self, down: bool) {
        self.st().down = down;
    }

    pub fn add_repo(&self, repo: &str) {
        self.st()
            .repos
            .entry(repo.to_ascii_lowercase())
            .or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
    }

    pub fn set_issue(&self, repo: &str, issue: Value) {
        let mut st = self.st();
        let mut issue = issue;
        let number = issue["number"].as_u64().expect("issue number");
        issue["nameWithOwner"] = json!(repo);
        st.repos.entry(repo.to_ascii_lowercase()).or_insert_with(|| json!({"viewerDefaultMergeMethod": "MERGE"}));
        st.issues.insert(issue_key(repo, number), issue);
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

fn operation(query: &str) -> &'static str {
    if query.starts_with("query Reconcile(") {
        return "reconcile";
    }
    if query.starts_with("query FlowIssues(") {
        return "flowIssues";
    }
    if query.starts_with(crate::flow::FLOW_TICKETS) {
        return "flowTickets";
    }
    for op in ["enqueuePullRequest", "mergePullRequest", "enablePullRequestAutoMerge"] {
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
    json!({"number": number, "title": i["title"], "url": format!("https://github.com/{name}/issues/{number}"),
           "createdAt": i["createdAt"], "state": i["state"], "labels": i["labels"], "milestone": i["milestone"],
           "parent": parent, "blockedBy": {"totalCount": blockers.len(), "nodes": blockers},
           "subIssues": {"totalCount": subs.len(), "nodes": subs}})
}

fn flow_issues(st: &GhState, variables: &Value) -> Value {
    let repo = format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())
        .to_ascii_lowercase();
    if !st.repos.contains_key(&repo) {
        return gh_error(&format!("Could not resolve to a Repository with the name '{repo}'."));
    }
    let label = variables["label"].as_str().unwrap().to_ascii_lowercase();
    let mut matching: Vec<&Value> = st
        .issues
        .iter()
        .filter(|((r, _), i)| {
            *r == repo
                && i["state"] == "OPEN"
                && i["labels"]["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["name"].as_str().is_some_and(|n| n.to_ascii_lowercase() == label))
        })
        .map(|(_, i)| i)
        .collect();
    matching.sort_by(|a, b| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()));
    let start: usize = variables["after"].as_str().and_then(|c| c.parse().ok()).unwrap_or(0);
    let size = if st.issue_page == 0 { 100 } else { st.issue_page };
    let end = (start + size).min(matching.len());
    let nodes: Vec<Value> = matching[start.min(end)..end].iter().map(|i| full_issue_node(st, i)).collect();
    let name = matching.first().and_then(|i| i["nameWithOwner"].as_str()).unwrap_or(&repo).to_string();
    json!({"data": {"repository": {"nameWithOwner": name, "issues": {
        "pageInfo": {"hasNextPage": end < matching.len(), "endCursor": (end < matching.len()).then(|| end.to_string())},
        "nodes": nodes
    }}}})
}

fn flow_tickets(st: &GhState, variables: &Value) -> Value {
    let mut data = serde_json::Map::new();
    let mut errs = Vec::new();
    for i in 0.. {
        let Some(owner) = variables[format!("o{i}")].as_str() else { break };
        let repo = format!("{owner}/{}", variables[format!("r{i}")].as_str().unwrap()).to_ascii_lowercase();
        let number = variables[format!("n{i}")].as_u64().unwrap();
        let alias = format!("t{i}");
        let issue = st.issues.get(&issue_key(&repo, number)).map(|i| full_issue_node(st, i));
        if issue.is_none() {
            errs.push(json!({"type": "NOT_FOUND", "path": [alias, "issue"], "message": "no issue"}));
        }
        let name =
            issue.as_ref().map_or(json!(repo), |_| st.issues[&issue_key(&repo, number)]["nameWithOwner"].clone());
        data.insert(alias, json!({"nameWithOwner": name, "issue": issue}));
    }
    let mut out = json!({"data": data});
    if !errs.is_empty() {
        out["errors"] = json!(errs);
    }
    out
}

impl crate::github::GitHub for FakeGitHub {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, anyhow::Result<Value>> {
        Box::pin(async move {
            let mut st = self.st();
            let op = operation(query);
            st.calls.push(json!({"operation": op, "query": query, "variables": variables}));
            if st.down {
                anyhow::bail!("gh api graphql failed: gh: not logged in");
            }
            if let Some(message) = st.errors.get(op) {
                return Ok(gh_error(message));
            }
            if op == "flowIssues" {
                return Ok(flow_issues(&st, &variables));
            }
            if op == "flowTickets" {
                return Ok(flow_tickets(&st, &variables));
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
                    if let Some(message) = st.read_errors.get(&(repo.clone(), number)) {
                        errs.push(json!({"path": [alias, "pullRequest"], "message": message}));
                        pr = Value::Null;
                    } else if let Some((field, message)) = st.field_errors.get(&(repo.clone(), number)) {
                        errs.push(json!({"path": [alias, "pullRequest", field], "message": message}));
                        pr[field.as_str()] = Value::Null;
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
                let mut repository = meta;
                repository["pullRequest"] = pr;
                return Ok(json!({"data": {"repository": repository}}));
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn write_executable_and_copy_executable_preserve_content_and_exec() {
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
