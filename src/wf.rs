use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::board::{
    BOARD, CardWorker, LaunchArgs, NO_BUTTONS, NewCard, Remotes, checked_identity, checked_options, identity_args,
    reply_options,
};
use crate::clock::{Clock, iso, local_clock, minutes_since};
use crate::codehost::CodeHost;
use crate::config::{CLOSING, Config, LinkParams, UP_KINDS};
use crate::github::GitHub;
use crate::issues::{FileIssuesArgs, IssueArgs};
use crate::merge::MergeArgs;
use crate::reach::{Reach, ReachPoint};
use crate::sentinels::{Sentinel, Sentinels};
use crate::store::{Bind, Identity, Link, Pending, PrBrief, Store};
use crate::t3::{T3Client, T3Error};
use crate::tokens::{T3Tokens, TokenSource};
use crate::usage::UsageSource;

pub const ACTIVE: &[&str] = &["preparing", "queued", "starting", "running"];
pub const ENDED_BADLY: &[&str] = &["failed", "interrupted", "cancelled"];
pub const WATCHER: &str = "t3cm";
const OPTION_KINDS: &[&str] = &["gate", "blocker", "report"];
pub const LAST_PASS: &str = "watcher_last_pass";
pub(crate) const NO_CALLER: &str = "wf needs the caller's T3 thread id: connect with the X-T3-Thread header \
(Claude Code: X-T3-Thread: ${T3CODE_THREAD_ID}) or pass `thread` to execute";

pub fn waiting_ids() -> Vec<String> {
    ['a', 'b', 'c', 'd'].iter().flat_map(|c| (1..=9).map(move |n| format!("{c}{n}"))).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitArgs {
    role: String,
    label: Option<String>,
    #[serde(default)]
    root: bool,
    issue: Option<String>,
    #[serde(rename = "parentIssue")]
    parent_issue: Option<String>,
    part: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenChildArgs {
    role: String,
    label: String,
    issue: Option<String>,
    #[serde(rename = "parentIssue")]
    parent_issue: Option<String>,
    part: Option<String>,
    pr: Option<PrBrief>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinArgs {
    parent: String,
    label: String,
    role: String,
    down: Option<i64>,
    miss: Option<i64>,
    stall: Option<i64>,
    issue: Option<String>,
    #[serde(rename = "parentIssue")]
    parent_issue: Option<String>,
    part: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendArgs {
    pub(crate) kind: String,
    pub(crate) body: String,
    pub(crate) gate: Option<String>,
    pub(crate) wake: Option<bool>,
    pub(crate) options: Option<Vec<String>>,
    #[serde(skip)]
    pub(crate) expect_pr: Option<Option<crate::store::LinkPr>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyArgs {
    to: String,
    body: String,
    #[serde(default)]
    reopen: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GateArgs {
    open: Option<String>,
    #[serde(default)]
    clear: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PauseArgs {
    child: Option<String>,
    paused: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildArgs {
    child: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandleArgs {
    handle: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct BoardArgs {
    #[serde(default)]
    all: bool,
    parent: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerArgs {
    id: String,
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CardsArgs {
    #[serde(default)]
    all: bool,
    context: Option<String>,
    projects: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct UsageStatsArgs {
    window: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FlowArgs {
    #[serde(default)]
    graph: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FeedArgs {
    limit: Option<usize>,
    context: Option<String>,
    projects: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PickArgs {
    pool: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PickedArgs {
    pick: i64,
    thread: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptArgs {
    from: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitAddArgs {
    text: String,
    from: Option<String>,
    link: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitResolveArgs {
    id: String,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn parse<T: DeserializeOwned>(method: &str, args: Value) -> Result<T> {
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args).map_err(|e| anyhow!("wf.{method}: {e}"))
}

pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub(crate) fn thread_is_busy(th: &Value) -> bool {
    let status = th["status"].as_str().unwrap_or("");
    ACTIVE.contains(&status)
        || status == "waiting"
        || truthy(&th["blockedOnThreadWait"])
        || truthy(&th["pendingBackgroundTasks"])
        || truthy(&th["pendingRequestCount"])
}

pub(crate) fn thread_is_stopped(th: &Value) -> bool {
    const TERMINAL: &[&str] = &["idle", "completed", "interrupted", "failed", "cancelled", "rolled_back"];
    let Some(obj) = th.as_object() else {
        return false;
    };
    let status = obj.get("status").and_then(Value::as_str).unwrap_or("");
    let no_wait = obj.get("blockedOnThreadWait").and_then(Value::as_bool) == Some(false);
    let no_bg = obj.get("pendingBackgroundTasks").and_then(Value::as_array).is_some_and(Vec::is_empty);
    let no_req = obj.get("pendingRequestCount").and_then(Value::as_u64) == Some(0);
    TERMINAL.contains(&status) && no_wait && no_bg && no_req
}

pub(crate) fn count(v: &Value) -> i64 {
    match v {
        Value::Array(a) => a.len() as i64,
        Value::Number(n) => n.as_i64().unwrap_or(0),
        other => truthy(other) as i64,
    }
}

fn quote_label(label: &str) -> String {
    if label.is_empty() || label.chars().any(|c| c.is_whitespace() || c == '"' || c == ']') {
        Value::String(label.into()).to_string()
    } else {
        label.into()
    }
}

pub fn header(kind: &str, from: &str, handle: &str, gate: Option<&str>) -> String {
    let gate = gate.map(|g| format!(" gate={g}")).unwrap_or_default();
    format!("[t3cm {kind} from={} msg={handle}{gate}]", quote_label(from))
}

pub fn join_target(message: &str) -> Option<(String, String)> {
    let json_string = |s: &str| {
        let mut it = serde_json::Deserializer::from_str(s).into_iter::<String>();
        let v = it.next()?.ok()?;
        Some((v, s[it.byte_offset()..].to_string()))
    };
    let (_, block) = message.rsplit_once("## t3-codemode link\n")?;
    let (_, rest) = block.split_once("async () => wf.join({ parent: ")?;
    let (parent, rest) = json_string(rest)?;
    let (label, _) = json_string(rest.strip_prefix(", label: ")?)?;
    Some((parent, label))
}

pub fn capped(body: &str, handle: &str, cap: usize) -> String {
    let n = body.chars().count();
    if n <= cap {
        return body.to_string();
    }
    let head: String = body.chars().take(cap).collect();
    format!("{}\n... ({} more chars; full: wf.show(\"{handle}\"))", head.trim_end(), n - cap)
}

pub const HANDLED: &str = "handled";
const RELAY_GATE: &str = "relay:";
const BLOCKER_GATE: &str = "relay:blocker";

fn relays(kind: &str, gate: Option<&str>) -> bool {
    let ticket_owned = |g: &str| g == "merge" || g.starts_with("delivery:");
    kind == "blocker" || (kind == "gate" && !gate.is_some_and(ticket_owned))
}

fn release_hold(ln: &mut Link) {
    if ln.gate.as_deref() == Some(BLOCKER_GATE) {
        ln.gate = None;
        ln.gate_since = None;
    }
}

fn gate_name(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join("_");
    if joined.is_empty() { "user".into() } else { joined }
}

pub struct Wf {
    pub(crate) store: Arc<Store>,
    pub(crate) t3: Arc<dyn T3Client>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: tokio::sync::watch::Sender<Arc<Config>>,
    pub(crate) lock: tokio::sync::Mutex<()>,
    pub(crate) remotes: Arc<dyn Remotes>,
    pub(crate) github: Arc<dyn GitHub>,
    pub(crate) host: Arc<dyn CodeHost>,
    pub(crate) live: Mutex<HashMap<String, (u64, Value)>>,
    pub(crate) live_seq: AtomicU64,
    pub(crate) closed_prs: Mutex<HashMap<String, ClosedPrs>>,
    pub(crate) known_projects: Mutex<HashMap<String, Value>>,
    pub(crate) repo_aliases: Mutex<HashMap<String, String>>,
    pub(crate) issue_titles: Mutex<HashMap<String, String>>,
    pub(crate) title_misses: Mutex<std::collections::HashSet<String>>,
    pub(crate) changes: tokio::sync::broadcast::Sender<()>,
    pub(crate) asks_lock: tokio::sync::Mutex<()>,
    pub(crate) pool_lock: tokio::sync::Mutex<()>,
    pub(crate) reconcile_lock: tokio::sync::Mutex<()>,
    pub(crate) flow_lock: tokio::sync::Mutex<()>,
    pub(crate) triage_lock: tokio::sync::Mutex<()>,
    pub(crate) issue_edit_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub(crate) flow_due: std::sync::atomic::AtomicBool,
    pub(crate) usage: Mutex<Option<Arc<dyn UsageSource>>>,
    pub(crate) tokens: Mutex<Arc<dyn TokenSource>>,
    pub(crate) ids: Mutex<Arc<dyn crate::board::IdSource>>,
    pub(crate) token_retry: Mutex<std::collections::HashSet<String>>,
    pub(crate) release: Mutex<Option<crate::release::Version>>,
    pub(crate) skills: crate::drift::Cache,
    pub(crate) slot_folders: Mutex<Option<crate::slots::Folders>>,
    pub(crate) drivers: Mutex<HashMap<String, String>>,
    pub(crate) config_path: Mutex<Option<PathBuf>>,
    pub(crate) config_lock: tokio::sync::Mutex<()>,
    pub(crate) models_rev: AtomicU64,
    pub(crate) models_session: String,
    pub(crate) state_rev: AtomicU64,
    pub(crate) context_writes: Mutex<()>,
    pub(crate) routing: Mutex<Option<Value>>,
    pub(crate) reach: Arc<Reach>,
    pub(crate) sentinels: Sentinels,
}

const MAX_PR_PAGES: usize = 20;
const PR_LIST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

type ClosedPrs = ((u64, u64), Value);

type Consumed = Vec<(String, Vec<String>)>;

struct Flush {
    sent: Vec<(String, Vec<String>)>,
    dropped: Vec<(String, Vec<String>)>,
    errors: Vec<String>,
}

impl Wf {
    pub fn new(
        store: Arc<Store>,
        t3: Arc<dyn T3Client>,
        clock: Arc<dyn Clock>,
        config: Arc<Config>,
        remotes: Arc<dyn Remotes>,
        github: Arc<dyn GitHub>,
        gitlab: Arc<dyn crate::gitlab::GitLab>,
    ) -> Wf {
        let reach = Arc::new(Reach::new());
        store.report_to(reach.clone());
        if store.node_role(BOARD).ok().flatten().as_deref() != Some("overseer") {
            let _ = store.node_set(BOARD, "overseer", &iso(clock.now()));
        }
        if store.meta_get(crate::reconcile::SINCE).ok().flatten().is_none() {
            let _ = store.meta_set(crate::reconcile::SINCE, &iso(clock.now()));
        }
        let rev = crate::board::clock_rev(&*clock);
        let config = tokio::sync::watch::Sender::new(config);
        let running = config.subscribe();
        let cli: crate::gitlab_host::CliFor =
            Arc::new(move |h: &str| crate::config::gitlab_cli(&running.borrow().forges, h).map(String::from));
        let host = crate::codehost::Router::new(
            crate::github_host::github_host(github.clone()),
            Arc::new(crate::gitlab_host::GitlabHost::new(gitlab, cli)),
        );
        Wf {
            store,
            t3,
            clock,
            config,
            lock: tokio::sync::Mutex::new(()),
            remotes,
            host: Arc::new(host),
            github,
            live: Mutex::new(HashMap::new()),
            live_seq: AtomicU64::new(0),
            closed_prs: Mutex::new(HashMap::new()),
            known_projects: Mutex::new(HashMap::new()),
            repo_aliases: Mutex::new(HashMap::new()),
            issue_titles: Mutex::new(HashMap::new()),
            title_misses: Mutex::new(std::collections::HashSet::new()),
            changes: tokio::sync::broadcast::channel(64).0,
            asks_lock: tokio::sync::Mutex::new(()),
            pool_lock: tokio::sync::Mutex::new(()),
            reconcile_lock: tokio::sync::Mutex::new(()),
            flow_lock: tokio::sync::Mutex::new(()),
            triage_lock: tokio::sync::Mutex::new(()),
            issue_edit_locks: Mutex::new(HashMap::new()),
            flow_due: std::sync::atomic::AtomicBool::new(false),
            usage: Mutex::new(None),
            tokens: Mutex::new(Arc::new(T3Tokens)),
            ids: Mutex::new(Arc::new(crate::board::RandomIds)),
            token_retry: Mutex::new(std::collections::HashSet::new()),
            release: Mutex::new(None),
            skills: crate::drift::Cache::default(),
            slot_folders: Mutex::new(None),
            drivers: Mutex::new(HashMap::new()),
            config_path: Mutex::new(None),
            config_lock: tokio::sync::Mutex::new(()),
            models_rev: AtomicU64::new(0),
            models_session: crate::board::random_id(),
            state_rev: AtomicU64::new(rev),
            context_writes: Mutex::new(()),
            routing: Mutex::new(None),
            reach,
            sentinels: Sentinels::new(),
        }
    }

    pub fn reach(&self) -> &Reach {
        &self.reach
    }

    #[cfg(feature = "test-sentinels")]
    pub fn sentinels(&self) -> &Sentinels {
        &self.sentinels
    }

    #[inline(always)]
    pub(crate) fn sentinel(&self, s: Sentinel) -> bool {
        self.sentinels.is_set(s)
    }

    pub(crate) fn retire_merge_wait(&self, w: &crate::store::MergeWait) -> Result<bool> {
        if self.sentinel(Sentinel::MergeWaitRemoveNotCas) {
            self.store.merge_wait_remove(&w.repo, w.pr)
        } else {
            self.store.merge_wait_retire(w)
        }
    }

    pub fn config(&self) -> Arc<Config> {
        self.config.borrow().clone()
    }

    pub fn config_changes(&self) -> tokio::sync::watch::Receiver<Arc<Config>> {
        self.config.subscribe()
    }

    pub fn set_config_path(&self, path: PathBuf) {
        *self.config_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.config_path.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(crate::config::default_path)
    }

    pub(crate) fn now(&self) -> String {
        iso(self.clock.now())
    }

    pub(crate) fn mins(&self, s: Option<&str>) -> Option<f64> {
        minutes_since(&*self.clock, s)
    }

    pub fn watcher_alive(&self) -> bool {
        let last = self.store.meta_get(LAST_PASS).ok().flatten();
        let limit = 3.0 * self.config().watcher_interval_secs.max(60) as f64 / 60.0;
        self.mins(last.as_deref()).is_some_and(|m| m <= limit)
    }

    fn watcher_line(&self) -> String {
        let last = self.store.meta_get(LAST_PASS).ok().flatten();
        match self.mins(last.as_deref()) {
            Some(m) if self.watcher_alive() => format!("running (last pass {m:.0} min ago)"),
            Some(m) => format!("stale (last pass {m:.0} min ago); check `{}`", crate::platform::SERVICE.status),
            None => "no pass recorded yet".into(),
        }
    }

    fn mark_pass(&self) -> Result<()> {
        self.store.meta_set(LAST_PASS, &self.now())
    }

    pub(crate) async fn read_thread(&self, id: &str) -> Result<Value, T3Error> {
        let seq = self.next_live_seq();
        match self.t3.call("t3_thread_read", json!({"threadId": id, "limit": 1, "maxCharsPerItem": 1})).await {
            Ok(r) => {
                let th = self.with_closed_prs(id, r["thread"].clone()).await;
                self.note_live_at(id, &th, seq);
                Ok(th)
            }
            Err(e) => {
                if e.is_not_found() {
                    self.note_live_at(id, &json!({"status": "gone"}), seq);
                }
                Err(e)
            }
        }
    }

    async fn with_closed_prs(&self, id: &str, mut th: Value) -> Value {
        let counts = th["pullRequestCount"].as_u64().zip(th["openPullRequestCount"].as_u64());
        let Some(counts) = counts.filter(|(all, open)| all > open) else {
            self.closed_prs.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            return th;
        };
        let cached = self.closed_prs.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
        let prs = match cached {
            Some((at, prs))
                if at == counts && prs.as_array().is_some_and(|a| a.iter().all(|p| p["state"] == "merged")) =>
            {
                prs
            }
            _ => match tokio::time::timeout(PR_LIST_DEADLINE, self.list_prs(id)).await.ok().flatten() {
                Some(prs) => {
                    self.closed_prs.lock().unwrap_or_else(|e| e.into_inner()).insert(id.into(), (counts, prs.clone()));
                    prs
                }
                None => {
                    th["pullRequestsStale"] = json!(true);
                    return th;
                }
            },
        };
        th["pullRequests"] = prs;
        th
    }

    async fn list_prs(&self, id: &str) -> Option<Value> {
        let mut prs = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PR_PAGES {
            let mut args = json!({"threadId": id});
            if let Some(c) = &cursor {
                args["cursor"] = json!(c);
            }
            let r = self.t3.call("list_thread_pull_requests", args).await.ok()?;
            prs.extend(r["pullRequests"].as_array()?.iter().cloned());
            cursor = r["nextCursor"].as_str().filter(|c| !c.is_empty()).map(String::from);
            if cursor.is_none() {
                return Some(json!(prs));
            }
        }
        None
    }

    pub async fn dispatch(&self, method: &str, caller: Option<&str>, args: Value) -> Result<Value> {
        self.dispatch_from(method, caller, &crate::wf_slots::Reported::default(), args).await
    }

    pub async fn dispatch_from(
        &self,
        method: &str,
        caller: Option<&str>,
        reported: &crate::wf_slots::Reported,
        args: Value,
    ) -> Result<Value> {
        if matches!(method, "slot" | "slots" | "renewal") {
            return self.slot_call(method, caller.filter(|c| !c.is_empty()), reported, args).await;
        }
        let out = match method {
            "pr" => self.pr_call(caller, reported, args).await,
            _ => self.dispatch_inner(method, caller, args).await,
        };
        if matches!(method, "send" | "close") {
            self.flow_if_due().await;
        }
        if out.is_ok()
            && !matches!(
                method,
                "show"
                    | "inbox"
                    | "board"
                    | "cards"
                    | "feed"
                    | "waiting.list"
                    | "pools"
                    | "usageStats"
                    | "status"
                    | "flow"
            )
        {
            self.changed();
        }
        out
    }

    async fn dispatch_inner(&self, method: &str, caller: Option<&str>, args: Value) -> Result<Value> {
        match method {
            "show" => return self.show(&parse::<HandleArgs>(method, args)?.handle),
            "cards" => {
                let a: CardsArgs = parse(method, args)?;
                let filter = self.project_filter(a.context.as_deref(), a.projects)?;
                return Ok(json!(self.cards_in(a.all, filter.as_ref())?));
            }
            "feed" => {
                let a: FeedArgs = parse(method, args)?;
                let filter = self.project_filter(a.context.as_deref(), a.projects)?;
                return Ok(json!(self.feed_in(a.limit.unwrap_or(self.config().board.feed), filter.as_ref())?));
            }
            "answer" => {
                let a: AnswerArgs = parse(method, args)?;
                return self.answer(&a.id, &a.text).await;
            }
            "pick" => return Ok(serde_json::to_value(self.pick(&parse::<PickArgs>(method, args)?.pool).await?)?),
            "picked" => {
                let a: PickedArgs = parse(method, args)?;
                return self.picked(a.pick, &a.thread);
            }
            "pools" => {
                parse::<Empty>(method, args)?;
                return self.pools().await;
            }
            "usageStats" => {
                let a: UsageStatsArgs = parse(method, args)?;
                return self.usage_stats(a.window.as_deref()).await;
            }
            "flow" => {
                let a: FlowArgs = parse(method, args)?;
                return self.flow(a.graph).await;
            }
            "flow.next" => {
                parse::<Empty>(method, args)?;
                return self.flow_next().await;
            }
            "status" => {
                parse::<Empty>(method, args)?;
                self.refresh_skills();
                return Ok(self.service_view());
            }
            "merge" => return self.merge(caller.filter(|c| !c.is_empty()), parse::<MergeArgs>(method, args)?).await,
            "fileIssues" => {
                return self
                    .file_issues(caller.filter(|c| !c.is_empty()), parse::<FileIssuesArgs>(method, args)?)
                    .await;
            }
            "launchTicket" => {
                let r = self.launch_ticket(parse::<LaunchArgs>(method, args)?).await?;
                if r["ok"] == false {
                    bail!("{} (recorded as board card {})", r["error"].as_str().unwrap_or("launch failed"), r["card"]);
                }
                return Ok(r);
            }
            "dispatch" => {
                let a: crate::board::DispatchArgs = parse(method, args)?;
                let r = self.dispatch_request(caller, a).await?;
                if r["ok"] == false {
                    bail!(
                        "{} (recorded as board card {})",
                        r["error"].as_str().unwrap_or("dispatch failed"),
                        r["card"]
                    );
                }
                return Ok(r);
            }
            _ => {}
        }
        let board_args = if method == "board" { Some(parse::<BoardArgs>(method, args.clone())?) } else { None };
        if let Some(BoardArgs { all, parent: Some(root) }) = board_args {
            return self.board_view(&root, all).await;
        }
        let me = caller.filter(|c| !c.is_empty()).ok_or_else(|| anyhow!(NO_CALLER))?;
        match method {
            "init" => self.init(me, parse(method, args)?).await,
            "openChild" => {
                let a: OpenChildArgs = parse(method, args)?;
                let id =
                    checked_identity("openChild", a.issue.as_deref(), a.parent_issue.as_deref(), a.part.as_deref())?;
                let brief = a.pr.map(crate::wf_pr::checked_brief).transpose()?;
                self.open_child(me, &a.role, &a.label, id, brief).await
            }
            "join" => self.join(me, parse(method, args)?).await,
            "issue" => self.report_issue(me, parse::<IssueArgs>(method, args)?).await,
            "send" => self.send(me, parse(method, args)?).await,
            "reply" => self.reply(me, parse(method, args)?).await,
            "gate" => self.gate(me, parse(method, args)?).await,
            "pause" => self.pause(me, parse(method, args)?).await,
            "close" => self.close(me, &parse::<ChildArgs>(method, args)?.child).await,
            "inbox" => {
                parse::<Empty>(method, args)?;
                self.inbox(me).await
            }
            "board" => self.board_of(me, board_args.unwrap_or_default().all).await,
            "adopt" => self.adopt(me, &parse::<AdoptArgs>(method, args)?.from).await,
            "sweep" => {
                parse::<Empty>(method, args)?;
                self.sweep(me).await
            }
            "waiting.add" => {
                let a: WaitAddArgs = parse(method, args)?;
                self.waiting_add(me, &a.text, a.from, a.link)
            }
            "waiting.resolve" => {
                let a: WaitResolveArgs = parse(method, args)?;
                self.waiting_resolve(me, &a.id, a.note.as_deref())
            }
            "waiting.list" => {
                parse::<Empty>(method, args)?;
                Ok(json!(self.store.waiting_open(me)?))
            }
            other => bail!("wf has no method {other}; search the catalog with kind \"wf\""),
        }
    }

    async fn init(&self, me: &str, a: InitArgs) -> Result<Value> {
        let config = self.config();
        config.check_role(&a.role)?;
        let id = checked_identity("init", a.issue.as_deref(), a.parent_issue.as_deref(), a.part.as_deref())?;
        let guard = self.lock.lock().await;
        let open = self.store.link_get(me)?.filter(|l| !l.closed);
        if !a.root && open.is_none() && config.board_oversees(&a.role) {
            drop(guard);
            let thread = self.read_thread(me).await.ok();
            let usable = |l: &str| Some(l.trim().to_string()).filter(|l| !l.is_empty());
            let label = a
                .label
                .as_deref()
                .and_then(usable)
                .or_else(|| thread.as_ref().and_then(|t| t["title"].as_str()).and_then(usable))
                .unwrap_or_else(|| me.to_string());
            let args = JoinArgs {
                parent: BOARD.into(),
                label: label.clone(),
                role: a.role.clone(),
                down: None,
                miss: None,
                stall: None,
                issue: id.issue,
                parent_issue: id.parent_issue,
                part: id.part,
            };
            let joined = self.bind(me, args, thread.as_ref(), true).await?;
            return Ok(json!({"thread": me, "role": a.role, "root": false, "parent": joined["parent"],
                "label": joined["label"], "watcher": self.watcher_line()}));
        }
        self.store.node_set(me, &a.role, &self.now())?;
        let mut out = json!({"thread": me, "role": a.role, "root": open.is_none(), "watcher": self.watcher_line()});
        if let Some(l) = open {
            out["parent"] = json!(l.parent);
            out["label"] = json!(l.label);
        }
        Ok(out)
    }

    async fn open_child(
        &self,
        me: &str,
        role: &str,
        label: &str,
        identity: Identity,
        brief: Option<PrBrief>,
    ) -> Result<Value> {
        if label.trim().is_empty() {
            bail!("wf.openChild: label must not be empty");
        }
        let _g = self.lock.lock().await;
        let my_role = self
            .store
            .node_role(me)?
            .ok_or_else(|| anyhow!("call wf.init({{role}}) (no parent) or your brief's wf.join(...) first"))?;
        let params = self.config().link_params(&my_role, role)?;
        self.store.pending_put(&Pending {
            parent: me.into(),
            label: label.into(),
            role: role.into(),
            params,
            at: self.now(),
            reported: false,
            thread: None,
            link: None,
            project: None,
            identity: identity.clone(),
            brief,
        })?;
        let call = format!(
            "async () => wf.join({{ parent: {}, label: {}, role: {}, down: {}, miss: {}, stall: {}{} }})",
            json!(me),
            json!(label),
            json!(role),
            params.down,
            params.miss,
            params.stall,
            identity_args(&identity)
        );
        let brief = format!(
            "## t3-codemode link\n\
             This thread is a child on a t3-codemode link. Before anything else, call the t3cm `execute` tool with:\n\
             {call}\n\
             Report routine progress with wf.send({{ kind: \"note\", body }}) (it never wakes the parent); \
             event, gate, blocker and handoff wake it. Finish with wf.send({{ kind: \"handoff\", body }})."
        );
        Ok(json!({"brief": brief, "params": params}))
    }

    pub async fn note_launch(&self, parent: &str, label: &str, launched: &Value) -> Result<()> {
        let Some(thread) = launched["threadId"].as_str() else {
            return Ok(());
        };
        let _g = self.lock.lock().await;
        let row = self.store.pending_all()?.into_iter().find(|p| p.parent == parent && p.label == label);
        if let Some(mut p) = row.filter(|p| p.thread.is_none()) {
            p.thread = Some(thread.into());
            p.link = launched["link"].as_str().map(String::from);
            self.store.pending_put(&p)?;
        }
        Ok(())
    }

    async fn join(&self, me: &str, a: JoinArgs) -> Result<Value> {
        self.config().check_role(&a.role)?;
        checked_identity("join", a.issue.as_deref(), a.parent_issue.as_deref(), a.part.as_deref())?;
        let thread = self.read_thread(me).await.ok();
        self.bind(me, a, thread.as_ref(), false).await
    }

    async fn bind(&self, me: &str, a: JoinArgs, thread: Option<&Value>, auto: bool) -> Result<Value> {
        let field = |k: &str| thread.and_then(|t| t[k].as_str()).map(String::from);
        let (link_url, project) = (field("link"), field("projectId"));
        let given = checked_identity("join", a.issue.as_deref(), a.parent_issue.as_deref(), a.part.as_deref())?;
        let _g = self.lock.lock().await;
        let parent_role = self.store.node_role(&a.parent)?;
        let config = self.config();
        let base = parent_role.as_deref().and_then(|r| config.link_params(r, &a.role).ok()).unwrap_or(config.link);
        let params = LinkParams {
            down: a.down.unwrap_or(base.down),
            miss: a.miss.unwrap_or(base.miss),
            stall: a.stall.unwrap_or(base.stall),
        };
        params.validate().map_err(|e| anyhow!("the brief's link numbers are invalid: {e}"))?;
        let now = self.now();
        let (reserved, reserved_brief) = match auto {
            true => (Identity::default(), None),
            false => self
                .store
                .pending_all()?
                .into_iter()
                .find(|p| p.parent == a.parent && p.label == a.label)
                .map(|p| (p.identity, p.brief))
                .unwrap_or_default(),
        };
        let launched_by = auto.then_some(BOARD);
        let bound = self.store.link_bind_launched(me, !auto, launched_by, |current, launch| {
            if auto && let Some(l) = current.as_ref().filter(|l| !l.closed) {
                return Bind::Kept(l.clone());
            }
            let old = current.filter(|l| l.parent == a.parent);
            let identity = given.clone().or(launch.map(|p| p.identity.clone()).unwrap_or_default()).or(match &old {
                Some(o) => o.identity.clone(),
                None => reserved.clone(),
            });
            let brief = launch
                .and_then(|p| p.brief.clone())
                .or_else(|| reserved_brief.clone())
                .or_else(|| old.as_ref().and_then(|o| o.brief.clone()));
            Bind::Saved(Link {
                child: me.into(),
                parent: a.parent.clone(),
                label: launch.map(|p| p.label.clone()).unwrap_or_else(|| a.label.clone()),
                role: a.role.clone(),
                parent_role,
                params,
                link: link_url.or_else(|| old.as_ref().and_then(|o| o.link.clone())),
                opened: old.as_ref().map(|o| o.opened.clone()).unwrap_or_else(|| now.clone()),
                last_up: now.clone(),
                last_seen: None,
                last_nudge: None,
                nudges: 0,
                escalated: false,
                reported: old.as_ref().filter(|_| !auto).map(|o| o.reported.clone()).unwrap_or_default(),
                gate: old.as_ref().filter(|_| !auto).and_then(|o| o.gate.clone()),
                gate_since: old.as_ref().filter(|_| !auto).and_then(|o| o.gate_since.clone()),
                gate_pr: old.as_ref().filter(|_| !auto).and_then(|o| o.gate_pr.clone()),
                closed: false,
                closed_at: None,
                closed_how: None,
                unread: old.as_ref().map(|o| o.unread.clone()).unwrap_or_default(),
                bad_run: None,
                settled_at: None,
                restored_at: None,
                base: Default::default(),
                project: project
                    .or_else(|| launch.and_then(|p| p.project.clone()))
                    .or_else(|| old.as_ref().and_then(|o| o.project.clone()))
                    .or_else(|| self.live_of(me).and_then(|v| v["project"].as_str().map(String::from))),
                paused_at: old.as_ref().filter(|_| !auto).and_then(|o| o.paused_at.clone()),
                identity,
                brief,
                pr: old.as_ref().and_then(|o| o.pr.clone()),
            })
        })?;
        let ln = match bound {
            Bind::Kept(l) => return Ok(json!({"parent": l.parent, "label": l.label})),
            Bind::Saved(l) => l,
        };
        Ok(json!({
            "parent": ln.parent, "label": ln.label, "params": params,
            "message": format!("Joined. The watcher nudges this thread after {} idle minutes with no gate open; \
                there are no timers or check-ins to keep. Routine progress: wf.send({{kind: \"note\"}}).", params.down)
        }))
    }

    pub(crate) async fn send(&self, me: &str, a: SendArgs) -> Result<Value> {
        if !UP_KINDS.contains(&a.kind.as_str()) {
            bail!("wf.send: kind must be one of {}", UP_KINDS.join(", "));
        }
        if a.body.trim().is_empty() {
            bail!("wf.send: empty body");
        }
        let options = match &a.options {
            Some(_) if !OPTION_KINDS.contains(&a.kind.as_str()) => {
                bail!("wf.send: options apply only to kinds {}", OPTION_KINDS.join(", "))
            }
            Some(o) => Some(checked_options(o)?),
            None => None,
        };
        let _g = self.lock.lock().await;
        let mut ln = self
            .store
            .link_get(me)?
            .ok_or_else(|| anyhow!("this thread has no parent link; it reports to the user directly"))?;
        if ln.closed {
            if a.expect_pr.is_some() {
                return Ok(json!({"status": "superseded", "detail": "the link closed while t3cm checked the PR"}));
            }
            bail!("the link to your parent is closed; nothing to send on it");
        }
        if a.expect_pr.as_ref().is_some_and(|seen| ln.pr != *seen) {
            return Ok(json!({"status": "superseded", "detail": "a newer head of this PR was recorded first"}));
        }
        if a.expect_pr.is_some()
            && let Some(p) = ln.pr.as_mut()
        {
            p.waiting = false;
        }
        if CLOSING.contains(&a.kind.as_str())
            && let Err(e) = self.read_thread(me).await
            && !e.is_not_found()
        {
            self.live.lock().unwrap_or_else(|e| e.into_inner()).remove(me);
        }
        let now = self.now();
        if a.kind == "gate" {
            let name = gate_name(a.gate.as_deref().unwrap_or("user"));
            ln.gate_pr = if ln.parent == BOARD && name == "merge" {
                crate::reconcile::card_pr(&a.body, &self.config().forges).map(|p| (p.repo, p.number))
            } else {
                None
            };
            ln.gate = Some(name);
            ln.gate_since = Some(now.clone());
        }
        let handle = self.store.msg_new(me, &ln.parent, &a.kind, &a.body, &now)?;
        ln.last_up = now.clone();
        ln.nudges = 0;
        ln.escalated = false;
        let policy = self.config().link_policy(ln.parent_role.as_deref(), &ln.role);
        let mut wake = a.wake.unwrap_or_else(|| policy.wake.contains(&a.kind));
        let closing = CLOSING.contains(&a.kind.as_str());
        if closing && ln.parent != BOARD {
            self.drop_relayed(&ln.child, &ln.parent, "the worker finished")?;
        }
        if a.kind == "gate" && ln.parent != BOARD && !relays(&a.kind, ln.gate.as_deref()) {
            let worker = ln.child.as_str();
            let older =
                |c: &crate::board::Card| c.kind == "gate" && c.worker.as_ref().is_some_and(|w| w.thread == worker);
            if !self.resolve_cards(older, "superseded by a newer gate")?.is_empty() {
                self.sync_relay_gate(&ln.parent)?;
            }
        }
        if wake
            && !policy.relay
            && relays(&a.kind, ln.gate.as_deref())
            && self.store.link_get(&ln.parent).ok().flatten().is_some_and(|t| !t.closed && t.parent == BOARD)
        {
            self.reach.hit(ReachPoint::RelayOffFallback);
        }
        if wake
            && policy.relay
            && relays(&a.kind, ln.gate.as_deref())
            && let Some(ticket) = self.store.link_get(&ln.parent)?.filter(|t| !t.closed && t.parent == BOARD)
        {
            return self.relay_up(ln, ticket, &a.kind, &handle, &a.body, options);
        }
        if closing {
            ln.closed = true;
            ln.closed_at = Some(now.clone());
            ln.closed_how = Some(a.kind.clone());
            ln.gate = None;
            ln.gate_since = None;
            ln.gate_pr = None;
            wake = true;
        }
        if ln.parent == BOARD {
            if closing {
                self.reach.hit(ReachPoint::TicketClosed);
                self.flow_soon();
            }
            self.store.link_save(&ln)?;
            if closing && !self.sentinel(Sentinel::OutboxNoPurge) && self.store.outbox_drop_to(&ln.child)? > 0 {
                self.reach.hit(ReachPoint::OutboxHeldThenClosed);
            }
            return self.board_receive(&ln, &a.kind, &handle, &a.body, options);
        }
        if !wake {
            ln.unread.push(handle.clone());
            self.store.link_save(&ln)?;
            return Ok(json!({"handle": handle, "delivery": "noted",
                "detail": "no wake; your parent sees it in the digest of its next wake"}));
        }
        let text = format!(
            "{}\n{}",
            header(&a.kind, &ln.label, &handle, ln.gate.as_deref()),
            capped(&a.body, &handle, policy.body_cap)
        );
        if !closing {
            self.store.link_save(&ln)?;
            return self.send_wake(&ln.parent, &text, &handle).await;
        }
        let purge = !self.sentinel(Sentinel::OutboxNoPurge);
        if self.store.link_close(&ln, purge, Some((&ln.parent, &text, &handle)), &now)? > 0 {
            self.reach.hit(ReachPoint::OutboxHeldThenClosed);
        }
        let mut out = self.deliver_queued(&ln.parent, &handle).await?;
        out["closed"] = json!(true);
        Ok(out)
    }

    fn relay_up(
        &self,
        mut ln: Link,
        ticket: Link,
        kind: &str,
        handle: &str,
        body: &str,
        options: Option<Vec<String>>,
    ) -> Result<Value> {
        let now = self.now();
        let gate = ln.gate.clone().filter(|_| kind == "gate").unwrap_or_else(|| kind.to_string());
        let worker = CardWorker { thread: ln.child.clone(), label: ln.label.clone(), link: ln.link.clone(), gate };
        let options = options.unwrap_or_else(|| reply_options(body));
        let card = self.board_card(NewCard {
            kind,
            label: Some(&ticket.label),
            ticket: Some(&ticket.child),
            link: ln.link.as_deref().or(ticket.link.as_deref()),
            body,
            handle: Some(handle),
            options: Some(options.clone()),
            pr: None,
            approval: None,
            offer: None,
            worker: Some(&worker),
        })?;
        let what = if kind == "gate" { format!("{} gate", worker.gate) } else { kind.to_string() };
        let note = format!(
            "{what} from {} ({handle}) went straight to the board as card {card}: {}",
            ln.label,
            crate::t3::truncate_chars(&body.split_whitespace().collect::<Vec<_>>().join(" "), 120)
        );
        let h = self.store.msg_new(WATCHER, &ticket.child, HANDLED, &note, &now)?;
        ln.unread.push(h);
        if kind == "blocker" && ln.gate.is_none() {
            ln.gate = Some(BLOCKER_GATE.into());
            ln.gate_since = Some(now);
        }
        self.store.link_save(&ln)?;
        self.sync_relay_gate(&ticket.child)?;
        let mut out = json!({"handle": handle, "delivery": "board", "card": card, "options": options,
            "detail": format!("on the user's board as {card}, past your parent; the answer arrives here as a [t3cm reply]")});
        if kind == "gate" && options.is_empty() {
            out["warning"] = json!(NO_BUTTONS);
        }
        Ok(out)
    }

    pub(crate) fn sync_merge_gate_pr(&self, ticket: &str) -> Result<()> {
        let Some(mut t) = self.store.link_get(ticket)?.filter(|t| !t.closed && t.gate.as_deref() == Some("merge"))
        else {
            return Ok(());
        };
        let Some((repo, number)) = &t.gate_pr else { return Ok(()) };
        let cards = self.cards()?;
        let own =
            |c: &crate::board::Card| c.kind == "gate" && c.worker.is_none() && c.ticket.as_deref() == Some(ticket);
        if cards.iter().any(|c| own(c) && c.pr.as_ref().is_some_and(|p| p.repo == *repo && p.number == *number)) {
            return Ok(());
        }
        t.gate_pr = cards
            .iter()
            .filter(|c| own(c))
            .find_map(|c| c.pr.as_ref().filter(|p| p.ready).map(|p| (p.repo.clone(), p.number)));
        self.store.link_save(&t)
    }

    pub(crate) fn sync_relay_gate(&self, ticket: &str) -> Result<()> {
        let Some(mut t) = self.store.link_get(ticket)?.filter(|t| !t.closed) else { return Ok(()) };
        if t.gate.as_deref().is_some_and(|g| !g.starts_with(RELAY_GATE)) {
            return Ok(());
        }
        let cards = self.cards()?;
        let held = cards.iter().rev().filter(|c| c.ticket.as_deref() == Some(ticket)).find_map(|c| c.worker.as_ref());
        let want = held.map(|w| format!("{RELAY_GATE}{}", w.gate));
        if t.gate != want {
            t.gate_since = want.as_ref().map(|_| self.now());
            t.gate = want;
            t.nudges = 0;
            t.escalated = false;
            self.store.link_save(&t)?;
        }
        Ok(())
    }

    pub(crate) fn release_relayed(&self, card: &crate::board::Card) -> Result<()> {
        let (Some(w), Some(ticket)) = (&card.worker, &card.ticket) else { return Ok(()) };
        if card.kind == "blocker"
            && let Some(mut wl) = self.store.link_get(&w.thread)?
            && wl.gate.as_deref() == Some(BLOCKER_GATE)
            && !self
                .cards()?
                .iter()
                .any(|c| c.kind == "blocker" && c.worker.as_ref().is_some_and(|o| o.thread == w.thread))
        {
            wl.gate = None;
            wl.gate_since = None;
            self.store.link_save(&wl)?;
        }
        self.sync_relay_gate(ticket)
    }

    pub(crate) fn release_worker_holds(&self, ticket: &str) -> Result<()> {
        for mut w in self.store.children(ticket, true)? {
            if w.gate.as_deref() == Some(BLOCKER_GATE) {
                release_hold(&mut w);
                self.store.link_save(&w)?;
            }
        }
        Ok(())
    }

    fn drop_relayed(&self, worker: &str, ticket: &str, note: &str) -> Result<()> {
        let mine = |c: &crate::board::Card| c.worker.as_ref().is_some_and(|w| w.thread == worker);
        if !self.resolve_cards(mine, note)?.is_empty() {
            self.sync_relay_gate(ticket)?;
        }
        Ok(())
    }

    async fn reply(&self, me: &str, a: ReplyArgs) -> Result<Value> {
        if a.body.trim().is_empty() {
            bail!("wf.reply: empty body");
        }
        let _g = self.lock.lock().await;
        let ln = self.find_child(me, &a.to)?;
        if ln.closed && !a.reopen {
            bail!("the link to {} is closed; pass reopen: true to send anyway (it starts a new turn there)", ln.label);
        }
        let handle = self.store.msg_new(me, &ln.child, "reply", &a.body, &self.now())?;
        let text = format!(
            "{}\n{}",
            header("reply", "parent", &handle, None),
            capped(&a.body, &handle, self.config().reply_cap)
        );
        let allow_closed = ln.closed && a.reopen;
        if allow_closed {
            self.reach.hit(ReachPoint::ReplyReopenedClosed);
        }
        self.send_wake_inner(&ln.child, &text, &handle, allow_closed).await
    }

    async fn gate(&self, me: &str, a: GateArgs) -> Result<Value> {
        if a.clear == a.open.is_some() {
            bail!("wf.gate takes {{open: \"<what>\"}} or {{clear: true}}");
        }
        let _g = self.lock.lock().await;
        let mut ln = self
            .store
            .link_get(me)?
            .filter(|l| !l.closed)
            .ok_or_else(|| anyhow!("this thread has no open parent link to hold a gate on"))?;
        let was_merge = ln.gate.as_deref() == Some("merge");
        let prev_pr = ln.gate_pr.clone();
        ln.gate = a.open.as_deref().map(gate_name);
        ln.gate_since = ln.gate.as_ref().map(|_| self.now());
        ln.gate_pr = None;
        ln.nudges = 0;
        ln.escalated = false;
        self.store.link_save(&ln)?;
        if a.clear && ln.parent == BOARD {
            self.resolve_cards(
                |c| {
                    let is_ready = c.pr.as_ref().is_some_and(|p| p.ready);
                    let matches_gate_pr = was_merge
                        && prev_pr
                            .as_ref()
                            .is_some_and(|(r, n)| c.pr.as_ref().is_some_and(|p| p.repo == *r && p.number == *n));
                    c.kind == "gate"
                        && c.worker.is_none()
                        && c.ticket.as_deref() == Some(ln.child.as_str())
                        && (self.sentinel(Sentinel::GateClearWipesOtherReady) || !is_ready || matches_gate_pr)
                },
                "gate cleared in the ticket",
            )?;
            let next_ready = self.cards()?.into_iter().find_map(|c| {
                (c.kind == "gate" && c.worker.is_none() && c.ticket.as_deref() == Some(ln.child.as_str()))
                    .then(|| c.pr.filter(|p| p.ready).map(|p| (p.repo, p.number)))
                    .flatten()
            });
            if let Some(next_pr) = next_ready {
                self.reach.hit(ReachPoint::GateClearPreservedReadyCard);
                ln.gate = Some("merge".into());
                ln.gate_since = Some(self.now());
                ln.gate_pr = Some(next_pr);
                self.store.link_save(&ln)?;
            } else {
                self.sync_relay_gate(&ln.child)?;
            }
        } else if a.clear {
            self.drop_relayed(&ln.child, &ln.parent, "gate cleared by the worker")?;
        }
        let gate = self.store.link_get(me)?.and_then(|l| l.gate);
        Ok(json!({"gate": gate}))
    }

    async fn pause(&self, me: &str, a: PauseArgs) -> Result<Value> {
        let _g = self.lock.lock().await;
        let mut ln = match a.child.as_deref() {
            Some(child) => self.find_child(me, child)?,
            None => self
                .store
                .link_get(me)?
                .ok_or_else(|| anyhow!("this thread has no parent link to pause; pass {{child}} to pause a child"))?,
        };
        if ln.closed {
            bail!("{}'s link is closed; there is nothing to pause", ln.label);
        }
        self.set_paused(&mut ln, a.paused.unwrap_or(true));
        self.store.link_save(&ln)?;
        Ok(json!({"label": ln.label, "paused": ln.paused_at.is_some()}))
    }

    pub(crate) fn set_paused(&self, ln: &mut Link, paused: bool) {
        if paused == ln.paused_at.is_some() {
            return;
        }
        ln.paused_at = paused.then(|| self.now());
        ln.nudges = 0;
        ln.escalated = false;
        ln.last_nudge = None;
    }

    fn find_child(&self, me: &str, reference: &str) -> Result<Link> {
        self.store
            .children(me, false)?
            .into_iter()
            .rev()
            .find(|l| l.child == reference || l.label == reference)
            .ok_or_else(|| anyhow!("{reference:?} is not a child link of this thread; wf.board() lists them"))
    }

    async fn close(&self, me: &str, child: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let mut ln = self.find_child(me, child)?;
        ln.closed = true;
        ln.closed_at = Some(self.now());
        ln.closed_how = Some("closed-by-parent".into());
        if !self.sentinel(Sentinel::CloseKeepsGate) {
            ln.gate = None;
            ln.gate_since = None;
            ln.gate_pr = None;
        }
        self.store.link_save(&ln)?;
        if !self.sentinel(Sentinel::OutboxNoPurge) && self.store.outbox_drop_to(&ln.child)? > 0 {
            self.reach.hit(ReachPoint::OutboxHeldThenClosed);
        }
        if ln.parent == BOARD {
            self.flow_soon();
        } else {
            self.drop_relayed(&ln.child, &ln.parent, "the worker's link closed")?;
        }
        Ok(json!({"closed": ln.label}))
    }

    async fn inbox(&self, me: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let mut out = Vec::new();
        for mut ln in self.store.children(me, false)? {
            if ln.unread.is_empty() {
                continue;
            }
            for h in &ln.unread {
                if let Some(m) = self.store.msg_get(h)? {
                    out.push(json!({"handle": h, "at": local_clock(&m.at), "label": ln.label, "kind": m.kind, "head": m.head(160)}));
                }
            }
            ln.unread.clear();
            self.store.link_save(&ln)?;
        }
        Ok(Value::Array(out))
    }

    fn show(&self, handle: &str) -> Result<Value> {
        let m = self
            .store
            .msg_get(handle)?
            .ok_or_else(|| anyhow!("no message {handle:?} (only the most recent 5000 are kept)"))?;
        Ok(json!(m))
    }

    pub(crate) async fn board_of(&self, me: &str, all: bool) -> Result<Value> {
        let (parent, kids) = {
            let _g = self.lock.lock().await;
            (self.store.link_get(me)?, self.store.children(me, !all)?)
        };
        let mut children = Vec::new();
        for k in kids {
            let live = if k.closed { None } else { self.read_thread(&k.child).await.ok() };
            let status = match (&live, k.closed) {
                (_, true) => k.closed_how.clone().unwrap_or_else(|| "closed".into()),
                (Some(th), _) => th["status"].as_str().unwrap_or("?").to_string(),
                (None, _) => "unreachable".into(),
            };
            let live = live.unwrap_or(Value::Null);
            let last = self.store.msg_last_from(&k.child)?;
            children.push(json!({
                "label": k.label, "role": k.role, "thread": k.child, "link": k.link.clone().or_else(|| live["link"].as_str().map(String::from)),
                "status": status, "asks": count(&live["pendingRequestCount"]), "background": count(&live["pendingBackgroundTasks"]),
                "gate": k.gate, "paused": k.paused_at.is_some(),
                "issue": k.identity.issue, "parentIssue": k.identity.parent_issue, "part": k.identity.part,
                "lastUp": local_clock(&k.last_up), "nudges": k.nudges, "unread": k.unread.len(),
                "settled": k.settled_at.is_some(),
                "last": last.map(|m| json!({"handle": m.handle, "kind": m.kind, "head": m.head(90)})),
            }));
        }
        let parent = parent.map(|p| json!({"thread": p.parent, "label": p.label, "closed": p.closed, "gate": p.gate}));
        Ok(json!({"parent": parent, "children": children}))
    }

    async fn adopt(&self, me: &str, from: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let children = self.store.children(from, true)?;
        let workers: Vec<String> = children.iter().map(|l| l.child.clone()).collect();
        let adopted: Vec<String> = children.iter().map(|l| l.label.clone()).collect();
        let prs: Vec<(String, u64)> =
            children.iter().filter_map(|l| l.pr.as_ref().map(|p| (p.repo.clone(), p.number))).collect();
        let relayed = |c: &crate::board::Card| {
            let ready_for = |p: &crate::reconcile::CardPr| p.ready && prs.contains(&(p.repo.clone(), p.number));
            c.ticket.as_deref() == Some(from)
                && (c.worker.as_ref().is_some_and(|w| workers.contains(&w.thread))
                    || c.pr.as_ref().is_some_and(ready_for))
        };
        let moved: Vec<crate::board::Card> = self.cards()?.into_iter().filter(relayed).collect();
        let heir = self.store.link_get(me)?;
        for c in &moved {
            let mut data = self.store.waiting_data(BOARD, false)?.into_iter().find(|r| r.3 == c.seq).and_then(|r| r.1);
            if let Some(d) = data.as_mut() {
                d["ticket"] = json!(me);
                let label = heir.as_ref().map(|t| t.label.as_str()).or(c.label.as_deref());
                let link = match &c.worker {
                    Some(w) => w.link.as_deref().or_else(|| heir.as_ref().and_then(|t| t.link.as_deref())),
                    None => heir.as_ref().and_then(|t| t.link.as_deref()),
                };
                self.store.waiting_retarget(BOARD, c.seq, label, link, d)?;
            }
        }
        let mut seen_prs: Vec<(String, u64)> = prs.clone();
        for ln in &children {
            if let Some(p) = &ln.pr {
                for (r, n) in &p.followups {
                    if !seen_prs.iter().any(|(er, en)| er == r && en == n) {
                        seen_prs.push((r.clone(), *n));
                    }
                }
            }
            if let Some(live) = self.live_of(&ln.child) {
                let urls = live["prs"].as_array().into_iter().flatten().chain([&live["pr"]["url"]]);
                let forges = self.config().forges.clone();
                for r in urls.filter_map(Value::as_str).flat_map(|u| crate::reconcile::change_refs(u, &forges)) {
                    let repo = r.key();
                    if !seen_prs.iter().any(|(er, en)| er == &repo && *en == r.number) {
                        seen_prs.push((repo, r.number));
                    }
                }
            }
        }
        for c in &moved {
            if let Some(p) = &c.pr
                && !seen_prs.iter().any(|(er, en)| er == &p.repo && *en == p.number)
            {
                seen_prs.push((p.repo.clone(), p.number));
            }
        }
        for mut ln in children {
            ln.parent = me.into();
            self.store.link_save(&ln)?;
        }
        let open_cards = self.cards()?;
        for (repo, number) in &seen_prs {
            if me != BOARD && me != from {
                self.reach.note_adopted(repo, *number, me);
            }
            let Some(mut seen) = self.store.pr_seen_get(repo, *number)? else { continue };
            let Some(pos) = seen.tickets.iter().position(|(x, _)| x == from) else { continue };
            let still_tracked_by_from =
                open_cards.iter().any(|c| {
                    c.worker.is_none()
                        && c.ticket.as_deref() == Some(from)
                        && c.pr.as_ref().is_some_and(|p| p.repo == *repo && p.number == *number)
                }) || self.store.merge_wait_get(repo, *number)?.is_some_and(|w| w.thread.as_deref() == Some(from));
            if still_tracked_by_from {
                continue;
            }
            let (_, since) = seen.tickets.remove(pos);
            if me != BOARD {
                match seen.tickets.iter_mut().find(|(x, _)| x == me) {
                    Some((_, s)) if since.as_str() < s.as_str() => *s = since,
                    Some(_) => {}
                    None => seen.tickets.push((me.into(), since)),
                }
            }
            self.store.pr_seen_put(&seen)?;
        }
        let moved_merge_pr = moved
            .iter()
            .filter(|c| c.worker.is_none())
            .find_map(|c| c.pr.as_ref().map(|p| (p.repo.clone(), p.number)))
            .or_else(|| {
                open_cards.iter().find_map(|c| {
                    (c.ticket.as_deref() == Some(me) && c.worker.is_none())
                        .then(|| {
                            c.pr.as_ref()
                                .filter(|p| p.ready && prs.contains(&(p.repo.clone(), p.number)))
                                .map(|p| (p.repo.clone(), p.number))
                        })
                        .flatten()
                })
            });
        let has_ready_merge = moved_merge_pr.is_some();
        if has_ready_merge {
            let me_ready = open_cards
                .iter()
                .filter(|c| {
                    c.kind == "gate"
                        && c.worker.is_none()
                        && c.ticket.as_deref() == Some(me)
                        && c.pr.as_ref().is_some_and(|p| p.ready)
                })
                .count();
            if me_ready >= 2 {
                self.reach.hit(ReachPoint::TwoReadyCardsOneTicket);
            }
            self.store.waiting_resolve_releasing(BOARD, &[], &[from], "moved to the adopting ticket", &self.now())?;
            self.sync_merge_gate_pr(from)?;
            if let Some(mut t) = heir.filter(|t| !t.closed)
                && t.gate.as_deref().is_none_or(|g| g == "merge" || g.starts_with(RELAY_GATE))
            {
                let already_merge = t.gate.as_deref() == Some("merge");
                t.gate = Some("merge".into());
                t.gate_since = Some(self.now());
                if !already_merge {
                    t.gate_pr = moved_merge_pr;
                }
                t.nudges = 0;
                t.escalated = false;
                self.store.link_save(&t)?;
            }
        }
        if !moved.is_empty() || has_ready_merge {
            self.sync_relay_gate(from)?;
            self.sync_relay_gate(me)?;
            self.changed();
        }
        let pending = self.store.pending_retarget(from, me)?;
        self.store.outbox_retarget(from, me)?;
        Ok(json!({"adopted": adopted, "pending": pending}))
    }

    async fn sweep(&self, me: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        self.reach.hit(ReachPoint::SweepExecuted);
        let open: Vec<String> = self.store.children(me, true)?.into_iter().map(|l| l.label).collect();
        let mut dropped = Vec::new();
        for p in self.store.pending_all()?.into_iter().filter(|p| p.parent == me) {
            self.store.pending_remove(&p.parent, &p.label)?;
            dropped.push(p.label);
        }
        let parent_open = self.store.link_get(me)?.is_some_and(|l| !l.closed);
        Ok(json!({"open": open, "dropped": dropped, "parentOpen": parent_open}))
    }

    fn waiting_add(&self, me: &str, text: &str, from: Option<String>, link: Option<String>) -> Result<Value> {
        if text.trim().is_empty() {
            bail!("wf.waiting.add: empty text");
        }
        Ok(json!({"id": self.alloc_waiting(me, text, from, link, None)?}))
    }

    fn waiting_resolve(&self, me: &str, id: &str, note: Option<&str>) -> Result<Value> {
        match self.store.waiting_resolve(me, id, note, &self.now())? {
            Some(text) => Ok(json!({"id": id, "text": text, "resolved": true})),
            None => bail!("no open waiting-list item {id:?}; wf.waiting.list() shows the open ones"),
        }
    }

    pub(crate) async fn flush_to(&self, to: &str) -> Result<()> {
        let _g = self.lock.lock().await;
        self.flush_outbox(Some(to), !self.watcher_alive()).await.map(|_| ())
    }

    pub(crate) async fn send_wake(&self, to: &str, text: &str, handle: &str) -> Result<Value> {
        self.send_wake_inner(to, text, handle, false).await
    }

    async fn send_wake_inner(&self, to: &str, text: &str, handle: &str, allow_closed: bool) -> Result<Value> {
        self.store.outbox_push_with(to, text, handle, &self.now(), allow_closed)?;
        self.deliver_queued(to, handle).await
    }

    async fn deliver_queued(&self, to: &str, handle: &str) -> Result<Value> {
        let force = !self.watcher_alive();
        let f = self.flush_outbox(Some(to), force).await?;
        if f.dropped.iter().any(|(_, hs)| hs.iter().any(|h| h == handle)) {
            return Ok(json!({"handle": handle, "delivery": "noted",
                "detail": "the recipient's link is closed; wake dropped"}));
        }
        if f.sent.iter().any(|(_, hs)| hs.iter().any(|h| h == handle)) {
            return Ok(json!({"handle": handle, "delivery": "sent"}));
        }
        let detail = match f.errors.first() {
            Some(e) => format!("delivery failed ({e}); the watcher retries"),
            None => "the recipient is mid-turn; the watcher delivers it, merged with anything else queued, when the turn ends".into(),
        };
        Ok(json!({"handle": handle, "delivery": "queued", "detail": detail}))
    }

    fn digest_for(&self, parent: &str, exclude: &[String]) -> Result<(String, Consumed)> {
        let cap = self.config().digest_lines;
        let mut lines = Vec::new();
        let mut consumed = Vec::new();
        let mut overflow = 0;
        for ln in self.store.children(parent, false)? {
            let mut notes = Vec::new();
            let mut entries = Vec::new();
            for h in ln.unread.iter().filter(|h| !exclude.contains(h)) {
                match self.store.msg_get(h)? {
                    Some(m) if m.kind == HANDLED => entries.push((
                        format!("- handled for you {}: {} [{}]", local_clock(&m.at), m.head(200), m.handle),
                        vec![m.handle],
                    )),
                    Some(m) => notes.push(m),
                    None => {}
                }
            }
            if let Some(last) = notes.last() {
                let more = if notes.len() > 1 { format!(" (+{} earlier)", notes.len() - 1) } else { String::new() };
                let line =
                    format!("- {} {} {} [{}]{more}", ln.label, local_clock(&last.at), last.head(120), last.handle);
                entries.push((line, notes.iter().map(|m| m.handle.clone()).collect()));
            }
            let mut done = Vec::new();
            for (line, handles) in entries {
                if lines.len() >= cap {
                    overflow += 1;
                    continue;
                }
                lines.push(line);
                done.extend(handles);
            }
            if !done.is_empty() {
                consumed.push((ln.child.clone(), done));
            }
        }
        if overflow > 0 {
            lines.push(format!("- ... {overflow} more lines: wf.inbox()"));
        }
        if lines.is_empty() {
            return Ok((String::new(), consumed));
        }
        Ok((
            format!("\n\nNotes since your last wake (wf.show(\"<msg>\") for one in full):\n{}", lines.join("\n")),
            consumed,
        ))
    }

    async fn flush_outbox(&self, only_to: Option<&str>, force: bool) -> Result<Flush> {
        self.store.outbox_drop_to(BOARD)?;
        let mut groups: Vec<(String, Vec<crate::store::OutboxItem>)> = Vec::new();
        for item in self.store.outbox_all()? {
            match groups.iter_mut().find(|(to, _)| *to == item.to) {
                Some((_, g)) => g.push(item),
                None => groups.push((item.to.clone(), vec![item])),
            }
        }
        let mut f = Flush { sent: Vec::new(), dropped: Vec::new(), errors: Vec::new() };
        for (to, mut items) in groups {
            if only_to.is_some_and(|o| o != to) {
                continue;
            }
            if !self.sentinel(Sentinel::OutboxNoPurge) && self.store.link_get(&to)?.is_some_and(|l| l.closed) {
                let (keep, drop): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| i.allow_closed);
                if !drop.is_empty() {
                    self.reach.hit(ReachPoint::OutboxHeldThenClosed);
                    let drop_ids: Vec<i64> = drop.iter().map(|i| i.id).collect();
                    let drop_handles: Vec<String> = drop.into_iter().map(|i| i.handle).collect();
                    self.store.outbox_delete(&drop_ids)?;
                    f.dropped.push((to.clone(), drop_handles));
                }
                if keep.is_empty() {
                    continue;
                }
                items = keep;
            }
            if !force && let Ok(th) = self.read_thread(&to).await {
                let held = self.mins(Some(&items[0].queued)).unwrap_or(0.0);
                if ACTIVE.contains(&th["status"].as_str().unwrap_or("")) && held < self.config().max_hold_minutes {
                    self.reach.hit(ReachPoint::OutboxHeld);
                    continue;
                }
            }
            let handles: Vec<String> = items.iter().map(|i| i.handle.clone()).collect();
            let mut text = items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>().join("\n\n");
            let (digest, consumed) = self.digest_for(&to, &handles)?;
            text.push_str(&digest);
            match self.t3.call("t3_thread_send", json!({"threadId": to, "message": text, "mode": "queue"})).await {
                Err(e) => {
                    self.reach.hit(ReachPoint::OutboxSendFailed);
                    f.errors.push(format!("{to}: {e}"));
                }
                Ok(_) => {
                    for (child, done) in consumed {
                        if let Some(mut l) = self.store.link_get(&child)? {
                            l.unread.retain(|h| !done.contains(h));
                            self.store.link_save(&l)?;
                        }
                    }
                    self.store.outbox_delete(&items.iter().map(|i| i.id).collect::<Vec<_>>())?;
                    f.sent.push((to, handles));
                }
            }
        }
        Ok(f)
    }

    fn report_once(&self, ln: &mut Link, key: String, kind: &str, text: &str, actions: &mut Vec<String>) -> Result<()> {
        if ln.reported.contains(&key) {
            return Ok(());
        }
        ln.reported.push(key.clone());
        let handle = self.store.msg_new(WATCHER, &ln.parent, kind, text, &self.now())?;
        self.deliver_up(ln, kind, &handle, text)?;
        actions.push(format!("{kind} to {} about {}: {key}", ln.parent, ln.label));
        Ok(())
    }

    fn deliver_up(&self, ln: &Link, kind: &str, handle: &str, body: &str) -> Result<()> {
        if ln.parent == BOARD {
            self.board_card(NewCard {
                kind,
                label: Some(&ln.label),
                ticket: Some(&ln.child),
                link: ln.link.as_deref(),
                body,
                handle: Some(handle),
                options: None,
                pr: None,
                approval: None,
                offer: None,
                worker: None,
            })?;
            return Ok(());
        }
        let msg = format!("{}\n{body}", header(kind, &ln.label, handle, None));
        self.store.outbox_push(&ln.parent, &msg, handle, &self.now())
    }

    fn where_is(ln: &Link) -> String {
        ln.link.clone().unwrap_or_else(|| ln.child.clone())
    }

    async fn tick_link(&self, ln: &mut Link, actions: &mut Vec<String>) -> Result<()> {
        let p = ln.params;
        let now = self.now();
        let th = match self.read_thread(&ln.child).await {
            Ok(th) => th,
            Err(e) if e.is_not_found() => {
                ln.closed = true;
                ln.closed_at = Some(now.clone());
                ln.closed_how = Some("thread-gone".into());
                ln.gate = None;
                ln.gate_since = None;
                ln.gate_pr = None;
                if !self.sentinel(Sentinel::OutboxNoPurge) && self.store.outbox_drop_to(&ln.child)? > 0 {
                    self.reach.hit(ReachPoint::OutboxHeldThenClosed);
                }
                let body = format!("{}'s thread is gone; its link is closed.", ln.label);
                let h = self.store.msg_new(WATCHER, &ln.parent, "note", &body, &now)?;
                if ln.parent == BOARD {
                    self.reach.hit(ReachPoint::TicketThreadGone);
                    self.flow_soon();
                    self.resolve_ticket_cards(&ln.child, "ticket thread gone")?;
                } else {
                    self.reach.hit(ReachPoint::WorkerThreadGone);
                    ln.unread.push(h);
                    self.drop_relayed(&ln.child, &ln.parent, "worker thread gone")?;
                }
                actions.push(format!("closed {}: thread gone", ln.label));
                return Ok(());
            }
            Err(_) => return Ok(()),
        };
        if th["archived"] == true {
            ln.closed = true;
            ln.closed_at = Some(now);
            ln.closed_how = Some("archived".into());
            ln.gate = None;
            ln.gate_since = None;
            ln.gate_pr = None;
            if !self.sentinel(Sentinel::OutboxNoPurge) && self.store.outbox_drop_to(&ln.child)? > 0 {
                self.reach.hit(ReachPoint::OutboxHeldThenClosed);
            }
            if ln.parent == BOARD {
                self.reach.hit(ReachPoint::TicketArchived);
                self.flow_soon();
                self.resolve_ticket_cards(&ln.child, "ticket archived")?;
            } else {
                self.reach.hit(ReachPoint::WorkerArchived);
                self.drop_relayed(&ln.child, &ln.parent, "worker archived")?;
            }
            actions.push(format!("closed {}: archived", ln.label));
            return Ok(());
        }
        if ln.link.is_none() {
            ln.link = th["link"].as_str().map(String::from);
        }
        let updated = th["updatedAt"].as_str();
        ln.last_seen = updated.map(String::from);
        if ln.paused_at.is_some() {
            self.reach.hit(ReachPoint::PausedLinkSkipped);
            return Ok(());
        }
        let status = th["status"].as_str().unwrap_or("");
        let busy =
            ACTIVE.contains(&status) || truthy(&th["blockedOnThreadWait"]) || truthy(&th["pendingBackgroundTasks"]);
        let idle = self.mins(updated).unwrap_or(0.0);
        let latest_run = th["latestRunId"].as_str().map(String::from);
        if truthy(&th["pendingRequestCount"]) || status == "waiting" {
            if idle >= p.down as f64 {
                let text = format!(
                    "{} is waiting on an answer to a question or approval in its own thread for {idle:.0} min: {}",
                    ln.label,
                    Self::where_is(ln)
                );
                let key = format!("asks@{}", latest_run.as_deref().unwrap_or("-"));
                self.report_once(ln, key, "report", &text, actions)?;
            }
            return Ok(());
        }
        if busy {
            let quiet = self.mins(Some(&ln.last_up)).unwrap_or(0.0);
            if ln.gate.is_none() && quiet >= p.stall as f64 {
                let text = format!(
                    "{} has been busy but sent nothing for {quiet:.0} min. t3.t3_thread_read shows where it is: {}",
                    ln.label,
                    Self::where_is(ln)
                );
                let key = format!("stall@{}", ln.last_up);
                if !ln.reported.contains(&key) {
                    self.reach.hit(ReachPoint::StallReported);
                }
                self.report_once(ln, key, "report", &text, actions)?;
            }
            return Ok(());
        }
        if ln.gate.is_some() {
            return Ok(());
        }
        let bad_end = ENDED_BADLY.contains(&status) && ln.bad_run != latest_run;
        if !bad_end && self.config().board.reconcile_idle_minutes > 0 && ln.pr.as_ref().is_some_and(|p| p.waiting) {
            return Ok(());
        }
        let since_nudge = ln.last_nudge.as_deref().and_then(|t| self.mins(Some(t)));
        let due = bad_end || (idle >= p.down as f64 && since_nudge.is_none_or(|s| s >= p.down as f64));
        if !due {
            return Ok(());
        }
        if ln.nudges >= p.miss {
            if !ln.escalated {
                ln.escalated = true;
                self.reach.hit(ReachPoint::IdleEscalated);
                let body = format!(
                    "{} has sat idle for {idle:.0} min through {} nudges, with no gate open. Look at it \
                     (t3.t3_thread_read), then wf.reply, relaunch it, or wf.close({{child: {}}}). Its thread: {}",
                    ln.label,
                    ln.nudges,
                    json!(ln.child),
                    Self::where_is(ln)
                );
                let handle = self.store.msg_new(WATCHER, &ln.parent, "escalate", &body, &now)?;
                self.deliver_up(ln, "escalate", &handle, &body)?;
                actions.push(format!("escalate {} to {}", ln.label, ln.parent));
            }
            return Ok(());
        }
        ln.nudges += 1;
        ln.last_nudge = Some(now.clone());
        if bad_end {
            self.reach.hit(ReachPoint::BadEndNudged);
            ln.bad_run = latest_run;
        }
        let why = if bad_end {
            format!("Your last run ended {status}.")
        } else {
            format!("This thread has been idle {idle:.0} min with its link to its parent open and no gate.")
        };
        let handle = self.store.msg_new(WATCHER, &ln.child, "nudge", &why, &now)?;
        let text = format!(
            "{}\n{why} Run your role's resume step, then tell your parent what changed with wf.send \
             (kind \"note\" for routine progress). Waiting on a person: wf.gate({{open: \"<what>\"}}). \
             Done: wf.send({{kind: \"handoff\", body}}). Stopped for good: wf.send({{kind: \"closed\", body}}). \
             Nudge {} of {}.",
            header("nudge", "watcher", &handle, None),
            ln.nudges,
            p.miss
        );
        actions.push(format!("nudge {} ({}/{})", ln.label, ln.nudges, p.miss));
        let sent =
            self.t3.call("t3_thread_send", json!({"threadId": ln.child, "message": text, "mode": "queue"})).await;
        if sent.is_err() {
            self.store.outbox_push(&ln.child, &text, &handle, &now)?;
        }
        Ok(())
    }

    pub async fn tick(&self) -> Result<Vec<String>> {
        let _g = self.lock.lock().await;
        let mut actions = Vec::new();
        let links = self.store.links_open()?;
        let pending = self.store.pending_all()?;
        if links.is_empty() && pending.is_empty() && self.store.outbox_all()?.is_empty() {
            self.mark_pass()?;
            self.changed();
            return Ok(actions);
        }
        for ln in links {
            let Some(mut ln) = self.store.link_get(&ln.child)?.filter(|l| !l.closed) else { continue };
            self.tick_link(&mut ln, &mut actions).await?;
            self.store.link_save(&ln)?;
        }
        for mut p in pending.into_iter().filter(|p| !p.reported) {
            let age = self.mins(Some(&p.at)).unwrap_or(0.0);
            if age < p.params.down as f64 {
                continue;
            }
            p.reported = true;
            let thread = p
                .link
                .as_deref()
                .filter(|_| p.parent != BOARD)
                .map(|l| format!(" Its thread: {l}."))
                .unwrap_or_default();
            let body = format!(
                "{} was opened {age:.0} min ago and never joined.{thread} Check its launch; wf.sweep() drops it.",
                p.label
            );
            let now = self.now();
            let handle = self.store.msg_new(WATCHER, &p.parent, "report", &body, &now)?;
            if p.parent == BOARD {
                self.board_card(NewCard {
                    kind: "report",
                    label: Some(&p.label),
                    ticket: p.thread.as_deref(),
                    link: p.link.as_deref(),
                    body: &body,
                    handle: Some(&handle),
                    options: None,
                    pr: None,
                    approval: None,
                    offer: None,
                    worker: None,
                })?;
            } else {
                let msg = format!("{}\n{body}", header("report", &p.label, &handle, None));
                self.store.outbox_push(&p.parent, &msg, &handle, &now)?;
            }
            self.store.pending_put(&p)?;
            actions.push(format!("report never-joined {}", p.label));
        }
        let f = self.flush_outbox(None, false).await?;
        actions.extend(f.sent.iter().map(|(to, h)| format!("delivered {} to {to}", h.join("+"))));
        actions.extend(f.errors.iter().map(|e| format!("delivery failed: {e}")));
        self.mark_pass()?;
        self.changed();
        Ok(actions)
    }
}
