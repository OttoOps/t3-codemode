use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::board::random_id;
use crate::codehost::{Change, Forge, Landed, Landing, State, Viewed};
use crate::store::{MergeWait, WaitStatus};
use crate::wf::Wf;
use crate::wf_pr::change_of;

const METHODS: &[&str] = &["MERGE", "SQUASH", "REBASE"];
const REFUSED: &[&str] = &["BEHIND", "DIRTY"];
const UNKNOWN_READS: usize = 3;
const UNKNOWN_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq)]
pub enum Ready {
    Yes,
    Not(String),
    Failing(String),
}

pub fn ready(c: &Change) -> Ready {
    let state = c.merge_state.as_str();
    match state {
        "CLEAN" | "HAS_HOOKS" => {
            return match &c.checks {
                Some(t) if !t.failing.is_empty() => Ready::Failing(t.failing.join(", ")),
                Some(t) if !t.pending.is_empty() => {
                    Ready::Not(format!("merge state {state}, checks still running: {}", t.pending.join(", ")))
                }
                _ => Ready::Yes,
            };
        }
        "UNSTABLE" => {}
        s => return Ready::Not(format!("merge state {s}")),
    }
    let unread = || Ready::Not("merge state UNSTABLE, and its checks could not be read".into());
    let Some(t) = &c.checks else { return unread() };
    if !t.failing.is_empty() {
        Ready::Failing(t.failing.join(", "))
    } else if !t.complete {
        unread()
    } else if !t.pending.is_empty() {
        Ready::Not(format!("merge state UNSTABLE, checks still running: {}", t.pending.join(", ")))
    } else {
        Ready::Yes
    }
}

#[derive(Debug, PartialEq)]
pub enum CheckState {
    Passed(Vec<String>),
    Pending(String),
    Failing(String),
    Unread,
}

pub fn check_state(c: &Change) -> CheckState {
    let Some(t) = &c.checks else { return CheckState::Unread };
    if !t.failing.is_empty() {
        CheckState::Failing(t.failing.join(", "))
    } else if !t.complete {
        CheckState::Unread
    } else if !t.pending.is_empty() {
        CheckState::Pending(t.pending.join(", "))
    } else {
        CheckState::Passed(t.settled.clone())
    }
}

pub fn checks(c: &Change) -> Option<&'static str> {
    let t = c.checks.as_ref()?;
    if !t.failing.is_empty() {
        Some("failing")
    } else if !t.complete {
        None
    } else if !t.pending.is_empty() {
        Some("pending")
    } else {
        Some("passing")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeArgs {
    pub(crate) repo: String,
    #[serde(default)]
    pub(crate) pr: u64,
    pub(crate) head: String,
    pub(crate) method: Option<String>,
}

struct Outcome {
    repo: String,
    pr: u64,
    head: String,
}

impl Outcome {
    fn with(&self, status: &str, extra: Value) -> Value {
        let mut v = json!({"status": status, "repo": self.repo, "pr": self.pr, "head": self.head});
        if let (Some(v), Value::Object(extra)) = (v.as_object_mut(), extra) {
            v.extend(extra);
        }
        v
    }

    fn refused(&self, reason: impl Into<String>) -> Value {
        self.with("refused", json!({"reason": reason.into()}))
    }
}

impl Wf {
    pub(crate) async fn merge(&self, caller: Option<&str>, a: MergeArgs) -> Result<Value> {
        let r = change_of("wf.merge", &a.repo, a.pr, &self.config().forges)?;
        let head = a.head.trim().to_ascii_lowercase();
        if head.len() != 40 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("wf.merge: head must be the full 40-character head commit SHA, got {:?}", a.head);
        }
        let method = match a.method.map(|m| m.trim().to_ascii_uppercase()) {
            Some(m) if !METHODS.contains(&m.as_str()) => bail!("wf.merge: method must be merge, squash or rebase"),
            m => m,
        };
        let out = Outcome { repo: r.key(), pr: r.number, head: head.clone() };
        let forge = r.forge.name();
        let kind = match r.forge {
            Forge::Github => "pull request",
            Forge::Gitlab { .. } => "merge request",
        };
        let mut viewed = Viewed::NotFound;
        for attempt in 0..UNKNOWN_READS {
            if attempt > 0 {
                tokio::time::sleep(UNKNOWN_WAIT).await;
            }
            viewed = self.host.view(&r).await?;
            let Viewed::Found(d) = &viewed else { break };
            if d.change.state != State::Open || d.change.merge_state != "UNKNOWN" {
                break;
            }
        }
        let pr = match viewed {
            Viewed::Refused(e) => return Ok(out.refused(e)),
            Viewed::NotFound => return Ok(out.refused(format!("{}#{} is not a {kind}", out.repo, out.pr))),
            Viewed::Found(d) => d.change,
        };
        let now = pr.head.clone();
        let merged = |commit: &str| -> Result<()> {
            self.store.merge_wait_remove(&out.repo, out.pr)?;
            if let Some(c) = caller {
                self.store.pr_event_mark(&out.repo, out.pr, &format!("merged:{commit}"), c, &self.now())?;
            }
            Ok(())
        };
        match pr.state {
            State::Merged => {
                if now != head {
                    self.store.merge_wait_remove(&out.repo, out.pr)?;
                    return Ok(out.refused(format!("already merged, at head {now}")));
                }
                merged(pr.commit())?;
                return Ok(out.with("merged", json!({"commit": pr.commit})));
            }
            State::Open => {}
            State::Closed => return Ok(out.refused(format!("the {kind} is closed"))),
        }
        if pr.draft {
            return Ok(out.refused(format!("the {kind} is a draft")));
        }
        if now != head {
            return Ok(out.refused(format!("head moved: the {kind} is at {now}")));
        }
        let id = pr.id.as_deref().ok_or_else(|| anyhow!("{forge} returned no id for {}#{}", out.repo, out.pr))?;
        let state = pr.merge_state.as_str();
        let ready = ready(&pr);
        let failing = |f: &str| out.refused(format!("a check failed at head {}: {f}", &head[..7]));
        let can_wait = !REFUSED.contains(&state) && check_state(&pr) != CheckState::Unread;
        let record = |status: WaitStatus| -> Result<String> {
            let approval = random_id();
            let label = match caller {
                Some(c) => self.store.link_get(c)?.map(|l| l.label),
                None => None,
            };
            self.store.merge_wait_put(&MergeWait {
                repo: out.repo.clone(),
                pr: out.pr,
                head: head.clone(),
                pull_request_id: id.into(),
                method: method.clone(),
                thread: caller.map(String::from),
                label,
                merge_state: state.into(),
                at: self.now(),
                status,
                approval: approval.clone(),
                failures: 0,
                last_error: None,
            })?;
            Ok(approval)
        };

        if pr.queue {
            if pr.queued {
                let approval = record(WaitStatus::Sent)?;
                return Ok(out.with("queued", json!({"position": pr.position, "approval": approval})));
            }
            if let Ready::Failing(f) = &ready {
                return Ok(failing(f));
            }
            if ready != Ready::Yes {
                let approval = record(WaitStatus::Waiting)?;
                return Ok(out.with("waiting", json!({"mergeState": state, "approval": approval})));
            }
            let method = method.clone().or_else(|| pr.method.clone()).unwrap_or_else(|| "MERGE".into());
            return match self.host.merge(&r, id, &head, Landing::Queue { method: &method }).await? {
                Landed::Refused(e) => Ok(out.refused(e)),
                Landed::Waiting(e) => {
                    let approval = record(WaitStatus::Waiting)?;
                    Ok(out.with("waiting", json!({"mergeState": state, "approval": approval, "reason": e})))
                }
                Landed::Queued { position } => {
                    let approval = record(WaitStatus::Sent)?;
                    Ok(out.with("queued", json!({"position": position, "approval": approval})))
                }
                Landed::Merged { commit } => {
                    merged(commit.as_deref().unwrap_or_default())?;
                    Ok(out.with("merged", json!({"commit": commit})))
                }
            };
        }

        match &ready {
            Ready::Yes => {}
            Ready::Failing(f) => return Ok(failing(f)),
            Ready::Not(_) if can_wait => {
                let approval = record(WaitStatus::Waiting)?;
                return Ok(out.with("waiting", json!({"mergeState": state, "approval": approval})));
            }
            Ready::Not(why) => return Ok(out.refused(format!("not mergeable yet ({why})"))),
        }
        let method = method.clone().or_else(|| pr.method.clone()).unwrap_or_else(|| "MERGE".into());
        match self.host.merge(&r, id, &head, Landing::Merge { method: &method }).await? {
            Landed::Refused(e) => Ok(out.refused(e)),
            Landed::Waiting(e) if can_wait => {
                let approval = record(WaitStatus::Waiting)?;
                Ok(out.with("waiting", json!({"mergeState": state, "approval": approval, "reason": e})))
            }
            Landed::Waiting(e) => Ok(out.refused(e)),
            Landed::Merged { commit } => {
                merged(commit.as_deref().unwrap_or_default())?;
                Ok(out.with("merged", json!({"commit": commit, "method": method.to_lowercase()})))
            }
            Landed::Queued { position } => {
                let approval = record(WaitStatus::Sent)?;
                Ok(out.with("queued", json!({"position": position, "approval": approval})))
            }
        }
    }
}
