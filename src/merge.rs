use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::board::{gh_name, random_id};
use crate::github::errors;
use crate::store::{MergeWait, WaitStatus};
use crate::wf::Wf;

pub const MERGEABLE: &[&str] = &["CLEAN", "HAS_HOOKS", "UNSTABLE"];
const METHODS: &[&str] = &["MERGE", "SQUASH", "REBASE"];
const UNKNOWN_READS: usize = 3;
const UNKNOWN_WAIT: Duration = Duration::from_secs(1);

pub const READ_PR: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { \
viewerDefaultMergeMethod pullRequest(number: $number) { id state isDraft headRefOid mergeStateStatus \
isMergeQueueEnabled isInMergeQueue mergeQueueEntry { position } mergeCommit { oid } } } }";
pub const ENQUEUE_PR: &str = "mutation($id: ID!, $head: GitObjectID!) { enqueuePullRequest(input: { \
pullRequestId: $id, expectedHeadOid: $head }) { mergeQueueEntry { position } } }";
pub const MERGE_PR: &str = "mutation($id: ID!, $head: GitObjectID!, $method: PullRequestMergeMethod!) { \
mergePullRequest(input: { pullRequestId: $id, expectedHeadOid: $head, mergeMethod: $method }) { \
pullRequest { mergeCommit { oid } } } }";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeArgs {
    repo: String,
    pr: u64,
    head: String,
    method: Option<String>,
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
        let (owner, name) = a
            .repo
            .trim()
            .split_once('/')
            .filter(|(o, n)| gh_name(o) && gh_name(n))
            .ok_or_else(|| anyhow!("wf.merge: repo must be \"owner/name\", got {:?}", a.repo))?;
        if a.pr == 0 {
            bail!("wf.merge: pr must be a pull request number");
        }
        let head = a.head.trim().to_ascii_lowercase();
        if head.len() != 40 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("wf.merge: head must be the full 40-character head commit SHA, got {:?}", a.head);
        }
        let method = match a.method.map(|m| m.trim().to_ascii_uppercase()) {
            Some(m) if !METHODS.contains(&m.as_str()) => bail!("wf.merge: method must be merge, squash or rebase"),
            m => m,
        };
        let out = Outcome { repo: format!("{owner}/{name}").to_ascii_lowercase(), pr: a.pr, head: head.clone() };

        let mut read = Value::Null;
        for attempt in 0..UNKNOWN_READS {
            if attempt > 0 {
                tokio::time::sleep(UNKNOWN_WAIT).await;
            }
            read = self.github.graphql(READ_PR, json!({"owner": owner, "name": name, "number": a.pr})).await?;
            let pr = &read["data"]["repository"]["pullRequest"];
            if errors(&read).is_some() || pr["state"] != "OPEN" || pr["mergeStateStatus"] != "UNKNOWN" {
                break;
            }
        }
        if let Some(e) = errors(&read) {
            return Ok(out.refused(e));
        }
        let repo = &read["data"]["repository"];
        let pr = &repo["pullRequest"];
        if pr.is_null() {
            return Ok(out.refused(format!("{}#{} is not a pull request", out.repo, out.pr)));
        }
        let now = pr["headRefOid"].as_str().unwrap_or_default().to_ascii_lowercase();
        let merged = |commit: &str| -> Result<()> {
            self.store.merge_wait_remove(&out.repo, out.pr)?;
            if let Some(c) = caller {
                self.store.pr_event_mark(&out.repo, out.pr, &format!("merged:{commit}"), c, &self.now())?;
            }
            Ok(())
        };
        match pr["state"].as_str() {
            Some("MERGED") => {
                if now != head {
                    self.store.merge_wait_remove(&out.repo, out.pr)?;
                    return Ok(out.refused(format!("already merged, at head {now}")));
                }
                merged(pr["mergeCommit"]["oid"].as_str().unwrap_or_default())?;
                return Ok(out.with("merged", json!({"commit": pr["mergeCommit"]["oid"]})));
            }
            Some("OPEN") => {}
            other => {
                return Ok(out.refused(format!("the pull request is {}", other.unwrap_or("unknown").to_lowercase())));
            }
        }
        if pr["isDraft"] == true {
            return Ok(out.refused("the pull request is a draft"));
        }
        if now != head {
            return Ok(out.refused(format!("head moved: the pull request is at {now}")));
        }
        let id = pr["id"].as_str().ok_or_else(|| anyhow!("GitHub returned no pull request id"))?;
        let state = pr["mergeStateStatus"].as_str().unwrap_or("UNKNOWN");
        let mergeable = MERGEABLE.contains(&state);
        let record = |status: WaitStatus| -> Result<()> {
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
                approval: random_id(),
                failures: 0,
                last_error: None,
            })
        };

        if pr["isMergeQueueEnabled"] == true {
            if pr["isInMergeQueue"] == true {
                record(WaitStatus::Sent)?;
                return Ok(out.with("queued", json!({"position": pr["mergeQueueEntry"]["position"]})));
            }
            if !mergeable {
                record(WaitStatus::Waiting)?;
                return Ok(out.with("waiting", json!({"mergeState": state})));
            }
            let r = self.github.graphql(ENQUEUE_PR, json!({"id": id, "head": head})).await?;
            if let Some(e) = errors(&r) {
                return Ok(out.refused(e));
            }
            record(WaitStatus::Sent)?;
            let position = &r["data"]["enqueuePullRequest"]["mergeQueueEntry"]["position"];
            return Ok(out.with("queued", json!({"position": position})));
        }

        if !mergeable {
            return Ok(out.refused(format!("not mergeable yet (merge state {state})")));
        }
        let method = method
            .clone()
            .or_else(|| repo["viewerDefaultMergeMethod"].as_str().map(String::from))
            .unwrap_or_else(|| "MERGE".into());
        let r = self.github.graphql(MERGE_PR, json!({"id": id, "head": head, "method": method})).await?;
        if let Some(e) = errors(&r) {
            return Ok(out.refused(e));
        }
        let commit = &r["data"]["mergePullRequest"]["pullRequest"]["mergeCommit"]["oid"];
        merged(commit.as_str().unwrap_or_default())?;
        Ok(out.with("merged", json!({"commit": commit, "method": method.to_lowercase()})))
    }
}
