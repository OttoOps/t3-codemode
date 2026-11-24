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
                if let Some(p) = args.get("projectId").filter(|v| !v.is_null()) {
                    th["projectId"] = p.clone();
                }
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
            tokio::task::yield_now().await;
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
    delay: Duration,
    repos: BTreeMap<String, Value>,
    repo_names: BTreeMap<String, String>,
    repo_aliases: BTreeMap<String, String>,
    repo_labels: BTreeMap<String, Vec<(String, String, String)>>,
    prs: BTreeMap<(String, u64), Value>,
    issues: BTreeMap<(String, u64), Value>,
    issue_page: usize,
    calls: Vec<Value>,
    errors: BTreeMap<String, (String, String)>,
    unlabelled_open_error: Option<String>,
    read_errors: BTreeMap<(String, u64), String>,
    field_errors: BTreeMap<(String, u64), (String, String)>,
    down: bool,
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
        pr["id"] = json!(format!("PR_{repo}_{number}"));
        if pr.get("headRefName").is_none() {
            pr["headRefName"] = json!(format!("feat/{number}"));
            pr["headRef"] = json!({"id": format!("REF_{repo}_{number}")});
            pr["isCrossRepository"] = json!(false);
        }
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

    pub fn fail_operation(&self, operation: &str, message: &str) {
        self.st().errors.insert(operation.into(), ("UNPROCESSABLE".into(), message.into()));
    }

    pub fn fail_operation_with_type(&self, operation: &str, err_type: &str, message: &str) {
        self.st().errors.insert(operation.into(), (err_type.into(), message.into()));
    }

    pub fn clear_operation_errors(&self) {
        self.st().errors.clear();
    }

    pub fn fail_unlabelled_open_issues(&self, message: Option<&str>) {
        self.st().unlabelled_open_error = message.map(String::from);
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
    if want_state == "OPEN"
        && label.is_none()
        && let Some(msg) = &st.unlabelled_open_error
    {
        return gh_error(msg);
    }
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
        .or(canonical.as_deref())
        .unwrap_or(&repo)
        .to_string();
    json!({"data": {"repository": {"nameWithOwner": name, "issues": {
        "pageInfo": {"hasNextPage": end < matching.len(), "endCursor": (end < matching.len()).then(|| end.to_string())},
        "nodes": nodes
    }}}})
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
            let mut st = self.st();
            let op = operation(query);
            let mut call = json!({"operation": op, "query": query, "variables": variables});
            if !headers.is_empty() {
                let hdrs: serde_json::Map<String, Value> =
                    headers.iter().map(|(k, v)| ((*k).to_string(), json!(v))).collect();
                call["headers"] = Value::Object(hdrs);
            }
            st.calls.push(call);
            if st.down {
                anyhow::bail!("gh api graphql failed: gh: not logged in");
            }
            if let Some((err_type, message)) = st.errors.get(op) {
                return Ok(gh_typed_error(err_type, message));
            }
            if op == "repoCanonical" {
                return Ok(repo_canonical(&st, &variables));
            }
            if op == "repoId" {
                return Ok(repo_id_query(&st, &variables));
            }
            if op == "issueNode" {
                return Ok(issue_node_query(&mut st, &variables));
            }
            if op == "repoLabel" {
                return Ok(repo_label_query(&st, &variables));
            }
            if op == "createLabel" {
                return Ok(create_label_mut(&mut st, &variables));
            }
            if op == "addLabelsToLabelable" {
                return Ok(add_labels_mut(&mut st, &variables));
            }
            if op == "addBlockedBy" {
                return Ok(add_blocked_by_mut(&mut st, &variables));
            }
            if op == "removeBlockedBy" {
                return Ok(remove_blocked_by_mut(&mut st, &variables));
            }
            if op == "addSubIssue" {
                return Ok(add_sub_issue_mut(&mut st, &variables));
            }
            if op == "removeSubIssue" {
                return Ok(remove_sub_issue_mut(&mut st, &variables));
            }
            if op == "createIssue" {
                return Ok(create_issue_mut(&mut st, &variables));
            }
            if op == "flowIssues" {
                return Ok(flow_issues(&st, query, &variables));
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
