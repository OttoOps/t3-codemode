use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{BOARD, Card, NewCard, gh_name};
use crate::clock::{local_clock, parse_ts};
use crate::codehost::{Change, ChangeRef, CodeHost, Landed, Landing, Read, State};
use crate::config::{ForgeKind, Forges};
use crate::merge::{Ready, checks, ready};
use crate::reach::ReachPoint;
use crate::sentinels::Sentinel;
use crate::store::{MergeWait, PrSeen, WaitStatus, Wake};
use crate::wf::{Wf, header};

pub const GITHUB: &str = "github";
pub const LAST_CHECK: &str = "reconcile_last";
pub const SINCE: &str = "reconcile_since";
const BATCH: usize = 100;
const PAGE_LOAD_GAP_MINS: f64 = 0.5;
const MAX_FAILURES: u32 = 3;
const EXIT_WINDOW_MINS: f64 = 30.0;
const HELD: &str = "has a failing check at its approved head";
const NO_REASON: &str = "no reason recorded";

fn no_ticket(forge: &str) -> String {
    format!("no ticket tracks it to take a reply, so act on it on {forge} or with wf.merge and then dismiss this card")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardPr {
    pub repo: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Vec<CardStage>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CardStage {
    pub id: String,
    pub scope: String,
}

pub fn pr_refs(text: &str) -> Vec<(String, String, u64)> {
    let mut out: Vec<(String, String, u64)> = Vec::new();
    for (i, m) in text.match_indices("github.com/") {
        if !at_host_start(&text[..i]) {
            continue;
        }
        let rest = &text[i + m.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.')))
            .unwrap_or(rest.len());
        let parts: Vec<&str> = rest[..end].split('/').collect();
        if let [owner, repo, "pull", n, ..] = parts.as_slice()
            && gh_name(owner)
            && gh_name(repo)
            && let Some(n) = n.trim_end_matches('.').parse::<u64>().ok().filter(|n| (1..=i32::MAX as u64).contains(n))
            && !out.iter().any(|(o, r, k)| o.eq_ignore_ascii_case(owner) && r.eq_ignore_ascii_case(repo) && *k == n)
        {
            out.push((owner.to_string(), repo.to_string(), n));
        }
    }
    out
}

pub fn change_refs(text: &str, forges: &Forges) -> Vec<ChangeRef> {
    let mut out: Vec<ChangeRef> = pr_refs(text)
        .into_iter()
        .map(|(o, r, n)| ChangeRef::github(&format!("{o}/{r}").to_ascii_lowercase(), n))
        .collect();
    for (host, _) in forges.iter().filter(|(_, f)| f.kind == ForgeKind::Gitlab) {
        merge_request_refs(text, host, &mut out);
    }
    out
}

pub(crate) fn gl_name(s: &str) -> bool {
    !s.is_empty() && s != "-" && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn merge_request_refs(text: &str, host: &str, out: &mut Vec<ChangeRef>) {
    let lower = text.to_ascii_lowercase();
    let needle = format!("{}/", host.to_ascii_lowercase());
    for (i, m) in lower.match_indices(&needle) {
        if lower[..i].ends_with("www.") || !at_host_start(&lower[..i]) {
            continue;
        }
        let rest = &lower[i + m.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.')))
            .unwrap_or(rest.len());
        let Some((path, tail)) = rest[..end].split_once("/-/merge_requests/") else { continue };
        let n = tail.split('/').next().unwrap_or_default().trim_end_matches('.');
        if path.split('/').count() >= 2
            && path.split('/').all(gl_name)
            && let Some(n) = n.parse::<u64>().ok().filter(|n| (1..=i32::MAX as u64).contains(n))
        {
            let r = ChangeRef::gitlab(host, path, n);
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
}

fn at_host_start(before: &str) -> bool {
    let before = before.strip_suffix("www.").unwrap_or(before);
    before.ends_with("://")
        || before
            .chars()
            .last()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '@')))
}

fn approved_head(body: &str) -> Option<String> {
    let words: Vec<&str> = body.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    words.windows(2).find_map(|w| {
        let sha = w[1];
        let hex = sha.chars().all(|c| c.is_ascii_hexdigit()) && sha.chars().any(|c| c.is_ascii_digit());
        (w[0].eq_ignore_ascii_case("head") && (7..=40).contains(&sha.len()) && hex).then(|| sha.to_ascii_lowercase())
    })
}

pub fn card_pr(body: &str, forges: &Forges) -> Option<CardPr> {
    let action: Vec<&str> = body.lines().filter(|l| l.contains("ACTION NEEDED")).collect();
    let mut refs = change_refs(&action.join("\n"), forges);
    if refs.len() != 1 {
        refs = change_refs(body, forges);
    }
    let [r] = refs.as_slice() else { return None };
    Some(CardPr { repo: r.key(), number: r.number, head: approved_head(body), ready: false, after: None })
}

fn readiness(seen: &PrSeen) -> Value {
    json!({
        "isDraft": seen.draft,
        "mergeStateStatus": seen.merge_state,
        "reviewDecision": seen.review_decision,
        "checks": seen.checks,
    })
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn why_dropped(reason: &str) -> String {
    match reason {
        "merge_conflict" => "a merge conflict".into(),
        "failed_checks" => "failed checks".into(),
        "manual" => "removed by hand".into(),
        "" => "no reason given".into(),
        other => other.replace('_', " "),
    }
}

fn later(event: &str, since: &str) -> bool {
    match (parse_ts(event), parse_ts(since)) {
        (Some(e), Some(s)) => e >= s,
        _ => false,
    }
}

#[derive(Default)]
struct Tracked {
    repo: String,
    number: u64,
    since: String,
    tickets: Vec<(String, String)>,
    cards: Vec<i64>,
    approval: Option<String>,
    unnamed: Vec<String>,
}

impl Tracked {
    fn label(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    fn change(&self) -> ChangeRef {
        ChangeRef::of_key(&self.repo, self.number)
    }

    fn forge(&self) -> &'static str {
        self.change().forge.name()
    }

    fn sender(&self) -> &'static str {
        self.change().forge.sender()
    }

    fn url(&self, host: &dyn CodeHost) -> String {
        host.url(&self.change())
    }

    fn add(&mut self, ticket: Option<&str>, since: &str) {
        if since.is_empty() {
            return;
        }
        if self.since.is_empty() || since < self.since.as_str() {
            self.since = since.into();
        }
        let Some(t) = ticket.filter(|t| *t != BOARD && !t.is_empty()) else { return };
        match self.tickets.iter_mut().find(|(x, _)| x == t) {
            Some((_, s)) if since < s.as_str() => *s = since.into(),
            Some(_) => {}
            None => self.tickets.push((t.into(), since.into())),
        }
    }

    fn pr(&self, head: &str) -> CardPr {
        CardPr { repo: self.repo.clone(), number: self.number, head: Some(head.into()), ..CardPr::default() }
    }
}

struct Ask {
    what: String,
    reply: &'static str,
    context: String,
}

fn exit_ask(what: String, context: String) -> Ask {
    Ask { what, reply: "reply \"requeue\" to queue it again, or \"hold\" to leave it out", context }
}

#[derive(Default)]
struct Pass {
    actions: Vec<String>,
    woken: Vec<String>,
}

struct OpenOutcome {
    advance: Option<MergeWait>,
    clear_left: bool,
}

fn add<'a>(map: &'a mut BTreeMap<(String, u64), Tracked>, repo: &str, n: u64) -> &'a mut Tracked {
    let repo = repo.to_ascii_lowercase();
    map.entry((repo.clone(), n)).or_insert_with(|| Tracked { repo, number: n, ..Tracked::default() })
}

impl Wf {
    pub fn pr_view(&self, pr: &Value) -> Value {
        if !pr.is_object() {
            return pr.clone();
        }
        crate::board::with_verdict(self.pr_seen_view(pr))
    }

    fn pr_seen_view(&self, pr: &Value) -> Value {
        let mut pr = pr.clone();
        if pr["state"] == "open"
            && let Some(r) = pr["url"].as_str().and_then(|u| change_refs(u, &self.config().forges).into_iter().next())
            && let Ok(Some(seen)) = self.store.pr_seen_get(&r.key(), r.number)
            && seen.state == "OPEN"
        {
            if seen.queued {
                pr["state"] = json!("queued");
            }
            if seen.merge_state.is_some() {
                pr["readiness"] = readiness(&seen);
            }
        }
        pr
    }

    pub async fn reconcile_if_due(&self) -> Result<Vec<String>> {
        let config = self.config();
        let b = &config.board;
        if b.reconcile_idle_minutes == 0 {
            return Ok(Vec::new());
        }
        let active = !self.store.merge_waits()?.is_empty()
            || self.store.pr_seen_any_queued()?
            || !self.ready_waiting()?.is_empty();
        let every = if active { b.reconcile_active_minutes } else { b.reconcile_idle_minutes } as f64;
        self.reconcile_after(every).await
    }

    pub async fn reconcile_soon(&self) -> Result<Vec<String>> {
        if self.config().board.reconcile_idle_minutes == 0 {
            return Ok(Vec::new());
        }
        self.reconcile_after(PAGE_LOAD_GAP_MINS).await
    }

    pub async fn reconcile(&self) -> Result<Vec<String>> {
        self.reconcile_after(0.0).await
    }

    pub async fn reconcile_ran(&self) -> Result<Option<Vec<String>>> {
        self.reconcile_pass(0.0).await
    }

    async fn reconcile_after(&self, gap_mins: f64) -> Result<Vec<String>> {
        Ok(self.reconcile_pass(gap_mins).await?.unwrap_or_default())
    }

    async fn reconcile_pass(&self, gap_mins: f64) -> Result<Option<Vec<String>>> {
        let Ok(_running) = self.reconcile_lock.try_lock() else { return Ok(None) };
        if gap_mins > 0.0 && self.mins(self.store.meta_get(LAST_CHECK)?.as_deref()).is_some_and(|m| m < gap_mins) {
            return Ok(Some(Vec::new()));
        }
        self.store.meta_set(LAST_CHECK, &self.now())?;
        self.learn_card_prs()?;
        let floor = self.store.meta_get(SINCE)?.unwrap_or_default();
        let tracked = self.tracked(&floor)?;
        let mut pass = Pass::default();
        let mut failed = Vec::new();
        let mut unread: Vec<(String, u64)> = Vec::new();
        let list: Vec<&Tracked> = tracked.values().collect();
        for chunk in list.chunks(BATCH) {
            let refs: Vec<ChangeRef> = chunk.iter().map(|t| t.change()).collect();
            let read_at = self.now();
            let reads = self.host.read(&refs).await?;
            if reads.len() != chunk.len() {
                bail!("the code host returned {} reads for {} pull requests", reads.len(), chunk.len());
            }
            for (t, read) in chunk.iter().zip(reads) {
                let done = match read {
                    Read::Found(o) => self.fix(t, *o, &read_at, &floor, &mut pass).await,
                    Read::NotFound(why) => self.missing(t, &why, &mut pass).await,
                    Read::Failed(why) => Err(anyhow!("{why}")),
                };
                if let Err(e) = done {
                    unread.push((t.repo.clone(), t.number));
                    failed.push(format!("{}: {e:#}", t.label()));
                }
            }
        }
        for row in self.store.pr_seen_all()? {
            if !tracked.contains_key(&(row.repo.clone(), row.pr)) {
                self.store.pr_seen_remove(&row.repo, row.pr)?;
            }
        }
        for (worker, p) in self.ready_waiting()? {
            if unread.contains(&(p.repo.clone(), p.number)) {
                continue;
            }
            let due = match self.store.pr_seen_get(&p.repo, p.number)? {
                None => true,
                Some(s) => {
                    s.state != "OPEN"
                        || s.draft
                        || !crate::wf_pr::same_head(&s.head, &p.head)
                        || matches!(s.checks.as_deref(), None | Some("failing"))
                        || (s.checks.as_deref() != Some("pending") && s.merge_state.as_deref() != Some("UNKNOWN"))
                }
            };
            if !due {
                continue;
            }
            self.reach.hit(ReachPoint::ReadyWaitingRetried);
            match self.retry_ready(&worker, &p).await {
                Ok((action, told)) => {
                    pass.actions.push(action);
                    if told {
                        pass.woken.push(worker.child.clone());
                    }
                }
                Err(e) => failed.push(format!("{}#{}: {e:#}", p.repo, p.number)),
            }
        }
        pass.woken.sort();
        pass.woken.dedup();
        for t in &pass.woken {
            self.flush_to(t).await?;
        }
        self.flow_due.store(false, std::sync::atomic::Ordering::Release);
        match self.flow_pick(None).await {
            Ok(v) => pass.actions.extend(
                v["picked"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["label"].as_str())
                    .map(|l| format!("flow picked {l}")),
            ),
            Err(e) => failed.push(format!("flow: {e:#}")),
        }
        if !pass.actions.is_empty() {
            self.changed();
        }
        if !failed.is_empty() {
            if !pass.actions.is_empty() {
                tracing::info!(actions = ?pass.actions, "pull request reconcile");
            }
            bail!("pull request reconcile failed for {}", failed.join("; "));
        }
        Ok(Some(pass.actions))
    }

    fn learn_card_prs(&self) -> Result<()> {
        for (_, data, _, seq) in self.store.waiting_data(BOARD, false)? {
            let Some(mut data) = data.filter(|d| d.get("pr").is_none()) else { continue };
            let body = match data["handle"].as_str().map(|h| self.store.msg_get(h)).transpose()?.flatten() {
                Some(m) => m.body,
                None => continue,
            };
            data["pr"] = json!(card_pr(&body, &self.config().forges));
            self.store.waiting_set_data(BOARD, seq, &data)?;
        }
        Ok(())
    }

    fn tracked(&self, floor: &str) -> Result<BTreeMap<(String, u64), Tracked>> {
        let mut map = BTreeMap::new();
        let forges = self.config().forges.clone();
        for ln in self.store.links_open()?.into_iter().filter(|l| l.role == "worker") {
            let live = self.live_of(&ln.child);
            let urls: Vec<String> = live
                .as_ref()
                .map(|l| {
                    let prs = l["prs"].as_array().into_iter().flatten();
                    prs.chain([&l["pr"]["url"]]).filter_map(|u| u.as_str().map(String::from)).collect()
                })
                .unwrap_or_default();
            for r in urls.iter().flat_map(|u| change_refs(u, &forges)) {
                add(&mut map, &r.key(), r.number).add(Some(&ln.parent), &ln.opened);
            }
            if let Some(p) = ln.pr.as_ref().filter(|p| p.waiting) {
                add(&mut map, &p.repo, p.number).add(Some(&ln.parent), &ln.opened);
            }
            let shown = live
                .as_ref()
                .and_then(|l| l["pr"]["url"].as_str())
                .and_then(|u| change_refs(u, &forges).into_iter().next());
            if let Some(r) = shown.filter(|_| ln.identity.issue.is_none()) {
                add(&mut map, &r.key(), r.number).unnamed.push(ln.child.clone());
            }
        }
        for card in self.cards()?.into_iter().filter(|c| c.worker.is_none()) {
            if let Some(p) = &card.pr {
                let t = add(&mut map, &p.repo, p.number);
                t.add(card.ticket.as_deref(), &card.at);
                t.cards.push(card.seq);
            }
        }
        for w in self.store.merge_waits()? {
            let t = add(&mut map, &w.repo, w.pr);
            t.add(w.thread.as_deref(), &w.at);
            t.approval = Some(w.approval);
        }
        for row in self.store.pr_seen_all()? {
            let live = (row.queued || row.left.is_some()) && row.state == "OPEN";
            if !live && !map.contains_key(&(row.repo.clone(), row.pr)) {
                continue;
            }
            let t = add(&mut map, &row.repo, row.pr);
            t.add(None, &row.since);
            for (ticket, since) in &row.tickets {
                t.add(Some(ticket), since);
            }
        }
        for t in map.values_mut() {
            if t.since.as_str() < floor {
                t.since = floor.into();
            }
            for (_, s) in t.tickets.iter_mut().filter(|(_, s)| s.as_str() < floor) {
                *s = floor.into();
            }
        }
        Ok(map)
    }

    async fn name_workers(&self, t: &Tracked, o: &Change) -> Result<()> {
        let Some(issue) = o.closing_issue.clone().filter(|_| !t.unnamed.is_empty()) else { return Ok(()) };
        let mut named = false;
        let forges = self.config().forges.clone();
        {
            let _g = self.lock.lock().await;
            for w in &t.unnamed {
                let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
                let shown = live.get(w).and_then(|(_, l)| {
                    l["pr"]["url"].as_str().and_then(|u| change_refs(u, &forges).into_iter().next())
                });
                if !shown.is_some_and(|r| r.number == t.number && r.key() == t.repo) {
                    continue;
                }
                if let Some(mut ln) = self.store.link_get(w)?.filter(|l| !l.closed && l.identity.issue.is_none()) {
                    ln.identity.issue = Some(issue.clone());
                    self.store.link_save(&ln)?;
                    named = true;
                }
            }
        }
        if named {
            self.changed();
        }
        Ok(())
    }

    fn live_tickets(
        &self,
        t: &Tracked,
        cards: &[Card],
        wait: Option<&MergeWait>,
        prev: Option<&PrSeen>,
        floor: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add_ticket = |ticket: Option<&str>, since: &str| {
            let Some(ticket) = ticket.filter(|x| *x != BOARD && !x.is_empty()) else { return };
            if since.is_empty() {
                return;
            }
            match out.iter_mut().find(|(x, _)| x == ticket) {
                Some((_, s)) if since < s.as_str() => *s = since.into(),
                Some(_) => {}
                None => out.push((ticket.into(), since.into())),
            }
        };
        for c in cards {
            add_ticket(c.ticket.as_deref(), &c.at);
        }
        let forges = self.config().forges.clone();
        if let Some(w) = wait {
            add_ticket(w.thread.as_deref(), &w.at);
        }
        for wl in self.store.links_open()?.into_iter().filter(|l| l.role == "worker") {
            let pr_matches = wl.pr.as_ref().is_some_and(|p| {
                (p.repo == t.repo && p.number == t.number)
                    || p.followups.iter().any(|(r, n)| r == &t.repo && *n == t.number)
            });
            let live_matches = !pr_matches
                && self.live_of(&wl.child).is_some_and(|l| {
                    let prs = l["prs"].as_array().into_iter().flatten();
                    prs.chain([&l["pr"]["url"]])
                        .filter_map(Value::as_str)
                        .flat_map(|u| change_refs(u, &forges))
                        .any(|r| r.number == t.number && r.key() == t.repo)
                });
            if pr_matches || live_matches {
                add_ticket(Some(&wl.parent), &wl.opened);
            }
        }
        if !self.sentinel(Sentinel::ReconcileSkipsIndependentPrSeen)
            && let Some(p) = prev
        {
            for (ticket, since) in &p.tickets {
                add_ticket(Some(ticket), since);
            }
        }
        for (_, s) in out.iter_mut().filter(|(_, s)| s.as_str() < floor) {
            *s = floor.into();
        }
        Ok(out)
    }

    async fn fix(&self, t: &Tracked, o: Change, read_at: &str, floor: &str, pass: &mut Pass) -> Result<()> {
        self.name_workers(t, &o).await?;
        let (advance, merged) = {
            let _g = self.lock.lock().await;
            if !self.tracked(floor)?.contains_key(&(t.repo.clone(), t.number)) {
                self.reach.hit(ReachPoint::ReconcileDroppedUntracked);
                self.store.pr_seen_remove(&t.repo, t.number)?;
                return Ok(());
            }
            let cards: Vec<Card> = self.cards()?.into_iter().filter(|c| t.cards.contains(&c.seq)).collect();
            let wait = self.wait(t)?;
            let sent = wait.as_ref().filter(|w| w.status == WaitStatus::Sent).map(|w| w.at.clone());
            let prev = self.store.pr_seen_get(&t.repo, t.number)?;
            let live_tickets = self.live_tickets(t, &cards, wait.as_ref(), prev.as_ref(), floor)?;
            if self.reach.take_adopted(&t.repo, t.number, &live_tickets) {
                self.reach.hit(ReachPoint::ReconcileAdoptedPr);
            }
            let since = live_tickets
                .iter()
                .map(|(_, s)| s.as_str())
                .chain(std::iter::once(t.since.as_str()))
                .filter(|s| !s.is_empty())
                .min()
                .unwrap_or(t.since.as_str());
            let mut left = if o.state != State::Open || o.queued {
                None
            } else if prev.as_ref().is_some_and(|p| p.queued) {
                Some(self.now())
            } else {
                let kept = prev.as_ref().and_then(|p| p.left.clone());
                match &sent {
                    Some(approved) => kept.filter(|l| later(l, approved)).or_else(|| Some(self.now())),
                    None => kept,
                }
            };
            let after = match prev.as_ref().and_then(|p| p.queued_at.as_deref()) {
                Some(q) if later(q, since) => q,
                _ => since,
            };
            let mut merged = None;
            let advance = match o.state {
                State::Merged | State::Closed => {
                    merged = self.finished(t, &o, &cards, wait.as_ref(), &live_tickets, since, pass)?;
                    None
                }
                State::Open => {
                    if prev.as_ref().is_some_and(|p| p.state == State::Closed.as_str()) {
                        self.reach.hit(ReachPoint::PrReopenedReconciled);
                    }
                    let outcome = self.open(t, &o, &cards, wait, &live_tickets, after, left.as_deref(), pass)?;
                    if outcome.clear_left {
                        left = None;
                    }
                    outcome.advance
                }
            };
            let queued_at = match &prev {
                _ if o.state == State::Open && o.queued => Some(read_at.to_string()),
                Some(p) => p.queued_at.clone(),
                None => None,
            };
            let left = match left {
                Some(l) => {
                    let status = self.wait(t)?.map(|w| w.status);
                    self.exited(t, &o, &l, &live_tickets, after, status, pass)?
                }
                None => None,
            };
            let seen = PrSeen {
                repo: t.repo.clone(),
                pr: t.number,
                state: o.state.as_str().into(),
                head: o.head.clone(),
                queued: o.queued,
                since: since.to_string(),
                tickets: live_tickets,
                at: self.now(),
                left,
                queued_at,
                draft: o.draft,
                merge_state: Some(o.merge_state.clone()),
                review_decision: o.review_decision.clone(),
                checks: checks(&o).map(String::from),
                rev: 0,
            };
            let shown = |p: &PrSeen| (p.state.clone(), p.queued, readiness(p));
            let moved = prev.as_ref().map(shown) != Some(shown(&seen));
            self.store.pr_seen_put(&seen)?;
            if moved {
                self.changed();
            }
            (advance, merged)
        };
        if let Some(rec) = merged {
            pass.woken.extend(self.after_merge(&rec, o.commit()).await?);
        }
        match advance {
            Some(w) => self.advance(t, &o, w, read_at, pass).await,
            None => Ok(()),
        }
    }

    async fn missing(&self, t: &Tracked, why: &str, pass: &mut Pass) -> Result<()> {
        let _g = self.lock.lock().await;
        self.store.pr_seen_remove(&t.repo, t.number)?;
        if let Some(w) = self.wait(t)? {
            self.failed(t, w, why, None, pass)?;
        }
        Ok(())
    }

    fn wait(&self, t: &Tracked) -> Result<Option<MergeWait>> {
        let w = self.store.merge_wait_get(&t.repo, t.number)?;
        Ok(w.filter(|w| t.approval.as_ref() == Some(&w.approval)))
    }

    #[allow(clippy::too_many_arguments)]
    fn finished(
        &self,
        t: &Tracked,
        o: &Change,
        cards: &[Card],
        wait: Option<&MergeWait>,
        live_tickets: &[(String, String)],
        since: &str,
        pass: &mut Pass,
    ) -> Result<Option<crate::merge_answer::Answered>> {
        let merged = o.state == State::Merged;
        let end = o.ended.clone().unwrap_or_else(|| self.now());
        let when = local_clock(&end);
        let forge = t.forge();
        let note = if merged {
            format!("merged on {forge} at {}, {when}", short(o.commit()))
        } else {
            format!("closed on {forge} without merging, {when}")
        };
        let stale: Vec<&Card> =
            cards.iter().filter(|c| parse_ts(&c.at).zip(parse_ts(&end)).is_none_or(|(a, e)| a <= e)).collect();
        let code = match merged {
            true if later(&end, since) => self.code_merged(&t.repo, t.number, &o.head, cards)?,
            true => None,
            false => {
                self.answered_drop(&t.repo, t.number)?;
                None
            }
        };
        let event = match (merged, o.commit()) {
            (true, "") => format!("merged:{end}"),
            (true, commit) => format!("merged:{commit}"),
            (false, _) => format!("closed:{end}"),
        };
        let body = if merged {
            format!(
                "{} merged on {forge} at {}, {when} (merge commit {}). Run your post-merge steps. {}",
                t.label(),
                short(o.commit()),
                o.commit(),
                t.url(&*self.host)
            )
        } else {
            format!("{} was closed on {forge} without merging, {when}. {}", t.label(), t.url(&*self.host))
        };
        let targets: Vec<&str> = live_tickets.iter().filter(|(_, s)| later(&end, s)).map(|(x, _)| x.as_str()).collect();
        if !later(&end, since) {
            self.reach.hit(if merged {
                ReachPoint::MergeEventPredatesTracking
            } else {
                ReachPoint::CloseEventPredatesTracking
            });
        }
        let mut told = 0;
        for ticket in &targets {
            if code.as_ref().is_some_and(|c| c.ticket == *ticket) {
                self.store.pr_event_mark(&t.repo, t.number, &event, ticket, &self.now())?;
            } else if self.store.link_get(ticket)?.is_some_and(|l| !l.closed) {
                told += self.wake(t, &event, ticket, "event", &body, pass)? as usize;
            }
        }
        let on_feed = told == 0
            && code.is_none()
            && later(&end, since)
            && (wait.is_some() || !stale.is_empty())
            && self.store.pr_event_mark(&t.repo, t.number, &event, BOARD, &self.now())?;
        let seqs: Vec<i64> = stale.iter().map(|c| c.seq).collect();
        let gated: Vec<&str> = stale.iter().filter(|c| c.kind == "gate").filter_map(|c| c.ticket.as_deref()).collect();
        let resolved = self.resolve_releasing(&seqs, &gated, &note)?;
        if let Some(w) = wait {
            self.retire_merge_wait(w)?;
        }
        if on_feed {
            self.feed_note(t, None, &body)?;
        } else if told == 0 && resolved > 0 {
            let who = targets.first().copied().or_else(|| live_tickets.first().map(|(x, _)| x.as_str()));
            self.feed_note(t, who, &format!("{}: {note}", t.label()))?;
        }
        if told > 0 || on_feed || resolved > 0 || code.is_some() {
            if later(&end, since) {
                self.reach.hit(ReachPoint::FinishedEventReported);
            }
            pass.actions.push(format!("{} {note}", t.label()));
        }
        Ok(code)
    }

    fn resolve_releasing(&self, seqs: &[i64], tickets: &[&str], note: &str) -> Result<usize> {
        let resolved = self.store.waiting_resolve_releasing(BOARD, seqs, tickets, note, &self.now())?;
        for ticket in tickets {
            self.sync_merge_gate_pr(ticket)?;
            self.sync_relay_gate(ticket)?;
        }
        if resolved > 0 {
            self.changed();
        }
        Ok(resolved)
    }

    #[allow(clippy::too_many_arguments)]
    fn open(
        &self,
        t: &Tracked,
        o: &Change,
        cards: &[Card],
        mut wait: Option<MergeWait>,
        live_tickets: &[(String, String)],
        after: &str,
        left: Option<&str>,
        pass: &mut Pass,
    ) -> Result<OpenOutcome> {
        if self.dropped(t, o, wait.as_ref(), live_tickets, after, pass)?
            && let Some(w) = wait.take()
        {
            self.retire_merge_wait(&w)?;
        }
        let this_pr = |c: &Card| c.pr.as_ref().is_some_and(|p| p.repo == t.repo && p.number == t.number);
        let behind = |c: &Card| {
            let held_at = c.pr.as_ref().and_then(|p| p.head.as_deref());
            held_at.is_some_and(|head| !crate::wf_pr::same_head(&o.head, head))
        };
        let note = format!("head moved to {} after this hold", short(&o.head));
        self.resolve_cards(|c| c.kind == "report" && this_pr(c) && c.text.contains(HELD) && behind(c), &note)?;
        let now = ready(o);
        if o.queued || now == Ready::Yes {
            let note = if o.queued { "it is in the merge queue" } else { "it is ready to merge" };
            let stale = |c: &Card| c.text.contains(HELD) || (o.queued && c.text.contains(NO_REASON));
            self.resolve_cards(|c| c.kind == "report" && this_pr(c) && stale(c), note)?;
        }
        let mut moved: Vec<(Option<String>, String, Vec<i64>)> = Vec::new();
        for c in cards.iter().filter(|c| c.kind == "gate") {
            let Some(ticket) = c.ticket.as_deref() else { continue };
            let ready_at = c.pr.as_ref().filter(|p| p.ready).and_then(|p| p.head.as_deref());
            let approved_ready = ready_at.is_some_and(|b| {
                !crate::wf_pr::same_head(&o.head, b)
                    && wait
                        .as_ref()
                        .is_some_and(|w| w.thread.as_deref() == Some(ticket) && crate::wf_pr::same_head(&w.head, b))
            });
            if let Some(base) = ready_at.filter(|b| !crate::wf_pr::same_head(&o.head, b) && !approved_ready) {
                let note = format!(
                    "head moved to {} before the merge was approved at {}; the worker sends ready for it",
                    short(&o.head),
                    short(base)
                );
                self.resolve_releasing(&[c.seq], &[ticket], &note)?;
                self.reach.hit(ReachPoint::ReadyCardDroppedHeadMoved);
                pass.woken.extend(self.release_ready_worker(ticket, &t.repo, t.number, &o.head)?);
                self.feed_note(
                    t,
                    Some(ticket),
                    &format!("{} merge card dropped: the head moved to {}", t.label(), short(&o.head)),
                )?;
                pass.actions.push(format!("{} merge card dropped: head moved to {}", t.label(), short(&o.head)));
                continue;
            }
            if !approved_ready
                && !self.store.link_get(ticket)?.is_some_and(|l| !l.closed && l.gate.as_deref() == Some("merge"))
            {
                continue;
            }
            let Some(base) = c.pr.as_ref().and_then(|p| p.head.clone()) else {
                self.set_card_pr(c, t.pr(&o.head))?;
                continue;
            };
            if crate::wf_pr::same_head(&o.head, &base) {
                continue;
            }
            match moved.iter_mut().find(|(x, _, _)| x.as_deref() == Some(ticket)) {
                Some((_, _, seqs)) => seqs.push(c.seq),
                None => moved.push((Some(ticket.into()), base, vec![c.seq])),
            }
        }
        if let Some(w) = &wait
            && w.head != o.head
            && !moved.iter().any(|(x, _, _)| *x == w.thread)
        {
            moved.push((w.thread.clone(), w.head.clone(), Vec::new()));
        }
        let mut clear_left = false;
        for (ticket, base, seqs) in &moved {
            if wait.as_ref().is_some_and(|w| w.thread == *ticket && w.head == o.head) {
                self.resolve_releasing(seqs, ticket.as_deref().as_slice(), "superseded by a newer merge approval")?;
                continue;
            }
            let approval = wait.as_ref().filter(|w| w.thread == *ticket).map(|w| w.approval.as_str());
            self.moved(t, o, ticket.as_deref(), base, seqs, approval, pass)?;
            if let Some(w) = wait.take_if(|w| w.thread == *ticket) {
                self.retire_merge_wait(&w)?;
                if w.status == WaitStatus::Sent {
                    clear_left = true;
                }
            }
        }
        let Some(mut w) = wait.filter(|w| w.head == o.head) else {
            return Ok(OpenOutcome { advance: None, clear_left });
        };
        let advance = match (w.status, o.queued) {
            (WaitStatus::Waiting, true) => {
                w.status = WaitStatus::Sent;
                if self.store.merge_wait_swap(&w)? {
                    self.feed_note(
                        t,
                        w.thread.as_deref(),
                        &format!("{} is in the merge queue at its approved head", t.label()),
                    )?;
                }
                None
            }
            (WaitStatus::Waiting, false) if !o.draft && now != Ready::Yes => {
                if let Ready::Failing(why) = &now {
                    self.held(t, o, &w, why, pass)?;
                }
                None
            }
            (WaitStatus::Waiting, false) if !o.draft => {
                if !w.pull_request_id.is_empty() && o.id.as_deref() == Some(w.pull_request_id.as_str()) {
                    let marked = MergeWait { advancing: Some(self.now()), ..w.clone() };
                    self.store.merge_wait_swap(&marked)?;
                    Some(w)
                } else {
                    self.reach.hit(ReachPoint::ReassignedIdRejected);
                    let ask = Ask {
                        what: format!("{} is no longer the pull request that was approved", t.label()),
                        reply: "reply \"merge\" to approve it again, or \"hold\" to leave it",
                        context: format!(
                            "The approval was for {} node \"{}\". That number now resolves to \"{}\".",
                            t.forge(),
                            w.pull_request_id,
                            o.id.as_deref().unwrap_or_default()
                        ),
                    };
                    self.raise(t, w.thread.as_deref(), "report", &ask, &o.head, None, Some(&w.approval), pass)?;
                    self.retire_merge_wait(&w)?;
                    None
                }
            }
            (WaitStatus::Sent, true) if w.failures > 0 => {
                w.failures = 0;
                w.last_error = None;
                self.store.merge_wait_swap(&w)?;
                None
            }
            (WaitStatus::Sent, false) => {
                let event = left.map(|l| format!("left:{l}"));
                let why = format!("it left the merge queue and {} recorded no reason", t.forge());
                self.failed(t, w, &why, event.as_deref(), pass)?;
                None
            }
            _ => None,
        };
        Ok(OpenOutcome { advance, clear_left })
    }

    fn held(&self, t: &Tracked, o: &Change, w: &MergeWait, why: &str, pass: &mut Pass) -> Result<()> {
        let event = format!("checks:{}", w.approval);
        let ticket = w.thread.as_deref();
        if self.store.pr_event_seen(&t.repo, t.number, &event, ticket.unwrap_or(BOARD))? {
            return Ok(());
        }
        let ask = Ask {
            what: format!("{} {HELD} {}: {why}", t.label(), short(&o.head)),
            reply: if t.forge() == "GitHub" {
                "reply \"fix\" to send it back to the worker, or re-run the check on GitHub"
            } else {
                "reply \"fix\" to send it back to the worker, or re-run the pipeline on GitLab"
            },
            context: "The approval stays. The board queues or merges the pull request once every check at this \
head has passed or ended cancelled, skipped or neutral. A push drops the approval."
                .into(),
        };
        self.raise(t, ticket, "report", &ask, &o.head, Some(&event), Some(&w.approval), pass)?;
        let text = format!("{} is held: a check failed at its approved head {}", t.label(), short(&o.head));
        self.feed_note(t, ticket, &text)?;
        pass.actions.push(text);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn dropped(
        &self,
        t: &Tracked,
        o: &Change,
        wait: Option<&MergeWait>,
        live_tickets: &[(String, String)],
        after: &str,
        pass: &mut Pass,
    ) -> Result<bool> {
        let Some((reason, at)) = o.removal.as_ref().filter(|(r, _)| !o.queued && r != "merged") else {
            return Ok(false);
        };
        if !later(at, after) {
            self.reach.hit(ReachPoint::QueueDropPredatesTracking);
            return Ok(false);
        }
        let superseded = format!("removed-superseded:{at}");
        if self.store.pr_event_seen(&t.repo, t.number, &superseded, BOARD)? {
            return Ok(false);
        }
        let newest = self.store.merge_wait_get(&t.repo, t.number)?;
        if newest.is_some_and(|w| parse_ts(&w.at).zip(parse_ts(at)).is_some_and(|(w, r)| w > r)) {
            self.store.pr_event_mark(&t.repo, t.number, &superseded, BOARD, &self.now())?;
            return Ok(false);
        }
        let why = why_dropped(reason);
        let what = format!("{} was dropped from the merge queue ({why}) at {}", t.label(), local_clock(at));
        if self.report_exit(
            t,
            o,
            wait,
            live_tickets,
            at,
            &format!("removed:{at}"),
            exit_ask(what, String::new()),
            &why,
            pass,
        )? {
            self.reach.hit(ReachPoint::QueueDropReported);
            let this_pr = |c: &Card| c.pr.as_ref().is_some_and(|p| p.repo == t.repo && p.number == t.number);
            let note = format!("{} recorded the reason: {why}", t.forge());
            self.resolve_cards(|c| c.kind == "report" && this_pr(c) && c.text.contains(NO_REASON), &note)?;
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exited(
        &self,
        t: &Tracked,
        o: &Change,
        left: &str,
        live_tickets: &[(String, String)],
        after: &str,
        status: Option<WaitStatus>,
        pass: &mut Pass,
    ) -> Result<Option<String>> {
        if o.removal.as_ref().is_some_and(|(_, at)| later(at, after)) {
            return Ok(None);
        }
        if status == Some(WaitStatus::Sent) || self.mins(Some(left)).is_some_and(|m| m < EXIT_WINDOW_MINS) {
            return Ok(Some(left.into()));
        }
        if status.is_some() {
            return Ok(None);
        }
        let what = format!("{} left the merge queue at {}, {NO_REASON}", t.label(), local_clock(left));
        let context =
            format!("{} still showed no removal reason {EXIT_WINDOW_MINS} minutes after the exit.", t.forge());
        self.report_exit(
            t,
            o,
            None,
            live_tickets,
            left,
            &format!("left:{left}"),
            exit_ask(what, context),
            NO_REASON,
            pass,
        )?;
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn report_exit(
        &self,
        t: &Tracked,
        o: &Change,
        wait: Option<&MergeWait>,
        live_tickets: &[(String, String)],
        at: &str,
        event: &str,
        ask: Ask,
        why: &str,
        pass: &mut Pass,
    ) -> Result<bool> {
        let mut raised = false;
        let mut to: Vec<Option<&str>> =
            live_tickets.iter().filter(|(_, s)| later(at, s)).map(|(x, _)| Some(x.as_str())).collect();
        if to.is_empty() {
            to.push(None);
        }
        for ticket in to {
            if self.store.pr_event_seen(&t.repo, t.number, event, ticket.unwrap_or(BOARD))? {
                continue;
            }
            let approval = wait.filter(|w| w.thread.as_deref() == ticket).map(|w| w.approval.as_str());
            self.raise(t, ticket, "report", &ask, &o.head, Some(event), approval, pass)?;
            self.feed_note(t, ticket, &format!("{} was dropped from the merge queue: {why}", t.label()))?;
            pass.actions.push(format!("{} dropped from the merge queue: {why}", t.label()));
            raised = true;
        }
        Ok(raised)
    }

    #[allow(clippy::too_many_arguments)]
    fn moved(
        &self,
        t: &Tracked,
        o: &Change,
        ticket: Option<&str>,
        base: &str,
        old: &[i64],
        approval: Option<&str>,
        pass: &mut Pass,
    ) -> Result<()> {
        let label = t.label();
        let (kind, ask) = if o.queued {
            let ask = Ask {
                what: format!(
                    "{label} is in the merge queue at head {}, not at its approved head {}",
                    short(&o.head),
                    short(base)
                ),
                reply: if t.forge() == "GitHub" {
                    "reply \"ok\" to let it merge at that head, or take it out of the queue on GitHub"
                } else {
                    "reply \"ok\" to let it merge at that head, or take it out of the merge train on GitLab"
                },
                context: format!("The approval was for head {base}. The queued head is {}.", o.head),
            };
            ("report", ask)
        } else {
            let ask = Ask {
                what: format!("{label} moved to head {} after it was approved at {}", short(&o.head), short(base)),
                reply: "reply \"merge\" to merge it at that head, or \"hold\" to wait",
                context: format!(
                    "The approval was for head {base}, so it no longer applies. The new head is {}.",
                    o.head
                ),
            };
            ("gate", ask)
        };
        self.raise(t, ticket, kind, &ask, &o.head, None, approval, pass)?;
        let note = format!("head moved to {} after this approval; replaced by a new card", short(&o.head));
        let release = ticket.filter(|_| kind == "report");
        self.resolve_releasing(old, release.as_slice(), &note)?;
        self.feed_note(t, ticket, &format!("{label} merge approval replaced: the head moved to {}", short(&o.head)))?;
        pass.actions.push(format!("{label} merge approval replaced: head moved to {}", short(&o.head)));
        Ok(())
    }

    async fn advance(&self, t: &Tracked, o: &Change, w: MergeWait, read_at: &str, pass: &mut Pass) -> Result<()> {
        let method = w.method.clone().or_else(|| o.method.clone()).unwrap_or_else(|| "MERGE".into());
        let how = if o.queue { Landing::Queue { method: &method } } else { Landing::Merge { method: &method } };
        let resp = self.host.merge(&t.change(), o.id.as_deref().unwrap_or_default(), &w.head, how).await;
        let _g = self.lock.lock().await;
        let queued = match &resp {
            Err(e) => return self.failed(t, w, &format!("{e:#}"), None, pass),
            Ok(Landed::Refused(e)) => return self.failed(t, w, e, None, pass),
            Ok(Landed::Waiting(e)) => {
                let mut next = w.clone();
                next.last_error = Some(e.clone());
                self.store.merge_wait_swap(&next)?;
                return Ok(());
            }
            Ok(Landed::Queued { position }) => Some(*position),
            Ok(Landed::Merged { .. }) => None,
        };
        let mut sent = w.clone();
        sent.status = WaitStatus::Sent;
        sent.failures = 0;
        sent.last_error = None;
        if !self.store.merge_wait_swap(&sent)? && !self.sentinel(Sentinel::AdvanceIgnoresSwapResult) {
            return Ok(());
        }
        self.reach.hit(if queued.is_some() { ReachPoint::AdvanceEnqueued } else { ReachPoint::AdvanceMerged });
        if queued.is_some()
            && let Some(mut seen) = self.store.pr_seen_get(&t.repo, t.number)?
        {
            seen.queued = true;
            seen.queued_at = Some(read_at.into());
            self.store.pr_seen_put(&seen)?;
            self.changed();
        }
        let text = match queued {
            Some(position) => format!(
                "{} queued at its approved head {} (position {}) once it was {}",
                t.label(),
                short(&w.head),
                json!(position),
                o.merge_state
            ),
            None => {
                format!("{} merged at its approved head {} once it was {}", t.label(), short(&w.head), o.merge_state)
            }
        };
        self.feed_note(t, w.thread.as_deref(), &text)?;
        pass.actions.push(text);
        Ok(())
    }

    fn failed(&self, t: &Tracked, w: MergeWait, why: &str, event: Option<&str>, pass: &mut Pass) -> Result<()> {
        let mut next = w.clone();
        next.failures += 1;
        next.last_error = Some(why.into());
        if next.failures < MAX_FAILURES {
            self.store.merge_wait_swap(&next)?;
            return Ok(());
        }
        let verb = if w.status == WaitStatus::Sent { "keep" } else { "queue" };
        let ask = Ask {
            what: format!("{} would not {verb} {} ({why})", t.forge(), t.label()),
            reply: "reply \"merge\" to try again, or \"hold\" to leave it",
            context: format!(
                "It was approved at head {}. {} refused {MAX_FAILURES} checks in a row.",
                w.head,
                t.forge()
            ),
        };
        self.raise(t, w.thread.as_deref(), "report", &ask, &w.head, event, Some(&w.approval), pass)?;
        self.retire_merge_wait(&w)?;
        self.feed_note(t, w.thread.as_deref(), &format!("{} could not be queued: {why}", t.label()))?;
        pass.actions.push(format!("{} could not be queued: {why}", t.label()));
        Ok(())
    }

    fn wake(&self, t: &Tracked, event: &str, ticket: &str, kind: &str, body: &str, pass: &mut Pass) -> Result<bool> {
        let now = self.now();
        let from = t.sender();
        let wake = Wake { repo: &t.repo, pr: t.number, event: Some(event), ticket, from, kind, body, at: &now };
        let fresh = self.store.pr_event_wake(&wake, |h| format!("{}\n{body}", header(kind, from, h, None)))?;
        if fresh.is_some() {
            pass.woken.push(ticket.into());
        }
        Ok(fresh.is_some())
    }

    #[allow(clippy::too_many_arguments)]
    fn raise(
        &self,
        t: &Tracked,
        ticket: Option<&str>,
        kind: &str,
        ask: &Ask,
        head: &str,
        event: Option<&str>,
        approval: Option<&str>,
        pass: &mut Pass,
    ) -> Result<()> {
        let link = ticket.map(|x| self.store.link_get(x)).transpose()?.flatten().filter(|l| !l.closed);
        let unticketed = no_ticket(t.forge());
        let act = if link.is_some() { ask.reply } else { &unticketed };
        let from = t.sender();
        let tail = if ask.context.is_empty() { String::new() } else { format!("\n\n{}", ask.context) };
        let body = format!("**ACTION NEEDED:** {}: {act}, here: {}{tail}", ask.what, t.url(&*self.host));
        let mark = |to: &str| match event {
            Some(e) => self.store.pr_event_mark(&t.repo, t.number, e, to, &self.now()).map(|_| ()),
            None => Ok(()),
        };
        match link {
            Some(mut l) if l.parent == BOARD => {
                let handle = self.store.msg_new(from, &l.child, kind, &body, &self.now())?;
                self.card_add(NewCard {
                    kind,
                    label: Some(&l.label),
                    ticket: Some(&l.child),
                    link: l.link.as_deref(),
                    body: &body,
                    handle: Some(&handle),
                    options: None,
                    pr: Some(t.pr(head)),
                    approval,
                    offer: None,
                    worker: None,
                })?;
                if kind == "gate" && l.gate.as_deref().is_none_or(|g| g == "merge" || g.starts_with("relay:")) {
                    let already_merge = l.gate.as_deref() == Some("merge");
                    l.gate = Some("merge".into());
                    l.gate_since = Some(self.now());
                    if !already_merge {
                        l.gate_pr = Some((t.repo.clone(), t.number));
                    }
                    l.nudges = 0;
                    l.escalated = false;
                    self.store.link_save(&l)?;
                }
                mark(&l.child)
            }
            Some(l) => {
                let now = self.now();
                let wake =
                    Wake { repo: &t.repo, pr: t.number, event, ticket: &l.child, from, kind, body: &body, at: &now };
                if self.store.pr_event_wake(&wake, |h| format!("{}\n{body}", header(kind, from, h, None)))?.is_some() {
                    pass.woken.push(l.child.clone());
                }
                Ok(())
            }
            None => {
                let handle = self.store.msg_new(from, BOARD, "report", &body, &self.now())?;
                self.card_add(NewCard {
                    kind: "report",
                    label: Some(from),
                    ticket: None,
                    link: None,
                    body: &body,
                    handle: Some(&handle),
                    options: Some(Vec::new()),
                    pr: Some(t.pr(head)),
                    approval,
                    offer: None,
                    worker: None,
                })?;
                mark(ticket.unwrap_or(BOARD))
            }
        }
    }

    fn set_card_pr(&self, c: &Card, pr: CardPr) -> Result<()> {
        let row = self.store.waiting_data(BOARD, false)?.into_iter().find(|(_, _, _, seq)| *seq == c.seq);
        let Some((_, Some(mut data), _, seq)) = row else { return Ok(()) };
        data["pr"] = json!(pr);
        self.store.waiting_set_data(BOARD, seq, &data)
    }

    fn feed_note(&self, t: &Tracked, ticket: Option<&str>, text: &str) -> Result<()> {
        let on_board = |t: &&str| self.store.link_get(t).ok().flatten().is_some_and(|l| l.parent == BOARD);
        let to = ticket.filter(on_board).unwrap_or(BOARD);
        self.store.msg_new(t.sender(), to, "note", text, &self.now())?;
        self.changed();
        Ok(())
    }
}
