use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{BOARD, Card};
use crate::codehost::{Branch, ChangeRef, CodeHost, Forge};
use crate::merge::MergeArgs;
use crate::reconcile::{CardPr, CardStage};
use crate::store::{Link, MergeWait, WaitStatus};
use crate::wf::{WATCHER, Wf, capped, header};
use crate::wf_pr::{chain_label, same_head};

const DONE: &str = "merged-in-code";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Answered {
    pub repo: String,
    pub pr: u64,
    pub head: String,
    pub ticket: String,
    pub worker: String,
    pub worker_label: String,
    pub answer: String,
    pub card: String,
    pub method: Option<String>,
    pub approved: Vec<String>,
    pub after: Vec<CardStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<String>,
}

impl Answered {
    fn change(&self) -> ChangeRef {
        ChangeRef::of_key(&self.repo, self.pr)
    }

    fn url(&self, host: &dyn CodeHost) -> String {
        host.url(&self.change())
    }

    fn merged(&self, host: &dyn CodeHost, commit: &str) -> String {
        match self.answer.as_str() {
            "" => format!(
                "{} merged on {} at {}, outside t3cm (merge commit {commit})",
                self.url(host),
                self.change().forge.name(),
                short(commit)
            ),
            a => {
                format!(
                    "{} merged at {} on the user's {} (merge commit {commit})",
                    self.url(host),
                    short(commit),
                    json!(a)
                )
            }
        }
    }
}

pub(crate) enum Job {
    Merge(Answered),
    Done(Value),
}

fn key(repo: &str, pr: u64) -> String {
    format!("merge_answer:{repo}#{pr}")
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn full_sha(head: Option<&String>) -> Option<String> {
    head.filter(|h| h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit())).map(|h| h.to_ascii_lowercase())
}

fn listed(ids: &[&str]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.to_string(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

impl Wf {
    pub(crate) fn answered_get(&self, repo: &str, pr: u64) -> Result<Option<Answered>> {
        Ok(self.store.meta_get(&key(repo, pr))?.and_then(|s| serde_json::from_str(&s).ok()))
    }

    fn answered_put(&self, a: &Answered) -> Result<()> {
        self.store.meta_set(&key(&a.repo, a.pr), &serde_json::to_string(a)?)
    }

    pub(crate) fn answered_drop(&self, repo: &str, pr: u64) -> Result<()> {
        self.store.meta_del(&key(repo, pr)).map(|_| ())
    }

    fn worker_of(&self, ticket: &str, repo: &str, pr: u64) -> Result<Option<Link>> {
        Ok(self.store.children(ticket, true)?.into_iter().find(|w| {
            w.role == "worker" && !w.closed && w.pr.as_ref().is_some_and(|p| p.repo == repo && p.number == pr)
        }))
    }

    pub(crate) fn merge_job(&self, card: &Card, ticket: &str, text: &str) -> Result<Option<Job>> {
        let Some(p) = card.pr.as_ref().filter(|_| card.worker.is_none()) else { return Ok(None) };
        if self.config().board.reconcile_idle_minutes == 0 {
            return Ok(None);
        }
        if self.store.link_get(ticket)?.is_none_or(|t| t.closed || t.parent != BOARD) {
            return Ok(None);
        }
        if card.kind == "gate" && p.ready {
            return self.ready_job(card, p, ticket, text);
        }
        let Some(mut rec) = self.answered_get(&p.repo, p.number)?.filter(|r| r.ticket == ticket) else {
            return Ok(None);
        };
        let Some(head) = full_sha(p.head.as_ref()).filter(|_| card.approval.is_some()) else { return Ok(None) };
        let word = text.to_ascii_lowercase();
        let merge = match word.as_str() {
            "ok" if card.kind == "report" => false,
            "requeue" | "merge" => true,
            _ => {
                self.answered_drop(&p.repo, p.number)?;
                if let Some(w) = self
                    .store
                    .merge_wait_get(&p.repo, p.number)?
                    .filter(|w| card.approval.as_ref() == Some(&w.approval))
                {
                    self.retire_merge_wait(&w)?;
                }
                return Ok(None);
            }
        };
        if !same_head(&rec.head, &head) {
            rec.approved.clear();
        }
        rec.head = head;
        rec.card = card.id.clone();
        if !merge {
            let approval = self.new_id();
            let seen = self.store.pr_seen_get(&p.repo, p.number)?;
            self.store.merge_wait_put(&MergeWait {
                repo: p.repo.clone(),
                pr: p.number,
                head: rec.head.clone(),
                pull_request_id: String::new(),
                method: rec.method.clone(),
                thread: Some(ticket.into()),
                label: self.store.link_get(ticket)?.map(|l| l.label),
                merge_state: seen.and_then(|s| s.merge_state).unwrap_or_else(|| "UNKNOWN".into()),
                at: self.now(),
                status: WaitStatus::Sent,
                approval: approval.clone(),
                failures: 0,
                last_error: None,
                advancing: None,
            })?;
            rec.approval = Some(approval);
        }
        self.answered_put(&rec)?;
        if !self.store.waiting_resolve_seq(BOARD, card.seq, Some(&format!("answered: {text}")), &self.now())? {
            return Ok(None);
        }
        self.sync_merge_gate_pr(ticket)?;
        self.changed();
        if merge {
            return Ok(Some(Job::Merge(rec)));
        }
        let note = format!(
            "the board's answer \"ok\" on card {} accepted {} at head {}; t3cm keeps waiting for {} to merge it",
            card.id,
            rec.url(&*self.host),
            short(&rec.head),
            rec.change().forge.name()
        );
        self.handled(&rec.worker, ticket, &note)?;
        Ok(Some(Job::Done(json!({"card": card.id, "accepted": rec.head, "delivery": "t3cm"}))))
    }

    fn ready_job(&self, card: &Card, p: &CardPr, ticket: &str, text: &str) -> Result<Option<Job>> {
        let Some(option) = card.options.iter().find(|o| o.eq_ignore_ascii_case(text) && o.starts_with("merge")) else {
            return Ok(None);
        };
        let (Some(after), Some(head)) = (p.after.as_ref(), full_sha(p.head.as_ref())) else { return Ok(None) };
        let Some(w) = self.worker_of(ticket, &p.repo, p.number)? else { return Ok(None) };
        let Some(method) = w.brief.as_ref().and_then(|b| b.method.clone()) else { return Ok(None) };
        let Some(k) = (0..=after.len()).find(|k| {
            let ids: Vec<&str> = ["merge"].into_iter().chain(after[..*k].iter().map(|s| s.id.as_str())).collect();
            chain_label(&ids) == *option
        }) else {
            return Ok(None);
        };
        let rec = Answered {
            repo: p.repo.clone(),
            pr: p.number,
            head,
            ticket: ticket.into(),
            worker: w.child.clone(),
            worker_label: w.label.clone(),
            answer: option.clone(),
            card: card.id.clone(),
            method: Some(method),
            approved: after[..k].iter().map(|s| s.id.clone()).collect(),
            after: after.clone(),
            approval: None,
        };
        self.answered_put(&rec)?;
        if !self.store.waiting_resolve_seq(BOARD, card.seq, Some(&format!("answered: {text}")), &self.now())? {
            self.answered_drop(&rec.repo, rec.pr)?;
            return Ok(None);
        }
        if let Some(mut tl) = self.store.link_get(ticket)?.filter(|l| l.gate.as_deref() == Some("merge")) {
            tl.gate_pr = None;
            self.store.link_save(&tl)?;
        }
        let note = format!(
            "the board's answer {} on card {} went to t3cm, which merges {} at head {} in code",
            json!(option),
            card.id,
            rec.url(&*self.host),
            short(&rec.head)
        );
        self.handled(&rec.worker, ticket, &note)?;
        self.changed();
        Ok(Some(Job::Merge(rec)))
    }

    pub(crate) async fn run_merge(&self, rec: Answered, said: &str, card_text: &str) -> Result<Value> {
        let args = MergeArgs { repo: rec.repo.clone(), pr: rec.pr, head: rec.head.clone(), method: rec.method.clone() };
        let out = match self.merge(Some(&rec.ticket), args).await {
            Ok(v) => v,
            Err(e) => json!({"status": "error", "reason": format!("{e:#}")}),
        };
        let status = out["status"].as_str().unwrap_or_default().to_string();
        let mut result = json!({"card": rec.card, "delivery": "t3cm", "merge": out});
        match status.as_str() {
            "merged" => {
                let commit = out["commit"].as_str().unwrap_or_default().to_string();
                for t in self.after_merge(&rec, &commit).await? {
                    self.flush_to(&t).await?;
                }
            }
            "queued" | "waiting" => {
                let _g = self.lock.lock().await;
                let mut rec = rec.clone();
                rec.approval = out["approval"].as_str().map(String::from);
                self.answered_put(&rec)?;
                let forge = rec.change().forge.name();
                let what = match status.as_str() {
                    "queued" => format!("it is in the merge queue at position {}", out["position"]),
                    _ => format!(
                        "{forge} does not let it merge yet (merge state {})",
                        out["mergeState"].as_str().unwrap_or("UNKNOWN")
                    ),
                };
                let note = format!(
                    "t3cm holds the approval for {} at head {}: {what}. It merges once {forge} allows, and an admin merge on {forge} counts too",
                    rec.url(&*self.host),
                    short(&rec.head)
                );
                self.handled(&rec.worker, &rec.ticket, &note)?;
                self.changed();
            }
            _ => {
                let reason = out["reason"].as_str().unwrap_or("no reason given").to_string();
                {
                    let _g = self.lock.lock().await;
                    self.answered_drop(&rec.repo, rec.pr)?;
                }
                let body = format!(
                    "The user says {} about {}: {card_text}\n\nt3cm ran wf.merge for it at head {}, and the outcome was {status}: {reason}. \
                     t3cm left this to the ticket: act on that outcome as \"On the user's word\" item 2 says.",
                    json!(said),
                    rec.card,
                    rec.head
                );
                let h = self.store.msg_new(BOARD, &rec.ticket, "reply", &body, &self.now())?;
                let msg =
                    format!("{}\n{}", header("reply", BOARD, &h, None), capped(&body, &h, self.config().reply_cap));
                result["ticket"] = self.send_wake(&rec.ticket, &msg, &h).await?;
            }
        }
        Ok(result)
    }

    pub(crate) fn code_merged(&self, repo: &str, number: u64, head: &str, cards: &[Card]) -> Result<Option<Answered>> {
        if let Some(rec) = self.answered_get(repo, number)? {
            if same_head(&rec.head, head) {
                return Ok(Some(rec));
            }
            self.answered_drop(repo, number)?;
        }
        let ready = cards.iter().find(|c| {
            c.kind == "gate"
                && c.worker.is_none()
                && c.pr.as_ref().is_some_and(|p| {
                    p.ready && p.after.is_some() && p.head.as_deref().is_some_and(|h| same_head(h, head))
                })
        });
        let Some((card, p)) = ready.and_then(|c| c.pr.as_ref().map(|p| (c, p))) else { return Ok(None) };
        let Some(ticket) = card.ticket.as_deref() else { return Ok(None) };
        if self.store.link_get(ticket)?.is_none_or(|t| t.closed || t.parent != BOARD) {
            return Ok(None);
        }
        let Some(w) = self.worker_of(ticket, repo, number)? else { return Ok(None) };
        Ok(Some(Answered {
            repo: repo.into(),
            pr: number,
            head: head.into(),
            ticket: ticket.into(),
            worker: w.child,
            worker_label: w.label,
            answer: String::new(),
            card: card.id.clone(),
            method: None,
            approved: Vec::new(),
            after: p.after.clone().unwrap_or_default(),
            approval: None,
        }))
    }

    pub(crate) async fn after_merge(&self, rec: &Answered, commit: &str) -> Result<Vec<String>> {
        let done = |to: &str, what: &str| self.store.pr_event_seen(&rec.repo, rec.pr, &format!("{DONE}{what}"), to);
        let mark = |to: &str, what: &str| {
            self.store.pr_event_mark(&rec.repo, rec.pr, &format!("{DONE}{what}"), to, &self.now()).map(|_| ())
        };
        if done(&rec.worker, "")? {
            return Ok(Vec::new());
        }
        let branch = self.delete_branch(&rec.repo, rec.pr).await;
        let _g = self.lock.lock().await;
        if done(&rec.worker, "")? {
            return Ok(Vec::new());
        }
        let parent = self.store.link_get(&rec.worker)?.map(|l| l.parent).filter(|p| p != BOARD);
        let ticket = parent.as_deref().unwrap_or(&rec.ticket);
        let pr_left: Vec<&str> = rec.after.iter().filter(|s| s.scope == "pr").map(|s| s.id.as_str()).collect();
        let handoff = format!("Merged #{} at {commit}. Hand off.", rec.pr);
        let mut flush = Vec::new();
        let worker_line = match self.store.link_get(&rec.worker)?.filter(|l| !l.closed) {
            Some(mut wl) if pr_left.is_empty() => {
                if !done(&rec.worker, ":handoff")? {
                    self.queue_worker_reply(&rec.worker, &handoff)?;
                    mark(&rec.worker, ":handoff")?;
                }
                if wl.gate.as_deref() == Some("merge") {
                    wl.gate = None;
                    wl.gate_since = None;
                    self.store.link_save(&wl)?;
                }
                flush.push(rec.worker.clone());
                format!("sent {} {}", rec.worker_label, json!(handoff))
            }
            Some(_) => format!(
                "has not told {}, which still owns the PR's {} stage; send it \"Delivered: <summary>. Hand off.\" once the PR's stages finish",
                rec.worker_label,
                listed(&pr_left)
            ),
            None => format!("found {}'s link closed, so it told no worker", rec.worker_label),
        };
        let what = rec.merged(&*self.host, commit);
        let ticket_open = self.store.link_get(ticket)?.is_some_and(|t| !t.closed);
        if done(ticket, ":ticket")? {
        } else if rec.after.is_empty() || !ticket_open {
            self.handled(&rec.worker, ticket, &format!("{what}; t3cm {branch} and {worker_line}"))?;
            mark(ticket, ":ticket")?;
        } else {
            let stages: Vec<String> = rec.after.iter().map(|s| format!("{} ({})", s.id, s.scope)).collect();
            let ids: Vec<&str> = rec.approved.iter().map(String::as_str).collect();
            let approved = if ids.is_empty() { "none of them".to_string() } else { listed(&ids) };
            let body = format!(
                "{} is merged: {what}. t3cm {branch} and {worker_line}.\nRemaining delivery stages for this PR, in order: {}. \
                 The user's answer approved {approved}. Run them as \"Delivery stages\" says.",
                rec.worker_label,
                stages.join(", ")
            );
            let now = self.now();
            let h = self.store.msg_new(WATCHER, ticket, "event", &body, &now)?;
            let text =
                format!("{}\n{}", header("event", WATCHER, &h, None), capped(&body, &h, self.config().reply_cap));
            self.store.outbox_push(ticket, &text, &h, &now)?;
            mark(ticket, ":ticket")?;
            flush.push(ticket.to_string());
        }
        let own: Vec<i64> = self
            .cards()?
            .into_iter()
            .filter(|c| {
                c.kind == "gate"
                    && c.worker.is_none()
                    && c.ticket.as_deref() == Some(ticket)
                    && c.pr.as_ref().is_some_and(|p| p.repo == rec.repo && p.number == rec.pr)
            })
            .map(|c| c.seq)
            .collect();
        let note = format!("merged at {}", short(commit));
        self.store.waiting_resolve_releasing(BOARD, &own, &[ticket], &note, &self.now())?;
        self.sync_merge_gate_pr(ticket)?;
        self.sync_relay_gate(ticket)?;
        self.store.merge_wait_remove(&rec.repo, rec.pr)?;
        self.answered_drop(&rec.repo, rec.pr)?;
        mark(&rec.worker, "")?;
        self.flow_soon();
        self.changed();
        Ok(flush)
    }

    async fn delete_branch(&self, repo: &str, pr: u64) -> String {
        let change = ChangeRef::of_key(repo, pr);
        let forge = change.forge.name();
        let what = match change.forge {
            Forge::Github => "the PR",
            Forge::Gitlab { .. } => "the merge request",
        };
        match self.host.delete_branch(&change).await {
            Branch::Unread(e) => format!("left the branch, as it could not read it on {forge} ({e})"),
            Branch::NotMerged(b) => format!("left branch {b}, as {forge} does not show {what} merged yet"),
            Branch::Fork(b) => format!("left branch {b}, which is on a fork"),
            Branch::Gone(b) => format!("found branch {b} already deleted on {forge}"),
            Branch::TipMoved { branch, tip } => {
                format!("left branch {branch}, as its tip {} is not the merged head", short(&tip))
            }
            Branch::Deleted(b) => format!("deleted branch {b}"),
            Branch::Failed { branch, why } => format!("could not delete branch {branch} ({why})"),
        }
    }
}
