use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::board::{BOARD, Card, NewCard, gh_name};
use crate::clock::{local_clock, parse_ts};
use crate::github::errors;
use crate::merge::{MERGEABLE, MergeArgs};
use crate::store::{GateSeen, MergeWait, PrSeen};
use crate::wf::{Wf, header};

pub const GITHUB: &str = "github";
pub const LAST_CHECK: &str = "reconcile_last";
const BATCH: usize = 100;
const PAGE_LOAD_GAP_MINS: f64 = 0.5;
const PR_FIELDS: &str = "fragment F on PullRequest { state isDraft headRefOid mergeStateStatus isInMergeQueue \
mergeQueueEntry { position } mergeCommit { oid } mergedAt closedAt \
timelineItems(last: 1, itemTypes: [REMOVED_FROM_MERGE_QUEUE_EVENT]) { nodes { ... on RemovedFromMergeQueueEvent { \
reason createdAt } } } }";

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

fn unanswerable(body: &str) -> String {
    let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
    let (Some(i), Some(j)) = (first.find(": reply \""), first.rfind(", here: ")) else { return body.into() };
    if j < i {
        return body.into();
    }
    format!(
        "{}: no ticket tracks it to take a reply, so act on it on GitHub or with wf.merge and then dismiss this card, \
         here: {}{}{rest}",
        &first[..i],
        &first[j + ", here: ".len()..],
        if rest.is_empty() { "" } else { "\n" }
    )
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn why_dropped(reason: &str) -> String {
    match reason {
        "merge_conflict" => "a merge conflict".into(),
        "failed_checks" => "failed checks".into(),
        "manual" => "removed by hand".into(),
        other => other.replace('_', " "),
    }
}

#[derive(Default)]
struct Tracked {
    owner: String,
    name: String,
    number: u64,
    tickets: Vec<String>,
    cards: Vec<Card>,
    wait: Option<MergeWait>,
}

impl Tracked {
    fn repo(&self) -> String {
        format!("{}/{}", self.owner, self.name).to_ascii_lowercase()
    }

    fn label(&self) -> String {
        format!("{}#{}", self.repo(), self.number)
    }

    fn url(&self) -> String {
        format!("https://github.com/{}/{}/pull/{}", self.owner, self.name, self.number)
    }

    fn ticket(&mut self, t: Option<&str>) {
        if let Some(t) = t.filter(|t| *t != BOARD)
            && !self.tickets.iter().any(|x| x == t)
        {
            self.tickets.push(t.to_string());
        }
    }
}

fn track<'a>(map: &'a mut BTreeMap<(String, u64), Tracked>, owner: &str, name: &str, n: u64) -> &'a mut Tracked {
    map.entry((format!("{owner}/{name}").to_ascii_lowercase(), n)).or_insert_with(|| Tracked {
        owner: owner.into(),
        name: name.into(),
        number: n,
        ..Tracked::default()
    })
}

fn batch_query(chunk: &[&Tracked]) -> (String, Value) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    let mut vars = serde_json::Map::new();
    for (i, t) in chunk.iter().enumerate() {
        params.push(format!("$o{i}: String!, $r{i}: String!, $n{i}: Int!"));
        fields.push(format!("p{i}: repository(owner: $o{i}, name: $r{i}) {{ pullRequest(number: $n{i}) {{ ...F }} }}"));
        vars.insert(format!("o{i}"), json!(t.owner));
        vars.insert(format!("r{i}"), json!(t.name));
        vars.insert(format!("n{i}"), json!(t.number));
    }
    let query = format!("query Reconcile({}) {{ {} }} {PR_FIELDS}", params.join(", "), fields.join(" "));
    (query, Value::Object(vars))
}

impl Wf {
    pub(crate) fn note_seen(&self, mut row: PrSeen) -> Result<()> {
        let prev = self.store.pr_seen_get(&row.repo, row.pr)?.unwrap_or_default();
        row.thread = row.thread.or(prev.thread);
        row.gate = prev.gate;
        row.at = self.now();
        self.store.pr_seen_put(&row)
    }

    pub fn pr_view(&self, pr: &Value) -> Value {
        let mut pr = pr.clone();
        if pr["state"] == "open"
            && let Some((o, r, n)) = pr["url"].as_str().and_then(|u| pr_refs(u).into_iter().next())
            && let Ok(Some(seen)) = self.store.pr_seen_get(&format!("{o}/{r}").to_ascii_lowercase(), n)
            && seen.queued
            && seen.state == "OPEN"
        {
            pr["state"] = json!("queued");
        }
        pr
    }

    pub async fn reconcile_if_due(&self) -> Result<Vec<String>> {
        let b = &self.config.board;
        if b.reconcile_idle_minutes == 0 {
            return Ok(Vec::new());
        }
        let active = !self.store.merge_waits()?.is_empty()
            || self.store.pr_seen_all()?.iter().any(|p| p.queued && p.state == "OPEN");
        let every = if active { b.reconcile_active_minutes } else { b.reconcile_idle_minutes } as f64;
        self.reconcile_after(every).await
    }

    pub async fn reconcile_soon(&self) -> Result<Vec<String>> {
        if self.config.board.reconcile_idle_minutes == 0 {
            return Ok(Vec::new());
        }
        self.reconcile_after(PAGE_LOAD_GAP_MINS).await
    }

    pub async fn reconcile(&self) -> Result<Vec<String>> {
        self.reconcile_after(0.0).await
    }

    async fn reconcile_after(&self, gap_mins: f64) -> Result<Vec<String>> {
        let Ok(_running) = self.reconcile_lock.try_lock() else { return Ok(Vec::new()) };
        let last = self.store.meta_get(LAST_CHECK)?;
        if gap_mins > 0.0 && self.mins(last.as_deref()).is_some_and(|m| m < gap_mins) {
            return Ok(Vec::new());
        }
        self.store.meta_set(LAST_CHECK, &self.now())?;
        let tracked = self.tracked()?;
        let seen: HashMap<(String, u64), PrSeen> =
            self.store.pr_seen_all()?.into_iter().map(|p| ((p.repo.clone(), p.pr), p)).collect();
        let mut actions = Vec::new();
        let list: Vec<&Tracked> = tracked.values().collect();
        for chunk in list.chunks(BATCH) {
            let (query, vars) = batch_query(chunk);
            let resp = self.github.graphql(&query, vars).await?;
            if !resp["data"].is_object() {
                bail!("GitHub reconcile query failed: {}", errors(&resp).unwrap_or_else(|| "no data".into()));
            }
            for (i, t) in chunk.iter().enumerate() {
                let pr = &resp["data"][format!("p{i}")]["pullRequest"];
                if pr.is_object() {
                    self.fix(t, pr, seen.get(&(t.repo(), t.number)), &mut actions).await?;
                }
            }
        }
        for row in self.store.pr_seen_all()? {
            if row.state == "OPEN" && !row.queued && !tracked.contains_key(&(row.repo.clone(), row.pr)) {
                self.store.pr_seen_remove_if(&row)?;
            }
        }
        if !actions.is_empty() {
            self.changed();
        }
        Ok(actions)
    }

    fn tracked(&self) -> Result<BTreeMap<(String, u64), Tracked>> {
        let mut map = BTreeMap::new();
        for ln in self.store.links_open()?.into_iter().filter(|l| l.role == "worker") {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner()).get(&ln.child).cloned();
            let urls: Vec<String> = live
                .as_ref()
                .map(|l| {
                    let prs = l["prs"].as_array().into_iter().flatten();
                    prs.chain([&l["pr"]["url"]]).filter_map(|u| u.as_str().map(String::from)).collect()
                })
                .unwrap_or_default();
            for (o, r, n) in urls.iter().flat_map(|u| pr_refs(u)) {
                track(&mut map, &o, &r, n).ticket(Some(&ln.parent));
            }
        }
        for card in self.cards()? {
            let mut refs = pr_refs(&card.text);
            if refs.is_empty()
                && let Some(m) = card.handle.as_deref().map(|h| self.store.msg_get(h)).transpose()?.flatten()
            {
                refs = pr_refs(&m.body);
            }
            if let [(o, r, n)] = refs.as_slice() {
                let t = track(&mut map, o, r, *n);
                t.ticket(card.ticket.as_deref());
                t.cards.push(card);
            }
        }
        for w in self.store.merge_waits()? {
            let Some((o, r)) = w.repo.split_once('/') else { continue };
            let t = track(&mut map, o, r, w.pr);
            t.ticket(w.thread.as_deref());
            t.wait = Some(w);
        }
        for row in self.store.pr_seen_all()?.into_iter().filter(|p| p.queued && p.state == "OPEN") {
            let Some((o, r)) = row.repo.split_once('/') else { continue };
            track(&mut map, o, r, row.pr).ticket(row.thread.as_deref());
        }
        Ok(map)
    }

    async fn fix(&self, t: &Tracked, pr: &Value, prev: Option<&PrSeen>, actions: &mut Vec<String>) -> Result<()> {
        let repo = t.repo();
        if self.store.pr_seen_get(&repo, t.number)?.as_ref() != prev {
            return Ok(());
        }
        let prev = prev.cloned();
        let state = pr["state"].as_str().unwrap_or_default().to_string();
        let head = pr["headRefOid"].as_str().unwrap_or_default().to_ascii_lowercase();
        let queued = pr["isInMergeQueue"] == true;
        let removal = &pr["timelineItems"]["nodes"][0];
        let removed_at = removal["createdAt"].as_str().map(String::from);
        let mut row = PrSeen {
            repo: repo.clone(),
            pr: t.number,
            state: state.clone(),
            head: head.clone(),
            queued,
            removed_at: removed_at.clone(),
            thread: prev
                .as_ref()
                .and_then(|p| p.thread.clone())
                .or_else(|| t.wait.as_ref().and_then(|w| w.thread.clone()))
                .or_else(|| t.tickets.first().cloned()),
            gate: None,
            at: self.now(),
        };
        let moved = prev.as_ref().map(|p| (p.state.as_str(), p.queued)) != Some((state.as_str(), queued));
        match state.as_str() {
            "MERGED" | "CLOSED" => {
                self.finished(t, pr, prev.as_ref(), actions).await?;
                if self.store.pr_seen_swap(prev.as_ref(), &row)? && moved {
                    self.changed();
                }
                Ok(())
            }
            "OPEN" => {
                let dropped = removal["reason"].as_str().filter(|r| *r != "merged");
                if let (Some(p), Some(reason), false) = (&prev, dropped, queued)
                    && p.removed_at != removed_at
                {
                    self.dropped(t, p, reason, removal["createdAt"].as_str().unwrap_or_default(), actions).await?;
                }
                row.gate = self.check_gate(t, prev.as_ref(), &head, actions).await?;
                if self.store.pr_seen_swap(prev.as_ref(), &row)? && moved {
                    self.changed();
                }
                if let Some(w) = &t.wait {
                    self.advance_wait(t, w, pr, &head, queued, actions).await?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn finished(&self, t: &Tracked, pr: &Value, prev: Option<&PrSeen>, actions: &mut Vec<String>) -> Result<()> {
        let merged = pr["state"] == "MERGED";
        let commit = pr["mergeCommit"]["oid"].as_str().unwrap_or_default();
        let ended = pr[if merged { "mergedAt" } else { "closedAt" }].as_str().unwrap_or_default();
        let when = local_clock(ended);
        let before = |c: &Card| match (parse_ts(&c.at), parse_ts(ended)) {
            (Some(opened), Some(end)) => opened <= end,
            _ => true,
        };
        let stale: Vec<i64> = t.cards.iter().filter(|c| before(c)).map(|c| c.seq).collect();
        let note = if merged {
            format!("merged on GitHub at {}, {when}", short(commit))
        } else {
            format!("closed on GitHub without merging, {when}")
        };
        let _g = self.lock.lock().await;
        let resolved = self.resolve_cards(|c| stale.contains(&c.seq), &note)?;
        if let Some(w) = &t.wait {
            self.store.merge_wait_remove_if(w)?;
        }
        let state = pr["state"].as_str().unwrap_or_default();
        let fire = match prev {
            Some(p) => p.state != state,
            None => !stale.is_empty() || t.wait.is_some(),
        };
        if fire {
            let body = if merged {
                format!(
                    "{} merged on GitHub at {}, {when} (merge commit {commit}). Run your post-merge steps. {}",
                    t.label(),
                    short(commit),
                    t.url()
                )
            } else {
                format!("{} was closed on GitHub without merging, {when}. {}", t.label(), t.url())
            };
            if self.tell(&t.tickets, &body).await? == 0 {
                self.feed_note(None, &body)?;
            }
            actions.push(format!("{} {note}", t.label()));
        } else if !resolved.is_empty() {
            self.feed_note(t.tickets.first().map(String::as_str), &format!("{}: {note}", t.label()))?;
            actions.push(format!("{} cards {} resolved: {note}", t.label(), resolved.join(", ")));
        }
        Ok(())
    }

    async fn dropped(
        &self,
        t: &Tracked,
        prev: &PrSeen,
        reason: &str,
        at: &str,
        actions: &mut Vec<String>,
    ) -> Result<()> {
        let why = why_dropped(reason);
        let body = format!(
            "**ACTION NEEDED:** {label} was dropped from the merge queue ({why}) at {}: reply \"requeue\" to queue it \
             again, or \"hold\" to leave it out, here: {url}",
            local_clock(at),
            label = t.label(),
            url = t.url()
        );
        let ticket = t.tickets.first().map(String::as_str);
        let _g = self.lock.lock().await;
        if self.store.pr_seen_get(&prev.repo, prev.pr)?.as_ref() != Some(prev) {
            return Ok(());
        }
        self.raise(ticket, "report", &body).await?;
        self.feed_note(ticket, &format!("{} was dropped from the merge queue: {why}", t.label()))?;
        actions.push(format!("{} dropped from the merge queue: {why}", t.label()));
        Ok(())
    }

    async fn check_gate(
        &self,
        t: &Tracked,
        prev: Option<&PrSeen>,
        head: &str,
        actions: &mut Vec<String>,
    ) -> Result<Option<GateSeen>> {
        let mut gate = None;
        for c in t.cards.iter().filter(|c| c.kind == "gate") {
            let Some(ticket) = c.ticket.as_deref() else { continue };
            if self.store.link_get(ticket)?.is_some_and(|l| !l.closed && l.gate.as_deref() == Some("merge")) {
                gate = Some((c, ticket));
                break;
            }
        }
        let Some((card, ticket)) = gate else { return Ok(None) };
        let base = match prev.and_then(|p| p.gate.as_ref()) {
            Some(g) if g.seq == card.seq => g.head.clone(),
            _ => {
                let body = card.handle.as_deref().map(|h| self.store.msg_get(h)).transpose()?.flatten();
                body.and_then(|m| approved_head(&m.body)).unwrap_or_else(|| head.to_string())
            }
        };
        if head.starts_with(&base) {
            return Ok(Some(GateSeen { seq: card.seq, head: base }));
        }
        let label = t.label();
        let _g = self.lock.lock().await;
        let note = format!("head moved to {} after this gate opened; replaced by a new gate", short(head));
        if self.resolve_cards(|c| c.seq == card.seq, &note)?.is_empty() {
            return Ok(None);
        }
        let body = format!(
            "**ACTION NEEDED:** {label} moved to head {} after its merge gate opened: reply \"merge\" to merge it at \
             that head, or \"hold\" to wait, here: {}\n\n\
             The gate was for head {}, so that approval no longer applies. The new head is {head}.",
            short(head),
            t.url(),
            short(&base)
        );
        let id = self.raise(Some(ticket), "gate", &body).await?;
        self.feed_note(Some(ticket), &format!("{label} merge gate replaced: the head moved to {}", short(head)))?;
        actions.push(format!("{label} merge gate replaced: head moved"));
        let seq = id.and_then(|id| self.cards().ok()?.into_iter().find(|c| c.id == id).map(|c| c.seq));
        Ok(seq.map(|seq| GateSeen { seq, head: head.to_string() }))
    }

    async fn advance_wait(
        &self,
        t: &Tracked,
        w: &MergeWait,
        pr: &Value,
        head: &str,
        queued: bool,
        actions: &mut Vec<String>,
    ) -> Result<()> {
        let label = t.label();
        if queued {
            self.store.merge_wait_remove_if(w)?;
            return Ok(());
        }
        if head != w.head {
            let _g = self.lock.lock().await;
            if !self.store.merge_wait_remove_if(w)? {
                return Ok(());
            }
            let body = format!(
                "**ACTION NEEDED:** {label} moved to head {} before it could be queued: reply \"merge\" to merge it \
                 at that head, or \"hold\" to wait, here: {}\n\n\
                 The merge was approved at head {}, so it was not queued. The new head is {head}.",
                short(head),
                t.url(),
                short(&w.head)
            );
            if let Err(e) = self.raise(w.thread.as_deref(), "gate", &body).await {
                self.store.merge_wait_restore(w)?;
                return Err(e);
            }
            self.feed_note(w.thread.as_deref(), &format!("{label} was not queued: the head moved to {}", short(head)))?;
            actions.push(format!("{label} waiting merge dropped: head moved"));
            return Ok(());
        }
        let state = pr["mergeStateStatus"].as_str().unwrap_or("UNKNOWN");
        if !MERGEABLE.contains(&state) || pr["isDraft"] == true {
            return Ok(());
        }
        let args = MergeArgs { repo: w.repo.clone(), pr: w.pr, head: w.head.clone(), method: w.method.clone() };
        let out = match self.merge(w.thread.as_deref(), args).await {
            Ok(out) => out,
            Err(e) => {
                tracing::warn!("queueing {label} failed: {e:#}");
                return Ok(());
            }
        };
        let _g = self.lock.lock().await;
        match out["status"].as_str() {
            Some("queued") => {
                let text = format!(
                    "{label} queued at its approved head {} (position {}) once it was {state}",
                    short(head),
                    out["position"]
                );
                self.feed_note(w.thread.as_deref(), &text)?;
                actions.push(text);
            }
            Some("merged") => {
                let commit = out["commit"].as_str().unwrap_or_default();
                let body = format!(
                    "{label} merged on GitHub at {}, {} (merge commit {commit}). Run your post-merge steps. {}",
                    short(commit),
                    local_clock(&self.now()),
                    t.url()
                );
                if self.tell(&t.tickets, &body).await? == 0 {
                    self.feed_note(None, &body)?;
                }
                actions.push(format!("{label} merged"));
            }
            Some("refused") => {
                if !self.store.merge_wait_remove_if(w)? {
                    return Ok(());
                }
                let reason = out["reason"].as_str().unwrap_or("no reason given");
                let body = format!(
                    "**ACTION NEEDED:** GitHub would not queue {label} ({reason}): decide what to do, here: {}\n\n\
                     It was waiting to be queued at head {}.",
                    t.url(),
                    short(head)
                );
                self.raise(w.thread.as_deref(), "report", &body).await?;
                self.feed_note(w.thread.as_deref(), &format!("{label} could not be queued: {reason}"))?;
                actions.push(format!("{label} could not be queued: {reason}"));
            }
            _ => {}
        }
        Ok(())
    }

    async fn tell(&self, tickets: &[String], body: &str) -> Result<usize> {
        let mut told = 0;
        for t in tickets {
            if self.store.link_get(t)?.is_some_and(|l| l.closed) {
                continue;
            }
            let handle = self.store.msg_new(GITHUB, t, "event", body, &self.now())?;
            let text = format!("{}\n{body}", header("event", GITHUB, &handle, None));
            self.send_wake(t, &text, &handle).await?;
            told += 1;
        }
        Ok(told)
    }

    async fn raise(&self, ticket: Option<&str>, kind: &str, body: &str) -> Result<Option<String>> {
        let link = match ticket {
            Some(t) => self.store.link_get(t)?,
            None => None,
        };
        let answerable = ticket.is_some() && link.as_ref().is_none_or(|l| !l.closed);
        let body = if answerable { body.to_string() } else { unanswerable(body) };
        let body = body.as_str();
        let handle = self.store.msg_new(GITHUB, ticket.unwrap_or(BOARD), kind, body, &self.now())?;
        match (ticket, link) {
            (_, Some(mut l)) if l.parent == BOARD && !l.closed => {
                if kind == "gate" {
                    l.gate = Some("merge".into());
                    l.gate_since = Some(self.now());
                    l.nudges = 0;
                    l.escalated = false;
                    self.store.link_save(&l)?;
                }
                let card = NewCard {
                    kind,
                    label: Some(&l.label),
                    ticket: Some(&l.child),
                    link: l.link.as_deref(),
                    body,
                    handle: Some(&handle),
                    options: None,
                };
                Ok(Some(self.board_card(card)?))
            }
            (Some(t), l) if l.as_ref().is_none_or(|l| !l.closed) => {
                let text = format!("{}\n{body}", header(kind, GITHUB, &handle, None));
                self.send_wake(t, &text, &handle).await?;
                Ok(None)
            }
            _ => {
                let card = NewCard {
                    kind,
                    label: Some(GITHUB),
                    ticket: None,
                    link: None,
                    body,
                    handle: Some(&handle),
                    options: Some(Vec::new()),
                };
                Ok(Some(self.card_add(card)?))
            }
        }
    }

    fn feed_note(&self, ticket: Option<&str>, text: &str) -> Result<()> {
        let on_board = |t: &&str| self.store.link_get(t).ok().flatten().is_some_and(|l| l.parent == BOARD);
        let to = ticket.filter(on_board).unwrap_or(BOARD);
        self.store.msg_new(GITHUB, to, "note", text, &self.now())?;
        self.changed();
        Ok(())
    }
}
