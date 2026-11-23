use std::collections::HashSet;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::board::{BOARD, Issue, LaunchArgs, parse_github_issue_only};
use crate::clock::local_clock;
use crate::store::NewIssue;
use crate::wf::Wf;

const MAX_TITLE: usize = 300;
const MAX_NOTE: usize = 2000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueArgs {
    pub url: String,
    pub title: Option<String>,
    pub note: Option<String>,
}

pub fn issue_key(i: &Issue) -> String {
    i.slug().to_ascii_lowercase()
}

fn text(s: Option<String>, what: &str, max: usize) -> Result<Option<String>> {
    let Some(s) = s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else { return Ok(None) };
    if s.chars().count() > max {
        bail!("wf.issue: {what} is over {max} chars");
    }
    Ok(Some(s))
}

fn seq_of(id: &str) -> Result<i64> {
    id.strip_prefix('n')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| anyhow!("no new issue {id:?}; new issue ids look like n3"))
}

pub fn issue_view(seq: i64, i: &NewIssue) -> Value {
    let mut v = json!(i);
    v["id"] = json!(format!("n{seq}"));
    v["atLocal"] = json!(local_clock(&i.at));
    v["dismissed"] = json!(i.dismissed_at.is_some());
    v["dismissedAtLocal"] = json!(i.dismissed_at.as_deref().map(local_clock));
    v
}

fn matches_github_ticket(ticket_issue: Option<&str>, ticket_label: &str, issue: &Issue) -> bool {
    match ticket_issue {
        Some(raw) => parse_github_issue_only(raw).is_some_and(|gh| gh.slug().eq_ignore_ascii_case(&issue.slug())),
        None => ticket_label.eq_ignore_ascii_case(&issue.label()),
    }
}

fn github_ticket_key(ticket_issue: Option<&str>, ticket_label: &str) -> Option<String> {
    match ticket_issue {
        Some(raw) => parse_github_issue_only(raw).map(|gh| gh.slug().to_ascii_lowercase()),
        None => Some(ticket_label.to_ascii_lowercase()),
    }
}

impl Wf {
    fn ticket_for(&self, issue: &Issue) -> Result<Option<Value>> {
        if let Some(l) = self
            .store
            .children(BOARD, false)?
            .into_iter()
            .find(|l| matches_github_ticket(l.identity.issue.as_deref(), &l.label, issue))
        {
            let link = l.link.clone().or_else(|| self.live_of(&l.child)?["link"].as_str().map(String::from));
            return Ok(Some(json!({"label": l.label, "thread": l.child, "link": link, "done": l.closed})));
        }
        Ok(self
            .store
            .pending_all()?
            .into_iter()
            .find(|p| p.parent == BOARD && matches_github_ticket(p.identity.issue.as_deref(), &p.label, issue))
            .map(|p| json!({"label": p.label, "thread": p.thread, "link": p.link, "done": false})))
    }

    fn ticket_keys(&self) -> Result<HashSet<String>> {
        let mut out: HashSet<String> = self
            .store
            .children(BOARD, false)?
            .into_iter()
            .filter_map(|l| github_ticket_key(l.identity.issue.as_deref(), &l.label))
            .collect();
        out.extend(
            self.store
                .pending_all()?
                .into_iter()
                .filter(|p| p.parent == BOARD)
                .filter_map(|p| github_ticket_key(p.identity.issue.as_deref(), &p.label)),
        );
        Ok(out)
    }

    fn already_known(&self, issue: &Issue) -> Result<Option<Value>> {
        let url = issue.reference();
        if let Some(t) = self.ticket_for(issue)? {
            let message = format!("{url} already has a ticket on the board ({}); nothing added", t["label"]);
            return Ok(Some(json!({"added": false, "reason": "ticket", "ticket": t, "message": message})));
        }
        if let Some((seq, i)) = self.store.new_issue_by_key(&issue_key(issue))? {
            let message = match i.dismissed_at {
                Some(_) => format!("{url} was already reported as n{seq} and is dismissed; nothing added"),
                None => format!("{url} is already on the board's new issues as n{seq}; nothing added"),
            };
            return Ok(Some(
                json!({"added": false, "reason": "reported", "issue": issue_view(seq, &i), "message": message}),
            ));
        }
        Ok(None)
    }

    async fn thread_meta(&self, thread: &str) -> (Option<String>, Option<String>) {
        let th =
            self.t3.call("t3_thread_read", json!({"threadId": thread, "limit": 1, "maxCharsPerItem": 1})).await.ok();
        let field = |k: &str| th.as_ref().and_then(|r| r["thread"][k].as_str()).map(String::from);
        (field("link"), field("title"))
    }

    async fn backfill_thread(&self, key: &str) -> Result<()> {
        let Some((_, i)) = self.store.new_issue_by_key(key)? else { return Ok(()) };
        if i.thread_link.is_some() && i.thread_title.is_some() {
            return Ok(());
        }
        let (link, title) = self.thread_meta(&i.thread).await;
        let _g = self.lock.lock().await;
        let Some((seq, mut i)) = self.store.new_issue_by_key(key)? else { return Ok(()) };
        let before = (i.thread_link.clone(), i.thread_title.clone());
        i.thread_link = i.thread_link.or(link);
        i.thread_title = i.thread_title.or(title);
        if (i.thread_link.clone(), i.thread_title.clone()) != before {
            self.store.new_issue_update(seq, &i)?;
            self.changed();
        }
        Ok(())
    }

    pub async fn report_issue(&self, me: &str, a: IssueArgs) -> Result<Value> {
        let issue = parse_github_issue_only(&a.url).ok_or_else(|| {
            anyhow!(
                "wf.issue: {:?} is not a GitHub issue; use https://github.com/<owner>/<repo>/issues/<n> or <owner>/<repo>#<n>",
                a.url
            )
        })?;
        let (owner, repo, number) = issue.github_parts().expect("GitHub issue has owner, repo and number");
        self.backfill_thread(&issue_key(&issue)).await?;
        if let Some(v) = self.already_known(&issue)? {
            return Ok(v);
        }
        let title = text(a.title, "title", MAX_TITLE)?;
        let note = text(a.note, "note", MAX_NOTE)?;
        let (thread_link, thread_title) = self.thread_meta(me).await;
        let entry = NewIssue {
            url: issue.reference().to_string(),
            repo: format!("{owner}/{repo}"),
            number,
            label: issue.label(),
            title,
            note,
            thread: me.to_string(),
            thread_link,
            thread_title,
            at: self.now(),
            dismissed_at: None,
            restored_at: None,
        };
        let _g = self.lock.lock().await;
        if let Some(v) = self.already_known(&issue)? {
            return Ok(v);
        }
        let seq = self
            .store
            .new_issue_add(&issue_key(&issue), &entry)?
            .ok_or_else(|| anyhow!("wf.issue: {} was reported at the same moment; nothing added", entry.url))?;
        let message = format!("{} is on the board's new issues as n{seq}", entry.url);
        Ok(json!({"added": true, "issue": issue_view(seq, &entry), "message": message}))
    }

    pub fn new_issues(&self) -> Result<(Vec<Value>, Vec<Value>)> {
        let tickets = self.ticket_keys()?;
        let (mut open, mut dismissed) = (Vec::new(), Vec::new());
        for (seq, i) in self.store.new_issues()?.into_iter().rev() {
            let slug = format!("{}#{}", i.repo, i.number).to_ascii_lowercase();
            if tickets.contains(&slug) || tickets.contains(&i.label.to_ascii_lowercase()) {
                continue;
            }
            let v = issue_view(seq, &i);
            if i.dismissed_at.is_some() { dismissed.push(v) } else { open.push(v) }
        }
        Ok((open, dismissed))
    }

    fn new_issue_row(&self, id: &str) -> Result<(i64, NewIssue)> {
        let seq = seq_of(id)?;
        let i = self.store.new_issue_get(seq)?.ok_or_else(|| anyhow!("no new issue {id:?}"))?;
        Ok((seq, i))
    }

    pub async fn launch_new_issue(&self, id: &str) -> Result<Value> {
        let (_, i) = self.new_issue_row(id)?;
        let args = LaunchArgs {
            issue: i.url,
            parent_issue: None,
            part: None,
            project: None,
            title: i.title,
            model: None,
            effort: None,
        };
        let mut r = self.launch_ticket(args).await?;
        r["id"] = json!(id);
        Ok(r)
    }

    pub async fn dismiss_new_issue(&self, id: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let (seq, mut i) = self.new_issue_row(id)?;
        if i.dismissed_at.is_some() {
            bail!("new issue {id} is already dismissed");
        }
        i.dismissed_at = Some(self.now());
        self.store.new_issue_update(seq, &i)?;
        self.changed();
        Ok(json!({"id": id, "dismissed": true}))
    }

    pub async fn restore_new_issue(&self, id: &str) -> Result<Value> {
        let _g = self.lock.lock().await;
        let (seq, mut i) = self.new_issue_row(id)?;
        if i.dismissed_at.is_none() {
            bail!("new issue {id} is not dismissed");
        }
        i.dismissed_at = None;
        i.restored_at = Some(self.now());
        self.store.new_issue_update(seq, &i)?;
        self.changed();
        Ok(json!({"id": id, "dismissed": false}))
    }
}
