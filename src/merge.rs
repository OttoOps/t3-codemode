use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::codehost::{Change, ChangeRef, Forge, Landed, Landing, State, Viewed};
use crate::store::{MergeWait, WaitStatus};
use crate::wf::Wf;
use crate::wf_pr::change_of;

const METHODS: &[&str] = &["MERGE", "SQUASH", "REBASE"];
const REFUSED: &[&str] = &["BEHIND", "DIRTY"];
const UNKNOWN_READS: usize = 3;
const UNKNOWN_WAIT: Duration = Duration::from_secs(1);
const CAS_TRIES: usize = 3;

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

struct Seen {
    wait: Option<MergeWait>,
    ended: Option<(String, u64)>,
}

struct Attempt<'a> {
    r: &'a ChangeRef,
    out: &'a Outcome,
    caller: Option<&'a str>,
    method: &'a Option<String>,
    kind: &'a str,
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
        let out = Outcome { repo: r.key(), pr: r.number, head };
        let kind = match r.forge {
            Forge::Github => "pull request",
            Forge::Gitlab { .. } => "merge request",
        };
        let at = Attempt { r: &r, out: &out, caller, method: &method, kind };
        for _ in 0..CAS_TRIES {
            let prior = self.seen(&out)?;
            if let Some(v) = self.merge_once(&at, &prior).await? {
                return Ok(v);
            }
        }
        Ok(out.refused(format!(
            "the merge approval for {}#{} kept changing while t3cm read the {kind}; send wf.merge again",
            out.repo, out.pr
        )))
    }

    async fn view_settled(&self, r: &ChangeRef) -> Result<Viewed> {
        let mut viewed = Viewed::NotFound;
        for attempt in 0..UNKNOWN_READS {
            if attempt > 0 {
                tokio::time::sleep(UNKNOWN_WAIT).await;
            }
            viewed = self.host.view(r).await?;
            let Viewed::Found(d) = &viewed else { break };
            if d.change.state != State::Open || d.change.merge_state != "UNKNOWN" {
                break;
            }
        }
        Ok(viewed)
    }

    fn seen(&self, out: &Outcome) -> Result<Seen> {
        let wait = self.store.merge_wait_get(&out.repo, out.pr)?;
        let seen = self.store.pr_seen_get(&out.repo, out.pr)?;
        let ended = seen.filter(|s| s.state != State::Open.as_str()).map(|s| (s.state, s.rev));
        Ok(Seen { wait, ended })
    }

    fn merged_at(&self, at: &Attempt<'_>, commit: &str) -> Result<()> {
        self.store.merge_wait_remove(&at.out.repo, at.out.pr)?;
        if let Some(c) = at.caller {
            self.store.pr_event_mark(&at.out.repo, at.out.pr, &format!("merged:{commit}"), c, &self.now())?;
        }
        Ok(())
    }

    fn record(
        &self,
        at: &Attempt<'_>,
        prior: &Seen,
        id: &str,
        state: &str,
        status: WaitStatus,
    ) -> Result<Option<String>> {
        let out = at.out;
        let now = self.seen(out)?;
        if now.wait != prior.wait || (now.ended.is_some() && now.ended != prior.ended) {
            return Ok(None);
        }
        let approval = self.new_id();
        let label = match at.caller {
            Some(c) => self.store.link_get(c)?.map(|l| l.label),
            None => None,
        };
        self.store.merge_wait_put(&MergeWait {
            repo: out.repo.clone(),
            pr: out.pr,
            head: out.head.clone(),
            pull_request_id: id.into(),
            method: at.method.clone(),
            thread: at.caller.map(String::from),
            label,
            merge_state: state.into(),
            at: self.now(),
            status,
            approval: approval.clone(),
            failures: 0,
            last_error: None,
            advancing: None,
        })?;
        Ok(Some(approval))
    }

    async fn landed(&self, at: &Attempt<'_>, landed: Landed) -> Result<Landed> {
        let Landed::Refused(e) = landed else { return Ok(landed) };
        let Viewed::Found(d) = self.host.view(at.r).await? else { return Ok(Landed::Refused(e)) };
        let pr = d.change;
        Ok(match pr.state {
            State::Merged if pr.head == at.out.head => Landed::Merged { commit: Some(pr.commit().to_string()) },
            State::Open if pr.queued && pr.head == at.out.head => Landed::Queued { position: pr.position },
            _ => Landed::Refused(e),
        })
    }

    async fn merge_once(&self, at: &Attempt<'_>, prior: &Seen) -> Result<Option<Value>> {
        let (r, out, kind) = (at.r, at.out, at.kind);
        let forge = r.forge.name();
        let head = out.head.as_str();
        let pr = match self.view_settled(r).await? {
            Viewed::Refused(e) => return Ok(Some(out.refused(e))),
            Viewed::NotFound => return Ok(Some(out.refused(format!("{}#{} is not a {kind}", out.repo, out.pr)))),
            Viewed::Found(d) => d.change,
        };
        let now = pr.head.clone();
        match pr.state {
            State::Merged => {
                let _g = self.lock.lock().await;
                if now != head {
                    self.store.merge_wait_remove(&out.repo, out.pr)?;
                    return Ok(Some(out.refused(format!("already merged, at head {now}"))));
                }
                self.merged_at(at, pr.commit())?;
                return Ok(Some(out.with("merged", json!({"commit": pr.commit}))));
            }
            State::Open => {}
            State::Closed => return Ok(Some(out.refused(format!("the {kind} is closed")))),
        }
        if pr.draft {
            return Ok(Some(out.refused(format!("the {kind} is a draft"))));
        }
        if now != head {
            return Ok(Some(out.refused(format!("head moved: the {kind} is at {now}"))));
        }
        let id = pr.id.as_deref().ok_or_else(|| anyhow!("{forge} returned no id for {}#{}", out.repo, out.pr))?;
        let state = pr.merge_state.as_str();
        let ready = ready(&pr);
        let failing = |f: &str| out.refused(format!("a check failed at head {}: {f}", &head[..7]));
        let can_wait = !REFUSED.contains(&state) && check_state(&pr) != CheckState::Unread;
        let hold = async |status: WaitStatus, done: &(dyn Fn(String) -> Value + Sync)| {
            let _g = self.lock.lock().await;
            Ok::<_, anyhow::Error>(self.record(at, prior, id, state, status)?.map(done))
        };
        let sent = async |position: &Option<u64>| {
            let _g = self.lock.lock().await;
            let approval = self.record(at, prior, id, state, WaitStatus::Sent)?;
            Ok::<_, anyhow::Error>(Some(out.with("queued", json!({"position": position, "approval": approval}))))
        };

        if pr.queue {
            if pr.queued {
                return hold(WaitStatus::Sent, &|a| {
                    out.with("queued", json!({"position": pr.position, "approval": a}))
                })
                .await;
            }
            if let Ready::Failing(f) = &ready {
                return Ok(Some(failing(f)));
            }
            if ready != Ready::Yes {
                return hold(WaitStatus::Waiting, &|a| {
                    out.with("waiting", json!({"mergeState": state, "approval": a}))
                })
                .await;
            }
            let method = at.method.clone().or_else(|| pr.method.clone()).unwrap_or_else(|| "MERGE".into());
            let landed = self.host.merge(r, id, head, Landing::Queue { method: &method }).await?;
            return match self.landed(at, landed).await? {
                Landed::Refused(e) => Ok(Some(out.refused(e))),
                Landed::Waiting(e) => {
                    hold(WaitStatus::Waiting, &|a| {
                        out.with("waiting", json!({"mergeState": state, "approval": a, "reason": e}))
                    })
                    .await
                }
                Landed::Queued { position } => sent(&position).await,
                Landed::Merged { commit } => {
                    let _g = self.lock.lock().await;
                    self.merged_at(at, commit.as_deref().unwrap_or_default())?;
                    Ok(Some(out.with("merged", json!({"commit": commit}))))
                }
            };
        }

        match &ready {
            Ready::Yes => {}
            Ready::Failing(f) => return Ok(Some(failing(f))),
            Ready::Not(_) if can_wait => {
                return hold(WaitStatus::Waiting, &|a| {
                    out.with("waiting", json!({"mergeState": state, "approval": a}))
                })
                .await;
            }
            Ready::Not(why) => return Ok(Some(out.refused(format!("not mergeable yet ({why})")))),
        }
        let method = at.method.clone().or_else(|| pr.method.clone()).unwrap_or_else(|| "MERGE".into());
        let landed = self.host.merge(r, id, head, Landing::Merge { method: &method }).await?;
        let ours = matches!(landed, Landed::Merged { .. });
        match self.landed(at, landed).await? {
            Landed::Refused(e) => Ok(Some(out.refused(e))),
            Landed::Waiting(e) if can_wait => {
                hold(WaitStatus::Waiting, &|a| {
                    out.with("waiting", json!({"mergeState": state, "approval": a, "reason": e}))
                })
                .await
            }
            Landed::Waiting(e) => Ok(Some(out.refused(e))),
            Landed::Merged { commit } => {
                let _g = self.lock.lock().await;
                self.merged_at(at, commit.as_deref().unwrap_or_default())?;
                let extra = match ours {
                    true => json!({"commit": commit, "method": method.to_lowercase()}),
                    false => json!({"commit": commit}),
                };
                Ok(Some(out.with("merged", extra)))
            }
            Landed::Queued { position } => sent(&position).await,
        }
    }
}
